using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

public static partial class KnowledgeExport
{
    // ---------- файл заявки: имя и текст ----------

    /// <summary>Значение свойства source: по нему файл узнаётся как выгруженный нами (а не как заметка пользователя с номером заявки в имени).</summary>
    internal const string SourceName = "intraservice";

    /// <summary>Имя файла: «номер — название.md». Из названия убраны знаки, запрещённые Windows, и те, что ломают ссылки
    /// Obsidian (# ^ [ ] |), обратные кавычки (код внутри ссылки [[…]] в оглавлении — а код бывает DataviewJS) и «%%»
    /// (комментарий Obsidian спрятал бы оглавление ниже); длинное — обрезано до titleMax знаков. По номеру в начале имени заявка
    /// находится при следующей выгрузке.</summary>
    internal static string FileName(int id, string title, int titleMax = 80)
    {
        var clean = Regex.Replace(Regex.Replace(Regex.Replace(Secrets.Mask(OneLine(title)), @"[\\/:*?""<>|#^\[\]`\x00-\x1F]", " "), "%{2,}", "%"), @"\s+", " ")
            .Trim().TrimEnd('.', ' ');
        // не разрезать пару-заменитель (эмодзи в конце): имя с половинкой не переживёт запись в UTF-8, и ссылка на файл не найдёт его
        if (clean.Length > titleMax) clean = clean[..(titleMax > 0 && char.IsHighSurrogate(clean[titleMax - 1]) ? titleMax - 1 : titleMax)].TrimEnd('.', ' ');
        return clean.Length == 0 ? $"{id}.md" : $"{id} — {clean}.md";
    }

    /// <summary>Заявка файлом Markdown: свойства, шапка, описание, переписка по времени.</summary>
    internal static string Format(IntraserviceFound f, IReadOnlyList<IntraserviceEvent> events, string url, DateTimeOffset exported,
        bool truncated = false, ExportContext? context = null, IReadOnlyList<Attachment>? attachments = null)
    {
        // не сказано, что с вложениями, — только известны (просмотр, буфер, запрос агенту ничего не качают)
        attachments ??= (f.Extra?.Files ?? Array.Empty<IntraserviceFile>()).Select(file => new Attachment(file)).ToList();
        // пароли — первым делом: файл уходит в GitHub, этот же текст — в запрос агенту в облаке
        // название — одной строкой до скрытия: склейка строк может сложить новое «пароль: x», а показывают его одной строкой
        f = f with { Name = Secrets.Mask(OneLine(f.Name)), Description = f.Description is { } description ? Secrets.Mask(description) : null };
        events = events.Select(e => e.Comment is { } comment ? e with { Comment = Secrets.Mask(comment) } : e).ToList();
        var facts = TicketHistory.Of(f, events, context ?? ExportContext.Unknown);
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
        Prop(sb, "company", x?.Company);
        ListProp(sb, "assets", x?.Assets);
        if (x?.Files is { } files) sb.Append($"files: {files.Count.ToString(CultureInfo.InvariantCulture)}\n");
        // сколько вложений нет в базе (не скачаны по настройке или не скачались): сменят настройку — выгрузка докачает
        if (attachments.Count(a => a.State is AttachmentState.Skipped or AttachmentState.Failed) is > 0 and var missing)
            sb.Append($"files_missing: {missing.ToString(CultureInfo.InvariantCulture)}\n");
        DateProp(sb, "closed", x?.Closed);
        // история урезана (длиннее MaxLifetimePages) — её начала нет: участников, реакцию и счёт по ней не пишем
        if (!truncated) ListProp(sb, "participants", facts.Participants);
        Prop(sb, "my_role", facts.MyRole);
        if (!truncated) HoursProp(sb, "first_reaction_h", facts.FirstReactionHours);
        HoursProp(sb, "resolve_h", facts.ResolveHours);
        if (!truncated)
            sb.Append($"reopened: {facts.Reopened.ToString(CultureInfo.InvariantCulture)}\n")
              .Append($"comments_public: {facts.PublicComments.ToString(CultureInfo.InvariantCulture)}\n")
              .Append($"comments_internal: {facts.InternalComments.ToString(CultureInfo.InvariantCulture)}\n");
        Prop(sb, "closing_comment", facts.ClosingComment);
        Prop(sb, "url", url);
        // точная версия заявки на сервере — по ней выгрузка узнаёт неизменный файл: в changed секунды без долей. Вложение не
        // скачалось — версии нет: следующая выгрузка перечитает заявку и попробует снова
        if (f.Changed is { } changed && attachments.All(a => a.State != AttachmentState.Failed))
            sb.Append($"revision: {MachineDate.Revision(changed)}\n");
        sb.Append($"source: {SourceName}\n");
        sb.Append($"format: {FormatVersion}\n");
        DateProp(sb, "exported", exported);
        sb.Append("---\n\n");

        sb.Append($"# {f.Id} · {Md(f.Name)}\n\n");
        Line(sb, ("Статус", f.Status), ("создана", Show(f.Created)), ("изменена", Show(f.Changed)), ("решена", Show(x?.Resolved)));
        Line(sb, ("Инициатор", f.Creator), ("исполнители", f.Executors), ("группа", f.ExecutorGroup));
        Line(sb, ("Сервис", x?.Service), ("тип", x?.Type), ("категории", x?.Categories));
        Line(sb, ("Ссылка", url));

        sb.Append("\n## Описание\n\n");
        sb.Append(string.IsNullOrWhiteSpace(f.Description) ? "_Описания нет._\n" : Escape(f.Description.Trim()) + "\n");

        if (attachments.Count > 0)
        {
            sb.Append("\n## Вложения\n\n");
            foreach (var a in attachments) sb.Append("- ").Append(AttachmentLine(a)).Append('\n');
        }

        sb.Append("\n## Переписка\n");
        if (truncated)
            sb.Append($"\n_Переписка длиннее: показаны последние {events.Count} записей, более ранние не выгружены._\n");
        var written = 0;
        string? previous = null;
        // страницы приходят свежими сверху: разворачиваем, чтобы при равных датах осталась очерёдность записей
        foreach (var e in Enumerable.Reverse(events).OrderBy(e => e.Date ?? DateTimeOffset.MinValue))
        {
            // статус меняется на этой записи — пишем его; просто смена полей без комментария — шум, пропускаем
            var status = e.Status.Length > 0 && e.Status != previous ? e.Status : null;
            if (e.Status.Length > 0) previous = e.Status;
            if (e.Comment is null && status is null) continue;
            sb.Append($"\n### {(e.Date is { } d ? Show(d) : "без даты")} — {(e.Author.Length > 0 ? Md(e.Author) : "—")}")
              .Append(e.IsPublic == false ? " (внутренний)" : "")
              .Append(status is not null ? $" · статус «{Md(status)}»" : "")
              .Append('\n');
            if (e.Comment is not null) sb.Append('\n').Append(Escape(e.Comment.Trim())).Append('\n');
            written++;
        }
        if (written == 0) sb.Append("\n_Переписки нет._\n");
        return sb.ToString();
    }

