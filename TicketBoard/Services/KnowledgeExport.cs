using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Итог выгрузки: под отбор попало Found; записано новых Created, переписано изменившихся Updated, не менялись
/// Unchanged (их переписку не перечитывали), не прочитались Failed (с первой ошибкой). Error не пуст — выгрузка не
/// состоялась, её остановили или есть что сказать о списке; что успели — сохранено.</summary>
public sealed record ExportResult(int Found, int Created, int Updated, int Unchanged, int Failed, string FirstError, string Error);

/// <summary>Заявки для базы знаний: по отбору (TaskQuery) — список с сервера, у каждой заявки — карточка (сервис, тип: в
/// списке их нет) и вся переписка (все страницы), и всё это — Markdown. Одним куском (BuildAsync) — для просмотра и
/// буфера обмена; файлами (RunAsync, ExportRowsAsync), по одному на заявку, в папку tickets, плюс оглавление _index.md.
/// Формат — для агентов и Obsidian: свойства (YAML) в начале файла, описание, переписка по времени — от первой записи к
/// последней. Повторная выгрузка перечитывает только изменившиеся заявки (по Changed). В Интрасервис ничего не пишет;
/// телефоны и почта людей в файлы не попадают.</summary>
public static partial class KnowledgeExport
{
    /// <summary>Версия формата файла. Поменялся формат — увеличить: следующая выгрузка перепишет все файлы.
    /// 2 — сервис, тип, категории и группа из карточки заявки (0.11.0).</summary>
    public const int FormatVersion = 2;
    public const string TicketsFolder = "tickets";
    public const string IndexFile = "_index.md";

    /// <summary>ponytail: не больше стольких заявок за выгрузку (10 страниц списка по 200, по запросу переписки на
    /// каждую — минуты). Понадобится больше — поднять и показывать оставшееся время.</summary>
    public const int MaxLimit = 2000;

    /// <summary>Не больше стольких страниц переписки одной заявки (по 50 записей) — у живых заявок столько не бывает.</summary>
    private const int MaxLifetimePages = 20;

    private static readonly UTF8Encoding Utf8 = new(encoderShouldEmitUTF8Identifier: false);

    /// <summary>Что не так с числом заявок — текстом для окна; null — всё в порядке.</summary>
    public static string? InvalidLimit(int limit) =>
        limit is < 1 or > MaxLimit ? $"«Не больше, заявок» — от 1 до {MaxLimit}" : null;

    /// <summary>Выгрузка по отбору: список (не больше limit заявок, свежие по изменению первыми) и файлы по нему. Условия
    /// по дате сервер, возможно, не применит (формат даты в запросе на живом сервере не проверен) — страхуемся: список
    /// сортирован по изменению, и ниже границы «изменена с» он обрывается; строки вне периода создания и изменения
    /// пропускаются, о чём сказано в итоге.</summary>
    public static async Task<ExportResult> RunAsync(HttpIntraserviceClient client, TaskQuery query, int limit, string dir,
        Func<int, string> ticketUrl, IProgress<string>? progress, CancellationToken ct)
    {
        static ExportResult Fail(string error) => new(0, 0, 0, 0, 0, "", error);
        if (InvalidLimit(limit) is { } invalid) return Fail(invalid);
        try
        {
            // страниц — на одну больше, чем нужно на MaxLimit: сдвинувшийся список повторяет строки, а повтор не в счёт
            var (listed, _, _, listError) = await AutoSyncRules.ReadAllPagesAsync(page =>
                {
                    progress?.Report($"Читаю список заявок… страница {page}");
                    return client.GetTasksAsync(query, page, ct, caller: nameof(KnowledgeExport));
                }, HttpIntraserviceClient.ExecutorPageSize, MaxLimit / HttpIntraserviceClient.ExecutorPageSize + 1,
                stopAt: f => query.Changed.From is { } from && f.Changed is { } changed && changed.DateTime < from.AddDays(-1), maxRows: limit);
            var rows = listed.Where(f => !query.Outside(f)).ToList();
            var notes = new List<string>();
            if (listError.Length > 0) notes.Add($"Список пришёл не целиком: {listError}");
            if (rows.Count < listed.Count)
                notes.Add($"Сервер вернул заявки вне выбранного периода ({listed.Count - rows.Count}) — они пропущены: условие по дате он, похоже, не применил");
            if (rows.Count == 0) return listError.Length > 0 ? Fail(listError) : new(0, 0, 0, 0, 0, "", string.Join("\n", notes));
            return await ExportRowsAsync(client, rows, dir, ticketUrl, progress, ct, notes);
        }
        catch (OperationCanceledException) { return Fail("Остановлено, файлы не тронуты."); }   // ещё до заявок — писать нечего
    }

