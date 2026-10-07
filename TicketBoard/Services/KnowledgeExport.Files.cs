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
        CancellationToken ct)
    {
        var map = new Dictionary<int, Known>();
        var files = 0;
        foreach (var path in Directory.EnumerateFiles(ticketsDir, "*.md", SearchOption.AllDirectories))
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

    /// <summary>Файл с прошлой выгрузки ещё годится: формат тот же и заявку с тех пор не меняли (дата изменения в свойствах
    /// файла та же, что в списке). Нет свойств, занят, испорчен — не годится, файл перепишется.</summary>
    internal static bool IsCurrent(string path, DateTimeOffset changed)
    {
        var p = ReadProps(path);
        return p is not null
            && int.TryParse(p.GetValueOrDefault("format"), NumberStyles.None, CultureInfo.InvariantCulture, out var format) && format == FormatVersion
            && DateTimeOffset.TryParse(p.GetValueOrDefault("changed"), CultureInfo.InvariantCulture, DateTimeStyles.None, out var was)
            && was.UtcTicks == changed.UtcTicks;
    }

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
            // без свойств (испорчен, не наш) — не трогаем: перепишется вместе с заявкой, если она попадёт в выгрузку
            if (ReadProps(path) is not { } props || !props.ContainsKey("id")) continue;
            var shard = ShardOf(DateTimeOffset.TryParse(props.GetValueOrDefault("created"), CultureInfo.InvariantCulture,
                DateTimeStyles.None, out var created) ? created : null);
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

    /// <summary>Сначала во временный файл, потом подменой: оборванная запись не оставит полфайла. Не вышло (файл занят,
    /// диск полон) — временный убираем, чтобы он не остался лежать среди заметок.</summary>
    private static void WriteAtomic(string path, string text)
    {
        var tmp = path + ".tmp";
        try
        {
            File.WriteAllText(tmp, text, Utf8);
            File.Move(tmp, path, overwrite: true);
        }
        catch { TryDelete(tmp); throw; }
    }

    private static void TryDelete(string path)
    {
        try { File.Delete(path); } catch (IOException) { } catch (UnauthorizedAccessException) { }   // открыт в редакторе — останется
    }

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
    internal static string WriteIndexes(string dir, IEnumerable<string> shards)
    {
        var ticketsDir = Path.Combine(dir, TicketsFolder);
        if (!Directory.Exists(ticketsDir)) return "";
        var error = "";
        void Try(Action write, string what)
        {
            try { write(); }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                if (error.Length == 0) error = $"Оглавление {what} не записано: {ex.Message}";
            }
        }

        var need = new HashSet<string>(shards, StringComparer.OrdinalIgnoreCase);
        var months = Directory.EnumerateDirectories(ticketsDir).ToList();
        foreach (var folder in months)
            if (!File.Exists(Path.Combine(folder, IndexFile))) need.Add(Path.GetFileName(folder));
        // ничего не тронуто и всё оглавлено — не переписываем (прервавшаяся сразу выгрузка не должна плодить файлы)
        if (need.Count == 0 && (months.Count == 0 || File.Exists(Path.Combine(dir, IndexFile)))) return "";
        foreach (var shard in need) Try(() => WriteMonthIndex(ticketsDir, shard), $"{shard}/{IndexFile}");
        Try(() => WriteRootIndex(dir, ticketsDir), IndexFile);
        return error;
    }

    private static IEnumerable<string> TicketFiles(string folder) =>
        Directory.EnumerateFiles(folder, "*.md").Where(p => IdInName.IsMatch(Path.GetFileName(p)));

    /// <summary>Оглавление месяца: заявки по созданию, свежие сверху. Месяц опустел (заявки переложили) — оглавление и папка
    /// убираются.</summary>
    private static void WriteMonthIndex(string ticketsDir, string shard)
    {
        var folder = Path.Combine(ticketsDir, shard);
        if (!Directory.Exists(folder)) return;
        var entries = TicketFiles(folder)
            .Select(path => (Name: Path.GetFileNameWithoutExtension(path), Props: ReadProps(path)))
            .Where(e => e.Props is not null && e.Props.ContainsKey("id"))
            .Select(e => (e.Name, Props: e.Props!))
            .OrderByDescending(e => e.Props.GetValueOrDefault("created") ?? "", StringComparer.Ordinal)
            .ThenBy(e => e.Name, StringComparer.Ordinal)
            .ToList();
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
            var created = DateTimeOffset.TryParse(p.GetValueOrDefault("created"), CultureInfo.InvariantCulture, DateTimeStyles.None, out var c)
                ? " · " + c.ToString("dd.MM.yyyy", CultureInfo.InvariantCulture) : "";
            var status = p.GetValueOrDefault("status") is { Length: > 0 } s ? $" · {s}" : "";
            sb.Append($"- [[{name}]]{created}{status}\n");
        }
        WriteAtomic(indexPath, sb.ToString());
    }

    /// <summary>Общее оглавление: как читать файлы (для агента, который будет разбирать выгрузку в базу знаний) и список
    /// месяцев со ссылками на их оглавления. Число заявок — по именам файлов, содержимое не читается.</summary>
    private static void WriteRootIndex(string dir, string ticketsDir)
    {
        var months = Directory.EnumerateDirectories(ticketsDir)
            .Select(folder => (Name: Path.GetFileName(folder), Count: TicketFiles(folder).Count()))
            .Where(m => m.Count > 0)
            .OrderBy(m => m.Name == NoDateFolder)   // «без даты» — в конец
            .ThenByDescending(m => m.Name, StringComparer.Ordinal)
            .ToList();
        var sb = new StringBuilder("# Заявки из Интрасервиса\n\n");
        sb.Append($"Выгрузка TicketBoard для базы знаний — обновлено {DateTime.Now.ToString("dd.MM.yyyy HH:mm", CultureInfo.InvariantCulture)}, заявок: {months.Sum(m => m.Count)}.\n\n");
        sb.Append($"Заявки лежат в папке `{TicketsFolder}`, по подпапкам — месяцам создания (`{TicketsFolder}/2026-08`): в каждой оглавление `{IndexFile}` и по файлу на заявку. ")
          .Append("В начале файла свойства: номер, статус, даты (создана, изменена, решена), сервис, тип, категории, инициатор, исполнители и их группа, ссылка. ")
          .Append("Дальше описание заявки и переписка по времени — от первой записи к последней; «(внутренний)» — комментарий, которого заявитель не видел, ")
          .Append("«статус «…»» — на этой записи статус поменялся. Решение обычно в последних записях закрытой заявки. ")
          .Append("Телефоны и почта людей в выгрузку не попадают. Файл переписывается при следующей выгрузке, если заявка менялась.\n\n");
        sb.Append("## По месяцам\n\n");
        foreach (var (name, count) in months)
            sb.Append($"- [{name}]({TicketsFolder}/{Uri.EscapeDataString(name)}/{IndexFile}) — заявок: {count}\n");
        WriteAtomic(Path.Combine(dir, IndexFile), sb.ToString());
    }
}
