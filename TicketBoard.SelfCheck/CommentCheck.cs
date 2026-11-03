using System.Text;
using System.Text.Json;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;
using static TicketBoard.SelfCheck.BoardServer;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка записи в Интрасервис (этап 8, шаг 1): комментарий из панели. Выключатель, поле только у карточки с
/// номером и API, предпросмотр, запрос целиком (метод, путь, заголовки, тело), сверка переписки перед отправкой, 409, отказы,
/// «неизвестно» без повтора, черновики у каждой заявки. Настоящий MainViewModel и клиент против поддельного сервера
/// (BoardServer принимает запись только здесь — AcceptWrites), в одном потоке (SingleThread).</summary>
internal static class CommentCheck
{
    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("комментарий");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        var server = new BoardServer();
        var (answerWas, timeoutWas) = (AskWindow.Answer, HttpIntraserviceClient.WriteTimeout);
        using (var data = new TempDataDir("tb-comment"))
        {
        try
        {
            server.AcceptWrites = true;   // запись принимаем и записываем — сколько их было и какие, видно в Writes
            server.Put(8101, "Принтер не печатает");
            server.Comment(8101, Requester, RequesterId, "Принтер в 305 не печатает");
            server.Put(8102, "Нет доступа к папке");
            server.Put(8103, "Почта не приходит");

            var boardDir = Path.Combine(data.Path, "board");
            new TicketStore(boardDir).Save(new[]
            {
                new Ticket { Title = "Принтер не печатает", IntraserviceId = 8101, Status = TicketStatus.InProgress },
                new Ticket { Title = "Нет доступа к папке", IntraserviceId = 8102, Status = TicketStatus.InProgress },
                new Ticket { Title = "Почта не приходит", IntraserviceId = 8103, Status = TicketStatus.Waiting },
                new Ticket { Title = "Своя задача", Status = TicketStatus.Inbox },
            });
            var settings = new AppSettings { IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me" };
            var client = new HttpIntraserviceClient(server.Url, "me", "p");
            var board = new MainViewModel(new TicketStore(boardDir), settings, new IntraserviceLinkParser(settings), client);
            var logged = new List<string>();
            board.Log = logged.Add;
            BoardCheck.WatchBoard(pump, board);
            var notified = new List<string?>();
            board.PropertyChanged += (_, e) => notified.Add(e.PropertyName);

            Ticket Card(string title) => board.AllTickets.First(t => t.Title == title);
            var (a, b, own) = (Card("Принтер не печатает"), Card("Нет доступа к папке"), Card("Своя задача"));
            // выбрать карточку и дождаться её переписки на экране
            async Task<bool> Open(Ticket t)
            {
                board.SelectedTicket = t;
                return await SingleThread.Until(() => board.CommentsMessage != "загружаю…");
            }
            Task Send() => board.SendCommentCommand.ExecuteAsync(null);
            // после отправки переписка перечитывается сама — дождаться, прежде чем писать следующий
            Task<bool> Settled() => SingleThread.Until(() => board.CommentsMessage != "загружаю…");
            List<(string Heading, string Text)> Shown() { lock (AskWindow.Shown) return AskWindow.Shown.ToList(); }
            void ClearShown() { lock (AskWindow.Shown) AskWindow.Shown.Clear(); }
            JsonElement Body(FakeRequest w) => JsonDocument.Parse(w.Body).RootElement;
            string Text(FakeRequest w) => Body(w).GetProperty("Comment").GetString() ?? "";
            string ChangedOf(int id) => $"{server.ChangedOf(id):yyyy-MM-ddTHH:mm:ss.fffffff}";
            int Lifetimes(int id) => server.Count($"/api/tasklifetime?taskid={id}");
            AskWindow.Answer = (_, _) => true;

            // 1. запись выключена (по умолчанию): поля нет, команда ничего не шлёт, даже вызванная
            Check("загрузилась переписка", await Open(a) && board.Comments.Any(c => c.Text == "Принтер в 305 не печатает"), () => board.CommentsMessage);
            Check("запись выключена — поля у панели нет", !board.CanComment);
            board.CommentText = "Проверьте картридж";
            ClearShown();
            await Send();
            Check("…и команда ничего не шлёт и не спрашивает", server.Writes.Count == 0 && Shown().Count == 0,
                () => $"записей: {server.Writes.Count}, окон: {Shown().Count}");

            // 2. включили — поле есть у карточки с номером и при API; без номера или без API — нет
            notified.Clear();
            settings.AllowIntraserviceWrite = true;
            board.ApplySettings(client);
            Check("включили — поле есть, панель узнала сразу", board.CanComment && notified.Contains(nameof(MainViewModel.CanComment)));
            board.SelectedTicket = own;
            Check("у карточки без номера поля нет", !board.CanComment);
            board.ApplySettings(null);
            await Open(a);
            Check("без API поля нет", !board.CanComment);
            board.ApplySettings(client);
            await Open(a);
            Check("с API — снова есть", board.CanComment);

            // 3. внутренний по умолчанию: предпросмотр, один PUT целиком, после — поле пустое и комментарий в переписке
            Check("по умолчанию — внутренний", board.CommentInternal);
            const string first = "Проверьте картридж, пожалуйста.\r\nЕсли не поможет — заменим.";
            board.CommentText = "  " + first + "  \r\n";
            var changed = ChangedOf(8101);
            ClearShown();
            await Send();
            var shown = Shown();
            Check("предпросмотр: номер, название, «внутренний», текст целиком", shown is [var (heading, text)]
                && heading.Contains("#8101") && text.Contains("Принтер не печатает") && text.Contains("Внутренний — увидят только сотрудники")
                && text.Contains(first),
                () => string.Join("\n---\n", shown.Select(s => $"{s.Heading}\n{s.Text}")));
            var writes = server.Writes;
            Check("ровно один PUT на api/task/8101", writes is [{ Method: "PUT", Target: "/api/task/8101" }],
                () => string.Join("\n", writes.Select(w => $"{w.Method} {w.Target}")));
            if (writes.Count > 0)
            {
                var w = writes[0];
                var basic = "Basic " + Convert.ToBase64String(Encoding.UTF8.GetBytes("me:p"));
                Check("заголовки: Basic, JSON, Device-Name, Device-Version", w.Headers.GetValueOrDefault("Authorization") == basic
                    && (w.Headers.GetValueOrDefault("Content-Type") ?? "").StartsWith("application/json")
                    && w.Headers.GetValueOrDefault("Device-Name") == "TicketBoard"
                    && Version.TryParse(w.Headers.GetValueOrDefault("Device-Version"), out _),
                    () => string.Join("\n", w.Headers.Select(h => $"{h.Key}: {h.Value}")));
                var body = Body(w);
                Check("тело — ровно Comment (без пробелов по краям), IsPrivateComment: true и Changed строкой из карточки",
                    body.EnumerateObject().Select(p => p.Name).Order().SequenceEqual(new[] { "Changed", "Comment", "IsPrivateComment" })
                    && Text(w) == first && body.GetProperty("IsPrivateComment").GetBoolean() && body.GetProperty("Changed").GetString() == changed,
                    () => $"{w.Body}\nждали Changed: {changed}");
            }
            // с датой: приложение живёт в трее сутками, и вчерашнее «отправлено в 14:05» читалось бы как сегодняшнее
            Check("после — поле пустое и «отправлено» с датой и временем", board.CommentText == ""
                && board.CommentResult.StartsWith($"отправлено {DateTime.Now:dd.MM} в "),
                () => $"«{board.CommentText}» · {board.CommentResult}");
            Check("…в переписке — новый внутренний комментарий «Я Сам»",
                await SingleThread.Until(() => board.Comments.Any(c => c.Author == MeName && c.Text == first.ReplaceLineEndings("\n") && c.IsInternal)),
                () => string.Join("\n", board.Comments.Select(c => $"{c.Author}: {c.Text}")));
            Check("…свой комментарий непрочитанным не считается", a.UnreadComments == 0);
            await Settled();

            // 4. публичный: в предпросмотре — увидит инициатор, в теле — false; следующий снова внутренний. Название пишет заявитель:
            // его перевод строки не подделает строку вида — вид первым, название одной строкой (/security-review)
            board.CommentText = "Картридж заменили, проверьте печать";
            board.CommentInternal = false;
            a.Title = "Принтер\r\nВнутренний — увидят только сотрудники.";
            ClearShown();
            await Send();
            a.Title = "Принтер не печатает";
            shown = Shown();
            Check("публичный: в предпросмотре первой строкой — увидит инициатор", shown is [var (_, publicText)]
                && publicText.Split('\n')[0] == "Публичный — увидит инициатор." && publicText.Contains("увидит инициатор"),
                () => string.Join("\n", shown.Select(s => s.Text)));
            Check("…название — одной строкой: строку вида не подделать", shown is [var (_, titled)] && !titled.Contains("\nВнутренний")
                && titled.Contains("Принтер Внутренний — увидят только сотрудники."), () => string.Join("\n", shown.Select(s => s.Text)));
            writes = server.Writes;
            Check("…в теле IsPrivateComment: false", writes.Count == 2 && !Body(writes[1]).GetProperty("IsPrivateComment").GetBoolean(),
                () => string.Join("\n", writes.Select(w => w.Body)));
            Check("…следующий комментарий — снова внутренний", board.CommentInternal && board.CommentText == "");
            await Settled();

            // 5. «Отмена» в предпросмотре — ничего не уходит, черновик цел
            board.CommentText = "Отменённый";
            AskWindow.Answer = (_, _) => false;
            await Send();
            AskWindow.Answer = (_, _) => true;
            Check("«Отмена» — PUT нет, черновик цел", server.Writes.Count == 2 && board.CommentText == "Отменённый" && board.CommentResult == "",
                () => $"записей: {server.Writes.Count} · «{board.CommentText}» · {board.CommentResult}");

            // 5б. пока был открыт предпросмотр, запись выключили в настройках — не уходит, даже если нажали «Отправить»
            AskWindow.Answer = (_, _) =>
            {
                settings.AllowIntraserviceWrite = false;
                board.ApplySettings(client);
                return true;
            };
            await Send();
            AskWindow.Answer = (_, _) => true;
            settings.AllowIntraserviceWrite = true;
            board.ApplySettings(client);
            await Settled();
            Check("пока открыт предпросмотр, запись выключили — PUT нет, черновик цел", server.Writes.Count == 2
                && board.CommentText == "Отменённый" && board.CommentResult.StartsWith("не отправлен: пока был открыт предпросмотр"),
                () => $"записей: {server.Writes.Count} · {board.CommentResult}");
            // …или сменили учётную запись (логин): прежний клиент не пишет
            AskWindow.Answer = (_, _) =>
            {
                settings.IntraserviceLogin = "other";
                board.ApplySettings(new HttpIntraserviceClient(server.Url, "other", "p"));
                return true;
            };
            await Send();
            AskWindow.Answer = (_, _) => true;
            settings.IntraserviceLogin = "me";
            board.ApplySettings(client);
            await Settled();
            Check("…сменили учётную запись — тоже", server.Writes.Count == 2 && board.CommentResult.StartsWith("не отправлен: пока был открыт предпросмотр"),
                () => $"записей: {server.Writes.Count} · {board.CommentResult}");
            // а просто сохранили настройки (новый клиент той же учётной записи) — уходит
            board.CommentText = "Сохранили настройки";
            AskWindow.Answer = (_, _) =>
            {
                board.ApplySettings(new HttpIntraserviceClient(server.Url, "me", "p"));
                return true;
            };
            await Send();
            AskWindow.Answer = (_, _) => true;
            board.ApplySettings(client);
            await Settled();
            Check("…просто сохранили настройки — уходит", server.Writes.Count == 3 && Text(server.Writes[2]) == "Сохранили настройки"
                && board.CommentResult.StartsWith("отправлено"), () => $"записей: {server.Writes.Count} · {board.CommentResult}");
            board.CommentText = "Отменённый";

            // 6. пустой — ни запросов, ни предпросмотра
            board.CommentText = " \r\n ";
            var (cards, lifetimes) = (server.Count("/api/task/8101"), Lifetimes(8101));
            ClearShown();
            await Send();
            Check("пустой — ни запросов, ни предпросмотра", server.Count("/api/task/8101") == cards && Lifetimes(8101) == lifetimes
                && Shown().Count == 0 && server.Writes.Count == 3 && board.CommentResult == "напишите комментарий", () => board.CommentResult);

            // 7. пока писали, ответил заявитель — комментарий не уходит, новая запись на экране; снова — уходит
            int count;
            board.CommentText = "Ответ на старое";
            server.Comment(8101, Requester, RequesterId, "Уже заработало, спасибо");
            ClearShown();
            await Send();
            Check("пока писали, ответил заявитель — предпросмотра и PUT нет", Shown().Count == 0 && server.Writes.Count == 3,
                () => $"окон: {Shown().Count}, записей: {server.Writes.Count}");
            Check("…новая запись на экране", board.Comments.Any(c => c.Text == "Уже заработало, спасибо"));
            Check("…сказано и черновик цел", board.CommentResult.Contains("пока вы писали") && board.CommentText == "Ответ на старое",
                () => board.CommentResult);
            await Send();
            Check("…снова «Отправить…» — отправлено", server.Writes.Count == 4 && Text(server.Writes[3]) == "Ответ на старое"
                && board.CommentResult.StartsWith("отправлено"), () => board.CommentResult);
            await Settled();

            // 7в. пока писали, сменили только статус (запись без комментария спрятана, пока глаз не нажат) — так и сказано
            board.CommentText = "После смены статуса";
            server.StatusChange(8101, Working);
            await Send();
            Check("сменили только статус — не уходит, сказано, что его показывает кнопка-глаз", server.Writes.Count == 4 && !board.ShowAllEvents
                && board.CommentResult.Contains("пока вы писали") && board.CommentResult.Contains("кнопка-глаз"), () => board.CommentResult);
            await Send();
            Check("…снова — отправлено", server.Writes.Count == 5 && board.CommentResult.StartsWith("отправлено"), () => board.CommentResult);
            await Settled();

            // 7б. переписки ещё нет на экране (выбрали карточку и сразу «Отправить…», пока она грузится) — не уходит, теперь она есть
            var c = Card("Почта не приходит");
            board.SelectedTicket = c;
            board.CommentText = "Сразу после выбора";
            ClearShown();
            count = server.Writes.Count;
            await Send();
            Check("переписки ещё не было на экране — предпросмотра и PUT нет, так и сказано", Shown().Count == 0 && server.Writes.Count == count
                && board.CommentResult.StartsWith("не отправлен: переписки ещё не было на экране") && board.CommentText == "Сразу после выбора"
                && board.CommentsMessage == "переписки нет", () => $"{board.CommentResult} · {board.CommentsMessage}");
            await Send();
            Check("…снова — отправлено", server.Writes.Count == count + 1 && board.CommentResult.StartsWith("отправлено"), () => board.CommentResult);
            await Settled();
            await Open(a);

            // 8. изменили, пока открыт предпросмотр, — сервер отвечает 409, ничего не пишет; переписка перечитана
            board.CommentText = "Во время предпросмотра";
            changed = ChangedOf(8101);
            AskWindow.Answer = (_, _) => { server.Comment(8101, Requester, RequesterId, "А теперь опять не печатает"); return true; };
            lifetimes = Lifetimes(8101);
            count = server.Writes.Count;
            await Send();
            AskWindow.Answer = (_, _) => true;
            writes = server.Writes;
            Check("изменили, пока открыт предпросмотр, — PUT с прежним Changed", writes.Count == count + 1
                && Body(writes[^1]).GetProperty("Changed").GetString() == changed, () => string.Join("\n", writes.Select(w => w.Body)));
            Check("…409: не отправлен, заявку изменили, ответ сервера", board.CommentResult.StartsWith("не отправлен: заявку изменили")
                && board.CommentResult.Contains("Заявка изменена пользователем") && board.CommentText == "Во время предпросмотра", () => board.CommentResult);
            Check("…переписка перечитана — новая запись на экране",
                await SingleThread.Until(() => board.Comments.Any(c => c.Text == "А теперь опять не печатает")) && Lifetimes(8101) == lifetimes + 2,
                () => $"чтений переписки: {Lifetimes(8101) - lifetimes}");
            Check("…и комментарий не записан", server.Writes.Count == count + 1 && board.Comments.All(c => c.Text != "Во время предпросмотра"));

            // 9. отказ (403) — не отправлен, с ответом сервера; черновик цел
            server.WriteAnswer = _ => (403, "{\"Message\":\"Нет прав на изменение заявки\"}");
            await Send();
            Check("403 — не отправлен: нет доступа, с ответом сервера", board.CommentResult.StartsWith("не отправлен: нет доступа (HTTP 403)")
                && board.CommentResult.Contains("Нет прав на изменение заявки") && board.CommentText == "Во время предпросмотра", () => board.CommentResult);

            // 10. ошибка сервера (500) — неизвестно, дошёл ли; один PUT; в журнале — номер заявки, но не текст
            server.WriteAnswer = _ => (500, "{\"Message\":\"An error has occurred.\"}");
            logged.Clear();
            count = server.Writes.Count;
            await Send();
            Check("500 — неизвестно, дошёл ли; черновик цел", board.CommentResult.StartsWith("неизвестно, дошёл ли")
                && board.CommentText == "Во время предпросмотра" && server.Writes.Count == count + 1, () => board.CommentResult);
            Check("…в журнале — номер заявки, а текста комментария нет", logged.Any(l => l.Contains("#8101"))
                && !logged.Any(l => l.Contains("Во время предпросмотра")), () => string.Join("\n", logged));
            server.WriteAnswer = null;

            // 11. сервер молчит дольше срока записи — неизвестно; запись не повторяется (дошла позже — она на экране)
            (server.WriteWait, HttpIntraserviceClient.WriteTimeout) = (TimeSpan.FromMilliseconds(1500), TimeSpan.FromMilliseconds(300));
            board.CommentText = "Медленный сервер";
            count = server.Writes.Count;
            await Send();
            Check("сервер молчит — неизвестно, дошёл ли", board.CommentResult.StartsWith("неизвестно, дошёл ли") && board.CommentResult.Contains("не ответил")
                && board.CommentText == "Медленный сервер", () => board.CommentResult);
            Check("…переписка перечитана: дошёл позже — он на экране",
                await SingleThread.Until(() => board.Comments.Any(c => c.Text == "Медленный сервер")), () => string.Join("\n", board.Comments.Select(c => c.Text)));
            Check("…и запись не повторилась", server.Writes.Count == count + 1, () => $"записей: {server.Writes.Count - count}");
            (server.WriteWait, HttpIntraserviceClient.WriteTimeout) = (TimeSpan.Zero, timeoutWas);
            board.CommentText = "";

            // 12. карточка перед отправкой не прочиталась (500) — не отправлен, PUT нет
            board.CommentText = "Карточка не читается";
            server.Quietly(8101, t => t.CardFailures = 1);
            count = server.Writes.Count;
            ClearShown();
            await Send();
            Check("карточка не прочиталась — не отправлен, ни предпросмотра, ни PUT", board.CommentResult.StartsWith("не отправлен: ошибка сервера (HTTP 500)")
                && Shown().Count == 0 && server.Writes.Count == count && board.CommentText == "Карточка не читается", () => board.CommentResult);
            server.Quietly(8101, t => t.LifetimeFails = 503);
            await Send();
            server.Quietly(8101, t => t.LifetimeFails = 0);
            Check("…переписка не прочиталась — тоже", board.CommentResult.StartsWith("не отправлен: ошибка сервера (HTTP 503)")
                && Shown().Count == 0 && server.Writes.Count == count, () => board.CommentResult);

            // 12б. пока сверялась переписка, выбрали другую карточку — не уходит; на экране — переписка выбранной, а не этой
            server.CardGate.Reset();
            var cardReads = server.Count("/api/task/8101");
            var checking = Send();
            await SingleThread.Until(() => server.Count("/api/task/8101") > cardReads);
            board.SelectedTicket = b;
            server.CardGate.Set();
            await checking;
            Check("выбрали другую, пока сверялась, — ни предпросмотра, ни PUT", Shown().Count == 0 && server.Writes.Count == count);
            await Settled();
            Check("…на экране переписка выбранной (её нет), а не той", board.CommentsMessage == "переписки нет" && board.Comments.Count == 0,
                () => $"{board.CommentsMessage} · {string.Join(" | ", board.Comments.Select(c => c.Text))}");
            await Open(a);
            Check("…у той — так и сказано", board.CommentResult.StartsWith("не отправлен: выбрали другую заявку"), () => board.CommentResult);

            // 13. черновики — у каждой заявки свои: текст одной не уйдёт в другую
            board.CommentText = "Текст А";
            notified.Clear();
            await Open(b);
            Check("выбрали другую — панель перечитывает поле, вид, итог и само поле",
                new[] { nameof(MainViewModel.CommentText), nameof(MainViewModel.CommentInternal), nameof(MainViewModel.CommentResult), nameof(MainViewModel.CanComment) }
                    .All(notified.Contains), () => string.Join(", ", notified.Distinct()));
            Check("выбрали Б — поле пустое, внутренний, строка итога пустая", board.CommentText == "" && board.CommentInternal && board.CommentResult == "",
                () => $"«{board.CommentText}» · {board.CommentResult}");
            board.CommentText = "Текст Б";
            board.CommentInternal = false;
            await Open(a);
            Check("вернулись в А — текст и итог А", board.CommentText == "Текст А" && board.CommentInternal
                && board.CommentResult.StartsWith("не отправлен: выбрали другую заявку"), () => $"«{board.CommentText}» · {board.CommentResult}");
            count = server.Writes.Count;
            await Send();
            writes = server.Writes;
            Check("отправка из А — в А с текстом А", writes.Count == count + 1 && writes[^1].Target == "/api/task/8101" && Text(writes[^1]) == "Текст А",
                () => string.Join("\n", writes.Skip(count).Select(w => $"{w.Target} {w.Body}")));
            await Open(b);
            Check("черновик Б цел", board.CommentText == "Текст Б" && !board.CommentInternal, () => $"«{board.CommentText}»");

            // 13б. дописали, пока шла запись, — это уже следующий комментарий: после «отправлено» он в поле
            await Open(a);
            board.CommentText = "Первый";
            server.WriteWait = TimeSpan.FromMilliseconds(400);
            count = server.Writes.Count;
            var writing = Send();
            await SingleThread.Until(() => server.Writes.Count == count + 1);
            board.CommentText = "Первый и второй";
            await writing;
            server.WriteWait = TimeSpan.Zero;
            Check("дописали, пока шла запись, — ушёл первый, второй остался в поле", server.Writes[^1].Target == "/api/task/8101"
                && Text(server.Writes[^1]) == "Первый"
                && board.CommentText == "Первый и второй" && board.CommentResult.StartsWith("отправлено"), () => $"«{board.CommentText}» · {board.CommentResult}");
            await Settled();
            board.CommentText = "";

            // 14. выбрали другую, пока отправлялась А, — строка Б не тронута; итог А — когда А выбрана снова
            await Open(a);
            board.CommentText = "Пока выбирали другую";
            server.WriteWait = TimeSpan.FromMilliseconds(600);
            count = server.Writes.Count;
            var sending = Send();
            await SingleThread.Until(() => server.Writes.Count == count + 1);
            board.SelectedTicket = b;
            await sending;
            server.WriteWait = TimeSpan.Zero;
            Check("пока шла отправка А, выбрали Б — строка и черновик Б не тронуты", board.CommentResult == "" && board.CommentText == "Текст Б",
                () => $"«{board.CommentText}» · {board.CommentResult}");
            await Open(a);
            Check("…итог А виден, когда А выбрана снова", board.CommentResult.StartsWith("отправлено") && board.CommentText == "",
                () => $"«{board.CommentText}» · {board.CommentResult}");
            Check("…и переписка у неё свежая — с этим комментарием, а не из кэша", board.Comments.Any(c => c.Text == "Пока выбирали другую"),
                () => string.Join(" | ", board.Comments.Select(c => c.Text)));

            // 15. клиент: сервер не отвечает вовсе (порт закрыт) — запрос не ушёл; 204 без тела — отправлено
            var gone = new BoardServer();
            var gonePort = gone.Port;
            gone.Dispose();
            var refused = await new HttpIntraserviceClient($"http://127.0.0.1:{gonePort}", "me", "p").CommentAsync(8101, "x", true, null);
            Check("порт закрыт — не отправлен: сервер недоступен", refused is { Outcome: WriteOutcome.Refused, Conflict: false }
                && refused.Error.StartsWith("сервер недоступен"), () => $"{refused.Outcome}: {refused.Error}");
            server.WriteAnswer = _ => (204, "");
            var noBody = await client.CommentAsync(8102, "без тела", true, null);
            Check("204 без тела — отправлено", noBody is { Outcome: WriteOutcome.Done, Error: "" }, () => $"{noBody.Outcome}: {noBody.Error}");
            Check("…без Changed — тело без него", server.Writes is [.., var last] && Text(last) == "без тела"
                && !Body(last).TryGetProperty("Changed", out _), () => string.Join("\n", server.Writes.Select(w => w.Body)));
            server.WriteAnswer = _ => (400, "{\"Message\":\"Comment is required\"}");
            var bad = await client.CommentAsync(8102, "x", true, null);
            Check("400 — отказ с ответом сервера", bad is { Outcome: WriteOutcome.Refused, Conflict: false } && bad.Error.Contains("HTTP 400")
                && bad.Error.Contains("Comment is required"), () => $"{bad.Outcome}: {bad.Error}");
            server.WriteAnswer = _ => (409, "{\"Message\":\"changed\"}");
            var conflict = await client.CommentAsync(8102, "x", true, "2026-01-01T00:00:00");
            Check("409 — отказ, Conflict", conflict is { Outcome: WriteOutcome.Refused, Conflict: true } && conflict.Error.Contains("HTTP 409"),
                () => $"{conflict.Outcome}: {conflict.Error}");
            // переадресация (страница входа) и html-страница с 200 — не «записано»: неизвестно, записано ли
            server.WriteAnswer = w => w.Target == "/api/task/8102" ? (302, "/Account/Login") : (200, "<html><body>Вход в систему</body></html>");
            var redirected = await client.CommentAsync(8102, "x", true, null);
            Check("переадресовали (на страницу входа) — не отправлено, а неизвестно", redirected is { Outcome: WriteOutcome.Unknown }
                && redirected.Error.Contains("переадресовал") && redirected.Error.Contains("/Account/Login"), () => $"{redirected.Outcome}: {redirected.Error}");
            server.WriteAnswer = _ => (200, "<html><body>Вход в систему</body></html>");
            var page = await client.CommentAsync(8102, "x", true, null);
            Check("html-страница с 200 — тоже неизвестно, с её текстом", page is { Outcome: WriteOutcome.Unknown } && page.Error.Contains("страницу")
                && page.Error.Contains("Вход в систему"), () => $"{page.Outcome}: {page.Error}");
            server.WriteAnswer = null;

            Check("доска менялась только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            Check("сервер не получал неожиданных запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
        }
        finally
        {
            server.Dispose();
            (AskWindow.Answer, HttpIntraserviceClient.WriteTimeout) = (answerWas, timeoutWas);
        }
        }
        checks.AssertAll();
    }
}
