using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;

namespace TicketBoard.Services;

/// <summary>Поддельный Интрасервис для самопроверок (ClaudeRelay, KnowledgeExport): настоящий клиент ходит к нему по HTTP
/// на loopback, ответ на каждый запрос даёт respond(путь с параметрами). По одному соединению за раз, ответ целиком
/// и Connection: close. Клиент, оборвавший запрос на полпути (отмена поиска), сервер не роняет: следующий получит ответ.
/// Соединение, по которому запрос так и не пришёл (HttpClient открыл его для отменённого запроса и держит свободным в пуле
/// минуту), ждём недолго и бросаем: иначе сервер, обслуживающий по одному, встал бы на эту минуту.</summary>
internal static class FakeIntraservice
{
    /// <summary>Сколько ждём запрос по принятому соединению.</summary>
    private static readonly TimeSpan RequestWait = TimeSpan.FromSeconds(2);

    /// <summary>Запустить на loopback, на свободном порту. Остановить — listener.Stop() в конце самопроверки.</summary>
    public static (TcpListener Listener, int Port) Start(Func<string, (int Code, string Json)> respond)
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        _ = Task.Run(async () =>
        {
            while (true)
            {
                TcpClient connection;
                try { connection = await listener.AcceptTcpClientAsync(); }
                catch (Exception) { return; }   // Stop() в конце самопроверки
                using (connection)
                {
                    try
                    {
                        var stream = connection.GetStream();
                        var head = new StringBuilder();
                        var buf = new byte[4096];
                        using var wait = new CancellationTokenSource(RequestWait);
                        while (!head.ToString().Contains("\r\n\r\n"))
                        {
                            var read = await stream.ReadAsync(buf, wait.Token);
                            if (read == 0) break;
                            head.Append(Encoding.ASCII.GetString(buf, 0, read));
                        }
                        if (!head.ToString().Contains("\r\n\r\n")) continue;   // клиент ушёл, так ничего и не спросив
                        var target = head.ToString().Split(' ') is { Length: > 1 } parts ? parts[1] : "";
                        var (code, json) = Refused(target) ?? respond(target);
                        var body = Encoding.UTF8.GetBytes(json);
                        await stream.WriteAsync(Encoding.ASCII.GetBytes(
                            $"HTTP/1.1 {code} X\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {body.Length}\r\nConnection: close\r\n\r\n"));
                        await stream.WriteAsync(body);
                    }
                    catch (Exception e) when (e is IOException or SocketException or OperationCanceledException) { /* клиент ушёл, не дождавшись ответа, или так и не спросил */ }
                }
            }
        });
        return (listener, ((IPEndPoint)listener.LocalEndpoint).Port);
    }

    /// <summary>Чего не принимает живой сервер, не принимает и этот — тем же ответом, что видел пользователь: count у живого
    /// логический, и count=all из документации даёт HTTP 400 (2026-10-08).</summary>
    private static (int Code, string Json)? Refused(string target)
    {
        var query = target.IndexOf('?') is var at and >= 0 ? target[(at + 1)..] : "";
        foreach (var pair in query.Split('&'))
            if (pair.StartsWith("count=", StringComparison.OrdinalIgnoreCase) && !bool.TryParse(Uri.UnescapeDataString(pair[6..]), out _))
                return (400, "{\"errors\":{\"count\":[\"The value '" + pair[6..] + "' is not valid.\"]},"
                    + "\"type\":\"https://tools.ietf.org/html/rfc7231#section-6.5.1\",\"title\":\"One or more validation errors occurred.\","
                    + "\"status\":400,\"traceId\":\"00-fake-00\"}");
        return null;
    }
}
