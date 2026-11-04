using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Статус в Интрасервисе при переносе карточки руками (этап 8, шаг 2).</summary>
public sealed partial class MainViewModel
{
    // заглушка: проверки сначала красные
    private void OfferStatus(Ticket t) { }

    /// <summary>Все начатые предложения сменить статус закончились — для самопроверки.</summary>
    internal Task OffersSettled() => Task.CompletedTask;
}
