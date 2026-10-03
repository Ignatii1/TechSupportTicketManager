using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;

namespace TicketBoard.Services;

/// <summary>Поддельный Интрасервис для самопроверок (ClaudeRelay, KnowledgeExport): настоящий клиент ходит к нему по HTTP
/// на loopback, ответ на каждый запрос даёт respond(путь с параметрами). По одному соединению за раз, ответ целиком
/// и Connection: close. Клиент, оборвавший запрос на полпути (отмена поиска), сервер не роняет: следующий получит ответ.</summary>
internal static class FakeIntraservice
{
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
                        while (!head.ToString().Contains("\r\n\r\n"))
                        {
                            var read = await stream.ReadAsync(buf);
                            if (read == 0) break;
                            head.Append(Encoding.ASCII.GetString(buf, 0, read));
                        }
                        var target = head.ToString().Split(' ') is { Length: > 1 } parts ? parts[1] : "";
                        var (code, json) = respond(target);
                        var body = Encoding.UTF8.GetBytes(json);
                        await stream.WriteAsync(Encoding.ASCII.GetBytes(
                            $"HTTP/1.1 {code} X\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {body.Length}\r\nConnection: close\r\n\r\n"));
                        await stream.WriteAsync(body);
                    }
                    catch (Exception e) when (e is IOException or SocketException) { /* клиент ушёл, не дождавшись ответа */ }
                }
            }
        });
        return (listener, ((IPEndPoint)listener.LocalEndpoint).Port);
    }
}
