using System.Diagnostics;
using System.Windows.Threading;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;
using static TicketBoard.SelfCheck.BoardServer;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка карточки и панели — того, что человек делает на доске руками: быстрое добавление (окно и разбор
/// введённого), ⟳ одной карточки, дочитывание людей у старых карточек, переписка в панели (пауза, память, ⟳, строки и чипы
/// статуса, скрытые смены статуса, внутренние, «есть ещё», сообщения, «прочитано» — и свежий комментарий от
/// автообновления), заметки, фильтры и счётчики колонок, лимит «В работе», клавиши (стрелки, приоритет, панель, ссылка,
/// удаление с «Отмена»), возраст карточки, доска без API. Настоящие MainViewModel и QuickCaptureViewModel против
/// поддельного сервера (BoardServer), в одном потоке (SingleThread), как BoardCheck. Окна настроек здесь нет: его проверка
/// хоткея — на типах WPF (Key), и подделка их проверяла бы саму себя.</summary>
internal static class CardCheck
{
    private const string EmptyHint = "Ссылка вида …/Task/View/702180 или просто номер заявки";

    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var failures = 0;
        void Check(string name, bool ok, Func<string>? details = null)
        {
            if (ok) return;
            failures++;
            Console.Error.WriteLine($"  карточка, не прошло: {name}" + (details is null ? "" : $"\n    {details().Replace("\n", "\n    ")}"));
        }

        var server = new BoardServer();
        var dataDir = Path.Combine(Path.GetTempPath(), $"tb-card-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dataDir);
        var (dataDirWas, answerWas, clipboardWas) = (App.DataDir, AskWindow.Answer, ClipboardWatcher.Last);
        App.DataDir = dataDir;
        try
        {
            // ---- сервер ----
            server.Put(7011, "Не та заявка", mine: false, executors: "Иванов И.");
            server.Put(70112, "Принтер не печатает", description: "Не печатает с утра");
            server.Comment(70112, "Сидоров С.", 9, "Проверил, бумага есть");
            server.StatusChange(70112, Working);
            server.Comment(70112, Requester, 5, "Всё ещё не печатает");
            var internalNote = server.Comment(70112, MeName, MeId, "Заказал картридж", isPublic: false);
            server.Change(70112, t => t.MoreEvents = true);
            server.Put(70114, "Нет доступа к 1С", description: "Пишет «нет лицензии»");
            server.Put(70115, "Только смены статуса");
            server.StatusChange(70115, Working);
            server.StatusChange(70115, Open);
            server.Put(70117, "Переписка закрыта");
            server.Change(70117, t => t.EventsRefused = true);
            server.Put(7020, "Старая заявка", Working);
            server.Put(7021, "Без контактов");
            server.Change(7021, t => t.NoPhone = true);

            // ---- доска: старая карточка без людей (до 0.8.0), карточка без контактов (до 0.9.0), заведённая руками ----
            var boardDir = Path.Combine(dataDir, "board");
            new TicketStore(boardDir).Save(new[]
            {
                new Ticket { Title = "Старая", IntraserviceId = 7020, ExternalStatus = "Открыта", Status = TicketStatus.InProgress },
                new Ticket { Title = "Без контактов", IntraserviceId = 7021, Executors = MeName, Status = TicketStatus.InProgress },
                new Ticket { Title = "Позвонить Иванову", Status = TicketStatus.Waiting },
            });
            var settings = new AppSettings { IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me", AutoSyncSkipIds = new[] { 70114 } };
            var client = new HttpIntraserviceClient(server.Url, "me", "p");
            var parser = new IntraserviceLinkParser(settings);
            var board = new MainViewModel(new TicketStore(boardDir), settings, parser, client);
            List<DispatcherTimer> timers;
            lock (DispatcherTimer.Created) timers = DispatcherTimer.Created.ToList();
            var saveTimer = timers.Single(t => t.Interval == TimeSpan.FromMilliseconds(600));
            var ageTimer = timers.Single(t => t.Interval == TimeSpan.FromMinutes(10));
            BoardCheck.WatchBoard(pump, board);

            Ticket Card(string title) => board.AllTickets.First(t => t.Title == title);
            ColumnViewModel Col(TicketStatus s) => board.ColumnFor(s);
            List<Ticket> Visible(TicketStatus s) => Col(s).View.Cast<Ticket>().ToList();
            (string Heading, string Text) Shown() { lock (AskWindow.Shown) return AskWindow.Shown.Count > 0 ? AskWindow.Shown[^1] : ("", ""); }
            void UserMoves(Ticket t, TicketStatus to) { board.SelectedTicket = t; board.MoveToCommand.Execute(to); board.SelectedTicket = null; }
            string Lifetimes(int id) => $"/api/tasklifetime?taskid={id}&";

            Check("доска открылась: «Входящие» пусты при непустой доске — подсказка; подписи лимита и скрытых",
                Col(TicketStatus.Inbox) is { VisibleCount: 0, ShowEmptyHint: true } && Col(TicketStatus.InProgress) is { VisibleCount: 2, Hint: "лимит 5" }
                && Col(TicketStatus.Done).Hint == "скрыты старше 7 д" && !board.IsBoardEmpty);

            // 1. окно быстрого добавления: подсказка, номер, название по номеру после паузы — только по последнему номеру
            var qc = new QuickCaptureViewModel(parser, settings, client);
            Check("быстрое добавление: при первом открытии — подсказка, номера нет",
                qc is { Text: "", HasNumber: false, NumberText: "", Preview: "", DigitsSetPriority: false } && qc.Hint == EmptyHint, () => $"«{qc.Hint}»");
            foreach (var typed in new[] { "70", "701", "7011", "70112" }) qc.Text = typed;   // набирают по цифре
            Check("номер распознан, ищем название", qc is { HasNumber: true, NumberText: "#70112", DigitsSetPriority: false, Preview: "Ищу в Интрасервисе…" }
                && qc.Hint == "Будет создана заявка с этим названием", () => $"{qc.NumberText} · {qc.Preview} · {qc.Hint}");
            Check("название — после паузы и только по последнему номеру", await SingleThread.Until(() => qc.Preview == "Принтер не печатает")
                && server.Count("/api/task/7011?") == 0 && server.Count("/api/task/70112?") == 1, () => qc.Preview);
            qc.Text = "70112 ";
            await Task.Delay(500);
            Check("тот же номер — второй раз не ищем", qc.Preview == "Принтер не печатает" && server.Count("/api/task/70112?") == 1);
            qc.Text = $"{server.Url}/Task/View/70113";
            Check("ссылка: номер из неё, цифры 1/2/3 — приоритет", qc is { HasNumber: true, NumberText: "#70113", DigitsSetPriority: true });
            Check("нет такой заявки — одной строкой", await SingleThread.Until(() => qc.Preview.StartsWith("заявка не найдена"))
                && !qc.Preview.Contains('\n'), () => qc.Preview);
            qc.Priority = TicketPriority.High;
            qc.Reset();
            Check("сброс: пусто, «средний», подсказка", qc is { Text: "", Priority: TicketPriority.Mid, HasNumber: false, Preview: "" } && qc.Hint == EmptyHint);
            var qcOff = new QuickCaptureViewModel(parser, settings, null);
            qcOff.Text = "70112";
            await Task.Delay(500);
            Check("без API название не ищем", qcOff.Preview == "" && server.Count("/api/task/70112?") == 1);
            qcOff.ApplySettings(client);
            qcOff.Text = "70112 ";
            Check("API настроили — тот же номер ищем заново", await SingleThread.Until(() => qcOff.Preview == "Принтер не печатает"), () => qcOff.Preview);

            // 2. Enter в окне — AddFromCapture: номер, ссылка и название из введённого; с номером — сразу синхронизация
            var c1 = board.AddFromCapture($"{server.Url}/Task/View/70112 — не печатает на 3 этаже", TicketPriority.High);
            Check("ссылка с текстом: номер и ссылка — из ссылки, название — из текста",
                c1 is { IntraserviceId: 70112, Title: "не печатает на 3 этаже", Priority: TicketPriority.High, Status: TicketStatus.Inbox }
                && c1.Url == $"{server.Url}/Task/View/70112", () => $"{c1.Title} · {c1.Url}");
            Check("новая — сверху во «Входящих», выбрана, панель открыта, «обновляю…»", Col(TicketStatus.Inbox).Items.FirstOrDefault() == c1
                && board.SelectedTicket == c1 && board.IsPanelOpen && board.SyncMessage == "обновляю…", () => board.SyncMessage);
            Check("после синхронизации: статус, описание и люди с сервера, своё название остаётся",
                await SingleThread.Until(() => board.SyncMessage == "" && c1.ExternalStatus is not null)
                && c1 is { ExternalStatus: "В работе", Description: "Не печатает с утра", Title: "не печатает на 3 этаже", Creator: Requester, Executors: MeName },
                () => $"{c1.ExternalStatus} · {c1.Description} · {board.SyncMessage}");
            var c2 = board.AddFromCapture("70114", TicketPriority.Mid);
            Check("голый номер: ссылка из адреса сервера, название пока «Заявка #N»",
                c2 is { IntraserviceId: 70114, Title: "Заявка #70114" } && c2.Url == settings.TicketUrl(70114), () => $"{c2.Title} · {c2.Url}");
            Check("…номер снят с пропусков автообновления — и в settings.json",
                settings.AutoSyncSkipIds is [] && AppSettings.Load(dataDir, out _).AutoSyncSkipIds is []);
            Check("…после синхронизации название и описание — с сервера",
                await SingleThread.Until(() => c2.Title == "Нет доступа к 1С") && c2.Description == "Пишет «нет лицензии»", () => c2.Title);
            var taskReads = server.Count("/api/task/");
            var c3 = board.AddFromCapture("Купить картридж", TicketPriority.Low);
            Check("текст без номера: карточка без ссылки и без запросов",
                c3 is { IntraserviceId: null, Url: "", Title: "Купить картридж", Priority: TicketPriority.Low }
                && board.SyncMessage == "" && server.Count("/api/task/") == taskReads);
            var c4 = board.AddFromCapture("№ 70115", TicketPriority.Mid);
            Check("«№ 70115»: номер есть, название — с сервера",
                c4.IntraserviceId == 70115 && await SingleThread.Until(() => c4.Title == "Только смены статуса"), () => c4.Title);
            var c5 = board.AddKnown(70117, "", "Переписка закрыта", TicketPriority.Mid);
            var c6 = board.AddKnown(70119, "", "Удалённая", TicketPriority.Mid);
            Check("удалённая на сервере — в панели «заявка не найдена»",
                await SingleThread.Until(() => board.SyncMessage.StartsWith("заявка не найдена (HTTP 404)")), () => board.SyncMessage);
            await SingleThread.Until(() => c5.ExternalStatus is not null);

            // 3. ⟳ в панели — одна карточка
            server.Change(70114, t => t.StatusId = Working);
            board.SelectedTicket = c2;
            await board.RefreshFromIntraserviceCommand.ExecuteAsync(null);
            Check("⟳: статус обновлён, сообщение пустое", c2.ExternalStatus == "В работе" && board.SyncMessage == "", () => board.SyncMessage);
            board.SelectedTicket = Card("Позвонить Иванову");
            await board.RefreshFromIntraserviceCommand.ExecuteAsync(null);
            Check("⟳ у карточки без номера — так и сказано", board.SyncMessage == "у заявки нет номера", () => board.SyncMessage);

            // 4. у старой карточки люди дочитываются при открытии — раз за запуск и только люди
            var old = Card("Старая");
            board.SelectedTicket = old;
            Check("старая карточка без людей — дочитаны при открытии", await SingleThread.Until(() => old.Executors is not null)
                && old is { Creator: Requester, CreatorPhone: Phone, Executors: MeName, ExecutorGroup: Group });
            Check("…только люди: статус не тронут", old.ExternalStatus == "Открыта", () => $"{old.ExternalStatus}");
            var noPhone = Card("Без контактов");
            board.SelectedTicket = noPhone;
            board.SelectedTicket = old;
            board.SelectedTicket = noPhone;
            await SingleThread.Until(() => server.Count("/api/task/7021?") == 1);
            await Task.Delay(300);
            Check("контактов на сервере нет — второй раз за запуск не спрашиваем",
                server.Count("/api/task/7021?") == 1 && server.Count("/api/task/7020?") == 1 && noPhone.CreatorContacts == "");
            var c1Reads = server.Count("/api/task/70112?");
            board.SelectedTicket = c1;
            await Task.Delay(100);
            Check("люди известны — при открытии не спрашиваем", server.Count("/api/task/70112?") == c1Reads);

            // 5. переписка в панели
            board.SelectedTicket = null;
            var l12 = server.Count(Lifetimes(70112));
            board.SelectedTicket = c1;
            Check("переписка грузится — после паузы", board.CommentsMessage == "загружаю…" && server.Count(Lifetimes(70112)) == l12);
            Check("…и пришла", await SingleThread.Until(() => board.Comments.Count == 4), () => board.CommentsMessage);
            Check("строки — свежие сверху; внутренний отмечен; чип статуса — где он сменился и у самой старой",
                board.Comments.ToList() is [{ Author: MeName, Text: "Заказал картридж", IsInternal: true, StatusChange: null },
                    { Author: Requester, Text: "Всё ещё не печатает", IsInternal: false, StatusChange: null },
                    { Author: "Система", Text: null, StatusChange: "В работе" },
                    { Author: "Сидоров С.", Text: "Проверил, бумага есть", StatusChange: "Открыта" }],
                () => string.Join("\n", board.Comments));
            Check("смена статуса без текста скрыта — видно 3; «есть ещё»", board is { HiddenEventsCount: 1, VisibleCommentsCount: 3, CommentsTruncated: true, CommentsMessage: "" }
                && board.CommentsView.Cast<object>().Count() == 3);
            board.ShowAllEvents = true;
            Check("«глаз» — видно все 4", board.VisibleCommentsCount == 4 && board.CommentsView.Cast<object>().Count() == 4);
            board.ShowAllEvents = false;
            board.SelectedTicket = c3;
            Check("без номера — так и сказано", board.CommentsMessage == "у заявки нет номера" && board.Comments.Count == 0);
            var l14 = server.Count(Lifetimes(70114));
            board.SelectedTicket = c2;
            board.SelectedTicket = c4;   // пробежали стрелкой мимо c2
            Check("только смены статуса — так и сказано", await SingleThread.Until(() => board.CommentsMessage == "только смены статуса"), () => board.CommentsMessage);
            Check("мимо чего пробежали — не спрашивали", server.Count(Lifetimes(70114)) == l14);
            board.SelectedTicket = c2;
            Check("переписки нет — так и сказано", await SingleThread.Until(() => board.CommentsMessage == "переписки нет"), () => board.CommentsMessage);
            board.SelectedTicket = c5;
            Check("переписку не дают — ответ сервера в панели",
                await SingleThread.Until(() => board.CommentsMessage.StartsWith("нет доступа (HTTP 403)")), () => board.CommentsMessage);
            l12 = server.Count(Lifetimes(70112));
            board.SelectedTicket = c1;
            Check("снова открыли — из памяти, сразу и без запроса",
                board.Comments.Count == 4 && board.CommentsMessage == "" && server.Count(Lifetimes(70112)) == l12);
            board.RefreshCommentsCommand.Execute(null);
            Check("⟳ переписки — мимо памяти и без паузы", board.CommentsMessage == "загружаю…"
                && await SingleThread.Until(() => server.Count(Lifetimes(70112)) == l12 + 1 && board.Comments.Count == 4), () => board.CommentsMessage);

            // «прочитано»: панель открыта на активной доске — до самого нового комментария; свежий от автообновления — нет
            c1.UnreadComments = 2;
            board.SelectedTicket = null;
            board.IsPanelOpen = false;
            board.IsBoardActive = true;
            board.SelectedTicket = c1;
            Check("доска перед глазами, панель открыта — прочитано до самого нового", c1.UnreadComments == 0 && c1.CommentsSeenAt == At(internalNote));
            Check("первый заход автообновления на этой доске прошёл", await board.AutoSyncAsync() == AutoSyncOutcome.Done, () => board.AutoSyncState);
            var fresh = server.Comment(70112, "Сидоров С.", 9, "Картридж привезли");
            Check("свежий комментарий — заход прошёл", await board.AutoSyncAsync() == AutoSyncOutcome.Done, () => board.AutoSyncState);
            Check("свежий комментарий открытой карточки — сразу в панели, но не прочитан: смотрит ли человек, неизвестно",
                board.Comments.FirstOrDefault()?.Text == "Картридж привезли" && c1.UnreadComments == 1, () => $"{c1.UnreadComments}");
            board.MarkCommentsSeen();   // щелчок или клавиша на доске (MainWindow)
            Check("…действие на доске — прочитан", c1.UnreadComments == 0 && c1.CommentsSeenAt == At(fresh));
            board.IsBoardActive = false;

            // 6. заметки
            saveTimer.Fire();
            board.SelectedTicket = c1;
            board.NewNoteText = "  позвонить после обеда  ";
            board.AddNoteCommand.Execute(null);
            board.NewNoteText = "   ";
            board.AddNoteCommand.Execute(null);
            board.NewNoteText = "картридж заказан";
            board.AddNoteCommand.Execute(null);
            Check("заметки: обрезаны, пустая не добавлена, новая — сверху, поле очищено",
                c1.Notes is [{ Text: "картридж заказан" }, { Text: "позвонить после обеда" }] && board.NewNoteText == "" && c1.NotesCount == 2);
            board.DeleteNoteCommand.Execute(c1.Notes[1]);
            Check("заметку удалили", c1.Notes is [{ Text: "картридж заказан" }] && c1.NotesCount == 1);
            Check("заметки заводят сохранение", saveTimer.IsEnabled);
            saveTimer.Fire();
            var reloaded = new MainViewModel(new TicketStore(boardDir), new AppSettings(), new IntraserviceLinkParser(new AppSettings()), null);
            Check("заметка — на диске", reloaded.AllTickets.FirstOrDefault(t => t.IntraserviceId == 70112)?.Notes is [{ Text: "картридж заказан" }]);
            board.SelectedTicket = null;

            // 7. фильтры и счётчики
            UserMoves(c3, TicketStatus.Done);
            c3.CompletedAt = DateTimeOffset.Now.AddDays(-10);
            UserMoves(c4, TicketStatus.Done);
            ageTimer.Fire();   // раз в 10 минут доска пересчитывает возраст и фильтры
            Check("«Готово»: старше 7 дней — скрыта, подпись об этом",
                Col(TicketStatus.Done) is { VisibleCount: 1, Hint: "скрыты старше 7 д" } && Visible(TicketStatus.Done) is [var recent] && recent == c4);
            board.HideOldDone = false;
            Check("…«показать все» — видны обе, подписи нет", Col(TicketStatus.Done) is { VisibleCount: 2, Hint: "" });
            board.HideOldDone = true;
            board.SearchText = "КАРТРИДЖ";
            Check("поиск без учёта регистра и по заметкам; скрытая старая не всплывает",
                Visible(TicketStatus.Inbox) is [var byNote] && byNote == c1 && Col(TicketStatus.Done).VisibleCount == 0);
            board.SearchText = "#70114";
            Check("поиск по номеру", Visible(TicketStatus.Inbox) is [var byNumber] && byNumber == c2);
            board.SearchText = "с утра";
            Check("поиск по описанию", Visible(TicketStatus.Inbox) is [var byText] && byText == c1);
            board.SearchText = "Task/View/70117";
            Check("поиск по ссылке", Visible(TicketStatus.Inbox) is [var byUrl] && byUrl == c5);
            board.SearchText = "иванову";
            Check("во «Входящих» ничего, в других есть — подсказка «всё в работе»",
                Col(TicketStatus.Inbox) is { VisibleCount: 0, ShowEmptyHint: true } && Col(TicketStatus.Waiting).VisibleCount == 1 && !board.IsBoardEmpty);
            board.SearchText = "нет такой";
            Check("ничего не нашлось — доска пуста, без подсказки", board.IsBoardEmpty && !Col(TicketStatus.Inbox).ShowEmptyHint);
            board.SearchText = "";
            board.PriorityFilter = MainViewModel.PriorityFilters[1];   // «Высокий»
            Check("фильтр по приоритету", Visible(TicketStatus.Inbox) is [var high] && high == c1 && Col(TicketStatus.InProgress).VisibleCount == 0);
            board.PriorityFilter = MainViewModel.PriorityFilters[0];
            foreach (var t in new[] { c1, c2, c5, c6 }) UserMoves(t, TicketStatus.InProgress);
            Check("в «В работе» больше лимита — перегруз", Col(TicketStatus.InProgress) is { VisibleCount: 6, IsOverloaded: true, IsOverloadedTotal: true });
            board.SearchText = "Старая";
            Check("поиск сузил колонку — видимых в лимите, а всего (трей) — перегруз",
                Col(TicketStatus.InProgress) is { VisibleCount: 1, IsOverloaded: false, IsOverloadedTotal: true });
            board.SearchText = "";

            // 8. клавиши: стрелки, приоритет, панель, ссылка, удаление с «Отмена»
            board.SelectedTicket = c2;
            board.MoveSelected(+1);
            Check("→ — следующая колонка", c2.Status == TicketStatus.Waiting && board.SelectedStatus == TicketStatus.Waiting && Col(TicketStatus.Waiting).Items.Contains(c2));
            board.MoveSelected(+5);
            Check("→ дальше края — «Готово»", c2.Status == TicketStatus.Done && c2.CompletedAt is not null);
            board.MoveSelected(-10);
            Check("← дальше края — «Входящие»", c2.Status == TicketStatus.Inbox && c2.CompletedAt is null && Col(TicketStatus.Inbox).Items.Contains(c2));
            board.SelectedStatus = TicketStatus.InProgress;
            Check("статус в панели — тот же перенос", c2.Status == TicketStatus.InProgress && Col(TicketStatus.InProgress).Items.Contains(c2)
                && !Col(TicketStatus.Inbox).Items.Contains(c2));
            board.SetSelectedPriority(TicketPriority.Low);
            Check("1/2/3 — приоритет выбранной", c2.Priority == TicketPriority.Low);
            board.IsPanelOpen = true;
            board.TogglePanel();
            var closed = !board.IsPanelOpen;
            board.TogglePanel();
            Check("пробел — панель закрыть и открыть", closed && board.IsPanelOpen);
            board.ClosePanelCommand.Execute(null);
            Check("× — панель закрыта", !board.IsPanelOpen);
            board.CopyLinkCommand.Execute(null);
            Check("ссылка — в буфер", ClipboardWatcher.Last == c2.Url && c2.Url.Length > 0);
            AskWindow.Answer = (_, _) => false;   // «Отмена»
            board.DeleteSelectedCommand.Execute(null);
            Check("удаление с «Отмена» — на месте; в вопросе номер и что в Интрасервисе она останется", board.AllTickets.Contains(c2)
                && Shown() == ("Удалить заявку с доски?", "#70114  «Нет доступа к 1С»\n\nВ Интрасервисе она останется."), () => $"{Shown()}");
            board.SelectedTicket = Card("Позвонить Иванову");
            board.DeleteSelectedCommand.Execute(null);
            Check("…у карточки без номера — только название", Shown() == ("Удалить заявку с доски?", "«Позвонить Иванову»"), () => $"{Shown()}");
            AskWindow.Answer = (_, _) => true;
            board.ClosePanelCommand.Execute(null);   // Esc: сначала закрывает панель, второй Esc снимает выбор
            board.SelectedTicket = null;
            board.TogglePanel();
            Check("без выбранной карточки панель не открыть", !board.IsPanelOpen);

            // 9. возраст карточки: «сегодня», дни, жёлтый за день до порога, красный с порога (3 дня); «Готово» не краснеет
            var aged = new Ticket { Title = "возраст" };
            string Age(int days, TicketStatus status = TicketStatus.Inbox)
            {
                aged.Status = status;
                aged.StatusChangedAt = DateTimeOffset.Now.AddDays(-days);
                return $"{aged.AgeLabel} | {aged.AgeText} | {aged.AgeState}";
            }
            foreach (var (days, expected) in new[]
            {
                (0, "сегодня | с сегодня | Neutral"), (1, "1 д | 1 день | Neutral"), (2, "2 д | 2 дня | Warn"),
                (3, "3 д | 3 дня — просрочена | Overdue"), (5, "5 д | 5 дней — просрочена | Overdue"),
                (11, "11 д | 11 дней — просрочена | Overdue"), (21, "21 д | 21 день — просрочена | Overdue"),
            })
                Check($"возраст {days} д: {expected}", Age(days) == expected, () => Age(days));
            Check("в «Готово» возраст не краснеет", Age(30, TicketStatus.Done).EndsWith("| Neutral"), () => Age(30, TicketStatus.Done));

            // 10. доска без API: везде — куда идти настраивать
            var offDir = Path.Combine(dataDir, "offline");
            new TicketStore(offDir).Save(new[] { new Ticket { Title = "С номером", IntraserviceId = 70112 } });
            var offline = new MainViewModel(new TicketStore(offDir), new AppSettings(), new IntraserviceLinkParser(new AppSettings()), null);
            offline.SelectedTicket = offline.AllTickets.First();
            Check("без API: переписка — так и сказано", offline.CommentsMessage == "API не настроен");
            await offline.RefreshFromIntraserviceCommand.ExecuteAsync(null);
            Check("…⟳ — куда идти", offline.SyncMessage == "API не настроен: трей → Настройки…");
            await offline.ImportMineCommand.ExecuteAsync(null);
            Check("…импорт — то же окном", Shown() == ("Импорт моих заявок", "API не настроен: трей → Настройки…"));
            await offline.RefreshAllCommand.ExecuteAsync(null);
            Check("…F5 — то же", Shown() == ("Обновление статусов", "API не настроен: трей → Настройки…"));
            Check("…автообновление не начинается, заголовок без «обновлено»",
                await offline.AutoSyncAsync() == AutoSyncOutcome.Skipped && offline.BoardTitle == "Заявки");

            // 11. доска с API, но обновлять нечего и на мне ничего нет
            using (var nobody = new BoardServer())
            {
                nobody.Put(9001, "Чужая", mine: false, executors: "Иванов И.");
                var lonelyDir = Path.Combine(dataDir, "lonely");
                new TicketStore(lonelyDir).Save(new[] { new Ticket { Title = "Без номера" } });
                var lonelySettings = new AppSettings { IntraserviceBaseUrl = nobody.Url, IntraserviceLogin = "me" };
                App.DataDir = lonelyDir;   // её settings.json — отдельно
                var lonely = new MainViewModel(new TicketStore(lonelyDir), lonelySettings, new IntraserviceLinkParser(lonelySettings),
                    new HttpIntraserviceClient(nobody.Url, "me", "p"));
                await lonely.RefreshAllCommand.ExecuteAsync(null);
                Check("F5 на доске без номеров — обновлять нечего", Shown() == ("Обновление статусов", "на доске нет заявок с номером — обновлять нечего"), () => $"{Shown()}");
                await lonely.ImportMineCommand.ExecuteAsync(null);
                Check("импорт, когда на мне ничего нет", Shown() == ("Импорт моих заявок", "открытых заявок, где вы исполнитель, не нашлось"), () => $"{Shown()}");
                Check("…и без лишних запросов", nobody.Unexpected.Count == 0, () => string.Join("\n", nobody.Unexpected));
                App.DataDir = dataDir;
            }

            Check("доска менялась только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            Check("сервер не получал неожиданных запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
        }
        finally
        {
            server.Dispose();
            (App.DataDir, AskWindow.Answer, ClipboardWatcher.Last) = (dataDirWas, answerWas, clipboardWas);
            try { Directory.Delete(dataDir, recursive: true); } catch (IOException) { }
        }
        Debug.Assert(failures == 0, $"Карточка и панель: не прошло проверок — {failures} (список выше)");
    }
}
