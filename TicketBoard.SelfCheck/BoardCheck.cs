using System.Diagnostics;
using System.Text.Json;
using System.Windows.Threading;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка доски и Интрасервиса: настоящий MainViewModel — импорт моих заявок, F5 («Обновить статусы») и
/// автообновление (новые на меня, первый заход-отсчёт, пропуски удалённых, закрытые, снова открытые, переданные, новые
/// комментарии и «прочитано», ответ инициатора, несколько событий в одном уведомлении, неполный список, сбой, смена настроек посреди захода) — против поддельного
/// сервера на loopback, где заявки меняются между заходами. Идёт в одном потоке со своим SynchronizationContext, как в
/// UI-потоке WPF: доска на это рассчитывает (и проверка следит, чтобы ничего на доске не менялось из другого потока).
/// Таймеры сами не тикают (WpfStubs.cs): сохранение и первый заход — тиком таймера доски (Fire), остальные заходы —
/// напрямую (AutoSyncAsync).</summary>
internal static class BoardCheck
{
    private const int MeId = 7;
    private const string MeName = "Я Сам";
    private const string Requester = "Петрова А.";
    private const string Phone = "+7 900 000-00-00";
    private const string Group = "Вторая линия";
    private const int Open = 31, Working = 32, Fixed = 29, Final = 30, Cancelled = 34;
    // «Выполнена» закрыта признаком IsFixed, «Закрыта» — IsFinal, «Отменена» — только по названию из ClosedStatusNames
    private const string Statuses = """[{"Id":31,"Name":"Открыта","IsFixed":false,"IsFinal":false},{"Id":32,"Name":"В работе"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта","IsFinal":true},{"Id":34,"Name":"Отменена"}]""";

    /// <summary>Заявка поддельного сервера. Mine — я в исполнителях (попадает в список «мои открытые»).</summary>
    private sealed class FakeTask(int id, string name)
    {
        public int Id { get; } = id;
        public string Name { get; set; } = name;
        public int StatusId { get; set; } = Open;
        public bool Mine { get; set; } = true;
        public string Executors { get; set; } = MeName;
        public DateTime Changed { get; set; }
        public List<(DateTime Date, string Editor, int EditorId, string Text)> Comments { get; } = new();
    }

    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var failures = 0;
        void Check(string name, bool ok, Func<string>? details = null)
        {
            if (ok) return;
            failures++;
            Console.Error.WriteLine($"  доска, не прошло: {name}" + (details is null ? "" : $"\n    {details().Replace("\n", "\n    ")}"));
        }
        static async Task<bool> Until(Func<bool> condition, int ms = 8000)
        {
            var sw = Stopwatch.StartNew();
            while (!condition() && sw.ElapsedMilliseconds < ms) await Task.Delay(20);
            return condition();
        }

        // ---- поддельный сервер: состояние меняет проверка (под lock), отвечает поток сервера ----
        var server = new object();
        var tasks = new Dictionary<int, FakeTask>();
        var asked = new List<string>();
        var unexpected = new List<string>();
        var clock = new DateTime(2026, 10, 1, 9, 0, 0);
        var failList = 0;            // список заявок отвечает этим кодом
        var brokenPaging = false;    // первая страница обещает больше, чем отдаёт, вторая — 503: список неполный
        // закрыты — список заявок ждёт, пока проверка их не откроет: настройки меняются ровно посреди захода, без гонки
        var listGate = new ManualResetEventSlim(true);

