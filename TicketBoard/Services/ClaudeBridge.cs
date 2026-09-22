using System.Globalization;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text.Unicode;

namespace TicketBoard.Services;

/// <summary>Карточка доски для Claude: снимок, собранный в UI-потоке (App.BoardSnapshot). CompletedAt — только у «Готово».</summary>
public sealed record BridgeCard(int? Id, string Title, string Column, string Priority, string? IntraserviceStatus,
    int DaysInColumn, DateTimeOffset? CompletedAt, string Url, string Description, IReadOnlyList<BridgeNote> Notes);

public sealed record BridgeNote(DateTimeOffset Date, string Text);

/// <summary>Мост для Claude: крошечный HTTP-сервер на 127.0.0.1. Отдаёт страницу чата (папка Bridge, ресурсы exe)
/// и API только для чтения: поиск, заявка, её история и доска. В Claude данные уходят из браузера — у приложения
/// интернета нет, сюда ходит только страница. Защита: слушаем только loopback; заголовок Host — только наш (иначе
/// чужой сайт достал бы API через DNS rebinding); /api — только с ключом из ссылки (другие программы и пользователи
/// этой машины, пока порт наш). Предел: страница помнит ключи в браузере для адреса 127.0.0.1:порт — займи этот порт
/// кто-то другой, пока TicketBoard выключен, его страница их прочтёт. Поэтому мост — для компьютера с одним
/// пользователем Windows (README, «Claude» → «Безопасность»).</summary>
public sealed partial class ClaudeBridge : IDisposable
{
    /// <summary>Сбой обработки запроса (не сетевой) — в errors.log; App подставляет AppendLog.</summary>
    public static Action<string>? Log { get; set; }

    // ponytail: свой HTTP на TcpListener — только GET, запрос без тела, ответ целиком и Connection: close. Страница и
    // пять методов больше и не просят; HttpListener на Windows — это http.sys с его резервированием URL, Kestrel — +30 МБ.
    private const int MaxHead = 16 * 1024;
    private const string Csp = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; " +
        "connect-src 'self' https://api.anthropic.com; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

    /// <summary>Файлы страницы: встроены в exe как ресурсы Bridge/имя (TicketBoard.csproj).</summary>
    private static readonly Dictionary<string, string> Assets = new(StringComparer.Ordinal)
    {
        ["/"] = "index.html",
        ["/app.js"] = "app.js",
        ["/app.css"] = "app.css",
        ["/anthropic-sdk.mjs"] = "anthropic-sdk.mjs",
    };

