using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    /// <summary>Самопроверка: имена файлов, экранирование, формат — и живая выгрузка настоящим клиентом против поддельного
    /// Интрасервиса (FakeIntraservice) в три захода: первый, повторный без изменений, после переименования заявки. Даты
    /// списка — от сегодняшнего дня: с «последними 365 днями» неподвижные даты однажды выпали бы из периода.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // имя файла: номер и название без знаков, запрещённых Windows и ломающих ссылки Obsidian
        Debug.Assert(FileName(702180, "Принтер: «HP» / замятие? [срочно] #1") == "702180 — Принтер «HP» замятие срочно 1.md");
        Debug.Assert(FileName(5, " ?? ") == "5.md" && FileName(5, new string('я', 100)) == $"5 — {new string('я', 80)}.md");
        // текст как есть, но не разметка: заголовок, тег Obsidian, черта; номер «#702180» тегом не бывает — не трогаем
        Debug.Assert(Escape("# не заголовок\nтекст #тег и #702180\n---\r\n  ## тоже") ==
            "\\# не заголовок\nтекст \\#тег и #702180\n\\---\n  \\## тоже");

        // отбор: хоть «мои», хоть слова; период и потолок — в пределах (0 дней — за всё время)
        Debug.Assert(Invalid(new()) is null && Invalid(new(Mine: false, Words: "VPN", Days: 0, Limit: MaxLimit)) is null);
        Debug.Assert(Invalid(new(Days: -1)) is not null && Invalid(new(Days: MaxDays + 1)) is not null
            && Invalid(new(Limit: 0)) is not null && Invalid(new(Limit: MaxLimit + 1)) is not null
            && Invalid(new(Status: (ExportStatus)7)) is not null);
        // без «моих» и без слов — это выгрузка всего сервера: не начинаем
        var none = RunAsync(new HttpIntraserviceClient("http://127.0.0.1:1", "u", "p"), new(Mine: false, Words: " "),
            Path.GetTempPath(), new HashSet<string>(), _ => "", null, CancellationToken.None).GetAwaiter().GetResult();
        Debug.Assert(none.Error.Length > 0 && none.Found == 0);

        var now = DateTimeOffset.Now;
        static string Iso(DateTimeOffset d) => d.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture);
        var dates = new Dates(Created501: Iso(now.AddDays(-14)), Recent: Iso(now.AddDays(-30)), Old: Iso(now.AddDays(-800)));
        var title501 = "Принтер не печатает";
        var changed501 = Iso(now.AddDays(-10));
        var (listener, port) = FakeIntraservice.Start(target => Respond(target, title501, changed501, dates));
        var dir = Path.Combine(Path.GetTempPath(), $"tb-export-{Guid.NewGuid():N}");
        try
        {
            var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "user", "pass");
            var closed = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "Закрыта" };
            ExportResult Run() => RunAsync(client, new(Mine: true, Status: ExportStatus.Closed, Days: 365, Limit: 100), dir, closed,
                id => $"https://hd/Task/View/{id}", null, CancellationToken.None).GetAwaiter().GetResult();

            // первый заход: 503 старше периода — не попала; 502 не отдала переписку — в «не прочитались», файла нет;
            // у 504 ровно 50 записей и Paginator «страница 1 из 1» — вторую страницу не просим (её нет — 404 сломал бы)
            var first = Run();
            Debug.Assert(first is { Found: 3, Created: 2, Updated: 0, Unchanged: 0, Failed: 1, Error: "" } && first.FirstError.StartsWith("#502"));
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
            // две записи в одну секунду — в том порядке, в каком их сделали (сервер отдаёт свежие сверху)
            var text504 = File.ReadAllText(Path.Combine(dir, TicketsFolder, "504 — Почта не уходит.md"), Encoding.UTF8);
            Debug.Assert(text504.IndexOf("ответ A", StringComparison.Ordinal) < text504.IndexOf("ответ B", StringComparison.Ordinal)
                && text504.Split("\n### ").Length == 50 + 1);
            var index = File.ReadAllText(Path.Combine(dir, IndexFile), Encoding.UTF8);
            Debug.Assert(index.Contains($"- [[501 — Принтер не печатает]] · {now.AddDays(-14):dd.MM.yyyy} · Выполнена\n") && index.Contains("заявок: 2."));

            // повторный заход без изменений: переписку не перечитываем
            var again = Run();
            Debug.Assert(again is { Found: 3, Created: 0, Updated: 0, Unchanged: 2, Failed: 1 });

            // заявку переименовали и она менялась: файл переписан под новым именем, прежний убран — и тот, что без
            // свойств (испорчен руками), узнаётся по номеру в имени; чужая заметка без номера остаётся
            var broken = Path.Combine(dir, TicketsFolder, "501 — Принтер.md");
            var note = Path.Combine(dir, TicketsFolder, "заметки.md");
            File.WriteAllText(broken, "испорчен");
            File.WriteAllText(note, "моё");
            title501 = "Принтер HP не печатает";
            changed501 = Iso(now.AddDays(-9));
            var renamed = Run();
            Debug.Assert(renamed is { Created: 0, Updated: 1, Unchanged: 1 } && !File.Exists(file) && !File.Exists(broken)
                && File.Exists(note) && File.Exists(Path.Combine(dir, TicketsFolder, "501 — Принтер HP не печатает.md")));

            // запись не удалась (на месте файла — папка) — ошибка наружу, временный файл не остаётся
            var blocked = Path.Combine(dir, "занято.md");
            Directory.CreateDirectory(blocked);
            var threw = false;
            try { WriteAtomic(blocked, "текст"); }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { threw = true; }
            Debug.Assert(threw && !File.Exists(blocked + ".tmp"));
        }
        finally
        {
            listener.Stop();
            try { Directory.Delete(dir, recursive: true); } catch (IOException) { }
        }
    }

    private sealed record Dates(string Created501, string Recent, string Old);

    /// <summary>Ответы поддельного сервера: я — 7, статусы, четыре мои закрытые заявки (503 — старше периода), переписка 501
    /// на двух страницах (51 запись, свежие сверху), 504 — ровно 50 записей на одной странице, 502 — 404.</summary>
    private static (int Code, string Json) Respond(string target, string title501, string changed501, Dates d)
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
                  {"Id":501,"Name":"TITLE","StatusId":29,"Created":"CREATED","Changed":"CHANGED",
                   "Creator":"Петрова А.","CreatorPhone":"+7 999 000-00-00","CreatorEmail":"petrova@example.ru",
                   "Executors":"Иванов И., Я Сам","ExecutorGroup":"Первая линия","ServiceName":"Принтеры","Description":"<p>Замятие в лотке 2</p>"},
                  {"Id":502,"Name":"VPN не подключается","StatusId":30,"Created":"RECENT","Changed":"RECENT"},
                  {"Id":504,"Name":"Почта не уходит","StatusId":29,"Created":"RECENT","Changed":"RECENT"},
                  {"Id":503,"Name":"Старое","StatusId":29,"Created":"OLD","Changed":"OLD"}],
                 "Statuses":[{"Id":29,"Name":"Выполнена"},{"Id":30,"Name":"Закрыта"}],"Paginator":{"Count":4,"Page":1,"PageCount":1}}
                """.Replace("TITLE", title501).Replace("CHANGED", changed501).Replace("CREATED", d.Created501)
                   .Replace("RECENT", d.Recent).Replace("OLD", d.Old));
        }
        const string Statuses = """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}]""";
        static string Lifetime(IEnumerable<string> rows, int page, int pages) =>
            """{"TaskLifetimeList":{"TaskLifetimes":[ROWS],"Statuses":STATUSES,"Paginator":{"Page":PAGE,"PageCount":PAGES}}}"""
                .Replace("ROWS", string.Join(",", rows)).Replace("STATUSES", Statuses)
                .Replace("PAGES", pages.ToString()).Replace("PAGE", page.ToString());
        static string Row(string date, string editor, int status, string comment, bool isPublic) =>
            "{\"Date\":\"" + date + "\",\"Editor\":\"" + editor + "\",\"StatusId\":" + status + ",\"Comments\":\"<p>" + comment
            + "</p>\",\"IsPublic\":" + (isPublic ? "true" : "false") + "}";
        if (target.StartsWith("/api/tasklifetime?taskid=501"))
        {
            // 51 запись: первая страница — 50 свежих (51…2), вторая — самая первая; запись 2 — внутренняя
            var page = target.Contains("&page=2") ? 2 : 1;
            var numbers = page == 1 ? Enumerable.Range(2, 50).Reverse() : new[] { 1 };
            return (200, Lifetime(numbers.Select(n => Row(
                $"2026-09-{(n == 51 ? 20 : 18):00}T{9 + n / 10:00}:{n % 10 * 5:00}:00", n == 51 ? "Иванов И." : "Сидоров",
                n == 51 ? 29 : 31, $"запись {n}", n != 2)), page, 2));
        }
        if (target.StartsWith("/api/tasklifetime?taskid=504"))
        {
            if (target.Contains("&page=")) return (404, """{"Message":"no such page"}""");   // страницы 2 нет
            // ровно 50: две последние — в одну секунду (сервер ставит более позднюю первой), до них — 48 по минуте
            var rows = new[] { Row("2026-09-11T10:00:00", "Сидоров", 29, "ответ B", true), Row("2026-09-11T10:00:00", "Сидоров", 31, "ответ A", true) }
                .Concat(Enumerable.Range(1, 48).Reverse().Select(n => Row($"2026-09-10T{9 + n / 60:00}:{n % 60:00}:00", "Петров", 31, $"строка {n}", true)));
            return (200, Lifetime(rows, 1, 1));
        }
        return (404, """{"Message":"not found"}""");
    }
}
