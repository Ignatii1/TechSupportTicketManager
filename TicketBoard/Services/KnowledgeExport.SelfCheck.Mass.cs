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
        // у каждого захода свой «поколение» в адресе: запросы, брошенные клиентом при отмене и сбое, сервер разбирает с опозданием,
        // и в счётчиках следующего захода им делать нечего. Без этого проверка зависела бы от того, как быстро шумит процессор
        HttpIntraserviceClient client = new($"http://127.0.0.1:{port}/g0", "u", "p");
        void Fresh()
        {
            server.NextGeneration();
            client = new($"http://127.0.0.1:{port}/g{server.Generation}", "u", "p");
        }
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
            // первая страница — со счётом по умолчанию (общее число для хода), дальше — без счёта: count=false и HasNextPage
            Debug.Assert(server.ListTargets.Count == 2 && server.ListTargets[0].Contains("sort=Created%20asc,%20Id%20asc&pagesize=200&page=1")
                && !server.ListTargets[0].Contains("count=") && server.ListTargets[1].Contains("&count=false&") && server.ListTargets[1].EndsWith("&page=2"));
            // выгрузка идёт по страницам, а не после всего списка: карточки второй страницы просят, когда все 200 заявок первой уже
            // записаны; а сама вторая страница запрошена заранее, пока шла первая (журнал: «L2» среди первых запросов)
            var cardIds = server.Log.Where(x => x[0] == 'C').Select(x => int.Parse(x[1..], CultureInfo.InvariantCulture)).ToList();
            Debug.Assert(cardIds.Count == Count && cardIds.Take(200).All(id => id <= MassServer.IdOf(199)) && cardIds.Skip(200).All(id => id > MassServer.IdOf(199)));
            Debug.Assert(server.Log.IndexOf("L2") is >= 0 and < 100);
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
            Fresh();
            using var stopper = new CancellationTokenSource();
            server.OnCard = n => { if (n == 70) stopper.Cancel(); };
            var dirStop = Path.Combine(root, "stop");
            var stopped = All(dirStop, ct: stopper.Token);
            Debug.Assert(stopped is { Complete: false, Failed: 0 } && stopped.Error.Contains("Остановлено") && stopped.Created is > 0 and < Count);
            Debug.Assert(stopped.Found == stopped.Created && OnDisk(dirStop).Count == stopped.Created);
            Debug.Assert(File.ReadAllText(Path.Combine(dirStop, IndexFile)).Contains($"заявок: {stopped.Created}."));
            Fresh();
            var resumed = All(dirStop);
            Debug.Assert(resumed is { Complete: true, Failed: 0, Updated: 0, Error: "" } && resumed.Unchanged == stopped.Created
                && resumed.Created == Count - stopped.Created && server.Cards == resumed.Created && OnDisk(dirStop).Count == Count);

            // 2б. остановили на последней странице (список больше не просят): итог всё равно «остановлено», а не «готово»
            Fresh();
            using var lastStopper = new CancellationTokenSource();
            server.OnCard = n => { if (n == 215) lastStopper.Cancel(); };
            var stoppedLast = All(Path.Combine(root, "stop-last"), ct: lastStopper.Token);
            Debug.Assert(stoppedLast is { Complete: false, Failed: 0 } && stoppedLast.Error.Contains("Остановлено") && stoppedLast.Created is > 200 and < Count);

            // 2в. переписка длиннее предела страниц (здесь 2 из 5): самые свежие записи на месте, а о том, что ранних нет, сказано в файле;
            // с настоящим пределом читается целиком, и пометки нет
            Fresh();
            var pages = MaxLifetimePages;
            MaxLifetimePages = 2;
            try
            {
                var shortened = BuildAsync(client, server.Row(1), Url(MassServer.LongId), CancellationToken.None).GetAwaiter().GetResult();
                Debug.Assert(shortened.Error == "" && shortened.Text.Contains("показаны последние 100 записей") && shortened.Text.Contains("запись 2-49")
                    && shortened.Text.Contains("запись 1-0") && !shortened.Text.Contains("запись 3-0"));
            }
            finally { MaxLifetimePages = pages; }
            var whole = BuildAsync(client, server.Row(1), Url(MassServer.LongId), CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(whole.Error == "" && whole.Text.Contains("запись 5-49") && whole.Text.Contains("запись 3-0") && !whole.Text.Contains("показаны последние"));
            Debug.Assert(!File.ReadAllText(Path.Combine(dirAll, TicketsFolder, Month(4), Named(4))).Contains("показаны последние"));

            // 3. ничего не менялось: ни одной карточки и переписки, только две страницы списка
            Fresh();
            var same = All(dirStop);
            Debug.Assert(same is { Complete: true, Created: 0, Updated: 0, Failed: 0, Unchanged: Count });
            Debug.Assert(server.Cards == 0 && server.Lifetimes == 0 && server.ListTargets.Count == 2);

            // 4. изменилась и переименована одна: переписана на месте, прежнее имя убрано, оглавление месяца новое
            Fresh();
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

            // 4б. повторная выгрузка, где почти всё готово, а у тридцати заявок карточка вдруг отказывает (404): это не «ничего не читается»
            Fresh();
            var back = All(dirStop);   // сервер вернул десятой заявке прежнее название и дату: она переписывается ещё раз, остальное готово
            Debug.Assert(back is { Updated: 1, Created: 0, Failed: 0, Unchanged: Count - 1 });
            Fresh();
            for (var i = 30; i < 60; i++) server.Bumps[MassServer.IdOf(i)] = TimeSpan.FromHours(3);
            server.Fault = (kind, id) => kind == "card" && id is >= 10031 and <= 10060 ? 404 : 0;
            var refused = All(dirStop);
            Debug.Assert(refused is { Complete: true, Created: 0, Updated: 0, Failed: 30 } && refused.Unchanged == Count - 30 && refused.Error == "");
            server.Bumps.Clear();

            // 5. папка прежней раскладки (0.10–0.12, всё прямо в tickets): файлы переезжают по месяцам без запросов; чужая заметка
            // и испорченный файл (без наших свойств) остаются где лежат — заявка 10100 пишется рядом, а не вместо него
            Fresh();
            var dirOld = Path.Combine(root, "legacy");
            var seed = All(dirOld, limit: 3);
            Debug.Assert(seed is { Found: 3, Created: 3 });
            var ticketsOld = Path.Combine(dirOld, TicketsFolder);
            foreach (var f in OnDisk(dirOld)) File.Move(f, Path.Combine(ticketsOld, Path.GetFileName(f)));
            foreach (var d in Directory.GetDirectories(ticketsOld)) Directory.Delete(d, recursive: true);
            File.Delete(Path.Combine(dirOld, IndexFile));
            File.WriteAllText(Path.Combine(ticketsOld, "заметки.md"), "моё");
            File.WriteAllText(Path.Combine(ticketsOld, "10100 — мусор.md"), "испорчен");
            File.WriteAllText(Path.Combine(ticketsOld, "10200 — моё.md"), "---\nid: 10200\ntitle: \"моё\"\n---\n\nмоя заметка с похожими свойствами\n");
            // и папки пользователя среди месяцев: с его заметкой про заявку 10005 и пустая — не месяцы, не оглавляются и не убираются
            var userNotes = Path.Combine(ticketsOld, "свои", "10005 — заметка про принтер.md");
            Directory.CreateDirectory(Path.GetDirectoryName(userNotes)!);
            File.WriteAllText(userNotes, "---\ntitle: \"моё\"\n---\n\nтекст\n");
            Directory.CreateDirectory(Path.Combine(ticketsOld, "пустая"));
            Fresh();
            var migrated = All(dirOld);
            Debug.Assert(migrated is { Complete: true, Unchanged: 3, Updated: 0, Failed: 0 } && migrated.Created == Count - 3);
            Debug.Assert(migrated.Error.Contains("переложены в папки по месяцам: 3"));
            Debug.Assert(!server.Log.Contains("C10230") && !server.Log.Contains("C10229") && !server.Log.Contains("C10228"));
            Debug.Assert(Directory.GetFiles(ticketsOld, "*.md").Select(Path.GetFileName).Order().SequenceEqual(new[] { "10100 — мусор.md", "10200 — моё.md", "заметки.md" }.Order()));
            Debug.Assert(OnDisk(dirOld).Count == Count + 4);   // 230 заявок, три чужих файла в tickets и заметка в папке пользователя
            var oldRoot = File.ReadAllText(Path.Combine(dirOld, IndexFile));
            Debug.Assert(File.Exists(userNotes) && Directory.Exists(Path.Combine(ticketsOld, "пустая")) && !File.Exists(Path.Combine(ticketsOld, "свои", IndexFile))
                && !oldRoot.Contains("свои") && !oldRoot.Contains("пустая") && oldRoot.Contains($"заявок: {Count}."));

            // 6. файл заявки под старым именем в том же месяце (её переименовали): выгрузка выбранных оставляет один, на нужном месте.
            // Дубль в чужом месяце она не ищет (обходить ради нескольких заявок все сотни тысяч файлов не стоит) — его уберёт выгрузка всех
            var jan = Path.Combine(ticketsOld, Month(0), Named(0));
            var oldName = Path.Combine(ticketsOld, Month(0), "10001 — Старое название.md");
            var elsewhere = Path.Combine(ticketsOld, Month(Count - 1), Named(0));
            File.Copy(jan, oldName);
            File.Copy(jan, elsewhere);
            Fresh();
            var twice = ExportRowsAsync(client, new[] { server.Row(0) }, dirOld, Url, null, CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(twice is { Found: 1, Updated: 1, Created: 0, Failed: 0, Complete: true } && File.Exists(jan) && !File.Exists(oldName) && File.Exists(elsewhere));
            Fresh();
            var cleaned = All(dirOld);
            Debug.Assert(cleaned is { Complete: true, Failed: 0, Updated: 1, Unchanged: Count - 1 } && !File.Exists(elsewhere) && File.Exists(jan)
                && OnDisk(dirOld).Count == Count + 4 && File.Exists(userNotes));

            // 7. сбои сети и сервера повторяются (карточка — 503 дважды, переписка — 500 один раз), а «нет такой» — нет
            var attempts = new Dictionary<string, int>();
            int Attempt(string key) => attempts[key] = attempts.GetValueOrDefault(key) + 1;
            Fresh();
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

            // 7а. глубокая папка хранилища: путь к tickets 232 знака — от названий не остаётся ничего, только номер, чтобы путь не превысил
            // 250 (MAX_PATH Windows — 260, и запись упала бы ошибкой, похожей на «диск полон»); при повторе имена те же — без изменений
            Fresh();
            var dirDeep = Path.Combine(root, "deep" + new string('x', 232 - (root.Length + 1 + "deep".Length + 1 + TicketsFolder.Length)));
            var deepRows = new[] { server.Row(7), server.Row(8), server.Row(9) };
            var deep = ExportRowsAsync(client, deepRows, dirDeep, Url, null, CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(deep is { Found: 3, Created: 3, Failed: 0, Complete: true } && OnDisk(dirDeep).Count == 3);
            Debug.Assert(OnDisk(dirDeep).All(f => f.Length <= 250 && Path.GetFileName(f) == $"{int.Parse(Path.GetFileNameWithoutExtension(f), CultureInfo.InvariantCulture)}.md"));
            Fresh();
            var deepAgain = ExportRowsAsync(client, deepRows, dirDeep, Url, null, CancellationToken.None).GetAwaiter().GetResult();
            Debug.Assert(deepAgain is { Found: 3, Created: 0, Updated: 0, Unchanged: 3 });

            // 7б. битая заявка: на её карточку сервер всегда отвечает 500. Ей один повтор, не три; серию из шестидесяти таких подряд
            // выгрузка переживает и идёт дальше (иначе дальше битого места не пройти ни при каком повторе); а сервер, что отвечает
            // 500 на всё, останавливает её после ста
            Fresh();
            server.Fault = (kind, id) => kind == "card" && id is >= 10041 and <= 10100 ? 500 : 0;
            var damaged = All(Path.Combine(root, "damaged"));
            Debug.Assert(damaged is { Complete: true, Failed: 60, Created: Count - 60 } && damaged.FirstError.Contains("ошибка сервера (HTTP 500)"));
            Debug.Assert(server.Log.Count(x => x[0] == 'C' && int.Parse(x[1..], CultureInfo.InvariantCulture) is >= 10041 and <= 10100) == 2 * 60);
            Fresh();
            server.Fault = (kind, _) => kind == "card" ? 500 : 0;
            var broken = All(Path.Combine(root, "broken"));
            Debug.Assert(broken is { Complete: false, Created: 0 } && broken.Failed is >= ServerErrorLimit and < ServerErrorLimit + 8 && broken.Error.Contains("ошибкой 500"));

            // 8. сервер лёг (503 на каждую карточку): после серии сбоев выгрузка прерывается, а не долбит его; файлов нет
            Fresh();
            server.Fault = (kind, _) => kind == "card" ? 503 : 0;
            var dirDown = Path.Combine(root, "down");
            var down = All(dirDown);
            Debug.Assert(down is { Complete: false, Created: 0 } && down.Failed is >= TransientLimit and < TransientLimit + 8);
            Debug.Assert(down.Error.Contains("подряд не прочитались из-за сети или сервера") && down.Error.Contains("запустите выгрузку снова"));
            Debug.Assert(server.Cards <= (TransientLimit + 8) * (1 + RetryDelays.Length) && OnDisk(dirDown).Count == 0 && !File.Exists(Path.Combine(dirDown, IndexFile)));

            // 9. логин перестал приниматься (401 на карточках): сразу стоп; ни одна не читается (404 на всех): стоп после серии
            Fresh();
            server.Fault = (kind, _) => kind == "card" ? 401 : 0;
            var denied = All(Path.Combine(root, "denied"));
            Debug.Assert(denied is { Complete: false, Created: 0 } && denied.Failed <= 8 && denied.Error.Contains("не принимает логин и пароль (HTTP 401)"));
            Fresh();
            server.Fault = (kind, _) => kind == "card" ? 404 : 0;
            var nothing = All(Path.Combine(root, "nothing"));
            Debug.Assert(nothing is { Complete: false, Created: 0 } && nothing.Failed is >= NothingWorksLimit and < NothingWorksLimit + 8
                && nothing.Error.Contains($"ни одна из {NothingWorksLimit} заявок не прочиталась"));

            // 10. сервер не принял сортировку по созданию (400): первая страница заново по изменению, об этом сказано; 401 на
            // списке — не повод менять сортировку, ошибка сразу
            Fresh();
            server.RejectCreatedSort = 400;
            var dirFallback = Path.Combine(root, "fallback");
            var fallback = All(dirFallback);
            Debug.Assert(fallback is { Complete: true, Created: Count, Failed: 0 } && fallback.Error.Contains("Сервер не принял сортировку по дате создания"));
            Debug.Assert(server.ListTargets.Count == 3 && server.ListTargets[0].Contains("sort=Created") && server.ListTargets[1].Contains("sort=Changed%20desc&pagesize=200&page=1"));
            // и если сервер на незнакомое поле сортировки отвечает ошибкой 500: повторы исчерпаны (4 запроса) — и тот же запасной порядок
            Fresh();
            server.RejectCreatedSort = 500;
            var fallback500 = All(Path.Combine(root, "fallback500"));
            Debug.Assert(fallback500 is { Complete: true, Created: Count, Failed: 0 } && fallback500.Error.Contains("Сервер не принял сортировку по дате создания"));
            Debug.Assert(server.ListTargets.Count == 2 + 2 && server.ListTargets.Count(t => t.Contains("sort=Created")) == 2);   // 500: один повтор
            // а если список не отдаётся вовсе (400 на любой запрос, дело не в сортировке), запасной порядок не помог — о сортировке не пишем
            Fresh();
            server.FailLists = 400;
            var dirListFail = Path.Combine(root, "listfail");
            Directory.CreateDirectory(Path.Combine(dirListFail, TicketsFolder));
            File.Copy(Path.Combine(dirAll, TicketsFolder, Month(3), Named(3)), Path.Combine(dirListFail, TicketsFolder, Named(3)));   // есть что переложить: итог, а не просто ошибка
            var listFail = All(dirListFail);
            Debug.Assert(listFail is { Complete: false } && listFail.Error.Contains("Список пришёл не целиком") && listFail.Error.Contains("переложены")
                && !listFail.Error.Contains("не принял сортировку"));
            Fresh();
            server.ListUnauthorized = true;
            var noList = All(Path.Combine(root, "nolist"));
            Debug.Assert(noList is { Found: 0, Complete: false } && noList.Error.Contains("(HTTP 401)") && server.ListTargets.Count == 1);

            // 11. сервер отдал не по созданию, как просили: выгрузка идёт, о порядке сказано; не понимает page — стоп, без цикла
            Fresh();
            server.IgnoreSort = true;
            var unsorted = All(Path.Combine(root, "unsorted"));
            Debug.Assert(unsorted is { Complete: true, Created: Count } && unsorted.Error.Contains("не по дате создания"));
            Fresh();
            server.IgnorePage = true;
            var stuck = All(Path.Combine(root, "stuck"));
            Debug.Assert(stuck is { Complete: false, Found: 200, Created: 200 } && stuck.Error.Contains("одну и ту же страницу") && server.ListTargets.Count == 2);

            // 11б. список сдвинулся, пока читали: вторая страница начинается с последней заявки первой — она не выгружается дважды
            Fresh();
            server.Overlap = true;
            var shifted = All(Path.Combine(root, "shifted"));
            Debug.Assert(shifted is { Complete: true, Created: Count, Failed: 0, Error: "" } && server.Cards == Count);

            // 11в. сервер режет страницу до 25 заявок и счёта не присылает: неполная страница конца не доказывает — выгружено всё
            Fresh();
            server.NoCount = true;
            var capped = All(Path.Combine(root, "capped"));
            Debug.Assert(capped is { Complete: true, Created: Count, Failed: 0, Error: "" } && server.ListTargets.Count == (Count + 24) / 25);

            // 11г. сервер досчитал только до потолка (Count = CountCeiling — «столько или больше») и, по самому строгому прочтению
            // документации (стр. 14), со счётом за потолком списка не отдаёт: выгружено всё — страницы после первой идут с
            // count=false; ход без «из», в итоге — сколько выгружено. «Не больше 50» при «210 или больше» — сколько выйдет, известно
            var ceiling = HttpIntraserviceClient.CountCeiling;
            HttpIntraserviceClient.CountCeiling = 210;
            try
            {
                Fresh();
                server.CountCap = 210;
                var capProgress = new Collected();
                var overCap = All(Path.Combine(root, "overcap"), progress: capProgress);
                // «210 или больше» — второй проход всегда (пропуск при сдвиге списка там счётом не заметить): ещё две страницы
                Debug.Assert(overCap is { Complete: true, Created: Count, Failed: 0, Error: "" } && server.ListTargets.Count == 4
                    && !server.ListTargets[0].Contains("count=") && server.ListTargets[1].Contains("&count=false&")
                    && server.ListTargets[2].Contains("sort=Changed%20desc"));
                Debug.Assert(capProgress.Messages.All(m => !m.Contains(" из ")) && capProgress.Messages[^1].StartsWith($"Выгружено {Count}"));
                Fresh();
                server.CountCap = 210;
                var capLimitProgress = new Collected();
                var capLimited = All(Path.Combine(root, "overcap50"), limit: 50, progress: capLimitProgress);
                Debug.Assert(capLimited is { Found: 50, Created: 50, Complete: true } && capLimitProgress.Messages[^1] == "Выгружено 50 из 50");
            }
            finally { HttpIntraserviceClient.CountCeiling = ceiling; }

            // 11д. сервер не принял count=false (400 на второй странице): та же страница со счётом, выгрузка не обрывается
            Fresh();
            server.RejectNoCount = true;
            var noCountRefused = All(Path.Combine(root, "nocountrefused"));
            Debug.Assert(noCountRefused is { Complete: true, Created: Count, Failed: 0 } && noCountRefused.Error.Contains("без счёта")
                && server.ListTargets.Count == 3 && server.ListTargets[1].Contains("count=false") && !server.ListTargets[2].Contains("count="));
            // 400 на второй странице не из-за счёта (со счётом тоже 400): причина — ответ сервера, а не «не принял без счёта»
            Fresh();
            server.FailFrom = 2;
            server.FailCode = 400;
            var page2Refused = All(Path.Combine(root, "page2refused"));
            Debug.Assert(page2Refused is { Complete: false, Created: 200 } && page2Refused.Error.Contains("Список пришёл не целиком")
                && !page2Refused.Error.Contains("без счёта") && server.ListTargets.Count == 3);
            // а если вдобавок счёт упирается в потолок и за ним список пуст — «всё» не всё: так и сказано, выгрузка не закончена
            HttpIntraserviceClient.CountCeiling = 210;
            try
            {
                Fresh();
                server.RejectNoCount = true;
                server.CountCap = 210;
                var cutAtCap = All(Path.Combine(root, "cutatcap"));
                // второй проход, в другом порядке, дочитывает другой конец обрезанного списка — но о потолке всё равно сказано
                Debug.Assert(cutAtCap is { Complete: false, Created: Count, Failed: 0 } && cutAtCap.Error.Contains("только первые 210")
                    && cutAtCap.Error.Contains("дочитал ещё 20"));
                // count=false принят, но не понят (ответ со счётом, без HasNextPage) — та же обрезка, то же «не закончено»
                Fresh();
                server.IgnoreNoCount = true;
                server.CountCap = 210;
                var ignoredNoCount = All(Path.Combine(root, "ignorednocount"));
                Debug.Assert(ignoredNoCount is { Complete: false, Created: Count } && ignoredNoCount.Error.Contains("только первые 210")
                    && !ignoredNoCount.Error.Contains("без счёта"));
                // счёт на потолке (250), а список кончился раньше (230): «кончился раньше, чем обещал», а не «ровно на потолке»
                HttpIntraserviceClient.CountCeiling = 250;
                Fresh();
                server.IgnoreNoCount = true;
                server.ClaimCount = 250;
                var shortOfCap = All(Path.Combine(root, "shortofcap"));
                Debug.Assert(shortOfCap is { Created: Count } && shortOfCap.Error.Contains("раньше, чем обещал сервер")
                    && !shortOfCap.Error.Contains("ровно на потолке"));
            }
            finally { HttpIntraserviceClient.CountCeiling = ceiling; }

            // 11е. порядок по созданию на стыке страниц не постоянный (у пользователя: «найдено 260, выгружено 259»): вторая
            // страница повторила последнюю заявку первой, а следующую потеряла — второй проход, по изменению, её находит
            Fresh();
            server.UnstableBoundary = true;
            var unstable = All(Path.Combine(root, "unstable"));
            Debug.Assert(unstable is { Complete: true, Created: Count, Failed: 0 } && unstable.Error.Contains("которых не было в первом: 1")
                && unstable.Error.Contains("повторял заявки на стыке страниц: 1") && !unstable.Error.Contains("раньше, чем обещал")
                && server.ListTargets.Count == 4 && server.ListTargets[2].Contains("sort=Changed%20desc"));   // второй проход — весь список
            // то же при «тысяче или больше» (по счёту не проверить — второй проход по повтору на стыке)
            HttpIntraserviceClient.CountCeiling = 210;
            try
            {
                Fresh();
                server.UnstableBoundary = true;
                server.CountCap = 210;
                var unstableCapped = All(Path.Combine(root, "unstablecapped"));
                Debug.Assert(unstableCapped is { Created: Count, Failed: 0 } && unstableCapped.Error.Contains("которых не было в первом: 1")
                    && !unstableCapped.Error.Contains("Сервер насчитал"));   // «посчитана дважды» — только при точном счёте
            }
            finally { HttpIntraserviceClient.CountCeiling = ceiling; }
            // счёт догнан (за время выгрузки появились новые — здесь: счёт на одну меньше), а повтор на стыке был: второй проход
            // всё равно нужен — потерянная заявка находится
            Fresh();
            server.UnstableBoundary = true;
            server.ClaimCount = Count - 1;
            var masked = All(Path.Combine(root, "masked"));
            Debug.Assert(masked is { Created: Count, Failed: 0 } && masked.Error.Contains("которых не было в первом: 1"));
            // второй проход: count=false не принят (400) — та же страница со счётом, проход доходит до конца
            Fresh();
            server.UnstableBoundary = true;
            server.RejectNoCountWhenChanged = true;
            var secondRefused = All(Path.Combine(root, "secondrefused"));
            Debug.Assert(secondRefused is { Complete: true, Created: Count } && !secondRefused.Error.Contains("не удался")
                && secondRefused.Error.Contains("которых не было в первом: 1"));
            // без HasNextPage, а за концом сервер повторяет последнюю страницу: оба прохода видят в этом конец, а не «та же страница»
            Fresh();
            server.UnstableBoundary = true;
            server.ClampNoHasNext = true;
            var clamped = All(Path.Combine(root, "clamped"));
            Debug.Assert(clamped is { Complete: true, Created: Count, Failed: 0 } && !clamped.Error.Contains("одну и ту же страницу")
                && clamped.Error.Contains("которых не было в первом: 1") && clamped.Error.Contains("на стыке страниц: 1)"));   // повтор хвоста — не стык
            // а сервер, что не понимает page и сам режет страницу до 25 (без счёта): вторая страница — та же первая, это «та же
            // страница снова», а не конец списка
            Fresh();
            server.IgnorePage = true;
            server.NoCount = true;
            var ignoredShort = All(Path.Combine(root, "ignoredshort"));
            Debug.Assert(ignoredShort is { Complete: false, Created: 25 } && ignoredShort.Error.Contains("одну и ту же страницу"));
            // второй проход получает одну и ту же страницу (page не понят): находит, что на ней, и не ходит по кругу
            Fresh();
            server.UnstableBoundary = true;
            server.IgnorePageWhenChanged = true;
            var stuckSecond = All(Path.Combine(root, "stucksecond"));
            Debug.Assert(stuckSecond is { Created: Count, Failed: 0, Complete: false } && server.ListTargets.Count == 4   // 2 + 2: вторая — та же, стоп
                && stuckSecond.Error.Contains("одну и ту же страницу"));
            // счёт — потолок («1000 или больше»), а отдано меньше: «посчитана дважды» тут не вывод — только при точном счёте
            Fresh();
            server.UnstableBoundary = true;
            server.ClaimCount = HttpIntraserviceClient.CountCeiling;
            var cappedClaim = All(Path.Combine(root, "cappedclaim"));
            Debug.Assert(cappedClaim is { Created: Count } && cappedClaim.Error.Contains("которых не было в первом: 1") && !cappedClaim.Error.Contains("Сервер насчитал"));
            // сервер насчитал на одну больше, чем отдаёт: второй проход ничего не нашёл — так и сказано, совета «повторите позже» нет
            Fresh();
            server.ClaimCount = Count + 1;
            var inflated = All(Path.Combine(root, "inflated"));
            Debug.Assert(inflated is { Complete: true, Created: Count } && inflated.Error.Contains($"Сервер насчитал {Count + 1}, а разных заявок в списке {Count}")
                && !inflated.Error.Contains("раньше, чем обещал"));
            // а на пятьдесят больше — не «посчитана дважды»: список не дочитан, выгрузка не закончена
            Fresh();
            server.ClaimCount = Count + 50;
            var farShort = All(Path.Combine(root, "farshort"));
            Debug.Assert(farShort is { Complete: false, Created: Count } && farShort.Error.Contains($"раньше, чем обещал сервер ({Count} из {Count + 50})")
                && !farShort.Error.Contains("посчитана дважды"));

            // 11ж. файл записан, а потом пропал (антивирус удаляет по содержимому): итог называет такие заявки и не «Готово»
            Fresh();
            var dirVanish = Path.Combine(root, "vanish");
            var removed = new List<int>();
            // три — с первой страницы (проверяются со страницей отставания), одна — с последней (проверяется в конце)
            void Remove(int count, Func<int, bool> which)
            {
                foreach (var file in Directory.EnumerateFiles(Path.Combine(dirVanish, TicketsFolder), "*", SearchOption.AllDirectories)
                             .Where(f => TryIdOf(Path.GetFileName(f), out var id) && which(id)).Take(count).ToList())
                {
                    File.Delete(file);
                    removed.Add(TryIdOf(Path.GetFileName(file), out var id) ? id : 0);
                }
            }
            server.OnCard = n =>
            {
                if (n == 100) Remove(3, _ => true);
                if (n == Count - 1) Remove(1, id => id > MassServer.IdOf(199));
            };
            var vanish = All(dirVanish);
            Debug.Assert(removed.Count == 4 && vanish is { Complete: false, Created: Count } && vanish.Error.Contains("Записано, но уже нет на диске: 4")
                && removed.All(id => vanish.Error.Contains($"#{id}")) && OnDisk(dirVanish).Count == Count - 4);

            // файл пропадает не сразу, а через мгновение после записи (как у антивируса): итог ждёт VanishWait и видит это
            var vanishWaitWas = VanishWait;
            VanishWait = TimeSpan.FromMilliseconds(800);
            try
            {
                Fresh();
                var dirLate = Path.Combine(root, "vanishlate");
                var lateRemoved = new List<int>();
                server.OnCard = n =>
                {
                    if (n != Count) return;
                    _ = Task.Run(async () =>
                    {
                        await Task.Delay(100);
                        foreach (var file in Directory.EnumerateFiles(Path.Combine(dirLate, TicketsFolder), "*", SearchOption.AllDirectories)
                                     .Where(f => TryIdOf(Path.GetFileName(f), out var id) && id > MassServer.IdOf(199)).Take(1).ToList())
                        {
                            File.Delete(file);
                            lock (lateRemoved) lateRemoved.Add(TryIdOf(Path.GetFileName(file), out var id) ? id : 0);
                        }
                    });
                };
                var lateRun = All(dirLate);
                lock (lateRemoved)
                    Debug.Assert(lateRemoved.Count == 1 && lateRun is { Complete: false } && lateRun.Error.Contains("Записано, но уже нет на диске: 1")
                        && lateRun.Error.Contains($"#{lateRemoved[0]}"));
            }
            finally { VanishWait = vanishWaitWas; }

            // 12. сервер условие по дате не применил (отдал всё): чтение обрывается за концом периода, на первой же странице
            Fresh();
            var until = new DateTime(2026, 1, 31);
            var inPeriod = Enumerable.Range(0, Count).Count(i => MassServer.CreatedOf(i) <= until.AddDays(1));
            var cut = All(Path.Combine(root, "cut"), query: new TaskQuery(Created: new(null, until)));
            Debug.Assert(cut is { Complete: true, Error: "" } && cut.Found == inPeriod && inPeriod < 200 && server.ListTargets.Count == 1);

            // 13. «не больше 210»: свежие по изменению первыми (двести с первой страницы и десять со второй), без сортировки по созданию
            Fresh();
            var dirLimit = Path.Combine(root, "limit");
            var limited = All(dirLimit, limit: 210);
            Debug.Assert(limited is { Found: 210, Created: 210, Complete: true } && server.ListTargets.Count == 2 && server.ListTargets[0].Contains("sort=Changed%20desc"));
            var limitedNames = OnDisk(dirLimit).Select(Path.GetFileName).ToList();
            Debug.Assert(limitedNames.Any(n => n!.StartsWith("10021 —")) && !limitedNames.Any(n => n!.StartsWith("10020 —")) && limitedNames.Any(n => n!.StartsWith("10230 —")));

            // 14. диск не пишет (на месте папки января лежит файл): после серии ошибок записи выгрузка прерывается
            Fresh();
            var dirBlock = Path.Combine(root, "block");
            Directory.CreateDirectory(Path.Combine(dirBlock, TicketsFolder));
            File.WriteAllText(Path.Combine(dirBlock, TicketsFolder, Month(0)), "я файл, а не папка");
            var blocked = All(dirBlock);
            Debug.Assert(blocked is { Complete: false, Created: 0 } && blocked.Failed is >= WriteFailLimit and < WriteFailLimit + 8);
            Debug.Assert(blocked.Error.Contains("файлы не записываются") && blocked.FirstError.Contains("файл не записан"));

            // 15. срок запроса у каждого свой: страница списка, не ответившая за срок, — сбой сети (его повторяют); отмена — не срок
            Fresh();
            server.ListDelayMs = 400;
            var late = client.GetTasksAsync(new TaskQuery(), 1, timeout: TimeSpan.FromMilliseconds(80)).GetAwaiter().GetResult();
            Debug.Assert(late.Error.StartsWith("сервер не ответил") && HttpIntraserviceClient.IsTransient(late.Error));
            Fresh();
            using var cancelled = new CancellationTokenSource();
            cancelled.Cancel();
            var threw = false;
            try { client.GetTasksAsync(new TaskQuery(), 1, cancelled.Token).GetAwaiter().GetResult(); }
            catch (OperationCanceledException) { threw = true; }
            Debug.Assert(threw);
            Fresh();
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
        /// <summary>Заявка с длинной перепиской (пять страниц).</summary>
        public const int LongId = 10002;

        public readonly Dictionary<int, string> Titles = new();
        public readonly Dictionary<int, TimeSpan> Bumps = new();
        public bool IgnoreSort, IgnorePage, ListUnauthorized, Overlap, NoCount;
        /// <summary>Потолок счёта по умолчанию (0 — считает всё): Count не больше него, и со счётом за ним список пуст — самое
        /// строгое прочтение документации (стр. 14). Без счёта (count=false) — весь список и HasNextPage вместо Count.</summary>
        public int CountCap;
        /// <summary>Список без счёта (count=false) — 400, как отказ проверки параметров; IgnoreNoCount — принят, но не понят:
        /// ответ как со счётом.</summary>
        public bool RejectNoCount, IgnoreNoCount;
        /// <summary>Со страницы FailFrom (0 — нет) список отвечает FailCode, с каким угодно счётом.</summary>
        public int FailFrom, FailCode;
        /// <summary>Счёт, который сервер называет (0 — настоящий): обещает больше, чем отдаст.</summary>
        public int ClaimCount;
        /// <summary>Порядок по созданию на стыке первых двух страниц не постоянный (равные даты): без счёта вторая страница
        /// начинается с последней заявки первой, а заявка, что шла за ней, не приходит вовсе.</summary>
        public bool UnstableBoundary;
        /// <summary>Список по изменению отдаётся одной и той же первой страницей, какую ни проси.</summary>
        public bool IgnorePageWhenChanged;
        /// <summary>Список по изменению без счёта (count=false) — 400 (второй проход), по созданию — как обычно.</summary>
        public bool RejectNoCountWhenChanged;
        /// <summary>Без счёта — без HasNextPage, а страница за концом списка — снова последняя.</summary>
        public bool ClampNoHasNext;
        /// <summary>Код ответа на список, отсортированный по созданию (0 — отвечает как обычно), и на любой список.</summary>
        public int RejectCreatedSort, FailLists;
        public int ListDelayMs;
        /// <summary>Номер захода: запрос с другим номером в адресе («/g3/api/…») — брошенный прежним заходом, его не считаем.</summary>
        public int Generation;
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

        /// <summary>Новый заход: счётчики, журнал и сбои — с нуля; запросы прежнего захода больше не считаются.</summary>
        public void NextGeneration()
        {
            Generation++;
            Cards = Lifetimes = Lists = 0;
            Log.Clear();
            ListTargets.Clear();
            Fault = null;
            OnCard = null;
            RejectCreatedSort = FailLists = 0;
            IgnoreSort = IgnorePage = ListUnauthorized = Overlap = NoCount = false;
            ListDelayMs = CountCap = 0;
            RejectNoCount = IgnoreNoCount = false;
            FailFrom = FailCode = ClaimCount = 0;
            UnstableBoundary = IgnorePageWhenChanged = RejectNoCountWhenChanged = ClampNoHasNext = false;
        }

        private static int Query(string target, string name, int fallback) =>
            Regex.Match(target, $@"[?&]{name}=(\d+)") is { Success: true } m ? int.Parse(m.Groups[1].Value, CultureInfo.InvariantCulture) : fallback;

        public (int Code, string Json) Respond(string target)
        {
            var prefix = Regex.Match(target, @"^/g(\d+)(/.*)$");
            if (prefix.Success)
            {
                target = prefix.Groups[2].Value;
                if (int.Parse(prefix.Groups[1].Value, CultureInfo.InvariantCulture) != Volatile.Read(ref Generation))
                    return (503, """{"Message":"запрос прежнего захода"}""");   // клиент его уже бросил — ответ никому не нужен
            }
            if (target.StartsWith("/api/taskstatus"))
                return (200, """[{"Id":29,"Name":"Выполнена","IsFixed":true}]""");
            if (target.StartsWith("/api/task?"))
            {
                Lists++;
                ListTargets.Add(target);
                var page = IgnorePage || IgnorePageWhenChanged && target.Contains("sort=Changed") ? 1 : Query(target, "page", 1);
                var size = NoCount ? 25 : Query(target, "pagesize", 25);   // NoCount: сервер сам режет страницу и счёта не присылает
                Log.Add("L" + page);
                if (ListDelayMs > 0) Thread.Sleep(ListDelayMs);
                if (ListUnauthorized) return (401, """{"Message":"Authorization has been denied"}""");
                if (FailLists > 0) return (FailLists, """{"Message":"bad request"}""");
                if (RejectCreatedSort > 0 && target.Contains("sort=Created")) return (RejectCreatedSort, """{"Message":"Invalid sort field"}""");
                if (RejectNoCountWhenChanged && target.Contains("count=false") && target.Contains("sort=Changed"))
                    return (400, """{"errors":{"count":["The value 'false' is not valid."]},"status":400}""");
                if (RejectNoCount && target.Contains("count=false")) return (400, """{"errors":{"count":["The value 'false' is not valid."]},"status":400}""");
                if (FailFrom > 0 && page >= FailFrom) return (FailCode, """{"Message":"page refused"}""");
                var ascending = !IgnoreSort && target.Contains("sort=Created%20asc");
                var counted = IgnoreNoCount || !target.Contains("count=false");
                var listed = counted && CountCap > 0 ? Math.Min(Count, CountCap) : Count;
                var order = (ascending ? Enumerable.Range(0, Count) : Enumerable.Range(0, Count).Reverse()).Take(listed);
                // Overlap: список «сдвинулся» — каждая следующая страница начинается с последней заявки предыдущей
                if (ClampNoHasNext && !counted) page = Math.Min(page, Math.Max(1, (listed + size - 1) / size));   // за концом — последняя
                var skip = (page - 1) * size - (Overlap && page > 1 ? 1 : 0);
                var picked = order.Skip(skip).Take(size).ToList();
                if (UnstableBoundary && ascending && !counted && page == 2 && picked.Count > 0) picked[0] = order.ElementAt(skip - 1);
                var rows = picked.Select(RowJson).ToList();
                var paginator = NoCount ? ""
                    : counted ? $",\"Paginator\":{{\"Count\":{(ClaimCount > 0 ? ClaimCount : listed)},\"Page\":{page},\"PageCount\":{(listed + size - 1) / size},\"PageSize\":{size},\"CountOnPage\":{rows.Count}}}"
                    : ClampNoHasNext ? $",\"Paginator\":{{\"Page\":{page},\"PageSize\":{size},\"CountOnPage\":{rows.Count}}}"
                    : $",\"Paginator\":{{\"Page\":{page},\"PageSize\":{size},\"CountOnPage\":{rows.Count},\"HasNextPage\":{(skip + rows.Count < listed ? "true" : "false")}}}";
                return (200, "{\"Tasks\":[" + string.Join(",", rows) + "],\"Statuses\":[{\"Id\":29,\"Name\":\"Выполнена\"}]" + paginator + "}");
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
                if (id == LongId)   // переписка на пять страниц (по 50 записей, свежие сверху)
                {
                    var at = Query(target, "page", 1);
                    var entries = Enumerable.Range(0, 50).Select(k => "{\"Date\":\"" + Iso(Start.AddDays(10).AddMinutes(-((at - 1) * 50 + k)))
                        + "\",\"Editor\":\"Иванов\",\"StatusId\":29,\"Comments\":\"<p>запись " + at + "-" + k + "</p>\",\"IsPublic\":true}");
                    return (200, "{\"TaskLifetimeList\":{\"TaskLifetimes\":[" + string.Join(",", entries) + "],"
                        + "\"Statuses\":[{\"Id\":29,\"Name\":\"Выполнена\"}],\"Paginator\":{\"Page\":" + at + ",\"PageCount\":5}}}");
                }
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