    private static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Encoder = JavaScriptEncoder.Create(UnicodeRanges.All),   // кириллица как есть, а не escape-кодами по 6 байт
    };

    private readonly TcpListener _listener;
    private readonly byte[] _key;
    private readonly AppSettings _settings;
    private readonly Func<HttpIntraserviceClient?> _client;
    private readonly Func<int?, CancellationToken, Task<IReadOnlyList<BridgeCard>>> _board;
    private readonly string _version;
    private readonly CancellationTokenSource _stop = new();

    /// <param name="settings">общий экземпляр настроек: адрес Интрасервиса для ссылок читается при каждом запросе</param>
    /// <param name="client">текущий клиент API — после смены настроек он другой, поэтому функция, а не значение</param>
    /// <param name="board">снимок доски (null) или карточки с этим номером; вызывается из пула потоков,
    /// собирать — в UI-потоке</param>
    public ClaudeBridge(int port, string key, AppSettings settings, Func<HttpIntraserviceClient?> client,
        Func<int?, CancellationToken, Task<IReadOnlyList<BridgeCard>>> board, string version)
    {
        _listener = new TcpListener(IPAddress.Loopback, port);   // неверный порт — ArgumentOutOfRangeException
        _key = Encoding.UTF8.GetBytes(key);
        _settings = settings;
        _client = client;
        _board = board;
        _version = version;
    }

    /// <summary>Порт, который реально слушаем (в самопроверке просим 0 — любой свободный).</summary>
    public int Port => ((IPEndPoint)_listener.LocalEndpoint).Port;

    /// <summary>Ссылка для браузера. Ключ — во фрагменте: он не уходит ни на сервер, ни в Referer; страница забирает его
    /// себе и стирает из адресной строки.</summary>
    public static string LinkFor(int port, string key) => $"http://127.0.0.1:{port}/#{key}";

    public static string NewKey() => Convert.ToHexString(RandomNumberGenerator.GetBytes(16)).ToLowerInvariant();

    /// <summary>Начать слушать. Порт занят — SocketException наружу: App покажет его в настройках и в трее.</summary>
    public void Start()
    {
        _listener.Start();
        _ = AcceptLoop();
    }

    public void Dispose()
    {
        _stop.Cancel();
        _listener.Stop();
    }

    private async Task AcceptLoop()
    {
        while (!_stop.IsCancellationRequested)
        {
            TcpClient connection;
            try { connection = await _listener.AcceptTcpClientAsync(_stop.Token).ConfigureAwait(false); }
            catch (Exception) when (_stop.IsCancellationRequested) { return; }
            // клиент ушёл, пока его принимали, — обычное дело; любая другая ошибка сокета повторилась бы на каждом
            // витке и крутила цикл вхолостую — лучше остановиться и оставить след в логе
            catch (SocketException ex) when (ex.SocketErrorCode is SocketError.ConnectionReset or SocketError.ConnectionAborted) { continue; }
            catch (Exception ex) { Log?.Invoke($"ClaudeBridge остановился: {ex}"); return; }
            _ = Serve(connection);
        }
    }

    private async Task Serve(TcpClient connection)
    {
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
            cts.CancelAfter(TimeSpan.FromSeconds(60));   // два запроса к Интрасервису по 10 с — с запасом
            var stream = connection.GetStream();
            var head = await ReadHead(stream, cts.Token).ConfigureAwait(false);
            var reply = head is null || ParseHead(head) is not { } request
                ? Text(400, "Не понял запрос")
                : await Route(request, cts.Token).ConfigureAwait(false);
            await Write(stream, reply, cts.Token).ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is IOException or SocketException or OperationCanceledException or ObjectDisposedException) { }
        catch (Exception ex) { Log?.Invoke($"ClaudeBridge: {ex}"); }   // из пула потоков — иначе исключение пропало бы молча
        finally { connection.Dispose(); }
    }

    // ---------- HTTP ----------

    /// <summary>Разобранный запрос: метод, путь без query, параметры (раскодированы) и заголовки (имя — без учёта регистра).</summary>
    internal sealed record Request(string Method, string Path, IReadOnlyDictionary<string, string> Query,
        IReadOnlyDictionary<string, string> Headers);

    private sealed record Reply(int Code, string Type, byte[] Body);

    /// <summary>Строка запроса и заголовки до пустой строки; не HTTP/1.x или путь не с «/» — null.</summary>
    internal static Request? ParseHead(string head)
    {
        var lines = head.Split("\r\n");
        var first = lines[0].Split(' ');
        if (first.Length != 3 || !first[1].StartsWith('/') || !first[2].StartsWith("HTTP/1.", StringComparison.Ordinal)) return null;

        var target = first[1];
        var q = target.IndexOf('?');
        var query = new Dictionary<string, string>(StringComparer.Ordinal);
        if (q >= 0)
            foreach (var pair in target[(q + 1)..].Split('&', StringSplitOptions.RemoveEmptyEntries))
            {
                var eq = pair.IndexOf('=');
                query[Unescape(eq < 0 ? pair : pair[..eq])] = eq < 0 ? "" : Unescape(pair[(eq + 1)..]);
            }

        var headers = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var line in lines.Skip(1))
        {
            var colon = line.IndexOf(':');
            if (colon > 0) headers[line[..colon].Trim()] = line[(colon + 1)..].Trim();
        }
        return new(first[0], q < 0 ? target : target[..q], query, headers);
    }

    private static string Unescape(string s) => Uri.UnescapeDataString(s.Replace('+', ' '));

    private static async Task<string?> ReadHead(Stream stream, CancellationToken ct)
    {
        var buf = new byte[MaxHead];
        var n = 0;
        while (n < buf.Length)
        {
            var read = await stream.ReadAsync(buf.AsMemory(n), ct).ConfigureAwait(false);
            if (read == 0) return null;
            n += read;
            var end = buf.AsSpan(0, n).IndexOf("\r\n\r\n"u8);
            if (end >= 0) return Encoding.Latin1.GetString(buf, 0, end);   // query закодирован %XX — байты как есть
        }
        return null;   // заголовки длиннее 16 КБ — не наш клиент
    }

    private static async Task Write(Stream stream, Reply reply, CancellationToken ct)
    {
        var head = $"HTTP/1.1 {reply.Code} {Reason(reply.Code)}\r\n" +
            $"Content-Type: {reply.Type}\r\nContent-Length: {reply.Body.Length}\r\n" +
            "Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n" +
            $"Content-Security-Policy: {Csp}\r\n" + (reply.Code == 405 ? "Allow: GET\r\n" : "") +
            "Connection: close\r\n\r\n";
        await stream.WriteAsync(Encoding.ASCII.GetBytes(head), ct).ConfigureAwait(false);
        await stream.WriteAsync(reply.Body, ct).ConfigureAwait(false);
    }

    private static string Reason(int code) => code switch
    {
        200 => "OK", 400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden", 404 => "Not Found",
        405 => "Method Not Allowed", _ => "Error",
    };

    private static Reply Text(int code, string text) => new(code, "text/plain; charset=utf-8", Encoding.UTF8.GetBytes(text));

    private static Reply JsonReply(int code, object value) =>
        new(code, "application/json; charset=utf-8", JsonSerializer.SerializeToUtf8Bytes(value, Json));

    // ---------- маршруты ----------

    private async Task<Reply> Route(Request r, CancellationToken ct)
    {
        if (!HostMatches(r.Headers.GetValueOrDefault("Host"), Port)) return Text(403, "Чужой адрес");
        if (r.Method != "GET") return Text(405, "Только GET");
        if (!r.Path.StartsWith("/api/", StringComparison.Ordinal)) return Asset(r.Path);
        if (!KeyMatches(r.Headers.GetValueOrDefault("X-Bridge-Key")))
            return JsonReply(401, new { error = "Нет ключа моста или он сменился: откройте ссылку из настроек TicketBoard заново" });

        return r.Path switch
        {
            // адрес Интрасервиса нужен странице: ссылки в ответе Claude она делает кликабельными только на него
            "/api/info" => JsonReply(200, new
            {
                app = "TicketBoard", version = _version, intraservice = _client() is not null,
                intraserviceUrl = _settings.IntraserviceBaseUrl.Trim().TrimEnd('/'),
            }),
            "/api/search" => await SearchReply(r.Query.GetValueOrDefault("q") ?? "", ct).ConfigureAwait(false),
            "/api/ticket" => await TicketReply(r.Query, ct).ConfigureAwait(false),
            "/api/history" => await HistoryReply(r.Query, ct).ConfigureAwait(false),
            "/api/board" => await BoardReply(ct).ConfigureAwait(false),
            _ => JsonReply(404, new { error = "Нет такого метода" }),
        };
    }

    internal static bool HostMatches(string? host, int port) =>
        string.Equals(host, $"127.0.0.1:{port}", StringComparison.OrdinalIgnoreCase) ||
        string.Equals(host, $"localhost:{port}", StringComparison.OrdinalIgnoreCase) ||
        port == 80 && (string.Equals(host, "127.0.0.1", StringComparison.OrdinalIgnoreCase) ||   // порт 80 браузер в Host не пишет
                       string.Equals(host, "localhost", StringComparison.OrdinalIgnoreCase));

    private bool KeyMatches(string? key) => key is not null && CryptographicOperations.FixedTimeEquals(Encoding.UTF8.GetBytes(key), _key);

    private static Reply Asset(string path)
    {
        if (!Assets.TryGetValue(path, out var name)) return Text(404, "Нет такой страницы");
        using var s = typeof(ClaudeBridge).Assembly.GetManifestResourceStream("Bridge/" + name);
        if (s is null) return Text(404, "Нет такой страницы");
        using var m = new MemoryStream();
        s.CopyTo(m);
        var type = Path.GetExtension(name) switch
        {
            ".html" => "text/html; charset=utf-8",
            ".css" => "text/css; charset=utf-8",
            _ => "text/javascript; charset=utf-8",
        };
        return new(200, type, m.ToArray());
    }

    private static readonly Reply NotConfigured = JsonReply(200, new { error = "API Интрасервиса не настроен: трей → Настройки" });
    private static readonly Reply BadId = JsonReply(400, new { error = "Нужен номер заявки: id — целое больше нуля" });

    private async Task<Reply> SearchReply(string text, CancellationToken ct)
    {
        text = text.Trim();
        if (text.Length is 0 or > 200) return JsonReply(400, new { error = "Нужна строка поиска, до 200 символов" });
        if (_client() is not { } client) return NotConfigured;
        var r = await client.SearchAsync(text, ct).ConfigureAwait(false);
        if (r.Error.Length > 0) return JsonReply(200, new { error = r.Error });
        return JsonReply(200, new
        {
            total = r.Total,
            shown = r.Found.Count,
            tickets = r.Found.Select(f => new
            {
                id = f.Id, title = f.Name, status = f.Status, creator = f.Creator, created = f.Created,
                description = Cut(f.Description, 400), url = _settings.TicketUrl(f.Id),
            }),
        });
    }

    /// <summary>Доска как её видит пользователь: «Готово» старше HideDoneOlderThanDays — только счётчиком (их находит
    /// поиск). Длинные описания и заметки обрезаны: этот ответ Claude пересылает с каждым следующим запросом хода.</summary>
    private async Task<Reply> BoardReply(CancellationToken ct)
    {
        var all = await _board(null, ct).ConfigureAwait(false);
        var hideBefore = DateTimeOffset.Now.AddDays(-_settings.HideDoneOlderThanDays);
        var shown = all.Where(c => c.CompletedAt is not { } done || done >= hideBefore).ToList();
        return JsonReply(200, new
        {
            cards = shown.Select(c => c with
            {
                Description = Cut(c.Description, 300) ?? "",
                Notes = c.Notes.Select(n => n with { Text = Cut(n.Text, 500) ?? "" }).ToList(),
            }),
            hiddenDone = all.Count - shown.Count,
            hiddenDoneOlderThanDays = _settings.HideDoneOlderThanDays,
        });
    }

    private async Task<Reply> TicketReply(IReadOnlyDictionary<string, string> query, CancellationToken ct)
    {
        if (!TryId(query, out var id)) return BadId;
        var card = (await _board(id, ct).ConfigureAwait(false)).FirstOrDefault();
        if (_client() is not { } client)
            return card is null ? NotConfigured : JsonReply(200, new { id, board = card, error = "API Интрасервиса не настроен — есть только карточка на доске" });
        var r = await client.GetTaskAsync(id, ct).ConfigureAwait(false);
        return r.Task is { } t
            ? JsonReply(200, new { id = t.Id, title = t.Name, status = t.Status, description = t.Description, url = _settings.TicketUrl(t.Id), board = card })
            : JsonReply(200, new { id, error = r.Error, board = card });
    }

    private async Task<Reply> HistoryReply(IReadOnlyDictionary<string, string> query, CancellationToken ct)
    {
        if (!TryId(query, out var id)) return BadId;
        if (_client() is not { } client) return NotConfigured;
        var r = await client.GetLifetimeAsync(id, ct).ConfigureAwait(false);
        if (r.Error.Length > 0) return JsonReply(200, new { id, error = r.Error });
        return JsonReply(200, new
        {
            id,
            hasMore = r.HasMore,
            events = r.Events.Select(e => new
            {
                date = e.Date, author = e.Author, status = e.Status, comment = Cut(e.Comment, 4000), isPublic = e.IsPublic,
            }),
        });
    }

    private static bool TryId(IReadOnlyDictionary<string, string> query, out int id) =>
        int.TryParse(query.GetValueOrDefault("id"), NumberStyles.None, CultureInfo.InvariantCulture, out id) && id > 0;

    /// <summary>Длинный текст — с явной пометкой, что обрезан: Claude должен знать, что видит не всё.</summary>
    private static string? Cut(string? text, int max) =>
        text is null || text.Length <= max ? text : $"{text[..max]}… [обрезано, всего {text.Length} символов]";
}
