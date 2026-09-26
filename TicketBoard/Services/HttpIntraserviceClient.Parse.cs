using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.CompilerServices;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Разбор ответов Интрасервиса: все имена полей API — только здесь. Форма json у половины методов в документации не показана (только xml), поэтому разборщики терпят несколько обёрток и пропускают негодные строки.</summary>
public sealed partial class HttpIntraserviceClient
{
    private static readonly Regex HtmlBreaks = new(@"<br\s*/?>|</p>|</div>|</li>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex HtmlTags = new(@"<[^>]*>", RegexOptions.Compiled);
    private static readonly Regex ManyNewlines = new(@"\n\s*\n\s*\n+", RegexOptions.Compiled);
    private static readonly Regex WcfDate = new(@"^/Date\((-?\d+)(?:[+-]\d{4})?\)/$", RegexOptions.Compiled);
    private static readonly string[] DateFormats = { "dd.MM.yyyy HH:mm:ss", "dd.MM.yyyy HH:mm", "dd.MM.yyyy" };
    // границы DateTimeOffset в миллисекундах от 1970-01-01 UTC: за ними FromUnixTimeMilliseconds бросает исключение
    private static readonly long UnixMsMin = DateTimeOffset.MinValue.ToUnixTimeMilliseconds();
    private static readonly long UnixMsMax = DateTimeOffset.MaxValue.ToUnixTimeMilliseconds();

    // ---------- разбор ответа: все имена полей API — только здесь ----------

    /// <summary>Ответ api/task/{id}?include=status по документации: {"Task": {...}, "Statuses": [{"Id", "Name"}]}.
    /// Поля заявки: Id, Name, Description, StatusId, StatusName, Creator, Executors, ExecutorGroup, Changed. Если обёртки
    /// Task нет — читаем корень.</summary>
    internal static IntraserviceTask? Parse(string json, int id)
    {
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        if (root.ValueKind != JsonValueKind.Object) return null;
        var task = Prop(root, "Task") is { ValueKind: JsonValueKind.Object } t ? t : root;
        if (Str(task, "Name") is not { } name) return null;

        return new(Int(task, "Id") ?? id, name.Trim(), StatusOf(task, root), HtmlToText(Str(task, "Description")),
            Field(task, "Creator"), Names(task, "Executors"), Field(task, "ExecutorGroup"), Date(task, "Changed"));
    }

    /// <summary>Ответ api/tasklifetime?include=status: {"TaskLifetimeList": {"TaskLifetimes": [...], "Statuses": [...],
    /// "Paginator": {...}}}. Терпим и обёртку попроще ({"TaskLifetimes": [...]}), и голый массив — json-формы в
    /// документации нет, там xml. Поля записи: Date, Editor, EditorId, StatusId, Comments, IsPublic.</summary>
    internal static (IReadOnlyList<IntraserviceEvent> Events, bool HasMore)? ParseLifetime(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, "TaskLifetimes", "TaskLifetimeList") is not { } u) return null;

        var events = new List<IntraserviceEvent>();
        foreach (var e in u.Rows.EnumerateArray())
            if (e.ValueKind == JsonValueKind.Object)
                events.Add(new(Date(e, "Date"), Str(e, "Editor")?.Trim() ?? "", StatusOf(e, u.Blocks),
                    HtmlToText(Str(e, "Comments")), Bool(e, "IsPublic"), Int(e, "EditorId")));