    /// <summary>Заявка Markdown-ом: карточка (сервис, тип, категории, группа — в строках списка их нет) и вся переписка,
    /// по формату файла выгрузки. Для просмотра, буфера обмена и файлов. Не прочиталась карточка или переписка — пустой
    /// текст и причина: половина заявки хуже, чем никакой (файл потом сочли бы неизменным).</summary>
    public static async Task<(string Text, string Error)> BuildAsync(HttpIntraserviceClient client, IntraserviceFound row,
        string url, CancellationToken ct)
    {
        // ponytail: карточка — запросом на каждую заявку: в строках списка живой сервер не присылает сервис и тип. Может,
        // отдал бы их fields= у списка — не проверено на живом сервере.
        var details = await client.GetTaskAsync(row.Id, ct);
        if (details.Task is not { } task) return ("", details.Error);
        var (events, error) = await ReadLifetimeAsync(client, row.Id, ct);
        return error.Length > 0 ? ("", error) : (Format(WithDetails(row, task), events, url, DateTimeOffset.Now), "");
    }

    /// <summary>Файлы по готовому списку заявок (из RunAsync или выбранные в окне): по 4 разом; не менявшиеся с прошлой
    /// выгрузки пропускаются без запросов. notes — что сказать в итоге о списке.</summary>
    public static async Task<ExportResult> ExportRowsAsync(HttpIntraserviceClient client, IReadOnlyList<IntraserviceFound> rows,
        string dir, Func<int, string> ticketUrl, IProgress<string>? progress, CancellationToken ct, IReadOnlyList<string>? notes = null)
    {
        var ticketsDir = Path.Combine(dir, TicketsFolder);
        Directory.CreateDirectory(ticketsDir);
        var existing = ReadExisting(ticketsDir);
        int created = 0, updated = 0, unchanged = 0, failed = 0, done = 0;
        var firstError = "";
        void Failed(int id, string error)
        {
            if (Interlocked.Increment(ref failed) == 1) firstError = $"#{id}: {error}";
        }
        var stopped = false;
        using var gate = new SemaphoreSlim(4);   // по 4 запроса разом, как у F5: сервер общий
        try
        {
            await Task.WhenAll(rows.Select(async f =>
            {
                await gate.WaitAsync(ct);
                try
                {
                    var name = FileName(f.Id, f.Name);
                    var old = existing.GetValueOrDefault(f.Id);
                    // не менялась с прошлой выгрузки (и формат тот же, и название, и файл один) — переписку не перечитываем
                    if (old.Paths is [var only] && old.Format == FormatVersion && f.Changed is { } changed && old.Changed == changed
                        && Path.GetFileName(only) == name)
                    {
                        Interlocked.Increment(ref unchanged);
                        return;
                    }
                    var (text, error) = await BuildAsync(client, f, ticketUrl(f.Id), ct);
                    if (error.Length > 0) { Failed(f.Id, error); return; }
                    var path = Path.Combine(ticketsDir, name);
                    try
                    {
                        WriteAtomic(path, text);
                        // переименовали заявку (или остался дубль) — прежние файлы с этим номером больше не нужны
                        foreach (var stale in old.Paths ?? new())
                            if (!string.Equals(stale, path, StringComparison.OrdinalIgnoreCase)) TryDelete(stale);
                    }
                    catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                    {
                        Failed(f.Id, $"файл не записан: {ex.Message}");   // занят редактором, антивирусом — в следующий раз
                        return;
                    }
                    if (old.Paths is null) Interlocked.Increment(ref created);
                    else Interlocked.Increment(ref updated);
                }
                finally
                {
                    gate.Release();
                    progress?.Report($"Переписка: {Interlocked.Increment(ref done)} из {rows.Count}");
                }
            }));
        }
        catch (OperationCanceledException) { stopped = true; }

        var indexError = "";
        try { WriteIndex(dir); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            indexError = $"Оглавление {IndexFile} не записано: {ex.Message}";
        }
        var all = new List<string> { stopped ? "Остановлено — что успели, сохранено." : "" };
        all.AddRange(notes ?? Array.Empty<string>());
        all.Add(indexError);
        return new(rows.Count, created, updated, unchanged, failed, firstError, string.Join("\n", all.Where(n => n.Length > 0)));
    }

