using System.Globalization;
using System.Text.RegularExpressions;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using static TicketBoard.SelfCheck.BoardServer;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка зеркала (0.22.0): автовыгрузка с охватом «все закрытые» и «все заявки» (MainViewModel.AutoExportAsync →
/// ClosedExport) против своего BoardServer — что уходит (и чужие), сколько запросов за заход, отдых раз в час и «догоняю»,
/// уже верные файлы не тратят порцию (и у «моих»), смена охвата и отметка, вложения только моих. Правила без сервера —
/// ClosedExport.SelfCheck().</summary>
internal static class MirrorCheck
{
    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("зеркало");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        // часы сервера — двое суток назад, в окне первого захода (неделя)
        var now = DateTime.Now;
        using var server = new BoardServer(new DateTime(now.Year, now.Month, now.Day, now.Hour, now.Minute, 0).AddDays(-2));
        var (budgetWas, everyWas) = (ClosedExport.Budget, ClosedExport.MirrorEvery);
        using (var data = new TempDataDir("tb-mirror"))
        {
            try
            {
                // ---- сервер: закрытые мои и чужие, открытые, давняя ----
                server.Put(901, "Моя закрытая со скриншотом", Final);
                server.Quietly(901, t => t.Files.Add(new BoardServer.Attachment(9901, "экран.png", new byte[] { 1, 2 })));
                server.Put(902, "Чужая закрытая с отчётом", Final, mine: false, executors: "Иванов И.");
                server.Quietly(902, t => t.Files.Add(new BoardServer.Attachment(9902, "отчёт.pdf", new byte[] { 3 })));
                server.Put(903, "Чужая, где я писал", Working, mine: false, executors: "Иванов И.");
                server.Comment(903, MeName, MeId, "Посмотрел логи");
                server.StatusChange(903, Final, "Иванов И.", OtherId);
                server.Quietly(903, t => t.Files.Add(new BoardServer.Attachment(9903, "лог.txt", new byte[] { 4 })));
                server.Put(904, "Чужая открытая", Working, mine: false, executors: "Иванов И.");
                server.Put(905, "Моя открытая", Working);
                server.Put(906, "Давняя чужая закрытая", Final, mine: false, executors: "Иванов И.");
                server.SetChanged(906, now.AddDays(-8));

                var kb = Path.Combine(data.Path, "kb");
                var settings = new AppSettings
                {
                    IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me", KnowledgeDir = kb, AutoExportClosed = true,
                    AutoExportReach = ExportReach.Closed, KnowledgeAttachments = AttachmentScope.All,
                };
                var client = new HttpIntraserviceClient(server.Url, "me", "p");
                var board = new MainViewModel(new TicketStore(Path.Combine(data.Path, "board")), settings, new IntraserviceLinkParser(settings), client);
                BoardCheck.WatchBoard(pump, board);
                var logs = new List<string>();
                board.Log = logs.Add;
                string Log() => string.Join("\n", logs) + "\nтрей: " + board.AutoExportState;
                string? FileOf(int id) => Directory.Exists(Path.Combine(kb, "tickets"))
                    ? Directory.GetFiles(Path.Combine(kb, "tickets"), $"{id} — *.md", SearchOption.AllDirectories).SingleOrDefault() : null;
                string TextOf(int id) => FileOf(id) is { } path ? File.ReadAllText(path) : "нет файла";
                int Asked(string prefix) => server.Count(prefix);
                int Cards(int id) => Asked($"/api/task/{id}");
                int Lifetimes(int id) => Asked($"/api/tasklifetime?taskid={id}&");
                // запросы, что тратит заход: списки, карточки, истории, файлы вложений, справочник статусов
                (int Lists, int Cards, int Lifetimes, int Files, int Statuses) Spent() =>
                    (Asked("/api/task?"), Asked("/api/task/"), Asked("/api/tasklifetime"), Asked("/api/taskfile/"), Asked("/api/taskstatus"));
                string Since((int, int, int, int, int) was) => $"было {was}, стало {Spent()}";
                // с какой минуты просил список последний заход; «с недели назад» — с точностью до пары минут
                DateTime ListedSince() => DateTime.ParseExact(Uri.UnescapeDataString(server.Last("/api/task?").Split("ChangedMoreThan=")[1].Split('&')[0]),
                    "yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);
                bool FromWeekAgo() => Math.Abs((ListedSince() - DateTime.Now.AddDays(-7)).TotalMinutes) < 2;
                void Forget()   // выключили и включили: отметка забыта — как после полной выгрузки из «Поиска заявок» в новую базу
                {
                    settings.AutoExportClosed = false;
                    board.ApplySettings(client);
                    settings.AutoExportClosed = true;
                    board.ApplySettings(client);
                }

                // 1. все закрытые: и чужие, без чтения истории для решения; за заход — один список; вложения — только моих
                var was = Spent();
                Check("все закрытые: первый заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                foreach (var (id, why) in new[] { (901, "моя"), (902, "чужая"), (903, "чужая, где я только писал") })
                    Check($"выгружена: {why}", FileOf(id) is not null);
                foreach (var (id, why) in new[] { (904, "чужая открытая"), (905, "моя открытая"), (906, "закрытая раньше недели") })
                    Check($"не выгружена: {why}", FileOf(id) is null);
                Check("…один список: закрытые статусы, страница 500", Spent().Lists == was.Lists + 1 && server.Last("/api/task?").Contains("StatusIds=")
                    && server.Last("/api/task?").Contains("&pagesize=500&"), () => server.Last("/api/task?"));
                Check("…по карточке и истории на заявку — история чужой только для файла", new[] { 901, 902, 903 }.All(id => Cards(id) == 1 && Lifetimes(id) == 1)
                    && Spent().Cards == was.Cards + 3 && Spent().Lifetimes == was.Lifetimes + 3, () => Since(was));
                Check("вложения — моей и где я писал; чужой — нет, хоть настройка «всех выгружаемых»", Asked("/api/taskfile/9901") == 1
                    && Asked("/api/taskfile/9903") == 1 && Asked("/api/taskfile/9902") == 0, () => Since(was));
                Check("…у чужой в файле — «не скачано», роли нет; у моих — роль", TextOf(902).Contains("отчёт.pdf · не скачано")
                    && !TextOf(902).Contains("\nmy_role:") && TextOf(901).Contains("\nmy_role: \"исполнитель\"\n")
                    && TextOf(903).Contains("\nmy_role: \"участник\"\n"), () => TextOf(902));
                Check("…отметка — с охватом «все закрытые»", settings.AutoExportMarkReach == ExportReach.Closed && settings.AutoExportSince is not null,
                    () => $"{settings.AutoExportMarkReach}");
                Check("…подсказка трея: сколько выгружено, без «догоняю»", board.AutoExportState.StartsWith("база знаний: ")
                    && board.AutoExportState.Contains("выгружено 3") && !board.AutoExportState.Contains("догоняю"), () => board.AutoExportState);

                // 2. отдых: сразу после догнавшего захода — ни одного запроса; час прошёл — заход, и только список
                was = Spent();
                var tray = board.AutoExportState;
                Check("сразу после догнавшего захода — отдых", await board.AutoExportAsync() == AutoSyncOutcome.Skipped, Log);
                Check("…без единого запроса, подсказка трея прежняя", Spent() == was && board.AutoExportState == tray, () => Since(was));
                ClosedExport.MirrorEvery = TimeSpan.Zero;
                Check("час прошёл — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…только список: ни карточек, ни историй, ни файлов", Spent() == was with { Lists = was.Lists + 1, Statuses = was.Statuses + 1 },
                    () => Since(was));
                Check("…«новых нет»", board.AutoExportState.Contains("новых нет"), () => board.AutoExportState);

                // 3. порция 3 из семи новых: «догоняю» — и следующий заход сразу, без отдыха (3, 3, 1); догнал — отдых
                ClosedExport.Budget = 3;
                for (var id = 910; id <= 916; id++)
                    server.Put(id, $"Новая закрытая {id}", Final, mine: id % 2 == 0, executors: id % 2 == 0 ? MeName : "Иванов И.");
                var portions = new List<int>();
                foreach (var pass in new[] { 1, 2, 3 })
                {
                    was = Spent();
                    Check($"порция {pass} прошла", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                    portions.Add(Spent().Cards - was.Cards);
                    Check($"…«догоняю» — только пока не догнал ({pass})", board.AutoExportState.Contains("догоняю") == pass < 3, () => board.AutoExportState);
                    ClosedExport.MirrorEvery = everyWas;   // дальше — час: «догоняю» не ждёт его
                }
                Check("…порции 3, 3, 1 — по порядку изменения", portions.SequenceEqual(new[] { 3, 3, 1 })
                    && Enumerable.Range(910, 7).All(id => FileOf(id) is not null && Cards(id) == 1), () => string.Join(", ", portions));
                was = Spent();
                Check("догнал — отдых", await board.AutoExportAsync() == AutoSyncOutcome.Skipped && Spent() == was, () => Since(was));

                // 4. уже верные файлы порцию не тратят: отметку забыли (как после полной выгрузки из «Поиска заявок») — заход с
                // недели назад проходит десять выгруженных без карточек и историй и с порцией 3 не «догоняет»; изменённая — перечитана
                server.Comment(902, Requester, RequesterId, "А отчёт за сентябрь?");
                Forget();
                was = Spent();
                Check("после сброса — заход сразу, с недели назад", await board.AutoExportAsync() == AutoSyncOutcome.Done && FromWeekAgo(),
                    () => Log() + "\n" + server.Last("/api/task?"));
                Check("…только изменённая перечитана, остальные — без запросов", Spent().Cards == was.Cards + 1 && Cards(902) == 2
                    && Spent().Lifetimes == was.Lifetimes + 1 && FileOf(902) is { } f902 && File.ReadAllText(f902).Contains("отчёт за сентябрь"),
                    () => Since(was));
                Check("…не «догоняю», выгружена одна", board.AutoExportState.Contains("выгружено 1") && !board.AutoExportState.Contains("догоняю"),
                    () => board.AutoExportState);
                Check("…отметка — на последней", settings.AutoExportSince == At(server.ChangedOf(902)),
                    () => $"{settings.AutoExportSince:HH:mm:ss.fff} / {At(server.ChangedOf(902)):HH:mm:ss.fff}");

                // 5. охват расширили посреди захода: заход пишет отметку со своим охватом — и следующий («все заявки») забывает её:
                // с недели назад, без статусов в списке и без справочника, открытые выгружены
                server.Put(917, "Закрыта, пока меняли охват", Final, mine: false, executors: "Иванов И.");
                ClosedExport.MirrorEvery = TimeSpan.Zero;
                server.ListGate.Reset();
                var lists = Asked("/api/task?");
                var running = board.AutoExportAsync();
                await SingleThread.Until(() => Asked("/api/task?") > lists);
                settings.AutoExportReach = ExportReach.All;
                board.ApplySettings(client);
                server.ListGate.Set();
                Check("расширили охват посреди захода — заход закончился", await running == AutoSyncOutcome.Done && FileOf(917) is not null, Log);
                Check("…отметка — с охватом захода, «все закрытые»", settings.AutoExportMarkReach == ExportReach.Closed, () => $"{settings.AutoExportMarkReach}");
                ClosedExport.MirrorEvery = everyWas;
                was = Spent();
                ClosedExport.Budget = 2;   // ровно на две открытые: верный файл чужой 902 («не скачано» — по правилу зеркала) порцию не тратит
                Check("все заявки: заход сразу, хоть «все закрытые» отдыхают", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…уже верные (и чужая с «не скачано») порцию не тратят — не «догоняю»", !board.AutoExportState.Contains("догоняю"),
                    () => board.AutoExportState);
                ClosedExport.Budget = 3;
                Check("…с недели назад: отметка «все закрытые» им не годится", FromWeekAgo(), () => server.Last("/api/task?"));
                Check("…список без статусов, справочник не спрошен", !server.Last("/api/task?").Contains("StatusIds") && Spent().Statuses == was.Statuses,
                    () => server.Last("/api/task?"));
                Check("…открытые выгружены, давняя — нет; уже верные — без запросов", FileOf(904) is not null && FileOf(905) is not null
                    && FileOf(906) is null && Spent().Cards == was.Cards + 2, () => Since(was));
                Check("…отметка — с охватом «все заявки»", settings.AutoExportMarkReach == ExportReach.All, () => $"{settings.AutoExportMarkReach}");

                // 6. сузили до «все закрытые» — отметка годится: та же, не с недели назад; отдых «всех заявок» им не в счёт
                var markAll = settings.AutoExportSince;
                settings.AutoExportReach = ExportReach.Closed;
                board.ApplySettings(client);
                Check("сузили до всех закрытых — заход сразу", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…с той же отметки", !FromWeekAgo() && settings.AutoExportSince == markAll, () => server.Last("/api/task?"));

                // 7. мои закрытые после сброса: чужие с верным файлом — без чтения истории (раньше каждая читалась для разбора)
                settings.AutoExportReach = ExportReach.Mine;
                Forget();
                was = Spent();
                var read902 = Lifetimes(902);
                Check("мои после сброса — заход прошёл, с недели назад", await board.AutoExportAsync() == AutoSyncOutcome.Done && FromWeekAgo(), Log);
                Check("…страница и у моих — 500 строк: уже выгруженные проходятся одним заходом", server.Last("/api/task?").Contains("&pagesize=500&"),
                    () => server.Last("/api/task?"));
                // каждая прочитана один раз — для своего файла; и 902 больше не читается: в её файле вложение «не скачано», но роли в
                // нём нет — заявка не моя, и вложения ей не нужны (ревью, круг 2: раньше её история читалась ради разбора)
                Check("…чужие с верным файлом — без истории, карточек нет", new[] { 903, 911, 913, 915, 917 }.All(id => Lifetimes(id) == 1)
                    && Spent().Cards == was.Cards && Lifetimes(902) == read902 && Spent().Lifetimes == was.Lifetimes, () => Since(was));
                Check("…мои и в счёт не шли: «новых нет», не «догоняю»", board.AutoExportState.Contains("новых нет")
                    && !board.AutoExportState.Contains("догоняю"), () => board.AutoExportState);

                // 8. отметка без охвата (записана 0.21 и раньше) — «мои»: для всех закрытых не годится, с недели назад
                settings.AutoExportReach = ExportReach.Closed;
                settings.AutoExportMarkReach = null;
                board.ApplySettings(client);
                Check("отметка без охвата — все закрытые с недели назад", await board.AutoExportAsync() == AutoSyncOutcome.Done && FromWeekAgo(),
                    () => Log() + "\n" + server.Last("/api/task?"));

                // 9. список не отдался — «не удалось», отметка стоит; отдыха нет: следующий заход автообновления пробует снова (сервер
                // лежал — заработал, и зеркало не ждёт час), а отдыхает зеркало только после захода, что догнал без ошибок
                var mark = settings.AutoExportSince;
                server.FailList = 503;
                ClosedExport.MirrorEvery = TimeSpan.Zero;
                logs.Clear();
                Check("список не отдался — не удалось", await board.AutoExportAsync() == AutoSyncOutcome.Failed && settings.AutoExportSince == mark, Log);
                Check("…в подсказке трея — «не удалось»", board.AutoExportState.Contains("не удалось"), () => board.AutoExportState);
                ClosedExport.MirrorEvery = everyWas;
                was = Spent();
                Check("…следующий заход — сразу, без отдыха", await board.AutoExportAsync() == AutoSyncOutcome.Failed && Spent().Lists == was.Lists + 1,
                    () => Since(was));
                server.FailList = 0;
                Check("…сервер ожил — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                was = Spent();
                Check("…а после него — отдых", await board.AutoExportAsync() == AutoSyncOutcome.Skipped && Spent() == was, () => Since(was));
                ClosedExport.MirrorEvery = TimeSpan.Zero;

                // 10. заход дольше секунды — сколько длился, в подсказке трея
                server.Put(920, "Медленная карточка", Final, mine: false, executors: "Иванов И.");
                server.Quietly(920, t => t.CardWait = TimeSpan.FromMilliseconds(1100));
                Check("медленный заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                // секунда с лишним — «за 1 с», на медленной машине больше: число не сверяем
                Check("…в подсказке трея — сколько длился", Regex.IsMatch(board.AutoExportState, @"выгружено 1 за \d+ с(,|$)"), () => board.AutoExportState);
                // час отдыха — от начала догнавшего захода, а не от его конца: иначе заход по таймеру автообновления каждый раз
                // пропускал бы ближайший тик («раз в час» становился бы раз в час с тиком). Заход шёл дольше секунды — секунда отдыха
                // уже прошла
                ClosedExport.MirrorEvery = TimeSpan.FromSeconds(1);
                was = Spent();
                Check("отдых — от начала захода: секунда после медленного уже прошла", await board.AutoExportAsync() == AutoSyncOutcome.Done
                    && Spent().Lists == was.Lists + 1, () => Since(was));
                ClosedExport.MirrorEvery = TimeSpan.Zero;
                // ждал очереди писать (идёт выгрузка из «Поиска заявок») — это не время захода. Держим очередь 3 с с той минуты,
                // как список прочитан (дальше заход встаёт в очередь); сам заход куда быстрее 3 с — показанное время меньше
                server.Put(921, "Пока пишет окно поиска", Final, mine: false, executors: "Иванов И.");
                await KnowledgeExport.Writer.WaitAsync();
                lists = Asked("/api/task?");
                var waiting = board.AutoExportAsync();
                await SingleThread.Until(() => Asked("/api/task?") > lists);
                await Task.Delay(3000);
                KnowledgeExport.Writer.Release();
                Check("ждал очереди писать — заход прошёл", await waiting == AutoSyncOutcome.Done && FileOf(921) is not null, Log);
                Check("…ожидание — не время захода", board.AutoExportState.Contains("выгружено 1")
                    && !(Regex.Match(board.AutoExportState, @" за (\d+) с") is { Success: true } took && int.Parse(took.Groups[1].Value) >= 3)
                    && !board.AutoExportState.Contains(" мин"), () => board.AutoExportState);

                // 11. доска не тронута, всё — в своём потоке, лишних запросов нет
                Check("доска не тронута", !board.AllTickets.Any());
                Check("всё менялось в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
                Check("сервер не видел незнакомых запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
            }
            finally
            {
                ClosedExport.Budget = budgetWas;
                ClosedExport.MirrorEvery = everyWas;
                server.ListGate.Set();
            }
        }
        checks.AssertAll();
    }
}
