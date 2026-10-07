using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    // ---------- файл заявки: имя и текст ----------

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
}
