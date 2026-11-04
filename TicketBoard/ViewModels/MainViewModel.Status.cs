using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Статус в Интрасервисе при переносе карточки руками (этап 8, шаг 2): колонке назначен статус (настройки), а у
/// заявки он другой — окно «Сменить статус заявки #N?» (StatusForm), потом запись. Сюда приходят только переносы руками
/// (MoveByHand, OnDropped): автообновление и F5 переносят мимо.</summary>
public sealed partial class MainViewModel
{
    /// <summary>Окна смены статуса — по одному: перенесли две карточки подряд — второе ждёт, пока закроют первое (и пока
    /// уйдёт его запись).</summary>
    private readonly SemaphoreSlim _statusGate = new(1, 1);
    /// <summary>Заявка и колонка, для которых окно уже предложено: перенесли туда-обратно-туда — второе не нужно.</summary>
    private readonly HashSet<(int Ticket, TicketStatus Column)> _offering = new();
    private readonly List<Task> _offers = new();

    /// <summary>Сколько последних комментариев переписки показать в окне.</summary>
    private const int RecentInStatusForm = 3;

    /// <summary>Перенесли карточку руками в другую колонку — предложить тот же шаг в Интрасервисе (доска уже перенесла).</summary>
    private void OfferStatus(Ticket t)
    {
        _offers.RemoveAll(o => o.IsCompleted);
        _offers.Add(OfferStatusAsync(t, t.Status));
    }

    /// <summary>Все начатые предложения сменить статус закончились — для самопроверки.</summary>
    internal Task OffersSettled() => Task.WhenAll(_offers.ToArray());

    private async Task OfferStatusAsync(Ticket t, TicketStatus to)
    {
        if (!_settings.AllowStatusWrite || _intraservice is not { } client || t.IntraserviceId is not int n) return;
        if (_settings.StatusOnMove?.GetValueOrDefault(to) is not { } mapped) return;
        if (!_offering.Add((n, to))) return;
        var account = _settings.AccountKey;
        try
        {
            await _statusGate.WaitAsync();
            try { await OfferStatusCoreAsync(t, n, to, mapped, client, account); }
            finally { _statusGate.Release(); }
        }
        // начато перетаскиванием, ждать некому: исключение — в журнал, а не в никуда
        catch (Exception ex) { Log?.Invoke($"Статус заявки #{n}: {ex}"); }
        finally { _offering.Remove((n, to)); }
    }