    /// <summary>Строка вложения: лежит в базе — ссылкой Obsidian с путём от корня базы (картинка — встроенной, ![[…]]) и
    /// размером; нет — имя и почему нет (у ошибки — первая строка, без тела ответа сервера); только известно — имя.</summary>
    private static string AttachmentLine(Attachment a) => a.State switch
    {
        AttachmentState.Saved => $"{(Attachments.IsImage(a.Path!) ? "!" : "")}[[{a.Path}]]" + (a.Size is long size ? $" · {Attachments.Size(size)}" : ""),
        AttachmentState.Skipped => $"{Md(a.Title)} · не скачано",
        AttachmentState.Failed => $"{Md(a.Title)} · не скачалось: {Md(a.Note)}",
        _ => Md(a.Title),
    };

    /// <summary>Текст заявки как есть, но чтобы Markdown не принял его за разметку: «#» в начале строки — заголовок,
    /// «#слово» — тег Obsidian, строка «---» — черта или граница свойств, «```» и «~~~» в начале, в том числе в цитате и пункте
    /// списка, — ограда кода (без закрывающей она съела бы все следующие записи переписки, а ```dataviewjs выполнился бы),
    /// и не html (незакрытый «&lt;!--» тоже съел бы всё ниже), не команда плагинов и не «%%» — Inert.</summary>
    internal static string Escape(string text) => Inert(string.Join("\n", text.Replace("\r", "").Split('\n').Select(line =>
    {
        line = Tags(line);
        var body = line.TrimStart();
        var inner = body[Containers.Match(body).Length..];
        return body.StartsWith('#') ? line[..(line.Length - body.Length)] + "\\" + body
            : inner.StartsWith("```") || inner.StartsWith("~~~") ? line[..(line.Length - inner.Length)] + "\\" + inner
            : body.Trim() is "---" or "***" or "___" ? "\\" + line : line;
    })));

    /// <summary>Начало строки из маркеров цитаты и пунктов списка («> », «- », «1. », «12) »): ограда после них — тоже ограда.</summary>
    private static readonly Regex Containers = new(@"^(?:(?:>|[-*+](?=\s)|\d{1,9}[.)](?=\s))\s*)*", RegexOptions.CultureInvariant);

