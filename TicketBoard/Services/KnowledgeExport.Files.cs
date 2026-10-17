using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    // ---------- раскладка: папки по месяцам ----------

    private static readonly UTF8Encoding Utf8 = new(encoderShouldEmitUTF8Identifier: false);

    /// <summary>Папка заявок, у которых нет даты создания.</summary>
    public const string NoDateFolder = "без даты";

    private static readonly Regex ShardName = new($@"^(\d{{4}}-\d{{2}}|{Regex.Escape(NoDateFolder)})$", RegexOptions.CultureInvariant);

    /// <summary>Папка в tickets — месяц выгрузки (а не папка пользователя со своими заметками, которую оглавлять и убирать нельзя).</summary>
    internal static bool IsShard(string folder) => ShardName.IsMatch(Path.GetFileName(folder));

    /// <summary>Папка заявки — месяц создания («2026-08»): сотни тысяч файлов в одной папке не поднимут ни Проводник, ни Obsidian.
    /// Месяц — по дате из строки списка, в том поясе, в каком её прислал сервер (так же она записана в файл); даты нет — «без даты».</summary>
    internal static string ShardOf(DateTimeOffset? created) =>
        created is { } c ? c.ToString("yyyy-MM", CultureInfo.InvariantCulture) : NoDateFolder;

    /// <summary>Выгруженное раньше: путь файла от папки tickets (Rel; null — файла нет) и остальные файлы той же заявки, если
    /// их несколько (дубли), — перезапись уберёт лишние.</summary>
    internal readonly record struct Known(string? Rel, string[]? Others)
    {
        public IEnumerable<string> AllPaths()
        {
            if (Rel is not null) yield return Rel;
            foreach (var other in Others ?? Array.Empty<string>()) yield return other;
        }
    }

    /// <summary>Самый длинный путь файла, с которым Windows без «длинных путей» ещё работает (MAX_PATH — 260; запас на
    /// временное «.xxxx.tmp»).</summary>
    private const int MaxPathLength = 250;

    /// <summary>Папка месяца и путь файла заявки от tickets. Имя укорачивается, если путь иначе не влез бы в MAX_PATH (глубокая
    /// папка хранилища): запись упала бы ошибкой, похожей на «диск полон». От заявки и папки зависит только это — при каждой
    /// выгрузке то же самое, иначе заявка переписывалась бы каждый раз.</summary>
    internal static (string Shard, string Rel) PlaceOf(string ticketsDir, IntraserviceFound f)
    {
        var shard = ShardOf(f.Created);
        var fixedPart = ticketsDir.Length + 1 + shard.Length + 1 + f.Id.ToString(CultureInfo.InvariantCulture).Length + " — ".Length + ".md".Length;
        return (shard, Path.Combine(shard, FileName(f.Id, f.Name, Math.Clamp(MaxPathLength - fixedPart, 0, 80))));
    }

    /// <summary>Номер заявки в имени файла: «номер — название.md» или «номер.md».</summary>
    private static readonly Regex IdInName = new(@"^(\d+)( — .*)?\.md$", RegexOptions.CultureInvariant);

    private static bool TryIdOf(string fileName, out int id)
    {
        id = 0;
        return IdInName.Match(fileName) is { Success: true } m
            && int.TryParse(m.Groups[1].Value, NumberStyles.None, CultureInfo.InvariantCulture, out id);
    }

    /// <summary>Что уже лежит в tickets, во всех папках: номер заявки → её файл (файлы). Только по именам, содержимое не
    /// читается: перечитать заголовки сотен тысяч файлов — минуты, а нужны они лишь у заявок, что выгружаются сейчас (IsCurrent).
    /// wanted — нужны лишь эти номера (выгрузка выбранных не копит в памяти имена всех файлов); null — все. ponytail: словарь
    /// держит путь каждого файла, около сотни байт на заявку — на миллион заявок это сотня мегабайт; больше — хранить на диске.</summary>
    internal static Dictionary<int, Known> ScanNames(string ticketsDir, IReadOnlySet<int>? wanted, IProgress<string>? progress,
        CancellationToken ct, IEnumerable<string>? shards = null)
    {
        var map = new Dictionary<int, Known>();
        var files = 0;
        // shards — только эти папки месяцев (и файлы прямо в tickets): для нескольких выбранных заявок обходить все сотни тысяч
        // файлов незачем. Дубль заявки в чужом месяце так не найдётся — его уберёт ближайшая выгрузка всего списка
        var paths = shards is null ? Directory.EnumerateFiles(ticketsDir, "*.md", SearchOption.AllDirectories)
            : shards.Distinct(StringComparer.OrdinalIgnoreCase).Select(s => Path.Combine(ticketsDir, s)).Where(Directory.Exists)
                .SelectMany(folder => Directory.EnumerateFiles(folder, "*.md")).Concat(Directory.EnumerateFiles(ticketsDir, "*.md"));
        foreach (var path in paths)
        {
            if (++files % 5000 == 0)
            {
                ct.ThrowIfCancellationRequested();
                progress?.Report($"Смотрю, что уже выгружено… файлов: {files}");
            }
            if (!TryIdOf(Path.GetFileName(path), out var id) || (wanted is not null && !wanted.Contains(id))) continue;
            var rel = Path.GetRelativePath(ticketsDir, path);
            map[id] = map.TryGetValue(id, out var known)
                ? known with { Others = (known.Others ?? Array.Empty<string>()).Append(rel).ToArray() }
                : new Known(rel, null);
        }
        return map;
    }

    /// <summary>Файл с прошлой выгрузки ещё годится: формат тот же и заявку с тех пор не меняли (версия в свойствах файла —
    /// дата изменения до тика, MachineDate.Revision — та же, что в списке). Нет свойств, занят, испорчен — не годится, файл
    /// перепишется.</summary>
    internal static bool IsCurrent(string path, DateTimeOffset changed) => ReadProps(path) is { } p && IsCurrent(p, changed);

    /// <summary>То же по уже прочитанным свойствам файла.</summary>
    internal static bool IsCurrent(Dictionary<string, string> p, DateTimeOffset changed) =>
        int.TryParse(p.GetValueOrDefault("format"), NumberStyles.None, CultureInfo.InvariantCulture, out var format) && format == FormatVersion
            && p.GetValueOrDefault("revision") == MachineDate.Revision(changed);

    /// <summary>Файлы прежней раскладки (0.10–0.12: все прямо в tickets) — по папкам месяцев, не читая заявки с сервера: месяц
    /// берётся из свойства created самого файла. Файл с тем же именем в месяце уже есть, или файл занят, — остаётся как был: заявка
    /// при выгрузке перепишется на место, а лишнее уберётся. Чужие заметки (имя не с номера) не трогаем. Возвращает, сколько
    /// переложено; о месяцах, куда что-то попало, сообщается через touch (их оглавления перепишутся).</summary>
    internal static int MigrateFlat(string ticketsDir, Action<string?> touch, CancellationToken ct)
    {
        var moved = 0;
        // ToList: во время обхода папка меняется
        foreach (var path in Directory.EnumerateFiles(ticketsDir, "*.md").ToList())
        {
            ct.ThrowIfCancellationRequested();
            var name = Path.GetFileName(path);
            if (!IdInName.IsMatch(name)) continue;
            // не наш (заметка пользователя) или испорчен — не трогаем
            if (ReadProps(path) is not { } props || !IsOurs(props)) continue;
            var shard = ShardOf(MachineDate.TryRead(props.GetValueOrDefault("created"), out var created) ? created : null);
            var target = Path.Combine(ticketsDir, shard, name);
            try
            {
                if (File.Exists(target)) continue;
                Directory.CreateDirectory(Path.GetDirectoryName(target)!);
                File.Move(path, target);
                touch(shard);
                moved++;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* занят — переедет в следующий раз */ }
        }
        return moved;
    }

    // ---------- файлы ----------

    /// <summary>Паузы перед повторами записи, когда файл на миг держит антивирус или индексатор Windows (на сотнях тысяч новых
    /// файлов это случается). Меняется только самопроверкой.</summary>
    internal static TimeSpan[] WriteRetryDelays { get; set; } = { TimeSpan.FromMilliseconds(100), TimeSpan.FromMilliseconds(300), TimeSpan.FromSeconds(1) };

    /// <summary>Файл занят на время: нарушение совместного доступа (32) или блокировки (33) в Windows, отказ в доступе. Другая
    /// ошибка (на месте файла папка, диск полон, путь длинный) от повтора не пройдёт.</summary>
    internal static bool IsBusy(Exception ex) =>
        ex is UnauthorizedAccessException || (ex is IOException io && (io.HResult & 0xFFFF) is 32 or 33);

    /// <summary>Папка выгрузки — в git-репозитории (его отправляет Obsidian Git; она сама корень или его подпапка): временные
    /// файлы записи (*.tmp) — в .gitignore самой папки (действует на неё и вложенные), иначе коммит раз в минуту может поймать
    /// недописанный файл. Только дописываем, и только если строки нет; без памяти — .gitignore могли удалить. Пусто — сделано
    /// или не репозиторий; иначе — почему не вышло.</summary>
    internal static string IgnoreTempsInGit(string dir) =>
        InGitRepository(dir) && AppendOnce(Path.Combine(dir, ".gitignore"), "*.tmp") is { Length: > 0 } why ? $"не удалось дописать *.tmp в {why}" : "";

    /// <summary>Дописать строку в файл (.gitignore, .gitattributes), если такой ещё нет; файла нет — создать. Пусто — сделано;
    /// иначе «путь: причина» (точку ставит тот, кто продолжает).</summary>
    private static string AppendOnce(string path, string line)
    {
        // зовут и заход автовыгрузки (вне очереди писать), и выгрузка в свою очередь: прочитать и дописать — разом, иначе
        // оба увидят «строки нет», и она ляжет дважды
        lock (GitIgnoreLock)
        try
        {
            var text = File.Exists(path) ? File.ReadAllText(path) : "";
            if (!text.Split('\n').Any(l => l.Trim() == line))
                File.AppendAllText(path, (text.Length > 0 && !text.EndsWith('\n') ? "\n" : "") + line + "\n");
            return "";
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            return $"{path}: {ex.Message.TrimEnd('.', ' ')}";
        }
    }

    private static readonly object GitIgnoreLock = new();

    /// <summary>Папка или одна из папок над ней — репозиторий: в ней .git, папка или файл (рабочее дерево, подмодуль).</summary>
    private static bool InGitRepository(string dir)
    {
        for (var d = new DirectoryInfo(dir); d is not null; d = d.Parent)
            if (Directory.Exists(Path.Combine(d.FullName, ".git")) || File.Exists(Path.Combine(d.FullName, ".git")))
                return true;
        return false;
    }

    /// <summary>Сначала во временный файл, потом подменой: оборванная запись не оставит полфайла. Не вышло (файл занят,
    /// диск полон) — временный убираем, чтобы он не остался лежать среди заметок. Занят на время — повторяем после паузы.
    /// Временный записался, а подмена — нет: мешает то, что лежит на месте самого файла (папка, файл только для чтения), —
    /// это PathBlockedException; не записался и временный — беда папки (диск полон, нет доступа). Имя временного — своё у
    /// каждой попытки («X.md.1a2b.tmp»): оставшийся от сбоя или чужой «X.md.tmp» записи не мешает и за беду папки не сойдёт.</summary>
    private static void WriteAtomic(string path, string text) => WriteAtomic(path, w => w.Write(text), Utf8);

    /// <summary>То же потоком: write пишет во временный файл (таблица в десятки мегабайт не собирается строкой в памяти). Его
    /// исключение (не ошибка записи) уходит наверх как есть. Временный пишется один раз — снова, только если занят был он сам;
    /// занят файл на месте (открыт в Excel) — повторяется только подмена. finish — что сделать с записанным временным до
    /// подмены; ему же — когда временный создан (по часам файловой системы, до первой строки).</summary>
    internal static void WriteAtomic(string path, Action<TextWriter> write, Encoding encoding, Action<string, DateTime>? finish = null)
    {
        string tmp;
        for (var attempt = 0; ; attempt++)
        {
            // имя временного своё у каждой попытки: оставшийся от сбоя или чужой «X.md.tmp» не мешает и за беду папки не сойдёт
            tmp = $"{path}.{Random.Shared.Next(0x10000):x4}.tmp";
            try
            {
                DateTime created;
                using (var stream = new FileStream(tmp, FileMode.Create, FileAccess.Write, FileShare.Read))
                {
                    created = File.GetLastWriteTimeUtc(tmp);
                    using var writer = new StreamWriter(stream, encoding);
                    write(writer);
                }
                finish?.Invoke(tmp, created);
                break;
            }
            catch (Exception ex)
            {
                TryDelete(tmp);   // при любой ошибке: временный не должен остаться среди заметок
                if (!IsBusy(ex) || attempt >= WriteRetryDelays.Length) throw;
            }
            Thread.Sleep(WriteRetryDelays[attempt]);
        }
        for (var attempt = 0; ; attempt++)
        {
            try
            {
                File.Move(tmp, path, overwrite: true);
                return;
            }
            catch (Exception ex)
            {
                if (!IsBusy(ex) || attempt >= WriteRetryDelays.Length)
                {
                    TryDelete(tmp);
                    // временный записался, подмена — нет: мешает то, что на месте самого файла
                    if (ex is IOException or UnauthorizedAccessException) throw new PathBlockedException(ex.Message, ex);
                    throw;
                }
            }
            Thread.Sleep(WriteRetryDelays[attempt]);
        }
    }

    /// <summary>Записать файл заявки не дало то, что лежит на его месте (временный рядом записался).</summary>
    internal sealed class PathBlockedException(string message, Exception inner) : IOException(message, inner);

    /// <summary>Наш временный файл оборванной записи — с четырьмя знаками, какие ставит WriteAtomic: файла заявки
    /// («номер — название.md.1a2b.tmp») и оглавления («_index.md.1a2b.tmp»). «X.md.tmp» без них — не наш наверняка: так
    /// сохраняют и редакторы, а заметка «номер — название.md» бывает и своя; синхронизатор, другой редактор — тоже не наши.</summary>
    private static readonly Regex TicketTemp = new(@"^\d+( — .*)?\.md\.[0-9a-f]{4}\.tmp$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase),
        IndexTemp = new(@"^(_index\.md|tickets\.csv|SCHEMA\.md|CLAUDE\.md)\.[0-9a-f]{4}\.tmp$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);


    /// <summary>Старше — точно не пишется сейчас: убираем.</summary>
    private static readonly TimeSpan StaleTemp = TimeSpan.FromMinutes(10);

    /// <summary>Убрать в папке временные файлы оборванных записей (выключили компьютер посреди записи, антивирус не дал
    /// удалить временный): имя у каждой записи своё — сами они не перезапишутся. Каждой выгрузкой — она идёт, только когда
    /// есть что писать (автовыгрузка обычно — когда закрылась моя заявка; пока заход упирается в папку — каждым заходом, но
    /// тогда и сама выгрузка трогает ту же папку), так что перебор папки редкий. Свежие (их, может быть,
    /// пишут прямо сейчас) и чужие *.tmp не трогаем; indexOnly — только оглавления (корень папки выгрузки — это уже
    /// хранилище пользователя). Уборка — не повод срывать выгрузку: никаких исключений наружу.</summary>
    internal static void SweepTemps(string folder, bool indexOnly = false) =>
        SweepTemps(folder, name => IndexTemp.IsMatch(name) || !indexOnly && TicketTemp.IsMatch(name));

    private static void SweepTemps(string folder, Regex ours) => SweepTemps(folder, ours.IsMatch);

    private static void SweepTemps(string folder, Func<string, bool> ours)
    {
        IEnumerable<string> files;
        try { files = Directory.Exists(folder) ? Directory.EnumerateFiles(folder, "*.tmp").ToList() : Array.Empty<string>(); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException) { return; }
        foreach (var file in files)
        {
            if (!ours(Path.GetFileName(file))) continue;
            try
            {
                if (DateTime.UtcNow - File.GetLastWriteTimeUtc(file) > StaleTemp) TryDelete(file);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* этот не прочитался — остальные дальше */ }
        }
    }

    private static void TryDelete(string path)
    {
        try { File.Delete(path); } catch (IOException) { } catch (UnauthorizedAccessException) { }   // открыт в редакторе — останется
    }

    /// <summary>Файл — наш, выгрузки (source: intraservice): его можно переписать, удалить как лишний, назвать файлом заявки.
    /// Заметка пользователя с номером заявки в имени и испорченный файл — не наши.</summary>
    internal static bool IsOurs(Dictionary<string, string>? props) => props?.GetValueOrDefault("source") == SourceName;

    /// <summary>Свойства из начала файла (между «---»): ключ → значение без кавычек. Не наш файл — null.</summary>
    internal static Dictionary<string, string>? ReadProps(string path)
    {
        try
        {
            using var reader = new StreamReader(path, Utf8);
            if (reader.ReadLine() != "---") return null;
            var props = new Dictionary<string, string>();
            for (var i = 0; i < 60 && reader.ReadLine() is { } line; i++)
            {
                if (line == "---") return props;
                var colon = line.IndexOf(':');
                if (colon > 0) props[line[..colon].Trim()] = Unquote(line[(colon + 1)..].Trim());
            }
            return null;
        }
        catch (IOException) { return null; }
        catch (UnauthorizedAccessException) { return null; }
    }

    // ---------- оглавления ----------

    /// <summary>Оглавления: в каждой тронутой за выгрузку папке месяца — свой _index.md (и в тех, где его нет вовсе), в корне —
    /// общий, по месяцам. Месяц переписывается целиком (заголовки его файлов читаются заново), остальные не трогаются:
    /// выгрузка нескольких заявок в большую папку не перечитывает всё. Возвращает первую ошибку записи («» — всё записано):
    /// не вышло с одним месяцем — остальные пишутся.</summary>
    internal static string WriteIndexes(string dir, IEnumerable<string> shards, CancellationToken ct = default, IProgress<string>? progress = null)
    {
        var ticketsDir = Path.Combine(dir, TicketsFolder);
        if (!Directory.Exists(ticketsDir)) return "";
        var error = "";
        void Try(Action write, string failed)
        {
            try { write(); }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                if (error.Length == 0) error = $"{failed}: {ex.Message}";
            }
        }

        var need = new HashSet<string>(shards, StringComparer.OrdinalIgnoreCase);
        var months = new List<string>();
        Try(() =>
        {
            months = Directory.EnumerateDirectories(ticketsDir).Where(IsShard).ToList();
            foreach (var folder in months)
                if (!File.Exists(Path.Combine(folder, IndexFile))) need.Add(Path.GetFileName(folder));
        }, "Оглавление tickets не записано");
        // ничего не тронуто и всё оглавлено — оглавления не переписываем (прервавшаяся сразу выгрузка не должна плодить файлы)
        var indexes = need.Count > 0 || (months.Count > 0 && !File.Exists(Path.Combine(dir, IndexFile)));
        if (indexes)
            foreach (var shard in need) Try(() => WriteMonthIndex(ticketsDir, shard), $"Оглавление {shard}/{IndexFile} не записано");
        // после оглавлений месяцев (опустевший месяц они убирают) — один обход папок на общее оглавление и таблицу
        var onDisk = new List<MonthOnDisk>();
        Try(() => onDisk = MonthsOnDisk(ticketsDir), "Оглавление tickets не записано");
        if (indexes) Try(() => WriteRootIndex(dir, onDisk), $"Оглавление {IndexFile} не записано");
        // таблица — и когда ничего не тронуто: её прошлая запись могла не удаться (файл был открыт в Excel); не отстала — не
        // пишется. Остановили — не собирается: прежняя цела, догонит следующая выгрузка
        string Said(string note) => error.Length == 0 && note.Length > 0 ? error = note : error;
        try
        {
            if (onDisk.Count > 0 || File.Exists(Path.Combine(dir, KnowledgeTable.TableFile)))
                Try(() => Said(KnowledgeTable.Write(dir, need, onDisk, ct, progress)), $"Таблица {KnowledgeTable.TableFile} не записана");
        }
        catch (OperationCanceledException) { /* остановили */ }
        if (onDisk.Count > 0)
            Try(() => Said(KnowledgeTable.WriteGuides(dir)), $"{KnowledgeTable.SchemaFile} или {KnowledgeTable.AgentFile} не записан");
        return error;
    }

    internal static IEnumerable<string> TicketFiles(string folder) => TicketFileInfos(folder).Select(f => f.FullName);

    /// <summary>Файлы заявок папки месяца с датами записи — без чтения содержимого (в Windows даты приходят с самим обходом).</summary>
    internal static IEnumerable<FileInfo> TicketFileInfos(string folder) =>
        new DirectoryInfo(folder).EnumerateFiles("*.md").Where(f => IdInName.IsMatch(f.Name));

    /// <summary>Файлы заявок месяца со свойствами — в порядке оглавления и таблицы: по созданию, свежие сверху, при равном —
    /// по имени. Файл без свойств или без номера в них пропускается.</summary>
    internal static IEnumerable<(string Name, Dictionary<string, string> Props)> MonthEntries(string folder) =>
        TicketFiles(folder)
            .Select(path => (Name: Path.GetFileName(path), Props: ReadProps(path)))
            .Where(e => e.Props is not null && e.Props.ContainsKey("id"))
            .Select(e => (e.Name, Props: e.Props!))
            .OrderByDescending(e => e.Props.GetValueOrDefault("created") ?? "", StringComparer.Ordinal)
            .ThenBy(e => Path.GetFileNameWithoutExtension(e.Name), StringComparer.Ordinal);

    /// <summary>Папка месяца: сколько в ней файлов заявок и когда в ней что-то менялось последний раз (файл или сама папка —
    /// файл добавили, удалили, переименовали).</summary>
    internal readonly record struct MonthOnDisk(string Name, int Count, DateTime Newest);

    /// <summary>Месяцы в tickets в порядке оглавления — одним обходом: и счёт для общего оглавления, и даты для таблицы.</summary>
    internal static List<MonthOnDisk> MonthsOnDisk(string ticketsDir) =>
        InMonthOrder(Directory.EnumerateDirectories(ticketsDir).Where(IsShard).Select(Path.GetFileName).OfType<string>())
            .Select(name =>
            {
                var folder = Path.Combine(ticketsDir, name);
                var (count, newest) = TicketFileInfos(folder)
                    .Aggregate((Count: 0, Newest: Directory.GetLastWriteTimeUtc(folder)),
                        (a, f) => (a.Count + 1, f.LastWriteTimeUtc > a.Newest ? f.LastWriteTimeUtc : a.Newest));
                return new MonthOnDisk(name, count, newest);
            })
            .ToList();

    /// <summary>Порядок месяцев в оглавлении и таблице: свежие сверху, «без даты» — в конце.</summary>
    internal static IEnumerable<string> InMonthOrder(IEnumerable<string> months) =>
        months.OrderBy(m => m == NoDateFolder).ThenByDescending(m => m, StringComparer.Ordinal);

    /// <summary>Оглавление месяца: заявки по созданию, свежие сверху. Месяц опустел (заявки переложили) — оглавление и папка
    /// убираются.</summary>
    private static void WriteMonthIndex(string ticketsDir, string shard)
    {
        var folder = Path.Combine(ticketsDir, shard);
        if (!Directory.Exists(folder)) return;
        var entries = MonthEntries(folder).Select(e => (Name: Path.GetFileNameWithoutExtension(e.Name), e.Props)).ToList();
        var indexPath = Path.Combine(folder, IndexFile);
        if (entries.Count == 0)
        {
            TryDelete(indexPath);
            try { Directory.Delete(folder); } catch (IOException) { } catch (UnauthorizedAccessException) { }   // не пуста — останется
            return;
        }
        var sb = new StringBuilder($"# Заявки: {shard}\n\n");
        sb.Append($"Заявок: {entries.Count} · обновлено {DateTime.Now.ToString("dd.MM.yyyy HH:mm", CultureInfo.InvariantCulture)}\n\n");
        foreach (var (name, p) in entries)
        {
            var created = MachineDate.TryRead(p.GetValueOrDefault("created"), out var c)
                ? " · " + c.ToString("dd.MM.yyyy", CultureInfo.InvariantCulture) : "";
            var status = p.GetValueOrDefault("status") is { Length: > 0 } s ? $" · {Md(s)}" : "";   // и у прежних файлов, где оно как есть
            // имя прежнего файла (до 0.15.0) с обратными кавычками или «%%» — текстом: код внутри [[…]] и комментарий Obsidian;
            // без кавычек — непарная сомкнулась бы со статусом в строке; заявку выгрузят снова — файл переименуется, ссылка вернётся
            var entry = name.Contains('`') || name.Contains("%%") ? Md(name.Replace('`', ' ')) : $"[[{name}]]";
            sb.Append($"- {entry}{created}{status}\n");
        }
        WriteAtomic(indexPath, sb.ToString());
    }

    /// <summary>Общее оглавление: как читать файлы (для агента, который будет разбирать выгрузку в базу знаний) и список
    /// месяцев со ссылками на их оглавления. Число заявок — по именам файлов, содержимое не читается. ponytail: список имён всех
    /// месяцев читается при каждой выгрузке, а оглавление месяца — заголовки всех его файлов; на сотнях тысяч файлов это секунды,
    /// и от размера выгруженного не зависит только тем, что тронуто. Понадобится быстрее — брать счёт нетронутых месяцев из
    /// прежнего оглавления.</summary>
    private static void WriteRootIndex(string dir, IReadOnlyList<MonthOnDisk> onDisk)
    {
        var months = onDisk.Where(m => m.Count > 0).ToList();
        var sb = new StringBuilder("# Заявки из Интрасервиса\n\n");
        sb.Append($"Выгрузка TicketBoard для базы знаний — обновлено {DateTime.Now.ToString("dd.MM.yyyy HH:mm", CultureInfo.InvariantCulture)}, заявок: {months.Sum(m => m.Count)}.\n\n");
        sb.Append($"Заявки лежат в папке `{TicketsFolder}`, по подпапкам — месяцам создания (`{TicketsFolder}/2026-08`): в каждой оглавление `{IndexFile}` и по файлу на заявку. ")
          .Append("В начале файла свойства: номер, статус, даты (создана, изменена, решена, закрыта), сервис, тип, категории, оборудование, инициатор и его организация, ")
          .Append("исполнители и их группа, участники, ваша роль, часы до первой реакции и до решения, сколько раз переоткрывали, сколько комментариев, ")
          .Append($"закрывающий комментарий, ссылка — что значит каждое, сказано в `{KnowledgeTable.SchemaFile}`. ")
          .Append("Дальше описание заявки и переписка по времени — от первой записи к последней; «(внутренний)» — комментарий, которого заявитель не видел, ")
          .Append("«статус «…»» — на этой записи статус поменялся. Решение обычно в последних записях закрытой заявки. ")
          .Append("Текст заявок — только текст: html, команды плагинов Obsidian и «%%» в нём записаны сущностями html — `&lt;` — это «<», `&#36;` — «$», `&#37;` — «%». ")
          .Append("Телефоны и почта из карточек людей в выгрузку не попадают (в тексте заявок и комментариев — как написано); пароль после «пароль:» или «password=» заменён на «[скрыт]». ")
          .Append("Файл переписывается при следующей выгрузке, если заявка менялась. ")
          .Append($"Все заявки строкой на заявку — таблица `{KnowledgeTable.TableFile}`, её колонки — в `{KnowledgeTable.SchemaFile}`.\n\n");
        sb.Append("## По месяцам\n\n");
        foreach (var (name, count, _) in months)
            sb.Append($"- [{name}]({TicketsFolder}/{Uri.EscapeDataString(name)}/{IndexFile}) — заявок: {count}\n");
        WriteAtomic(Path.Combine(dir, IndexFile), sb.ToString());
    }
}
