using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

public sealed record IntraserviceTask(int Id, string Name, string Status, string? Description);

/// <summary>Заявка или короткое описание ошибки для UI («заявка не найдена», «сервер недоступен»). Секретов в тексте нет.</summary>
public sealed record IntraserviceResult(IntraserviceTask? Task, string Error);

/// <summary>Событие жизненного цикла заявки. Comment — null, если это просто смена статуса без комментария
/// (обычное дело, а не ошибка разбора); IsPublic — null, если сервер признак не прислал.</summary>
public sealed record IntraserviceEvent(DateTimeOffset? Date, string Author, string Status, string? Comment, bool? IsPublic);

/// <summary>Лента событий заявки: записи, признак «есть ещё страницы» и короткое описание ошибки для UI.</summary>
public sealed record IntraserviceLifetime(IReadOnlyList<IntraserviceEvent> Events, bool HasMore, string Error);

/// <summary>Найденная на сервере заявка (поиск идёт и по полям заявки, и по всем её комментариям).
/// Description — описание без html; null, если сервер его не прислал.</summary>
public sealed record IntraserviceFound(int Id, string Name, string Status, string? Creator, DateTimeOffset? Created,
    string? Description = null);

/// <summary>Результат поиска или страница списка заявок: строки (не больше страницы), общее их число
/// и описание ошибки для UI.</summary>
public sealed record IntraserviceSearchResult(IReadOnlyList<IntraserviceFound> Found, int Total, string Error);

/// <summary>Статус заявки (док., стр. 38): номер, название и два признака закрытости — «Заявка выполнена»
/// (IsFixed) и «Конечный» (IsFinal).</summary>
public sealed record IntraserviceStatus(int Id, string Name, bool IsFixed, bool IsFinal);