        return (events, HasNextPage(u.Blocks));
    }

    /// <summary>Ответ api/task?search=…&amp;include=status: {"TaskList": {"Tasks": [...], "Statuses": [...],
    /// "Paginator": {...}}} — так же терпим {"Tasks": [...]} и голый массив. Поля строки: Id, Name, StatusId, Created,
    /// Creator, Description, Executors, ExecutorGroup, Changed. Строка без номера или названия бесполезна — пропускаем
    /// её, а не весь ответ.</summary>
    internal static (IReadOnlyList<IntraserviceFound> Found, int Total)? ParseSearch(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, "Tasks", "TaskList") is not { } u) return null;

        var found = new List<IntraserviceFound>();
        foreach (var t in u.Rows.EnumerateArray())
            if (t.ValueKind == JsonValueKind.Object && Int(t, "Id") is int id && Str(t, "Name")?.Trim() is { Length: > 0 } name)
                found.Add(new(id, name, StatusOf(t, u.Blocks), Field(t, "Creator"), Date(t, "Created"),
                    HtmlToText(Str(t, "Description")), Names(t, "Executors"), Field(t, "ExecutorGroup"), Date(t, "Changed")));

        // общее число совпадений знает Paginator; нет его — знаем только то, что пришло
        var total = Paginator(u.Blocks) is { } p && Int(p, "Count") is int count ? count : found.Count;
        return (found, total);
    }

    /// <summary>Ответ api/user?getcurrentuserinfo=true (док., стр. 57): объект с полями Id, Login, Name, RoleType
    /// и прочими. В документации это xml с корнем &lt;CurrenUserInfo&gt; — буква «t» потеряна в самой документации,
    /// поэтому понимаем оба написания обёртки и голый объект. Нужны Id (нет его — null) и Name.</summary>
    internal static IntraserviceUser? ParseCurrentUser(string json)
    {
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        if (root.ValueKind != JsonValueKind.Object) return null;

        var wrapper = Prop(root, "CurrenUserInfo") ?? Prop(root, "CurrentUserInfo");
        var user = wrapper is { ValueKind: JsonValueKind.Object } w ? w : root;
        return Int(user, "Id") is int id && id > 0 ? new(id, Str(user, "Name")?.Trim() ?? "") : null; // нулевой номер — тоже не пользователь
    }

    /// <summary>Ответ api/taskstatus (док., стр. 38-39): в документации xml с корнем &lt;ArrayOfTaskStatusView&gt;,
    /// в json это, скорее всего, голый массив — терпим и его, и обёртки {"TaskStatusView": [...]} и {"Statuses": [...]}.
    /// Поля строки: Id, Name, IsFixed, IsFinal. Строка без номера бесполезна — пропускаем её, а не весь ответ.</summary>
    internal static IReadOnlyList<IntraserviceStatus>? ParseStatuses(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if ((Unwrap(doc.RootElement, "TaskStatusView", "ArrayOfTaskStatusView")
             ?? Unwrap(doc.RootElement, "Statuses", "TaskStatusList")) is not { } u) return null;

        var statuses = new List<IntraserviceStatus>();
        foreach (var s in u.Rows.EnumerateArray())
            if (s.ValueKind == JsonValueKind.Object && Int(s, "Id") is int id)
                statuses.Add(new(id, Str(s, "Name")?.Trim() ?? "", Bool(s, "IsFixed") ?? false, Bool(s, "IsFinal") ?? false));

        return statuses;
    }

    private static JsonElement? Prop(JsonElement e, string name)
    {
        foreach (var p in e.EnumerateObject())
            if (p.Name.Equals(name, StringComparison.OrdinalIgnoreCase)) return p.Value;
        return null;
    }

    private static string? Str(JsonElement e, string name) => Prop(e, name) is { ValueKind: JsonValueKind.String } v ? v.GetString() : null;

    private static int? Int(JsonElement e, string name) => Prop(e, name) is { ValueKind: JsonValueKind.Number } v && v.TryGetInt32(out var n) ? n : null;

    /// <summary>Строковое поле, где важно отличить «нет в ответе» (null — синхронизация оставит, что было) от «пусто»
    /// (""): json null — это «пусто».</summary>
    private static string? Field(JsonElement e, string name) => Prop(e, name) switch
    {
        { ValueKind: JsonValueKind.String } v => v.GetString()!.Trim(),
        { ValueKind: JsonValueKind.Null } => "",
        _ => null,
    };

    /// <summary>Люди в поле (Executors): строка «Иванов И. И., Петров П.» (запятая или точка с запятой), массив строк или
    /// массив объектов с Name — всё приводим к «Иванов И. И., Петров П.». Поля нет — null; пусто или json null — "".</summary>
    private static string? Names(JsonElement e, string name)
    {
        IEnumerable<string?>? names = Prop(e, name) switch
        {
            { ValueKind: JsonValueKind.String } v => v.GetString()!.Split(',', ';'),
            { ValueKind: JsonValueKind.Array } a => a.EnumerateArray().Select(i => i.ValueKind switch
            {
                JsonValueKind.String => i.GetString(),
                JsonValueKind.Object => Str(i, "Name"),
                _ => null,
            }).ToList(),
            { ValueKind: JsonValueKind.Null } => Array.Empty<string>(),
            _ => null,
        };
        return names is null ? null : string.Join(", ", names.Select(n => n?.Trim()).Where(n => !string.IsNullOrEmpty(n)));
    }

    /// <summary>Название статуса строки ответа: своё поле StatusName, иначе по StatusId из блока Statuses
    /// (он приходит по include=status), иначе «статус {id}». Статуса нет вовсе — пустая строка.</summary>
    private static string StatusOf(JsonElement row, JsonElement? blocks)
    {
        var name = Str(row, "StatusName");
        if (string.IsNullOrWhiteSpace(name) && Int(row, "StatusId") is int statusId)
            name = (blocks is { ValueKind: JsonValueKind.Object } b && Prop(b, "Statuses") is { ValueKind: JsonValueKind.Array } list
                ? list.EnumerateArray().Where(s => s.ValueKind == JsonValueKind.Object && Int(s, "Id") == statusId)
                      .Select(s => Str(s, "Name")).FirstOrDefault()
                : null) ?? $"статус {statusId}";
        return name?.Trim() ?? "";
    }

    /// <summary>Список строк ответа и объект, рядом с которым лежат блоки Statuses и Paginator. Понимаем три формы:
    /// {"Обёртка": {"Имя": [...]}}, {"Имя": [...]} и голый массив (у него соседних блоков нет). null — не тот ответ.</summary>
    private static (JsonElement Rows, JsonElement? Blocks)? Unwrap(JsonElement root, string name, string wrapper)
    {
        if (root.ValueKind == JsonValueKind.Array) return (root, null);
        if (root.ValueKind != JsonValueKind.Object) return null;
        // блоки Statuses и Paginator лежат рядом со списком: внутри обёртки, если она есть, иначе в корне
        var blocks = Prop(root, wrapper) is { ValueKind: JsonValueKind.Object } w ? w : root;
        if (Prop(blocks, name) is { ValueKind: JsonValueKind.Array } rows) return (rows, blocks);
        return null;
    }

    /// <summary>Блок Paginator: Count, Page, PageCount, PageSize, CountOnPage. В документации он объект, но в одном
    /// примере — массив из одного объекта; понимаем обе формы.</summary>
    private static JsonElement? Paginator(JsonElement? blocks)
    {
        if (blocks is not { ValueKind: JsonValueKind.Object } b || Prop(b, "Paginator") is not { } p) return null;
        if (p.ValueKind == JsonValueKind.Object) return p;
        if (p.ValueKind == JsonValueKind.Array)
            foreach (var item in p.EnumerateArray())
                if (item.ValueKind == JsonValueKind.Object) return item;
        return null;
    }

    /// <summary>Есть ли ещё страницы: номер страницы меньше их общего числа. Нет Paginator'а — считаем, что нет.</summary>
    private static bool HasNextPage(JsonElement? blocks) =>
        Paginator(blocks) is { } p && Int(p, "Page") is int page && Int(p, "PageCount") is int pages && page < pages;

    /// <summary>Дата из ответа. Три вида: «12.11.2015 13:44:53» из документации, ISO 8601 и wcf «/Date(1447335893000)/».
    /// Формат документации пробуем первым: инвариантная культура иначе прочитает 12.11 как 11 декабря.
    /// Непонятная дата — null, сама запись при этом не теряется.</summary>
    private static DateTimeOffset? Date(JsonElement e, string name)
    {
        if (Str(e, name)?.Trim() is not { Length: > 0 } s) return null;
        if (WcfDate.Match(s) is { Success: true } m
            && long.TryParse(m.Groups[1].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var ms))
            return ms >= UnixMsMin && ms <= UnixMsMax ? DateTimeOffset.FromUnixTimeMilliseconds(ms).ToLocalTime() : null;
        if (DateTimeOffset.TryParseExact(s, DateFormats, CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal, out var exact))
            return exact;
        return DateTimeOffset.TryParse(s, CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal, out var any) ? any : null;
    }

    /// <summary>Логическое поле: true/false в json или строкой «True»/«False» (в документации встречаются оба
    /// написания). Непонятное значение — null.</summary>
    private static bool? Bool(JsonElement e, string name) => Prop(e, name) switch
    {
        { ValueKind: JsonValueKind.True } => true,
        { ValueKind: JsonValueKind.False } => false,
        { ValueKind: JsonValueKind.String } v when bool.TryParse(v.GetString(), out var b) => b,
        _ => null,
    };

    /// <summary>Описание в Интрасервисе обычно HTML из редактора — оставляем текст.</summary>
    private static string? HtmlToText(string? html)
    {
        if (string.IsNullOrWhiteSpace(html)) return null;
        var text = WebUtility.HtmlDecode(HtmlTags.Replace(HtmlBreaks.Replace(html, "\n"), "")).Replace("\r", "");
        return ManyNewlines.Replace(text, "\n\n").Trim();
    }
}
