using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    /// <summary>Самопроверка выгрузки всех заявок: настоящий клиент против поддельного сервера на 230 заявок, созданных через
    /// каждые шесть часов (четыре месяца), по две страницы списка. Идёт по порядку: всё сразу и по страницам; остановка и
    /// продолжение; ничего не изменилось; изменилась одна; папка старой раскладки; повторы сбойных запросов; сервер лёг,
    /// отказал в логине, не отдаёт ничего; сервер не принял или не соблюл сортировку, не понимает страницы; период, который
    /// сервер не применил; потолок числа заявок; диск не пишет; дубль файла; срок запроса.</summary>
    [Conditional("DEBUG")]
    private static void SelfCheckMass()
    {
        var root = Path.Combine(Path.GetTempPath(), $"tb-mass-{Guid.NewGuid():N}");
        var server = new MassServer();
        var (listener, port) = FakeIntraservice.Start(server.Respond);
        var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "u", "p");
        var delays = RetryDelays;
        RetryDelays = new[] { TimeSpan.FromMilliseconds(1), TimeSpan.FromMilliseconds(1), TimeSpan.FromMilliseconds(1) };
        const int Count = MassServer.Count;
        string Url(int id) => $"https://hd/Task/View/{id}";
        static string Month(int i) => MassServer.CreatedOf(i).ToString("yyyy-MM", CultureInfo.InvariantCulture);
        static string Named(int i) => $"{MassServer.IdOf(i)} — Заявка {MassServer.IdOf(i)}.md";
        ExportResult All(string dir, int limit = 0, TaskQuery? query = null, IProgress<string>? progress = null, CancellationToken ct = default) =>
            RunAsync(client, query ?? new TaskQuery(), limit, dir, Url, progress, ct).GetAwaiter().GetResult();
        try
        {
            // 1. всё сразу, limit 0: две страницы списка по дате создания, файлы по страницам, оглавления
            var dirAll = Path.Combine(root, "all");
            var progress = new Collected();
            var all = All(dirAll, progress: progress);
            Debug.Assert(all is { Found: Count, Created: Count, Updated: 0, Unchanged: 0, Failed: 0, Complete: true, Error: "" });
            Debug.Assert(server.ListTargets.Count == 2 && server.ListTargets[0].Contains("sort=Created%20asc,%20Id%20asc&pagesize=200&page=1")
                && server.ListTargets[0].Contains("count=all") && server.ListTargets[1].EndsWith("&page=2"));
            // выгрузка идёт по страницам, а не после всего списка: вторую страницу просят, когда все 200 заявок первой уже записаны
            var beforePage2 = server.Log.Take(server.Log.IndexOf("L2")).ToList();
            Debug.Assert(beforePage2.Count(x => x[0] == 'C') == 200 && server.Log.Count(x => x[0] == 'C') == Count);
            Debug.Assert(progress.Messages.Any(m => m.Contains($"из {Count}")) && progress.Messages[^1] == $"Выгружено {Count} из {Count}");
            Debug.Assert(OnDisk(dirAll).Count == Count);
            for (var i = 0; i < Count; i++)
                Debug.Assert(File.Exists(Path.Combine(dirAll, TicketsFolder, Month(i), Named(i))));
            var months = Enumerable.Range(0, Count).GroupBy(Month).OrderByDescending(g => g.Key, StringComparer.Ordinal).ToList();
            Debug.Assert(months.Count >= 3);
            var rootIndex = File.ReadAllText(Path.Combine(dirAll, IndexFile));
            Debug.Assert(rootIndex.Contains($"заявок: {Count}."));
            foreach (var m in months)
            {
                Debug.Assert(rootIndex.Contains($"- [{m.Key}](tickets/{m.Key}/_index.md) — заявок: {m.Count()}\n"));
                var monthIndex = File.ReadAllText(Path.Combine(dirAll, TicketsFolder, m.Key, IndexFile));
                Debug.Assert(monthIndex.Contains($"Заявок: {m.Count()} ") && m.All(i => monthIndex.Contains($"[[{Path.GetFileNameWithoutExtension(Named(i))}]]")));
            }
            Debug.Assert(rootIndex.IndexOf($"[{months[0].Key}]", StringComparison.Ordinal) < rootIndex.IndexOf($"[{months[^1].Key}]", StringComparison.Ordinal));
            var sample = File.ReadAllText(Path.Combine(dirAll, TicketsFolder, Month(4), Named(4)));
            Debug.Assert(sample.StartsWith("---\nid: 10005\ntitle: \"Заявка 10005\"\n") && sample.Contains("\nservice: \"Сервис\"\n") && sample.Contains("решено 10005"));

            // 2. остановили на 70-м запросе карточки: записанное сохранено, оглавления по нему есть; повторный запуск
            // продолжает — карточки читаются только у недостающих
            server.Reset();
            using var stopper = new CancellationTokenSource();
            server.OnCard = n => { if (n == 70) stopper.Cancel(); };
            var dirStop = Path.Combine(root, "stop");
            var stopped = All(dirStop, ct: stopper.Token);
            Debug.Assert(stopped is { Complete: false, Failed: 0 } && stopped.Error.Contains("Остановлено") && stopped.Created is > 0 and < Count);
            Debug.Assert(stopped.Found == stopped.Created && OnDisk(dirStop).Count == stopped.Created);
            Debug.Assert(File.ReadAllText(Path.Combine(dirStop, IndexFile)).Contains($"заявок: {stopped.Created}."));
            server.Reset();
            var resumed = All(dirStop);
            Debug.Assert(resumed is { Complete: true, Failed: 0, Updated: 0, Error: "" } && resumed.Unchanged == stopped.Created
                && resumed.Created == Count - stopped.Created && server.Cards == resumed.Created && OnDisk(dirStop).Count == Count);

            // 3. ничего не менялось: ни одной карточки и переписки, только две страницы списка
            server.Reset();
            var same = All(dirStop);
            Debug.Assert(same is { Complete: true, Created: 0, Updated: 0, Failed: 0, Unchanged: Count });
            Debug.Assert(server.Cards == 0 && server.Lifetimes == 0 && server.ListTargets.Count == 2);

            // 4. изменилась и переименована одна: переписана на месте, прежнее имя убрано, оглавление месяца новое
            server.Reset();
            server.Titles[MassServer.IdOf(9)] = "Новое название";
            server.Bumps[MassServer.IdOf(9)] = TimeSpan.FromHours(2);
            var changed = All(dirStop);
            Debug.Assert(changed is { Updated: 1, Created: 0, Failed: 0, Unchanged: Count - 1 } && server.Cards == 1);
            var m10 = Path.Combine(dirStop, TicketsFolder, Month(9));
            Debug.Assert(File.Exists(Path.Combine(m10, "10010 — Новое название.md")) && !File.Exists(Path.Combine(m10, Named(9))));
            var m10Index = File.ReadAllText(Path.Combine(m10, IndexFile));
            Debug.Assert(m10Index.Contains("[[10010 — Новое название]]") && !m10Index.Contains("[[10010 — Заявка 10010]]"));
            server.Titles.Clear();
            server.Bumps.Clear();

            // 5. папка прежней раскладки (0.10–0.12, всё прямо в tickets): файлы переезжают по месяцам без запросов; чужая заметка
            // и испорченный файл (без свойств) остаются, испорченный перепишется заявкой на место
            server.Reset();
            var dirOld = Path.Combine(root, "legacy");
            var seed = All(dirOld, limit: 3);
            Debug.Assert(seed is { Found: 3, Created: 3 });
            var ticketsOld = Path.Combine(dirOld, TicketsFolder);
            foreach (var f in OnDisk(dirOld)) File.Move(f, Path.Combine(ticketsOld, Path.GetFileName(f)));
            foreach (var d in Directory.GetDirectories(ticketsOld)) Directory.Delete(d, recursive: true);
            File.Delete(Path.Combine(dirOld, IndexFile));
            File.WriteAllText(Path.Combine(ticketsOld, "заметки.md"), "моё");
            File.WriteAllText(Path.Combine(ticketsOld, "10100 — мусор.md"), "испорчен");
            server.Reset();
            var migrated = All(dirOld);
            Debug.Assert(migrated is { Complete: true, Unchanged: 3, Updated: 1, Failed: 0 } && migrated.Created == Count - 4);
            Debug.Assert(migrated.Error.Contains("переложены в папки по месяцам: 3"));
            Debug.Assert(!server.Log.Contains("C10230") && !server.Log.Contains("C10229") && !server.Log.Contains("C10228"));
            Debug.Assert(Directory.GetFiles(ticketsOld, "*.md").Select(Path.GetFileName).SequenceEqual(new[] { "заметки.md" }));
            Debug.Assert(OnDisk(dirOld).Count == Count + 1);   // 230 заявок и чужая заметка рядом (мусорный файл заменён заявкой)

            // 6. файл одной заявки в двух месяцах: перезапись оставляет один, на нужном месте
            var jan = Path.Combine(ticketsOld, Month(0), Named(0));
            File.Copy(jan, Path.Combine(ticketsOld, Month(Count - 1), Named(0)));
            server.Reset();
            var twice = ExportRowsAsync(client, new[] { server.Row(0) }, dirOld, Url, null, CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(twice is { Found: 1, Updated: 1, Created: 0, Failed: 0, Complete: true } && File.Exists(jan)
                && !File.Exists(Path.Combine(ticketsOld, Month(Count - 1), Named(0))) && OnDisk(dirOld).Count == Count + 1);

            // 7. сбои сети и сервера повторяются (карточка — 503 дважды, переписка — 500 один раз), а «нет такой» — нет
            var attempts = new Dictionary<string, int>();
            int Attempt(string key) => attempts[key] = attempts.GetValueOrDefault(key) + 1;
            server.Reset();
            server.Fault = (kind, id) => (kind, id) switch
            {
                ("card", 10005) => Attempt("card5") <= 2 ? 503 : 0,
                ("life", 10006) => Attempt("life6") <= 1 ? 500 : 0,
                ("card", 10007) => Attempt("card7") > 0 ? 404 : 0,
                _ => 0,
            };
            var dirRetry = Path.Combine(root, "retry");
            var retried = ExportRowsAsync(client, new[] { server.Row(4), server.Row(5), server.Row(6) }, dirRetry, Url, null, CancellationToken.None)
                .GetAwaiter().GetResult();
            Debug.Assert(retried is { Found: 3, Created: 2, Failed: 1, Complete: true } && retried.FirstError.StartsWith("#10007: заявка не найдена (HTTP 404)"));
            Debug.Assert(attempts["card5"] == 3 && attempts["life6"] == 2 && attempts["card7"] == 1);   // 404 не повторяется
            Debug.Assert(OnDisk(dirRetry).Count == 2);

            // 8. сервер лёг (503 на каждую карточку): после серии сбоев выгрузка прерывается, а не долбит его; файлов нет
            server.Reset();
            server.Fault = (kind, _) => kind == "card" ? 503 : 0;
            var dirDown = Path.Combine(root, "down");
            var down = All(dirDown);
            Debug.Assert(down is { Complete: false, Created: 0 } && down.Failed is >= TransientLimit and < TransientLimit + 8);
            Debug.Assert(down.Error.Contains("подряд не прочитались из-за сети или сервера") && down.Error.Contains("запустите выгрузку снова"));
            Debug.Assert(server.Cards <= (TransientLimit + 8) * (1 + RetryDelays.Length) && OnDisk(dirDown).Count == 0 && !File.Exists(Path.Combine(dirDown, IndexFile)));

            // 9. логин перестал приниматься (401 на карточках): сразу стоп; ни одна не читается (404 на всех): стоп после серии
            server.Reset();
            server.Fault = (kind, _) => kind == "card" ? 401 : 0;
            var denied = All(Path.Combine(root, "denied"));
            Debug.Assert(denied is { Complete: false, Created: 0 } && denied.Failed <= 8 && denied.Error.Contains("не принимает логин и пароль (HTTP 401)"));
            server.Reset();
            server.Fault = (kind, _) => kind == "card" ? 404 : 0;
            var nothing = All(Path.Combine(root, "nothing"));
            Debug.Assert(nothing is { Complete: false, Created: 0 } && nothing.Failed is >= NothingWorksLimit and < NothingWorksLimit + 8
                && nothing.Error.Contains($"ни одна из {NothingWorksLimit} заявок не прочиталась"));

            // 10. сервер не принял сортировку по созданию (400): первая страница заново по изменению, об этом сказано; 401 на
            // списке — не повод менять сортировку, ошибка сразу
            server.Reset();
            server.RejectCreatedSort = true;
            var dirFallback = Path.Combine(root, "fallback");
            var fallback = All(dirFallback);
            Debug.Assert(fallback is { Complete: true, Created: Count, Failed: 0 } && fallback.Error.Contains("Сервер не принял сортировку по дате создания"));
            Debug.Assert(server.ListTargets.Count == 3 && server.ListTargets[0].Contains("sort=Created") && server.ListTargets[1].Contains("sort=Changed%20desc&pagesize=200&page=1"));
            server.Reset();
            server.ListUnauthorized = true;
            var noList = All(Path.Combine(root, "nolist"));
            Debug.Assert(noList is { Found: 0, Complete: false } && noList.Error.Contains("(HTTP 401)") && server.ListTargets.Count == 1);

            // 11. сервер отдал не по созданию, как просили: выгрузка идёт, о порядке сказано; не понимает page — стоп, без цикла
            server.Reset();
            server.IgnoreSort = true;
            var unsorted = All(Path.Combine(root, "unsorted"));
            Debug.Assert(unsorted is { Complete: true, Created: Count } && unsorted.Error.Contains("не по дате создания"));
            server.Reset();
            server.IgnorePage = true;
            var stuck = All(Path.Combine(root, "stuck"));
            Debug.Assert(stuck is { Complete: false, Found: 200, Created: 200 } && stuck.Error.Contains("одну и ту же страницу") && server.ListTargets.Count == 2);

            // 12. сервер условие по дате не применил (отдал всё): чтение обрывается за концом периода, на первой же странице
            server.Reset();
            var until = new DateTime(2026, 1, 31);
            var inPeriod = Enumerable.Range(0, Count).Count(i => MassServer.CreatedOf(i) <= until.AddDays(1));
            var cut = All(Path.Combine(root, "cut"), query: new TaskQuery(Created: new(null, until)));
            Debug.Assert(cut is { Complete: true, Error: "" } && cut.Found == inPeriod && inPeriod < 200 && server.ListTargets.Count == 1);

            // 13. «не больше 210»: свежие по изменению первыми (двести с первой страницы и десять со второй), без сортировки по созданию
            server.Reset();
            var dirLimit = Path.Combine(root, "limit");
            var limited = All(dirLimit, limit: 210);
            Debug.Assert(limited is { Found: 210, Created: 210, Complete: true } && server.ListTargets.Count == 2 && server.ListTargets[0].Contains("sort=Changed%20desc"));
            var limitedNames = OnDisk(dirLimit).Select(Path.GetFileName).ToList();
            Debug.Assert(limitedNames.Any(n => n!.StartsWith("10021 —")) && !limitedNames.Any(n => n!.StartsWith("10020 —")) && limitedNames.Any(n => n!.StartsWith("10230 —")));

            // 14. диск не пишет (на месте папки января лежит файл): после серии ошибок записи выгрузка прерывается
            server.Reset();
            var dirBlock = Path.Combine(root, "block");
            Directory.CreateDirectory(Path.Combine(dirBlock, TicketsFolder));
            File.WriteAllText(Path.Combine(dirBlock, TicketsFolder, Month(0)), "я файл, а не папка");
            var blocked = All(dirBlock);
            Debug.Assert(blocked is { Complete: false, Created: 0 } && blocked.Failed is >= WriteFailLimit and < WriteFailLimit + 8);
            Debug.Assert(blocked.Error.Contains("файлы не записываются") && blocked.FirstError.Contains("файл не записан"));

            // 15. срок запроса у каждого свой: страница списка, не ответившая за срок, — сбой сети (его повторяют); отмена — не срок
            server.Reset();
            server.ListDelayMs = 400;
            var late = client.GetTasksAsync(new TaskQuery(), 1, timeout: TimeSpan.FromMilliseconds(80)).GetAwaiter().GetResult();
            Debug.Assert(late.Error.StartsWith("сервер не ответил") && HttpIntraserviceClient.IsTransient(late.Error));
            server.Reset();
            using var cancelled = new CancellationTokenSource();
            cancelled.Cancel();
            var threw = false;
            try { client.GetTasksAsync(new TaskQuery(), 1, cancelled.Token).GetAwaiter().GetResult(); }
            catch (OperationCanceledException) { threw = true; }
            Debug.Assert(threw);
            server.Reset();
        }
        finally
        {
            RetryDelays = delays;
            listener.Stop();
            try { Directory.Delete(root, recursive: true); } catch (IOException) { }
        }
    }

    /// <summary>Все файлы заявок в tickets (в любых папках), без оглавлений.</summary>
    private static List<string> OnDisk(string dir) => !Directory.Exists(Path.Combine(dir, TicketsFolder)) ? new()
        : Directory.EnumerateFiles(Path.Combine(dir, TicketsFolder), "*.md", SearchOption.AllDirectories)
            .Where(p => Path.GetFileName(p) != IndexFile).ToList();

    /// <summary>Ход выгрузки как есть, по порядку прихода.</summary>
    private sealed class Collected : IProgress<string>
    {
        public readonly List<string> Messages = new();
        public void Report(string value) { lock (Messages) Messages.Add(value); }
    }

    /// <summary>Поддельный Интрасервис на 230 заявок: номера с 10001, созданы через каждые шесть часов с 20 января 2026,
    /// изменены часом позже. Отдаёт страницы списка (по возрастанию создания, если просили `Created asc`, иначе свежие по
    /// изменению сверху), карточки и переписку; считает запросы и ведёт журнал (L&lt;страница&gt;, C&lt;номер&gt;). Сбои —
    /// по желанию теста: Fault(вид, номер) отвечает кодом ошибки или 0.</summary>
    private sealed class MassServer
    {
        public const int Count = 230;
        private static readonly DateTime Start = new(2026, 1, 20, 8, 0, 0);

        public static DateTime CreatedOf(int i) => Start.AddHours(i * 6);
        public static int IdOf(int i) => 10001 + i;

        public readonly Dictionary<int, string> Titles = new();
        public readonly Dictionary<int, TimeSpan> Bumps = new();
        public bool RejectCreatedSort, IgnoreSort, IgnorePage, ListUnauthorized;
        public int ListDelayMs;
        public Func<string, int, int>? Fault;
        public Action<int>? OnCard;
        public readonly List<string> Log = new();
        public readonly List<string> ListTargets = new();
        public int Cards, Lifetimes, Lists;

        private string TitleOf(int id) => Titles.GetValueOrDefault(id, $"Заявка {id}");
        private DateTime ChangedOf(int i) => CreatedOf(i).AddHours(1) + Bumps.GetValueOrDefault(IdOf(i));
        private static string Iso(DateTime d) => d.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture);
        private static DateTimeOffset Local(DateTime d) => DateTimeOffset.Parse(Iso(d), CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal);

        /// <summary>Строка списка так, как её разберёт клиент.</summary>
        public IntraserviceFound Row(int i) =>
            new(IdOf(i), TitleOf(IdOf(i)), "Выполнена", "Петрова А.", Local(CreatedOf(i)), Changed: Local(ChangedOf(i)));

        /// <summary>Новый заход: сначала дождаться, пока сервер разберёт брошенные клиентом соединения (отмена, сбой — запросы
        /// остаются в очереди и считались бы уже в следующем заходе), затем обнулить счётчики и сбои.</summary>
        public void Reset()
        {
            Quiesce();
            Cards = Lifetimes = Lists = 0;
            Log.Clear();
            ListTargets.Clear();
            Fault = null;
            OnCard = null;
            RejectCreatedSort = IgnoreSort = IgnorePage = ListUnauthorized = false;
            ListDelayMs = 0;
        }

        private void Quiesce()
        {
            int Seen() => Volatile.Read(ref Cards) + Volatile.Read(ref Lifetimes) + Volatile.Read(ref Lists);
            for (var last = -1; last != Seen();)
            {
                last = Seen();
                Thread.Sleep(120);
            }
        }

        private static int Query(string target, string name, int fallback) =>
            Regex.Match(target, $@"[?&]{name}=(\d+)") is { Success: true } m ? int.Parse(m.Groups[1].Value, CultureInfo.InvariantCulture) : fallback;

        public (int Code, string Json) Respond(string target)
        {
            if (target.StartsWith("/api/taskstatus"))
                return (200, """[{"Id":29,"Name":"Выполнена","IsFixed":true}]""");
            if (target.StartsWith("/api/task?"))
            {
                Lists++;
                ListTargets.Add(target);
                var page = IgnorePage ? 1 : Query(target, "page", 1);
                var size = Query(target, "pagesize", 25);
                Log.Add("L" + page);
                if (ListDelayMs > 0) Thread.Sleep(ListDelayMs);
                if (ListUnauthorized) return (401, """{"Message":"Authorization has been denied"}""");
                if (RejectCreatedSort && target.Contains("sort=Created")) return (400, """{"Message":"Invalid sort field"}""");
                var ascending = !IgnoreSort && target.Contains("sort=Created%20asc");
                var order = ascending ? Enumerable.Range(0, Count) : Enumerable.Range(0, Count).Reverse();
                var rows = order.Skip((page - 1) * size).Take(size).Select(RowJson).ToList();
                return (200, "{\"Tasks\":[" + string.Join(",", rows) + "],\"Statuses\":[{\"Id\":29,\"Name\":\"Выполнена\"}],"
                    + $"\"Paginator\":{{\"Count\":{Count},\"Page\":{page},\"PageCount\":{(Count + size - 1) / size},\"PageSize\":{size},\"CountOnPage\":{rows.Count}}}}}");
            }
            if (target.StartsWith("/api/task/"))
            {
                var id = int.Parse(target["/api/task/".Length..].Split('?')[0], CultureInfo.InvariantCulture);
                Cards++;
                Log.Add("C" + id);
                OnCard?.Invoke(Cards);
                if (Fault?.Invoke("card", id) is > 0 and var code) return (code, """{"Message":"fault"}""");
                return (200, $"{{\"Task\":{{\"Id\":{id},\"Name\":\"{TitleOf(id)}\",\"StatusName\":\"Выполнена\",\"ServiceName\":\"Сервис\","
                    + "\"Type\":\"Инцидент\",\"Categories\":\"Кат\",\"ExecutorGroup\":\"Группа\"}}");
            }
            if (target.StartsWith("/api/tasklifetime?"))
            {
                var id = Query(target, "taskid", 0);
                Lifetimes++;
                if (Fault?.Invoke("life", id) is > 0 and var code) return (code, """{"Message":"fault"}""");
                return (200, "{\"TaskLifetimeList\":{\"TaskLifetimes\":[{\"Date\":\"" + Iso(CreatedOf(id - 10001).AddHours(2))
                    + "\",\"Editor\":\"Иванов\",\"StatusId\":29,\"Comments\":\"<p>решено " + id + "</p>\",\"IsPublic\":true}],"
                    + "\"Statuses\":[{\"Id\":29,\"Name\":\"Выполнена\"}],\"Paginator\":{\"Page\":1,\"PageCount\":1}}}");
            }
            return (404, """{"Message":"not found"}""");
        }

        private string RowJson(int i) =>
            $"{{\"Id\":{IdOf(i)},\"Name\":\"{TitleOf(IdOf(i))}\",\"StatusId\":29,\"Created\":\"{Iso(CreatedOf(i))}\",\"Changed\":\"{Iso(ChangedOf(i))}\","
            + "\"Creator\":\"Петрова А.\",\"Executors\":\"Я Сам\",\"Description\":\"<p>описание</p>\"}";
    }
}
