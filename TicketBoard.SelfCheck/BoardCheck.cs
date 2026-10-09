using System.Diagnostics;
using System.Windows.Threading;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;
using static TicketBoard.SelfCheck.BoardServer;

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
    private static string Say(AutoSyncOutcome o) => o switch
    {
        AutoSyncOutcome.Done => "прошёл", AutoSyncOutcome.Failed => "не удался", AutoSyncOutcome.Dropped => "брошен", _ => "не начался",
    };

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


        var server = new BoardServer();
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
            var settings = new AppSettings { IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me" };
            HttpIntraserviceClient Client() => new(settings.IntraserviceBaseUrl, settings.IntraserviceLogin, "p");
            var board = new MainViewModel(new TicketStore(boardDir), settings, new IntraserviceLinkParser(settings), Client());
            List<DispatcherTimer> timers;   // реестр чистится в начале прогона (SingleThread.Run) — здесь только таймеры доски
            lock (DispatcherTimer.Created) timers = DispatcherTimer.Created.ToList();
            DispatcherTimer OneTimer(Func<DispatcherTimer, bool> which, string what) =>
                timers.Where(which).ToList() is [var one] ? one : throw new InvalidOperationException($"у доски не ровно один {what}");
            var saveTimer = OneTimer(t => t.Interval == TimeSpan.FromMilliseconds(600), "таймер сохранения (600 мс)");
            var syncTimer = OneTimer(t => t.IsEnabled && t.Interval == TimeSpan.FromSeconds(15),
                "включённый таймер автообновления с первым заходом через 15 с");

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
            // заход автообновления; уведомления этого захода. Заход ловит любое исключение («не удалось обновить») и бросает
            // себя молча (сменили учётную запись) — без сверки исхода упавший или брошенный заход сошёл бы за тихий
            var passes = 0;
            async Task<List<(string Title, string Text, Action? Click)>> Pass(AutoSyncOutcome expect = AutoSyncOutcome.Done)
            {
                var (from, logged) = (notes.Count, log.Count);
                passes++;
                var got = await board.AutoSyncAsync();
                Check($"заход {passes} {Say(expect)}", got == expect,
                    () => $"а он {Say(got)}: {board.AutoSyncState}\n" + string.Join("\n", log.Skip(logged)));
                return notes.Skip(from).ToList();
            }
            // что делает человек на доске: выбрал карточку, сделал, снял выбор (переписка выбранной не успевает загрузиться —
            // её 400 мс паузу снимает следующий выбор)
            void UserMoves(Ticket t, TicketStatus to) { board.SelectedTicket = t; board.MoveToCommand.Execute(to); board.SelectedTicket = null; }
            void UserDeletes(Ticket t) { board.SelectedTicket = t; board.DeleteSelectedCommand.Execute(null); board.SelectedTicket = null; }

            Check("доска сразу запоминает учётную запись в settings.json",
                AppSettings.Load(dataDir, out _).AutoSyncAccount == settings.AccountKey && settings.AutoSyncSkipIds is null);

            // 1. импорт: только мои открытые, карточка из строки списка; повторный ничего не трогает
            server.Put(101, "Принтер не печатает");
            server.Put(102, "Нет доступа к 1С", Working);
            server.Put(103, "Сломан сканер", Fixed);
            server.Put(104, "Чужая заявка", mine: false, executors: "Иванов И.");
            server.Put(107, "Отменённая", Cancelled);
            await board.ImportMineCommand.ExecuteAsync(null);
            Check("импорт: итог окном", Shown() == ("Импорт моих заявок", "Добавлено: 2, уже было: 0"), () => $"{Shown()}");
            Check("импорт заводит сохранение", saveTimer.IsEnabled);
            saveTimer.Fire();   // тик: tickets.json записан, таймер остановлен
            Check("импорт: спрошены я и только открытые статусы (закрытые — по признакам и по названию)",
                server.Last("/api/task?").Contains("ExecutorIds=7&StatusIds=31,32&"), () => server.Last("/api/task?"));
            Check("импорт: на доске мои открытые, без закрытых и чужих", Ids() == "101,102,199", Ids);
            var changed101 = At(server.ChangedOf(101));
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
            server.Change(101, t => t.StatusId = Final);
            server.Change(102, t => { t.Executors = "Я Сам, Сидоров С."; t.Name = "Нет доступа к 1С (срочно)"; });
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
            server.Change(101, t => t.StatusId = Fixed);
            AskWindow.Answer = (_, _) => true;    // «Перенести»
            await board.RefreshAllCommand.ExecuteAsync(null);
            Check("F5: закрыли иначе — спрашивает снова; «Перенести» — в «Готово»",
                Shown().Heading == "Перенести закрытые в «Готово»?" && Card(101) is { Status: TicketStatus.Done, CompletedAt: not null });
            Check("F5 закончился — флаг снят, заголовок обычный", !board.IsRefreshing && board.BoardTitle == "Заявки");

            // 3. первый заход автообновления — отсчёт: открытое моё, чего нет на доске, приносит импорт, а не он
            server.Put(105, "Заявка до автообновления");
            // первый заход — тиком таймера (его исход таймер не отдаёт): до него заголовок пуст, а «обновлено» в нём ставит
            // только дошедший до конца заход
            var fromNote = notes.Count;
            passes++;
            Check("до первого захода в заголовке ничего", board.AutoSyncState == "", () => board.AutoSyncState);
            syncTimer.Fire();
            Check("тик таймера — первый заход прошёл", await SingleThread.Until(() => board.AutoSyncState.Length > 0)
                && board.AutoSyncState.StartsWith("обновлено "), () => board.AutoSyncState + "\n" + string.Join("\n", log));
            Check("после первого захода таймер идёт дальше — раз в 5 минут", syncTimer.IsEnabled && syncTimer.Interval == TimeSpan.FromMinutes(5),
                () => $"{syncTimer.IsEnabled} {syncTimer.Interval}");
            var n = notes.Skip(fromNote).ToList();
            Check("первый заход: открытая до него не добавлена, уведомлений нет", Card(105) is null && n.Count == 0, () => Dump(n));
            Check("первый заход: её пропуск — в настройках и в settings.json",
                settings.AutoSyncSkipIds is [105] && AppSettings.Load(dataDir, out _).AutoSyncSkipIds is [105]);
            Check("первый заход: в заголовке «обновлено»", board.AutoSyncState.StartsWith("обновлено ")
                && !board.AutoSyncState.Contains("закрыты") && board.BoardTitle == $"Заявки · {board.AutoSyncState}", () => board.BoardTitle);
            var lifetimes = server.Count("/api/tasklifetime");
            n = await Pass();
            Check("заход без перемен: тихо, переписку не перечитывает", n.Count == 0 && server.Count("/api/tasklifetime") == lifetimes, () => Dump(n));
            Check("пропавшая с сервера — в лог один раз, а не каждый заход",
                log.Count(l => l.Contains("не удалось перечитать") && l.Contains("#199: заявка не найдена")) == 1, () => string.Join("\n", log));

            // 4. новая заявка на меня — во «Входящие» сверху и уведомление; щелчок показывает её
            server.Put(106, "Не работает VPN");
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
            server.Change(102, t => t.StatusId = Final);
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
            server.Change(102, t => t.StatusId = Open);
            n = await Pass();
            Check("снова открытая — из «Готово» во «Входящие»", Card(102) is { Status: TicketStatus.Inbox, AssignedToMe: true, CompletedAt: null });
            Check("…и уведомление", n is [("Заявку открыли снова", "Снова во «Входящих»:\n#102 Нет доступа к 1С", not null)], () => Dump(n));

            // 8. передали другому — «больше не на вас» с тем, на ком она теперь; карточка остаётся
            server.Change(105, t => { t.Mine = false; t.Executors = "Иванов И."; });
            n = await Pass();
            Check("переданная — уведомление «больше не на вас»",
                n is [("Заявка больше не на вас", "#105 Заявка до автообновления → теперь: Иванов И. (группа «Вторая линия»)", not null)],
                () => Dump(n));
            Check("…а карточка на месте", Card(105) is { Status: TicketStatus.Inbox, AssignedToMe: false });
            n = await Pass();
            Check("о передаче — один раз", n.Count == 0, () => Dump(n));

            // 9. комментарии: чужой — бейдж и уведомление, свой — бейдж гаснет; «прочитано» — только на активной доске
            server.Comment(106, "Сидоров С.", 9, "Проверил, роутер в порядке");
            n = await Pass();
            Check("чужой комментарий — бейдж и уведомление", Card(106)?.UnreadComments == 1
                && n is [("Новый комментарий в #106", "#106 Сидоров С.: Проверил, роутер в порядке", not null)], () => Dump(n));
            var mine = server.Comment(106, MeName, MeId, "Перезагрузите роутер, пожалуйста");
            n = await Pass();
            Check("свой ответ — бейдж гаснет, без уведомления", Card(106) is { UnreadComments: 0 } c106 && c106.CommentsSeenAt == At(mine)
                && n.Count == 0, () => Dump(n));
            var third = server.Comment(106, "Сидоров С.", 9, "Роутер перезагружен");
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
            server.Comment(106, "Сидоров С.", 9, "Жду логи");
            n = await Pass();
            Check("комментарий коллеги карточку «Ждёт ответа» не трогает", Card(106)?.Status == TicketStatus.Waiting
                && n is [("Новый комментарий в #106", "#106 Сидоров С.: Жду логи", _)], () => Dump(n));
            server.Comment(106, Requester, 5, "Всё равно не работает");
            n = await Pass();
            Check("ответ инициатора — снова «В работе»", Card(106)?.Status == TicketStatus.InProgress);
            Check("…и уведомление с его словами",
                n is [("Ответ инициатора в #106", "Снова «В работе»:\n#106 Петрова А.: Всё равно не работает", not null)], () => Dump(n));
            server.Comment(102, Requester, 5, "Спасибо, доступ появился");
            n = await Pass();
            Check("ответ инициатора в карточку не из «Ждёт ответа» её не двигает — только комментарий",
                Card(102)?.Status == TicketStatus.Inbox
                && n is [("Новый комментарий в #102", "#102 Петрова А.: Спасибо, доступ появился", not null)], () => Dump(n));
            UserMoves(Card(105)!, TicketStatus.Waiting);
            server.Comment(105, Requester, 5, "Можно закрывать");
            n = await Pass();
            Check("ответ инициатора в заявку, что уже не на мне, карточку «Ждёт ответа» не двигает",
                Card(105)?.Status == TicketStatus.Waiting
                && n is [("Новый комментарий в #105", "#105 Петрова А.: Можно закрывать", not null)], () => Dump(n));

            // 11. список оборвался на второй странице: новые добавляем, а «больше не на вас» по неполному списку не решаем
            server.BrokenPaging = true;
            server.Change(102, t => { t.Mine = false; t.Executors = "Кузнецов К."; });
            server.Put(108, "Новая во время сбоя");
            n = await Pass(AutoSyncOutcome.Failed);
            Check("неполный список: новая добавлена, передачу не объявляем",
                n is [("Новая заявка на вас", "#108 Новая во время сбоя", not null)] && Card(102)?.AssignedToMe == true, () => Dump(n));
            Check("неполный список: «не удалось обновить», причина — в лог",
                board.AutoSyncState == "не удалось обновить" && log.Count(l => l.StartsWith("Автообновление: ") && l.Contains("HTTP 503")) == 1,
                () => board.AutoSyncState + "\n" + string.Join("\n", log));
            n = await Pass(AutoSyncOutcome.Failed);
            Check("та же ошибка — в лог не повторяется", n.Count == 0 && log.Count(l => l.Contains("HTTP 503")) == 1, () => string.Join("\n", log));
            server.BrokenPaging = false;
            n = await Pass();
            Check("список снова целый — передачу видно",
                n is [("Заявка больше не на вас", "#102 Нет доступа к 1С → теперь: Кузнецов К. (группа «Вторая линия»)", not null)]
                && board.AutoSyncState.StartsWith("обновлено "), () => Dump(n) + "\n" + board.AutoSyncState);

            // 12. сервер лежит: «не удалось обновить», в логе один раз; поднялся — снова «обновлено»
            var logged = log.Count;
            server.FailList = 503;
            await Pass(AutoSyncOutcome.Failed);
            await Pass(AutoSyncOutcome.Failed);
            server.FailList = 0;
            Check("сервер лежит — «не удалось обновить», в логе один раз", board.AutoSyncState == "не удалось обновить" && log.Count == logged + 1,
                () => string.Join("\n", log.Skip(logged)));
            await Pass();
            Check("сервер вернулся — снова «обновлено»", board.AutoSyncState.StartsWith("обновлено "), () => board.AutoSyncState);
            Check("«кто я» спрошено один раз за запуск", server.Count("/api/user?getcurrentuserinfo=true") == 1);

            // 12б. несколько событий за заход — одно уведомление: в заголовке разделы, щелчок — действие первого (закрытые);
            // «Оставить» по щелчку тоже убирает карточку из счётчика в заголовке, и следующий заход её туда не вернёт
            server.Change(108, t => t.StatusId = Final);
            server.Put(111, "Ещё одна новая");
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
            server.Put(109, "Пришла, пока сохраняли настройки");
            var lists = server.Count("/api/task?");
            server.ListGate.Reset();
            var pass = Pass();
            Check("заход дошёл до списка — настройки сохраняются посреди него", await SingleThread.Until(() => server.Count("/api/task?") > lists));
            board.ApplySettings(Client());
            server.ListGate.Set();
            n = await pass;
            Check("сохранили настройки посреди захода — заход доведён", Card(109) is not null
                && n is [("Новая заявка на вас", "#109 Пришла, пока сохраняли настройки", _)], () => Dump(n));
            server.Put(110, "Чужой учётной записи");
            lists = server.Count("/api/task?");
            server.ListGate.Reset();
            pass = Pass(AutoSyncOutcome.Dropped);
            Check("заход дошёл до списка — учётная запись меняется посреди него", await SingleThread.Until(() => server.Count("/api/task?") > lists));
            settings.IntraserviceLogin = "other";
            board.ApplySettings(Client());
            server.ListGate.Set();
            n = await pass;
            Check("сменили учётную запись посреди захода — его итог не применён", Card(110) is null && n.Count == 0, () => Dump(n));
            Check("смена учётной записи: «была моей» и пропуски — заново, учётная запись — в settings.json",
                board.AllTickets.All(t => t.AssignedToMe is null) && settings.AutoSyncSkipIds is null
                && AppSettings.Load(dataDir, out _) is { AutoSyncSkipIds: null } file && file.AutoSyncAccount == settings.AccountKey);
            n = await Pass();
            Check("первый заход новой учётной записи — снова отсчёт: ни новых, ни «не на вас»",
                n.Count == 0 && Card(110) is null && settings.AutoSyncSkipIds is [110], () => Dump(n));

            int running;
            lock (DispatcherTimer.Created)
                running = DispatcherTimer.Created.Count(t => t.IsEnabled && t.Interval != TimeSpan.FromMilliseconds(600) && t.Interval != TimeSpan.FromMinutes(10));
            Check("таймер автообновления один, сколько ни сохраняй настройки", running == 1, () => $"идут: {running}");
            settings.AutoSyncMinutes = 0;
            board.ApplySettings(Client());
            Check("автообновление выключено — таймер стоит, в заголовке ни «обновлено», ни сбоя",
                !syncTimer.IsEnabled && board.AutoSyncState == "" && board.BoardTitle == "Заявки", () => board.BoardTitle);

            Check("доска менялась только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            Check("в логе нет исключений", !log.Any(l => l.Contains("Exception")), () => string.Join("\n", log));
            Check("каждая карточка — в колонке своего статуса, без повторов",
                board.Columns.All(c => c.Items.All(t => t.Status == c.Status)) && board.AllTickets.Count() == board.AllTickets.Distinct().Count());
            Check("сервер не получал неожиданных запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
        }
        finally
        {
            server.Dispose();
            App.DataDir = dataDirWas;
            AskWindow.Answer = answerWas;
            try { Directory.Delete(dataDir, recursive: true); } catch (IOException) { }
        }
        Debug.Assert(failures == 0, $"Доска: не прошло проверок — {failures} (список выше)");
    }
}