    /// <summary>Вся переписка заявки — страница за страницей. Сервер присылает Paginator — верим ему: «страниц больше нет» —
    /// стоп (иначе на заявке ровно в 50 записей просили бы несуществующую страницу). Не присылает — конец по неполной
    /// странице; не понимающий page сервер отдал бы ту же страницу снова — новых записей нет, стоп.</summary>
    internal static async Task<(List<IntraserviceEvent> Events, string Error)> ReadLifetimeAsync(HttpIntraserviceClient client,
        int id, CancellationToken ct)
    {
        var all = new List<IntraserviceEvent>();
        var keys = new HashSet<(DateTimeOffset?, string, string, string?)>();
        for (var page = 1; page <= MaxLifetimePages; page++)
        {
            var r = await client.GetLifetimePageAsync(id, page, ct);
            if (r.Error.Length > 0) return (all, r.Error);
            var added = 0;
            foreach (var e in r.Events)
                if (keys.Add((e.Date, e.Author, e.Status, e.Comment))) { all.Add(e); added++; }
            if (added == 0 || (r.Paged ? !r.HasMore : r.Events.Count < HttpIntraserviceClient.LifetimePageSize)) break;
        }
        return (all, "");
    }

    /// <summary>Строка списка, дополненная карточкой заявки: сервис, тип, категории, дата решения и группа — из карточки,
    /// а поля, которого карточка не прислала, — из строки. Остальное (название, статус, даты, люди, описание) — из строки:
    /// по её Changed выгрузка узнаёт неизменные заявки, а по названию — имя файла.</summary>
    internal static IntraserviceFound WithDetails(IntraserviceFound f, IntraserviceTask task)
    {
        var (card, row) = (task.Extra, f.Extra);
        return f with
        {
            ExecutorGroup = task.ExecutorGroup ?? f.ExecutorGroup,
            Extra = new(card?.Service ?? row?.Service, card?.Type ?? row?.Type, card?.Categories ?? row?.Categories,
                card?.Resolved ?? row?.Resolved),
        };
    }

    // ---------- файл заявки ----------

    /// <summary>Имя файла: «номер — название.md». Из названия убраны знаки, запрещённые Windows, и те, что ломают ссылки
    /// Obsidian (# ^ [ ] |); длинное — обрезано. По номеру в начале имени заявка находится при следующей выгрузке.</summary>
    internal static string FileName(int id, string title)
    {
        var clean = Regex.Replace(Regex.Replace(title, @"[\\/:*?""<>|#^\[\]\x00-\x1F]", " "), @"\s+", " ").Trim().TrimEnd('.', ' ');
        if (clean.Length > 80) clean = clean[..80].TrimEnd('.', ' ');
        return clean.Length == 0 ? $"{id}.md" : $"{id} — {clean}.md";
    }

    /// <summary>Заявка файлом Markdown: свойства, шапка, описание, переписка по времени.</summary>
    internal static string Format(IntraserviceFound f, IReadOnlyList<IntraserviceEvent> events, string url, DateTimeOffset exported)
    {
        var x = f.Extra;
        var sb = new StringBuilder("---\n");
        sb.Append($"id: {f.Id.ToString(CultureInfo.InvariantCulture)}\n");
        Prop(sb, "title", f.Name);
        Prop(sb, "status", f.Status);
        DateProp(sb, "created", f.Created);
        DateProp(sb, "changed", f.Changed);
        DateProp(sb, "resolved", x?.Resolved);
        Prop(sb, "service", x?.Service);
        Prop(sb, "type", x?.Type);
        ListProp(sb, "categories", x?.Categories);
        Prop(sb, "creator", f.Creator);
        ListProp(sb, "executors", f.Executors);
        Prop(sb, "group", f.ExecutorGroup);
        Prop(sb, "url", url);
        sb.Append("source: intraservice\n");
        sb.Append($"format: {FormatVersion}\n");
        DateProp(sb, "exported", exported);
        sb.Append("---\n\n");

        sb.Append($"# {f.Id} · {Tags(OneLine(f.Name))}\n\n");
        Line(sb, ("Статус", f.Status), ("создана", Show(f.Created)), ("изменена", Show(f.Changed)), ("решена", Show(x?.Resolved)));
        Line(sb, ("Инициатор", f.Creator), ("исполнители", f.Executors), ("группа", f.ExecutorGroup));
        Line(sb, ("Сервис", x?.Service), ("тип", x?.Type), ("категории", x?.Categories));
        Line(sb, ("Ссылка", url));

        sb.Append("\n## Описание\n\n");
        sb.Append(string.IsNullOrWhiteSpace(f.Description) ? "_Описания нет._\n" : Escape(f.Description.Trim()) + "\n");

        sb.Append("\n## Переписка\n");
        var written = 0;
        string? previous = null;
        // страницы приходят свежими сверху: разворачиваем, чтобы при равных датах осталась очерёдность записей
        foreach (var e in Enumerable.Reverse(events).OrderBy(e => e.Date ?? DateTimeOffset.MinValue))
        {
            // статус меняется на этой записи — пишем его; просто смена полей без комментария — шум, пропускаем
            var status = e.Status.Length > 0 && e.Status != previous ? e.Status : null;
            if (e.Status.Length > 0) previous = e.Status;
            if (e.Comment is null && status is null) continue;
            sb.Append($"\n### {(e.Date is { } d ? Show(d) : "без даты")} — {(e.Author.Length > 0 ? e.Author : "—")}")
              .Append(e.IsPublic == false ? " (внутренний)" : "")
              .Append(status is not null ? $" · статус «{status}»" : "")
              .Append('\n');
            if (e.Comment is not null) sb.Append('\n').Append(Escape(e.Comment.Trim())).Append('\n');
            written++;
        }
        if (written == 0) sb.Append("\n_Переписки нет._\n");
        return sb.ToString();
    }

