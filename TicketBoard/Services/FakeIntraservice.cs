using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Text;

namespace TicketBoard.Services;

/// <summary>Поддельный Интрасервис для самопроверок (ClaudeRelay, KnowledgeExport): настоящий клиент ходит к нему по HTTP
/// на loopback, ответ на каждый запрос даёт respond(путь с параметрами), на запись (не GET) — write(запрос целиком; нет его —
/// 405; код 3xx — переадресация, Json — куда; тело с «<» — html-страница). По одному соединению за раз, ответ целиком и Connection: close. Клиент, оборвавший запрос на полпути (отмена поиска), сервер не роняет: следующий получит ответ.
/// Соединение, по которому запрос так и не пришёл (HttpClient открыл его для отменённого запроса и держит свободным в пуле
/// минуту), ждём недолго и бросаем: иначе сервер, обслуживающий по одному, встал бы на эту минуту.</summary>
internal static class FakeIntraservice
{
    /// <summary>Сколько ждём запрос по принятому соединению.</summary>
    private static readonly TimeSpan RequestWait = TimeSpan.FromSeconds(2);

    /// <summary>Запустить на loopback, на свободном порту. Остановить — listener.Stop() в конце самопроверки. file — ответ
    /// файлом (вложение заявки) вместо json; null — запрос не к файлу. write — ответ на запрос с телом (PUT, POST…).</summary>
    public static (TcpListener Listener, int Port) Start(Func<string, (int Code, string Json)> respond, Func<string, FakeFile?>? file = null,
        Func<FakeRequest, (int Code, string Json)>? write = null)
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
                        // байтами, а не строкой: тело записи — UTF-8, а заголовки — ASCII
                        var received = new MemoryStream();
                        var buf = new byte[4096];
                        using var wait = new CancellationTokenSource(RequestWait);
                        int end;
                        while ((end = HeadEnd(received)) < 0)
                        {
                            var read = await stream.ReadAsync(buf, wait.Token);
                            if (read == 0) break;
                            received.Write(buf, 0, read);
                        }
                        if (end < 0) continue;   // клиент ушёл, так ничего и не спросив
                        var head = Encoding.ASCII.GetString(received.GetBuffer(), 0, end);
                        var parts = head.Split(' ');
                        var (method, target) = (parts[0], parts.Length > 1 ? parts[1] : "");
                        (int Code, string Json) answer;
                        var page = false;   // ответ на запись: переадресация (3xx — Json это Location) или html-страница
                        if (method != "GET")
                        {
                            var headers = Headers(head);
                            var length = int.TryParse(headers.GetValueOrDefault("Content-Length"), out var l) ? l : 0;
                            var sent = new MemoryStream();
                            sent.Write(received.GetBuffer(), end + 4, (int)received.Length - end - 4);
                            while (sent.Length < length)
                            {
                                var read = await stream.ReadAsync(buf, wait.Token);
                                if (read == 0) break;
                                sent.Write(buf, 0, read);
                            }
                            var request = new FakeRequest(method, target, headers, Encoding.UTF8.GetString(sent.GetBuffer(), 0, (int)sent.Length));
                            answer = write?.Invoke(request) ?? (405, "{\"Message\":\"The requested resource does not support http method '" + method + "'.\"}");
                            page = true;
                        }
                        else if (file?.Invoke(target) is { } f)
                        {
                            await SendFileAsync(stream, f, head + "\r\n\r\n");
                            continue;
                        }
                        else answer = Refused(target) ?? respond(target);
                        var (code, json) = answer;
                        if (page && code is >= 300 and < 400)
                        {
                            await stream.WriteAsync(Encoding.ASCII.GetBytes($"HTTP/1.1 {code} X\r\nLocation: {json}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
                            continue;
                        }
                        var body = Encoding.UTF8.GetBytes(json);
                        var type = page && json.StartsWith('<') ? "text/html" : "application/json";
                        await stream.WriteAsync(Encoding.ASCII.GetBytes(
                            $"HTTP/1.1 {code} X\r\nContent-Type: {type}; charset=utf-8\r\nContent-Length: {body.Length}\r\nConnection: close\r\n\r\n"));
                        await stream.WriteAsync(body);
                    }
                    catch (Exception e) when (e is IOException or SocketException or OperationCanceledException) { /* клиент ушёл, не дождавшись ответа, или так и не спросил */ }
                }
            }
        });
        return (listener, ((IPEndPoint)listener.LocalEndpoint).Port);
    }

    /// <summary>Файл целиком; Wait — сначала помолчать столько (заголовки не идут); Cut — послать столько байт и оборвать
    /// соединение (Content-Length — полный: клиент видит обрыв); Stall — после Cut байт замолчать на столько, потом дослать
    /// остальное (если клиент ещё ждёт).</summary>
    private static async Task SendFileAsync(NetworkStream stream, FakeFile f, string head)
    {
        if (f.Wait > TimeSpan.Zero) await Task.Delay(f.Wait);
        // Ranges — сервер понимает «Range: bytes=0-0»: отвечает 206 одним байтом и всей длиной в Content-Range (UnknownTotal —
        // «/*»: длина неизвестна); у пустого файла такого диапазона нет — 416 и «bytes */0»
        if (f.Ranges && head.Contains("\r\nRange: bytes=0-0\r\n", StringComparison.OrdinalIgnoreCase))
        {
            f.RangeServed?.Invoke();
            await stream.WriteAsync(Encoding.UTF8.GetBytes(f.Body.Length == 0
                ? "HTTP/1.1 416 X\r\nContent-Range: bytes */0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                : $"HTTP/1.1 206 X\r\nContent-Type: {f.Type}\r\nContent-Range: bytes 0-0/{(f.UnknownTotal ? "*" : f.Body.Length)}\r\n"
                    + "Content-Length: 1\r\nConnection: close\r\n\r\n"));
            if (f.Body.Length > 0) await stream.WriteAsync(f.Body.AsMemory(0, 1));
            return;
        }
        if (f.Location is { } location)
        {
            await stream.WriteAsync(Encoding.ASCII.GetBytes($"HTTP/1.1 302 X\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
            return;
        }
        await stream.WriteAsync(Encoding.UTF8.GetBytes(
            $"HTTP/1.1 {f.Code} X\r\nContent-Type: {f.Type}\r\n" + (f.NoLength ? "" : $"Content-Length: {f.Body.Length}\r\n") + "Connection: close\r\n\r\n"));
        var sent = Math.Min(f.Cut ?? f.Body.Length, f.Body.Length);
        await stream.WriteAsync(f.Body.AsMemory(0, sent));
        await stream.FlushAsync();
        f.Sent?.Invoke();
        if (f.Stall <= TimeSpan.Zero) return;   // оборвали — соединение закрывается
        await Task.Delay(f.Stall);
        await stream.WriteAsync(f.Body.AsMemory(sent));
    }

    /// <summary>Где кончаются заголовки (перед пустой строкой); -1 — ещё не пришли целиком.</summary>
    private static int HeadEnd(MemoryStream received)
    {
        var bytes = received.GetBuffer();
        for (var i = 0; i + 3 < received.Length; i++)
            if (bytes[i] == '\r' && bytes[i + 1] == '\n' && bytes[i + 2] == '\r' && bytes[i + 3] == '\n') return i;
        return -1;
    }

    /// <summary>Заголовки запроса по именам, без учёта регистра (первая строка — метод и путь — не заголовок).</summary>
    private static Dictionary<string, string> Headers(string head)
    {
        var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var line in head.Split("\r\n").Skip(1))
            if (line.IndexOf(':') is var at and > 0) headers[line[..at].Trim()] = line[(at + 1)..].Trim();
        return headers;
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

/// <summary>Запрос с телом (запись): метод, путь с параметрами, заголовки (имена без учёта регистра) и тело — строкой UTF-8.</summary>
internal sealed record FakeRequest(string Method, string Target, IReadOnlyDictionary<string, string> Headers, string Body);

/// <summary>Ответ поддельного сервера файлом: тело; Cut — сколько байт послать, прежде чем оборвать или замолчать; Stall —
/// сколько молчать после них; Sent — когда первые байты ушли (проверка останавливает выгрузку посреди файла); Type — тип
/// содержимого; Location — не файл, а переадресация туда (страница входа); Code — код ответа (отказ с телом своего типа);
/// Wait — сколько молчать до заголовков; Ranges — понимает «Range: bytes=0-0» (206, RangeServed — когда так ответил;
/// UnknownTotal — длина в Content-Range «*»); NoLength — без Content-Length (конец тела — закрытие соединения).</summary>
internal sealed record FakeFile(byte[] Body, int? Cut = null, TimeSpan Stall = default, Action? Sent = null,
    string Type = "application/octet-stream", string? Location = null, int Code = 200, TimeSpan Wait = default,
    bool Ranges = false, Action? RangeServed = null, bool NoLength = false, bool UnknownTotal = false);