/// <summary>REST API Интрасервиса (IntraService API v5.42): базовая авторизация логином и паролем пользователя,
/// GET {адрес}/api/task/{номер}, ответ в JSON по заголовку Accept.</summary>
public sealed class HttpIntraserviceClient
{
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(10) };
    private static readonly Regex HtmlBreaks = new(@"<br\s*/?>|</p>|</div>|</li>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex HtmlTags = new(@"<[^>]*>", RegexOptions.Compiled);
    private static readonly Regex ManyNewlines = new(@"\n\s*\n\s*\n+", RegexOptions.Compiled);
    private static readonly Regex WcfDate = new(@"^/Date\((-?\d+)(?:[+-]\d{4})?\)/$", RegexOptions.Compiled);
    private static readonly string[] DateFormats = { "dd.MM.yyyy HH:mm:ss", "dd.MM.yyyy HH:mm", "dd.MM.yyyy" };
    // границы DateTimeOffset в миллисекундах от 1970-01-01 UTC: за ними FromUnixTimeMilliseconds бросает исключение
    private static readonly long UnixMsMin = DateTimeOffset.MinValue.ToUnixTimeMilliseconds();
    private static readonly long UnixMsMax = DateTimeOffset.MaxValue.ToUnixTimeMilliseconds();
    // пустые списки для ответов с ошибкой
    private static readonly IntraserviceEvent[] NoEvents = Array.Empty<IntraserviceEvent>();
    private static readonly IntraserviceFound[] NoFound = Array.Empty<IntraserviceFound>();
    private static readonly IntraserviceStatus[] NoStatuses = Array.Empty<IntraserviceStatus>();

    private readonly string _base;
    private readonly AuthenticationHeaderValue _auth;

    public HttpIntraserviceClient(string baseUrl, string login, string password)
    {
        _base = baseUrl.Trim().TrimEnd('/');
        _auth = new("Basic", Convert.ToBase64String(Encoding.UTF8.GetBytes($"{login}:{password}")));
    }

    /// <summary>Клиент по настройкам; без адреса или логина/пароля — null (API выключен).</summary>
    public static HttpIntraserviceClient? From(AppSettings s) =>
        IsValidUrl(s.IntraserviceBaseUrl) && s.IntraserviceLogin.Trim().Length > 0 && s.IntraservicePassword.Length > 0
            ? new HttpIntraserviceClient(s.IntraserviceBaseUrl, s.IntraserviceLogin.Trim(), s.IntraservicePassword)
            : null;

    public static bool IsValidUrl(string url) =>
        Uri.TryCreate(url.Trim(), UriKind.Absolute, out var u) && (u.Scheme == Uri.UriSchemeHttps || u.Scheme == Uri.UriSchemeHttp);

    public static bool IsHttp(string url) => url.Trim().StartsWith("http://", StringComparison.OrdinalIgnoreCase);

    public async Task<IntraserviceResult> GetTaskAsync(int id, CancellationToken ct = default)
    {
        var (json, error) = await GetAsync($"api/task/{id}?include=status", "заявка не найдена", ct).ConfigureAwait(false);
        if (json is null) return new(null, error);
        try { return Parse(json, id) is { } task ? new(task, "") : new(null, Unparsed(json)); }
        catch (JsonException) { return new(null, Unparsed(json)); }
    }

    /// <summary>Жизненный цикл заявки (док., стр. 65): комментарии и смены статуса, последние сверху, не больше 50 записей.
    /// Пустая лента с текстом ошибки — обычный ответ, исключений наружу нет.</summary>
    public async Task<IntraserviceLifetime> GetLifetimeAsync(int id, CancellationToken ct = default)
    {
        var (json, error) = await GetAsync($"api/tasklifetime?taskid={id}&include=status&lastcommentsontop=true&pagesize=50",
            "заявка не найдена", ct).ConfigureAwait(false);
        if (json is null) return new(NoEvents, false, error);
        try
        {
            return ParseLifetime(json) is { } r ? new(r.Events, r.HasMore, "") : new(NoEvents, false, Unparsed(json));
        }
        catch (JsonException) { return new(NoEvents, false, Unparsed(json)); }
    }

    /// <summary>Поиск заявок на сервере (док., стр. 15): строка ищется в полях заявки и во всех её комментариях.
    /// Отдаём первые 20 совпадений, свежие сверху, и общее их число.</summary>
    public async Task<IntraserviceSearchResult> SearchAsync(string text, CancellationToken ct = default)
    {
        // ponytail: без fields — ответ жирнее, зато не упадёт на незнакомом имени поля; появится нужда экономить трафик — добавить fields и проверить на живом сервере.
        var (json, error) = await GetAsync($"api/task?search={Uri.EscapeDataString(text)}&include=status&sort=Changed%20desc&pagesize=20",
            "ничего не найдено", ct).ConfigureAwait(false);
        if (json is null) return new(NoFound, 0, error);
        try
        {
            return ParseSearch(json) is { } r ? new(r.Found, r.Total, "") : new(NoFound, 0, Unparsed(json));
        }
        catch (JsonException) { return new(NoFound, 0, Unparsed(json)); }
    }

    /// <summary>Номер текущего пользователя (док., стр. 56-57): GET api/user?getcurrentuserinfo=true.
    /// Нужен импорту, чтобы отобрать заявки, где исполнитель — он. Ошибка — короткая строка, исключений наружу нет.</summary>
    public async Task<(int? Id, string Error)> GetCurrentUserIdAsync(CancellationToken ct = default)
    {
        var (json, error) = await GetAsync("api/user?getcurrentuserinfo=true", "не удалось определить пользователя", ct).ConfigureAwait(false);
        if (json is null) return (null, error);
        try
        {
            if (ParseCurrentUserId(json) is { } id) return (id, "");
            return (null, Unparsed(json));
        }
        catch (JsonException) { return (null, Unparsed(json)); }
    }

    /// <summary>Все статусы заявок (док., стр. 38-39): GET api/taskstatus. По признакам «Заявка выполнена»
    /// и «Конечный» импорт решает, какие статусы считать открытыми. При ошибке список пустой.</summary>
    public async Task<(IReadOnlyList<IntraserviceStatus> Statuses, string Error)> GetStatusesAsync(CancellationToken ct = default)
    {
        var (json, error) = await GetAsync("api/taskstatus", "по этому адресу нет API", ct).ConfigureAwait(false);
        if (json is null) return (NoStatuses, error);
        try
        {
            if (ParseStatuses(json) is { } statuses) return (statuses, "");
            return (NoStatuses, Unparsed(json));
        }
        catch (JsonException) { return (NoStatuses, Unparsed(json)); }
    }

    /// <summary>Страница заявок, на которых пользователь — исполнитель (док., стр. 19-20: фильтры ExecutorIds
    /// и StatusIds, оба — номера через запятую). Страницы считаются с первой. Ответ той же формы, что и у поиска
    /// (Tasks + Statuses + Paginator), поэтому разбираем его тем же ParseSearch. Пустой список статусов — не
    /// «без фильтра»: такой запрос притащил бы и закрытые заявки, поэтому это ошибка, а не запрос.</summary>
    public async Task<IntraserviceSearchResult> GetExecutorTasksAsync(int executorId, IReadOnlyCollection<int> statusIds, int page, CancellationToken ct = default)
    {
        if (statusIds.Count == 0) return new(NoFound, 0, "не задан список открытых статусов");
        var ids = string.Join(",", statusIds); // StatusIds и ExecutorIds — номера через запятую
        // ponytail: без fields — ответ жирнее, зато не упадёт на незнакомом имени поля; появится нужда экономить трафик — добавить fields и проверить на живом сервере.
        var (json, error) = await GetAsync(
            $"api/task?ExecutorIds={executorId}&StatusIds={ids}&include=status&sort=Changed%20desc&pagesize=200&page={Math.Max(1, page)}",
            "по этому адресу нет API", ct).ConfigureAwait(false);
        if (json is null) return new(NoFound, 0, error);
        try
        {
            return ParseSearch(json) is { } r ? new(r.Found, r.Total, "") : new(NoFound, 0, Unparsed(json));
        }
        catch (JsonException) { return new(NoFound, 0, Unparsed(json)); }
    }

    /// <summary>Проверка адреса и логина: список статусов маленький. "" — всё хорошо.</summary>
    public async Task<string> CheckAsync(CancellationToken ct = default)
    {
        var (json, error) = await GetAsync("api/taskstatus", "по этому адресу нет API", ct).ConfigureAwait(false);
        if (json is null) return error;
        try { using var _ = JsonDocument.Parse(json); return ""; }
        catch (JsonException) { Unparsed(json); return "ответ не похож на API Интрасервиса"; }
    }

    /// <summary>Куда писать ответы, которые не удалось разобрать (App подключает errors.log). Форма json у половины
    /// методов в документации не показана вовсе — такой ответ и есть то, что нужно, чтобы починить разбор.</summary>
    public static Action<string>? LogUnparsed { get; set; }
    private static readonly ConcurrentDictionary<string, byte> LoggedCalls = new();

    /// <summary>Ответ пришёл, но не разобрался: его начало — в лог вместе с именем метода, в UI — короткая строка.
    /// Логина и пароля в теле нет (они в заголовке Authorization); имена и тексты заявок — есть, лог лежит рядом с exe.</summary>
    private static string Unparsed(string json, [CallerMemberName] string call = "")
    {
        const int head = 4000;
        // один образец на метод за запуск: F5 по двумстам карточкам дал бы двести одинаковых записей в лог без ротации.
        // Вызывается из пула потоков (ConfigureAwait(false)), отсюда потокобезопасный словарь.
        if (LoggedCalls.TryAdd(call, 0)) LogUnparsed?.Invoke($"{call}: не разобран ответ сервера ({json.Length} симв.):\n"
            + (json.Length > head ? json[..head] + "\n…" : json));
        return "непонятный ответ сервера";
    }

    private async Task<(string? Json, string Error)> GetAsync(string path, string notFound, CancellationToken ct)
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, $"{_base}/{path}");
        req.Headers.Authorization = _auth;
        req.Headers.Accept.ParseAdd("application/json");
        try
        {
            using var resp = await Http.SendAsync(req, ct).ConfigureAwait(false);
            if (resp.IsSuccessStatusCode) return (await resp.Content.ReadAsStringAsync(ct).ConfigureAwait(false), "");
            return (null, resp.StatusCode switch
            {
                HttpStatusCode.Unauthorized => "неверный логин или пароль",
                HttpStatusCode.Forbidden => "нет доступа",
                HttpStatusCode.NotFound => notFound,
                var c => $"ошибка сервера ({(int)c})",
            });
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested) { return (null, "сервер не ответил за 10 с"); }
        catch (HttpRequestException) { return (null, "сервер недоступен"); }
    }

    // ---------- разбор ответа: все имена полей API — только здесь ----------

    /// <summary>Ответ api/task/{id}?include=status по документации: {"Task": {...}, "Statuses": [{"Id", "Name"}]}.
    /// Поля заявки: Id, Name, Description, StatusId, StatusName. Если обёртки Task нет — читаем корень.</summary>
    internal static IntraserviceTask? Parse(string json, int id)
    {
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        if (root.ValueKind != JsonValueKind.Object) return null;
        var task = Prop(root, "Task") is { ValueKind: JsonValueKind.Object } t ? t : root;
        if (Str(task, "Name") is not { } name) return null;

        return new(Int(task, "Id") ?? id, name.Trim(), StatusOf(task, root), HtmlToText(Str(task, "Description")));
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
                    HtmlToText(Str(e, "Comments")), Bool(e, "IsPublic")));

        return (events, HasNextPage(u.Blocks));
    }

    /// <summary>Ответ api/task?search=…&amp;include=status: {"TaskList": {"Tasks": [...], "Statuses": [...],
    /// "Paginator": {...}}} — так же терпим {"Tasks": [...]} и голый массив. Поля строки: Id, Name, StatusId, Created,
    /// Creator, Description. Строка без номера или названия бесполезна — пропускаем её, а не весь ответ.</summary>
    internal static (IReadOnlyList<IntraserviceFound> Found, int Total)? ParseSearch(string json)
    {
        using var doc = JsonDocument.Parse(json);
        if (Unwrap(doc.RootElement, "Tasks", "TaskList") is not { } u) return null;

        var found = new List<IntraserviceFound>();
        foreach (var t in u.Rows.EnumerateArray())
            if (t.ValueKind == JsonValueKind.Object && Int(t, "Id") is int id && Str(t, "Name")?.Trim() is { Length: > 0 } name)
                found.Add(new(id, name, StatusOf(t, u.Blocks), Str(t, "Creator")?.Trim(), Date(t, "Created"),
                    HtmlToText(Str(t, "Description"))));

        // общее число совпадений знает Paginator; нет его — знаем только то, что пришло
        var total = Paginator(u.Blocks) is { } p && Int(p, "Count") is int count ? count : found.Count;
        return (found, total);
    }

    /// <summary>Ответ api/user?getcurrentuserinfo=true (док., стр. 57): объект с полями Id, Login, Name, RoleType
    /// и прочими. В документации это xml с корнем &lt;CurrenUserInfo&gt; — буква «t» потеряна в самой документации,
    /// поэтому понимаем оба написания обёртки и голый объект. Нужен только Id; нет его — null.</summary>
    internal static int? ParseCurrentUserId(string json)
    {
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;
        if (root.ValueKind != JsonValueKind.Object) return null;

        var wrapper = Prop(root, "CurrenUserInfo") ?? Prop(root, "CurrentUserInfo");
        var id = Int(wrapper is { ValueKind: JsonValueKind.Object } w ? w : root, "Id");
        return id > 0 ? id : null; // нулевой номер — тоже не пользователь
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

    /// <summary>ponytail: самопроверка разбора на образцах формы из документации (v5.42 и v5.51), только в Debug
    /// (вызов в App.OnStartup). Реальные ответы сервера появятся — добавить их сюда же.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        Debug.Assert(Parse("""{"Task":{"Id":159,"Name":"Принтер","Description":"<p>a &laquo;b&raquo;</p><p>c<br/>d</p>","StatusId":31},"Statuses":[{"Id":31,"Name":"Открыта"}]}""", 1)
            == new IntraserviceTask(159, "Принтер", "Открыта", "a «b»\nc\nd"));
        Debug.Assert(Parse("""{"Id":162,"Name":"D","StatusName":"Выполнена"}""", 1) is { Id: 162, Status: "Выполнена", Description: null });
        Debug.Assert(Parse("""{"Task":{"Name":"C","StatusId":56}}""", 7) is { Id: 7, Status: "статус 56" });
        Debug.Assert(Parse("""{"Message":"The request is invalid."}""", 1) is null);

        // Жизненный цикл: пример из документации (стр. 65-66), переведённый в json. Первая запись — просто смена
        // статуса, без ключа Comments; во второй комментарий и признак «виден клиенту» строкой.
        var life = ParseLifetime("""
            {"TaskLifetimeList":{"TaskLifetimes":[
              {"Date":"12.11.2015 13:44:53","EditorId":43,"Editor":"Администратор","StatusId":29},
              {"Date":"11.11.2015 15:24:10","EditorId":43,"Editor":"Администратор","StatusId":31,"Comments":"<p>Проверьте, пожалуйста</p>","IsPublic":"True"}],
              "Statuses":[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}],
              "Paginator":{"Count":2,"Page":1,"PageCount":1,"PageSize":25,"CountOnPage":2}}}
            """);
        Debug.Assert(life is not null && !life.Value.HasMore && life.Value.Events.Count == 2);
        Debug.Assert(life?.Events[0] is { Author: "Администратор", Status: "Выполнена", Comment: null, IsPublic: null });
        Debug.Assert(life?.Events[0].Date == new DateTimeOffset(new DateTime(2015, 11, 12, 13, 44, 53)));
        Debug.Assert(life?.Events[1] is { Status: "Открыта", Comment: "Проверьте, пожалуйста", IsPublic: true });
        // Голый массив, дата в ISO, статуса 7 в ответе нет, пустой комментарий — это не комментарий.
        var bare = ParseLifetime("""[{"Date":"2015-10-29T13:51:14.023","Editor":"Иванов","StatusId":7,"Comments":"","IsPublic":false}]""");
        Debug.Assert(bare is not null && !bare.Value.HasMore && bare.Value.Events.Count == 1);
        Debug.Assert(bare?.Events[0] is { Author: "Иванов", Status: "статус 7", Comment: null, IsPublic: false });
        Debug.Assert(bare?.Events[0].Date == new DateTimeOffset(new DateTime(2015, 10, 29, 13, 51, 14, 23)));
        // Дата в формате WCF — миллисекунды от 1970 UTC, тот же момент, что и в примере выше.
        Debug.Assert(ParseLifetime("""[{"Date":"/Date(1447335893000)/","Editor":"Иванов","StatusId":7}]""")?.Events[0].Date
            == new DateTimeOffset(2015, 11, 12, 13, 44, 53, TimeSpan.Zero));
        // Пустая страница, но не последняя.
        var page2 = ParseLifetime("""{"TaskLifetimes":[],"Paginator":{"Page":2,"PageCount":3}}""");
        Debug.Assert(page2 is not null && page2.Value.HasMore && page2.Value.Events.Count == 0);
        Debug.Assert(ParseLifetime("""{"Message":"The request is invalid."}""") is null);

        // Поиск: форма ответа из документации (стр. 16-17) — Tasks + Statuses + Paginator. Строка без Name пропадает,
        // общее число берём из Paginator.Count, а не из длины списка.
        var found = ParseSearch("""
            {"Tasks":[{"Id":159,"Name":"Принтер","StatusId":31,"Created":"26.11.2015 16:18:06","Creator":"Администратор"},
              {"Id":160,"StatusId":31},
              {"Id":161,"Name":"Сканер","StatusName":"В работе"}],
              "Statuses":[{"Id":31,"Name":"Открыта"}],
              "Paginator":{"Count":137,"Page":1,"PageCount":7,"PageSize":20,"CountOnPage":3}}
            """);
        Debug.Assert(found is not null && found.Value.Total == 137 && found.Value.Found.Count == 2);
        Debug.Assert(found?.Found[0] is { Id: 159, Name: "Принтер", Status: "Открыта", Creator: "Администратор" });
        Debug.Assert(found?.Found[0].Created == new DateTimeOffset(new DateTime(2015, 11, 26, 16, 18, 6)));
        Debug.Assert(found?.Found[1] is { Id: 161, Status: "В работе", Creator: null });
        // Обёртка TaskList, Paginator'а нет: общее число — сколько пришло, статуса нет вовсе — пустая строка.
        var wrapped = ParseSearch("""{"TaskList":{"Tasks":[{"Id":7,"Name":"C"}]}}""");
        Debug.Assert(wrapped is not null && wrapped.Value.Total == 1 && wrapped.Value.Found[0].Status == "");
        Debug.Assert(ParseSearch("""{"Message":"The request is invalid."}""") is null);
        // Описание в списке — тот же html из редактора, что и в карточке: чистим его так же.
        Debug.Assert(ParseSearch("""{"Tasks":[{"Id":7,"Name":"C","Description":"<p>a &laquo;b&raquo;</p><p>c<br/>d</p>"}]}""")
            ?.Found[0].Description == "a «b»\nc\nd");

        // Текущий пользователь: пример из документации (стр. 57), переведённый в json. Корень там назван
        // <CurrenUserInfo> — буква «t» потеряна в самой документации, поэтому понимаем оба написания и голый объект.
        Debug.Assert(ParseCurrentUserId("""
            {"CompanyId":30,"DefaultTaskFilterId":106,"Email":"test@test.ru","Id":1,"IsArchive":false,"Language":"ru",
             "Login":"admin","Name":"Администратор","RoleId":37,"RoleType":1,"UtcOffset":"+03:00"}
            """) == 1);
        Debug.Assert(ParseCurrentUserId("""{"CurrenUserInfo":{"Id":1,"Login":"admin","Name":"Администратор","RoleType":1}}""") == 1);
        Debug.Assert(ParseCurrentUserId("""{"CurrentUserInfo":{"Id":44,"Login":"test1"}}""") == 44);
        Debug.Assert(ParseCurrentUserId("""{"Message":"The request is invalid."}""") is null);

        // Статусы: пример из документации (стр. 39), переведённый в json. Голый массив; признаки приходят и
        // булевыми, и строкой; строка без номера пропадает, остальные читаются.
        var statuses = ParseStatuses("""
            [{"Id":31,"Name":"Открыта","IsCommentRequired":false,"IsFinal":false,"IsFixed":false,"IsInitial":true},
             {"Id":29,"Name":"Выполнена","IsFixed":"True","IsFinal":"False"},
             {"Id":30,"Name":"Закрыта","IsFinal":true},
             {"Name":"Без номера","IsFixed":true}]
            """);
        Debug.Assert(statuses is { Count: 3 });
        Debug.Assert(statuses?[0] == new IntraserviceStatus(31, "Открыта", false, false));
        Debug.Assert(statuses?[1] == new IntraserviceStatus(29, "Выполнена", true, false));
        Debug.Assert(statuses?[2] is { Id: 30, Name: "Закрыта", IsFixed: false, IsFinal: true });
        // Обёртки: из xml-документации в лоб и «как блок Statuses в ответе по заявкам».
        Debug.Assert(ParseStatuses("""{"ArrayOfTaskStatusView":{"TaskStatusView":[{"Id":7,"Name":"Открыта"}]}}""") is { Count: 1 });
        Debug.Assert(ParseStatuses("""{"Statuses":[{"Id":7,"Name":"Открыта"}]}""") is { Count: 1 });
        Debug.Assert(ParseStatuses("""{"Message":"The request is invalid."}""") is null);
    }

    private static JsonElement? Prop(JsonElement e, string name)
    {
        foreach (var p in e.EnumerateObject())
            if (p.Name.Equals(name, StringComparison.OrdinalIgnoreCase)) return p.Value;
        return null;
    }

    private static string? Str(JsonElement e, string name) => Prop(e, name) is { ValueKind: JsonValueKind.String } v ? v.GetString() : null;

    private static int? Int(JsonElement e, string name) => Prop(e, name) is { ValueKind: JsonValueKind.Number } v && v.TryGetInt32(out var n) ? n : null;

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
