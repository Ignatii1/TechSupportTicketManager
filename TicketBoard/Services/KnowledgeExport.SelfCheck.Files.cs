using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    /// <summary>Самопроверка вложений в базе (этап 5а): настоящий клиент против поддельного сервера, который отдаёт файлы.
    /// 601 — моя, три вложения (картинка, имя с запрещёнными знаками, крупный — в files-lfs); 602 — чужая; 603 — моя, файла
    /// на сервере нет (потом появляется). Дальше — повтор без изменений, правка заявки, убранное и переименованное вложение,
    /// настройки «всех» и «никаких», сбои посреди файла, «Остановить», длинный путь, просмотр.</summary>
    private static void SelfCheckFiles()
    {
        var checks = new List<string>();
        void Check(bool ok, string what) { if (!ok) checks.Add(what); }
        var server = new FilesServer();
        var (listener, port) = FakeIntraservice.Start(server.Respond, server.File);
        var dir = Path.Combine(Path.GetTempPath(), $"tb-files-{Guid.NewGuid():N}");
        var (lfsFrom, delays, headers, idle) = (LfsFrom, RetryDelays, HttpIntraserviceClient.FileHeadersTimeout, HttpIntraserviceClient.FileIdleTimeout);
        LfsFrom = 1500;   // 9003 — ровно 1500 байт: порог включительно
        RetryDelays = new[] { TimeSpan.Zero, TimeSpan.Zero, TimeSpan.Zero };
        HttpIntraserviceClient.FileIdleTimeout = TimeSpan.FromMilliseconds(300);
        try
        {
            Directory.CreateDirectory(dir);
            File.WriteAllText(Path.Combine(dir, ".gitattributes"), "*.psd binary\n");   // строки пользователя
            server.AttributesHaveRule = () => File.ReadAllText(Path.Combine(dir, ".gitattributes")).Contains(LfsRule);
            server.Vanish = () => File.Delete(Path.Combine(dir, FilesFolder, "2026-09", "614", "9906 — вирус.exe"));
            var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "user", "pass");
            var me = new IntraserviceUser(7, "Я Сам");
            var scope = AttachmentScope.Mine;
            var progress = new ConcurrentQueue<string>();
            ExportResult Export(CancellationToken ct, params int[] ids) => ExportRowsAsync(client, ids.Select(server.Row).ToList(), dir,
                id => $"https://hd/Task/View/{id}", new SyncProgress(progress.Enqueue), ct,
                context: new ExportContext(me, new HashSet<string>(), scope)).GetAwaiter().GetResult();
            ExportResult Run(params int[] ids) => Export(CancellationToken.None, ids);
            string Plain(int ticket, string name) => Path.Combine(dir, FilesFolder, "2026-09", $"{ticket}", name);
            string Large(int ticket, string name) => Path.Combine(dir, LfsFolder, "2026-09", $"{ticket}", name);
            string Md(int ticket) => File.ReadAllText(Directory.GetFiles(Path.Combine(dir, TicketsFolder, "2026-09"), $"{ticket} — *.md").Single(), Encoding.UTF8);
            bool Same(string path, int fileId) => File.Exists(path) && File.ReadAllBytes(path).SequenceEqual(server.Bodies[fileId]);
            bool Temps() => Directory.Exists(Path.Combine(dir, FilesFolder)) && Directory.EnumerateFiles(Path.Combine(dir, FilesFolder), "*.tmp", SearchOption.AllDirectories).Any();

            // 1. первый заход: у моей 601 — три файла побайтно, крупный — в files-lfs, правило LFS записано раньше, чем он
            // запрошен; чужую 602 не качаем; у 603 файла на сервере нет — файл заявки без revision, заявка не «готова»
            var first = Run(601, 602, 603);
            Check(Same(Plain(601, "9001 — ошибка.png"), 9001) && Same(Plain(601, "9002 — Акт сверка.pdf"), 9002)
                && Same(Large(601, "9003 — большой.zip"), 9003) && !File.Exists(Plain(601, "9003 — большой.zip")),
                "601: файлы на местах побайтно");
            Check(server.RuleBeforeLarge == true, $"правило LFS — до крупного файла: {server.RuleBeforeLarge}");
            var attributes = File.ReadAllText(Path.Combine(dir, ".gitattributes"));
            Check(attributes == "*.psd binary\n" + LfsRule + "\n", attributes);
            Check(!server.Asked.Contains(9101) && Md(602).Contains("\n- чужое.docx · не скачано\n") && Md(602).Contains("\nfiles_missing: 1\n")
                && !Md(601).Contains("files_missing"), "602 чужая — не качаем, в свойствах — сколько нет в базе");
            var md601 = Md(601);
            Check(md601.Contains("""

                ## Вложения

                - ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б
                - [[files/2026-09/601/9002 — Акт сверка.pdf]] · 500 Б
                - [[files-lfs/2026-09/601/9003 — большой.zip]] · 1,5 КБ

                ## Переписка
                """.Replace("\r", "")) && md601.Contains("\nfiles: 3\n") && md601.Contains("\nrevision: ") && md601.Contains($"\nformat: {FormatVersion}\n"), md601);
            var md603 = Md(603);
            Check(!md603.Contains("\nrevision: ") && md603.Contains("\n- нет на сервере.txt · не скачалось: файл не найден (HTTP 404)\n"), md603);
            Check(first.Current.SetEquals(new[] { 601, 602 }) && first.Errors.TryGetValue(603, out var e603)
                && e603.StartsWith("не скачалось вложение — файл заявки записан без него\n«нет на сервере.txt»: файл не найден (HTTP 404)") && ClosedExport.OfTicket(e603)
                && first.Failed == 0,
                $"{string.Join(",", first.Current)} {string.Join(" | ", first.Errors)}");
            Check(first.Error.Contains("Вложения: скачано 3 (2,2 КБ), крупных — в files-lfs (Git LFS): 1, не скачано (настройка «Вложения в базу знаний»): 1; "
                + "не скачались 1 — повторная выгрузка попробует их снова: #603 «нет на сервере.txt»: файл не найден (HTTP 404).")
                && first.Error.Contains("Вложения в базе: files — 800 Б, files-lfs — 1,5 КБ."), first.Error);
            Check(progress.Any(line => line.Contains(" · вложения ")), string.Join(" / ", progress));
            Check(!Temps(), "временных не осталось");

            // 2. файл 603 появился на сервере: перечитана только 603 (без revision), её файл скачан; потом — ни одного запроса
            server.Missing.Remove(9201);
            server.Asked.Clear();
            var second = Run(601, 602, 603);
            Check(second.Unchanged == 2 && second.Updated == 1 && server.Asked.SequenceEqual(new[] { 9201 }) && Md(603).Contains("\nrevision: ")
                && second.Current.SetEquals(new[] { 601, 602, 603 }) && File.Exists(Plain(603, "9201 — нет на сервере.txt")), $"второй заход: {string.Join(",", server.Asked)}");
            server.Asked.Clear();
            var third = Run(601, 602, 603);
            Check(third.Unchanged == 3 && server.Asked.IsEmpty && attributes == File.ReadAllText(Path.Combine(dir, ".gitattributes")), "третий — ничего");

            // 3. 601 изменилась, вложения те же — карточка перечитана, файлы не качаются; оборванная закачка старше десяти
            // минут убрана, свежая (может, её ещё пишут) — нет
            server.Touch(601);
            var (staleTemp, freshTemp) = (Plain(601, "9001 — ошибка.png.ab12.tmp"), Plain(601, "9001 — ошибка.png.cd34.tmp"));
            File.WriteAllText(staleTemp, "");
            File.SetLastWriteTimeUtc(staleTemp, DateTime.UtcNow.AddMinutes(-20));
            File.WriteAllText(freshTemp, "");
            var touched = Run(601);
            Check(touched.Updated == 1 && server.Asked.IsEmpty && Md(601).Contains("9003 — большой.zip"), "изменилась — файлы не качаются");
            Check(!File.Exists(staleTemp) && File.Exists(freshTemp), "старая оборванная закачка убрана, свежая — нет");
            File.Delete(freshTemp);

            // 4. вложение 9002 убрали с заявки — его файл удалён; файл пользователя в той же папке цел; 9001 лежит под прежним
            // именем — переименован, а не скачан заново
            Directory.CreateDirectory(Path.GetDirectoryName(Plain(601, "x"))!);
            File.WriteAllText(Plain(601, "мои заметки.txt"), "моё");
            if (File.Exists(Plain(601, "9001 — ошибка.png"))) File.Move(Plain(601, "9001 — ошибка.png"), Plain(601, "9001 — старое имя.png"));
            server.Cards[601] = server.Cards[601] with { Ids = "9001, 9003", Names = "ошибка.png||большой.zip" };
            server.Touch(601);
            Run(601);
            Check(!File.Exists(Plain(601, "9002 — Акт сверка.pdf")) && File.Exists(Plain(601, "мои заметки.txt"))
                && File.Exists(Plain(601, "9001 — ошибка.png")) && !File.Exists(Plain(601, "9001 — старое имя.png"))
                && File.Exists(Large(601, "9003 — большой.zip")) && server.Asked.IsEmpty && !Md(601).Contains("9002"), "убрали и переименовали");
            // карточка не прислала FileIds — ничего не удаляем
            server.Cards[601] = server.Cards[601] with { Ids = null };
            server.Touch(601);
            Run(601);
            Check(File.Exists(Plain(601, "9001 — ошибка.png")) && File.Exists(Large(601, "9003 — большой.zip")), "без FileIds — не удаляем");
            server.Cards[601] = server.Cards[601] with { Ids = "9001, 9003" };
            // лишняя копия того же вложения (руками, прежним именем) — убрана, нынешняя цела
            File.Copy(Plain(601, "9001 — ошибка.png"), Plain(601, "9001 — копия.png"));
            server.Touch(601);
            Run(601);
            Check(!File.Exists(Plain(601, "9001 — копия.png")) && Same(Plain(601, "9001 — ошибка.png"), 9001) && server.Asked.IsEmpty, "лишняя копия");

            // 5. «всех» — чужая скачана; «никаких» — новая не скачана, а лежащий файл — по-прежнему со ссылкой
            // настройку сменили на «всех» — заявка на сервере та же, но вложений в базе не хватает: перечитана и докачана
            scope = AttachmentScope.All;
            server.Asked.Clear();
            Run(602);
            Check(Same(Plain(602, "9101 — чужое.docx"), 9101) && server.Asked.Contains(9101) && !Md(602).Contains("files_missing"),
                "«всех» — скачано без правки заявки");
            scope = AttachmentScope.None;
            server.Touch(601);
            server.Touch(604);
            var none = Run(601, 604);
            Check(!server.Asked.Contains(9301) && Md(604).Contains("\n- журнал.txt · не скачано\n")
                && Md(601).Contains("- ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б\n") && !none.Error.Contains("Вложения в базе"), "«никаких»");
            scope = AttachmentScope.Mine;

            // 6. 503 — повтор и успех; обрыв посреди файла и тишина посреди файла — ошибка, ни файла, ни временного
            server.FailOnce[9301] = 1;
            server.Touch(604);
            Run(604);
            Check(Same(Plain(604, "9301 — журнал.txt"), 9301)
                && server.Asked.Count(id => id == 9301) == 2, $"503 и повтор: {string.Join(",", server.Asked)}");
            // файл вложения удалили с диска (антивирус, руками) — заявка не менялась, но файл вернулся
            File.Delete(Plain(604, "9301 — журнал.txt"));
            server.Asked.Clear();
            var restored = Run(604);
            Check(Same(Plain(604, "9301 — журнал.txt"), 9301) && restored.Updated == 1 && server.Asked.SequenceEqual(new[] { 9301 }),
                $"удалённый файл вернулся: {string.Join(",", server.Asked)}");
            var broken = Run(605, 611);
            var e605 = broken.Errors.GetValueOrDefault(605) ?? "";
            Check(!File.Exists(Plain(605, "9401 — обрыв.bin")) && !File.Exists(Plain(605, "9402 — тишина.bin")) && !Temps()
                && e605.Contains("соединение оборвалось"), $"обрыв: {string.Join(" | ", broken.Errors)}");
            // сбой вложения — беда самой заявки, даже сетевой и 401 (к файлам, может, не пускает вход API): файл заявки лёг,
            // автовыгрузка повторит её несколько заходов и пропустит, а не встанет на ней навсегда; выгрузка не прерывается
            Check(ClosedExport.OfTicket(e605) && broken.Errors.TryGetValue(611, out var e611) && ClosedExport.OfTicket(e611) && e611.Contains("HTTP 401")
                && broken.Complete && Md(611).Contains("\n- закрыт.pdf · не скачалось: неверный логин или пароль (HTTP 401)\n"), $"{e605}\n{broken.Error}");
            var md605 = Md(605);
            Check(md605.Contains("\n- обрыв.bin · не скачалось: соединение оборвалось") && md605.Contains($"\n- тишина.bin · не скачалось: сервер не ответил за {HttpIntraserviceClient.FileIdleTimeout.TotalSeconds:0.##} с: файл перестал приходить на "), md605);

            // 7. «Остановить» посреди файла — ни файла, ни временного, файл заявки не записан
            using (var stop = new CancellationTokenSource())
            {
                server.OnSent = stop.Cancel;
                var stopped = Export(stop.Token, 606);
                server.OnSent = null;
                Check(!stopped.Complete && !stopped.Current.Contains(606) && !File.Exists(Plain(606, "9501 — долгий.bin")) && !Temps()
                    && !Directory.GetFiles(Path.Combine(dir, TicketsFolder, "2026-09"), "606 — *.md").Any(), "остановили посреди файла");
            }

            // 8. путь у предела MAX_PATH — имя укорочено до 25 знаков (расширение цело), файл записан; следующая выгрузка (заявка
            // изменилась) его не качает. Глубина папки — от длины временной: в Windows она длиннее, чем /tmp в Linux, и папка
            // постоянной глубины не оставила бы имени места
            var lfsTail = Path.Combine(LfsFolder, "2026-09", "607");
            var deep = Path.Combine(dir, new string('d', MaxPathLength - 1 - 25 - dir.Length - 2 - lfsTail.Length));
            var deepRun = ExportRowsAsync(client, new[] { server.Row(607) }, deep, id => "", null, CancellationToken.None,
                context: new ExportContext(me, new HashSet<string>(), AttachmentScope.Mine)).GetAwaiter().GetResult();
            var deepFile = Directory.Exists(Path.Combine(deep, FilesFolder)) ? Directory.GetFiles(Path.Combine(deep, FilesFolder), "9601*", SearchOption.AllDirectories).SingleOrDefault() : null;
            Check(deepRun.Current.Contains(607) && deepFile is not null && Path.GetFileName(deepFile) == "9601 — " + new string('я', 14) + ".png"
                && Path.Combine(deep, lfsTail, Path.GetFileName(deepFile)).Length == MaxPathLength, $"длинный путь: {deepFile}");
            // итог без слушателя (автовыгрузка) — без обхода всей папки ради объёма
            Check(!deepRun.Error.Contains("Вложения в базе") && deepRun.Error.Contains("Вложения: скачано 1"), deepRun.Error);
            server.Asked.Clear();
            server.Touch(607);
            ExportRowsAsync(client, new[] { server.Row(607) }, deep, id => "", null, CancellationToken.None,
                context: new ExportContext(me, new HashSet<string>(), AttachmentScope.Mine)).GetAwaiter().GetResult();
            Check(server.Asked.IsEmpty, "длинный путь — не качаем снова");

            // 9. .gitattributes не записать (на его месте папка) — мелкие файлы ложатся, крупный — нет, и итог говорит почему
            var noRule = Path.Combine(dir, "без правила");
            Directory.CreateDirectory(Path.Combine(noRule, ".gitattributes"));
            server.Touch(601);
            server.LargeStalls = true;   // крупный приходит частями: не качаем его до конца, раз положить некуда
            var blocked = ExportRowsAsync(client, new[] { server.Row(601) }, noRule, id => "", null, CancellationToken.None,
                context: new ExportContext(me, new HashSet<string>(), AttachmentScope.Mine)).GetAwaiter().GetResult();
            var blockedMd = File.ReadAllText(Directory.GetFiles(Path.Combine(noRule, TicketsFolder, "2026-09"), "601 — *.md").Single(), Encoding.UTF8);
            Check(File.Exists(Path.Combine(noRule, FilesFolder, "2026-09", "601", "9001 — ошибка.png"))
                && !File.Exists(Path.Combine(noRule, LfsFolder, "2026-09", "601", "9003 — большой.zip"))
                && blockedMd.Contains("\n- большой.zip · не скачалось: крупный файл (от 1,5 КБ): нет правила Git LFS в .gitattributes\n")
                && !blockedMd.Contains(noRule) && blocked.Error.Contains("крупные файлы (от 1,5 КБ) не сохраняются, пока в .gitattributes нет строки"),
                $"{blocked.Error}\n{blockedMd}");
            server.LargeStalls = false;

            // запись вложения не удалась: папка заявки не создаётся (на её месте файл) — беда папки, автовыгрузка пережидает;
            // на месте файла — папка — мешает место самого файла; в файле заявки — без пути на диске
            File.WriteAllText(Path.Combine(dir, FilesFolder, "2026-09", "608"), "не папка");
            Directory.CreateDirectory(Plain(609, "9702 — блок.txt"));
            var writes = Run(608, 609);
            Check(writes.Errors.TryGetValue(608, out var e608) && ClosedExport.OfTicket(e608) && writes.Errors.TryGetValue(609, out var e609)
                && ClosedExport.OfTicket(e609) && !writes.NotWritten.Contains(608) && !writes.Current.Contains(608), string.Join(" | ", writes.Errors));
            Check(Md(608).Contains("\n- папка.txt · не скачалось: файл не записан\n") && !Md(608).Contains(dir) && !Md(609).Contains(dir), Md(608));

            // страница вместо файла: html у картинки и переадресация (страница входа) — не файл; html-вложение — файл
            var pages = Run(610);
            var md610 = Md(610);
            // логин и пароль, которые страница повторила (трассировка прокси), — не в ошибке
            var e610 = pages.Errors.GetValueOrDefault(610) ?? "";
            Check(e610.Contains("страницу вместо файла") && !e610.Contains("dXNlcjpwYXNz"), e610);
            Check(!File.Exists(Plain(610, "9801 — снимок.png")) && File.Exists(Plain(610, "9802 — страница.html")) && !File.Exists(Plain(610, "9803 — вход.png"))
                && md610.Contains("\n- снимок.png · не скачалось: сервер вернул страницу вместо файла\n")
                && md610.Contains("\n- вход.png · не скачалось: сервер переадресовал запрос файла") && !pages.Current.Contains(610), md610);

            // имён нет (не сошлись с номерами) — html не отличить от страницы входа: не берём (страница, сочтённая файлом,
            // осталась бы в базе навсегда); остальное — под номерами
            Run(613);
            Check(!File.Exists(Plain(613, "9811")) && File.Exists(Plain(613, "9812"))
                && Md(613).Contains("\n- файл 9811 · не скачалось: сервер вернул страницу вместо файла\n"), Md(613));

            // вложение записано, а потом пропало (антивирус удаляет по содержимому) — итог называет заявку, она не «готова»
            var vanished = Run(614);
            Check(!File.Exists(Plain(614, "9906 — вирус.exe")) && vanished.Error.Contains("Записано, но уже нет на диске: 1 (#614)")
                && !vanished.Current.Contains(614) && vanished.Errors.TryGetValue(614, out var e614) && e614.Contains("заявки или вложения"), vanished.Error);

            // имя от заявителя с «(HTTP 503)» — не повод счесть сбой вложения сетевым: первая строка ошибки постоянная
            var named = Run(615);
            Check(named.Errors.TryGetValue(615, out var e615) && ClosedExport.OfTicket(e615), e615 ?? "");

            // страница вместо файла оборвалась на середине — всё равно «страница», а не сетевой сбой (его бы повторяли)
            var cutPage = Run(617);
            Check(cutPage.Errors.TryGetValue(617, out var e617) && e617.Contains("страницу вместо файла"), e617 ?? "");

            // файл есть в списке папки, но пропал до того, как его прочли (антивирус между двумя шагами), — качается заново,
            // а не роняет заявку; папку вложений не прочитать — что там, неизвестно: вложения «не скачались» (и у чужой — она
            // перечитается), а уже лежащие файлы целы: убирается только то, чего нет в карточке
            Run(616);
            Check(Same(Plain(616, "9950 — скрин.png"), 9950), "616: скриншот лёг");
            server.Touch(616);
            Directory.CreateDirectory(Path.GetDirectoryName(Plain(618, "x"))!);
            Directory.CreateDirectory(Path.GetDirectoryName(Plain(612, "x"))!);
            var listWas = FileList;
            ExportResult listed;
            try
            {
                FileList = folder => folder.EndsWith(Path.Combine("2026-09", "612")) ? new[] { Path.Combine(folder, "9930 — призрак.txt") }
                    : folder.EndsWith(Path.Combine("2026-09", "616")) || folder.EndsWith(Path.Combine("2026-09", "618")) ? throw new IOException("нет доступа к папке")
                    : listWas(folder);
                listed = Run(612, 616, 618);
            }
            finally { FileList = listWas; }
            Check(listed.Current.Contains(612) && Same(Plain(612, "9930 — призрак.txt"), 9930), $"призрак: {string.Join(" | ", listed.Errors)}");
            Check(Md(616).Contains("\n- скрин.png · не скачалось: папка вложений не читается\n") && !listed.Current.Contains(616)
                && Same(Plain(616, "9950 — скрин.png"), 9950)
                && Md(618).Contains("\n- чужой скрин.png · не скачалось: папка вложений не читается\n") && !listed.Current.Contains(618),
                $"{Md(616)}\n{Md(618)}");

            // подмену временного держит антивирус — повторяем, как WriteAtomic, и файл ложится
            var (moveWas, moves) = (MoveFile, 0);
            ExportResult busy;
            try
            {
                MoveFile = (from, to) =>
                {
                    if (++moves <= 2) throw new IOException("занят", unchecked((int)0x80070020));   // нарушение совместного доступа
                    moveWas(from, to);
                };
                busy = Run(619);
            }
            finally { MoveFile = moveWas; }
            Check(busy.Current.Contains(619) && Same(Plain(619, "9940 — занят.txt"), 9940) && moves == 3, $"подмен: {moves}; {string.Join(" | ", busy.Errors)}");

            // русская страница в windows-1251 — и вместо файла (200), и отказом (403): текст прочитан, клиент не падает
            // исключением; страница, замолчавшая посреди, — всё равно страница, а не «сервер не ответил» (его повторяли бы)
            var pages1251 = Run(620, 621, 623);
            var (e620, e621) = (pages1251.Errors.GetValueOrDefault(620) ?? "", pages1251.Errors.GetValueOrDefault(621) ?? "");
            Check(e620.Contains("страницу вместо файла") && e620.Contains("Доступ запрещён") && e621.Contains("нет доступа (HTTP 403)")
                && e621.Contains("Доступ запрещён"), $"{e620}\n{e621}");
            // кодировки отказа нет совсем — тело не прочитать, но ошибка та же, строкой, а не исключением из клиента
            // и тело её не пропало: показано, как вышло (UTF-8), — сырой ответ пользователю нужнее, чем ничего
            Check(pages1251.Errors.GetValueOrDefault(623) is { } e623 && e623.Contains("нет доступа (HTTP 403):\n"), string.Join(" | ", pages1251.Errors));

            // ответ с незнакомой кодировкой и на успешный запрос (карточка) — читается как UTF-8, а не исключением из клиента; с
            // utf-7 (.NET её знает по имени, но не поддерживает — другое исключение) — так же, и у карточки, и у отказа; карточка
            // в windows-1251 прочитана по ней, а BOM UTF-8 главнее заголовка, который говорит windows-1251
            var oddCard = Run(624, 626, 627, 628, 630);
            Check(new[] { 624, 627, 628, 630 }.All(oddCard.Current.Contains) && oddCard.Failed == 0
                && oddCard.Errors.GetValueOrDefault(626) is { } e626 && e626.Contains("нет доступа (HTTP 403):\n")
                && new[] { 624, 627, 628, 630 }.All(id => Md(id).Contains("\nservice: \"Почта\"\n")), $"{oddCard.FirstError} {oddCard.Error} {string.Join(" | ", oddCard.Errors)}");
            // отказ на карточку с телом больше предела, оборванным за ним, — «нет доступа», а не «сервер недоступен»: тело ошибки
            // читается не дальше ErrorBodyBytes, а не целиком в память
            var bigRefusal = Run(629);
            Check(bigRefusal.Errors.GetValueOrDefault(629) is { } e629 && e629.Contains("нет доступа (HTTP 403)"), string.Join(" | ", bigRefusal.Errors));
            // карточка, оборвавшаяся посреди тела, — «соединение оборвалось», ошибкой, а не исключением из клиента
            var cutCard = Run(631);
            Check(cutCard.Errors.GetValueOrDefault(631) is { } e631 && e631.Contains("соединение оборвалось"), string.Join(" | ", cutCard.Errors));

            // тело ответа, замолчавшее дольше своего срока (FileIdleTimeout), — у отказа (403) и у страницы вместо файла — обрывается
            // на сроке, а вывод тот же, «нет доступа» и «страница», с тем, что успело прийти, а не «сервер не ответил»: сетевой сбой
            // повторяли бы минутами. По одной: поддельный сервер обслуживает соединения по очереди, а замолчавший ответ держит его;
            // спрошено — по разу, без повторов
            server.Asked.Clear();
            var slowRefusal = Run(625);
            var slowPage = Run(622);
            Check(slowRefusal.Errors.GetValueOrDefault(625) is { } e625 && e625.Contains("нет доступа (HTTP 403)") && !e625.Contains("Доступ запрещён")
                && slowPage.Errors.GetValueOrDefault(622) is { } e622 && e622.Contains("страницу вместо файла") && !e622.Contains("Доступ запрещён")
                && !e625.Contains("не ответил") && !e622.Contains("не ответил")
                && server.Asked.Count(id => id == 9965) == 1 && server.Asked.Count(id => id == 9962) == 1,
                $"{string.Join(" | ", slowRefusal.Errors)} {string.Join(" | ", slowPage.Errors)}; спрошено: {string.Join(",", server.Asked)}");

            // 10. просмотр (и буфер, и запрос агенту) — только имена, ни одного запроса файла
            server.Asked.Clear();
            var (preview, previewError) = BuildAsync(client, server.Row(601), "https://hd/Task/View/601", CancellationToken.None).GetAwaiter().GetResult();
            Check(previewError == "" && preview.Contains("\n## Вложения\n\n- ошибка.png\n- большой.zip\n") && server.Asked.IsEmpty, preview);

            // кусок, который долго пишется на диск (сетевой диск, антивирус), — не молчание сервера: пока он пишется, срок на кусок
            // стоит, и файл скачан целиком, хотя первая запись дольше этого срока
            var disk = new SlowDisk(TimeSpan.FromMilliseconds(600));
            var (got, gotLength, slowDiskError) = client.DownloadFileAsync(9968, disk, null, CancellationToken.None).GetAwaiter().GetResult();
            Check(slowDiskError == "" && got == 200_000 && disk.Length == 200_000, $"{got}: {slowDiskError}");

            // этап 5б: длина файла — из Content-Length; размер для панели — без чтения тела: stopAt: 1 кончает чтение на первом куске
            var (probed, probedLength, probeError) = client.DownloadFileAsync(9968, Stream.Null, null, CancellationToken.None, stopAt: 1).GetAwaiter().GetResult();
            Check(gotLength == 200_000 && probeError == "" && probedLength == 200_000 && probed is > 0 and < 200_000,
                $"длина {gotLength}; проба: прочитано {probed}, длина {probedLength}, {probeError}");
            // сервер понимает Range — проба просит один байт и берёт длину из Content-Range; без Range тот же файл — целиком
            var (ranged, rangedLength, rangedError) = client.DownloadFileAsync(9969, Stream.Null, null, CancellationToken.None, stopAt: 1, rangeTo: 0).GetAwaiter().GetResult();
            using var whole = new MemoryStream();
            var (wholeBytes, wholeLength, wholeError) = client.DownloadFileAsync(9969, whole, null, CancellationToken.None).GetAwaiter().GetResult();
            Check(rangedError == "" && ranged == 1 && rangedLength == 5000 && server.RangesServed == 1 && wholeError == "" && wholeBytes == 5000
                && wholeLength == 5000 && whole.Length == 5000, $"проба: {ranged} байт, длина {rangedLength}, «{rangedError}», диапазонов {server.RangesServed}; "
                + $"целиком: {wholeBytes}, длина {wholeLength}, «{wholeError}»");
            // без Content-Length (конец тела — закрытие соединения) — файл целиком, длины нет
            using var unsized = new MemoryStream();
            var (unsizedBytes, unsizedLength, unsizedError) = client.DownloadFileAsync(9970, unsized, null, CancellationToken.None).GetAwaiter().GetResult();
            Check(unsizedError == "" && unsizedBytes == 3000 && unsizedLength is null && unsized.Length == 3000,
                $"{unsizedBytes}, длина {unsizedLength}, «{unsizedError}»");

            // ревью 5б: 206 без общей длины («bytes 0-0/*») — длины нет, а не длина куска (1 байт); пустой файл на Range отвечает
            // 416 с «bytes */0» — это размер 0, а не отказ
            var (unknownBytes, unknownLength, unknownError) = client.DownloadFileAsync(9971, Stream.Null, null, CancellationToken.None, stopAt: 1, rangeTo: 0).GetAwaiter().GetResult();
            var (emptyBytes, emptyLength, emptyError) = client.DownloadFileAsync(9972, Stream.Null, null, CancellationToken.None, stopAt: 1, rangeTo: 0).GetAwaiter().GetResult();
            Check(unknownError == "" && unknownBytes == 1 && unknownLength is null && emptyError == "" && emptyBytes == 0 && emptyLength == 0,
                $"неизвестная длина: {unknownBytes} байт, длина {unknownLength}, «{unknownError}»; пустой: {emptyBytes}, длина {emptyLength}, «{emptyError}»");

            // заголовки не пришли за свой срок — в ошибке он, а не срок на кусок (замолчавшее тело — свой, 605); одним запросом:
            // поддельный сервер, пока молчит, держит соединение, — поэтому последней: запрос, на который клиент уже не ждёт ответа,
            // сервер ещё обслужит, и следующая проверка увидела бы его в Asked
            HttpIntraserviceClient.FileHeadersTimeout = TimeSpan.FromMilliseconds(200);
            var (_, _, silent) = client.DownloadFileAsync(9967, Stream.Null, null, CancellationToken.None).GetAwaiter().GetResult();
            HttpIntraserviceClient.FileHeadersTimeout = headers;
            Check(silent == $"сервер не ответил за {TimeSpan.FromMilliseconds(200).TotalSeconds:0.##} с", silent);
        }
        finally
        {
            (LfsFrom, RetryDelays, HttpIntraserviceClient.FileHeadersTimeout, HttpIntraserviceClient.FileIdleTimeout) = (lfsFrom, delays, headers, idle);
            listener.Stop();
            try { Directory.Delete(dir, recursive: true); } catch (IOException) { }
        }
        Debug.Assert(checks.Count == 0, "вложения:\n" + string.Join("\n", checks));
    }

    /// <summary>Диск, первая запись на который идёт долго, — сетевой диск или антивирус (проверка срока на кусок).</summary>
    private sealed class SlowDisk(TimeSpan first) : MemoryStream
    {
        private bool _waited;

        public override async ValueTask WriteAsync(ReadOnlyMemory<byte> buffer, CancellationToken ct = default)
        {
            if (!_waited) { _waited = true; await Task.Delay(first, ct); }
            await base.WriteAsync(buffer, ct);
        }
    }

    /// <summary>Поддельный сервер с вложениями: карточки (номера и имена файлов), короткая история, сами файлы. Missing — файлы,
    /// которых нет (404), FailOnce — сколько раз ответить 503, Asked — какие файлы спрашивали.</summary>
    private sealed class FilesServer
    {
        public sealed record Card(string Name, string Executors, string? Ids, string Names, string Changed);

        public readonly ConcurrentDictionary<int, Card> Cards = new()
        {
            [601] = new("Скриншот ошибки", "Я Сам", "9001, 9002, 9003", "ошибка.png||Акт: сверка.pdf||большой.zip", "2026-09-05T10:00:00.250000"),
            [602] = new("Чужая заявка", "Петров", "9101", "чужое.docx", "2026-09-06T10:00:00"),
            [603] = new("Битое вложение", "Я Сам", "9201", "нет на сервере.txt", "2026-09-07T10:00:00"),
            [604] = new("Журнал", "Я Сам", "9301", "журнал.txt", "2026-09-08T10:00:00"),
            [605] = new("Сбои посреди файла", "Я Сам", "9401, 9402", "обрыв.bin||тишина.bin", "2026-09-09T10:00:00"),
            [606] = new("Долгий файл", "Я Сам", "9501", "долгий.bin", "2026-09-10T10:00:00"),
            [607] = new("Длинное имя", "Я Сам", "9601", new string('я', 120) + ".png", "2026-09-11T10:00:00"),
            [608] = new("Папка не пишется", "Я Сам", "9701", "папка.txt", "2026-09-12T10:00:00"),
            [609] = new("Место занято", "Я Сам", "9702", "блок.txt", "2026-09-13T10:00:00"),
            [610] = new("Страница вместо файла", "Я Сам", "9801, 9802, 9803", "снимок.png||страница.html||вход.png", "2026-09-14T10:00:00"),
            [611] = new("Файл не пускает", "Я Сам", "9910", "закрыт.pdf", "2026-09-15T10:00:00"),
            [613] = new("Без имён", "Я Сам", "9811, 9812", "страница.html", "2026-09-16T10:00:00"),
            [614] = new("Антивирус", "Я Сам", "9906, 9907", "вирус.exe||после.txt", "2026-09-17T10:00:00"),
            [612] = new("Призрак в папке", "Я Сам", "9930", "призрак.txt", "2026-09-18T10:00:00"),
            [615] = new("Имя со «сбоем»", "Я Сам", "9920", "скан (HTTP 503).pdf", "2026-09-19T10:00:00"),
            [616] = new("Папка не читается", "Я Сам", "9950", "скрин.png", "2026-09-20T10:00:00"),
            [617] = new("Обрыв страницы", "Я Сам", "9813", "обрезанная.png", "2026-09-21T10:00:00"),
            [618] = new("Чужая, папка не читается", "Петров", "9951", "чужой скрин.png", "2026-09-22T10:00:00"),
            [619] = new("Занятый файл", "Я Сам", "9940", "занят.txt", "2026-09-23T10:00:00"),
            [620] = new("Страница 1251", "Я Сам", "9960", "схема.png", "2026-09-24T10:00:00"),
            [621] = new("Отказ 1251", "Я Сам", "9961", "схема.png", "2026-09-25T10:00:00"),
            [622] = new("Страница молчит", "Я Сам", "9962", "схема.png", "2026-09-26T10:00:00"),
            [623] = new("Отказ без кодировки", "Я Сам", "9963", "схема.png", "2026-09-27T10:00:00"),
            [624] = new("Карточка без кодировки", "Я Сам", null, "", "2026-09-28T10:00:00"),
            [625] = new("Отказ молчит", "Я Сам", "9965", "схема.png", "2026-09-29T10:00:00"),
            [626] = new("Отказ в utf-7", "Я Сам", "9966", "схема.png", "2026-09-30T10:00:00"),
            [627] = new("Карточка в utf-7", "Я Сам", null, "", "2026-09-30T11:00:00"),
            [628] = new("Карточка 1251", "Я Сам", null, "", "2026-09-30T12:00:00"),
            [629] = new("Отказ карточки", "Я Сам", null, "", "2026-09-30T13:00:00"),
            [630] = new("Карточка с BOM", "Я Сам", null, "", "2026-09-30T14:00:00"),
            [631] = new("Карточка оборвалась", "Я Сам", null, "", "2026-09-30T15:00:00"),
        };

        public readonly Dictionary<int, byte[]> Bodies = new[] { 9001, 9002, 9003, 9101, 9201, 9301, 9401, 9402, 9501, 9601, 9701, 9702, 9801, 9802, 9803, 9811, 9812, 9813, 9906, 9907, 9930, 9940, 9950, 9951 }
            .ToDictionary(id => id, id => Enumerable.Range(0, id switch { 9001 => 300, 9002 => 500, 9003 => 1500, _ => 600 })
                .Select(i => (byte)((i * 7 + id) % 251)).ToArray());
        public readonly HashSet<int> Missing = new() { 9201 };
        public readonly ConcurrentDictionary<int, int> FailOnce = new();
        public readonly ConcurrentQueue<int> Asked = new();
        public Action? OnSent;
        /// <summary>«&lt;html&gt;&lt;body&gt;Доступ запрещён&lt;/body&gt;&lt;/html&gt;» в windows-1251 — байтами (кодовых страниц самопроверка сама не включает).</summary>
        private static readonly byte[] Page1251 = Encoding.ASCII.GetBytes("<html><body>")
            .Concat(new byte[] { 0xC4, 0xEE, 0xF1, 0xF2, 0xF3, 0xEF, 0x20, 0xE7, 0xE0, 0xEF, 0xF0, 0xE5, 0xF9, 0xB8, 0xED })
            .Concat(Encoding.ASCII.GetBytes("</body></html>")).ToArray();
        /// <summary>Сколько раз сервер ответил на «Range: bytes=0-0» одним байтом (206).</summary>
        public int RangesServed;
        /// <summary>Крупный 9003 приходит частями с паузой: дочитать его до конца — дождаться паузы.</summary>
        public bool LargeStalls;
        /// <summary>Было ли правило LFS в .gitattributes, когда спросили крупный файл (null — не спрашивали).</summary>
        public bool? RuleBeforeLarge;

        public void Touch(int id) => Cards[id] = Cards[id] with { Changed = Parse(Cards[id].Changed).AddMinutes(1).ToString("yyyy-MM-dd'T'HH:mm:ss.ffffff", CultureInfo.InvariantCulture) };

        private static DateTime Parse(string date) => DateTime.Parse(date, CultureInfo.InvariantCulture);

        public IntraserviceFound Row(int id) => new(id, Cards[id].Name, "Выполнена", "Петрова", new DateTimeOffset(2026, 9, 1, 9, 0, 0, TimeSpan.FromHours(3)),
            "описание", Cards[id].Executors, Changed: new DateTimeOffset(Parse(Cards[id].Changed), TimeSpan.FromHours(3)));

        public FakeFile? File(string target)
        {
            // карточка с незнакомой и с неподдерживаемой кодировкой; в UTF-8 с BOM, а в заголовке — windows-1251
            if (target.StartsWith("/api/task/624?") || target.StartsWith("/api/task/627?") || target.StartsWith("/api/task/630?"))
            {
                var json = Encoding.UTF8.GetBytes("{\"Task\":{\"Id\":" + target[10..13] + ",\"Name\":\"Карточка\",\"StatusName\":\"Выполнена\",\"ServiceName\":\"Почта\",\"Type\":\"Инцидент\"}}");
                return target[10..13] switch
                {
                    "624" => new FakeFile(json, Type: "application/json; charset=x-no-such"),
                    "627" => new FakeFile(json, Type: "application/json; charset=utf-7"),
                    _ => new FakeFile(Encoding.UTF8.GetPreamble().Concat(json).ToArray(), Type: "application/json; charset=windows-1251"),
                };
            }
            if (target.StartsWith("/api/task/628?"))   // карточка в windows-1251: сервис «Почта» — её байтами
                return new FakeFile(Encoding.ASCII.GetBytes("{\"Task\":{\"Id\":628,\"Name\":\"Card\",\"StatusName\":\"Done\",\"ServiceName\":\"")
                    .Concat(new byte[] { 0xCF, 0xEE, 0xF7, 0xF2, 0xE0 }).Concat(Encoding.ASCII.GetBytes("\",\"Type\":\"Incident\"}}")).ToArray(),
                    Type: "application/json; charset=windows-1251");
            if (target.StartsWith("/api/task/631?"))   // карточка, оборванная посреди тела
                return new FakeFile(Encoding.UTF8.GetBytes("{\"Task\":{\"Id\":631,\"Name\":\"Карточка\",\"ServiceName\":\"Почта\"}}"), Cut: 20, Type: "application/json");
            if (target.StartsWith("/api/task/629?"))   // отказ на карточку страницей в 100 КБ, оборванной на 70 КБ
                return new FakeFile(Encoding.ASCII.GetBytes("<html><body>" + new string('x', 100_000) + "</body></html>"), Cut: 70_000, Code: 403, Type: "text/html");
            if (!target.StartsWith("/api/taskfile/") || !int.TryParse(target["/api/taskfile/".Length..], out var id)) return null;
            Asked.Enqueue(id);
            if (Missing.Contains(id) || id is 9910 or 9920 || FailOnce.TryGetValue(id, out var fails) && fails > 0) return null;   // ответит Respond
            if (id == 9003) RuleBeforeLarge = AttributesHaveRule?.Invoke();
            if (id == 9907) Vanish?.Invoke();   // первый файл заявки уже лёг — «антивирус» убирает его
            return id switch
            {
                9003 when LargeStalls => new FakeFile(new byte[2000], Cut: 1600, Stall: TimeSpan.FromMilliseconds(600)),
                9401 => new FakeFile(Bodies[id], Cut: 100),
                9801 => new FakeFile(Encoding.UTF8.GetBytes("<html><body><table><tr><td>Authorization</td><td>dXNlcjpwYXNz</td></tr></table></body></html>"),
                    Type: "text/html; charset=utf-8"),
                9802 or 9811 => new FakeFile(Bodies[id], Type: "text/html; charset=utf-8"),
                9813 => new FakeFile(Bodies[id], Cut: 10, Type: "text/html"),
                9960 => new FakeFile(Page1251, Type: "text/html; charset=windows-1251"),
                9961 => new FakeFile(Page1251, Code: 403, Type: "text/html; charset=windows-1251"),
                9963 => new FakeFile(Page1251, Code: 403, Type: "text/html; charset=x-no-such"),
                9965 => new FakeFile(Page1251, Code: 403, Cut: 10, Stall: TimeSpan.FromSeconds(2), Type: "text/html; charset=windows-1251"),
                9967 => new FakeFile(new byte[10], Wait: TimeSpan.FromSeconds(1)),
                9968 => new FakeFile(new byte[200_000]),
                9969 => new FakeFile(Enumerable.Range(0, 5000).Select(i => (byte)i).ToArray(), Ranges: true, RangeServed: () => Interlocked.Increment(ref RangesServed)),
                9970 => new FakeFile(new byte[3000], NoLength: true),
                9971 => new FakeFile(new byte[4000], Ranges: true, UnknownTotal: true),
                9972 => new FakeFile(Array.Empty<byte>(), Ranges: true),
                9966 => new FakeFile(Encoding.ASCII.GetBytes("<html><body>denied</body></html>"), Code: 403, Type: "text/html; charset=utf-7"),
                9962 => new FakeFile(Page1251, Cut: 10, Stall: TimeSpan.FromSeconds(2), Type: "text/html; charset=windows-1251"),
                9803 => new FakeFile(Bodies[id], Location: "/login"),
                9402 => new FakeFile(Bodies[id], Cut: 100, Stall: TimeSpan.FromMilliseconds(600)),
                9501 => new FakeFile(Bodies[id], Cut: 100, Stall: TimeSpan.FromSeconds(1), Sent: OnSent),
                _ => new FakeFile(Bodies[id]),
            };
        }

        /// <summary>Есть ли правило LFS в .gitattributes папки выгрузки — ставит проверка.</summary>
        public Func<bool>? AttributesHaveRule;
        public Action? Vanish;

        public (int Code, string Json) Respond(string target)
        {
            if (target.StartsWith("/api/taskfile/") && int.TryParse(target["/api/taskfile/".Length..], out var fileId))
            {
                if (FailOnce.TryGetValue(fileId, out var fails) && fails > 0)
                {
                    FailOnce[fileId] = fails - 1;
                    return (503, """{"Message":"busy"}""");
                }
                return fileId == 9910 ? (401, """{"Message":"Authorization has been denied for this request."}""") : (404, """{"Message":"not found"}""");
            }
            if (target.StartsWith("/api/task/") && int.TryParse(target["/api/task/".Length..].Split('?')[0], out var id) && Cards.TryGetValue(id, out var c))
                return (200, "{\"Task\":{\"Id\":" + id + ",\"Name\":\"" + c.Name + "\",\"StatusName\":\"Выполнена\",\"ServiceName\":\"Почта\",\"Type\":\"Инцидент\""
                    + (c.Ids is null ? "" : ",\"FileIds\":\"" + c.Ids + "\",\"FileNames\":\"" + c.Names + "\"") + "}}");
            if (target == "/login") return (200, "<html><body>Вход</body></html>");
            if (target.StartsWith("/api/tasklifetime?taskid="))
                return (200, """{"TaskLifetimeList":{"TaskLifetimes":[{"Date":"2026-09-01T10:00:00","Editor":"Петров","StatusId":29,"Comments":"готово","IsPublic":true}],"Statuses":[{"Id":29,"Name":"Выполнена","IsFixed":true}],"Paginator":{"Page":1,"PageCount":1}}}""");
            return (404, """{"Message":"not found"}""");
        }
    }

    /// <summary>Ход выгрузки — сразу в список, без SynchronizationContext (Progress&lt;T&gt; отдавал бы его в пул, позже).</summary>
    private sealed class SyncProgress(Action<string> report) : IProgress<string>
    {
        public void Report(string value) => report(value);
    }
}