    /// <summary>Не команда плагинов Obsidian и не разметка, что прячет текст: «&lt;» перед латинской буквой, «/», «!», «?», «%» —
    /// html или команда Templater (с триггером на новый файл она выполнилась бы, едва файл появится в хранилище; а html вроде
    /// &lt;code&gt;$&lt;span&gt;&lt;/span&gt;= …&lt;/code&gt; — DataviewJS без «$=» в тексте), «$=» — начало встроенного запроса
    /// DataviewJS (выполняется при показе заметки), «%%» — комментарий Obsidian (непарный спрятал бы всё ниже: батник с %%f).
    /// Заменяются сущностями html: в тексте Markdown показывает их теми же знаками, в коде (где сущности не раскрываются) они
    /// ничего не начинают, и повторная замена ничего не меняет. Цена: в коде — строках с отступом от 4 пробелов и `…` — они
    /// видны как записаны (List&amp;lt;T&gt;); зато xml в обычном тексте теперь виден, а не съеден как html. Автоссылки
    /// (&lt;https://…&gt;, &lt;почта@домен&gt;) остаются ссылками; «$=», «%%» и (Tags) «#слово» внутри них правятся, как везде,
    /// — такая ссылка поведёт не туда, как и до 0.15.0 с «#слово». «&lt;» решается после замен: почта, ставшая от них не почтой,
    /// — уже не ссылка, и повторная замена ничего не меняет. Текст заявок пишут люди со стороны, а автовыгрузка кладёт файлы
    /// без участия человека. ponytail: Templater и DataviewJS с началами по умолчанию; картинка по ссылке (![](…)) из текста
    /// заявки при показе по-прежнему загрузится.</summary>
    private static string Inert(string s) => HtmlStart.Replace(s.Replace("$=", "&#36;=").Replace("%%", "%&#37;"), "&lt;");

    /// <summary>«&lt;», с которого начался бы html или команда Templater: перед «/», «!», «?», «%» — всегда, перед латинской
    /// буквой — если это не автоссылка CommonMark (схема из 2–32 знаков и «:» или почта, до «&gt;» без пробелов). Почта
    /// здесь начинается с буквы: «&lt;%…@…&gt;» — команда Templater, а не ссылка.</summary>
    private static readonly Regex HtmlStart = new(
        @"<(?:(?=[/!?%])|(?=[A-Za-z])(?!(?:[A-Za-z][A-Za-z0-9+.\-]{1,31}:[^\x00-\x20\x7F<>]*"
        + @"|[A-Za-z][A-Za-z0-9.!#$%&'*+/=?^_`{|}~\-]*@[A-Za-z0-9](?:[A-Za-z0-9\-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9\-]{0,61}[A-Za-z0-9])?)*)>))",
        RegexOptions.CultureInvariant);

    /// <summary>Поле заявки в строке Markdown (заголовок, шапка, запись переписки): одной строкой, без тегов и команд плагинов.</summary>
    private static string Md(string s) => Inert(Tags(OneLine(s)));

    internal static string OneLine(string s) => Regex.Replace(s, @"\s+", " ").Trim();

    /// <summary>«#слово» — тег в Obsidian: экранируем, чтобы слова из заявок не засоряли теги базы знаний.</summary>
    private static string Tags(string s) => Regex.Replace(s, @"#(?=\p{L})", @"\#");

    /// <summary>Дата для человека — в том поясе, в каком её прислал сервер.</summary>
    private static string Show(DateTimeOffset? d) => d is { } v ? v.ToString("dd.MM.yyyy HH:mm", CultureInfo.InvariantCulture) : "";

    /// <summary>Строка шапки: «Метка: значение · метка: значение» из того, что известно.</summary>
    private static void Line(StringBuilder sb, params (string Label, string? Value)[] parts)
    {
        var known = parts.Where(p => !string.IsNullOrWhiteSpace(p.Value)).Select(p => $"{p.Label}: {Md(p.Value!)}").ToList();
        if (known.Count == 0) return;
        var line = string.Join(" · ", known);
        sb.Append(char.ToUpperInvariant(line[0])).Append(line[1..]).Append("  \n");
    }

    private static void Prop(StringBuilder sb, string key, string? value)
    {
        if (!string.IsNullOrWhiteSpace(value)) sb.Append($"{key}: {Quote(value)}\n");
    }

