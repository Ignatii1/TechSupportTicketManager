using System.IO;
using System.Net;
using System.Net.Http;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace TicketBoard.Services;

/// <summary>Итог записи в Интрасервис: Done — сервер принял; Refused — ничего не записано (сервер отказал — 4xx — или до него
/// не дошли); Unknown — записано ли, неизвестно (ответа не дождались, соединение оборвалось, ошибка сервера — 5xx).</summary>
public enum WriteOutcome { Done, Refused, Unknown }

/// <summary>Итог записи и текст для человека (при Done — пусто). Conflict — отказ 409: заявку изменили после того, как её
/// прочитали (Changed в запросе старше).</summary>
public sealed record IntraserviceWrite(WriteOutcome Outcome, string Error, bool Conflict = false);

/// <summary>Запись в Интрасервис (этап 8): пока — комментарий. Запись не повторяется ни после сбоя, ни после 5xx: повтор
/// чтения безвреден, повтор записи — два одинаковых комментария.</summary>
public sealed partial class HttpIntraserviceClient
{
    /// <summary>Сколько ждём ответ на запись — дольше чтения: записав комментарий, сервер ещё и рассылает уведомления.
    /// Меняется только самопроверкой.</summary>
    internal static TimeSpan WriteTimeout { get; set; } = TimeSpan.FromSeconds(30);

    /// <summary>Имя программы в заголовке Device-Name — по нему в истории заявки видно, что запись из TicketBoard.</summary>
    public const string DeviceName = "TicketBoard";
    private static readonly string DeviceVersion = typeof(HttpIntraserviceClient).Assembly.GetName().Version?.ToString(3) ?? "0.0.0";

    /// <summary>Свой клиент для записи — без пула соединений: запрос, оборвавшийся на соединении из пула до первого байта ответа,
    /// .NET сам повторяет на новом (так он обходит соединения, закрытые сервером в простое), а для записи это второй
    /// комментарий. Новое соединение не повторяется, а без пула каждая запись — по новому.</summary>
    private static readonly HttpClient WriteHttp = new(new SocketsHttpHandler { PooledConnectionLifetime = TimeSpan.Zero, MaxResponseDrainSize = 0 })
        { Timeout = Timeout.InfiniteTimeSpan };

    /// <summary>Комментарий в заявку (док.: PUT api/task/{номер}, поля Comment и IsPrivateComment). changed — Changed карточки,
    /// прочитанной перед отправкой, строкой, как её прислал сервер: изменили заявку после — сервер отвечает 409 и ничего не
    /// пишет; null — сервер Changed не прислал, уходит без проверки.</summary>
    public Task<IntraserviceWrite> CommentAsync(int id, string text, bool isPrivate, string? changed, CancellationToken ct = default) =>
        PutAsync($"api/task/{id}", Body(WithComment(new JsonObject(), text, isPrivate), changed), "заявка не найдена", ct);

    /// <summary>Сменить статус заявки (док.: PUT api/task/{номер}, поле StatusId) — с комментарием или без (пустой не
    /// уходит вовсе: ни Comment, ни IsPrivateComment). changed — как у CommentAsync: изменили заявку после — 409.</summary>
    public Task<IntraserviceWrite> ChangeStatusAsync(int id, int statusId, string comment, bool isPrivate, string? changed,
        CancellationToken ct = default)
    {
        var body = new JsonObject { ["StatusId"] = statusId };
        if (comment.Length > 0) WithComment(body, comment, isPrivate);
        return PutAsync($"api/task/{id}", Body(body, changed), "заявка не найдена", ct);
    }

    /// <summary>Комментарий в теле записи: текст и признак «внутренний» — у комментария и у смены статуса одни.</summary>
    private static JsonObject WithComment(JsonObject body, string text, bool isPrivate)
    {
        body["Comment"] = text;
        body["IsPrivateComment"] = isPrivate;
        return body;
    }

    /// <summary>Тело записи строкой — с Changed прочитанной карточки (null — сервер его не прислал: уходит без проверки).</summary>
    private static string Body(JsonObject body, string? changed)
    {
        if (changed is not null) body["Changed"] = changed;
        return body.ToJsonString();
    }

