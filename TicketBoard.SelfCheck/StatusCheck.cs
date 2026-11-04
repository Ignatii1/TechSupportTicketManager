using System.Text;
using System.Text.Json;
using GongSolutions.Wpf.DragDrop;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using TicketBoard.Views;
using static TicketBoard.SelfCheck.BoardServer;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка смены статуса в Интрасервисе при переносе карточки (этап 8, шаг 2): выключатель, статусы колонок,
/// четыре пути переноса руками и переносы, которые не пишут; окно (что в нём и что выбрано), запрос целиком, итоги, окна по
/// одному. Настоящий MainViewModel и клиент против BoardServer (запись принимается только здесь), окно — заглушка
/// StatusWindow: ответ задаёт проверка, а задача ответа держит окно «открытым».</summary>
internal static class StatusCheck
{
    public static void Run() => SingleThread.Run(RunAsync);

    /// <summary>Перетаскивание карточки на колонку: что передаёт gong.</summary>
    private sealed record Drag(object? Data) : IDropInfo;

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("статус");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        var server = new BoardServer();
        var (answerWas, askWas, timeoutWas) = (StatusWindow.Answer, AskWindow.Answer, HttpIntraserviceClient.WriteTimeout);
        using (var data = new TempDataDir("tb-status"))
        {
        try
        {
            server.AcceptWrites = true;
            server.Put(9101, "Принтер не печатает", Working);
            server.Change(9101, t => t.ToStatuses = new[] { Fixed, Waits, Final });
            server.Comment(9101, Requester, RequesterId, "Принтер в 305 не печатает");
            server.Comment(9101, MeName, MeId, "Проверю после обеда");
            server.Put(9102, "Нет доступа к папке", Open);                                     // прав в ответе нет
            server.Put(9103, "Уже выполнена", Fixed);
            server.Put(9104, "Закрыть нельзя", Working);
            server.Change(9104, t => t.ToStatuses = new[] { Waits });
            server.Put(9105, "Почта не приходит", Working);
            server.Put(9106, "Сканер", Working);
            server.Put(9107, "Ноутбук", Working);

            var boardDir = Path.Combine(data.Path, "board");
            new TicketStore(boardDir).Save(new[]
            {
                new Ticket { Title = "Принтер (моё название)", IntraserviceId = 9101, Status = TicketStatus.InProgress },
                new Ticket { Title = "Нет доступа к папке", IntraserviceId = 9102, Status = TicketStatus.InProgress },
                new Ticket { Title = "Уже выполнена", IntraserviceId = 9103, Status = TicketStatus.InProgress },
                new Ticket { Title = "Закрыть нельзя", IntraserviceId = 9104, Status = TicketStatus.InProgress },
                new Ticket { Title = "Почта не приходит", IntraserviceId = 9105, Status = TicketStatus.InProgress },
                new Ticket { Title = "Сканер", IntraserviceId = 9106, Status = TicketStatus.InProgress },
                new Ticket { Title = "Ноутбук", IntraserviceId = 9107, Status = TicketStatus.InProgress },
                new Ticket { Title = "Своя задача", Status = TicketStatus.InProgress },
            });
            var settings = new AppSettings
            {
                IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me",
                StatusOnMove = new() { [TicketStatus.Done] = new(Fixed, "Выполнена"), [TicketStatus.Waiting] = new(Waits, "Ожидание ответа пользователя") },
            };
            var client = new HttpIntraserviceClient(server.Url, "me", "p");
            var board = new MainViewModel(new TicketStore(boardDir), settings, new IntraserviceLinkParser(settings), client);
            var logged = new List<string>();
            board.Log = logged.Add;
            BoardCheck.WatchBoard(pump, board);

            Ticket Card(int id) => board.AllTickets.First(t => t.IntraserviceId == id);
            List<StatusForm> Forms() { lock (StatusWindow.Shown) return StatusWindow.Shown.ToList(); }
            void ClearForms() { lock (StatusWindow.Shown) StatusWindow.Shown.Clear(); }
            List<(string Heading, string Text)> Told() { lock (AskWindow.Shown) return AskWindow.Shown.ToList(); }
            void ClearTold() { lock (AskWindow.Shown) AskWindow.Shown.Clear(); }
            JsonElement Body(FakeRequest w) => JsonDocument.Parse(w.Body).RootElement;
            string ChangedOf(int id) => $"{server.ChangedOf(id):yyyy-MM-ddTHH:mm:ss.fffffff}";
            // ответ окна: выбрать статус (null — какой предложен), написать комментарий и нажать «Сменить статус»
            Func<StatusForm, Task<bool>> Apply(string comment, int? status = null, bool isInternal = false) => form =>
            {
                if (status is int id) form.Target = form.Choices.FirstOrDefault(s => s.Id == id);
                form.Comment = comment;
                form.Internal = isInternal;
                return Task.FromResult(true);
            };
            Task Settled() => board.OffersSettled();
            AskWindow.Answer = (_, _) => true;

            // 1. выключатель выключен (по умолчанию): переносы руками ничего не пишут и окна не показывают
            var a = Card(9101);
            board.SelectedTicket = a;
            ClearForms();
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("выключено — перенесли в «Готово»: окна нет, записи нет", Forms().Count == 0 && server.Writes.Count == 0 && a.Status == TicketStatus.Done,
                () => $"окон: {Forms().Count}, записей: {server.Writes.Count}, колонка: {a.Status}");
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();

            // 2. включили — колонке без статуса окна нет
            settings.AllowStatusWrite = true;
            board.ApplySettings(client);
            board.MoveToCommand.Execute(TicketStatus.Inbox);
            await Settled();
            Check("колонке статус не назначен («Входящие») — окна нет", Forms().Count == 0 && server.Writes.Count == 0);
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();

            // 3. «Готово» → «Выполнена» меню карточки: окно со всем, что нужно, ответ — один PUT целиком, статус на карточке новый
            StatusWindow.Answer = Apply("Заменили картридж");
            var changed = ChangedOf(9101);
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            var forms = Forms();
            Check("перенесли в «Готово» (меню) — окно", forms.Count == 1 && a.Status == TicketStatus.Done, () => $"окон: {forms.Count}, колонка: {a.Status}");
            if (forms is [var f])
            {
                Check("в окне: номер, название с сервера, нынешний статус", f.Ticket == 9101 && f.Title == "Принтер не печатает" && f.Current == "В работе",
                    () => $"{f.Ticket} · {f.Title} · {f.Current}");
                Check("…последние записи переписки, свежие первыми", f.Recent.Select(r => r.Text).SequenceEqual(new[] { "Проверю после обеда", "Принтер в 305 не печатает" }),
                    () => string.Join(" | ", f.Recent.Select(r => r.Text)));
                Check("…переходы — из прав, без нынешнего; выбран статус колонки", f.Choices.Select(s => s.Id).SequenceEqual(new[] { Fixed, Final, Waits })
                    && f.Target?.Id == Fixed && f.Note == "", () => $"{string.Join(", ", f.Choices.Select(s => s.Id))} · выбран {f.Target?.Id} · {f.Note}");
            }
            var writes = server.Writes;
            Check("один PUT на api/task/9101", writes is [{ Method: "PUT", Target: "/api/task/9101" }], () => string.Join("\n", writes.Select(w => w.Target)));
            if (writes is [var w1])
            {
                var body = Body(w1);
                Check("тело — ровно StatusId, Comment, IsPrivateComment: false и Changed строкой из карточки",
                    body.EnumerateObject().Select(p => p.Name).Order().SequenceEqual(new[] { "Changed", "Comment", "IsPrivateComment", "StatusId" })
                    && body.GetProperty("StatusId").GetInt32() == Fixed && body.GetProperty("Comment").GetString() == "Заменили картридж"
                    && !body.GetProperty("IsPrivateComment").GetBoolean() && body.GetProperty("Changed").GetString() == changed,
                    () => $"{w1.Body}\nждали Changed: {changed}");
                Check("…заголовки как у комментария", w1.Headers.GetValueOrDefault("Device-Name") == "TicketBoard"
                    && w1.Headers.GetValueOrDefault("Authorization") == "Basic " + Convert.ToBase64String(Encoding.UTF8.GetBytes("me:p")));
            }
            Check("статус Интрасервиса на карточке — новый, в строке панели сказано", a.ExternalStatus == "Выполнена"
                && board.SyncMessage == "статус в Интрасервисе: «Выполнена»", () => $"{a.ExternalStatus} · {board.SyncMessage}");

            // …и другими путями: ←/→, выбор колонки в панели, перетаскивание
            server.Quietly(9101, t => t.StatusId = Working);
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            ClearForms();
            board.MoveSelected(+2);   // на две колонки вправо: из «В работе» — в «Готово»
            await Settled();
            Check("←/→ — окно для колонки, куда пришла («Готово» через две)", a.Status == TicketStatus.Done && Forms() is [{ Target.Id: Fixed }]
                && server.Writes.Count == 2, () => $"{a.Status} · окон: {Forms().Count} · записей: {server.Writes.Count}");
            server.Quietly(9101, t => t.StatusId = Working);
            board.SelectedStatus = TicketStatus.InProgress;
            await Settled();
            ClearForms();
            board.SelectedStatus = TicketStatus.Done;
            await Settled();
            Check("выбор колонки в панели — окно", Forms() is [{ Target.Id: Fixed }] && server.Writes.Count == 3, () => $"окон: {Forms().Count}, записей: {server.Writes.Count}");
            server.Quietly(9101, t => t.StatusId = Working);
            board.SelectedStatus = TicketStatus.InProgress;
            await Settled();
            ClearForms();
            ((IDropTarget)board.ColumnFor(TicketStatus.Done)).Drop(new Drag(a));
            await Settled();
            Check("перетаскивание — окно", a.Status == TicketStatus.Done && Forms() is [{ Target.Id: Fixed }] && server.Writes.Count == 4,
                () => $"{a.Status} · окон: {Forms().Count}, записей: {server.Writes.Count}");
            ClearForms();
            ((IDropTarget)board.ColumnFor(TicketStatus.Done)).Drop(new Drag(a));   // в ту же колонку — перестановка
            await Settled();
            Check("перестановка внутри колонки — окна нет", Forms().Count == 0 && server.Writes.Count == 4);

            // 4. статус закрыт по названию, а колонка не «Готово» (Ждёт ответа → «Отменена»): заход автообновления не шлёт
            // «закрыта в Интрасервисе» о своём действии — карточка перечитана сразу после записи
            settings.StatusOnMove![TicketStatus.Waiting] = new(Cancelled, "Отменена");
            var e = Card(9105);
            board.SelectedTicket = e;
            StatusWindow.Answer = Apply("");
            board.MoveToCommand.Execute(TicketStatus.Waiting);
            await Settled();
            Check("«Отменена» из «Ждёт ответа» — ушла, на карточке «Отменена»", e.ExternalStatus == "Отменена" && server.Writes.Count == 5, () => $"{e.ExternalStatus}");
            var notes = new List<string>();
            board.Notify += (title, text, _) => notes.Add($"{title}: {text}");
            settings.AutoSyncMinutes = 5;
            board.ApplySettings(client);
            Check("заход автообновления прошёл", await board.AutoSyncAsync() is AutoSyncOutcome.Done or AutoSyncOutcome.Skipped, () => board.AutoSyncState);
            Check("…и не прислал «закрыта в Интрасервисе» о своём же действии", !notes.Any(n => n.Contains("закрыт") && n.Contains("Почта не приходит")),
                () => string.Join("\n", notes));
            settings.StatusOnMove![TicketStatus.Waiting] = new(Waits, "Ожидание ответа пользователя");

            // 5. обязательный комментарий: без него форма не даёт сменить, а пустой не уходит, даже если окно ответило «да»
            var b = Card(9102);
            board.SelectedTicket = b;
            ClearForms();
            StatusWindow.Answer = form => Task.FromResult(true);   // нажали, ничего не написав
            board.MoveToCommand.Execute(TicketStatus.Waiting);
            await Settled();
            forms = Forms();
            Check("в «Ожидание ответа пользователя» комментарий обязателен — форма не даёт сменить без него", forms is [{ CommentRequired: true, CanApply: false } fb]
                && fb.Target?.Id == Waits, () => string.Join(" | ", forms.Select(x => $"{x.Target?.Id} нужен:{x.CommentRequired} можно:{x.CanApply}")));
            Check("…и пустой не уходит", server.Writes.Count == 5, () => $"записей: {server.Writes.Count}");
            if (forms is [var req])
            {
                req.Comment = "Пришлите, пожалуйста, скриншот ошибки";
                Check("…с комментарием — можно", req.CanApply);
            }

            // 6. «Только на доске» — PUT нет, карточка в новой колонке
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            ClearForms();
            StatusWindow.Answer = _ => Task.FromResult(false);
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("«Только на доске» — PUT нет, карточка в «Готово»", Forms().Count == 1 && server.Writes.Count == 5 && b.Status == TicketStatus.Done);

            // 9. прав в ответе нет — весь справочник без нынешнего (решит сервер)
            Check("прав в ответе нет — весь справочник без нынешнего", Forms() is [var fNo]
                && fNo.Choices.Select(s => s.Id).Order().SequenceEqual(new[] { Fixed, Final, Working, Cancelled, Waits }.Order())
                && fNo.Target?.Id == Fixed, () => string.Join(", ", Forms().SelectMany(x => x.Choices).Select(s => s.Id)));
            //    …и сервер не принял include (400) — карточка без прав, то же
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            ClearForms();
            server.RefuseRights = true;
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            server.RefuseRights = false;
            Check("сервер не принял include прав (400) — окно по карточке без прав", Forms() is [var fRefused]
                && fRefused.Choices.Count == 5 && fRefused.Current == "Открыта", () => $"окон: {Forms().Count} · {string.Join(" | ", Told().Select(t => t.Text))}");

            // 7. статус у заявки уже тот — окна нет
            var done = Card(9103);
            board.SelectedTicket = done;
            ClearForms();
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("в Интрасервисе уже «Выполнена» — окна нет", Forms().Count == 0 && done.Status == TicketStatus.Done);

            // 8. статус колонки правами не разрешён — не выбран, сказано; выбрали другой — уходит он
            var d = Card(9104);
            board.SelectedTicket = d;
            ClearForms();
            StatusWindow.Answer = Apply("Уточните, пожалуйста, модель", status: Waits);
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("«Выполнена» правами не разрешена — не выбрана, сказано", Forms() is [{ Note: var note } fNot]
                && note.Contains("«Выполнена»") && note.Contains("недоступ") && fNot.Choices.Select(s => s.Id).SequenceEqual(new[] { Waits }),
                () => string.Join(" | ", Forms().Select(x => x.Note)));
            Check("…выбрали другой — уходит он", server.Writes is [.., var wOther] && Body(wOther).GetProperty("StatusId").GetInt32() == Waits
                && d.ExternalStatus == "Ожидание ответа пользователя", () => $"{server.Writes[^1].Body} · {d.ExternalStatus}");

            // 15. номер из настроек есть на сервере под другим названием — не выбран, сказано
            settings.StatusOnMove![TicketStatus.Done] = new(Fixed, "Решена");
            var c = Card(9106);
            board.SelectedTicket = c;
            ClearForms();
            StatusWindow.Answer = _ => Task.FromResult(false);
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("в настройках №29 «Решена», а на сервере №29 — «Выполнена»: не выбран, сказано", Forms() is [{ Target: null, Note: var stale }]
                && stale.Contains("«Решена»"), () => string.Join(" | ", Forms().Select(x => $"{x.Target?.Id} {x.Note}")));
            settings.StatusOnMove![TicketStatus.Done] = new(Fixed, "Выполнена");
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();

            // 10. перенесли обратно, пока читалась заявка, — окна нет
            ClearForms();
            server.CardGate.Reset();
            var cardReads = server.Count("/api/task/9106");
            board.MoveToCommand.Execute(TicketStatus.Done);
            await SingleThread.Until(() => server.Count("/api/task/9106") > cardReads);
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            server.CardGate.Set();
            await Settled();
            Check("перенесли обратно, пока читалась заявка, — окна нет", Forms().Count == 0 && c.Status == TicketStatus.InProgress, () => $"окон: {Forms().Count}");

            //     две карточки подряд — окна по одному: второе — когда первое закрыли
            server.Quietly(9101, t => t.StatusId = Working);
            board.SelectedTicket = a;
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            ClearForms();
            var first = new TaskCompletionSource<bool>();
            StatusWindow.Answer = form => form.Ticket == 9101 ? first.Task : Task.FromResult(false);
            board.MoveToCommand.Execute(TicketStatus.Done);                                 // А
            await SingleThread.Until(() => Forms().Count == 1);
            board.SelectedTicket = c;
            board.MoveToCommand.Execute(TicketStatus.Done);                                 // Б
            await Task.Delay(300);
            Check("пока открыто окно первой карточки, окна второй нет", Forms().Count == 1, () => string.Join(", ", Forms().Select(x => x.Ticket)));
            first.SetResult(false);
            await Settled();
            Check("…закрыли первое — открылось второе", Forms().Select(x => x.Ticket).SequenceEqual(new[] { 9101, 9106 }),
                () => string.Join(", ", Forms().Select(x => x.Ticket)));

            // 11. 409, 403, 500 и молчание — окно с ошибкой, PUT один, карточка перечитана; «неизвестно» — в журнале
            var g = Card(9107);
            board.SelectedTicket = g;
            StatusWindow.Answer = form => { server.Comment(9107, Requester, RequesterId, "Уже не нужно"); return Apply("Готово")(form); };
            ClearTold();
            var count = server.Writes.Count;
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("изменили, пока открыто окно, — 409: окно с ошибкой, статус прежний", server.Writes.Count == count + 1 && Told() is [var (h409, t409)]
                && h409.Contains("не изменён") && t409.Contains("изменили") && g.ExternalStatus == "В работе",
                () => $"{string.Join(" | ", Told().Select(x => $"{x.Heading}: {x.Text}"))} · {g.ExternalStatus}");
            foreach (var (code, json, title, words) in new[]
            {
                (403, "{\"Message\":\"Нет прав\"}", "не изменён", "нет доступа"),
                (500, "{\"Message\":\"An error has occurred.\"}", "Неизвестно", "неизвестно"),
            })
            {
                board.MoveToCommand.Execute(TicketStatus.InProgress);
                await Settled();
                server.WriteAnswer = _ => (code, json);
                StatusWindow.Answer = Apply("Готово");
                ClearTold();
                logged.Clear();
                count = server.Writes.Count;
                board.MoveToCommand.Execute(TicketStatus.Done);
                await Settled();
                server.WriteAnswer = null;
                Check($"{code} — окно с ошибкой, PUT один", server.Writes.Count == count + 1 && Told() is [var (hh, tt)] && hh.Contains(title) && tt.Contains(words),
                    () => string.Join(" | ", Told().Select(x => $"{x.Heading}: {x.Text}")));
                if (code == 500) Check("…«неизвестно» — в журнале, с номером заявки", logged.Any(l => l.Contains("#9107")), () => string.Join("\n", logged));
            }
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            (server.WriteWait, HttpIntraserviceClient.WriteTimeout) = (TimeSpan.FromMilliseconds(1200), TimeSpan.FromMilliseconds(300));
            ClearTold();
            count = server.Writes.Count;
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            (server.WriteWait, HttpIntraserviceClient.WriteTimeout) = (TimeSpan.Zero, timeoutWas);
            Check("сервер молчит — «неизвестно», запись не повторилась", Told() is [var (hStall, _)] && hStall.Contains("Неизвестно")
                && server.Writes.Count == count + 1, () => $"{string.Join(" | ", Told().Select(x => x.Heading))} · записей: {server.Writes.Count - count}");
            Check("…карточка перечитана — на ней статус, какой стал", await SingleThread.Until(() => g.ExternalStatus == "Выполнена"), () => $"{g.ExternalStatus}");

            // 12. переносы автообновления и вопрос F5 — без окна и без записи
            server.Quietly(9101, t => t.StatusId = Working);
            board.SelectedTicket = a;
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            ClearForms();
            count = server.Writes.Count;
            server.Change(9101, t => t.StatusId = Fixed);                                    // закрыли в браузере
            await board.RefreshAllCommand.ExecuteAsync(null);                                  // F5: «перенести закрытые в «Готово»?» — да
            await Settled();
            Check("F5 перенёс закрытую в «Готово» — окна нет, записи нет", a.Status == TicketStatus.Done && Forms().Count == 0 && server.Writes.Count == count,
                () => $"{a.Status} · окон: {Forms().Count} · записей: {server.Writes.Count - count}");
            server.Change(9101, t => t.StatusId = Open);                                     // переоткрыли
            await board.AutoSyncAsync();
            await Settled();
            Check("автообновление вернуло переоткрытую во «Входящие» — окна нет, записи нет", a.Status == TicketStatus.Inbox
                && Forms().Count == 0 && server.Writes.Count == count, () => $"{a.Status} · окон: {Forms().Count} · записей: {server.Writes.Count - count}");

            // 13. выключили запись или сменили учётную запись, пока открыто окно, — не уходит
            board.SelectedTicket = c;
            server.Quietly(9106, t => t.StatusId = Working);
            StatusWindow.Answer = form =>
            {
                settings.AllowStatusWrite = false;
                board.ApplySettings(client);
                return Apply("Готово")(form);
            };
            ClearTold();
            count = server.Writes.Count;
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("выключили, пока открыто окно, — не уходит, сказано", server.Writes.Count == count && Told() is [var (_, offText)] && offText.Contains("настройки"),
                () => $"записей: {server.Writes.Count - count} · {string.Join(" | ", Told().Select(x => x.Text))}");
            settings.AllowStatusWrite = true;
            board.ApplySettings(client);
            board.MoveToCommand.Execute(TicketStatus.InProgress);
            await Settled();
            StatusWindow.Answer = form =>
            {
                settings.IntraserviceLogin = "other";
                board.ApplySettings(new HttpIntraserviceClient(server.Url, "other", "p"));
                return Apply("Готово")(form);
            };
            ClearTold();
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("сменили учётную запись, пока открыто окно, — не уходит", server.Writes.Count == count, () => $"записей: {server.Writes.Count - count}");
            settings.IntraserviceLogin = "me";
            board.ApplySettings(client);

            // 16. клиент: смена статуса без комментария — только StatusId и Changed
            server.WriteAnswer = _ => (204, "");
            var bare = await client.ChangeStatusAsync(9106, Working, "", false, "2026-01-01T00:00:00");
            server.WriteAnswer = null;
            Check("без комментария — тело только StatusId и Changed", bare.Outcome == WriteOutcome.Done && server.Writes is [.., var wBare]
                && Body(wBare).EnumerateObject().Select(p => p.Name).Order().SequenceEqual(new[] { "Changed", "StatusId" }), () => server.Writes[^1].Body);

            // своя карточка без номера — окна нет
            board.SelectedTicket = board.AllTickets.First(t => t.IntraserviceId is null);
            ClearForms();
            board.MoveToCommand.Execute(TicketStatus.Done);
            await Settled();
            Check("карточка без номера — окна нет", Forms().Count == 0);

            Check("доска менялась только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            Check("сервер не получал неожиданных запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
        }
        finally
        {
            server.Dispose();
            (StatusWindow.Answer, AskWindow.Answer, HttpIntraserviceClient.WriteTimeout) = (answerWas, askWas, timeoutWas);
        }
        }
        checks.AssertAll();
    }
}
