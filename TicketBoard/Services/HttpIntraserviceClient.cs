using System.Buffers;
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

/// <summary>Заявка. Creator, Executors, ExecutorGroup — кто подал и кто работает («Иванов И. И., Петров П.»), CreatorPhone и
/// CreatorEmail — как с подавшим связаться: null — поля в ответе нет, пустая строка — есть, но пусто. Changed — когда
/// заявку меняли в последний раз: по нему автообновление замечает новые комментарии. Extra — сервис, тип и прочее для
/// выгрузки в базу знаний. CutDescription — что из того же текста сервера сделал разбор до 0.15.1 (LegacyText): по нему
/// доска узнаёт описание, им обрезанное, и чинит его (MainViewModel.Apply). Created — когда заявку создали (папка месяца в
/// базе знаний — «Спросить базу знаний»).</summary>
public sealed record IntraserviceTask(int Id, string Name, string Status, string? Description,
    string? Creator = null, string? Executors = null, string? ExecutorGroup = null, DateTimeOffset? Changed = null,
    string? CreatorPhone = null, string? CreatorEmail = null, IntraserviceExtra? Extra = null, string? CutDescription = null,
    DateTimeOffset? Created = null);

/// <summary>Заявка или короткое описание ошибки для UI («заявка не найдена», «сервер недоступен»). Секретов в тексте нет.</summary>
public sealed record IntraserviceResult(IntraserviceTask? Task, string Error);

/// <summary>Событие жизненного цикла заявки. Comment — null, если это просто смена статуса без комментария
/// (обычное дело, а не ошибка разбора); IsPublic — null, если сервер признак не прислал; AuthorId — номер автора
/// (EditorId), null — не прислал; Final — у статуса записи признак «выполнена» или «конечный» (блок Statuses ответа).</summary>
public sealed record IntraserviceEvent(DateTimeOffset? Date, string Author, string Status, string? Comment, bool? IsPublic,
    int? AuthorId = null, bool Final = false);

/// <summary>Лента событий заявки: записи, признак «есть ещё страницы», короткое описание ошибки для UI и Paged — прислал ли
/// сервер в Paginator номер страницы и число страниц (без них HasMore ничего не знает).</summary>
public sealed record IntraserviceLifetime(IReadOnlyList<IntraserviceEvent> Events, bool HasMore, string Error, bool Paged);

/// <summary>Найденная на сервере заявка (поиск идёт и по полям заявки, и по всем её комментариям).
/// Description — описание без html; null, если сервер его не прислал. CreatorId и ExecutorIds — номера инициатора и
/// исполнителей (для «моё ли» автовыгрузки); null — сервер их не прислал, тогда сверяются имена. CutDescription — как у
/// IntraserviceTask.</summary>
public sealed record IntraserviceFound(int Id, string Name, string Status, string? Creator, DateTimeOffset? Created,
    string? Description = null, string? Executors = null, string? ExecutorGroup = null, DateTimeOffset? Changed = null,
    string? CreatorPhone = null, string? CreatorEmail = null, IntraserviceExtra? Extra = null,
    int? CreatorId = null, IReadOnlyList<int>? ExecutorIds = null, string? CutDescription = null);

/// <summary>Текущий пользователь API: номер и имя — по ним автообновление отличает свои комментарии от чужих.</summary>
public sealed record IntraserviceUser(int Id, string Name);

/// <summary>Что ещё известно о заявке — для выгрузки в базу знаний: сервис, тип, категории (через запятую), когда решена
/// (фактически), организация инициатора, оборудование (через запятую), вложения, когда закрыта. null — сервер поле не
/// прислал. Живой сервер присылает сервис и тип только в карточке заявки (api/task/{id}), в строках списка их нет
/// (проверено 2026-10-02); дату закрытия — наоборот, только в строках списка (2026-10-12).</summary>
public sealed record IntraserviceExtra(string? Service, string? Type, string? Categories, DateTimeOffset? Resolved,
    string? Company = null, string? Assets = null, IReadOnlyList<IntraserviceFile>? Files = null, DateTimeOffset? Closed = null);

/// <summary>Вложение заявки: номер файла (GET api/taskfile/{Id}) и имя, как его прислал сервер; null — имени нет (сервер
/// прислал имена, которые с номерами не сопоставить).</summary>
public sealed record IntraserviceFile(int Id, string? Name);