    /// <summary>Текст заявки как есть, но чтобы Markdown не принял его за разметку: «#» в начале строки — заголовок,
    /// «#слово» — тег Obsidian, строка «---» — черта или граница свойств.</summary>
    internal static string Escape(string text) => string.Join("\n", text.Replace("\r", "").Split('\n').Select(line =>
    {
        line = Tags(line);
        var body = line.TrimStart();
        return body.StartsWith('#') ? line[..(line.Length - body.Length)] + "\\" + body
            : body.Trim() is "---" or "***" or "___" ? "\\" + line : line;
    }));

    private static string OneLine(string s) => Regex.Replace(s, @"\s+", " ").Trim();

    /// <summary>«#слово» — тег в Obsidian: экранируем, чтобы слова из заявок не засоряли теги базы знаний.</summary>
    private static string Tags(string s) => Regex.Replace(s, @"#(?=\p{L})", @"\#");

    /// <summary>Дата для человека — в том поясе, в каком её прислал сервер.</summary>
    private static string Show(DateTimeOffset? d) => d is { } v ? v.ToString("dd.MM.yyyy HH:mm", CultureInfo.InvariantCulture) : "";

    /// <summary>Строка шапки: «Метка: значение · метка: значение» из того, что известно.</summary>
    private static void Line(StringBuilder sb, params (string Label, string? Value)[] parts)
    {
        var known = parts.Where(p => !string.IsNullOrWhiteSpace(p.Value)).Select(p => $"{p.Label}: {Tags(OneLine(p.Value!))}").ToList();
        if (known.Count == 0) return;
        var line = string.Join(" · ", known);
        sb.Append(char.ToUpperInvariant(line[0])).Append(line[1..]).Append("  \n");
    }

    private static void Prop(StringBuilder sb, string key, string? value)
    {
        if (!string.IsNullOrWhiteSpace(value)) sb.Append($"{key}: {Quote(value)}\n");
    }