        (int, string) Respond(string target)
        {
            lock (server) asked.Add(target);
            if (target.StartsWith("/api/task?")) listGate.Wait(TimeSpan.FromSeconds(10));
            lock (server)
            {
                if (target == "/api/user?getcurrentuserinfo=true") return (200, $"{{\"Id\":{MeId},\"Name\":\"{MeName}\"}}");
                if (target == "/api/taskstatus") return (200, Statuses);
                if (target.StartsWith("/api/task?"))
                {
                    if (failList > 0) return (failList, "{\"Message\":\"Service Unavailable\"}");
                    var q = Query(target);
                    var page = int.Parse(q.GetValueOrDefault("page", "1"));
                    var size = int.Parse(q.GetValueOrDefault("pagesize", "50"));
                    var statusIds = q.GetValueOrDefault("StatusIds", "").Split(',', StringSplitOptions.RemoveEmptyEntries).Select(int.Parse).ToHashSet();
                    var mine = q.GetValueOrDefault("ExecutorIds") == $"{MeId}";
                    var rows = tasks.Values.Where(t => mine && t.Mine && statusIds.Contains(t.StatusId))
                        .OrderByDescending(t => t.Changed).ThenByDescending(t => t.Id).ToList();
                    if (brokenPaging)
                        return page == 1 ? (200, List(rows, rows.Count + 300, 1, 2)) : (503, "{\"Message\":\"Service Unavailable\"}");
                    return (200, List(rows.Skip((page - 1) * size).Take(size), rows.Count, page, Math.Max(1, (rows.Count + size - 1) / size)));
                }
                if (target.StartsWith("/api/task/") && int.TryParse(target["/api/task/".Length..].Split('?')[0], out var id))
                    return tasks.TryGetValue(id, out var t) ? (200, $"{{\"Task\":{Row(t)},\"Statuses\":{Statuses}}}") : (404, "{}");
                if (target.StartsWith("/api/tasklifetime?") && int.TryParse(Query(target).GetValueOrDefault("taskid"), out var lid))
                    return tasks.TryGetValue(lid, out var l) ? (200, Lifetime(l)) : (404, "{}");
                unexpected.Add(target);
                return (404, "{}");
            }
        }

        DateTime Tick() { lock (server) return clock = clock.AddMinutes(1); }
        void Put(int id, string name, int status = Open, bool mine = true, string executors = MeName)
        {
            lock (server) tasks[id] = new FakeTask(id, name) { StatusId = status, Mine = mine, Executors = executors, Changed = Tick() };
        }
        void Change(int id, Action<FakeTask> change) { lock (server) { change(tasks[id]); tasks[id].Changed = Tick(); } }
        DateTime Comment(int id, string editor, int editorId, string text)
        {
            lock (server)
            {
                var at = Tick();
                tasks[id].Comments.Add((at, editor, editorId, text));
                tasks[id].Changed = at;
                return at;
            }
        }
        DateTime ChangedOf(int id) { lock (server) return tasks[id].Changed; }
        int Count(string prefix) { lock (server) return asked.Count(t => t.StartsWith(prefix)); }
        string Last(string prefix) { lock (server) return asked.LastOrDefault(t => t.StartsWith(prefix)) ?? ""; }
        static DateTimeOffset At(DateTime d) => new(d);