    /// <summary>Заявка с правами на неё (include=usertaskrights — в какие статусы её можно перевести). Сервер такого include не
    /// принял (400; как он отдаёт права, живьём не видели) — карточка без прав: решит сервер, при записи.</summary>
    public async Task<IntraserviceCard> GetTaskWithRightsAsync(int id, CancellationToken ct = default)
    {
        var (json, error) = await GetAsync($"api/task/{id}?include=status,usertaskrights", "заявка не найдена", ct).ConfigureAwait(false);
        if (json is null)
        {
            if (HttpCode(error) != 400) return new(null, null, error);
            var plain = await GetTaskAsync(id, ct).ConfigureAwait(false);
            return new(plain.Task, null, plain.Error);
        }
        try { return Parse(json, id) is { } task ? new(task, ParseRights(json), "") : new(null, null, Unparsed(json)); }
        catch (JsonException) { return new(null, null, Unparsed(json)); }
    }

    /// <summary>PUT с телом JSON: итог — WriteOutcome, текст ошибки — того же вида, что у чтения (RefusalAsync), но с тем, что
    /// с записью: отказал сервер — не записано, ошибка сервера или нет ответа — неизвестно.</summary>
    private async Task<IntraserviceWrite> PutAsync(string path, string json, string notFound, CancellationToken ct,
        [CallerMemberName] string call = "")
    {
        using var req = new HttpRequestMessage(HttpMethod.Put, $"{_base}/{path}") { Content = new StringContent(json, Encoding.UTF8, "application/json") };
        req.Headers.Authorization = _auth;
        req.Headers.Accept.ParseAdd("application/json");
        req.Headers.Add("Device-Name", DeviceName);
        req.Headers.Add("Device-Version", DeviceVersion);
        req.Headers.ConnectionClose = true;   // и сервер закроет его после ответа — в пул оно не вернётся
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        deadline.CancelAfter(WriteTimeout);
        try
        {
            using var resp = await WriteHttp.SendAsync(req, HttpCompletionOption.ResponseHeadersRead, deadline.Token).ConfigureAwait(false);
            // ответил не API: HttpClient сам прошёл по переадресации (страница входа — уже без нашего входа; что бы она ни
            // ответила) или прокси отдал страницу. Записано ли — неизвестно: «отправлено» стёрло бы черновик, «заявка не
            // найдена» — неправда
            if (Redirected(resp, path) is { } where)
                return new(WriteOutcome.Unknown, $"сервер переадресовал запись на {where} — похоже, на страницу входа; записано ли, неизвестно");
            if (resp.IsSuccessStatusCode && IsPage(resp))
                return new(WriteOutcome.Unknown, "сервер вернул страницу вместо ответа API — записано ли, неизвестно:\n"
                    + await PageAsync(resp, path, call, "ответа", deadline.Token, ct).ConfigureAwait(false));
            if (resp.IsSuccessStatusCode) return new(WriteOutcome.Done, "");
            var code = (int)resp.StatusCode;
            var what = resp.StatusCode switch
            {
                HttpStatusCode.Conflict => "заявку изменили после того, как её прочитали",
                HttpStatusCode.Unauthorized or HttpStatusCode.Forbidden or HttpStatusCode.NotFound => null,   // как у чтения
                _ when code >= 500 => "ошибка сервера — записано ли, неизвестно",
                _ => "сервер не принял запись",
            };
            var error = await RefusalAsync(resp, notFound, path, call, deadline.Token, ct, what).ConfigureAwait(false);
            return new(code >= 500 ? WriteOutcome.Unknown : WriteOutcome.Refused, error, resp.StatusCode == HttpStatusCode.Conflict);
        }
        catch (OperationCanceledException) when (!ct.IsCancellationRequested)
        {
            return new(WriteOutcome.Unknown, $"сервер не ответил за {WriteTimeout.TotalSeconds:0.##} с — записано ли, неизвестно");
        }
        // соединения не было — запрос не ушёл
        catch (HttpRequestException e) when (e.HttpRequestError is HttpRequestError.NameResolutionError or HttpRequestError.ConnectionError
            or HttpRequestError.SecureConnectionError or HttpRequestError.ProxyTunnelError)
        {
            return new(WriteOutcome.Refused, $"сервер недоступен:\n{Reason(e)}");
        }
        catch (Exception e) when (e is HttpRequestException or IOException)
        {
            return new(WriteOutcome.Unknown, $"соединение оборвалось — записано ли, неизвестно:\n{Reason(e)}");
        }
    }
}