    private static void ListProp(StringBuilder sb, string key, string? commaList)
    {
        var items = (commaList ?? "").Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries);
        if (items.Length > 0) sb.Append($"{key}: [{string.Join(", ", items.Select(Quote))}]\n");
    }

    /// <summary>Дата в свойствах — ISO 8601 с поясом и долями секунды, если они есть: по ней выгрузка узнаёт, менялась ли
    /// заявка, так что запись должна читаться обратно ровно в то же значение.</summary>
    private static void DateProp(StringBuilder sb, string key, DateTimeOffset? value)
    {
        if (value is { } v) sb.Append($"{key}: {v.ToString("yyyy-MM-dd'T'HH:mm:ss.FFFFFFFzzz", CultureInfo.InvariantCulture)}\n");
    }

    /// <summary>Строка YAML в двойных кавычках: кавычки и обратная косая — с «\», переводы строк — пробелом.</summary>
    private static string Quote(string s) => "\"" + OneLine(s).Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"";

    private static string Unquote(string s) =>
        s.Length >= 2 && s[0] == '"' && s[^1] == '"' ? s[1..^1].Replace("\\\"", "\"").Replace("\\\\", "\\") : s;

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

    // ---------- что уже выгружено ----------

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

    /// <summary>Номер заявки в имени файла: «номер — название.md» или «номер.md».</summary>
    private static readonly Regex IdInName = new(@"^(\d+)( — .*)?\.md$", RegexOptions.CultureInvariant);

    /// <summary>Выгруженные раньше заявки: номер → его файлы (обычно один; больше — дубли, их уберёт перезапись), дата
    /// изменения и версия формата на момент выгрузки. Номер — из свойств файла, а не читаются они (файл занят, свойства
    /// испорчены руками) — из начала имени: такой файл перепишется, а при переименовании заявки уберётся, а не останется
    /// дублем.</summary>
    internal static Dictionary<int, (List<string>? Paths, DateTimeOffset? Changed, int Format)> ReadExisting(string ticketsDir)
    {
        var map = new Dictionary<int, (List<string>? Paths, DateTimeOffset? Changed, int Format)>();
        foreach (var path in Directory.EnumerateFiles(ticketsDir, "*.md"))
        {
            var p = ReadProps(path);
            var idText = p?.GetValueOrDefault("id")
                ?? (IdInName.Match(Path.GetFileName(path)) is { Success: true } m ? m.Groups[1].Value : null);
            if (!int.TryParse(idText, NumberStyles.None, CultureInfo.InvariantCulture, out var id)) continue;
            if (map.TryGetValue(id, out var known)) { known.Paths!.Add(path); continue; }
            map[id] = (new List<string> { path },
                DateTimeOffset.TryParse(p?.GetValueOrDefault("changed"), CultureInfo.InvariantCulture, DateTimeStyles.None, out var c) ? c : null,
                int.TryParse(p?.GetValueOrDefault("format"), NumberStyles.None, CultureInfo.InvariantCulture, out var v) ? v : 0);
        }
        return map;
    }

    /// <summary>Оглавление _index.md — по всем файлам в tickets, свежие сверху; в начале — как читать файлы (для агента,
    /// который будет разбирать выгрузку в базу знаний).</summary>
    internal static void WriteIndex(string dir)
    {
        var ticketsDir = Path.Combine(dir, TicketsFolder);
        var entries = !Directory.Exists(ticketsDir) ? new()
            : Directory.EnumerateFiles(ticketsDir, "*.md")
                .Select(path => (Path: path, Props: ReadProps(path)))
                .Where(e => e.Props is not null && e.Props.ContainsKey("id"))
                .Select(e => (Name: Path.GetFileNameWithoutExtension(e.Path), P: e.Props!))
                .OrderByDescending(e => e.P.GetValueOrDefault("created") ?? "", StringComparer.Ordinal)
                .ToList();
        var sb = new StringBuilder("# Заявки из Интрасервиса\n\n");
        sb.Append($"Выгрузка TicketBoard для базы знаний — обновлено {DateTime.Now.ToString("dd.MM.yyyy HH:mm", CultureInfo.InvariantCulture)}, заявок: {entries.Count}.\n\n");
        sb.Append("Каждый файл в папке `tickets` — одна заявка. В начале файла свойства: номер, статус, даты (создана, изменена, решена), ")
          .Append("сервис, тип, категории, инициатор, исполнители и их группа, ссылка. Дальше описание заявки и переписка по времени — ")
          .Append("от первой записи к последней; «(внутренний)» — комментарий, которого заявитель не видел, «статус «…»» — на этой записи ")
          .Append("статус поменялся. Решение обычно в последних записях закрытой заявки. Телефоны и почта людей в выгрузку не попадают. ")
          .Append("Файл переписывается при следующей выгрузке, если заявка менялась.\n\n");
        foreach (var (name, p) in entries)
        {
            var created = DateTimeOffset.TryParse(p.GetValueOrDefault("created"), CultureInfo.InvariantCulture, DateTimeStyles.None, out var c)
                ? " · " + c.ToString("dd.MM.yyyy", CultureInfo.InvariantCulture) : "";
            var status = p.GetValueOrDefault("status") is { Length: > 0 } s ? $" · {s}" : "";
            sb.Append($"- [[{name}]]{created}{status}\n");
        }
        WriteAtomic(Path.Combine(dir, IndexFile), sb.ToString());
    }
}
