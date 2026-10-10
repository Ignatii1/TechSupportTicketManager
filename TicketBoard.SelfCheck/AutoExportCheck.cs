using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using static TicketBoard.SelfCheck.BoardServer;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка автовыгрузки закрытых заявок (MainViewModel.AutoExportAsync → ClosedExport) против BoardServer: что
/// уходит в базу знаний (исполнитель, инициатор, участник) и что нет, отметка и её минута, порции, сбои списка и
/// карточки, смена учётной записи посреди захода, строки без даты и не по порядку, .gitignore, строка для трея. Доска
/// при этом не меняется. Правила без сервера — ClosedExport.SelfCheck().</summary>
internal static class AutoExportCheck
{
    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("автовыгрузка");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        // часы сервера — двое суток назад, в окне первого захода (неделя)
        var now = DateTime.Now;
        using var server = new BoardServer(new DateTime(now.Year, now.Month, now.Day, now.Hour, now.Minute, 0).AddDays(-2));
        var budgetWas = ClosedExport.Budget;
        using (var data = new TempDataDir("tb-autoexport"))
        {
            try
            {
                // ---- сервер: закрытые разных видов ----
                server.Put(801, "Исполнитель — я", Final);
                server.Put(802, "Инициатор — я", Final, mine: false, executors: "Иванов И.");
                server.Change(802, t => (t.Creator, t.CreatorId) = (MeName, MeId));
                server.Put(803, "Только мой комментарий", Working, mine: false, executors: "Иванов И.");
                server.Comment(803, MeName, MeId, "Посмотрел логи");
                server.StatusChange(803, Final, "Иванов И.", OtherId);
                server.Put(804, "Только моя смена статуса", Working, mine: false, executors: "Иванов И.");
                server.StatusChange(804, Fixed, MeName, MeId);
                server.Put(805, "Чужая закрытая", Working, mine: false, executors: "Иванов И.");
                server.Comment(805, Requester, RequesterId, "Я Сам, посмотрите, пожалуйста");
                server.StatusChange(805, Final, "Иванов И.", OtherId);
                server.Put(806, "Моя открытая");
                server.Put(807, "Давняя закрытая", Final);
                server.SetChanged(807, now.AddDays(-8));
                server.Put(808, "Отменённая — я исполнитель", Cancelled);   // закрыта по названию статуса (ClosedStatusNames)
                server.Put(809, "Без номеров: исполнитель — я", Final);
                server.Quietly(809, t => t.NoIds = true);
                server.Put(810, "Без номеров: инициатор — я", Final, mine: false, executors: "Иванов И.");
                server.Quietly(810, t => (t.NoIds, t.Creator) = (true, MeName));

                var kb = Path.Combine(data.Path, "kb");
                Directory.CreateDirectory(Path.Combine(kb, ".git"));
                File.WriteAllText(Path.Combine(kb, ".gitignore"), ".obsidian/workspace.json\n");
                var settings = new AppSettings { IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me", KnowledgeDir = kb };
                var board = new MainViewModel(new TicketStore(Path.Combine(data.Path, "board")), settings,
                    new IntraserviceLinkParser(settings), new HttpIntraserviceClient(server.Url, "me", "p"));
                BoardCheck.WatchBoard(pump, board);
                var logs = new List<string>();
                board.Log = logs.Add;
                string? FileOf(int id) => Directory.Exists(Path.Combine(kb, "tickets"))
                    ? Directory.GetFiles(Path.Combine(kb, "tickets"), $"{id} — *.md", SearchOption.AllDirectories).SingleOrDefault() : null;
                int Asked(string prefix) => server.Count(prefix);
                int Lifetimes(int id) => Asked($"/api/tasklifetime?taskid={id}&");
                int Cards(int id) => Asked($"/api/task/{id}");
                string Log() => string.Join("\n", logs);

                // 1. выключено — ни одного запроса
                var asked = Asked("/api/");
                Check("выключено — заход не начинается", await board.AutoExportAsync() == AutoSyncOutcome.Skipped);
                Check("…и ни одного запроса", Asked("/api/") == asked, () => $"запросов: {Asked("/api/") - asked}");
                Check("…и в подсказке трея ничего", board.AutoExportState == "", () => board.AutoExportState);

                // 2. первый заход: кто чьё
                settings.AutoExportClosed = true;
                Check("первый заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                foreach (var (id, why) in new[] { (801, "исполнитель — я"), (802, "инициатор — я"), (803, "только мой комментарий"),
                             (804, "только моя смена статуса"), (808, "закрыта по названию статуса"), (809, "исполнитель по имени"),
                             (810, "инициатор по имени") })
                    Check($"выгружена: {why}", FileOf(id) is not null);
                foreach (var (id, why) in new[] { (805, "чужая (моё имя — только в тексте комментария)"), (806, "моя открытая"),
                             (807, "закрытая раньше недели") })
                    Check($"не выгружена: {why}", FileOf(id) is null);
                Check("чужая: история прочитана один раз, карточка — ни разу", Lifetimes(805) == 1 && Cards(805) == 0,
                    () => $"историй {Lifetimes(805)}, карточек {Cards(805)}");
                Check("мои по строке: история — один раз, для файла", Lifetimes(801) == 1 && Lifetimes(802) == 1 && Lifetimes(809) == 1,
                    () => $"{Lifetimes(801)}, {Lifetimes(802)}, {Lifetimes(809)}");
                Check("мои по истории: история — дважды, разбор и файл", Lifetimes(803) == 2 && Lifetimes(804) == 2,
                    () => $"{Lifetimes(803)}, {Lifetimes(804)}");
                Check("в файле нет телефона инициатора", FileOf(801) is { } f801 && !File.ReadAllText(f801).Contains(Phone));
                Check("подсказка трея: время и сколько выгружено", board.AutoExportState.StartsWith("база знаний: ")
                    && board.AutoExportState.Contains("выгружено 7"), () => board.AutoExportState);

                // 3. отметка: изменение последней разобранной и номера её минуты — и в settings.json
                var last = new[] { 801, 802, 803, 804, 805, 808, 809, 810 }.MaxBy(server.ChangedOf);
                Check("отметка — изменение последней разобранной", settings.AutoExportSince == At(server.ChangedOf(last))
                    && settings.AutoExportSeen is { } seen && seen.Contains(last),
                    () => $"{settings.AutoExportSince:HH:mm:ss.fff} / {At(server.ChangedOf(last)):HH:mm:ss.fff}; [{string.Join(",", settings.AutoExportSeen ?? Array.Empty<int>())}]");
                Check("…и в settings.json", AppSettings.Load(data.Path, out _) is { AutoExportSince: { } saved } && saved == settings.AutoExportSince);
                var gitignore = File.ReadAllText(Path.Combine(kb, ".gitignore"));
                Check(".gitignore: строка *.tmp дописана, прежние целы", gitignore.Split('\n').Count(l => l.Trim() == "*.tmp") == 1
                    && gitignore.Contains(".obsidian/workspace.json"), () => gitignore);

                // 4. повторный заход без перемен — только список
                var (cards, lifetimes, lists) = (Asked("/api/task/"), Asked("/api/tasklifetime"), Asked("/api/task?"));
                Check("повторный заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…ни карточек, ни историй, один список", Asked("/api/task/") == cards && Asked("/api/tasklifetime") == lifetimes
                    && Asked("/api/task?") == lists + 1, () => $"карточек +{Asked("/api/task/") - cards}, историй +{Asked("/api/tasklifetime") - lifetimes}, списков +{Asked("/api/task?") - lists}");
                Check("…подсказка трея: новых нет", board.AutoExportState.Contains("новых нет"), () => board.AutoExportState);
                Check(".gitignore: второй раз не дописывается", File.ReadAllText(Path.Combine(kb, ".gitignore")) == gitignore);

                // 5. новая закрытая и дописанная старая закрытая — следующим заходом
                server.Put(811, "Новая закрытая", Final);
                server.Comment(801, Requester, RequesterId, "Спасибо, всё работает");
                Check("заход после перемен прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("новая закрытая — выгружена", FileOf(811) is not null);
                Check("дописанная закрытая — файл обновлён", FileOf(801) is { } again801 && File.ReadAllText(again801).Contains("Спасибо, всё работает"));
                Check("…в подсказке трея — и новая, и обновлённая", board.AutoExportState.Contains("выгружено 2"), () => board.AutoExportState);
                // новый запуск (своя память) с той же папкой: строка *.tmp уже есть — второй раз не дописывается
                var restarted = new MainViewModel(new TicketStore(Path.Combine(data.Path, "board2")), settings,
                    new IntraserviceLinkParser(settings), new HttpIntraserviceClient(server.Url, "me", "p"));
                Check("новый запуск: заход прошёл", await restarted.AutoExportAsync() == AutoSyncOutcome.Done);
                Check("….gitignore после перезапуска — строка *.tmp одна", File.ReadAllText(Path.Combine(kb, ".gitignore")) == gitignore,
                    () => File.ReadAllText(Path.Combine(kb, ".gitignore")));

                // 6. порции: семь заявок при порции 3; четыре — в одну минуту, и граница порции — между равными
                ClosedExport.Budget = 3;
                var tie = server.Tick();
                server.Put(820, "Порция: моя", Final);
                server.Put(821, "Порция: чужая", Final, mine: false, executors: "Иванов И.");
                server.SetChanged(821, tie.AddSeconds(10));
                server.Put(822, "Порция: моя, минута", Final);
                server.SetChanged(822, tie.AddSeconds(20));
                server.Put(823, "Порция: чужая, минута", Final, mine: false, executors: "Иванов И.");
                server.SetChanged(823, tie.AddSeconds(20));
                server.Put(824, "Порция: моя, та же секунда", Final);
                server.SetChanged(824, tie.AddSeconds(20));
                server.Put(825, "Порция: чужая, позже", Final, mine: false, executors: "Иванов И.");
                server.Put(826, "Порция: моя, последняя", Final);
                Check("порция 1 прошла", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…кончилась порция — «догоняю»", board.AutoExportState.Contains("догоняю"), () => board.AutoExportState);
                Check("…ровно три: 821, 822, 823 (по порядку изменения)", FileOf(822) is not null && FileOf(824) is null
                    && Lifetimes(821) == 1 && Lifetimes(823) == 1 && Lifetimes(825) == 0);
                Check("порция 2 прошла", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("порция 3 прошла", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("заход после порций прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("все мои выгружены, ни одна не потеряна", new[] { 820, 822, 824, 826 }.All(id => FileOf(id) is not null),
                    () => string.Join(", ", new[] { 820, 822, 824, 826 }.Where(id => FileOf(id) is null)));
                Check("ни одна не прочитана дважды", new[] { 821, 823, 825 }.All(id => Lifetimes(id) == 1)
                    && new[] { 820, 822, 824, 826 }.All(id => Cards(id) == 1),
                    () => string.Join(", ", new[] { 820, 821, 822, 823, 824, 825, 826 }.Select(id => $"{id}: истор. {Lifetimes(id)}, карт. {Cards(id)}")));
                Check("отметка — на последней", settings.AutoExportSince == At(server.ChangedOf(826)));
                ClosedExport.Budget = budgetWas;

                // 7а. сбой списка — отметка стоит, в лог один раз
                logs.Clear();
                var mark = settings.AutoExportSince;
                server.FailList = 503;
                Check("сбой списка — заход не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                Check("…и второй тоже", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                Check("…отметка стоит", settings.AutoExportSince == mark, () => $"{settings.AutoExportSince:HH:mm:ss.fff}, была {mark:HH:mm:ss.fff}");
                Check("…в лог — один раз за два сбойных захода", logs.Count == 1 && logs[0].StartsWith("Автовыгрузка"), Log);
                Check("…подсказка трея: не удалось", board.AutoExportState.Contains("не удалось"), () => board.AutoExportState);
                server.FailList = 0;

                // 7б. сбой карточки моей заявки — отметка перед ней; после неё «не мои» не перечитываются
                server.Put(830, "Моя, карточка сбоит", Final);
                server.Quietly(830, t => t.CardFails = true);
                server.Put(831, "Чужая после неё", Final, mine: false, executors: "Иванов И.");
                // обе — в ту же секунду, что последняя разобранная (массовое закрытие): «не моя» — ровно на отметке, но после сбойной
                server.SetChanged(830, server.ChangedOf(826));
                server.SetChanged(831, server.ChangedOf(826));
                server.Put(832, "Моя после неё", Final);
                Check("сбой карточки — заход не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed, Log);
                Check("…отметка перед сбойной", settings.AutoExportSince == mark);
                Check("…моя после неё всё равно выгружена", FileOf(832) is not null);
                Check("повтор со сбоем — тоже не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                Check("…«не моя» после сбойной не перечитана, готовая — без запросов", Lifetimes(831) == 1 && Cards(832) == 1,
                    () => $"история 831: {Lifetimes(831)}, карточка 832: {Cards(832)}");
                server.Quietly(830, t => t.CardFails = false);
                Check("починили — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…сбойная выгружена, отметка — за последней", FileOf(830) is not null && settings.AutoExportSince == At(server.ChangedOf(832)));

                // 9. строки без даты изменения и не по порядку — дальше не идём
                mark = settings.AutoExportSince;
                server.Put(850, "Новая, без даты в строке", Final);
                server.Put(851, "Ещё одна", Final);
                server.NoChanged = true;
                lifetimes = Asked("/api/tasklifetime");
                Check("строки без Changed — заход не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                Check("…отметка стоит, историй не читаем", settings.AutoExportSince == mark && Asked("/api/tasklifetime") == lifetimes);
                server.NoChanged = false;
                server.IgnoreSort = true;
                Check("список не по возрастанию — заход не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                Check("…отметка стоит", settings.AutoExportSince == mark, () => $"{settings.AutoExportSince:HH:mm:ss.fff}, была {mark:HH:mm:ss.fff}");
                server.IgnoreSort = false;
                Check("сервер исправился — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);

                // 8. посреди захода выключили выгрузку или сменили учётную запись — итог не записывается
                mark = settings.AutoExportSince;
                server.Put(840, "Закрыта, пока выключали", Final);
                server.ListGate.Reset();
                lists = Asked("/api/task?");
                var running = board.AutoExportAsync();
                await SingleThread.Until(() => Asked("/api/task?") > lists);
                settings.AutoExportClosed = false;
                server.ListGate.Set();
                Check("выключили посреди захода — итог не записан", await running == AutoSyncOutcome.Dropped && settings.AutoExportSince == mark);
                settings.AutoExportClosed = true;
                server.ListGate.Reset();
                lists = Asked("/api/task?");
                running = board.AutoExportAsync();
                await SingleThread.Until(() => Asked("/api/task?") > lists);
                settings.IntraserviceLogin = "other";
                board.ApplySettings(new HttpIntraserviceClient(server.Url, "other", "p"));
                server.ListGate.Set();
                Check("сменили учётную запись посреди захода — итог не записан", await running == AutoSyncOutcome.Dropped);
                Check("…отметка сброшена — заново, с недели назад", settings.AutoExportSince is null && settings.AutoExportSeen is null);
                Check("после смены — заход с начала прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…отметка снова есть", settings.AutoExportSince is not null);

                // 10. папка есть, но не репозиторий (без .git) — .gitignore не заводим
                var plain = Path.Combine(data.Path, "plain");
                Directory.CreateDirectory(plain);
                settings.KnowledgeDir = plain;
                server.Put(860, "В другую папку", Final);
                Check("заход в папку без .git прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("….gitignore не заведён", !File.Exists(Path.Combine(plain, ".gitignore")));

                // 13. доска не тронута, всё — в своём потоке, лишних запросов нет
                Check("доска не тронута", !board.AllTickets.Any());
                Check("всё менялось в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
                Check("сервер не видел незнакомых запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
            }
            finally
            {
                ClosedExport.Budget = budgetWas;
                server.ListGate.Set();
            }
        }
        checks.AssertAll();
    }
}