    private async Task OfferStatusCoreAsync(Ticket t, int n, TicketStatus to, StatusRef mapped, HttpIntraserviceClient client, string account)
    {
        // перенесли дальше или удалили с доски, пока читалась заявка или ждало другое окно, — это окно уже не нужно
        bool Gone() => t.Status != to || !AllTickets.Contains(t);
        if (Gone()) return;
        var card = await client.GetTaskWithRightsAsync(n);
        if (Gone()) return;
        if (card.Task is not { } task) { NotChanged(n, t.Title, $"заявка не прочиталась: {card.Error}"); return; }
        if (task.StatusId == mapped.Id) return;   // в Интрасервисе уже он
        // справочник и переписка друг от друга не зависят — вместе
        var directoryRead = client.GetStatusesAsync();
        var lifeRead = client.GetLifetimeAsync(n);
        var (directory, directoryError) = await directoryRead;
        var life = await lifeRead;
        if (directoryError.Length > 0) { NotChanged(n, t.Title, $"справочник статусов не прочитался: {directoryError}"); return; }
        if (life.Error.Length > 0) { NotChanged(n, t.Title, $"переписка не прочиталась: {life.Error}"); return; }
        if (Gone()) return;

        var title = KnowledgeExport.OneLine(task.Name);   // с сервера, одной строкой: название пишет заявитель
        // куда можно: по правам заявки, а нет их — весь справочник (решит сервер); без нынешнего статуса
        var choices = directory.Where(s => s.Id != task.StatusId && (card.ToStatuses is not { } allowed || allowed.Contains(s.Id))).ToList();
        if (choices.Count == 0)
        {
            NotChanged(n, title, card.ToStatuses is null
                ? $"из «{task.Status}» перевести её некуда — в справочнике статусов сервера других нет"
                : $"из «{task.Status}» перевести её вам некуда — права на заявку этого не дают");
            return;
        }
        var column = TicketStatusText.Of(to);
        var own = directory.FirstOrDefault(s => s.Id == mapped.Id);
        var (proposed, note) =
            own is null || own.Name != mapped.Name   // номер из настроек — от другого сервера (или статус переименовали)
                ? ((IntraserviceStatus?)null, $"В настройках для «{column}» — «{mapped.Name}», а на сервере такого статуса нет: выберите сами или «Только на доске».")
            : !choices.Contains(own)
                ? (null, $"«{own.Name}» этой заявке сейчас недоступен — права не дают. Можно выбрать из доступных.")
            : (own, "");
        var recent = ToRows(life.Events).Where(r => r.Text is not null).Take(RecentInStatusForm).ToList();
        var form = new StatusForm(n, title, task.Status, choices, proposed, note, recent);
        // «Только на доске» (или Esc); окно не даёт нажать без обязательного комментария — и здесь его тоже нет
        if (!await Views.StatusWindow.AskAsync(form) || form.Target is not { } target || !form.CanApply) return;
        // пока было открыто окно, карточку перенесли (автообновление: ответил заявитель) или убрали — статус был для другой колонки
        if (!AllTickets.Contains(t)) { NotChanged(n, title, "пока было открыто окно, карточку убрали с доски", ""); return; }
        if (t.Status != to)
        {
            NotChanged(n, title, $"пока было открыто окно, карточку перенесли в «{TicketStatusText.Of(t.Status)}» — перенесите её снова, если статус нужен", "");
            return;
        }
        // пока было открыто окно, запись выключили или сменили сервер или логин — выключатель главнее «Сменить статус»
        if (!_settings.AllowStatusWrite || _intraservice is null || _settings.AccountKey != account)
        {
            NotChanged(n, title, "пока было открыто окно, поменялись настройки — перенесите карточку снова");
            return;
        }

        var r = await client.ChangeStatusAsync(n, target.Id, form.Comment.Trim(), form.Internal, task.ChangedText);
        // как бы ни кончилось — карточку заново: статус на ней — какой стал в Интрасервисе (и автообновление не пришлёт «закрыта»
        // о своём же действии), переписку — тоже
        var fresh = await client.GetTaskAsync(n);
        if (fresh.Task is { } x && _settings.AccountKey == account) Apply(t, n, x);
        _commentCache.Remove(n);
        if (SelectedTicket == t) LoadComments(t, force: true);
        switch (r.Outcome)
        {
            case WriteOutcome.Done:
                if (SelectedTicket == t) SyncMessage = $"статус в Интрасервисе: «{target.Name}»";
                break;
            case WriteOutcome.Refused:
                NotChanged(n, title, r.Error + (r.Conflict ? "\nЗаявку изменили, пока было открыто окно: посмотрите переписку и перенесите карточку снова." : ""));
                break;
            default:
                Log?.Invoke($"Статус заявки #{n}: неизвестно, сменился ли: {r.Error}");   // текст комментария в журнал не пишем
                Views.AskWindow.Tell("Неизвестно, сменился ли статус",
                    $"Заявка #{n} «{title}»: {r.Error}\nКарточка перечитана — статус на ней такой, какой сейчас в Интрасервисе.");
                break;
        }
    }

    /// <summary>Статус в Интрасервисе не сменили — окном: перенос мог быть перетаскиванием при закрытой панели.</summary>
    private static void NotChanged(int n, string title, string why, string board = "\nНа доске карточка перенесена.") =>
        Views.AskWindow.Tell("Статус в Интрасервисе не изменён", $"Заявка #{n} «{title}»: {why}{board}");
}
