using System.Diagnostics;
using System.IO;
using System.Text;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    /// <summary>Самопроверка: имена файлов, экранирование, формат — и живая выгрузка настоящим клиентом против поддельного
    /// Интрасервиса (FakeIntraservice) в три захода: первый, повторный без изменений, после переименования заявки.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // имя файла: номер и название без знаков, запрещённых Windows и ломающих ссылки Obsidian
        Debug.Assert(FileName(702180, "Принтер: «HP» / замятие? [срочно] #1") == "702180 — Принтер «HP» замятие срочно 1.md");
        Debug.Assert(FileName(5, " ?? ") == "5.md" && FileName(5, new string('я', 100)) == $"5 — {new string('я', 80)}.md");
        // текст как есть, но не разметка: заголовок, тег Obsidian, черта; номер «#702180» тегом не бывает — не трогаем
        Debug.Assert(Escape("# не заголовок\nтекст #тег и #702180\n---\r\n  ## тоже") ==
            "\\# не заголовок\nтекст \\#тег и #702180\n\\---\n  \\## тоже");

        // без «моих» и без слов — это выгрузка всего сервера: не начинаем
        var none = RunAsync(new HttpIntraserviceClient("http://127.0.0.1:1", "u", "p"), new(Mine: false, Words: " "),
            Path.GetTempPath(), new HashSet<string>(), _ => "", null, CancellationToken.None).GetAwaiter().GetResult();
        Debug.Assert(none.Error.Length > 0 && none.Found == 0);

        var title501 = "Принтер не печатает";
        var changed501 = "2026-09-20T16:40:00";
        var (listener, port) = FakeIntraservice.Start(target => Respond(target, title501, changed501));
        var dir = Path.Combine(Path.GetTempPath(), $"tb-export-{Guid.NewGuid():N}");
        try
        {
            var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "user", "pass");
            var closed = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "Закрыта" };
            ExportResult Run() => RunAsync(client, new(Mine: true, Status: ExportStatus.Closed, Days: 365, Limit: 100), dir, closed,
                id => $"https://hd/Task/View/{id}", null, CancellationToken.None).GetAwaiter().GetResult();

            // первый заход: 503 старше периода — не попала; 502 не отдала переписку — в «не прочитались», файла нет
            var first = Run();
            Debug.Assert(first is { Found: 2, Created: 1, Updated: 0, Unchanged: 0, Failed: 1, Error: "" } && first.FirstError.StartsWith("#502"));
            var file = Path.Combine(dir, TicketsFolder, "501 — Принтер не печатает.md");
            Debug.Assert(File.Exists(file) && !File.Exists(Path.Combine(dir, TicketsFolder, "502 — VPN не подключается.md")));
            var text = File.ReadAllText(file, Encoding.UTF8);
            Debug.Assert(text.StartsWith("---\nid: 501\ntitle: \"Принтер не печатает\"\nstatus: \"Выполнена\"\n"));
            Debug.Assert(text.Contains("\nservice: \"Принтеры\"\n") && text.Contains("\nexecutors: [\"Иванов И.\", \"Я Сам\"]\n")
                && text.Contains("\nurl: \"https://hd/Task/View/501\"\n") && text.Contains($"\nformat: {FormatVersion}\n"));
            Debug.Assert(!text.Contains("+7 999") && !text.Contains("petrova@"));   // контакты людей — не в базу знаний
            Debug.Assert(text.Contains("# 501 · Принтер не печатает\n") && text.Contains("## Описание\n\nЗамятие в лотке 2\n"));
            // вся переписка — обе страницы, от первой записи к последней; внутренний помечен; статус — где поменялся
            var firstNote = text.IndexOf("запись 1\n", StringComparison.Ordinal);
            var lastNote = text.IndexOf("запись 51\n", StringComparison.Ordinal);
            Debug.Assert(firstNote > 0 && lastNote > firstNote && text.Split("\n### ").Length == 51 + 1);   // 51 запись — 51 заголовок
            Debug.Assert(text.Contains("— Сидоров (внутренний)\n\nзапись 2\n") && text.Contains("— Иванов И. · статус «Выполнена»\n"));
            var index = File.ReadAllText(Path.Combine(dir, IndexFile), Encoding.UTF8);
            Debug.Assert(index.Contains("- [[501 — Принтер не печатает]] · 18.09.2026 · Выполнена\n") && index.Contains("заявок: 1."));

            // повторный заход без изменений: переписку 501 не перечитываем
            var again = Run();
            Debug.Assert(again is { Found: 2, Created: 0, Updated: 0, Unchanged: 1, Failed: 1 });

            // заявку переименовали и она менялась: файл переписан под новым именем, прежний убран
            title501 = "Принтер HP не печатает";
            changed501 = "2026-09-21T09:00:00";
            var renamed = Run();
            Debug.Assert(renamed is { Created: 0, Updated: 1, Unchanged: 0 } && !File.Exists(file)
                && File.Exists(Path.Combine(dir, TicketsFolder, "501 — Принтер HP не печатает.md")));
        }
        finally
        {
            listener.Stop();
            try { Directory.Delete(dir, recursive: true); } catch (IOException) { }
        }
    }

    /// <summary>Ответы поддельного сервера: я — 7, статусы, три мои закрытые заявки (503 — двухлетней давности), переписка
    /// 501 на двух страницах (51 запись, свежие сверху), 502 — 404.</summary>
    private static (int Code, string Json) Respond(string target, string title501, string changed501)
    {
        if (target.StartsWith("/api/user?getcurrentuserinfo=true")) return (200, """{"Id":7,"Name":"Я Сам"}""");
        if (target.StartsWith("/api/taskstatus"))
            return (200, """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта"}]""");
        if (target.StartsWith("/api/task?"))
        {
            // отбор дошёл до сервера: мои (7), только закрытые статусы (29 — признак сервера, 30 — по названию из настроек)
            Debug.Assert(target.Contains("ExecutorIds=7&") && target.Contains("StatusIds=29,30&") && !target.Contains("search="));
            return (200, """
                {"Tasks":[
                  {"Id":501,"Name":"TITLE","StatusId":29,"Created":"2026-09-18T09:12:00","Changed":"CHANGED",
                   "Creator":"Петрова А.","CreatorPhone":"+7 999 000-00-00","CreatorEmail":"petrova@example.ru",
                   "Executors":"Иванов И., Я Сам","ExecutorGroup":"Первая линия","ServiceName":"Принтеры","Description":"<p>Замятие в лотке 2</p>"},
                  {"Id":502,"Name":"VPN не подключается","StatusId":30,"Created":"2026-08-01T10:00:00","Changed":"2026-09-01T10:00:00"},
                  {"Id":503,"Name":"Старое","StatusId":29,"Created":"2024-01-01T10:00:00","Changed":"2024-01-02T10:00:00"}],
                 "Statuses":[{"Id":29,"Name":"Выполнена"},{"Id":30,"Name":"Закрыта"}],"Paginator":{"Count":3,"Page":1,"PageCount":1}}
                """.Replace("TITLE", title501).Replace("CHANGED", changed501));
        }
        if (target.StartsWith("/api/tasklifetime?taskid=501"))
        {
            // 51 запись: первая страница — 50 свежих (51…2), вторая — самая первая; запись 2 — внутренняя
            var page = target.Contains("&page=2") ? 2 : 1;
            var numbers = page == 1 ? Enumerable.Range(2, 50).Reverse() : new[] { 1 };
            static string Row(int n) =>
                "{\"Date\":\"2026-09-" + (n == 51 ? "20" : "18") + "T" + (9 + n / 10).ToString("00") + ":" + (n % 10 * 5).ToString("00")
                + ":00\",\"Editor\":\"" + (n == 51 ? "Иванов И." : "Сидоров") + "\",\"StatusId\":" + (n == 51 ? 29 : 31)
                + ",\"Comments\":\"<p>запись " + n + "</p>\",\"IsPublic\":" + (n == 2 ? "false" : "true") + "}";
            return (200, """{"TaskLifetimeList":{"TaskLifetimes":[ROWS],"Statuses":[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}],"Paginator":{"Page":PAGE,"PageCount":2}}}"""
                .Replace("ROWS", string.Join(",", numbers.Select(Row))).Replace("PAGE", page.ToString()));
        }
        return (404, """{"Message":"not found"}""");
    }
}
