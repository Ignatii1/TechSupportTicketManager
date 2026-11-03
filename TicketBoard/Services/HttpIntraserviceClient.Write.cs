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

    /// <summary>Комментарий в заявку (док.: PUT api/task/{номер}, поля Comment и IsPrivateComment). changed — Changed карточки,
    /// прочитанной перед отправкой, строкой, как её прислал сервер: изменили заявку после — сервер отвечает 409 и ничего не
    /// пишет; null — сервер Changed не прислал, уходит без проверки.</summary>
    public Task<IntraserviceWrite> CommentAsync(int id, string text, bool isPrivate, string? changed, CancellationToken ct = default)
    {
        var body = new JsonObject { ["Comment"] = text, ["IsPrivateComment"] = isPrivate };
        if (changed is not null) body["Changed"] = changed;
        return PutAsync($"api/task/{id}", body.ToJsonString(), "заявка не найдена", ct);
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
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        deadline.CancelAfter(WriteTimeout);
        try
        {
            using var resp = await Http.SendAsync(req, HttpCompletionOption.ResponseHeadersRead, deadline.Token).ConfigureAwait(false);
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
