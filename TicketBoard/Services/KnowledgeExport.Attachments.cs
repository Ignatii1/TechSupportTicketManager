using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Чьи вложения выгрузка кладёт в базу (настройка «Вложения в базу знаний»): моих заявок — по умолчанию («моя» — роль
/// в файле: исполнитель, инициатор, участник), всех выгружаемых, никаких.</summary>
public enum AttachmentScope { Mine, All, None }

/// <summary>Что с вложением после выгрузки: Listed — только известно (просмотр, буфер и запрос агенту ничего не качают),
/// Saved — файл в базе, Skipped — не скачан по настройке, Failed — не скачался.</summary>
public enum AttachmentState { Listed, Saved, Skipped, Failed }

/// <summary>Вложение в файле заявки: Path — путь от корня базы через «/» (у Saved), Size — сколько байт; у Failed — Error (вся
/// ошибка: в итог и журнал, на этом компьютере) и Note (что написать в файл заявки — он уходит в GitHub: без путей на диске и
/// без тела ответа сервера).</summary>
public sealed record Attachment(IntraserviceFile File, AttachmentState State = AttachmentState.Listed, string? Path = null, long? Size = null,
    string Error = "", string Note = "")
{
    /// <summary>Имя для человека: как прислал сервер, одной строкой, пароли скрыты; имени нет — «файл N».</summary>
    public string Title => Secrets.Mask(KnowledgeExport.OneLine(File.Name is { Length: > 0 } name ? name : $"файл {File.Id}"));
}

/// <summary>Правила вложений в базе знаний: имя файла на диске, картинка ли, размер для человека.</summary>
public static class Attachments
{
    /// <summary>Знаки, запрещённые Windows, ломающие ссылки Obsidian (# ^ [ ] |) и код в ссылке (обратная кавычка), — как в
    /// именах файлов заявок (KnowledgeExport.FileName).</summary>
    private static readonly Regex Forbidden = new(@"[\\/:*?""<>|#^\[\]`\x00-\x1F]", RegexOptions.CultureInvariant);

    /// <summary>Имя файла вложения в папке заявки: «номер файла — имя». Номер впереди: имена в одной заявке повторяются (у писем
    /// из Outlook — image001.png в каждом), а по номеру выгрузка узнаёт свой файл. Имя — одной строкой, пароли скрыты, знаки,
    /// запрещённые Windows и ломающие ссылки Obsidian, — пробелы, «%%» (комментарий Obsidian) — «%», точки и пробелы в конце
    /// убраны (Windows их отбрасывает); на «.tmp» и окончания заметок Obsidian — с «_» (Neutralized). Не длиннее max знаков:
    /// укорачивается основа, расширение цело, пара-заменитель не разрезается; не влезает и расширение — один номер. От
    /// номера, имени и max зависит только это: при каждой выгрузке то же имя — файл не качается заново.</summary>
    public static string DiskName(int fileId, string? name, int max)
    {
        var id = fileId.ToString(CultureInfo.InvariantCulture);
        var clean = Regex.Replace(Regex.Replace(Forbidden.Replace(Secrets.Mask(KnowledgeExport.OneLine(name ?? "")), " "), "%{2,}", "%"), @"\s+", " ")
            .Trim().TrimEnd('.', ' ');
        var budget = max - id.Length - Separator.Length;
        if (clean.Length > budget)
        {
            var dot = clean.LastIndexOf('.');
            var ext = dot > 0 && clean.Length - dot <= 10 && !clean[dot..].Contains(' ') ? clean[dot..] : "";
            var stem = Cut(clean[..^ext.Length], budget - ext.Length).TrimEnd('.', ' ');
            clean = ext.Length == 0 || stem.Length > 0 ? stem + ext : "";
        }
        // окончание, у которого файл прячется или исполняется, — с «_» (и укороченное: им тоже может кончиться)
        if (Neutralized.Any(e => clean.EndsWith(e, StringComparison.OrdinalIgnoreCase))) clean = clean.Length < budget ? clean + "_" : clean[..^1] + "_";
        return clean.Length == 0 ? id : id + Separator + clean;
    }

    /// <summary>Окончания, после которых имени вложения дописывается «_»: .tmp — его прячет *.tmp из .gitignore; .md, .markdown,
    /// .canvas, .base — Obsidian открыл бы такой файл как свой: заметка выполняет команды Templater (с триггером на новый файл —
    /// едва файл появится в хранилище) и DataviewJS из своего текста, а текст вложения приходит как есть, мимо Inert. С «_» это
    /// просто файл: Obsidian его не разбирает, агент читает как текст.</summary>
    private static readonly string[] Neutralized = { ".tmp", ".md", ".markdown", ".canvas", ".base" };

    /// <summary>Между номером файла и его именем — как в именах файлов заявок.</summary>
    internal const string Separator = " — ";

    private static string Cut(string s, int n) =>
        n <= 0 ? "" : s.Length <= n ? s : s[..(char.IsHighSurrogate(s[n - 1]) ? n - 1 : n)];

    private static readonly Regex OurName = new($@"^(\d+)(?:{Separator}.*)?$", RegexOptions.CultureInvariant);

    /// <summary>Номер файла вложения по его имени на диске («номер — имя» или «номер»); false — файл не наш (заметка
    /// пользователя рядом, временный файл закачки).</summary>
    internal static bool TryIdOf(string fileName, out int id)
    {
        id = 0;
        return !fileName.EndsWith(".tmp", StringComparison.OrdinalIgnoreCase) && OurName.Match(fileName) is { Success: true } m
            && int.TryParse(m.Groups[1].Value, NumberStyles.None, CultureInfo.InvariantCulture, out id);
    }

    /// <summary>Качать ли вложения заявки при такой настройке и такой моей роли в ней (my_role) — одно правило у выгрузки и у
    /// проверки неизменной заявки «в базе всё, что нужно».</summary>
    public static bool Downloads(AttachmentScope scope, string myRole) => scope switch
    {
        AttachmentScope.All => true,
        AttachmentScope.Mine => myRole.Length > 0,
        _ => false,
    };

    private static readonly string[] Images = { ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp" };

    /// <summary>Картинка — Obsidian встроит её в заметку (![[…]]), остальное — ссылкой.</summary>
    public static bool IsImage(string diskName) => Images.Any(e => diskName.EndsWith(e, StringComparison.OrdinalIgnoreCase));

    private static readonly string[] Units = { "Б", "КБ", "МБ", "ГБ", "ТБ" };

    /// <summary>Размер для человека: «512 Б», «12 КБ», «3,4 МБ», «1,2 ГБ» (по 1024; до десяти — с десятыми, дальше — целое).</summary>
    public static string Size(long bytes)
    {
        double value = bytes;
        var unit = 0;
        for (; value >= 1024 && unit < Units.Length - 1; unit++) value /= 1024;
        var text = (unit == 0 || value >= 10 ? Math.Round(value).ToString("0", CultureInfo.InvariantCulture) : value.ToString("0.#", CultureInfo.InvariantCulture))
            .Replace('.', ',');
        return $"{text} {Units[unit]}";
    }

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var checks = new List<string>();
        void Check(bool ok, string what) { if (!ok) checks.Add(what); }

        Check(DiskName(691552, "ассортимент.xlsx", 200) == "691552 — ассортимент.xlsx", "номер и имя");
        // знаки, запрещённые Windows и ломающие ссылки Obsidian, — пробелы, как в именах файлов заявок
        Check(DiskName(5, "a:b*c?\"d<e>f|g#h^i[j]k`l\\m/n.png", 200) == "5 — a b c d e f g h i j k l m n.png", DiskName(5, "a:b*c?\"d<e>f|g#h^i[j]k`l\\m/n.png", 200));
        Check(DiskName(5, "100%% готово.txt", 200) == "5 — 100% готово.txt", "%%");
        Check(DiskName(5, "отчёт.docx. . ", 200) == "5 — отчёт.docx" && DiskName(5, "a\tb\n c.png", 200) == "5 — a b c.png", "точки и пробелы");
        // *.tmp из .gitignore спрятал бы такое вложение от git
        Check(DiskName(5, "dump.TMP", 200) == "5 — dump.TMP_", "имя на .tmp");
        Check(!DiskName(5, "пароль: 123.txt", 200).Contains("123"), "пароль в имени");
        Check(DiskName(5, "", 200) == "5" && DiskName(5, null, 200) == "5" && DiskName(5, " ?? ", 200) == "5", "пустое имя — номер");
        // длинное — до max знаков, расширение цело, пара-заменитель не разрезана; не влезает и расширение — один номер
        var cut = DiskName(5, new string('я', 300) + ".png", 50);
        Check(cut.Length <= 50 && cut.StartsWith("5 — яя") && cut.EndsWith("я.png") && cut == DiskName(5, new string('я', 300) + ".png", 50), cut);
        Check(DiskName(5, "я😀хвост.png", "5 — я".Length + 1 + ".png".Length) == "5 — я.png", DiskName(5, "я😀хвост.png", 10));
        Check(DiskName(5, "a.png", 2) == "5" && DiskName(5, "длинное имя.png", 8) == "5", DiskName(5, "длинное имя.png", 8));
        Check(TryIdOf("691552 — ассортимент.xlsx", out var ours) && ours == 691552 && TryIdOf("5", out var bare) && bare == 5
            && !TryIdOf("мои заметки.txt", out _) && !TryIdOf("5 — a.png.1a2b.tmp", out _) && !TryIdOf("5-a.png", out _)
            && TryIdOf(DiskName(5, "dump.tmp", 200), out var dump) && dump == 5, "свой файл по имени");
        // укороченное имя тоже не кончается на .tmp: такой файл не узнать своим, и git его не увидит
        var cutTmp = DiskName(5, "backup.tmp (копия из почты)", 14);
        Check(cutTmp.Length <= 14 && !cutTmp.EndsWith(".tmp", StringComparison.OrdinalIgnoreCase) && TryIdOf(cutTmp, out _), cutTmp);
        // заметка Obsidian из вложения — просто файл: её текст пришёл как есть, без Inert, а команды Templater (с триггером на
        // новый файл — едва файл появится) и DataviewJS в заметке выполнились бы; укороченное имя — тоже
        var cutNote = DiskName(5, "длинная заметка про сервер.md", 14);
        Check(DiskName(5, "инструкция.md", 200) == "5 — инструкция.md_" && DiskName(5, "x.MD", 200) == "5 — x.MD_"
            && DiskName(5, "доска.canvas", 200) == "5 — доска.canvas_" && DiskName(5, "b.base", 200) == "5 — b.base_"
            && DiskName(5, "c.markdown", 200) == "5 — c.markdown_" && DiskName(5, "d.md.", 200) == "5 — d.md_"
            && cutNote.Length <= 14 && !cutNote.EndsWith(".md", StringComparison.OrdinalIgnoreCase) && TryIdOf(DiskName(5, "e.md", 200), out _),
            $"{DiskName(5, "инструкция.md", 200)} {cutNote}");
        Check(IsImage("5 — a.PNG") && IsImage("5 — b.jpeg") && IsImage("5 — c.webp") && !IsImage("5 — a.pdf") && !IsImage("5") && !IsImage("5 — png"), "картинки");
        Check(Size(0) == "0 Б" && Size(1023) == "1023 Б" && Size(1536) == "1,5 КБ" && Size(12 * 1024) == "12 КБ"
            && Size(3_565_158) == "3,4 МБ" && Size(1_288_490_189) == "1,2 ГБ" && Size(10_650) == "10 КБ", $"{Size(1536)} {Size(12 * 1024)} {Size(3_565_158)}");
        Debug.Assert(checks.Count == 0, string.Join("\n", checks));
    }
}

public static partial class KnowledgeExport
{
    /// <summary>Папки вложений в корне базы: обычные файлы и крупные — под Git LFS.</summary>
    public const string FilesFolder = "files", LfsFolder = "files-lfs";

    /// <summary>С какого размера файл кладётся в files-lfs: GitHub предупреждает о файлах от 50 МБ и не принимает больше 100 МБ.
    /// Меняется только самопроверкой.</summary>
    internal static long LfsFrom { get; set; } = LargeFile;

    /// <summary>Порог крупного файла по умолчанию — его называет SCHEMA.md.</summary>
    internal const long LargeFile = 50L * 1024 * 1024;

    /// <summary>Строка .gitattributes: всё в files-lfs — через Git LFS (так её пишет git lfs track "files-lfs/**").</summary>
    internal const string LfsRule = "files-lfs/** filter=lfs diff=lfs merge=lfs -text";

    /// <summary>Правило LFS — в .gitattributes папки выгрузки, до первого файла выгрузки: крупный файл без него ушёл бы в git
    /// как есть, и GitHub не принял бы отправку (больше 100 МБ). В папке без git — тоже: станет репозиторием — правило уже на
    /// месте. Пусто — правило есть или дописано; иначе — почему нет (крупные файлы тогда не кладутся).</summary>
    internal static string KeepLargeInLfs(string dir) =>
        AppendOnce(Path.Combine(dir, ".gitattributes"), LfsRule) is { Length: > 0 } why ? $"не удалось дописать правило Git LFS в {why}" : "";

    /// <summary>Временный файл закачки вложения: «номер — имя.xxxx.tmp».</summary>
    private static readonly Regex FileTemp = new(@"^\d+( — .*)?\.[0-9a-f]{4}\.tmp$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);

    /// <summary>Вложения заявки — в files/ (от LfsFrom — в files-lfs/), в папку месяца создания и номера заявки. Файл на месте —
    /// не качается; лежит под другим именем (папку перенесли глубже, сервер переименовал файл) — переименовывается; нет —
    /// качается, если download (настройка и роль), иначе «не скачано». Оборванные закачки старше StaleTemp убираются.</summary>
    private static async Task<IReadOnlyList<Attachment>> SaveFilesAsync(Job job, IntraserviceFound f, string shard, bool download)
    {
        if (f.Extra?.Files is not { Count: > 0 } files) return Array.Empty<Attachment>();
        var (plainDir, largeDir) = FileDirs(job.Root, shard, f.Id);
        var max = MaxPathLength - largeDir.Length - 1;   // имя одно в обеих папках: по более длинной
        var onDisk = new Dictionary<int, List<string>>();
        try
        {
            foreach (var folder in new[] { plainDir, largeDir }.Where(Directory.Exists))
            {
                SweepTemps(folder, FileTemp);
                foreach (var path in Directory.EnumerateFiles(folder))
                    if (Attachments.TryIdOf(Path.GetFileName(path), out var known))
                        (onDisk.TryGetValue(known, out var paths) ? paths : onDisk[known] = new()).Add(path);
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            // что лежит в папке вложений, не узнать (сетевой диск, права): файл заявки пишем, вложения — в следующий раз
            return files.Select(file => job.FileFailed(f.Id, file, $"папка вложений не читается: {ex.Message}", "папка вложений не читается")).ToList();
        }
        var result = new List<Attachment>();
        foreach (var file in files)
        {
            var name = Attachments.DiskName(file.Id, file.Name, max);
            if (onDisk.GetValueOrDefault(file.Id) is [var first, ..] found && Kept(found.FirstOrDefault(p => Path.GetFileName(p) == name) ?? first, name) is var (existing, size))
            {
                Interlocked.Increment(ref job.FilesPresent);
                result.Add(new(file, AttachmentState.Saved, InBase(job.Root, existing), size));
            }
            else if (download) result.Add(await DownloadAsync(job, f.Id, file, plainDir, largeDir, name));
            else
            {
                Interlocked.Increment(ref job.FilesSkipped);
                result.Add(new(file, AttachmentState.Skipped));
            }
        }
        return result;
    }

    /// <summary>Вложения неизменной заявки в базе такие, как записаны в её файле: недостающие (files_missing) не стала бы качать
    /// и нынешняя настройка, а скачанные лежат на месте (их не удалили руками или антивирусом). Нет — заявку перечитать и
    /// докачать: файл на сервере тот же, а в базе его не хватает. Папку не прочитать — не повод перечитывать.</summary>
    private static bool FilesInBase(Job job, IntraserviceFound f, string shard, IReadOnlyDictionary<string, string> props)
    {
        static int Number(IReadOnlyDictionary<string, string> p, string key) =>
            int.TryParse(p.GetValueOrDefault(key), NumberStyles.None, CultureInfo.InvariantCulture, out var n) ? n : 0;
        var missing = Number(props, "files_missing");
        if (missing > 0 && Attachments.Downloads(job.Context.Attachments, props.GetValueOrDefault("my_role") ?? "")) return false;
        var saved = Number(props, "files") - missing;
        if (saved <= 0) return true;
        var (plainDir, largeDir) = FileDirs(job.Root, shard, f.Id);
        try
        {
            return new[] { plainDir, largeDir }.Where(Directory.Exists)
                .Sum(d => Directory.EnumerateFiles(d).Count(p => Attachments.TryIdOf(Path.GetFileName(p), out _))) >= saved;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { return true; }
    }

    /// <summary>Лежащий файл вложения — под нынешним именем (лежит под другим — переименовать; занят — останется как был) и его
    /// размер; пропал между списком папки и этим мигом — default: качать заново.</summary>
    private static (string Path, long Size) Kept(string existing, string name)
    {
        var place = Path.Combine(Path.GetDirectoryName(existing)!, name);
        if (existing != place)
            try { File.Move(existing, place); existing = place; }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* занят — останется под прежним именем */ }
        try { return (existing, new FileInfo(existing).Length); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { return default; }
    }

    private static (string Plain, string Large) FileDirs(string root, string shard, int id)
    {
        var ticket = id.ToString(CultureInfo.InvariantCulture);
        return (Path.Combine(root, FilesFolder, shard, ticket), Path.Combine(root, LfsFolder, shard, ticket));
    }

    private static string InBase(string root, string path) => Path.GetRelativePath(root, path).Replace('\\', '/');

    /// <summary>Один файл: во временный рядом с местом в files/ (сбой сети и 5xx — повтор с начала, RetryDelays), дорос до
    /// LfsFrom — переезжает в files-lfs/ (без правила LFS — не кладётся). Временный не остаётся ни при сбое, ни при «Остановить».</summary>
    private static async Task<Attachment> DownloadAsync(Job job, int ticket, IntraserviceFile file, string plainDir, string largeDir, string name)
    {
        var target = Path.Combine(plainDir, name);
        string? tmp = null;
        // правила LFS нет — крупный не положить: дальше порога его и не качаем
        var limit = job.LfsNote.Length > 0 ? LfsFrom : long.MaxValue;
        // html ждём у html-вложения — и у безымянного: по имени его не отличить (страницу входа всё равно выдаст переадресация)
        var html = file.Name is null || name.EndsWith(".html", StringComparison.OrdinalIgnoreCase) || name.EndsWith(".htm", StringComparison.OrdinalIgnoreCase);
        try
        {
            Directory.CreateDirectory(plainDir);
            var (bytes, error) = await Retrying(async () =>
            {
                if (tmp is not null) TryDelete(tmp);
                tmp = $"{target}.{Random.Shared.Next(0x10000):x4}.tmp";
                long seen = 0;
                await using var stream = new FileStream(tmp, FileMode.CreateNew, FileAccess.Write, FileShare.None);
                return await job.Client.DownloadFileAsync(file.Id, stream, n => { job.Received(n - seen); seen = n; }, job.Abort.Token, limit, html);
            }, r => r.Error, retry: true, job.Abort.Token);
            if (error.Length > 0) return job.FileFailed(ticket, file, error, HttpIntraserviceClient.Headline(error));
            var large = bytes >= LfsFrom;
            if (large && job.LfsNote.Length > 0)
                return job.FileFailed(ticket, file, $"крупный файл (от {Attachments.Size(LfsFrom)}) не сохранён: {job.LfsNote}",
                    $"крупный файл (от {Attachments.Size(LfsFrom)}): нет правила Git LFS в .gitattributes");
            var place = large ? Path.Combine(largeDir, name) : target;
            if (large) Directory.CreateDirectory(largeDir);
            // временный только что закрыт — антивирус или индексатор может держать его миг: подмену повторяем, как WriteAtomic
            for (var attempt = 0; ; attempt++)
                try { File.Move(tmp!, place, overwrite: true); break; }
                catch (Exception ex) when (IsBusy(ex) && attempt < WriteRetryDelays.Length) { await Task.Delay(WriteRetryDelays[attempt], job.Abort.Token); }
            tmp = null;
            job.FileSaved(bytes, large);
            job.Written.Enqueue((ticket, place, job.Now));   // на месте ли (антивирус удаляет через миг) — проверит CheckWritten
            return new(file, AttachmentState.Saved, InBase(job.Root, place), bytes);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            return job.FileFailed(ticket, file, $"файл не записан: {ex.Message}", "файл не записан");
        }
        finally
        {
            if (tmp is not null) TryDelete(tmp);
        }
    }

    /// <summary>Убрать файлы вложений, которых в карточке больше нет, и лишние копии того же номера — только наши (номер
    /// впереди); опустевшую папку заявки — тоже. Карточка не прислала FileIds — не трогаем ничего.</summary>
    private static void PruneFiles(Job job, IntraserviceFound f, string shard, IReadOnlyList<Attachment> kept)
    {
        if (f.Extra?.Files is null) return;
        var keep = kept.Where(a => a.Path is not null).Select(a => Path.GetFullPath(Path.Combine(job.Root, a.Path!)))
            .ToHashSet(StringComparer.OrdinalIgnoreCase);
        var (plainDir, largeDir) = FileDirs(job.Root, shard, f.Id);
        foreach (var folder in new[] { plainDir, largeDir }.Where(Directory.Exists))
            try
            {
                foreach (var path in Directory.EnumerateFiles(folder).ToList())
                    if (Attachments.TryIdOf(Path.GetFileName(path), out _) && !keep.Contains(Path.GetFullPath(path))) TryDelete(path);
                if (!Directory.EnumerateFileSystemEntries(folder).Any()) Directory.Delete(folder);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* не вышло — уберётся в следующий раз */ }
    }

    /// <summary>Строка итога о вложениях этой выгрузки; ничего с ними не было — пусто.</summary>
    private static string FilesNote(Job job)
    {
        var parts = new List<string>();
        if (job.FilesSaved > 0) parts.Add($"скачано {job.FilesSaved} ({Attachments.Size(job.FilesBytes)})");
        if (job.FilesPresent > 0) parts.Add($"уже были {job.FilesPresent}");
        if (job.FilesLarge > 0) parts.Add($"крупных — в {LfsFolder} (Git LFS): {job.FilesLarge}");
        if (job.FilesSkipped > 0) parts.Add($"не скачано (настройка «Вложения в базу знаний»): {job.FilesSkipped}");
        var failed = job.FilesFailed > 0 ? $"не скачались {job.FilesFailed} — повторная выгрузка попробует их снова: {job.FirstFileError}" : "";
        if (parts.Count == 0 && failed.Length == 0) return "";
        return "Вложения: " + string.Join(", ", parts) + (parts.Count > 0 && failed.Length > 0 ? "; " : "") + failed + ".";
    }

    /// <summary>Сколько занимают вложения в базе целиком — когда эта выгрузка что-то скачала; не прочитать папку — пусто.</summary>
    private static string VolumeNote(string dir)
    {
        try
        {
            static long SizeOf(string folder) => Directory.Exists(folder)
                ? Directory.EnumerateFiles(folder, "*", SearchOption.AllDirectories).Sum(p => new FileInfo(p).Length) : 0;
            return $"Вложения в базе: {FilesFolder} — {Attachments.Size(SizeOf(Path.Combine(dir, FilesFolder)))}, "
                + $"{LfsFolder} — {Attachments.Size(SizeOf(Path.Combine(dir, LfsFolder)))}.";
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { return ""; }
    }
}
