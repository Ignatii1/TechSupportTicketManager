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
public sealed partial class HttpIntraserviceClient
{
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(10) };
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
        catch (JsonException) { Unparsed(json); return $"ответ не похож на API Интрасервиса:\n{Evidence(json)}"; }
    }

    /// <summary>Куда писать ответы, которые не удалось разобрать (App подключает errors.log). Форма json у половины
    /// методов в документации не показана вовсе — такой ответ и есть то, что нужно, чтобы починить разбор.</summary>
    public static Action<string>? LogUnparsed { get; set; }
    private static readonly ConcurrentDictionary<string, byte> LoggedCalls = new();

    /// <summary>Ответ пришёл, но не разобрался: его начало — в лог вместе с именем метода, в UI — короткая строка.
    /// Логина и пароля в теле нет (они в заголовке Authorization); имена и тексты заявок — есть, лог лежит рядом с exe.</summary>
    private static string Unparsed(string json, [CallerMemberName] string call = "")
    {
        // один образец на метод за запуск: F5 по двумстам карточкам дал бы двести одинаковых записей в лог без ротации.
        // Вызывается из пула потоков (ConfigureAwait(false)), отсюда потокобезопасный словарь.
        if (LoggedCalls.TryAdd(call, 0)) LogUnparsed?.Invoke($"{call}: не разобран ответ сервера ({json.Length} симв.):\n{Head(Redact(json), LoggedChars)}");
        return $"непонятный ответ сервера:\n{Evidence(json)}";
    }

    private async Task<(string? Json, string Error)> GetAsync(string path, string notFound, CancellationToken ct,
        [CallerMemberName] string call = "")
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, $"{_base}/{path}");
        req.Headers.Authorization = _auth;
        req.Headers.Accept.ParseAdd("application/json");
        try
        {
            using var resp = await Http.SendAsync(req, ct).ConfigureAwait(false);
            if (resp.IsSuccessStatusCode) return (await resp.Content.ReadAsStringAsync(ct).ConfigureAwait(false), "");

            // первая строка — по-русски и коротко, дальше — что сервер ответил на самом деле: гадать по пересказу хуже
            var code = (int)resp.StatusCode;
            var what = resp.StatusCode switch
            {
                HttpStatusCode.Unauthorized => "неверный логин или пароль",
                HttpStatusCode.Forbidden => "нет доступа",
                HttpStatusCode.NotFound => notFound,
                _ => "ошибка сервера",
            };
            var body = "";
            try { body = await resp.Content.ReadAsStringAsync(ct).ConfigureAwait(false); }
            catch (Exception e) when (e is HttpRequestException or IOException) { /* тело не дочитали — хватит и кода */ }
            // страница-трассировка прокси или IIS может повторить заголовки запроса — вместе с нашим логином и паролем
            body = body.Replace(_auth.Parameter!, "***");
            if (string.IsNullOrWhiteSpace(body)) return (null, $"{what} (HTTP {code})");
            if (LoggedCalls.TryAdd($"{call} HTTP {code}", 0))
                LogUnparsed?.Invoke($"{call}: HTTP {code} на {path} ({body.Length} симв.):\n{Head(Redact(body), LoggedChars)}");
            return (null, $"{what} (HTTP {code}):\n{Evidence(body)}");
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested) { return (null, "сервер не ответил за 10 с"); }
        catch (HttpRequestException e) { return (null, $"сервер недоступен:\n{Reason(e)}"); }
        catch (IOException e) { return (null, $"соединение оборвалось:\n{Reason(e)}"); }
    }

    /// <summary>Сколько сырого ответа показываем в самом сообщении и сколько кладём в errors.log.</summary>
    private const int ShownChars = 1000, LoggedChars = 4000;

    private static readonly Regex HtmlNoise = new(@"<(style|script)\b[^>]*>.*?</\1\s*>",
        RegexOptions.IgnoreCase | RegexOptions.Singleline | RegexOptions.Compiled);
    // конец заголовка, строки таблицы и прочих блоков — перевод строки, иначе «401 - UnauthorizedServer Error» в одну строку
    private static readonly Regex HtmlBlockEnds = new(@"</(title|h[1-6]|tr|th|td|pre|header|section)\s*>",
        RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex BlankLines = new(@"\n[ \t]+(?=\n)|[ \t]+$", RegexOptions.Multiline | RegexOptions.Compiled);
    private static readonly Regex BasicToken = new(@"(\bBasic\s+)[A-Za-z0-9+/=]{6,}", RegexOptions.IgnoreCase | RegexOptions.Compiled);

    /// <summary>Любой Basic-токен в тексте — звёздочками: из base64 логин и пароль достаются за секунду.</summary>
    private static string Redact(string text) => BasicToken.Replace(text, "$1***");

    /// <summary>Коротко, в одну строку: фраза и первая строка подробностей — для мест, где под ошибку одна строка
    /// (подпись в быстром добавлении, вопрос «перенести в Готово?»). Целиком ошибка — в панели и в errors.log.</summary>
    public static string Brief(string error)
    {
        var lines = error.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        return lines.Length > 1 ? $"{lines[0].TrimEnd(':')} — {lines[1]}" : lines.FirstOrDefault() ?? "";
    }

    /// <summary>Тело ответа для сообщения об ошибке. json и xml — как есть, байт в байт. Html-страницу (ошибка IIS,
    /// страница прокси или входа) — её видимым текстом, с пометкой: сырой она начинается с килобайта стилей,
    /// и причины на экране не видно. В лог в любом случае уходит сырое.</summary>
    internal static string Evidence(string body)
    {
        var raw = Redact(body.Trim());
        var text = raw.StartsWith('<') && raw.Contains("<html", StringComparison.OrdinalIgnoreCase)
            ? "(html-страница, показан её текст)\n"
              + BlankLines.Replace(HtmlToText(HtmlBlockEnds.Replace(HtmlNoise.Replace(raw, ""), "<br>")) ?? "", "")
            : raw;
        return Head(text, ShownChars);
    }

    private static string Head(string text, int max) => text.Length > max ? text[..max] + "\n…" : text;

    /// <summary>Причина сетевой ошибки по цепочке исключений: «The SSL connection could not be established → The remote
    /// certificate is invalid…» — верхнее сообщение одно почти ничего не говорит. Пароля в них нет.</summary>
    private static string Reason(Exception e)
    {
        var parts = new List<string>();
        for (Exception? x = e; x is not null && parts.Count < 4; x = x.InnerException)
            if (!parts.Contains(x.Message)) parts.Add(x.Message);
        return string.Join(" → ", parts);
    }

}
