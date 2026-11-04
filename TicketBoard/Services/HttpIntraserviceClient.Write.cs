using System.IO;
using System.Net;
using System.Net.Http;
using System.Runtime.CompilerServices;
using System.Text;
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
    public Task<IntraserviceWrite> CommentAsync(int id, string text, bool isPrivate, string? changed, CancellationToken ct = default)
    {
        var body = new JsonObject { ["Comment"] = text, ["IsPrivateComment"] = isPrivate };
        if (changed is not null) body["Changed"] = changed;
        return PutAsync($"api/task/{id}", body.ToJsonString(), "заявка не найдена", ct);
    }

    // заглушка: проверки сначала красные
    public Task<IntraserviceWrite> ChangeStatusAsync(int id, int statusId, string comment, bool isPrivate, string? changed,
        CancellationToken ct = default) => Task.FromResult(new IntraserviceWrite(WriteOutcome.Refused, "не сделано"));

    // заглушка: проверки сначала красные
    public async Task<IntraserviceCard> GetTaskWithRightsAsync(int id, CancellationToken ct = default)
    {
        var r = await GetTaskAsync(id, ct).ConfigureAwait(false);
        return new(r.Task, null, r.Error);
    }

    // заглушка: проверки сначала красные
    internal static IReadOnlyList<int>? ParseRights(string json) => null;

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
