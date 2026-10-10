using System.Diagnostics;
using System.Globalization;
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
        var ui = Environment.CurrentManagedThreadId;   // поток окна (SingleThread)
        using var server = new BoardServer(new DateTime(now.Year, now.Month, now.Day, now.Hour, now.Minute, 0).AddDays(-2));
        var (budgetWas, giveUpWas, retryWas) = (ClosedExport.Budget, ExportFailures.GiveUpAfter, KnowledgeExport.RetryDelays);
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
                // с какой минуты просил список последний заход; «с недели назад» — с точностью до пары минут
                DateTime ListedSince() => DateTime.ParseExact(Uri.UnescapeDataString(server.Last("/api/task?").Split("ChangedMoreThan=")[1].Split('&')[0]),
                    "yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);
                bool FromWeekAgo() => Math.Abs((ListedSince() - DateTime.Now.AddDays(-7)).TotalMinutes) < 2;

                // 1. выключено — ни одного запроса
                var asked = Asked("/api/");
                Check("выключено — заход не начинается", await board.AutoExportAsync() == AutoSyncOutcome.Skipped);
                Check("…и ни одного запроса", Asked("/api/") == asked, () => $"запросов: {Asked("/api/") - asked}");
                Check("…и в подсказке трея ничего", board.AutoExportState == "", () => board.AutoExportState);

                // 2. первый заход: кто чьё
                settings.AutoExportClosed = true;
                Check("первый заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…одна страница: порция, номера минуты отметки и запас", Asked("/api/task?") == 1
                    && server.Last("/api/task?").Contains("pagesize=40&"), () => server.Last("/api/task?"));
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
                server.EmptyStatuses = true;
                Check("справочник статусов пуст — не удался, причина — на сервере", await board.AutoExportAsync() == AutoSyncOutcome.Failed
                    && logs.Any(l => l.Contains("ни одного статуса")) && settings.AutoExportSince == mark, Log);
                server.EmptyStatuses = false;

                // 7б. сбой карточки моей заявки — отметка перед ней; после неё «не мои» не перечитываются
                server.Put(830, "Моя, карточка сбоит", Final);
                server.Quietly(830, t => t.CardFails = true);
                server.Put(831, "Чужая после неё", Final, mine: false, executors: "Иванов И.");
                // обе — в ту же секунду, что последняя разобранная (массовое закрытие): «не моя» — ровно на отметке, но после сбойной
                server.SetChanged(830, server.ChangedOf(826));
                server.SetChanged(831, server.ChangedOf(826));
                server.Put(832, "Моя после неё", Final);
                server.Put(833, "Моя, история не отдаётся", Final);
                server.Quietly(833, t => t.LifetimeFails = 404);   // падает сразу, раньше повтора карточки 830
                logs.Clear();
                Check("сбой карточки — заход не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed, Log);
                Check("…отметка перед сбойной", settings.AutoExportSince == mark);
                Check("…в логе у сбойной — её собственная ошибка, а не соседней", logs.Any(l => l.Contains("#830") && l.Contains("HTTP 500"))
                    && !logs.Any(l => l.Contains("#830") && l.Contains("HTTP 404")), Log);
                Check("…моя после неё всё равно выгружена", FileOf(832) is not null);
                Check("повтор со сбоем — тоже не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                Check("…«не моя» после сбойной не перечитана, готовая — без запросов", Lifetimes(831) == 1 && Cards(832) == 1,
                    () => $"история 831: {Lifetimes(831)}, карточка 832: {Cards(832)}");
                server.Quietly(830, t => t.CardFails = false);
                server.Quietly(833, t => t.LifetimeFails = 0);
                Check("починили — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…сбойные выгружены, отметка — за последней", FileOf(830) is not null && FileOf(833) is not null
                    && settings.AutoExportSince == At(server.ChangedOf(833)));

                // 7в. история «не моей» не читается из-за самой заявки (500, 403) — заход идёт дальше; сбой сети (503) — стоп
                server.Put(834, "Чужая, история — 500", Final, mine: false, executors: "Иванов И.");
                server.Quietly(834, t => t.LifetimeFails = 500);
                server.Put(835, "Моя после неё", Final);
                server.Put(836, "Чужая, история — 503", Final, mine: false, executors: "Иванов И.");
                server.Quietly(836, t => t.LifetimeFails = 503);
                server.Put(837, "Чужая после неё", Final, mine: false, executors: "Иванов И.");
                mark = settings.AutoExportSince;
                Check("сбой истории — заход не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed, Log);
                Check("…отметка перед ней", settings.AutoExportSince == mark);
                Check("…моя после неё — выгружена тем же заходом", FileOf(835) is not null);
                Check("…после сбоя сети дальше не идём", Lifetimes(836) == 1 && Lifetimes(837) == 0,
                    () => $"история 836: {Lifetimes(836)}, 837: {Lifetimes(837)}");
                // 7г. сломанная заявка (500 на каждый запрос) — пять заходов подряд и не раньше часа (в проверке — сразу), и
                // отметка идёт дальше без неё; сбой сети в счёт не идёт: на нём отметка стоит, сколько бы ни длился
                ExportFailures.GiveUpAfter = TimeSpan.Zero;
                logs.Clear();
                for (var i = 2; i <= ExportFailures.GiveUpTimes; i++)
                    Check($"сломанная, заход {i} — не удался, отметка стоит", await board.AutoExportAsync() == AutoSyncOutcome.Failed
                        && settings.AutoExportSince == mark);
                Check("…шестой заход: сломанная пропущена — отметка за моей после неё, перед сбоем сети",
                    await board.AutoExportAsync() == AutoSyncOutcome.Failed && settings.AutoExportSince == At(server.ChangedOf(835)),
                    () => $"{settings.AutoExportSince:HH:mm:ss}, моя после неё — {server.ChangedOf(835):HH:mm:ss}");
                Check("…в логе — её номер, сколько раз и что делать", logs.Count(l => l.Contains("#834 пропущена")) == 1
                    && logs.Any(l => l.Contains("#834 пропущена") && l.Contains("5 заходов") && l.Contains("HTTP 500") && l.Contains("«Поиск заявок»")), Log);
                lifetimes = Lifetimes(834);
                for (var i = 0; i < ExportFailures.GiveUpTimes + 1; i++) await board.AutoExportAsync();
                Check("…пропущенную больше не спрашиваем", Lifetimes(834) == lifetimes, () => $"история 834: +{Lifetimes(834) - lifetimes}");
                Check("…сбой сети не пропускается, сколько бы заходов ни длился", settings.AutoExportSince == At(server.ChangedOf(835))
                    && Lifetimes(837) == 0 && !logs.Any(l => l.Contains("#836 пропущена")), Log);
                server.Quietly(836, t => t.LifetimeFails = 0);
                server.Quietly(834, t => t.LifetimeFails = 0);   // пропущена, отметка за ней; починим — для заходов с начала ниже
                Check("сеть починилась — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…отметка — за последней", settings.AutoExportSince == At(server.ChangedOf(837)));

                // 7д. моя, которая не выгружается (история — 404): после пяти заходов пропущена, в подсказке трея — «пропущено»;
                // изменилась — пробуем снова
                server.Put(838, "Моя, история не отдаётся никогда", Final);
                server.Quietly(838, t => t.LifetimeFails = 404);
                for (var i = 1; i <= ExportFailures.GiveUpTimes; i++)
                    Check($"не выгружается, заход {i} — не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed);
                logs.Clear();
                Check("…шестой: пропущена, заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…подсказка трея — «пропущено 1», файла нет, отметка — на ней", board.AutoExportState.Contains("пропущено 1")
                    && FileOf(838) is null && settings.AutoExportSince == At(server.ChangedOf(838)), () => board.AutoExportState);
                Check("…в логе — её номер и её ошибка", logs.Any(l => l.Contains("#838 пропущена") && l.Contains("HTTP 404")), Log);
                server.Quietly(838, t => t.LifetimeFails = 0);
                server.Comment(838, Requester, RequesterId, "Ну что там?");
                Check("изменилась — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…и она выгружена", FileOf(838) is not null);

                // 7е. сломанная — за моей, которая не выгружается из-за сети (503): сломанную пора пропустить, но отметка до неё
                // не дошла — счёт не теряется (не перечитываем), а в лог она попадает, когда отметка её пройдёт, — один раз
                KnowledgeExport.RetryDelays = new[] { TimeSpan.Zero, TimeSpan.Zero, TimeSpan.Zero };
                server.Put(842, "Моя, история — 503", Final);
                server.Quietly(842, t => t.LifetimeFails = 503);
                server.Put(843, "Чужая, история — 500", Final, mine: false, executors: "Иванов И.");
                server.Quietly(843, t => t.LifetimeFails = 500);
                logs.Clear();
                for (var i = 0; i < ExportFailures.GiveUpTimes + 2; i++) await board.AutoExportAsync();
                Check("…сломанная за сетевым сбоем: пропущена, но не перечитана и в лог ещё не попала",
                    Lifetimes(843) == ExportFailures.GiveUpTimes && !logs.Any(l => l.Contains("#843 пропущена")),
                    () => $"история 843: {Lifetimes(843)}\n{Log()}");
                server.Quietly(842, t => t.LifetimeFails = 0);
                Check("сеть починилась — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…моя выгружена, сломанная — в логе один раз, отметка за ней", FileOf(842) is not null
                    && logs.Count(l => l.Contains("#843 пропущена")) == 1 && settings.AutoExportSince == At(server.ChangedOf(843)), Log);
                server.Quietly(843, t => t.LifetimeFails = 0);
                KnowledgeExport.RetryDelays = retryWas;

                // 7ж. пока пишет другая выгрузка (окно «Поиск заявок») — автовыгрузка ждёт её, а не пишет те же файлы рядом
                server.Put(846, "Моя, пока идёт выгрузка из окна", Final);
                await KnowledgeExport.Writer.WaitAsync();
                var heldSince = Stopwatch.GetTimestamp();
                var waiting = board.AutoExportAsync();
                await Task.Delay(500);
                KnowledgeExport.Writer.Release();
                Check("…ждала, пока пишет другая выгрузка, потом выгрузила", await waiting == AutoSyncOutcome.Done && FileOf(846) is not null
                    && server.WaitedSince(heldSince, "/api/task/846") is { } waited && waited >= TimeSpan.FromMilliseconds(500),
                    () => $"карточка 846 — через {server.WaitedSince(heldSince, "/api/task/846")?.TotalMilliseconds} мс\n{Log()}");

                // 7з. файл моей заявки не записать (на его месте папка): это беда заявки, только пока другие файлы того же захода
                // пишутся (папка в порядке) — пять таких заходов, и пропуск; заход, где не записалось ничего, не в счёт (диск полон)
                var title847 = "Моя, путь файла занят папкой";
                server.Put(847, title847, Final);
                var blocked847 = Path.Combine(kb, KnowledgeExport.TicketsFolder,
                    KnowledgeExport.ShardOf(new DateTimeOffset(new DateTime(2026, 9, 1, 10, 0, 0))), KnowledgeExport.FileName(847, title847));
                Directory.CreateDirectory(blocked847);
                logs.Clear();
                Check("путь занят — не удался; ничего другого не записано — не в счёт", await board.AutoExportAsync() == AutoSyncOutcome.Failed
                    && await board.AutoExportAsync() == AutoSyncOutcome.Failed, Log);
                for (var i = 0; i < ExportFailures.GiveUpTimes; i++)
                {
                    server.Put(871 + i, $"Моя рядом с занятой {i}", Final);   // другой файл того же захода пишется
                    Check($"…заход {i + 1} с записью других — не удался", await board.AutoExportAsync() == AutoSyncOutcome.Failed
                        && FileOf(871 + i) is not null, Log);
                }
                Check("…следующий: её путь — её беда, пропущена, заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done
                    && logs.Any(l => l.Contains("#847 пропущена") && l.Contains("файл не записан")), Log);
                Directory.Delete(blocked847);   // пропущена, отметка за ней; уберём — для заходов с начала ниже
                ExportFailures.GiveUpAfter = giveUpWas;

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

                // 9б. равные изменения, номера — по убыванию (вторую часть sort сервер не соблюл): не беспорядок
                var tieAt = server.Tick();
                server.Put(855, "Одна минута: первая", Final);
                server.Put(856, "Одна минута: вторая", Final);
                server.SetChanged(855, tieAt);
                server.SetChanged(856, tieAt);
                server.TiesDescending = true;
                server.RepeatRow = 856;   // и строку сервер повторил
                Check("равные изменения в обратном порядке номеров — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…обе выгружены, отметка — на их минуте", FileOf(855) is not null && FileOf(856) is not null
                    && settings.AutoExportSince == At(tieAt) && settings.AutoExportSeen is { } tieSeen && tieSeen.Contains(855) && tieSeen.Contains(856));
                Check("…повторённая строка выгружена один раз", Cards(856) == 1 && Lifetimes(856) == 1,
                    () => $"карточек {Cards(856)}, историй {Lifetimes(856)}");
                server.TiesDescending = false;
                server.RepeatRow = 0;

                // 9в. в одну минуту изменилось больше заявок, чем сервер отдаёт за раз, и все отданные уже разобраны — ошибка, а не тишина
                var bulk = server.Tick();
                foreach (var id in Enumerable.Range(890, 8))
                {
                    server.Put(id, $"Массовое закрытие {id}", Final, mine: false, executors: "Иванов И.");
                    server.SetChanged(id, bulk);
                }
                server.PageCap = 5;
                logs.Clear();
                Check("минута больше страницы: первые заходы разбирают, что отдано", await board.AutoExportAsync() == AutoSyncOutcome.Done
                    && await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                mark = settings.AutoExportSince;
                Check("…а когда все отданные разобраны — не удался, с причиной", await board.AutoExportAsync() == AutoSyncOutcome.Failed
                    && logs.Any(l => l.Contains("за раз")), Log);
                Check("…отметка стоит", settings.AutoExportSince == mark);
                server.PageCap = 0;
                Check("сервер отдаёт больше — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…следующая страница — с номерами минуты отметки: порция, восемь и запас", await board.AutoExportAsync() == AutoSyncOutcome.Done
                    && server.Last("/api/task?").Contains("pagesize=48&"), () => server.Last("/api/task?"));
                Check("…каждая прочитана один раз, отметка — на их минуте со всеми номерами", Enumerable.Range(890, 8).All(id => Lifetimes(id) == 1)
                    && settings.AutoExportSince == At(bulk) && settings.AutoExportSeen is { } bulkSeen && Enumerable.Range(890, 8).All(bulkSeen.Contains),
                    () => string.Join(", ", Enumerable.Range(890, 8).Select(id => $"{id}: {Lifetimes(id)}")) + $"; [{string.Join(",", settings.AutoExportSeen ?? Array.Empty<int>())}]");

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
                Check("…и в подсказке трея прежней учётной записи ничего", board.AutoExportState == "", () => board.AutoExportState);
                Check("после смены — заход с начала прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…отметка снова есть", settings.AutoExportSince is not null);
                // выключили и тут же включили снова, пока заход шёл: отметка забыта — его итог не записывается поверх
                var otherAgain = new HttpIntraserviceClient(server.Url, "other", "p");
                server.ListGate.Reset();
                lists = Asked("/api/task?");
                running = board.AutoExportAsync();
                await SingleThread.Until(() => Asked("/api/task?") > lists);
                settings.AutoExportClosed = false;
                board.ApplySettings(otherAgain);
                settings.AutoExportClosed = true;
                board.ApplySettings(otherAgain);
                server.ListGate.Set();
                Check("выключили и включили посреди захода — итог не записан, отметка забыта", await running == AutoSyncOutcome.Dropped
                    && settings.AutoExportSince is null && settings.AutoExportSeen is null);
                Check("…следующий заход — с недели назад", await board.AutoExportAsync() == AutoSyncOutcome.Done && settings.AutoExportSince is not null, Log);
                // сменили папку базы посреди захода — его итог про прежнюю папку, не записывается
                server.ListGate.Reset();
                lists = Asked("/api/task?");
                mark = settings.AutoExportSince;
                running = board.AutoExportAsync();
                await SingleThread.Until(() => Asked("/api/task?") > lists);
                settings.KnowledgeDir = Path.Combine(data.Path, "moved");
                server.ListGate.Set();
                Check("сменили папку посреди захода — итог не записан", await running == AutoSyncOutcome.Dropped && settings.AutoExportSince == mark);
                settings.KnowledgeDir = kb;

                // 10. папка есть, но не репозиторий (без .git) — .gitignore не заводим
                var plain = Path.Combine(data.Path, "plain");
                Directory.CreateDirectory(plain);
                settings.KnowledgeDir = plain;
                server.Put(860, "В другую папку", Final);
                Check("заход в папку без .git прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…папка другая — заново, с недели назад, и отметка теперь этой папки", FromWeekAgo() && settings.AutoExportDir == plain,
                    () => $"{server.Last("/api/task?")}\n{settings.AutoExportDir}");
                Check("….gitignore не заведён", !File.Exists(Path.Combine(plain, ".gitignore")));

                // 10б. папка выгрузки — подпапка репозитория (корень — хранилище Obsidian): *.tmp — в .gitignore самой папки
                var vault = Path.Combine(data.Path, "vault");
                Directory.CreateDirectory(Path.Combine(vault, ".git"));
                var inner = Path.Combine(vault, "Заявки");
                Directory.CreateDirectory(inner);
                settings.KnowledgeDir = inner;
                server.Put(861, "В подпапку репозитория", Final);
                Check("заход в подпапку репозитория прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("….gitignore папки выгрузки — с *.tmp, корень репозитория не тронут",
                    File.Exists(Path.Combine(inner, ".gitignore")) && File.ReadAllText(Path.Combine(inner, ".gitignore")) == "*.tmp\n"
                    && !File.Exists(Path.Combine(vault, ".gitignore")));

                // 10в. .git — файл, а не папка (рабочее дерево git worktree, подмодуль)
                var worktree = Path.Combine(data.Path, "worktree");
                Directory.CreateDirectory(worktree);
                File.WriteAllText(Path.Combine(worktree, ".git"), "gitdir: ../vault/.git/worktrees/kb\n");
                settings.KnowledgeDir = worktree;
                server.Put(862, "В рабочее дерево", Final);
                Check("заход в рабочее дерево прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("….gitignore с *.tmp заведён", File.Exists(Path.Combine(worktree, ".gitignore"))
                    && File.ReadAllText(Path.Combine(worktree, ".gitignore")) == "*.tmp\n");

                // 10г. папку выгрузки не создать (на её месте файл, диска нет) — заход не удался, а не упал; в лог один раз
                var blocker = Path.Combine(data.Path, "blocker.txt");
                File.WriteAllText(blocker, "");
                settings.KnowledgeDir = Path.Combine(blocker, "kb");
                server.Put(863, "В несоздаваемую папку", Final);
                logs.Clear();
                async Task<AutoSyncOutcome?> Safely()
                {
                    try { return await board.AutoExportAsync(); }
                    catch (Exception ex) { logs.Add("ИСКЛЮЧЕНИЕ: " + ex.GetType().Name + ": " + ex.Message); return null; }
                }
                Check("папку не создать — заход не удался, а не упал", await Safely() == AutoSyncOutcome.Failed
                    && await Safely() == AutoSyncOutcome.Failed && board.AutoExportState.Contains("не удалось"), Log);
                Check("…в лог — один раз", logs.Count == 1 && logs[0].StartsWith("Автовыгрузка"), Log);
                settings.KnowledgeDir = kb;
                AutoSyncOutcome? lastPass = null;
                for (var i = 0; i < 6 && FileOf(863) is null; i++) lastPass = await board.AutoExportAsync();
                Check("папка снова есть — заходы с недели назад проходят, заявка выгружена", lastPass == AutoSyncOutcome.Done && FileOf(863) is not null, Log);

                // 10д. папки выгрузки ещё нет (новая подпапка хранилища): создаётся, и *.tmp в её .gitignore — до первого файла
                var fresh = Path.Combine(vault, "Новая");
                settings.KnowledgeDir = fresh;
                logs.Clear();
                Check("папки ещё не было — заход прошёл, в логе пусто", await board.AutoExportAsync() == AutoSyncOutcome.Done && logs.Count == 0, Log);
                Check("…папка создана, *.tmp в её .gitignore — тем же заходом", File.Exists(Path.Combine(fresh, ".gitignore"))
                    && File.ReadAllText(Path.Combine(fresh, ".gitignore")) == "*.tmp\n");

                // 10е. .gitignore не дописать (на его месте папка; нет доступа) — в лог раз за запуск, а не каждым заходом
                var stuck = Path.Combine(vault, "Закрытая");
                Directory.CreateDirectory(Path.Combine(stuck, ".gitignore"));
                settings.KnowledgeDir = stuck;
                logs.Clear();
                var logThreads = new List<int>();
                board.Log = s => { logs.Add(s); logThreads.Add(Environment.CurrentManagedThreadId); };
                Check(".gitignore не дописать — заходы проходят", await board.AutoExportAsync() == AutoSyncOutcome.Done
                    && await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                Check("…в лог — один раз", logs.Count(l => l.Contains("не удалось дописать *.tmp")) == 1, Log);
                Check("…и не из потока окна: файлы папки трогает заход в пуле", logs.Zip(logThreads)
                    .Where(p => p.First.Contains("не удалось дописать *.tmp")).All(p => p.Second != ui), () => string.Join(", ", logThreads));
                board.Log = logs.Add;

                // 12. выключили выгрузку — отметка забыта (включат — снова с недели назад), подсказка трея пуста;
                // автообновление 0 минут — подсказка пуста, а отметка цела (выгрузка не выключена — догонит)
                var other = new HttpIntraserviceClient(server.Url, "other", "p");
                Check("перед выключением в подсказке трея — итог", board.AutoExportState.Length > 0);
                settings.AutoExportClosed = false;
                board.ApplySettings(other);
                Check("выключили — отметка забыта", settings.AutoExportSince is null && settings.AutoExportSeen is null);
                Check("…и в settings.json", AppSettings.Load(data.Path, out _) is { AutoExportSince: null, AutoExportSeen: null });
                Check("…подсказка трея пуста", board.AutoExportState == "", () => board.AutoExportState);
                settings.AutoExportClosed = true;
                board.ApplySettings(other);
                Check("включили снова — заход прошёл", await board.AutoExportAsync() == AutoSyncOutcome.Done, Log);
                var from = server.Last("/api/task?");
                Check("…с недели назад", from.Contains("ChangedMoreThan=") && Math.Abs((DateTime.ParseExact(Uri.UnescapeDataString(
                    from.Split("ChangedMoreThan=")[1].Split('&')[0]), "yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture) - DateTime.Now.AddDays(-7)).TotalMinutes) < 2, () => from);
                mark = settings.AutoExportSince;
                var minutesWas = settings.AutoSyncMinutes;
                settings.AutoSyncMinutes = 0;
                board.ApplySettings(other);
                Check("автообновление 0 минут — подсказка трея пуста, отметка цела", board.AutoExportState == "" && settings.AutoExportSince == mark
                    && mark is not null, () => board.AutoExportState);
                settings.AutoSyncMinutes = minutesWas;
                board.ApplySettings(other);

                // 13. доска не тронута, всё — в своём потоке, лишних запросов нет
                Check("доска не тронута", !board.AllTickets.Any());
                Check("всё менялось в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
                Check("сервер не видел незнакомых запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
            }
            finally
            {
                ClosedExport.Budget = budgetWas;
                ExportFailures.GiveUpAfter = giveUpWas;
                KnowledgeExport.RetryDelays = retryWas;
                server.ListGate.Set();
            }
        }
        checks.AssertAll();
    }
}