/// <summary>Строка справочника Интрасервиса — сервис, тип заявки, сохранённый фильтр, сотрудник: номер для условий отбора
/// и название для человека. Path — для сервисов: номера от корня до этого через «|» («840|844|»); IsArchive — архивный
/// (сервис, тип); IsDefault — фильтр по умолчанию.</summary>
public sealed record IntraserviceRef(int Id, string Name, string? Path = null, bool IsArchive = false, bool IsDefault = false);

/// <summary>Результат поиска или страница списка заявок: строки (не больше страницы), общее их число, описание ошибки для UI
/// и HasNext — есть ли следующая страница, если сервер это сказал (Paginator.HasNextPage: его присылают, когда счёт не
/// заказан, count=false); null — не сказал. Total, равный CountCeiling, — «столько или больше» (см. Capped).</summary>
public sealed record IntraserviceSearchResult(IReadOnlyList<IntraserviceFound> Found, int Total, string Error, bool? HasNext = null);

/// <summary>Статус заявки (док., стр. 38): номер, название и два признака закрытости — «Заявка выполнена»
/// (IsFixed) и «Конечный» (IsFinal).</summary>
public sealed record IntraserviceStatus(int Id, string Name, bool IsFixed, bool IsFinal)
{
    /// <summary>Закрытый: с признаком «Заявка выполнена» или «Конечный» — или назван в ClosedStatusNames (closedNames, без
    /// учёта регистра — так их собирает AppSettings.ClosedNames). Одно правило у импорта, F5, поиска и автовыгрузки.</summary>
    public bool ClosedBy(IReadOnlySet<string> closedNames) => IsFixed || IsFinal || closedNames.Contains(Name);
}

/// <summary>REST API Интрасервиса (IntraService API v5.42): базовая авторизация логином и паролем пользователя,
/// GET {адрес}/api/task/{номер}, ответ в JSON по заголовку Accept.</summary>
public sealed partial class HttpIntraserviceClient
{
    // страницы ошибок IIS и прокси по-русски бывают в windows-1251: без кодовых страниц (они в .NET, но выключены) чтение тела
    // такого ответа бросает исключение, а клиент отдаёт ошибки строкой
    static HttpIntraserviceClient() => Encoding.RegisterProvider(CodePagesEncodingProvider.Instance);

    // срок у каждого запроса свой (GetAsync): обычный — 10 с, а страница большого списка для выгрузки ждёт дольше
    private static readonly HttpClient Http = new() { Timeout = Timeout.InfiniteTimeSpan };
    private static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(10);
    // пустые списки для ответов с ошибкой
    private static readonly IntraserviceEvent[] NoEvents = Array.Empty<IntraserviceEvent>();
    private static readonly IntraserviceFound[] NoFound = Array.Empty<IntraserviceFound>();
    private static readonly IntraserviceStatus[] NoStatuses = Array.Empty<IntraserviceStatus>();
    private static readonly IntraserviceRef[] NoRefs = Array.Empty<IntraserviceRef>();

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
    public Task<IntraserviceLifetime> GetLifetimeAsync(int id, CancellationToken ct = default) => GetLifetimePageAsync(id, 1, ct);

    /// <summary>Записей жизненного цикла на страницу — столько панель берёт с 0.4 (проверено на живом сервере).</summary>
    public const int LifetimePageSize = 50;

    /// <summary>Страница жизненного цикла (с первой), последние сверху. Первая — тем же адресом, что всегда, без page.</summary>
    public async Task<IntraserviceLifetime> GetLifetimePageAsync(int id, int page, CancellationToken ct = default)
    {
        var (json, error) = await GetAsync($"api/tasklifetime?taskid={id}&include=status&lastcommentsontop=true&pagesize={LifetimePageSize}"
            + (page > 1 ? $"&page={page}" : ""), "заявка не найдена", ct).ConfigureAwait(false);
        if (json is null) return new(NoEvents, false, error, false);
        try
        {
            return ParseLifetime(json) is { } r ? new(r.Events, r.HasMore, "", r.Paged) : new(NoEvents, false, Unparsed(json), false);
        }
        catch (JsonException) { return new(NoEvents, false, Unparsed(json), false); }
    }

    /// <summary>Поиск заявок на сервере (док., стр. 15): строка ищется в полях заявки и во всех её комментариях.
    /// Отдаём первые 20 совпадений, свежие сверху, и общее их число.</summary>
    public Task<IntraserviceSearchResult> SearchAsync(string text, CancellationToken ct = default) =>
        GetTasksAsync(new TaskQuery(Search: text), 1, ct, pageSize: 20, notFound: "ничего не найдено");

