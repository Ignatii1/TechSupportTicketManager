using System.Diagnostics;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using static TicketBoard.SelfCheck.BoardServer;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка панели карточки (этап 5б): поля 2.3 (должность и организация инициатора, сервис, тип, категории,
/// оборудование) и вложения — тихое чтение карточки при открытии (пауза, свежесть, только поля), размеры, «Открыть» (папка
/// временных, программы не запускаются), «Сохранить», ошибки, занятость, автообновление открытой карточки. Настоящий
/// MainViewModel против поддельного сервера (BoardServer), в одном потоке (SingleThread); что нужно от Windows (открыть
/// программой, Проводник, окно сохранения) — заглушка FileShell, она записывает вызовы.</summary>
internal static class PanelCheck
{
    /// <summary>Пауза перед чтением при открытии — 400 мс, как у переписки; с запасом на грубый таймер Windows.</summary>
    private static readonly TimeSpan Pause = TimeSpan.FromMilliseconds(350);

    public static void Run() => SingleThread.Run(RunAsync);

    private static byte[] Bytes(int length, int seed) => Enumerable.Range(0, length).Select(i => (byte)((i * 31 + seed) % 251)).ToArray();

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("панель");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        var server = new BoardServer();
        var (cacheWas, freshWas, saveWas, delaysWas, writeDelaysWas) = (MainViewModel.FilesCache, MainViewModel.CardFresh, FileShell.SaveAnswer,
            KnowledgeExport.RetryDelays, KnowledgeExport.WriteRetryDelays);
        using (var data = new TempDataDir("tb-panel"))
        {
        try
        {
            var cache = MainViewModel.FilesCache = Path.Combine(data.Path, "временные");
            var saveDir = Path.Combine(data.Path, "сохранённые");
            Directory.CreateDirectory(saveDir);

            // ---- сервер ----
            var (png, exe, pdf, docx, xlsx, lost, fresh) = (Bytes(3000, 1), Bytes(500, 2), Bytes(4000, 3), Bytes(7000, 4), Bytes(9000, 5), Bytes(100, 6), Bytes(20, 7));
            server.Put(7101, "Скриншот ошибки");
            server.Change(7101, t =>
            {
                (t.Service, t.Type, t.Categories, t.Assets, t.Company, t.Position) = ("Почта", "Инцидент", "Печать, Картриджи", "Принтер HP", "ООО Ромашка", "Бухгалтер");
                t.Files.AddRange(new BoardServer.Attachment[]
                {
                    new(900101, "экран.png", png), new(900102, "setup.exe", exe), new(900103, "акт.pdf", pdf) { Missing = true },
                    new(900104, "отчёт.docx", docx), new(900105, "смета.xlsx", xlsx), new(900106, "пропал.pdf", lost) { Missing = true },
                });
            });
            server.Put(7102, "Без вложений");
            server.Change(7102, t => t.Service = "1С");
            server.Put(7103, "Старая карточка", Working);
            server.Change(7103, t => { t.Service = "Почта"; t.Files.Add(new(900110, "лог.txt", Bytes(100, 8))); });
            server.Put(7104, "Без имён");
            server.Change(7104, t => { t.Files.Add(new(900120, "скрыто.png", Bytes(10, 9))); t.NoFileNames = true; });
            foreach (var (id, name) in new[] { (7105, "Первая"), (7106, "Вторая"), (7107, "Третья"), (7108, "Добавленная") }) server.Put(id, name);

            // ---- доска: карточки до 0.20.0 — ни полей карточки, ни вложений ----
            var boardDir = Path.Combine(data.Path, "board");
            new TicketStore(boardDir).Save(new[]
            {
                new Ticket { Title = "Скриншот ошибки", IntraserviceId = 7101, ExternalStatus = "Открыта", Status = TicketStatus.InProgress },
                new Ticket { Title = "Без вложений", IntraserviceId = 7102, Status = TicketStatus.InProgress },
                new Ticket { Title = "Старая карточка", IntraserviceId = 7103, ExternalStatus = "Открыта", Status = TicketStatus.InProgress },
                new Ticket { Title = "Без имён", IntraserviceId = 7104, Status = TicketStatus.Waiting },
                new Ticket { Title = "Первая", IntraserviceId = 7105, Status = TicketStatus.Waiting },
                new Ticket { Title = "Вторая", IntraserviceId = 7106, Status = TicketStatus.Waiting },
                new Ticket { Title = "Третья", IntraserviceId = 7107, Status = TicketStatus.Waiting },
            });
            var settings = new AppSettings { IntraserviceBaseUrl = server.Url, IntraserviceLogin = "me" };
            var client = new HttpIntraserviceClient(server.Url, "me", "p");
            var board = new MainViewModel(new TicketStore(boardDir), settings, new IntraserviceLinkParser(settings), client);
            BoardCheck.WatchBoard(pump, board);
            pump.WatchItems(board.AttachmentItems);

            Ticket Card(string title) => board.AllTickets.First(t => t.Title == title);
            AttachmentItem Item(string name) => board.AttachmentItems.First(a => a.Name == name);
            string[] Names() => board.AttachmentItems.Select(a => a.Name).ToArray();
            int Files(int id) => server.Count($"/api/taskfile/{id}");
            // папка временных — по серверу и заявке: одни номера на двух серверах — разные файлы
            string CacheOf(int ticket) => MainViewModel.FilesFolder(server.Url, ticket);
            Check("папка временных — своя у каждого сервера; «/» в конце и регистр адреса — тот же сервер",
                MainViewModel.FilesFolder("https://a.corp", 7) != MainViewModel.FilesFolder("https://b.corp", 7)
                && MainViewModel.FilesFolder("https://a.corp/", 7) == MainViewModel.FilesFolder("HTTPS://A.corp", 7)
                && Path.GetFileName(MainViewModel.FilesFolder("https://a.corp", 7)) == "7");
            string? CachedFile(int ticket, int file) => Directory.Exists(CacheOf(ticket))
                ? Directory.GetFiles(CacheOf(ticket), $"{file} *").Where(f => !f.EndsWith(".tmp")).SingleOrDefault() : null;
            bool TempsIn(string dir) => Directory.Exists(dir) && Directory.EnumerateFiles(dir, "*.tmp", SearchOption.AllDirectories).Any();
            void WatchItems() { foreach (var a in board.AttachmentItems) pump.Watch(a); }
            // пометка «из интернета» — поток Zone.Identifier: есть только в Windows (NTFS), здесь — в CI
            void Marked(string name, string? path)
            {
                if (!OperatingSystem.IsWindows()) { checks.Skipped(name, "потоки NTFS есть только в Windows — проверяется в CI"); return; }
                var zone = "";
                try { zone = path is null ? "" : File.ReadAllText(path + ":Zone.Identifier"); } catch (IOException) { }
                Check(name, zone.Contains("ZoneId=3") && zone.Contains("HostUrl=" + server.Url), () => zone);
            }

            // 5. ⟳ у карточки с полями 2.3: поля в карточке доски, тексты панели и карточки
            var shot = Card("Скриншот ошибки");
            board.SelectedTicket = shot;
            await board.RefreshFromIntraserviceCommand.ExecuteAsync(null);
            Check("⟳: поля карточки — в карточке доски", shot is
                {
                    Service: "Почта", TaskType: "Инцидент", Categories: "Печать, Картриджи", Assets: "Принтер HP",
                    CreatorCompany: "ООО Ромашка", CreatorPosition: "Бухгалтер",
                }, () => $"{shot.Service} | {shot.TaskType} | {shot.Categories} | {shot.Assets} | {shot.CreatorCompany} | {shot.CreatorPosition}");
            Check("…тексты: под инициатором «должность · организация», «сервис · тип», на карточке «сервис · организация»",
                shot is { CreatorRole: "Бухгалтер · ООО Ромашка", ServiceText: "Почта · Инцидент", CardContext: "Почта · ООО Ромашка" },
                () => $"{shot.CreatorRole} | {shot.ServiceText} | {shot.CardContext}");
            Check("…вложения карточки — в карточке доски и в панели, по порядку", shot.Files is { Count: 6 } && shot.FilesCount == 6
                && Names().SequenceEqual(new[] { "экран.png", "setup.exe", "акт.pdf", "отчёт.docx", "смета.xlsx", "пропал.pdf" }) && board.HasAttachments,
                () => string.Join(", ", Names()));
            WatchItems();

            // 7. размеры — после паузы, по запросу на файл (тело не дочитывается); у пропавшего на сервере размера нет
            Check("размеры пришли", await SingleThread.Until(() => Item("экран.png").Size == "2,9 КБ" && Item("setup.exe").Size == "500 Б"
                    && Item("отчёт.docx").Size == "6,8 КБ" && Item("смета.xlsx").Size == "8,8 КБ" && Files(900106) == 1),
                () => string.Join(", ", board.AttachmentItems.Select(a => $"{a.Name}: «{a.Size}»")));
            Check("…по запросу на файл; у пропавшего — пусто, без ошибки (её скажет «Открыть»)",
                new[] { 900101, 900102, 900103, 900104, 900105, 900106 }.All(id => Files(id) == 1) && Item("акт.pdf") is { Size: "", Status: "" },
                () => string.Join(", ", new[] { 900101, 900102, 900103, 900104, 900105, 900106 }.Select(id => $"{id}: {Files(id)}")));
            Check("…и запросом одного байта (Range): тело файла ради размера не идёт", server.RangesServed == 4, () => $"{server.RangesServed}");
            // 6. ⟳ уже прочитал карточку — панель её второй раз не читает
            Check("⟳ прочитал карточку — панель после паузы её не перечитывает", server.Count("/api/task/7101?") == 1,
                () => $"{server.Count("/api/task/7101?")}");
            board.SelectedTicket = null;
            board.SelectedTicket = shot;
            await Task.Delay(Pause + Pause);
            Check("снова открыли — ни карточки, ни размеров заново, и непришедший (404) не спрашиваем снова",
                server.Count("/api/task/7101?") == 1 && Files(900101) == 1 && Files(900104) == 1 && Files(900103) == 1);

            // 8. «Открыть» картинку — скачана побайтно в папку временных и открыта; второй раз — без запроса
            FileShell.Opened.Clear();
            await board.OpenAttachmentCommand.ExecuteAsync(Item("экран.png"));
            var shotPng = CachedFile(7101, 900101);
            Check("«Открыть»: скачана побайтно в папку временных и открыта", shotPng is not null && File.ReadAllBytes(shotPng).SequenceEqual(png)
                && FileShell.Opened.SequenceEqual(new[] { shotPng }) && Files(900101) == 2 && Item("экран.png").Status == "",
                () => $"{shotPng}; открыты: {string.Join(", ", FileShell.Opened)}; запросов: {Files(900101)}; «{Item("экран.png").Status}»");
            Marked("…и помечена «из интернета»", shotPng);
            Check("…в папке временных — только для чтения: правка в Word не ляжет поверх присланного",
                shotPng is not null && File.GetAttributes(shotPng).HasFlag(FileAttributes.ReadOnly));
            await board.OpenAttachmentCommand.ExecuteAsync(Item("экран.png"));
            Check("…второй раз — без запроса", Files(900101) == 2 && FileShell.Opened.Count == 2 && FileShell.Opened[^1] == shotPng);
            // лежит с прошлого запуска (папка временных переживает перезапуск) — целым: туда кладётся только скачанное до конца
            var earlier = Path.Combine(CacheOf(7101), "900104 — отчёт.docx");
            File.WriteAllBytes(earlier, docx);
            await board.OpenAttachmentCommand.ExecuteAsync(Item("отчёт.docx"));
            Check("…лежит с прошлого запуска — открыт без запроса", Files(900104) == 1 && FileShell.Opened[^1] == earlier,
                () => $"запросов: {Files(900104)}; открыт: {FileShell.Opened[^1]}");

            // 9. программа: скачана, не запущена — показана в Проводнике, сказано почему
            FileShell.ShownInFolder.Clear();
            await board.OpenAttachmentCommand.ExecuteAsync(Item("setup.exe"));
            var setup = CachedFile(7101, 900102);
            Check("«Открыть» программу: не запущена, показана в папке", setup is not null && File.ReadAllBytes(setup).SequenceEqual(exe)
                && !FileShell.Opened.Contains(setup) && FileShell.ShownInFolder.SequenceEqual(new[] { setup })
                && Item("setup.exe").Status.Contains("не открывает"), () => $"{setup}; «{Item("setup.exe").Status}»");

            // 10. файла на сервере нет — одной строкой, ни файла, ни временного; сервер исправился — открывается
            await board.OpenAttachmentCommand.ExecuteAsync(Item("акт.pdf"));
            Check("«Открыть» при 404: «не скачалось: …» одной строкой, ни файла, ни временного",
                Item("акт.pdf").Status.StartsWith("не скачалось: файл не найден (HTTP 404)") && !Item("акт.pdf").Status.Contains('\n')
                && CachedFile(7101, 900103) is null && !TempsIn(cache), () => $"«{Item("акт.pdf").Status}»");
            server.Change(7101, t => t.Files.First(f => f.Id == 900103).Missing = false);
            await board.OpenAttachmentCommand.ExecuteAsync(Item("акт.pdf"));
            Check("…сервер исправился — открывается, строка ошибки ушла", CachedFile(7101, 900103) is { } act && FileShell.Opened[^1] == act
                && File.ReadAllBytes(act).SequenceEqual(pdf) && Item("акт.pdf").Status == "", () => $"«{Item("акт.pdf").Status}»");

            // 11. «Сохранить»: путь — из окна Windows (имя исходное); скачанный — копией; нет — скачивается прямо туда
            FileShell.AskedToSave.Clear();
            FileShell.SaveAnswer = name => Path.Combine(saveDir, name);
            await board.SaveAttachmentCommand.ExecuteAsync(Item("экран.png"));
            Check("«Сохранить» скачанный: побайтно по выбранному пути, без запроса, имя в окне — исходное",
                File.Exists(Path.Combine(saveDir, "экран.png")) && File.ReadAllBytes(Path.Combine(saveDir, "экран.png")).SequenceEqual(png)
                && Files(900101) == 2 && FileShell.AskedToSave.SequenceEqual(new[] { "экран.png" }) && Item("экран.png").Status.StartsWith("сохранено"),
                () => $"запросов: {Files(900101)}; спрошено: {string.Join(", ", FileShell.AskedToSave)}; «{Item("экран.png").Status}»");
            await board.SaveAttachmentCommand.ExecuteAsync(Item("смета.xlsx"));
            Check("«Сохранить» нескачанный: скачан туда, временного рядом нет", File.Exists(Path.Combine(saveDir, "смета.xlsx"))
                && File.ReadAllBytes(Path.Combine(saveDir, "смета.xlsx")).SequenceEqual(xlsx) && Files(900105) == 2 && !TempsIn(saveDir),
                () => $"запросов: {Files(900105)}; «{Item("смета.xlsx").Status}»");
            Marked("…сохранённый помечен «из интернета» — и скачанный туда, и копия", Path.Combine(saveDir, "смета.xlsx"));
            Marked("…копия скачанного — тоже", Path.Combine(saveDir, "экран.png"));
            Check("…копия — не только для чтения: это уже файл пользователя", !File.GetAttributes(Path.Combine(saveDir, "экран.png")).HasFlag(FileAttributes.ReadOnly));
            FileShell.SaveAnswer = _ => null;
            var before = Directory.GetFiles(saveDir).Length;
            await board.SaveAttachmentCommand.ExecuteAsync(Item("пропал.pdf"));
            Check("…отменили окно — ничего и без запроса", Directory.GetFiles(saveDir).Length == before && Files(900106) == 1 && Item("пропал.pdf").Status == "");
            FileShell.SaveAnswer = name => Path.Combine(saveDir, name);
            await board.SaveAttachmentCommand.ExecuteAsync(Item("пропал.pdf"));
            Check("…файла на сервере нет — ошибка, ни файла, ни временного рядом", Item("пропал.pdf").Status.StartsWith("не скачалось: файл не найден")
                && !File.Exists(Path.Combine(saveDir, "пропал.pdf")) && !TempsIn(saveDir), () => $"«{Item("пропал.pdf").Status}»");

            // 12. два щелчка во время скачивания — один запрос; ушли на другую карточку — файл скачан и открыт
            server.Change(7101, t => t.Files.Add(new(900107, "второй.png", fresh)));
            var refreshAt = Stopwatch.GetTimestamp();
            await board.RefreshFromIntraserviceCommand.ExecuteAsync(null);
            Check("⟳ принёс новое вложение", Names().Contains("второй.png"), () => string.Join(", ", Names()));
            WatchItems();
            var second = Item("второй.png");
            Check("…и его размер — сразу, без паузы: карточку уже открыли", await SingleThread.Until(() => second.Size == "20 Б") && Files(900107) == 1
                    && server.WaitedSince(refreshAt, "/api/taskfile/900107") < Pause,
                () => $"«{second.Size}», запросов: {Files(900107)}, через {server.WaitedSince(refreshAt, "/api/taskfile/900107")?.TotalMilliseconds:0} мс");
            var first = board.OpenAttachmentCommand.ExecuteAsync(second);
            var again = board.OpenAttachmentCommand.ExecuteAsync(second);
            board.SelectedTicket = Card("Без вложений");
            await Task.WhenAll(first, again);
            var secondPath = CachedFile(7101, 900107);
            Check("два щелчка во время скачивания — один запрос файла", secondPath is not null && Files(900107) == 2, () => $"запросов: {Files(900107)}");
            Check("…ушли на другую карточку — файл всё равно скачан и открыт", secondPath is not null && FileShell.Opened[^1] == secondPath
                && File.ReadAllBytes(secondPath).SequenceEqual(fresh), () => $"{secondPath}; открыт: {FileShell.Opened[^1]}");

            // 6. тихое чтение карточки при открытии: после паузы, только поля и вложения, раз в CardFresh, по последней из бега
            var old = Card("Старая карточка");
            var opened = Stopwatch.GetTimestamp();
            board.SelectedTicket = old;
            Check("старая карточка: при открытии дочитаны поля и вложения", await SingleThread.Until(() => old.Files is { Count: 1 })
                && old is { Service: "Почта" } && Names().SequenceEqual(new[] { "лог.txt" }), () => $"{old.Service} | {string.Join(", ", Names())}");
            Check("…после паузы, а не сразу", server.WaitedSince(opened, "/api/task/7103?") >= Pause,
                () => $"через {server.WaitedSince(opened, "/api/task/7103?")?.TotalMilliseconds} мс");
            Check("…статус не тронут: его меняют синхронизации", old.ExternalStatus == "Открыта", () => $"{old.ExternalStatus}");
            board.SelectedTicket = Card("Без вложений");
            board.SelectedTicket = old;
            await Task.Delay(Pause + Pause);
            Check("…снова открыли раньше CardFresh — не читаем", server.Count("/api/task/7103?") == 1, () => $"{server.Count("/api/task/7103?")}");
            MainViewModel.CardFresh = TimeSpan.Zero;
            board.SelectedTicket = null;
            board.SelectedTicket = old;
            Check("…срок вышел — читаем снова", await SingleThread.Until(() => server.Count("/api/task/7103?") == 2), () => $"{server.Count("/api/task/7103?")}");
            MainViewModel.CardFresh = freshWas;
            foreach (var title in new[] { "Первая", "Вторая", "Третья" }) board.SelectedTicket = Card(title);   // бег стрелками
            Check("бег по карточкам: читается только последняя", await SingleThread.Until(() => server.Count("/api/task/7107?") == 1)
                && server.Count("/api/task/7105?") == 0 && server.Count("/api/task/7106?") == 0,
                () => $"7105: {server.Count("/api/task/7105?")}, 7106: {server.Count("/api/task/7106?")}, 7107: {server.Count("/api/task/7107?")}");
            // карточка с сервера идёт дольше паузы панели (VPN): чтение синхронизации ещё в пути, когда панель решает, читать ли
            var slowCard = TimeSpan.FromMilliseconds(800);
            server.Change(7108, t => t.CardWait = slowCard);
            var added = board.AddFromCapture("7108", TicketPriority.Mid);
            await Task.Delay(Pause + Pause + Pause + slowCard);
            Check("добавленную читает синхронизация — панель второй раз не читает, даже пока та ещё читает", added.Files is not null
                && server.Count("/api/task/7108?") == 1, () => $"{server.Count("/api/task/7108?")}");
            server.Change(7108, t => t.CardWait = TimeSpan.Zero);
            // тихое чтение не удалось (сеть, 500) — карточка не «свежая»: открыли снова — читаем снова
            server.Quietly(7106, t => t.CardFails = true);
            board.SelectedTicket = Card("Вторая");
            Check("карточка не прочиталась (500) — спрошена", await SingleThread.Until(() => server.Count("/api/task/7106?") == 1),
                () => $"{server.Count("/api/task/7106?")}");
            board.SelectedTicket = Card("Третья");
            board.SelectedTicket = Card("Вторая");
            Check("…открыли снова — читаем снова: неудачное чтение свежей карточку не делает",
                await SingleThread.Until(() => server.Count("/api/task/7106?") == 2), () => $"{server.Count("/api/task/7106?")}");
            server.Quietly(7106, t => t.CardFails = false);
            var nameless = Card("Без имён");
            board.SelectedTicket = nameless;
            Check("вложение без имени — «файл №…»", await SingleThread.Until(() => Names().SequenceEqual(new[] { "файл №900120" })), () => string.Join(", ", Names()));
            var none = Card("Без вложений");
            board.SelectedTicket = none;
            Check("у заявки без вложений раздела нет", await SingleThread.Until(() => none.Files is { Count: 0 }) && !board.HasAttachments
                && board.AttachmentItems.Count == 0, () => string.Join(", ", Names()));

            // размер, недоспрошенный из-за ухода на другую карточку (запрос отменён), спрашивается, когда её откроют снова
            server.Put(7109, "Медленный файл");
            server.Change(7109, t => t.Files.Add(new(900130, "медленный.bin", Bytes(50, 10)) { Wait = TimeSpan.FromSeconds(1) }));
            var slow = board.AddFromCapture("7109", TicketPriority.Mid);
            Check("медленный файл: размер спрошен", await SingleThread.Until(() => Files(900130) == 1), () => $"{Files(900130)}");
            board.SelectedTicket = Card("Без вложений");   // ушли, пока сервер молчит, — запрос размера отменён
            board.SelectedTicket = slow;
            Check("…ушли на другую карточку, пока он шёл, — открыли снова: спрошен снова, размер пришёл",
                await SingleThread.Until(() => board.AttachmentItems.FirstOrDefault()?.Size == "50 Б") && Files(900130) == 2,
                () => $"«{board.AttachmentItems.FirstOrDefault()?.Size}», запросов: {Files(900130)}");
            // скачивание — через временный рядом: пока файл не пришёл целиком, под настоящим именем его нет (иначе оборванный
            // при закрытии программы счёлся бы скачанным)
            var slowOpen = board.OpenAttachmentCommand.ExecuteAsync(board.AttachmentItems.Single());
            Check("…пока качается — под своим именем его нет, есть временный", await SingleThread.Until(() => Files(900130) == 3)
                && CachedFile(7109, 900130) is null && TempsIn(CacheOf(7109)), () => $"{CachedFile(7109, 900130)}");
            await slowOpen;
            Check("…докачался — на своём месте, временного нет", CachedFile(7109, 900130) is not null && !TempsIn(CacheOf(7109)));

            // ревью 5б: занятый антивирусом файл, сбой сети, ответ без длины, невидимые знаки в имени, опасное по мнению Windows
            server.Put(7110, "Разное");
            // имя длиннее места в пути: укороченное кончается на «.exe», хотя исходное — нет
            var cutRoom = KnowledgeExport.MaxPathLength - CacheOf(7110).Length - 1 - "900157".Length - " — ".Length;
            var cutExe = new string('a', cutRoom - 4) + ".exe." + new string('z', 40);
            server.Change(7110, t => t.Files.AddRange(new BoardServer.Attachment[]
            {
                new(900150, "занят.txt", Bytes(30, 11)), new(900151, "сбой.txt", Bytes(40, 12)), new(900152, "без длины.bin", Bytes(2048, 13)) { NoLength = true },
                new(900153, "счёт\u202Efdp.exe", Bytes(60, 14)), new(900154, "чертёж.xyz", Bytes(70, 15)),
                new(900155, "отчёт.url", Bytes(80, 16)), new(900156, "пароль: схема.png", Bytes(90, 17)), new(900157, cutExe, Bytes(50, 18)),
            }));
            board.AddFromCapture("7110", TicketPriority.Mid);
            AttachmentItem ItemOf(int file) => board.AttachmentItems.First(a => a.File.Id == file);
            Check("разное: размеры, кроме ответа без длины", await SingleThread.Until(() => Files(900154) == 1 && ItemOf(900154).Size == "70 Б")
                && ItemOf(900152).Size == "", () => string.Join(", ", board.AttachmentItems.Select(a => $"{a.Name}: «{a.Size}»")));
            WatchItems();
            Check("имя без знаков направления текста: «счёт» + RLO + «fdp.exe» не выглядит как «…exe.pdf»", ItemOf(900153).Name == "счётfdp.exe",
                () => ItemOf(900153).Name);
            KnowledgeExport.WriteRetryDelays = new[] { TimeSpan.Zero, TimeSpan.Zero, TimeSpan.Zero };
            var moveWas = KnowledgeExport.MoveFile;
            var moves = 0;
            KnowledgeExport.MoveFile = (from, to) =>
            {
                if (++moves <= 2) throw new IOException("занят", unchecked((int)0x80070020));   // нарушение совместного доступа
                moveWas(from, to);
            };
            try { await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900150)); }
            finally { KnowledgeExport.MoveFile = moveWas; }
            Check("файл держит антивирус — подмена повторяется, файл открыт", moves == 3 && CachedFile(7110, 900150) is { } busy && FileShell.Opened[^1] == busy,
                () => $"подмен: {moves}; «{ItemOf(900150).Status}»");
            KnowledgeExport.RetryDelays = new[] { TimeSpan.Zero, TimeSpan.Zero, TimeSpan.Zero };
            server.Change(7110, t => t.Files.First(f => f.Id == 900151).Fails = 1);
            await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900151));
            Check("сбой сети (503) — повтор, файл открыт", Files(900151) == 3 && CachedFile(7110, 900151) is { } retried && FileShell.Opened[^1] == retried
                && ItemOf(900151).Status == "" && !TempsIn(CacheOf(7110)), () => $"запросов: {Files(900151)}; «{ItemOf(900151).Status}»");
            await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900152));
            Check("ответ без длины — размер по скачанному", ItemOf(900152).Size == "2 КБ", () => $"«{ItemOf(900152).Size}»");
            await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900153));
            Check("…и такая программа не запускается", CachedFile(7110, 900153) is { } bidi && FileShell.ShownInFolder[^1] == bidi && !FileShell.Opened.Contains(bidi),
                () => $"«{ItemOf(900153).Status}»");
            FileShell.Dangerous = path => path.EndsWith(".xyz");
            try { await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900154)); }
            finally { FileShell.Dangerous = _ => false; }
            Check("опасное по мнению Windows (AssocIsDangerous) — тоже показывается в папке", CachedFile(7110, 900154) is { } risky
                && FileShell.ShownInFolder[^1] == risky && !FileShell.Opened.Contains(risky), () => $"«{ItemOf(900154).Status}»");
            // ярлык Проводник читает, даже просто показывая (значок по сети выдаёт NTLM): в папке — с «_», это просто файл
            await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900155));
            Check("ярлык (.url): в папке временных — «.url_», показан в папке, не открыт", CachedFile(7110, 900155) is { } link && link.EndsWith("отчёт.url_")
                && FileShell.ShownInFolder[^1] == link && !FileShell.Opened.Contains(link), () => $"{CachedFile(7110, 900155)}; «{ItemOf(900155).Status}»");
            await board.SaveAttachmentCommand.ExecuteAsync(ItemOf(900155));
            Check("…сохранённый — тоже «.url_»", File.Exists(Path.Combine(saveDir, "отчёт.url_")) && FileShell.AskedToSave[^1] == "отчёт.url_",
                () => $"«{ItemOf(900155).Status}»");
            // имя в папке временных — по правилу «Сохранить», а не базы: пароль не скрыт, окончание цело — Windows знает, чем открыть
            await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900156));
            Check("«пароль: схема.png» — в папке временных с окончанием .png и открыт", CachedFile(7110, 900156) is { } scheme
                && scheme.EndsWith("пароль схема.png") && FileShell.Opened[^1] == scheme, () => $"{CachedFile(7110, 900156)}; «{ItemOf(900156).Status}»");
            await board.OpenAttachmentCommand.ExecuteAsync(ItemOf(900157));
            Check("имя, укороченное по длине пути до «.exe», — тоже не запускается: решает и имя в папке", CachedFile(7110, 900157) is { } cut
                && cut.EndsWith(".exe") && FileShell.ShownInFolder[^1] == cut && !FileShell.Opened.Contains(cut),
                () => $"{CachedFile(7110, 900157)}; «{ItemOf(900157).Status}»");

            // 13. автообновление: строки списка полей карточки не стирают; у открытой карточки сдвинулся Changed — её вложения в
            // панели свежие (заход — после проверок тихого чтения: он сам меняет статус по строке списка)
            board.SelectedTicket = shot;
            await board.AutoSyncAsync();   // базовая линия переписки
            Check("заход автообновления (строки списка без полей карточки) их не стёр", shot is { Service: "Почта", CreatorPosition: "Бухгалтер" }
                && shot.Files?.Count == 7, () => $"{shot.Service} | {shot.CreatorPosition} | {shot.Files?.Count}");
            server.Change(7101, t => t.Files.Add(new(900108, "третий.png", fresh)));
            await board.AutoSyncAsync();
            Check("заход автообновления: открытая карточка изменилась — новое вложение в панели", await SingleThread.Until(() => Names().Contains("третий.png")),
                () => string.Join(", ", Names()));
            // панель закрыта (выбор остался) — карточку заход не перечитывает: её никто не видит
            board.ClosePanelCommand.Execute(null);
            var cardsClosed = server.Count("/api/task/7101?");
            server.Change(7101, t => t.Files.Add(new(900111, "пятый.png", fresh)));
            await board.AutoSyncAsync();
            await Task.Delay(Pause + Pause);
            Check("панель закрыта — заход автообновления карточку не перечитывает", board.SelectedTicket == shot && !board.IsPanelOpen
                && server.Count("/api/task/7101?") == cardsClosed, () => $"чтений: {server.Count("/api/task/7101?") - cardsClosed}");

            // строки списка (заход) — не чтение карточки: карточку, которую ещё не открывали, при открытии читаем
            board.SelectedTicket = Card("Первая");
            Check("после захода автообновления неоткрытая карточка при открытии читается — строка списка не карточка",
                await SingleThread.Until(() => server.Count("/api/task/7105?") == 1), () => $"{server.Count("/api/task/7105?")}");

            // F5 по открытой карточке, у которой сменились вложения, — карточка читается один раз: новая строка вложений не
            // запускает второе чтение (свежесть ставится раньше, чем список)
            board.SelectedTicket = shot;
            shot.CardReadAt = DateTimeOffset.Now.AddMinutes(-5);
            server.Change(7101, t => t.Files.Add(new(900109, "четвёртый.png", fresh)));
            var cardsAtF5 = server.Count("/api/task/7101?");
            await board.RefreshAllCommand.ExecuteAsync(null);
            Check("F5: новое вложение открытой карточки — в панели, размер спрошен, карточка прочитана один раз",
                await SingleThread.Until(() => Names().Contains("четвёртый.png") && Files(900109) == 1) && server.Count("/api/task/7101?") == cardsAtF5 + 1,
                () => $"чтений карточки: {server.Count("/api/task/7101?") - cardsAtF5}; {string.Join(", ", Names())}");

            // сохранили настройки той же учётной записи (лимит, пороги) — ничего не спрашивается заново
            var (cardsBefore, probesBefore) = (server.Count("/api/task/7101?"), Files(900101));
            board.ApplySettings(new HttpIntraserviceClient(server.Url, "me", "p"));
            await Task.Delay(Pause + Pause);
            Check("сохранили настройки той же учётной записи — ни карточки, ни размеров заново",
                server.Count("/api/task/7101?") == cardsBefore && Files(900101) == probesBefore,
                () => $"карточка: {server.Count("/api/task/7101?") - cardsBefore}; запросов файла: {Files(900101) - probesBefore}");
            // сменили учётную запись — карточка и размеры заново; строка, что качается, остаётся: иначе второй щелчок качал бы второй раз
            var busyRow = Item("смета.xlsx");
            busyRow.IsBusy = true;
            settings.IntraserviceLogin = "me2";
            board.ApplySettings(new HttpIntraserviceClient(server.Url, "me2", "p"));
            Check("сменили учётную запись — карточка и размеры спрошены заново", await SingleThread.Until(() => server.Count("/api/task/7101?") == cardsBefore + 1
                    && Files(900101) == probesBefore + 1 && Item("экран.png").Size == "2,9 КБ"),
                () => $"карточка: {server.Count("/api/task/7101?") - cardsBefore}; размер «{Item("экран.png").Size}», запросов файла: {Files(900101) - probesBefore}");
            Check("…строка, что качается, — та же", ReferenceEquals(Item("смета.xlsx"), busyRow));
            busyRow.IsBusy = false;

            // 12. без API — куда идти настраивать
            var offDir = Path.Combine(data.Path, "off");
            new TicketStore(offDir).Save(new[] { new Ticket { Title = "Без API", IntraserviceId = 7101, Files = new[] { new TicketFile(900101, "экран.png") } } });
            var off = new MainViewModel(new TicketStore(offDir), new AppSettings(), new IntraserviceLinkParser(new AppSettings()), null);
            BoardCheck.WatchBoard(pump, off);
            off.SelectedTicket = off.AllTickets.Single();
            var offItem = off.AttachmentItems.Single();
            await off.OpenAttachmentCommand.ExecuteAsync(offItem);
            Check("без API: вложение видно, «Открыть» говорит, куда идти", offItem.Status.StartsWith("API не настроен"), () => $"«{offItem.Status}»");

            Check("доска менялась только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            Check("сервер не получал неожиданных запросов", server.Unexpected.Count == 0, () => string.Join("\n", server.Unexpected));
        }
        finally
        {
            server.Dispose();
            (MainViewModel.FilesCache, MainViewModel.CardFresh, FileShell.SaveAnswer, KnowledgeExport.RetryDelays, KnowledgeExport.WriteRetryDelays)
                = (cacheWas, freshWas, saveWas, delaysWas, writeDelaysWas);
        }
        }
        checks.AssertAll();
    }
}
