namespace TicketBoard.Services;

// Заготовка под API Интрасервиса. Когда появится токен — реализуем HttpIntraserviceClient,
// а Ticket уже умеет хранить IntraserviceId / ExternalStatus / LastSyncAt.

public sealed record IntraserviceTask(int Id, string Name, string Status, string? Description);

public interface IIntraserviceClient
{
    Task<IntraserviceTask?> GetTaskAsync(int id, CancellationToken ct = default);
}

public sealed class NullIntraserviceClient : IIntraserviceClient
{
    public Task<IntraserviceTask?> GetTaskAsync(int id, CancellationToken ct = default)
        => Task.FromResult<IntraserviceTask?>(null);
}
