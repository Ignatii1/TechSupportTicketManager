namespace TicketBoard.Services;

/// <summary>Итог записи в Интрасервис: Done — сервер принял; Refused — ничего не записано (сервер отказал — 4xx — или до него
/// не дошли); Unknown — записано ли, неизвестно (ответа не дождались, соединение оборвалось, ошибка сервера — 5xx).</summary>
public enum WriteOutcome { Done, Refused, Unknown }

/// <summary>Итог записи и текст для человека (при Done — пусто). Conflict — отказ 409: заявку изменили после того, как её
/// прочитали (Changed в запросе старше).</summary>
public sealed record IntraserviceWrite(WriteOutcome Outcome, string Error, bool Conflict = false);

/// <summary>Запись в Интрасервис (этап 8): пока — комментарий.</summary>
public sealed partial class HttpIntraserviceClient
{
    /// <summary>Сколько ждём ответ на запись. Меняется только самопроверкой.</summary>
    internal static TimeSpan WriteTimeout { get; set; } = TimeSpan.FromSeconds(30);

    /// <summary>Имя программы в заголовке Device-Name — по нему в истории заявки видно, что запись из TicketBoard.</summary>
    public const string DeviceName = "TicketBoard";

    // заглушка: проверки сначала красные
    public Task<IntraserviceWrite> CommentAsync(int id, string text, bool isPrivate, string? changed, CancellationToken ct = default) =>
        Task.FromResult(new IntraserviceWrite(WriteOutcome.Refused, "не сделано"));
}
