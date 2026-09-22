using System.Diagnostics;
using System.IO;
using System.Net.Sockets;
using System.Text;

namespace TicketBoard.Services;

public sealed partial class ClaudeBridge
{
    /// <summary>Самопроверка моста: разбор запроса и живой обмен по loopback — страница без ключа, API только с ключом
    /// и только с нашим Host. Запускается только из TicketBoard.SelfCheck: открывать сокет при старте приложения незачем.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var r = ParseHead("GET /api/search?q=%D0%BF%D1%80%D0%B8%D0%BD%D1%82%D0%B5%D1%80+hp&x HTTP/1.1\r\nHost: 127.0.0.1:1\r\nx-bridge-key: k");
        Debug.Assert(r is { Method: "GET", Path: "/api/search" } && r.Query["q"] == "принтер hp" && r.Query["x"] == ""
            && r.Headers["X-Bridge-Key"] == "k" && r.Headers["host"] == "127.0.0.1:1");
        Debug.Assert(ParseHead("GET api HTTP/1.1") is null);
        Debug.Assert(ParseHead("GET / SPDY/3") is null);
        Debug.Assert(ParseHead("garbage") is null);
        Debug.Assert(Cut("абвгд", 3) == "абв… [обрезано, всего 5 символов]" && Cut("аб", 3) == "аб" && Cut(null, 3) is null);

        var settings = new AppSettings { IntraserviceBaseUrl = "https://hd.example/" };
        Debug.Assert(settings.TicketUrl(5) == "https://hd.example/Task/View/5" && new AppSettings().TicketUrl(5) == "");
        var card = new BridgeCard(5, "Принтер", "В работе", "средний", "Открыта", 2, "u/5", "", new[] { new BridgeNote(DateTimeOffset.UnixEpoch, "позвонить") });
        using var bridge = new ClaudeBridge(0, "k", settings, () => null,
            () => Task.FromResult<IReadOnlyList<BridgeCard>>(new[] { card }), "t");
        bridge.Start();
        var host = $"127.0.0.1:{bridge.Port}";

        string Get(string target, string hostHeader, string? key = null, string method = "GET")
        {
            using var c = new TcpClient();
            c.Connect("127.0.0.1", bridge.Port);
            var s = c.GetStream();
            s.Write(Encoding.ASCII.GetBytes($"{method} {target} HTTP/1.1\r\nHost: {hostHeader}\r\n" +
                (key is null ? "" : $"X-Bridge-Key: {key}\r\n") + "\r\n"));
            return new StreamReader(s, Encoding.UTF8).ReadToEnd();
        }

        var page = Get("/", host);
        Debug.Assert(page.StartsWith("HTTP/1.1 200") && page.Contains("text/html") && page.Contains("Content-Security-Policy")
            && page.Contains("app.js"));
        Debug.Assert(Get("/anthropic-sdk.mjs", host).Contains("text/javascript"));
        Debug.Assert(Get("/nope", host).StartsWith("HTTP/1.1 404"));
        Debug.Assert(Get("/api/info", host).StartsWith("HTTP/1.1 401"));                              // без ключа
        Debug.Assert(Get("/api/info", host, "K").StartsWith("HTTP/1.1 401"));                         // чужой ключ
        Debug.Assert(Get("/api/info", $"evil.example:{bridge.Port}", "k").StartsWith("HTTP/1.1 403"));  // DNS rebinding
        Debug.Assert(Get("/", $"evil.example:{bridge.Port}").StartsWith("HTTP/1.1 403"));
        Debug.Assert(Get("/api/info", host, "k", "POST").StartsWith("HTTP/1.1 405"));
        Debug.Assert(Get("/api/info", host, "k").Contains("\"intraserviceUrl\":\"https://hd.example\""));

        var board = Get("/api/board", $"localhost:{bridge.Port}", "k");
        Debug.Assert(board.StartsWith("HTTP/1.1 200") && board.Contains("\"title\":\"Принтер\"")
            && board.Contains("\"notes\":[{\"date\":\"1970-01-01T00:00:00+00:00\",\"text\":\"позвонить\"}]"));
        Debug.Assert(Get("/api/search?q=x", host, "k").Contains("не настроен"));
        Debug.Assert(Get("/api/search?q=", host, "k").StartsWith("HTTP/1.1 400"));
        Debug.Assert(Get("/api/ticket?id=5", host, "k").Contains("\"board\":{"));   // API нет, но карточка на доске есть
        Debug.Assert(Get("/api/ticket?id=-1", host, "k").StartsWith("HTTP/1.1 400"));
        Debug.Assert(Get("/api/history?id=5", host, "k").Contains("не настроен"));
        Debug.Assert(Get("/api/nope", host, "k").StartsWith("HTTP/1.1 404"));
    }
}
