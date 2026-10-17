using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    /// <summary>Самопроверка вложений в базе (этап 5а): настоящий клиент против поддельного сервера, который отдаёт файлы.
    /// 601 — моя, три вложения (картинка, имя с запрещёнными знаками, крупный — в files-lfs); 602 — чужая; 603 — моя, файла
    /// на сервере нет (потом появляется). Дальше — повтор без изменений, правка заявки, убранное и переименованное вложение,
    /// настройки «всех» и «никаких», сбои посреди файла, «Остановить», длинный путь, просмотр.</summary>
    private static void SelfCheckFiles()
    {
        var checks = new List<string>();
        void Check(bool ok, string what) { if (!ok) checks.Add(what); }
        var server = new FilesServer();
        var (listener, port) = FakeIntraservice.Start(server.Respond, server.File);
        var dir = Path.Combine(Path.GetTempPath(), $"tb-files-{Guid.NewGuid():N}");
        var (lfsFrom, delays, idle) = (LfsFrom, RetryDelays, HttpIntraserviceClient.FileIdleTimeout);
        LfsFrom = 1500;   // 9003 — ровно 1500 байт: порог включительно
        RetryDelays = new[] { TimeSpan.Zero, TimeSpan.Zero, TimeSpan.Zero };
        HttpIntraserviceClient.FileIdleTimeout = TimeSpan.FromMilliseconds(300);
        try
        {
            Directory.CreateDirectory(dir);
            File.WriteAllText(Path.Combine(dir, ".gitattributes"), "*.psd binary\n");   // строки пользователя
            server.AttributesHaveRule = () => File.ReadAllText(Path.Combine(dir, ".gitattributes")).Contains(LfsRule);
            var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "user", "pass");
            var me = new IntraserviceUser(7, "Я Сам");
            var scope = AttachmentScope.Mine;
            var progress = new ConcurrentQueue<string>();
            ExportResult Export(CancellationToken ct, params int[] ids) => ExportRowsAsync(client, ids.Select(server.Row).ToList(), dir,
                id => $"https://hd/Task/View/{id}", new SyncProgress(progress.Enqueue), ct,
                context: new ExportContext(me, new HashSet<string>(), scope)).GetAwaiter().GetResult();
            ExportResult Run(params int[] ids) => Export(CancellationToken.None, ids);
            string Plain(int ticket, string name) => Path.Combine(dir, FilesFolder, "2026-09", $"{ticket}", name);
            string Large(int ticket, string name) => Path.Combine(dir, LfsFolder, "2026-09", $"{ticket}", name);
            string Md(int ticket) => File.ReadAllText(Directory.GetFiles(Path.Combine(dir, TicketsFolder, "2026-09"), $"{ticket} — *.md").Single(), Encoding.UTF8);
            bool Same(string path, int fileId) => File.Exists(path) && File.ReadAllBytes(path).SequenceEqual(server.Bodies[fileId]);
            bool Temps() => Directory.Exists(Path.Combine(dir, FilesFolder)) && Directory.EnumerateFiles(Path.Combine(dir, FilesFolder), "*.tmp", SearchOption.AllDirectories).Any();

            // 1. первый заход: у моей 601 — три файла побайтно, крупный — в files-lfs, правило LFS записано раньше, чем он
            // запрошен; чужую 602 не качаем; у 603 файла на сервере нет — файл заявки без revision, заявка не «готова»
            var first = Run(601, 602, 603);
            Check(Same(Plain(601, "9001 — ошибка.png"), 9001) && Same(Plain(601, "9002 — Акт сверка.pdf"), 9002)
                && Same(Large(601, "9003 — большой.zip"), 9003) && !File.Exists(Plain(601, "9003 — большой.zip")),
                "601: файлы на местах побайтно");
            Check(server.RuleBeforeLarge == true, $"правило LFS — до крупного файла: {server.RuleBeforeLarge}");
            var attributes = File.ReadAllText(Path.Combine(dir, ".gitattributes"));
            Check(attributes == "*.psd binary\n" + LfsRule + "\n", attributes);
            Check(!server.Asked.Contains(9101) && Md(602).Contains("\n- чужое.docx · не скачано\n"), "602 чужая — не качаем");
            var md601 = Md(601);
            Check(md601.Contains("""

                ## Вложения

                - ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б
                - [[files/2026-09/601/9002 — Акт сверка.pdf]] · 500 Б
                - [[files-lfs/2026-09/601/9003 — большой.zip]] · 1,5 КБ

                ## Переписка
                """.Replace("\r", "")) && md601.Contains("\nfiles: 3\n") && md601.Contains("\nrevision: ") && md601.Contains($"\nformat: {FormatVersion}\n"), md601);
            var md603 = Md(603);
            Check(!md603.Contains("\nrevision: ") && md603.Contains("\n- нет на сервере.txt · не скачалось: файл не найден (HTTP 404)\n"), md603);
            Check(first.Current.SetEquals(new[] { 601, 602 }) && first.Errors.TryGetValue(603, out var e603)
                && e603.StartsWith("файл не найден (HTTP 404)") && e603.EndsWith("(вложение «нет на сервере.txt»)") && first.Failed == 0,
                $"{string.Join(",", first.Current)} {string.Join(" | ", first.Errors)}");
            Check(first.Error.Contains("Вложения: скачано 3 (2,2 КБ), крупных — в files-lfs (Git LFS): 1, не скачано (настройка «Вложения в базу знаний»): 1; "
                + "не скачались 1 — повторная выгрузка попробует их снова: #603 «нет на сервере.txt»: файл не найден (HTTP 404).")
                && first.Error.Contains("Вложения в базе: files — 800 Б, files-lfs — 1,5 КБ."), first.Error);
            Check(progress.Any(line => line.Contains(" · вложения ")), string.Join(" / ", progress));
            Check(!Temps(), "временных не осталось");

            // 2. файл 603 появился на сервере: перечитана только 603 (без revision), её файл скачан; потом — ни одного запроса
            server.Missing.Remove(9201);
            server.Asked.Clear();
            var second = Run(601, 602, 603);
            Check(second.Unchanged == 2 && second.Updated == 1 && server.Asked.SequenceEqual(new[] { 9201 }) && Md(603).Contains("\nrevision: ")
                && second.Current.SetEquals(new[] { 601, 602, 603 }) && File.Exists(Plain(603, "9201 — нет на сервере.txt")), $"второй заход: {string.Join(",", server.Asked)}");
            server.Asked.Clear();
            var third = Run(601, 602, 603);
            Check(third.Unchanged == 3 && server.Asked.IsEmpty && attributes == File.ReadAllText(Path.Combine(dir, ".gitattributes")), "третий — ничего");

            // 3. 601 изменилась, вложения те же — карточка перечитана, файлы не качаются; оборванная закачка старше десяти
            // минут убрана, свежая (может, её ещё пишут) — нет
            server.Touch(601);
            var (staleTemp, freshTemp) = (Plain(601, "9001 — ошибка.png.ab12.tmp"), Plain(601, "9001 — ошибка.png.cd34.tmp"));
            File.WriteAllText(staleTemp, "");
            File.SetLastWriteTimeUtc(staleTemp, DateTime.UtcNow.AddMinutes(-20));
            File.WriteAllText(freshTemp, "");
            var touched = Run(601);
            Check(touched.Updated == 1 && server.Asked.IsEmpty && Md(601).Contains("9003 — большой.zip"), "изменилась — файлы не качаются");
            Check(!File.Exists(staleTemp) && File.Exists(freshTemp), "старая оборванная закачка убрана, свежая — нет");
            File.Delete(freshTemp);

            // 4. вложение 9002 убрали с заявки — его файл удалён; файл пользователя в той же папке цел; 9001 лежит под прежним
            // именем — переименован, а не скачан заново
            Directory.CreateDirectory(Path.GetDirectoryName(Plain(601, "x"))!);
            File.WriteAllText(Plain(601, "мои заметки.txt"), "моё");
            if (File.Exists(Plain(601, "9001 — ошибка.png"))) File.Move(Plain(601, "9001 — ошибка.png"), Plain(601, "9001 — старое имя.png"));
            server.Cards[601] = server.Cards[601] with { Ids = "9001, 9003", Names = "ошибка.png||большой.zip" };
            server.Touch(601);
            Run(601);
            Check(!File.Exists(Plain(601, "9002 — Акт сверка.pdf")) && File.Exists(Plain(601, "мои заметки.txt"))
                && File.Exists(Plain(601, "9001 — ошибка.png")) && !File.Exists(Plain(601, "9001 — старое имя.png"))
                && File.Exists(Large(601, "9003 — большой.zip")) && server.Asked.IsEmpty && !Md(601).Contains("9002"), "убрали и переименовали");
            // карточка не прислала FileIds — ничего не удаляем
            server.Cards[601] = server.Cards[601] with { Ids = null };
            server.Touch(601);
            Run(601);
            Check(File.Exists(Plain(601, "9001 — ошибка.png")) && File.Exists(Large(601, "9003 — большой.zip")), "без FileIds — не удаляем");
            server.Cards[601] = server.Cards[601] with { Ids = "9001, 9003" };

            // 5. «всех» — чужая скачана; «никаких» — новая не скачана, а лежащий файл — по-прежнему со ссылкой
            scope = AttachmentScope.All;
            server.Touch(602);
            Run(602);
            Check(Same(Plain(602, "9101 — чужое.docx"), 9101), "«всех» — скачано");
            scope = AttachmentScope.None;
            server.Touch(601);
            server.Touch(604);
            var none = Run(601, 604);
            Check(!server.Asked.Contains(9301) && Md(604).Contains("\n- журнал.txt · не скачано\n")
                && Md(601).Contains("- ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б\n") && !none.Error.Contains("Вложения в базе"), "«никаких»");
            scope = AttachmentScope.Mine;

            // 6. 503 — повтор и успех; обрыв посреди файла и тишина посреди файла — ошибка, ни файла, ни временного
            server.FailOnce[9301] = 1;
            server.Touch(604);
            Run(604);
            Check(Same(Plain(604, "9301 — журнал.txt"), 9301)
                && server.Asked.Count(id => id == 9301) == 2, $"503 и повтор: {string.Join(",", server.Asked)}");
            var broken = Run(605);
            Check(!File.Exists(Plain(605, "9401 — обрыв.bin")) && !File.Exists(Plain(605, "9402 — тишина.bin")) && !Temps()
                && broken.Errors.TryGetValue(605, out var e605) && e605.StartsWith("соединение оборвалось"), $"обрыв: {string.Join(" | ", broken.Errors)}");
            var md605 = Md(605);
            Check(md605.Contains("\n- обрыв.bin · не скачалось: соединение оборвалось") && md605.Contains("\n- тишина.bin · не скачалось: сервер не ответил за "), md605);

            // 7. «Остановить» посреди файла — ни файла, ни временного, файл заявки не записан
            using (var stop = new CancellationTokenSource())
            {
                server.OnSent = stop.Cancel;
                var stopped = Export(stop.Token, 606);
                server.OnSent = null;
                Check(!stopped.Complete && !stopped.Current.Contains(606) && !File.Exists(Plain(606, "9501 — долгий.bin")) && !Temps()
                    && !Directory.GetFiles(Path.Combine(dir, TicketsFolder, "2026-09"), "606 — *.md").Any(), "остановили посреди файла");
            }

            // 8. путь у предела MAX_PATH — имя укорочено, файл записан; следующая выгрузка (заявка изменилась) его не качает
            var deep = Path.Combine(dir, new string('d', 150));
            var deepRun = ExportRowsAsync(client, new[] { server.Row(607) }, deep, id => "", null, CancellationToken.None,
                context: new ExportContext(me, new HashSet<string>(), AttachmentScope.Mine)).GetAwaiter().GetResult();
            var deepFile = Directory.Exists(Path.Combine(deep, FilesFolder)) ? Directory.GetFiles(Path.Combine(deep, FilesFolder), "9601*", SearchOption.AllDirectories).SingleOrDefault() : null;
            Check(deepRun.Current.Contains(607) && deepFile is not null && Path.Combine(deep, LfsFolder, "2026-09", "607", Path.GetFileName(deepFile)).Length <= 250
                && deepFile.EndsWith(".png"), $"длинный путь: {deepFile}");
            server.Asked.Clear();
            server.Touch(607);
            ExportRowsAsync(client, new[] { server.Row(607) }, deep, id => "", null, CancellationToken.None,
                context: new ExportContext(me, new HashSet<string>(), AttachmentScope.Mine)).GetAwaiter().GetResult();
            Check(server.Asked.IsEmpty, "длинный путь — не качаем снова");

            // 9. .gitattributes не записать (на его месте папка) — мелкие файлы ложатся, крупный — нет, и итог говорит почему
            var noRule = Path.Combine(dir, "без правила");
            Directory.CreateDirectory(Path.Combine(noRule, ".gitattributes"));
            server.Touch(601);
            var blocked = ExportRowsAsync(client, new[] { server.Row(601) }, noRule, id => "", null, CancellationToken.None,
                context: new ExportContext(me, new HashSet<string>(), AttachmentScope.Mine)).GetAwaiter().GetResult();
            var blockedMd = File.ReadAllText(Directory.GetFiles(Path.Combine(noRule, TicketsFolder, "2026-09"), "601 — *.md").Single(), Encoding.UTF8);
            Check(File.Exists(Path.Combine(noRule, FilesFolder, "2026-09", "601", "9001 — ошибка.png"))
                && !File.Exists(Path.Combine(noRule, LfsFolder, "2026-09", "601", "9003 — большой.zip"))
                && blockedMd.Contains("\n- большой.zip · не скачалось: крупный файл (1,5 КБ) не сохранён: не удалось дописать правило Git LFS в ")
                && blocked.Error.Contains("крупные файлы (от 1,5 КБ) не сохраняются, пока в .gitattributes нет строки"), $"{blocked.Error}\n{blockedMd}");

            // 10. просмотр (и буфер, и запрос агенту) — только имена, ни одного запроса файла
            server.Asked.Clear();
            var (preview, previewError) = BuildAsync(client, server.Row(601), "https://hd/Task/View/601", CancellationToken.None).GetAwaiter().GetResult();
            Check(previewError == "" && preview.Contains("\n## Вложения\n\n- ошибка.png\n- большой.zip\n") && server.Asked.IsEmpty, preview);
        }
        finally
        {
            (LfsFrom, RetryDelays, HttpIntraserviceClient.FileIdleTimeout) = (lfsFrom, delays, idle);
            listener.Stop();
            try { Directory.Delete(dir, recursive: true); } catch (IOException) { }
        }
        Debug.Assert(checks.Count == 0, "вложения:\n" + string.Join("\n", checks));
    }

    /// <summary>Поддельный сервер с вложениями: карточки (номера и имена файлов), короткая история, сами файлы. Missing — файлы,
    /// которых нет (404), FailOnce — сколько раз ответить 503, Asked — какие файлы спрашивали.</summary>
    private sealed class FilesServer
    {
        public sealed record Card(string Name, string Executors, string? Ids, string Names, string Changed);

        public readonly ConcurrentDictionary<int, Card> Cards = new()
        {
            [601] = new("Скриншот ошибки", "Я Сам", "9001, 9002, 9003", "ошибка.png||Акт: сверка.pdf||большой.zip", "2026-09-05T10:00:00.250000"),
            [602] = new("Чужая заявка", "Петров", "9101", "чужое.docx", "2026-09-06T10:00:00"),
            [603] = new("Битое вложение", "Я Сам", "9201", "нет на сервере.txt", "2026-09-07T10:00:00"),
            [604] = new("Журнал", "Я Сам", "9301", "журнал.txt", "2026-09-08T10:00:00"),
            [605] = new("Сбои посреди файла", "Я Сам", "9401, 9402", "обрыв.bin||тишина.bin", "2026-09-09T10:00:00"),
            [606] = new("Долгий файл", "Я Сам", "9501", "долгий.bin", "2026-09-10T10:00:00"),
            [607] = new("Длинное имя", "Я Сам", "9601", new string('я', 120) + ".png", "2026-09-11T10:00:00"),
        };

        public readonly Dictionary<int, byte[]> Bodies = new[] { 9001, 9002, 9003, 9101, 9201, 9301, 9401, 9402, 9501, 9601 }
            .ToDictionary(id => id, id => Enumerable.Range(0, id switch { 9001 => 300, 9002 => 500, 9003 => 1500, _ => 600 })
                .Select(i => (byte)((i * 7 + id) % 251)).ToArray());
        public readonly HashSet<int> Missing = new() { 9201 };
        public readonly ConcurrentDictionary<int, int> FailOnce = new();
        public readonly ConcurrentQueue<int> Asked = new();
        public Action? OnSent;
        /// <summary>Было ли правило LFS в .gitattributes, когда спросили крупный файл (null — не спрашивали).</summary>
        public bool? RuleBeforeLarge;

        public void Touch(int id) => Cards[id] = Cards[id] with { Changed = Parse(Cards[id].Changed).AddMinutes(1).ToString("yyyy-MM-dd'T'HH:mm:ss.ffffff", CultureInfo.InvariantCulture) };

        private static DateTime Parse(string date) => DateTime.Parse(date, CultureInfo.InvariantCulture);

        public IntraserviceFound Row(int id) => new(id, Cards[id].Name, "Выполнена", "Петрова", new DateTimeOffset(2026, 9, 1, 9, 0, 0, TimeSpan.FromHours(3)),
            "описание", Cards[id].Executors, Changed: new DateTimeOffset(Parse(Cards[id].Changed), TimeSpan.FromHours(3)));

        public FakeFile? File(string target)
        {
            if (!target.StartsWith("/api/taskfile/") || !int.TryParse(target["/api/taskfile/".Length..], out var id)) return null;
            Asked.Enqueue(id);
            if (Missing.Contains(id) || FailOnce.TryGetValue(id, out var fails) && fails > 0) return null;   // ответит Respond
            if (id == 9003) RuleBeforeLarge = AttributesHaveRule?.Invoke();
            return id switch
            {
                9401 => new FakeFile(Bodies[id], Cut: 100),
                9402 => new FakeFile(Bodies[id], Cut: 100, Stall: TimeSpan.FromMilliseconds(600)),
                9501 => new FakeFile(Bodies[id], Cut: 100, Stall: TimeSpan.FromSeconds(1), Sent: OnSent),
                _ => new FakeFile(Bodies[id]),
            };
        }

        /// <summary>Есть ли правило LFS в .gitattributes папки выгрузки — ставит проверка.</summary>
        public Func<bool>? AttributesHaveRule;

        public (int Code, string Json) Respond(string target)
        {
            if (target.StartsWith("/api/taskfile/") && int.TryParse(target["/api/taskfile/".Length..], out var fileId))
            {
                if (FailOnce.TryGetValue(fileId, out var fails) && fails > 0)
                {
                    FailOnce[fileId] = fails - 1;
                    return (503, """{"Message":"busy"}""");
                }
                return (404, """{"Message":"not found"}""");
            }
            if (target.StartsWith("/api/task/") && int.TryParse(target["/api/task/".Length..].Split('?')[0], out var id) && Cards.TryGetValue(id, out var c))
                return (200, "{\"Task\":{\"Id\":" + id + ",\"Name\":\"" + c.Name + "\",\"StatusName\":\"Выполнена\",\"ServiceName\":\"Почта\",\"Type\":\"Инцидент\""
                    + (c.Ids is null ? "" : ",\"FileIds\":\"" + c.Ids + "\",\"FileNames\":\"" + c.Names + "\"") + "}}");
            if (target.StartsWith("/api/tasklifetime?taskid="))
                return (200, """{"TaskLifetimeList":{"TaskLifetimes":[{"Date":"2026-09-01T10:00:00","Editor":"Петров","StatusId":29,"Comments":"готово","IsPublic":true}],"Statuses":[{"Id":29,"Name":"Выполнена","IsFixed":true}],"Paginator":{"Page":1,"PageCount":1}}}""");
            return (404, """{"Message":"not found"}""");
        }
    }

    /// <summary>Ход выгрузки — сразу в список, без SynchronizationContext (Progress&lt;T&gt; отдавал бы его в пул, позже).</summary>
    private sealed class SyncProgress(Action<string> report) : IProgress<string>
    {
        public void Report(string value) => report(value);
    }
}
