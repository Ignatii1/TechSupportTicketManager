using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>REST API Интрасервиса (IntraService API v5.42): базовая авторизация логином и паролем пользователя,
/// GET {адрес}/api/task/{номер}, ответ в JSON по заголовку Accept.</summary>
public sealed class HttpIntraserviceClient : IIntraserviceClient
{
    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(10) };
    private static readonly Regex HtmlBreaks = new(@"<br\s*/?>|</p>|</div>|</li>", RegexOptions.IgnoreCase | RegexOptions.Compiled);
    private static readonly Regex HtmlTags = new(@"<[^>]*>", RegexOptions.Compiled);
    private static readonly Regex ManyNewlines = new(@"\n\s*\n\s*\n+", RegexOptions.Compiled);

    private readonly string _base;
    private readonly AuthenticationHeaderValue _auth;

    public HttpIntraserviceClient(string baseUrl, string login, string password)
    {
        _base = baseUrl.Trim().TrimEnd('/');
        _auth = new("Basic", Convert.ToBase64String(Encoding.UTF8.GetBytes($"{login}:{password}")));
    }

    /// <summary>Клиент по настройкам; без адреса или логина/пароля — заглушка.</summary>
    public static IIntraserviceClient From(AppSettings s) =>
        IsValidUrl(s.IntraserviceBaseUrl) && s.IntraserviceLogin.Trim().Length > 0 && s.IntraservicePassword.Length > 0
            ? new HttpIntraserviceClient(s.IntraserviceBaseUrl, s.IntraserviceLogin.Trim(), s.IntraservicePassword)
            : new NullIntraserviceClient();

    public static bool IsValidUrl(string url) =>
        Uri.TryCreate(url.Trim(), UriKind.Absolute, out var u) && (u.Scheme == Uri.UriSchemeHttps || u.Scheme == Uri.UriSchemeHttp);

    public static bool IsHttp(string url) => url.Trim().StartsWith("http://", StringComparison.OrdinalIgnoreCase);

    public async Task<IntraserviceResult> GetTaskAsync(int id, CancellationToken ct = default)
    {
        var (json, error) = await GetAsync($"api/task/{id}?include=status", "заявка не найдена", ct).ConfigureAwait(false);
        if (json is null) return new(null, error);
        try { return Parse(json, id) is { } task ? new(task, "") : new(null, "непонятный ответ сервера"); }
        catch (JsonException) { return new(null, "непонятный ответ сервера"); }
    }

    /// <summary>Проверка адреса и логина: список статусов маленький. "" — всё хорошо.</summary>
    public async Task<string> CheckAsync(CancellationToken ct = default)
    {
        var (json, error) = await GetAsync("api/taskstatus", "по этому адресу нет API", ct).ConfigureAwait(false);
        if (json is null) return error;
        try { using var _ = JsonDocument.Parse(json); return ""; }
        catch (JsonException) { return "ответ не похож на API Интрасервиса"; }
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

        var status = Str(task, "StatusName");
        if (string.IsNullOrWhiteSpace(status) && Int(task, "StatusId") is int statusId)
            status = (Prop(root, "Statuses") is { ValueKind: JsonValueKind.Array } list
                ? list.EnumerateArray().Where(s => s.ValueKind == JsonValueKind.Object && Int(s, "Id") == statusId)
                      .Select(s => Str(s, "Name")).FirstOrDefault()
                : null) ?? $"статус {statusId}";

        return new(Int(task, "Id") ?? id, name.Trim(), status?.Trim() ?? "", HtmlToText(Str(task, "Description")));
    }

    private static JsonElement? Prop(JsonElement e, string name)
    {
        foreach (var p in e.EnumerateObject())
            if (p.Name.Equals(name, StringComparison.OrdinalIgnoreCase)) return p.Value;
        return null;
    }

    private static string? Str(JsonElement e, string name) => Prop(e, name) is { ValueKind: JsonValueKind.String } v ? v.GetString() : null;

    private static int? Int(JsonElement e, string name) => Prop(e, name) is { ValueKind: JsonValueKind.Number } v && v.TryGetInt32(out var n) ? n : null;

    /// <summary>Описание в Интрасервисе обычно HTML из редактора — оставляем текст.</summary>
    private static string? HtmlToText(string? html)
    {
        if (string.IsNullOrWhiteSpace(html)) return null;
        var text = WebUtility.HtmlDecode(HtmlTags.Replace(HtmlBreaks.Replace(html, "\n"), "")).Replace("\r", "");
        return ManyNewlines.Replace(text, "\n\n").Trim();
    }
}