    private static void ListProp(StringBuilder sb, string key, string? commaList) =>
        ListProp(sb, key, (commaList ?? "").Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries));

    private static void ListProp(StringBuilder sb, string key, IReadOnlyList<string> items)
    {
        if (items.Count > 0) sb.Append($"{key}: [{string.Join(", ", items.Select(Quote))}]\n");
    }

    /// <summary>Часы — с одной цифрой после точки (YAML; в таблице — запятая для русского Excel); не было — свойства нет.</summary>
    private static void HoursProp(StringBuilder sb, string key, double? hours)
    {
        if (hours is { } h) sb.Append($"{key}: {h.ToString("0.0", CultureInfo.InvariantCulture)}\n");
    }

    /// <summary>Дата в свойствах — в виде для программ (MachineDate): по ней же выгрузка узнаёт, менялась ли заявка.</summary>
    private static void DateProp(StringBuilder sb, string key, DateTimeOffset? value)
    {
        if (value is { } v) sb.Append($"{key}: {MachineDate.Of(v)}\n");
    }

    /// <summary>Строка YAML в двойных кавычках: кавычки и обратная косая — с «\», переводы строк — пробелом; значение — через
    /// Inert: Templater читает текст файла, а Dataview показывает значения свойств как Markdown (таблица заявок).</summary>
    private static string Quote(string s) => "\"" + Inert(OneLine(s)).Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"";

    internal static string Unquote(string s) =>
        s.Length >= 2 && s[0] == '"' && s[^1] == '"' ? s[1..^1].Replace("\\\"", "\"").Replace("\\\\", "\\") : s;
}

/// <summary>Дата для чтения программой — свойства файла и колонки tickets.csv, один вид везде: ISO 8601, секунды без долей,
/// местное время с поясом — «2026-10-16T09:45:39+03:00». Сервер присылает время без пояса; пояс — этого компьютера (как в
/// Windows), так что у дат одного ПК он один (в Москве — +03:00 с осени 2014), и строки сравниваются посимвольно и
/// сортируются как даты (поиск «изменённые после X» не теряет заявки того же дня). Человеческие даты в тексте — не здесь.</summary>
public static class MachineDate
{
    /// <summary>Доли секунды отбрасываются, не округляются: 39,999 — всё ещё 39-я секунда.</summary>
    public static string Of(DateTimeOffset date) => date.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture);

    /// <summary>Дата свойства файла (этого вида, прежнего — с долями, или правленая рукой, но с поясом); не дата — false.</summary>
    public static bool TryRead(string? text, out DateTimeOffset date) =>
        DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.None, out date);

    /// <summary>Точная версия заявки — её дата изменения до тика, числом (не датой: для программы выгрузки, не для поиска).
    /// По ней выгрузка узнаёт неизменный файл: по changed без долей две правки в одну секунду неотличимы, и автовыгрузка
    /// сочла бы файл готовым после первой — закрывающий комментарий второй не доехал бы никогда.</summary>
    public static string Revision(DateTimeOffset changed) => changed.UtcTicks.ToString(CultureInfo.InvariantCulture);

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var msk = TimeSpan.FromHours(3);
        var live = new DateTimeOffset(2026, 10, 16, 9, 45, 39, msk).AddTicks(3_629_720);   // как присылает сервер: микросекунды
        var whole = new DateTimeOffset(2026, 10, 16, 9, 45, 39, msk);
        Debug.Assert(Of(live) == "2026-10-16T09:45:39+03:00", Of(live));
        // доли отбрасываются, не округляются: так же режет и сравнение
        var almost = new DateTimeOffset(2026, 10, 16, 9, 45, 39, 999, msk);
        Debug.Assert(Of(almost) == "2026-10-16T09:45:39+03:00", Of(almost));
        // пояс — как у даты: местное время сервера с его смещением
        Debug.Assert(Of(new DateTimeOffset(2026, 10, 16, 9, 45, 39, TimeSpan.FromHours(5))) == "2026-10-16T09:45:39+05:00"
            && Of(new DateTimeOffset(2026, 10, 16, 6, 45, 39, TimeSpan.Zero)) == "2026-10-16T06:45:39+00:00");
        // версия — до тика: две правки в одну секунду различаются; тот же миг в другом поясе — та же версия
        Debug.Assert(Revision(live) != Revision(whole) && Revision(live) == Revision(live.ToOffset(TimeSpan.Zero))
            && Revision(whole) == "639277299390000000", Revision(whole));
        // чтение: этот вид, прежний (с долями), не дата
        Debug.Assert(TryRead(Of(live), out var back) && back == whole && TryRead("2026-10-16T09:45:39.362972+03:00", out var old) && old == live
            && !TryRead("вчера", out _) && !TryRead(null, out _));
    }
}