        var (listener, port) = FakeIntraservice.Start(Respond);
        var dataDir = Path.Combine(Path.GetTempPath(), $"tb-board-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dataDir);
        var dataDirWas = App.DataDir;
        var answerWas = AskWindow.Answer;
        App.DataDir = dataDir;
        try
        {
            var boardDir = Path.Combine(dataDir, "board");
            // карточка, заведённая руками давно: на сервере такой заявки уже нет — F5 и автообновление её не перечитают
            new TicketStore(boardDir).Save(new[] { new Ticket { Title = "Потерянная", IntraserviceId = 199, Status = TicketStatus.InProgress } });
            var settings = new AppSettings { IntraserviceBaseUrl = $"http://127.0.0.1:{port}", IntraserviceLogin = "me" };
            HttpIntraserviceClient Client() => new(settings.IntraserviceBaseUrl, settings.IntraserviceLogin, "p");
            int timersBefore;
            lock (DispatcherTimer.Created) timersBefore = DispatcherTimer.Created.Count;
            var board = new MainViewModel(new TicketStore(boardDir), settings, new IntraserviceLinkParser(settings), Client());
            List<DispatcherTimer> timers;
            lock (DispatcherTimer.Created) timers = DispatcherTimer.Created.Skip(timersBefore).ToList();
            var saveTimer = timers.Single(t => t.Interval == TimeSpan.FromMilliseconds(600));
            var syncTimer = timers.Single(t => t.Interval == TimeSpan.FromSeconds(15));   // первый заход — вскоре после запуска
            Check("автообновление включено — первый заход по таймеру", syncTimer.IsEnabled);

            // доска рассчитывает на один поток (UI-поток WPF): её свойства, колонки, карточки и переписка меняются только в нём
            var watched = new HashSet<Ticket>();
            void WatchCard(Ticket t) { if (watched.Add(t)) pump.Watch(t); }
            pump.Watch(board);
            pump.WatchItems(board.Comments);
            foreach (var c in board.Columns)
            {
                pump.Watch(c);
                pump.WatchItems(c.Items);
                c.Items.CollectionChanged += (_, e) => { foreach (var t in e.NewItems?.OfType<Ticket>() ?? []) WatchCard(t); };
                foreach (var t in c.Items) WatchCard(t);
            }

            var notes = new List<(string Title, string Text, Action? Click)>();
            var log = new List<string>();
            var revealed = new List<Ticket>();
            board.Notify += (title, text, click) => notes.Add((title, text, click));
            board.Log = log.Add;
            board.RevealRequested += revealed.Add;

            Ticket? Card(int id) => board.AllTickets.FirstOrDefault(t => t.IntraserviceId == id);
            string Ids() => string.Join(",", board.AllTickets.Select(t => t.IntraserviceId).Order());
            (string Heading, string Text) Shown() { lock (AskWindow.Shown) return AskWindow.Shown.Count > 0 ? AskWindow.Shown[^1] : ("", ""); }
            static string Dump(IEnumerable<(string Title, string Text, Action? Click)> list) =>
                string.Join("\n", list.Select(n => $"[{n.Title}] {n.Text}")) is { Length: > 0 } s ? s : "(уведомлений нет)";
            // заход автообновления; уведомления этого захода. Заход ловит любое исключение и пишет «не удалось обновить» —
            // без проверки исхода упавший заход сошёл бы за тихий; fails — заход, который и должен не удаться
            var passes = 0;
            async Task<List<(string Title, string Text, Action? Click)>> Pass(bool fails = false)
            {
                var (from, logged) = (notes.Count, log.Count);
                passes++;
                await board.AutoSyncAsync();
                if (!fails)
                    Check($"заход {passes} прошёл без сбоя", board.AutoSyncState.StartsWith("обновлено "),
                        () => board.AutoSyncState + "\n" + string.Join("\n", log.Skip(logged)));
                return notes.Skip(from).ToList();
            }
            // что делает человек на доске: выбрал карточку, сделал, снял выбор (переписка выбранной не успевает загрузиться —
            // её 400 мс паузу снимает следующий выбор)
            void UserMoves(Ticket t, TicketStatus to) { board.SelectedTicket = t; board.MoveToCommand.Execute(to); board.SelectedTicket = null; }
            void UserDeletes(Ticket t) { board.SelectedTicket = t; board.DeleteSelectedCommand.Execute(null); board.SelectedTicket = null; }

            Check("доска сразу запоминает учётную запись в settings.json",
                AppSettings.Load(dataDir, out _).AutoSyncAccount == settings.AccountKey && settings.AutoSyncSkipIds is null);

            // 1. импорт: только мои открытые, карточка из строки списка; повторный ничего не трогает
            Put(101, "Принтер не печатает");
            Put(102, "Нет доступа к 1С", Working);
            Put(103, "Сломан сканер", Fixed);
            Put(104, "Чужая заявка", mine: false, executors: "Иванов И.");
            Put(107, "Отменённая", Cancelled);
            await board.ImportMineCommand.ExecuteAsync(null);
            Check("импорт: итог окном", Shown() == ("Импорт моих заявок", "Добавлено: 2, уже было: 0"), () => $"{Shown()}");
            Check("импорт заводит сохранение", saveTimer.IsEnabled);
            saveTimer.Fire();   // тик: tickets.json записан, таймер остановлен
            Check("импорт: спрошены я и только открытые статусы (закрытые — по признакам и по названию)",
                Last("/api/task?").Contains("ExecutorIds=7&StatusIds=31,32&"), () => Last("/api/task?"));
            Check("импорт: на доске мои открытые, без закрытых и чужих", Ids() == "101,102,199", Ids);
            var changed101 = At(ChangedOf(101));
            Check("импорт: карточка из строки списка — во «Входящих», переписка до неё прочитана",
                Card(101) is { Title: "Принтер не печатает", ExternalStatus: "Открыта", Creator: Requester, CreatorPhone: Phone,
                    Executors: MeName, ExecutorGroup: Group, Priority: TicketPriority.Mid, Status: TicketStatus.Inbox, AssignedToMe: true } c101
                && c101.Url == settings.TicketUrl(101) && c101.CommentsCheckedFor == changed101 && c101.CommentsSeenAt == changed101.AddSeconds(1));
            UserMoves(Card(102)!, TicketStatus.InProgress);
            Check("перенос руками заводит сохранение", saveTimer.IsEnabled);
            await board.ImportMineCommand.ExecuteAsync(null);
            Check("повторный импорт: всё уже было", Shown() == ("Импорт моих заявок", "Добавлено: 0, уже было: 2"), () => $"{Shown()}");
            Check("повторный импорт не двигает карточку, которую перенесли руками", Card(102)?.Status == TicketStatus.InProgress);
            saveTimer.Fire();
            var reloaded = new MainViewModel(new TicketStore(boardDir), new AppSettings(), new IntraserviceLinkParser(new AppSettings()), null);
            Check("после перезапуска карточки на тех же местах, «моя» и «прочитано до» сохранены",
                reloaded.AllTickets.FirstOrDefault(t => t.IntraserviceId == 101) is { Status: TicketStatus.Inbox, AssignedToMe: true } r101
                && r101.CommentsSeenAt == changed101.AddSeconds(1)
                && reloaded.AllTickets.FirstOrDefault(t => t.IntraserviceId == 102)?.Status == TicketStatus.InProgress);

            // 2. F5: статусы и люди обновляются, о закрытой — вопрос; «Оставить» помнит карточка, пока статус тот же
            Change(101, t => t.StatusId = Final);
            Change(102, t => { t.Executors = "Я Сам, Сидоров С."; t.Name = "Нет доступа к 1С (срочно)"; });
            AskWindow.Answer = (_, _) => false;   // «Оставить»
            await board.RefreshAllCommand.ExecuteAsync(null);
            var (heading, text) = Shown();
            Check("F5: спрашивает о закрытой, перечисляя её", heading == "Перенести закрытые в «Готово»?"
                && text.EndsWith("\n\nЗакрыты в Интрасервисе (1):\n#101  Принтер не печатает"), () => text);
            Check("F5: в вопросе — итог с ошибкой по пропавшей на сервере",
                text.StartsWith("Обновлено: 2, не удалось: 1. Первая ошибка — #199: заявка не найдена"), () => text);
            Check("F5: «Оставить» — карточка на месте и помнит статус", Card(101) is { Status: TicketStatus.Inbox, KeptOpenStatus: "Закрыта" });
            Check("F5: исполнители обновлены, а название, данное карточке, — нет",
                Card(102) is { Executors: "Я Сам, Сидоров С.", Title: "Нет доступа к 1С", Status: TicketStatus.InProgress });
            await board.RefreshAllCommand.ExecuteAsync(null);
            Check("F5 ещё раз: об оставленной не спрашивает — только итог",
                Shown().Heading == "Обновление статусов" && Shown().Text.StartsWith("Обновлено: 2, не удалось: 1."), () => $"{Shown()}");
            Change(101, t => t.StatusId = Fixed);
            AskWindow.Answer = (_, _) => true;    // «Перенести»
            await board.RefreshAllCommand.ExecuteAsync(null);
            Check("F5: закрыли иначе — спрашивает снова; «Перенести» — в «Готово»",
                Shown().Heading == "Перенести закрытые в «Готово»?" && Card(101) is { Status: TicketStatus.Done, CompletedAt: not null });
            Check("F5 закончился — флаг снят, заголовок обычный", !board.IsRefreshing && board.BoardTitle == "Заявки");

            // 3. первый заход автообновления — отсчёт: открытое моё, чего нет на доске, приносит импорт, а не он
            Put(105, "Заявка до автообновления");
            var fromNote = notes.Count;
            syncTimer.Fire();
            Check("тик таймера — заход прошёл", await Until(() => board.AutoSyncState.Length > 0) && board.AutoSyncState.StartsWith("обновлено "),
                () => board.AutoSyncState + "\n" + string.Join("\n", log));
            Check("после первого захода таймер — раз в 5 минут", syncTimer is { IsEnabled: true } && syncTimer.Interval == TimeSpan.FromMinutes(5));
            var n = notes.Skip(fromNote).ToList();
            Check("первый заход: открытая до него не добавлена, уведомлений нет", Card(105) is null && n.Count == 0, () => Dump(n));
            Check("первый заход: её пропуск — в настройках и в settings.json",
                settings.AutoSyncSkipIds is [105] && AppSettings.Load(dataDir, out _).AutoSyncSkipIds is [105]);
            Check("первый заход: в заголовке «обновлено»", board.AutoSyncState.StartsWith("обновлено ")
                && !board.AutoSyncState.Contains("закрыты") && board.BoardTitle == $"Заявки · {board.AutoSyncState}", () => board.BoardTitle);
            var lifetimes = Count("/api/tasklifetime");
            n = await Pass();
            Check("заход без перемен: тихо, переписку не перечитывает", n.Count == 0 && Count("/api/tasklifetime") == lifetimes, () => Dump(n));
            Check("пропавшая с сервера — в лог один раз, а не каждый заход",
                log.Count(l => l.Contains("не удалось перечитать") && l.Contains("#199: заявка не найдена")) == 1, () => string.Join("\n", log));

            // 4. новая заявка на меня — во «Входящие» сверху и уведомление; щелчок показывает её
            Put(106, "Не работает VPN");
            n = await Pass();
            Check("новая на меня — во «Входящие», сверху", board.ColumnFor(TicketStatus.Inbox).Items.FirstOrDefault()?.IntraserviceId == 106
                && Card(106) is { AssignedToMe: true, Title: "Не работает VPN" });
            Check("новая на меня — уведомление", n is [("Новая заявка на вас", "#106 Не работает VPN", not null)], () => Dump(n));
            Check("пропущенная при отсчёте так и не добавлена", Card(105) is null);
            if (n is [var added]) added.Click?.Invoke();
            Check("щелчок по уведомлению — карточка выбрана и показана",
                revealed.LastOrDefault() == Card(106) && board.SelectedTicket == Card(106) && board.IsPanelOpen);
            board.SelectedTicket = null;
            board.IsPanelOpen = false;

            // 5. удалённую с доски автообновление не возвращает; импорт возвращает и снимает пропуск
            UserDeletes(Card(106)!);
            Check("удаление — с вопросом, и номер в пропусках", Shown().Heading == "Удалить заявку с доски?" && Card(106) is null
                && settings.AutoSyncSkipIds is [105, 106]);
            n = await Pass();
            Check("удалённую автообновление не возвращает", Card(106) is null && n.Count == 0, () => Dump(n));
            await board.ImportMineCommand.ExecuteAsync(null);
            Check("импорт возвращает и удалённую, и открытую до автообновления",
                Shown() == ("Импорт моих заявок", "Добавлено: 2, уже было: 1") && Card(105) is not null && Card(106) is not null, () => $"{Shown()}");
            Check("…и снимает их пропуск", settings.AutoSyncSkipIds is [] && AppSettings.Load(dataDir, out _).AutoSyncSkipIds is []);
            n = await Pass();
            Check("после импорта заход тихий", n.Count == 0, () => Dump(n));

            // 6. закрыли в Интрасервисе: уведомление один раз, счётчик в заголовке; сам не переносит, щелчок — вопрос F5
            Change(102, t => t.StatusId = Final);
            var closedNote = await Pass();
            Check("закрытая в Интрасервисе — уведомление",
                closedNote is [("Заявка закрыта в Интрасервисе", "Щёлкните, чтобы перенести в «Готово»:\n#102 Нет доступа к 1С", not null)],
                () => Dump(closedNote));
            Check("…сама карточка не переносится", Card(102) is { Status: TicketStatus.InProgress, ExternalStatus: "Закрыта" });
            Check("…и счётчик в заголовке", board.AutoSyncState.EndsWith(" · закрыты в Интрасервисе: 1 — F5"), () => board.AutoSyncState);
            n = await Pass();
            Check("о закрытой — один раз, а не каждый заход", n.Count == 0 && board.AutoSyncState.EndsWith("закрыты в Интрасервисе: 1 — F5"),
                () => Dump(n) + "\n" + board.AutoSyncState);
            if (closedNote is [var closed]) closed.Click?.Invoke();
            Check("щелчок — тот же вопрос, что у F5, и перенос",
                Shown() == ("Перенести закрытые в «Готово»?", "Закрыты в Интрасервисе (1):\n#102  Нет доступа к 1С")
                && Card(102)?.Status == TicketStatus.Done, () => $"{Shown()}");
            Check("перенесли — счётчик из заголовка ушёл", board.AutoSyncState.StartsWith("обновлено ") && !board.AutoSyncState.Contains("закрыты"),
                () => board.AutoSyncState);

            // 7. снова открыли, пока карточка в «Готово» — обратно во «Входящие»
            Change(102, t => t.StatusId = Open);
            n = await Pass();
            Check("снова открытая — из «Готово» во «Входящие»", Card(102) is { Status: TicketStatus.Inbox, AssignedToMe: true, CompletedAt: null });
            Check("…и уведомление", n is [("Заявку открыли снова", "Снова во «Входящих»:\n#102 Нет доступа к 1С", not null)], () => Dump(n));

            // 8. передали другому — «больше не на вас» с тем, на ком она теперь; карточка остаётся
            Change(105, t => { t.Mine = false; t.Executors = "Иванов И."; });
            n = await Pass();
            Check("переданная — уведомление «больше не на вас»",
                n is [("Заявка больше не на вас", "#105 Заявка до автообновления → теперь: Иванов И. (группа «Вторая линия»)", not null)],
                () => Dump(n));
            Check("…а карточка на месте", Card(105) is { Status: TicketStatus.Inbox, AssignedToMe: false });
            n = await Pass();
            Check("о передаче — один раз", n.Count == 0, () => Dump(n));

            // 9. комментарии: чужой — бейдж и уведомление, свой — бейдж гаснет; «прочитано» — только на активной доске
            Comment(106, "Сидоров С.", 9, "Проверил, роутер в порядке");
            n = await Pass();
            Check("чужой комментарий — бейдж и уведомление", Card(106)?.UnreadComments == 1
                && n is [("Новый комментарий в #106", "#106 Сидоров С.: Проверил, роутер в порядке", not null)], () => Dump(n));
            var mine = Comment(106, MeName, MeId, "Перезагрузите роутер, пожалуйста");
            n = await Pass();
            Check("свой ответ — бейдж гаснет, без уведомления", Card(106) is { UnreadComments: 0 } c106 && c106.CommentsSeenAt == At(mine)
                && n.Count == 0, () => Dump(n));
            var third = Comment(106, "Сидоров С.", 9, "Роутер перезагружен");
            n = await Pass();
            Check("ещё чужой — снова бейдж", Card(106)?.UnreadComments == 1 && n.Count == 1, () => Dump(n));
            board.SelectedTicket = Card(106);
            Check("открыта в панели неактивной доски — ещё не прочитана", board.IsPanelOpen && Card(106)?.UnreadComments == 1);
            board.IsBoardActive = true;
            Check("доска перед глазами — прочитана до самого нового", Card(106) is { UnreadComments: 0 } seen && seen.CommentsSeenAt == At(third));
            board.IsBoardActive = false;
            board.SelectedTicket = null;
            board.IsPanelOpen = false;

            // 10. «Ждёт ответа»: комментарий коллеги её не трогает, ответ инициатора — снова «В работе»
            UserMoves(Card(106)!, TicketStatus.Waiting);
            Comment(106, "Сидоров С.", 9, "Жду логи");
            n = await Pass();
            Check("комментарий коллеги карточку «Ждёт ответа» не трогает", Card(106)?.Status == TicketStatus.Waiting
                && n is [("Новый комментарий в #106", "#106 Сидоров С.: Жду логи", _)], () => Dump(n));
            Comment(106, Requester, 5, "Всё равно не работает");
            n = await Pass();
            Check("ответ инициатора — снова «В работе»", Card(106)?.Status == TicketStatus.InProgress);
            Check("…и уведомление с его словами",
                n is [("Ответ инициатора в #106", "Снова «В работе»:\n#106 Петрова А.: Всё равно не работает", not null)], () => Dump(n));
            Comment(102, Requester, 5, "Спасибо, доступ появился");
            n = await Pass();
            Check("ответ инициатора в карточку не из «Ждёт ответа» её не двигает — только комментарий",
                Card(102)?.Status == TicketStatus.Inbox
                && n is [("Новый комментарий в #102", "#102 Петрова А.: Спасибо, доступ появился", not null)], () => Dump(n));
            UserMoves(Card(105)!, TicketStatus.Waiting);
            Comment(105, Requester, 5, "Можно закрывать");
            n = await Pass();
            Check("ответ инициатора в заявку, что уже не на мне, карточку «Ждёт ответа» не двигает",
                Card(105)?.Status == TicketStatus.Waiting
                && n is [("Новый комментарий в #105", "#105 Петрова А.: Можно закрывать", not null)], () => Dump(n));

            // 11. список оборвался на второй странице: новые добавляем, а «больше не на вас» по неполному списку не решаем
            brokenPaging = true;
            Change(102, t => { t.Mine = false; t.Executors = "Кузнецов К."; });
            Put(108, "Новая во время сбоя");
            n = await Pass(fails: true);
            Check("неполный список: новая добавлена, передачу не объявляем",
                n is [("Новая заявка на вас", "#108 Новая во время сбоя", not null)] && Card(102)?.AssignedToMe == true, () => Dump(n));
            Check("неполный список: «не удалось обновить», причина — в лог",
                board.AutoSyncState == "не удалось обновить" && log.Count(l => l.StartsWith("Автообновление: ") && l.Contains("HTTP 503")) == 1,
                () => board.AutoSyncState + "\n" + string.Join("\n", log));
            n = await Pass(fails: true);
            Check("та же ошибка — в лог не повторяется", n.Count == 0 && log.Count(l => l.Contains("HTTP 503")) == 1, () => string.Join("\n", log));
            brokenPaging = false;
            n = await Pass();
            Check("список снова целый — передачу видно",
                n is [("Заявка больше не на вас", "#102 Нет доступа к 1С → теперь: Кузнецов К. (группа «Вторая линия»)", not null)]
                && board.AutoSyncState.StartsWith("обновлено "), () => Dump(n) + "\n" + board.AutoSyncState);

            // 12. сервер лежит: «не удалось обновить», в логе один раз; поднялся — снова «обновлено»
            var logged = log.Count;
            failList = 503;
            await Pass(fails: true);
            await Pass(fails: true);
            failList = 0;
            Check("сервер лежит — «не удалось обновить», в логе один раз", board.AutoSyncState == "не удалось обновить" && log.Count == logged + 1,
                () => string.Join("\n", log.Skip(logged)));
            await Pass();
            Check("сервер вернулся — снова «обновлено»", board.AutoSyncState.StartsWith("обновлено "), () => board.AutoSyncState);
            Check("«кто я» спрошено один раз за запуск", Count("/api/user?getcurrentuserinfo=true") == 1);

            // 12б. несколько событий за заход — одно уведомление: в заголовке разделы, щелчок — действие первого (закрытые);
            // «Оставить» по щелчку тоже убирает карточку из счётчика в заголовке, и следующий заход её туда не вернёт
            Change(108, t => t.StatusId = Final);
            Put(111, "Ещё одна новая");
            n = await Pass();
            Check("два события за заход — одно уведомление с обоими разделами", n is [("Заявки — закрыты: 1 · новые: 1",
                "Щёлкните, чтобы перенести в «Готово»:\n#108 Новая во время сбоя\nНовые: #111 Ещё одна новая", not null)], () => Dump(n));
            int ShownCount() { lock (AskWindow.Shown) return AskWindow.Shown.Count; }
            var shownBefore = ShownCount();
            AskWindow.Answer = (_, _) => false;   // «Оставить»
            if (n is [var both]) both.Click?.Invoke();
            AskWindow.Answer = (_, _) => true;
            Check("щелчок — вопрос о закрытой; «Оставить» — карточка на месте, счётчик ушёл",
                ShownCount() == shownBefore + 1 && Shown().Heading == "Перенести закрытые в «Готово»?"
                && Card(108) is { Status: TicketStatus.Inbox, KeptOpenStatus: "Закрыта" }
                && board.AutoSyncState.StartsWith("обновлено ") && !board.AutoSyncState.Contains("закрыты"), () => board.AutoSyncState);
            n = await Pass();
            Check("оставленная не возвращается ни в уведомления, ни в счётчик", n.Count == 0 && !board.AutoSyncState.Contains("закрыты"),
                () => Dump(n) + "\n" + board.AutoSyncState);

            // 13. посреди захода сохранили настройки: тот же адрес и логин — заход доводится; другой логин — его итог выброшен
            Put(109, "Пришла, пока сохраняли настройки");
            var lists = Count("/api/task?");
            listGate.Reset();
            var pass = Pass();
            Check("заход дошёл до списка — настройки сохраняются посреди него", await Until(() => Count("/api/task?") > lists));
            board.ApplySettings(Client());
            listGate.Set();
            n = await pass;
            Check("сохранили настройки посреди захода — заход доведён", Card(109) is not null
                && n is [("Новая заявка на вас", "#109 Пришла, пока сохраняли настройки", _)], () => Dump(n));
            Put(110, "Чужой учётной записи");
            lists = Count("/api/task?");
            listGate.Reset();
            pass = Pass();
            Check("заход дошёл до списка — учётная запись меняется посреди него", await Until(() => Count("/api/task?") > lists));
            settings.IntraserviceLogin = "other";
            board.ApplySettings(Client());
            listGate.Set();
            n = await pass;
            Check("сменили учётную запись посреди захода — его итог не применён", Card(110) is null && n.Count == 0, () => Dump(n));
            Check("смена учётной записи: «была моей» и пропуски — заново, учётная запись — в settings.json",
                board.AllTickets.All(t => t.AssignedToMe is null) && settings.AutoSyncSkipIds is null
                && AppSettings.Load(dataDir, out _) is { AutoSyncSkipIds: null } file && file.AutoSyncAccount == settings.AccountKey);
            n = await Pass();
            Check("первый заход новой учётной записи — снова отсчёт: ни новых, ни «не на вас»",
                n.Count == 0 && Card(110) is null && settings.AutoSyncSkipIds is [110], () => Dump(n));

            settings.AutoSyncMinutes = 0;
            board.ApplySettings(Client());
            Check("автообновление выключено — таймер стоит, в заголовке ни «обновлено», ни сбоя",
                !syncTimer.IsEnabled && board.AutoSyncState == "" && board.BoardTitle == "Заявки", () => board.BoardTitle);

            Check("доска менялась только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            Check("в логе нет исключений", !log.Any(l => l.Contains("Exception")), () => string.Join("\n", log));
            Check("каждая карточка — в колонке своего статуса, без повторов",
                board.Columns.All(c => c.Items.All(t => t.Status == c.Status)) && board.AllTickets.Count() == board.AllTickets.Distinct().Count());
            Check("сервер не получал неожиданных запросов", unexpected.Count == 0, () => string.Join("\n", unexpected));
        }
        finally
        {
            listGate.Set();
            listener.Stop();
            App.DataDir = dataDirWas;
            AskWindow.Answer = answerWas;
            try { Directory.Delete(dataDir, recursive: true); } catch (IOException) { }
        }
        Debug.Assert(failures == 0, $"Доска: не прошло проверок — {failures} (список выше)");
    }

    private static Dictionary<string, string> Query(string target)
    {
        var q = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var at = target.IndexOf('?');
        if (at < 0) return q;
        foreach (var pair in target[(at + 1)..].Split('&', StringSplitOptions.RemoveEmptyEntries))
        {
            var kv = pair.Split('=', 2);
            q[kv[0]] = kv.Length > 1 ? Uri.UnescapeDataString(kv[1]) : "";
        }
        return q;
    }

    private static string J(string s) => JsonSerializer.Serialize(s);

    private static string Row(FakeTask t) =>
        $"{{\"Id\":{t.Id},\"Name\":{J(t.Name)},\"StatusId\":{t.StatusId},\"Creator\":{J(Requester)},\"CreatorPhone\":{J(Phone)},"
        + $"\"Executors\":{J(t.Executors)},\"ExecutorGroup\":{J(Group)},\"Created\":\"2026-09-01T10:00:00\",\"Changed\":\"{t.Changed:s}\"}}";

    private static string List(IEnumerable<FakeTask> rows, int count, int page, int pages) =>
        $"{{\"Tasks\":[{string.Join(",", rows.Select(Row))}],\"Statuses\":{Statuses},"
        + $"\"Paginator\":{{\"Count\":{count},\"Page\":{page},\"PageCount\":{pages}}}}}";

    /// <summary>Переписка, свежие сверху (lastcommentsontop=true).</summary>
    private static string Lifetime(FakeTask t) =>
        "{\"TaskLifetimeList\":{\"TaskLifetimes\":[" + string.Join(",", t.Comments.OrderByDescending(c => c.Date).Select(c =>
            $"{{\"Date\":\"{c.Date:s}\",\"Editor\":{J(c.Editor)},\"EditorId\":{c.EditorId},\"Comments\":{J(c.Text)},\"IsPublic\":true,\"StatusId\":{t.StatusId}}}"))
        + "],\"Paginator\":{\"Page\":1,\"PageCount\":1}}}";
}
