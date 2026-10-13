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
    private const string HtmlTagNames = "p|br|div|span|b|i|u|s|em|strong|font|a|img|ul|ol|li|dl|dt|dd|table|thead|tbody|tfoot|tr|td|th"
        + "|caption|h[1-6]|blockquote|pre|code|hr|sub|sup|strike|center|small|big|nobr|wbr|html|head|body|style|title|meta|link"
        + "|section|article|header|footer|nav|aside|main|address|figure|figcaption|form|fieldset|legend|label|input|button";
    // конец тега: «>», «/>» или атрибуты через пробел — «<p/7z>» из кода маркировки не тег
    private const string TagEnd = @"(?:\s[^<>]*)?/?>";
    private static readonly Regex HtmlBrOrImg = new($@"<br{TagEnd}|<img\s[^<>]*>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex HtmlOpenTags = new($@"<({HtmlTagNames}){TagEnd}", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex HtmlCloseTags = new($@"</({HtmlTagNames})[ \t]*>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    /// <summary>Абзацы, таблица, документ: в таком html пробелы и переводы строк исходника — лишь пробел (письмо Outlook переносит
    /// в исходнике строки посреди фразы), кроме &lt;pre&gt;. Без них (одни &lt;br&gt; или строчный тег) перевод строки — строка.</summary>
    private static readonly Regex HtmlBlocks = new($@"<(?:p|div|table|li|dl|h[1-6]|blockquote|section|article|html|body){TagEnd}",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);
    // не текст: стили, скрипты и заголовок документа (title в head — подпись окна, браузер его в странице не показывает)
    private static readonly Regex HtmlNoiseStart = new($@"<(style|script|head|title){TagEnd}", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    // <pre> — как есть: в нём строки — строки
    private static readonly Regex HtmlPreStart = new($@"<(pre){TagEnd}", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex HtmlTitleStart = new($@"<(title){TagEnd}", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly ConcurrentDictionary<string, Regex> HtmlBlockEnds = new(StringComparer.OrdinalIgnoreCase);
    private static Regex BlockEnd(string name) =>
        HtmlBlockEnds.GetOrAdd(name, n => new Regex($@"</{Regex.Escape(n)}\s*>", RegexOptions.IgnoreCase | RegexOptions.Compiled));
    private static readonly Regex HtmlSpace = new(@"\s+", RegexOptions.Compiled);
    // начало ячейки — вместе с абзацем в ней (у Word в каждой ячейке свой <p>): не новая строка; конец ячейки — пробел, вместе
    // с концом абзаца в ней и пробелами вокруг
    private static readonly Regex HtmlCellStarts = new($@"<t[dh]{TagEnd}(?:\s|<(?:p|div){TagEnd})*", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    // (?<!\s) у шаблонов, начинающихся с пробелов: совпадение — лишь с первого пробела серии, иначе длинную серию без тега за
    // ней перечитывал бы каждый её пробел (100 000 пробелов — 2 минуты)
    private static readonly Regex HtmlCells = new($@"(?<!\s)(?:\s|<br{TagEnd}|</(?:p|div)\s*>)*</t[dh]\s*>\s*", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    // новая строка: <br>, <hr>, начало и конец блока; несколько подряд («</h1></div><div>») — одна строка, и перевод строки
    // исходника сразу за ними — та же строка
    private static readonly Regex HtmlBreaks = new($@"(?:<br{TagEnd}|(?<!\s)(?:\s*(?:</?(?:p|div|li|ul|ol|dl|dt|dd|tr|table|h[1-6]|blockquote|pre"
        + $@"|header|footer|section|article|form|fieldset|legend|caption|address){TagEnd}|<hr{TagEnd}))+)(?:[ \t]*\r?\n)?",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);
    /// <summary>Что снимается в html — разметка, которую браузер не показывает: тег (и Word: «o:p», «v:shape»), объявление
    /// («&lt;!DOCTYPE»), условный блок Outlook («&lt;![if …]&gt;»). «&lt;», за которым не имя тега («&lt; 5 шт»), — текст.</summary>
    private static readonly Regex HtmlTags = new($@"</?[a-z][a-z0-9-]*(?::[a-z][a-z0-9-]*)?{TagEnd}|<[!?][^<>]*>",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex SpaceRuns = new(@"[ \t]{2,}", RegexOptions.Compiled);
    private static readonly Regex LineSpaces = new(@"[ \t]*\n[ \t]*", RegexOptions.Compiled);
    private static readonly Regex LineEndSpaces = new(@"[ \t]+\n", RegexOptions.Compiled);
    // строка из одних пробелов (и неразрывных: пустой абзац Word — «&nbsp;») — пустая: иначе Markdown склеил бы абзацы
    private static readonly Regex WhiteLines = new(@"^[^\S\n]+$", RegexOptions.Multiline | RegexOptions.Compiled);
    private static readonly Regex ManyNewlines = new(@"\n\s*\n\s*\n+", RegexOptions.Compiled);
    private static readonly Regex LegacyBreaks = new(@"<br\s*/?>|</p>|</div>|</li>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex LegacyTags = new(@"<[^>]*>", RegexOptions.Compiled);

    /// <summary>Разбор описания до 0.15.1: «тегом» считалось всё от «&lt;» до «&gt;» — и «&lt; 5 шт, заказ &gt;» тоже. Только чтобы
    /// узнать на доске описание, им обрезанное (MainViewModel.Apply чинит его). ponytail: убрать, когда старые карточки починятся.</summary>
    internal static string? LegacyText(string? html)
    {
        if (string.IsNullOrWhiteSpace(html)) return null;
        var text = WebUtility.HtmlDecode(LegacyTags.Replace(LegacyBreaks.Replace(html, "\n"), "")).Replace("\r", "");
        return ManyNewlines.Replace(text, "\n\n").Trim();
    }
    private static readonly Regex WcfDate = new(@"^/Date\((-?\d+)(?:[+-]\d{4})?\)/$", RegexOptions.Compiled);
    private static readonly string[] DateFormats = { "dd.MM.yyyy HH:mm:ss", "dd.MM.yyyy HH:mm", "dd.MM.yyyy" };
    // границы DateTimeOffset в миллисекундах от 1970-01-01 UTC: за ними FromUnixTimeMilliseconds бросает исключение
    private static readonly long UnixMsMin = DateTimeOffset.MinValue.ToUnixTimeMilliseconds();
    private static readonly long UnixMsMax = DateTimeOffset.MaxValue.ToUnixTimeMilliseconds();

    // ---------- разбор ответа: все имена полей API — только здесь ----------

    /// <summary>Ответ api/task/{id}?include=status по документации: {"Task": {...}, "Statuses": [{"Id", "Name"}]}.
    /// Поля заявки: Id, Name, Description, StatusId, StatusName, Creator, CreatorPhone, CreatorEmail, Executors,
    /// ExecutorGroup, Changed и для выгрузки — ExtraOf. Если обёртки Task нет — читаем корень.</summary>
    internal static IntraserviceTask? Parse(string json, int id)
    {
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        if (root.ValueKind != JsonValueKind.Object) return null;
        var task = Prop(root, "Task") is { ValueKind: JsonValueKind.Object } t ? t : root;
        // заявка — с номером или названием; ни того ни другого — не заявка (ошибка сервера: {"Message": …}). Без названия
        // (из почты без темы) — «Заявка #N», как в списке: иначе её не выгрузить
        var number = Int(task, "Id") is > 0 and var given ? given : (int?)null;   // 0 — не номер заявки
        if (number is null && Str(task, "Name") is null) return null;
        // название по умолчанию — по запрошенному номеру: по нему доска узнаёт, что название ещё можно заменить настоящим
        var name = Str(task, "Name")?.Trim() is { Length: > 0 } n ? n : AutoTitle(id);

        return new(number ?? id, name, StatusOf(task, root), HtmlToText(Str(task, "Description")),
            Field(task, "Creator"), Names(task, "Executors"), Field(task, "ExecutorGroup"), Date(task, "Changed"),
            Field(task, "CreatorPhone"), Field(task, "CreatorEmail"), ExtraOf(task), LegacyText(Str(task, "Description")),
            Date(task, "Created"));
    }

    /// <summary>Сервис, тип, категории и фактическая дата решения — для выгрузки и списка результатов. Имена — по живому
    /// ответу api/task/{id} (2026-10-02): ServiceName, Type (не TypeName), Categories, ResolutionDateFact. В строках списка
    /// названия сервиса нет, только ServiceId (док., стр. 11): название берём из блока Services (include=service) —
    /// services. Ни одного поля нет — null.</summary>
    private static IntraserviceExtra? ExtraOf(JsonElement t, IReadOnlyDictionary<int, string>? services = null)
    {
        var service = Field(t, "ServiceName") is { Length: > 0 } named ? named
            : Int(t, "ServiceId") is int serviceId ? services?.GetValueOrDefault(serviceId) : null;
        var x = new IntraserviceExtra(service, Field(t, "Type"), Names(t, "Categories"), Date(t, "ResolutionDateFact"));
        return x is { Service: null, Type: null, Categories: null, Resolved: null } ? null : x;
    }

    /// <summary>Названия сервисов из блока Services ответа со списком заявок (include=service): номер → название.</summary>
    private static Dictionary<int, string>? ServiceNames(JsonElement? blocks)
    {
        if (blocks is not { ValueKind: JsonValueKind.Object } b || Prop(b, "Services") is not { ValueKind: JsonValueKind.Array } list)
            return null;
        var names = new Dictionary<int, string>();
        foreach (var s in Refs(list)) names[s.Id] = s.Name;   // блок тот же, что у ParseTaskServices, — и чтение одно
        return names;
    }

    /// <summary>Ответ справочника — сервисов (api/service), типов заявки (api/tasktype), сохранённых фильтров (api/filter),
    /// сотрудников (api/user), док., стр. 33-37, 53, 60: {"ServiceList": {"Services": [...], "Paginator": {...}}}; так же терпим
    /// {"Services": [...]} и голый массив (так, скорее всего, отвечает api/filter). Поля строки: Id, Name, Path (сервисы),
    /// IsArchive (сервисы, типы), IsDefault (фильтры). Строка без номера или названия бесполезна — пропускаем её. Total —
    /// Paginator.Count (нет его — сколько пришло), HasMore — есть ли следующая страница.</summary>
    internal static (IReadOnlyList<IntraserviceRef> Items, int Total, bool HasMore)? ParseRefs(string json, string name, string wrapper)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, name, wrapper) is not { } u) return null;

        var items = Refs(u.Rows);
        var total = Paginator(u.Blocks) is { } p && Int(p, "Count") is int count ? count : items.Count;
        return (items, total, PageInfo(u.Blocks) is { } pages && pages.Page < pages.Pages);
    }

    /// <summary>Строки справочника; без номера или названия — пропуск.</summary>
    private static List<IntraserviceRef> Refs(JsonElement rows) =>
        rows.EnumerateArray()
            .Where(r => r.ValueKind == JsonValueKind.Object && Int(r, "Id") is not null && Str(r, "Name")?.Trim() is { Length: > 0 })
            .Select(r => new IntraserviceRef(Int(r, "Id")!.Value, Str(r, "Name")!.Trim(), Str(r, "Path")?.Trim(),
                Bool(r, "IsArchive") ?? false, Bool(r, "IsDefault") ?? false))
            .ToList();

    /// <summary>Сервисы из блока Services списка заявок (include=service, док., стр. 16-17: сервисы заявок этой страницы)
    /// и сколько заявок на странице. Блока нет — сервисов нет (пустой список); ответ не список заявок или голый массив
    /// (блоков у него не бывает, а строки его — заявки, не сервисы) — null.</summary>
    internal static (IReadOnlyList<IntraserviceRef> Services, int Rows)? ParseTaskServices(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, "Tasks", "TaskList") is not { Blocks: { } blocks } u) return null;   // голый массив — без блоков
        var services = Prop(blocks, "Services") is { ValueKind: JsonValueKind.Array } list ? Refs(list) : new List<IntraserviceRef>();
        return (services, u.Rows.GetArrayLength());
    }

    /// <summary>Ответ api/tasklifetime?include=status: {"TaskLifetimeList": {"TaskLifetimes": [...], "Statuses": [...],
    /// "Paginator": {...}}}. Терпим и обёртку попроще ({"TaskLifetimes": [...]}), и голый массив — json-формы в
    /// документации нет, там xml. Поля записи: Date, Editor, EditorId, StatusId, Comments, IsPublic.</summary>
    internal static (IReadOnlyList<IntraserviceEvent> Events, bool HasMore, bool Paged)? ParseLifetime(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, "TaskLifetimes", "TaskLifetimeList") is not { } u) return null;

        var events = new List<IntraserviceEvent>();
        foreach (var e in u.Rows.EnumerateArray())
            if (e.ValueKind == JsonValueKind.Object)
                events.Add(new(Date(e, "Date"), Str(e, "Editor")?.Trim() ?? "", StatusOf(e, u.Blocks),
                    HtmlToText(Str(e, "Comments")), Bool(e, "IsPublic"), Int(e, "EditorId")));

        var pages = PageInfo(u.Blocks);
        return (events, pages is { } p && p.Page < p.Pages, pages is not null);
    }

    /// <summary>Ответ api/task?search=…&amp;include=status: {"TaskList": {"Tasks": [...], "Statuses": [...],
    /// "Paginator": {...}}} — так же терпим {"Tasks": [...]} и голый массив. Поля строки: Id, Name, StatusId, Created,
    /// Creator, CreatorPhone, CreatorEmail, Description, Executors, ExecutorGroup, Changed, а для выгрузки ещё ExtraOf (сервиса
    /// и типа живой сервер в списке не присылает). Строка без номера или названия бесполезна — пропускаем её, а не весь ответ.</summary>
    internal static (IReadOnlyList<IntraserviceFound> Found, int Total, bool? HasNext)? ParseSearch(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, "Tasks", "TaskList") is not { } u) return null;

        var services = ServiceNames(u.Blocks);
        var found = new List<IntraserviceFound>();
        foreach (var t in u.Rows.EnumerateArray())
            // без названия (заявка из почты без темы) — «Заявка #N», как на доске, а не пропуск: автовыгрузка иначе прошла бы мимо
            if (t.ValueKind == JsonValueKind.Object && Int(t, "Id") is int id)
                found.Add(new(id, Str(t, "Name")?.Trim() is { Length: > 0 } name ? name : AutoTitle(id), StatusOf(t, u.Blocks), Field(t, "Creator"), Date(t, "Created"),
                    HtmlToText(Str(t, "Description")), Names(t, "Executors"), Field(t, "ExecutorGroup"), Date(t, "Changed"),
                    Field(t, "CreatorPhone"), Field(t, "CreatorEmail"), ExtraOf(t, services),
                    CreatorId: Int(t, "CreatorId"), ExecutorIds: Ids(t, "ExecutorIds"), CutDescription: LegacyText(Str(t, "Description"))));

        // общее число совпадений знает Paginator; нет его — знаем только то, что пришло. Счёт не заказан (count=false) — вместо
        // него HasNextPage (док., стр. 14): в Paginator или рядом со списком
        var paginator = Paginator(u.Blocks);
        var total = paginator is { } p && Int(p, "Count") is int count ? count : found.Count;
        var hasNext = (paginator is { } q ? Bool(q, "HasNextPage") : null)
            ?? (u.Blocks is { ValueKind: JsonValueKind.Object } b ? Bool(b, "HasNextPage") : null);
        return (found, total, hasNext);
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

    /// <summary>Номера в поле (ExecutorIds): строка «11, 12» (так в живой карточке, 2026-10-02), массив чисел или строк с
    /// числами. Поля нет — null (тогда «моё ли» сверяет имена); пусто или json null — пустой список; не число — пропускаем.</summary>
    private static IReadOnlyList<int>? Ids(JsonElement e, string name)
    {
        IEnumerable<string?>? parts = Prop(e, name) switch
        {
            { ValueKind: JsonValueKind.String } v => v.GetString()!.Split(',', ';'),
            { ValueKind: JsonValueKind.Array } a => a.EnumerateArray().Select(i => i.ValueKind switch
            {
                JsonValueKind.Number => i.GetRawText(),
                JsonValueKind.String => i.GetString(),
                _ => null,
            }).ToList(),
            { ValueKind: JsonValueKind.Null } => Array.Empty<string>(),
            _ => null,
        };
        return parts?.Select(p => int.TryParse(p?.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var n) ? n : (int?)null)
            .OfType<int>().ToList();
    }

    private static readonly Regex UnknownStatus = new(@"^статус \d+$", RegexOptions.Compiled);

    /// <summary>Статус разобран в название: не пусто и не заглушка «статус N» (StatusOf не нашёл имени по номеру). Только
    /// по такому можно решать «закрыта заявка или открыта» — заглушка не совпадёт ни с одним закрытым названием.</summary>
    public static bool IsResolvedStatus(string? status) => !string.IsNullOrWhiteSpace(status) && !UnknownStatus.IsMatch(status);

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

    /// <summary>Номер страницы и число страниц из Paginator. null — блока нет или в нём нет обоих чисел: тогда он
    /// о следующей странице ничего не говорит, и судить о ней приходится по длине страницы.</summary>
    private static (int Page, int Pages)? PageInfo(JsonElement? blocks) =>
        Paginator(blocks) is { } p && Int(p, "Page") is int page && Int(p, "PageCount") is int pages ? (page, pages) : null;

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

    /// <summary>Описание и комментарий — текстом. Html из редактора или письма (узнаётся по разметке — IsHtml) — его видимый
    /// текст (HtmlText). Простой текст (так их шлёт живой сервер, так приходит почта) — как есть: «&lt;» и «&gt;» в нём — знаки
    /// («остаток &lt; 5 шт, заказ &gt; 10», коды маркировки), и снятый «тег» молча унёс бы текст между ними. Сущности — раскрыть
    /// в обоих.</summary>
    private static string? HtmlToText(string? html) =>
        string.IsNullOrWhiteSpace(html) ? null : IsHtml(html) ? HtmlText(html) : Tidy(html);

    /// <summary>Html узнаётся по элементу известного тега — открывающий и такой же закрывающий (тот — в одной строке), по
    /// &lt;br&gt; или &lt;img …&gt;: в простом тексте их не бывает. Одиночный «тег» — бывает: код маркировки («&lt;p/7z&gt;»,
    /// «&lt;b&gt;», «&lt;TTCx&gt;»), почта «&lt;i.ivanov@corp.ru&gt;», упомянутый или процитированный тег («&lt;div class=…&gt; не
    /// закрыт», «лишний &lt;/td&gt;»), — и html-разбор снял бы куски кодов и свёл строки в одну. Линейно: имена закрывающих, затем —
    /// есть ли среди открывающих такое же (поиск пары от каждого открывающего перебирал бы все закрывающие после него).
    /// ponytail: html из одних открывающих тегов без &lt;br&gt; («&lt;p&gt;a&lt;p&gt;b») — как есть; редакторы закрывают теги.</summary>
    private static bool IsHtml(string text)
    {
        if (HtmlBrOrImg.IsMatch(text)) return true;
        var lastClose = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);   // элемент — открывающий раньше закрывающего
        foreach (Match m in HtmlCloseTags.Matches(text)) lastClose[m.Groups[1].Value] = m.Index;
        return lastClose.Count > 0 && HtmlOpenTags.Matches(text).Any(m => lastClose.TryGetValue(m.Groups[1].Value, out var end) && m.Index < end);
    }

    /// <summary>Текст без html-комментариев (и условных блоков Outlook). Линейно: незакрытый комментарий — значит, дальше
    /// закрывающего нет ни у одного, и остаток — текст (у «&lt;!--.*?--&gt;» каждый незакрытый читал бы текст до конца).</summary>
    private static string WithoutComments(string text)
    {
        var result = new StringBuilder(text.Length);
        var done = 0;
        for (int at; (at = text.IndexOf("<!--", done, StringComparison.Ordinal)) >= 0;)
        {
            var end = text.IndexOf("-->", at + 4, StringComparison.Ordinal);
            if (end < 0) break;
            result.Append(text, done, at - done);
            done = end + 3;
        }
        return result.Append(text, done, text.Length - done).ToString();
    }

    /// <summary>Блок — открывающий из start и такой же закрывающий — остаётся как есть (keep) или снимается, текст между блоками
    /// — через between (нет его — как есть). Линейно: имя,
    /// чьего закрывающего дальше нет, больше не ищется (у «.*?&lt;/\1» каждый незакрытый открывающий читал текст до конца).</summary>
    private static string Blocks(string text, Regex start, bool keep, Func<string, string>? between = null)
    {
        var result = new StringBuilder(text.Length);
        var unclosed = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        int done = 0, from = 0;
        for (Match m; (m = start.Match(text, from)).Success;)
        {
            from = m.Index + m.Length;
            var name = m.Groups[1].Value;
            if (unclosed.Contains(name)) continue;
            if (BlockEnd(name).Match(text, from) is not { Success: true } end) { unclosed.Add(name); continue; }
            Between(done, m.Index);
            if (keep) result.Append(text, m.Index, end.Index + end.Length - m.Index);
            done = from = end.Index + end.Length;
        }
        Between(done, text.Length);
        return result.ToString();

        void Between(int at, int to)
        {
            if (between is null) result.Append(text, at, to - at);
            else result.Append(between(text[at..to]));
        }
    }

    /// <summary>Видимый текст html, как его показывает браузер: комментарии (и условные Outlook), затем стили, скрипты и
    /// заголовок документа — долой; блок (абзац, пункт, строка таблицы, заголовок) — с новой строки, ячейки — через пробел;
    /// разметка снята (HtmlTags). Есть абзацы или таблица — пробелы и переводы строк исходника сводятся к пробелу, кроме
    /// &lt;pre&gt; (HtmlBlocks). ponytail: в &lt;pre&gt; сохраняются строки, но не отступы и не пустые строки подряд — для трассы
    /// стека на странице ошибки хватает; код с отступами в html-заявке не встречали.</summary>
    private static string HtmlText(string html)
    {
        var text = Blocks(WithoutComments(html), HtmlNoiseStart, keep: false);   // сначала комментарии: «<style>» в комментарии — не стиль
        var blocks = HtmlBlocks.IsMatch(text);
        if (blocks) text = Blocks(text, HtmlPreStart, keep: true, t => HtmlSpace.Replace(t, " "));
        text = HtmlCells.Replace(HtmlCellStarts.Replace(text, ""), " ");
        text = HtmlTags.Replace(HtmlBreaks.Replace(text, "\n"), "").Replace("\r", "");
        return Tidy(blocks ? LineSpaces.Replace(SpaceRuns.Replace(text, " "), "\n") : LineEndSpaces.Replace(text, "\n"));
    }

    /// <summary>Html-страница ошибки (IIS, прокси, вход) для сообщения: её заголовок («401 - Unauthorized» — подпись окна, но
    /// причину часто называет только он) и видимый текст.</summary>
    private static string PageText(string html)
    {
        var bare = WithoutComments(html);   // заголовок в комментарии — не заголовок
        var title = HtmlTitleStart.Match(bare) is { Success: true } m && BlockEnd("title").Match(bare, m.Index + m.Length) is { Success: true } e
            ? Tidy(HtmlSpace.Replace(bare[(m.Index + m.Length)..e.Index], " ")) : "";
        var text = HtmlText(html);
        // с заголовка начинается одна из первых строк («401 - Unauthorized: …») — второй раз не нужен; «Входящие» — не «Вход»,
        // а глубже в тексте его отрезал бы предел сообщения
        var repeated = text.Split('\n').Take(3).Any(line => line.StartsWith(title, StringComparison.Ordinal)
            && (line.Length == title.Length || !char.IsLetterOrDigit(line[title.Length])));
        return title.Length == 0 || repeated ? text : text.Length == 0 ? title : $"{title}\n{text}";
    }

    private static string Tidy(string text) =>
        ManyNewlines.Replace(WhiteLines.Replace(WebUtility.HtmlDecode(text).Replace("\r", ""), ""), "\n\n").Trim();
}