    /// <summary>Текущий пользователь (док., стр. 56-57): GET api/user?getcurrentuserinfo=true. Номер нужен импорту, чтобы
    /// отобрать заявки, где исполнитель — он; номер и имя — автообновлению, чтобы не считать новыми свои же комментарии.
    /// Ошибка — короткая строка, исключений наружу нет.</summary>
    public async Task<(IntraserviceUser? User, string Error)> GetCurrentUserAsync(CancellationToken ct = default)
    {
        var (json, error) = await GetAsync("api/user?getcurrentuserinfo=true", "не удалось определить пользователя", ct).ConfigureAwait(false);
        if (json is null) return (null, error);
        try
        {
            if (ParseCurrentUser(json) is { } user) return (user, "");
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

    /// <summary>Записей справочника на страницу: API отдаёт не больше 2000 (док., стр. 9) — берём с запасом.</summary>
    private const int RefPageSize = 1000;

    /// <summary>Справочник постранично, пока Paginator обещает следующую (не больше maxPages страниц). Ошибка — что пришло
    /// до неё, то и отдаём, вместе с текстом ошибки. paged: false — ответ не листается (api/filter), параметры страницы
    /// не передаём.</summary>
    private async Task<(IReadOnlyList<IntraserviceRef> Items, int Total, string Error)> GetRefsAsync(string path, string name,
        string wrapper, string notFound, int pageSize, int maxPages, bool paged, CancellationToken ct, string caller)
    {
        var items = new List<IntraserviceRef>();
        var total = 0;
        for (var page = 1; page <= maxPages; page++)
        {
            var url = !paged ? path : $"{path}{(path.Contains('?') ? '&' : '?')}pagesize={pageSize}&page={page}";
            var (json, error) = await GetAsync(url, notFound, ct, caller).ConfigureAwait(false);
            if (json is null) return (items, total, error);
            try
            {
                if (ParseRefs(json, name, wrapper) is not { } r) return (items, total, Unparsed(json, caller));
                items.AddRange(r.Items);
                total = Math.Max(total, r.Total);
                if (!paged || !r.HasMore || r.Items.Count == 0) break;
            }
            catch (JsonException) { return (items, total, Unparsed(json, caller)); }
        }
        return (items, total, "");
    }

    /// <summary>Сервисы для условия «сервис» (док., стр. 34) — и архивные, и неактуальные: заявки у них тоже бывают. Весь
    /// справочник отдаётся только с правом на просмотр списка сервисов; отказ или пустой ответ — сервисы, на которые назначен
    /// сам пользователь (for=filtertasks); нет и их — сервисы последних заявок, что видит учётная запись (блок Services списка
    /// заявок). Note — что это за список, если он не весь справочник (или почему пуст), для подсказки в окне. Error — что-то
    /// не прочиталось и ничего не нашлось: окно спросит снова при следующем открытии; пусто без ошибок — ответ, его помнят.</summary>
    public async Task<(IReadOnlyList<IntraserviceRef> Items, string Error, string Note)> GetServicesAsync(CancellationToken ct = default,
        [CallerMemberName] string caller = "")
    {
        const string all = "api/service?fields=Id,Name,Path,IsArchive&archive=true&inactive=true";
        // сбой, который пройдёт сам (сеть, срок, 502-504…), или неверный логин — не отказ в справочнике: неполный список
        // запомнился бы до конца сеанса, а так следующее открытие окна спросит весь справочник снова. 500 — не сбой: так сервер
        // отвечает и на запрет, которого не ждал, — тогда нужны запасные списки
        static bool Retry(string e) => e.Length > 0 && (HttpCode(e) == 401 || IsTransient(e) && HttpCode(e) != 500);
        var (items, _, error) = await GetRefsAsync(all, "Services", "ServiceList", "по этому адресу нет API", RefPageSize, 5, true, ct, caller).ConfigureAwait(false);
        if (error.Length == 0 && items.Count > 0) return (items, "", "");
        if (Retry(error)) return (NoRefs, error, "");
        var why = error.Length > 0 ? Headline(error) : "пустой ответ";   // ответ сервера целиком — в errors.log
        var (own, _, ownError) = await GetRefsAsync(all + "&for=filtertasks", "Services", "ServiceList", "по этому адресу нет API", RefPageSize, 5, true, ct, caller).ConfigureAwait(false);
        if (Retry(ownError)) return (NoRefs, ownError, "");
        if (ownError.Length == 0 && own.Count > 0) return (own, "", $"Весь список сервисов сервер не отдал ({why}) — здесь те, на которые вы назначены.");
        var (seen, rows, seenError) = await GetTaskServicesAsync(ct, caller).ConfigureAwait(false);
        if (seen.Count > 0)
            return (seen, "", $"Весь список сервисов сервер не отдал ({why}) — здесь сервисы последних {rows} заявок (вложенные — лишь те, что в списке).");
        // не вышло и из заявок — причина целиком: и почему не весь справочник, и почему не из заявок (там может быть сбой,
        // который пройдёт, — тогда стоит открыть окно снова)
        if (seenError.Length > 0) return (NoRefs, $"{why}; сервисы из заявок — {Headline(seenError)}", "");
        return new[] { error, ownError }.FirstOrDefault(e => e.Length > 0) is { } first ? (NoRefs, first, "")
            : (NoRefs, "", "Сервер не вернул ни одного сервиса — ни списком, ни в заявках. Сервис можно задать сохранённым фильтром.");
    }

    /// <summary>Столько последних (по изменению) заявок просматривает GetTaskServicesAsync — одной страницей: окно ждёт
    /// справочники перед первым поиском.</summary>
    private const int TaskServiceRows = 1000;

    /// <summary>Сервисы, у которых есть заявки, — из блока Services страницы последних заявок (include=service, док., стр.
    /// 16-17). Для учётной записи, которой справочник сервисов не отдают, а заявки видны. Поля строки — как в примере
    /// документации (fields=Id,Name,ServiceId), счёт не нужен (count=false; не принят — со счётом). Срок — 30 с, а не 10:
    /// сервер сортирует все видимые заявки. Rows — сколько заявок просмотрено.</summary>
    private async Task<(IReadOnlyList<IntraserviceRef> Items, int Rows, string Error)> GetTaskServicesAsync(CancellationToken ct, string caller)
    {
        Task<(string? Json, string Error)> Page(bool counted) =>
            GetAsync(new TaskQuery(IncludeArchived: true).ToUrl(1, TaskServiceRows, detailed: true, counted) + "&fields=Id,Name,ServiceId",
                "по этому адресу нет API", ct, caller, TimeSpan.FromSeconds(30));
        var (json, error) = await Page(counted: false).ConfigureAwait(false);
        if (json is null && HttpCode(error) == 400) (json, error) = await Page(counted: true).ConfigureAwait(false);
        if (json is null) return (NoRefs, 0, error);
        try
        {
            if (ParseTaskServices(json) is not { } r) return (NoRefs, 0, Unparsed(json, caller));
            // по имени: окно показывает неполный список без отступов, и порядок по пути (номерам) выглядел бы случайным
            var ordered = r.Services.DistinctBy(x => x.Id).OrderBy(x => x.Name, StringComparer.CurrentCultureIgnoreCase).ToList();
            return (ordered, r.Rows, "");
        }
        catch (JsonException) { return (NoRefs, 0, Unparsed(json, caller)); }
    }

    /// <summary>Типы заявок для условия «тип» (док., стр. 60-61), с архивными.</summary>
    public async Task<(IReadOnlyList<IntraserviceRef> Items, string Error)> GetTaskTypesAsync(CancellationToken ct = default,
        [CallerMemberName] string caller = "")
    {
        var (items, _, error) = await GetRefsAsync("api/tasktype?fields=Id,Name,IsArchive&archive=true", "TaskTypes", "TaskTypeList",
            "по этому адресу нет API", RefPageSize, 5, true, ct, caller).ConfigureAwait(false);
        return (items, error);
    }

    /// <summary>Сохранённые фильтры заявок из веб-интерфейса (док., стр. 37): отбор, которого нет в окне поиска, собирают
    /// там один раз и выбирают здесь.</summary>
    public async Task<(IReadOnlyList<IntraserviceRef> Items, string Error)> GetSavedFiltersAsync(CancellationToken ct = default,
        [CallerMemberName] string caller = "")
    {
        var (items, _, error) = await GetRefsAsync("api/filter?resource=task", "FilterView", "ArrayOfFilterView",
            "по этому адресу нет API", RefPageSize, 1, false, ct, caller).ConfigureAwait(false);
        return (items, error);
    }

    /// <summary>Сотрудники, у которых text есть в имени, логине, почте, должности и т. п. (док., стр. 53-54), — по ним
    /// ищут заявки по исполнителю и заявителю. Просим на одного больше max: Total больше max — совпадений слишком много.</summary>
    public async Task<(IReadOnlyList<IntraserviceRef> Items, int Total, string Error)> FindUsersAsync(string text, int max,
        CancellationToken ct = default, [CallerMemberName] string caller = "") =>
        await GetRefsAsync($"api/user?fields=Id,Name&search={Uri.EscapeDataString(text.Trim())}", "Users", "UserList",
            "по этому адресу нет API", max + 1, 1, true, ct, caller).ConfigureAwait(false);

    /// <summary>Сколько заявок на страницу просит GetTasksAsync; сервер вправе отдать меньше.</summary>
    public const int ExecutorPageSize = 200;

    /// <summary>До скольких сервер считает заявки списка по умолчанию (count=true, док., стр. 14: «ограничено 1000»). Точный
    /// счёт документация обещает по count=all, но живой сервер его не принимает (HTTP 400: count у него логический, 2026-10-08),
    /// так что Count, равный этому числу, — «столько или больше». Свойство — чтобы самопроверка могла его уменьшить.</summary>
    internal static int CountCeiling { get; set; } = 1000;

    /// <summary>Общее число — лишь нижняя граница: сервер досчитал до потолка и дальше не считал.</summary>
    public static bool Capped(int total) => total == CountCeiling;

    /// <summary>Страница заявок, на которых пользователь — исполнитель (док., стр. 19-20: фильтры ExecutorIds
    /// и StatusIds, оба — номера через запятую). Пустой список статусов — не «без фильтра»: такой запрос притащил бы
    /// и закрытые заявки, поэтому это ошибка, а не запрос.</summary>
    public Task<IntraserviceSearchResult> GetExecutorTasksAsync(int executorId, IReadOnlyCollection<int> statusIds, int page, CancellationToken ct = default) =>
        statusIds.Count == 0 ? Task.FromResult(new IntraserviceSearchResult(NoFound, 0, "не задан список открытых статусов"))
            : GetTasksAsync(new TaskQuery(ExecutorIds: new[] { executorId }, StatusIds: statusIds), page, ct);

    /// <summary>Страница заявок по отбору (док., стр. 14-20; условия и адрес — TaskQuery). Свежие по изменению сверху,
    /// страницы с первой. Ответ той же формы, что и у поиска (Tasks + Statuses + Paginator), поэтому разбираем его тем же
    /// ParseSearch. detailed — для окна поиска: названия сервисов в том же ответе; counted: false — без общего числа, только
    /// «есть ли следующая страница» (см. TaskQuery.ToUrl).</summary>
    public async Task<IntraserviceSearchResult> GetTasksAsync(TaskQuery query, int page, CancellationToken ct = default,
        int pageSize = ExecutorPageSize, string notFound = "по этому адресу нет API", bool detailed = false,
        TimeSpan? timeout = null, [CallerMemberName] string caller = "", bool counted = true)
    {
        // в лог — имя того, кто спросил (поиск, импорт, выгрузка): образец ответа пишется один на имя за запуск
        var (json, error) = await GetAsync(query.ToUrl(page, pageSize, detailed, counted), notFound, ct, caller, timeout).ConfigureAwait(false);
        if (json is null) return new(NoFound, 0, error);
        try
        {
            return ParseSearch(json) is { } r ? new(r.Found, r.Total, "", r.HasNext) : new(NoFound, 0, Unparsed(json, caller));
        }
        catch (JsonException) { return new(NoFound, 0, Unparsed(json, caller)); }
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
        LogOnce(call, $"{call}: не разобран ответ сервера ({json.Length} симв.):\n{Head(Redact(json), LoggedChars)}");
        return $"непонятный ответ сервера:\n{Evidence(json)}";
    }

    private async Task<(string? Json, string Error)> GetAsync(string path, string notFound, CancellationToken ct,
        [CallerMemberName] string call = "", TimeSpan? timeout = null)
    {
        using var req = new HttpRequestMessage(HttpMethod.Get, $"{_base}/{path}");
        req.Headers.Authorization = _auth;
        req.Headers.Accept.ParseAdd("application/json");
        // срок отсчитывается на весь запрос, с чтением тела, — как раньше Timeout у HttpClient; отмена снаружи — не срок
        var limit = timeout ?? DefaultTimeout;
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        deadline.CancelAfter(limit);
        try
        {
            using var resp = await Http.SendAsync(req, deadline.Token).ConfigureAwait(false);
            KnownCharset(resp.Content);
            if (resp.IsSuccessStatusCode) return (await resp.Content.ReadAsStringAsync(deadline.Token).ConfigureAwait(false), "");
            return (null, await RefusalAsync(resp, notFound, path, call, deadline.Token, ct).ConfigureAwait(false));
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested) { return (null, $"сервер не ответил за {limit.TotalSeconds:0.##} с"); }
        catch (HttpRequestException e) { return (null, $"сервер недоступен:\n{Reason(e)}"); }
        catch (IOException e) { return (null, $"соединение оборвалось:\n{Reason(e)}"); }
    }

    /// <summary>Ответ с кодом ошибки — текстом: первая строка по-русски и коротко, дальше — что сервер ответил на самом деле
    /// (гадать по пересказу хуже); тело — в errors.log, один образец на метод и код за запуск.</summary>
    private async Task<string> RefusalAsync(HttpResponseMessage resp, string notFound, string path, string call, CancellationToken deadline,
        CancellationToken ct)
    {
        var code = (int)resp.StatusCode;
        var what = resp.StatusCode switch
        {
            HttpStatusCode.Unauthorized => "неверный логин или пароль",
            HttpStatusCode.Forbidden => "нет доступа",
            HttpStatusCode.NotFound => notFound,
            _ => "ошибка сервера",
        };
        var (text, cut) = await HeadAsync(resp.Content, deadline, ct).ConfigureAwait(false);   // не дочитали — хватит и кода
        var body = Masked(text);
        if (string.IsNullOrWhiteSpace(body)) return $"{what} (HTTP {code})";
        LogOnce($"{call} HTTP {code}", $"{call}: HTTP {code} на {path} ({body.Length} симв.{(cut ? $", только начало — {ErrorBodyBytes / 1024} КБ" : "")}):\n{Head(Redact(body), LoggedChars)}");
        return $"{what} (HTTP {code}):\n{Evidence(body)}";
    }

    /// <summary>Тело ответа для ошибки и журнала: страница-трассировка прокси или IIS может повторить заголовки запроса — вместе
    /// с нашим логином и паролем (Redact ловит только «Basic …», а тут токен может стоять и без слова).</summary>
    private string Masked(string body) => body.Replace(_auth.Parameter!, "***");

    /// <summary>Образец ответа — в errors.log, один на ключ (метод и код) за запуск.</summary>
    private static void LogOnce(string key, string text)
    {
        if (LoggedCalls.TryAdd(key, 0)) LogUnparsed?.Invoke(text);
    }

    /// <summary>Страница вместо файла — ошибкой с её текстом (начало, HeadAsync: страница оборвалась или замолчала — та же
    /// «страница»), логин и пароль, если страница их повторила, — звёздочками; начало — в errors.log, один образец за запуск.</summary>
    private async Task<string> PageInsteadOfFileAsync(HttpResponseMessage resp, string path, CancellationToken deadline, CancellationToken ct)
    {
        var (text, cut) = await HeadAsync(resp.Content, deadline, ct).ConfigureAwait(false);
        var page = Masked(text);
        LogOnce($"{nameof(DownloadFileAsync)} html", $"{nameof(DownloadFileAsync)}: страница вместо файла на {path} ({page.Length} симв.{(cut ? $", только начало — {ErrorBodyBytes / 1024} КБ" : "")}):\n{Head(Redact(page), LoggedChars)}");
        return $"сервер вернул страницу вместо файла:\n{Evidence(page)}";
    }

    /// <summary>Сколько тела ответа читаем ради ошибки: ответ с кодом ошибки может прийти и с самим файлом в сотни мегабайт.</summary>
    private const int ErrorBodyBytes = 64 * 1024;

    /// <summary>Начало тела ответа — для текста ошибки: не больше ErrorBodyBytes, по его кодировке (KnownCharset её уже
    /// проверил) или по BOM, иначе UTF-8; Cut — пришло больше, чем прочитано. Оборвалось или замолчало до срока (deadline) — что
    /// успело прийти: код ответа уже известен, а сетевой сбой повторяли бы зря; остановили (ct) — наверх.</summary>
    private static async Task<(string Text, bool Cut)> HeadAsync(HttpContent content, CancellationToken deadline, CancellationToken ct)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(ErrorBodyBytes);
        try
        {
            var length = 0;
            try
            {
                await using var body = await content.ReadAsStreamAsync(deadline).ConfigureAwait(false);
                for (int read; length < ErrorBodyBytes && (read = await body.ReadAsync(buffer.AsMemory(length, ErrorBodyBytes - length), deadline).ConfigureAwait(false)) > 0;)
                    length += read;
            }
            catch (Exception e) when (e is HttpRequestException or IOException || e is OperationCanceledException && !ct.IsCancellationRequested) { }
            var encoding = content.Headers.ContentType?.CharSet is { Length: > 0 } charset ? Encoding.GetEncoding(charset.Trim('"')) : Encoding.UTF8;
            using var reader = new StreamReader(new MemoryStream(buffer, 0, length), encoding, detectEncodingFromByteOrderMarks: true);
            return (reader.ReadToEnd(), length == ErrorBodyBytes);
        }
        finally { ArrayPool<byte>.Shared.Return(buffer); }
    }

    /// <summary>Кодировку из заголовка ответа, которой .NET не знает (кодовые страницы включены в статическом конструкторе —
    /// windows-1251 знакома), — убрать, сразу после ответа: читатели тела иначе бросают исключение, а без неё читают по BOM или
    /// как UTF-8 — сырой ответ лучше никакого, а исключение из клиента, который отдаёт ошибки строкой, хуже всего.</summary>
    private static void KnownCharset(HttpContent content)
    {
        if (content.Headers.ContentType is not { CharSet: { Length: > 0 } charset } type) return;
        try { Encoding.GetEncoding(charset.Trim('"')); }
        catch (ArgumentException) { type.CharSet = null; }
    }

    /// <summary>Сколько ждём заголовки ответа на файл и сколько — каждый следующий его кусок. Общего срока нет: файл в
    /// гигабайт по VPN качается долго, а замолчавший посреди файла сервер — сбой. Меняются только самопроверкой.</summary>
    internal static TimeSpan FileHeadersTimeout { get; set; } = TimeSpan.FromSeconds(60);
    internal static TimeSpan FileIdleTimeout { get; set; } = TimeSpan.FromSeconds(60);

    /// <summary>Вложение заявки (док., стр. 33: GET api/taskfile/{номер файла}) — потоком в to; progress — сколько байт пришло к
    /// этому мигу. stopAt — дальше не нужно: пришло столько — чтение кончается, файл не целый (Bytes ≥ stopAt). Не файл, а
    /// страница — переадресация (страница входа: Basic-вход к файлам может и не пускать — живьём проверено только в браузере)
    /// или html у вложения, которое не html (htmlExpected), — ошибка: иначе страница легла бы вместо файла и считалась бы
    /// скачанной. Ошибка сети или сервера — текстом того же вида, что у запросов (IsTransient решает, поможет ли повтор);
    /// ошибка записи в to — не сети: уходит наверх исключением, как и отмена.</summary>
    public async Task<(long Bytes, string Error)> DownloadFileAsync(int fileId, Stream to, Action<long>? progress, CancellationToken ct,
        long stopAt = long.MaxValue, bool htmlExpected = false)
    {
        var path = $"api/taskfile/{fileId}";
        using var req = new HttpRequestMessage(HttpMethod.Get, $"{_base}/{path}");
        req.Headers.Authorization = _auth;
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        var limit = FileHeadersTimeout;
        deadline.CancelAfter(limit);
        long bytes = 0;
        HttpResponseMessage? resp = null;
        Stream? body = null;
        try
        {
            var buffer = new byte[81920];
            while (true)
            {
                int read;
                try
                {
                    if (body is null)
                    {
                        resp = await Http.SendAsync(req, HttpCompletionOption.ResponseHeadersRead, deadline.Token).ConfigureAwait(false);
                        KnownCharset(resp.Content);
                        limit = FileIdleTimeout;   // тело — ошибки или файла — читается со своим сроком, не с остатком срока на заголовки
                        deadline.CancelAfter(limit);
                        if (!resp.IsSuccessStatusCode) return (0, await RefusalAsync(resp, "файл не найден", path, nameof(DownloadFileAsync), deadline.Token, ct).ConfigureAwait(false));
                        if (resp.RequestMessage?.RequestUri is { } final && !final.AbsolutePath.EndsWith("/" + path, StringComparison.OrdinalIgnoreCase))
                            return (0, $"сервер переадресовал запрос файла на {final.AbsolutePath} — похоже, на страницу входа: файл по логину и паролю API не отдан");
                        if (!htmlExpected && resp.Content.Headers.ContentType?.MediaType == "text/html")
                            return (0, await PageInsteadOfFileAsync(resp, path, deadline.Token, ct).ConfigureAwait(false));
                        body = await resp.Content.ReadAsStreamAsync(deadline.Token).ConfigureAwait(false);
                    }
                    deadline.CancelAfter(limit);   // срок — на каждый кусок, а не на весь файл
                    read = await body.ReadAsync(buffer, deadline.Token).ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (!ct.IsCancellationRequested)
                {
                    return (bytes, $"сервер не ответил за {limit.TotalSeconds:0.##} с" + (body is null ? "" : $": файл перестал приходить на {Attachments.Size(bytes)}"));
                }
                catch (HttpRequestException e) { return (bytes, $"сервер недоступен:\n{Reason(e)}"); }
                catch (IOException e) { return (bytes, $"соединение оборвалось" + (bytes > 0 ? $" на {Attachments.Size(bytes)}" : "") + $":\n{Reason(e)}"); }
                if (read == 0) return (bytes, "");
                await to.WriteAsync(buffer.AsMemory(0, read), ct).ConfigureAwait(false);
                bytes += read;
                progress?.Invoke(bytes);
                if (bytes >= stopAt) return (bytes, "");
            }
        }
        finally
        {
            body?.Dispose();
            resp?.Dispose();
        }
    }

    /// <summary>Сколько сырого ответа показываем в самом сообщении и сколько кладём в errors.log.</summary>
    private const int ShownChars = 1000, LoggedChars = 4000;

    private static readonly Regex BasicToken = new(@"(\bBasic\s+)[A-Za-z0-9+/=]{6,}", RegexOptions.IgnoreCase | RegexOptions.Compiled);

    /// <summary>Любой Basic-токен в тексте — звёздочками: из base64 логин и пароль достаются за секунду.</summary>
    private static string Redact(string text) => BasicToken.Replace(text, "$1***");

    /// <summary>Название заявки, пока настоящего нет: так его ставит доска и так разбор называет заявку без темы; по нему
    /// синхронизация узнаёт, что название ещё можно заменить настоящим (Apply).</summary>
    public static string AutoTitle(int id) => $"Заявка #{id}";

    /// <summary>Первая строка ошибки — короткая фраза с кодом, без ответа сервера.</summary>
    public static string Headline(string error) => error.Split('\n')[0].Trim().TrimEnd(':');

    /// <summary>Коротко, в одну строку: фраза и первая строка подробностей — для мест, где под ошибку одна строка
    /// (подпись в быстром добавлении, вопрос «перенести в Готово?»). Целиком ошибка — в панели и в errors.log.</summary>
    public static string Brief(string error)
    {
        var lines = error.Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        return lines.Length > 1 ? $"{lines[0].TrimEnd(':')} — {lines[1]}" : lines.FirstOrDefault() ?? "";
    }

    private static readonly Regex HttpCodeInError = new(@"\(HTTP (\d{3})\)", RegexOptions.CultureInvariant | RegexOptions.Compiled);

    /// <summary>Код ответа сервера из текста ошибки («ошибка сервера (HTTP 500):…» — он в первой строке; в теле ответа, что
    /// ниже, может быть что угодно). null — ошибка не из ответа: сеть, срок, разбор.</summary>
    public static int? HttpCode(string error) =>
        HttpCodeInError.Match(error.Split('\n')[0]) is { Success: true } m ? int.Parse(m.Groups[1].Value, CultureInfo.InvariantCulture) : null;

    /// <summary>Сбой, который может пройти при повторе: сервер не ответил за срок, не достался или оборвал соединение, ответил
    /// 5xx, 408 или 429. Не пройдёт: неверный логин (401), нет доступа (403), нет такой заявки (404), любой другой отказ 4xx
    /// и ответ, который не разобрался, — повтор получил бы то же самое.</summary>
    public static bool IsTransient(string error) =>
        error.StartsWith("сервер не ответил", StringComparison.Ordinal) || error.StartsWith("сервер недоступен", StringComparison.Ordinal)
        || error.StartsWith("соединение оборвалось", StringComparison.Ordinal) || HttpCode(error) is >= 500 or 408 or 429;

    /// <summary>Тело ответа для сообщения об ошибке. json и xml — как есть, байт в байт. Html-страницу (ошибка IIS,
    /// страница прокси или входа) — её видимым текстом, с пометкой: сырой она начинается с килобайта стилей,
    /// и причины на экране не видно. В лог в любом случае уходит сырое.</summary>
    internal static string Evidence(string body)
    {
        var raw = Redact(body.Trim());
        var text = raw.StartsWith('<') && raw.Contains("<html", StringComparison.OrdinalIgnoreCase)
            ? "(html-страница, показан её текст)\n" + PageText(raw)
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
