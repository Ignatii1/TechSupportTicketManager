namespace TicketBoard.Services;

public sealed record IntraserviceTask(int Id, string Name, string Status, string? Description);

/// <summary>Заявка или короткое описание ошибки для UI («заявка не найдена», «сервер недоступен»). Секретов в тексте нет.</summary>
public sealed record IntraserviceResult(IntraserviceTask? Task, string Error);

public interface IIntraserviceClient
{
    Task<IntraserviceResult> GetTaskAsync(int id, CancellationToken ct = default);
}

/// <summary>API не настроен (нет адреса или логина/пароля).</summary>
public sealed class NullIntraserviceClient : IIntraserviceClient
{
    public Task<IntraserviceResult> GetTaskAsync(int id, CancellationToken ct = default)
        => Task.FromResult(new IntraserviceResult(null, "API не настроен"));
}
