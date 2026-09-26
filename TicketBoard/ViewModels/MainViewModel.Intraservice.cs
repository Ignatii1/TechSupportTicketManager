using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Windows;
using System.Windows.Data;
using System.Windows.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Доска и Интрасервис: синхронизация одной карточки, импорт «моих» заявок, обновление статусов всех (F5). Всё только читает из Интрасервиса.</summary>
public sealed partial class MainViewModel
{
    // ---------- Интрасервис ----------

    [RelayCommand]
    private Task RefreshFromIntraservice() => SelectedTicket is Ticket t ? SyncAsync(t) : Task.CompletedTask;

    /// <summary>Статус из Интрасервиса; название — только если оно ещё автоматическое «Заявка #N», описание — только если пустое.</summary>
    private async Task SyncAsync(Ticket t)
    {
        if (SelectedTicket == t) SyncMessage = "обновляю…";
        string message;
        if (t.IntraserviceId is not int n) message = "у заявки нет номера";
        else if (_intraservice is not { } client) message = "API не настроен: трей → Настройки…";
        else
        {
            var r = await client.GetTaskAsync(n);
            message = r.Error;
            if (r.Task is IntraserviceTask x) Apply(t, n, x);
        }
        if (SelectedTicket == t) SyncMessage = message;
    }

    /// <summary>Что синхронизация меняет в карточке: статус Интрасервиса, инициатора, исполнителей и группу — всегда
    /// (поля нет в ответе — оставляет, что было); название — только пока оно автоматическое «Заявка #N»; описание —
    /// только пустое. Колонку, заметки и приоритет не трогает. Changed запоминает для проверки новых комментариев.</summary>
    private static void Apply(Ticket t, int n, IntraserviceTask x)
    {
        if (t.Title == $"Заявка #{n}" && x.Name.Length > 0) t.Title = x.Name;
        if (string.IsNullOrWhiteSpace(t.Description) && !string.IsNullOrEmpty(x.Description)) t.Description = x.Description;
        t.ExternalStatus = x.Status;
        if (x.Creator is not null) t.Creator = x.Creator;
        if (x.Executors is not null) t.Executors = x.Executors;
        if (x.ExecutorGroup is not null) t.ExecutorGroup = x.ExecutorGroup;
        if (x.Changed is not null) t.ServerChanged = x.Changed;
        t.LastSyncAt = DateTimeOffset.Now;
    }

    /// <summary>Строка списка (импорт, автообновление) как заявка — для Apply.</summary>
    private static IntraserviceTask AsTask(IntraserviceFound f) =>
        new(f.Id, f.Name, f.Status, f.Description, f.Creator, f.Executors, f.ExecutorGroup, f.Changed);

    /// <summary>Названия закрытых статусов из настроек. Список правится руками, поэтому терпим пустые строки,
    /// лишние пробелы и отсутствие самого списка.</summary>
    private HashSet<string> ClosedNames() => (_settings.ClosedStatusNames ?? Array.Empty<string>())
        .Where(n => !string.IsNullOrWhiteSpace(n)).Select(n => n.Trim()).ToHashSet(StringComparer.OrdinalIgnoreCase);

    // ---------- импорт моих заявок ----------

    /// <summary>Идёт импорт — кнопка на доске и пункт меню в трее на это время недоступны.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(BoardTitle))] private bool _isImporting;

    /// <summary>Заголовок окна. Импорт и обновление идут секунды, а итог приходит окном в конце — без подсказки
    /// кажется, что ничего не происходит.</summary>
    public string BoardTitle => IsImporting ? "Заявки — импорт…" : IsRefreshing ? "Заявки — обновляю статусы…"
        : AutoSyncState.Length > 0 ? $"Заявки · {AutoSyncState}" : "Заявки";

    private bool CanImport() => !IsImporting;

    /// <summary>Флаг сменился — доступность команды пересчитываем явно: сама она об этом не узнает.</summary>
    partial void OnIsImportingChanged(bool value) => ImportMineCommand.NotifyCanExecuteChanged();

    /// <summary>Сколько страниц списка тянем за один импорт.</summary>
    private const int ImportPages = 10;

    /// <summary>Тянет с сервера открытые заявки, где исполнитель — текущий пользователь, и кладёт их во «Входящие».
    /// Только вручную (трей и кнопка на доске), только вниз: в Интрасервис не пишем. Заявку, которая уже есть
    /// на доске, не трогает вовсе — ни колонку, ни заметки, ни приоритет, — а лишь считает её в «уже было»,
    /// поэтому импорт можно запускать сколько угодно раз.</summary>
    [RelayCommand(CanExecute = nameof(CanImport))]
    private async Task ImportMine()
    {
        IsImporting = true;
        try
        {
            if (_intraservice is not { } client) { Report(ImportTitle, "API не настроен: трей → Настройки…"); return; }

            var mine = await FetchMyOpenAsync(client);
            if (mine.Rows.Count == 0)
            {
                Report(ImportTitle, mine.Error.Length > 0 ? mine.Error : "открытых заявок, где вы исполнитель, не нашлось");
                return;
            }

            var onBoard = BoardIds();
            var inbox = ColumnFor(TicketStatus.Inbox);
            var now = DateTimeOffset.Now;
            var added = new List<int>();
            var had = 0;
            foreach (var f in mine.Rows)
            {
                if (!onBoard.Add(f.Id)) { had++; continue; }   // уже на доске (или пришла дважды) — не трогаем её
                inbox.Items.Add(NewCard(f, now));   // без MarkAppear: полсотни карточек, влетающих разом, — шум, а не подсказка
                added.Add(f.Id);
            }
            if (added.Count > 0)
            {
                ScheduleSave();          // одно сохранение на весь импорт, а не на каждую заявку
                AllowAutoSync(added);    // вернули удалённые — автообновление снова их ведёт; settings.json — тоже один раз
            }

            // молчаливый обрыв хуже недогруза: и ошибка, и упёршийся потолок страниц должны быть видны
            var partial = mine.Error.Length > 0 ? $"\nЗагружены не все страницы: {mine.Error}"
                : !mine.Complete ? $"\nВзяты первые {mine.Rows.Count} из {mine.Total} — запустите импорт ещё раз"
                : "";
            Report(ImportTitle, $"Добавлено: {added.Count}, уже было: {had}{partial}");
        }
        finally { IsImporting = false; }
    }

    /// <summary>Мои открытые заявки — для импорта и автообновления. Rows пуст, Error не пуст — не вышло вовсе; оба не пусты —
    /// пришли не все страницы. Complete — список целый (правило — AutoSyncRules.ReadAllPagesAsync); только по целому
    /// автообновление решает, что заявка перестала быть моей. Closed — закрытые статусы: признаки сервера («выполнена»,
    /// «конечный») плюс ClosedStatusNames.</summary>
    private sealed record MyOpenTickets(IReadOnlyList<IntraserviceFound> Rows, int Total, bool Complete, string Error,
        HashSet<string> Closed);

    /// <summary>Кто я на сервере — спрашиваем раз за запуск; сменили адрес или логин — ApplySettings сбрасывает.
    /// Номер — для списка «мои открытые», номер и имя — чтобы не считать новыми свои же комментарии.</summary>
    private IntraserviceUser? _me;

    private async Task<MyOpenTickets> FetchMyOpenAsync(HttpIntraserviceClient client)
    {
        static MyOpenTickets Fail(string error) => new(Array.Empty<IntraserviceFound>(), 0, false, error, new());

        // запоминаем, только если клиент всё ещё текущий: пока шёл запрос, могли сохранить настройки с другим логином
        if (_me is not { } me)
        {
            var (user, userError) = await client.GetCurrentUserAsync();
            if (user is null) return Fail(userError);
            me = user;
            if (ReferenceEquals(_intraservice, client)) _me = user;
        }

        // справочник — каждый раз, без кэша: новый открытый статус, не попавший в фильтр, выбросил бы свои заявки из
        // списка, и автообновление сочло бы их не моими
        var (statuses, statusError) = await client.GetStatusesAsync();
        if (statusError.Length > 0) return Fail(statusError);
        // пустой справочник и «все статусы закрытые» — разные беды: первая на сервере, вторую чинит сам пользователь
        if (statuses.Count == 0) return Fail("сервер не вернул ни одного статуса заявок");

        var closed = ClosedNames();
        var openIds = statuses.Where(s => !s.IsFixed && !s.IsFinal && !closed.Contains(s.Name)).Select(s => s.Id).ToList();
        if (openIds.Count == 0) return Fail("все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json");
        closed.UnionWith(statuses.Where(s => s.IsFixed || s.IsFinal).Select(s => s.Name));

        // ponytail: потолок 10 страниц по 200 — 2000 заявок; упрётся — добавить постраничную докачку.
        var (rows, total, complete, error) = await AutoSyncRules.ReadAllPagesAsync(
            page => client.GetExecutorTasksAsync(me.Id, openIds, page), HttpIntraserviceClient.ExecutorPageSize, ImportPages);
        return new(rows, total, complete, error, closed);
    }

    private HashSet<int> BoardIds() =>
        AllTickets.Where(t => t.IntraserviceId is not null).Select(t => t.IntraserviceId!.Value).ToHashSet();

    /// <summary>Карточка из строки списка сервера. Все поля — до Track: иначе каждое присваивание заведёт таймер
    /// сохранения. В колонку кладёт и сохраняет вызывающий — один раз на пачку.</summary>
    private Ticket NewCard(IntraserviceFound f, DateTimeOffset now)
    {
        var t = new Ticket
        {
            Title = f.Name,
            IntraserviceId = f.Id,
            Url = TicketUrl(f.Id),
            Description = f.Description ?? "",
            ExternalStatus = f.Status,
            LastSyncAt = now,
            Priority = TicketPriority.Mid,   // приоритеты сервера в компании не заполняют
            Creator = f.Creator,
            Executors = f.Executors,
            ExecutorGroup = f.ExecutorGroup,
            // переписка до появления на доске — прочитана: новым будет только то, что напишут после этого Changed
            ServerChanged = f.Changed,
            CommentsCheckedFor = f.Changed,
            CommentsSeenAt = f.Changed,
        };
        Track(t);
        return t;
    }

    // ---------- обновить статусы всех карточек ----------

    /// <summary>Идёт обновление — пункт в трее и F5 на это время недоступны.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(BoardTitle))] private bool _isRefreshing;

    private bool CanRefresh() => !IsRefreshing;

    partial void OnIsRefreshingChanged(bool value) => RefreshAllCommand.NotifyCanExecuteChanged();

    /// <summary>Перечитывает из Интрасервиса все карточки с номером, кроме «Готово», и предлагает перенести в «Готово»
    /// те, что там уже закрыты, — по тому же правилу, что и импорт: признаки статуса плюс ClosedStatusNames.
    /// Импорт — половина петли: без этого закрытая днём заявка висела бы «В работе». Только по запросу (трей, F5) —
    /// автообновление перечитывает тихо и спрашивает по щелчку на уведомлении; в Интрасервис ничего не пишет; без согласия
    /// ничего не переносит.</summary>
    [RelayCommand(CanExecute = nameof(CanRefresh))]
    private async Task RefreshAll()
    {
        IsRefreshing = true;
        try
        {
            if (_intraservice is not { } client) { Report(RefreshTitle, "API не настроен: трей → Настройки…"); return; }

            var cards = AllTickets.Where(t => t.IntraserviceId is not null && t.Status != TicketStatus.Done).ToList();
            if (cards.Count == 0) { Report(RefreshTitle, "на доске нет заявок с номером — обновлять нечего"); return; }

            // справочник статусов и карточки друг от друга не зависят — идут параллельно
            var statusesTask = client.GetStatusesAsync();

            // ponytail: запрос на карточку, по 4 разом; карточек станут сотни — один список по ChangedMoreThan
            var (fresh, failed) = await RecheckAsync(client, cards);   // fresh: по статусу недельной давности не переносим
            _commentCache.Clear();   // статусы сменились — переписка, скорее всего, тоже

            // закрытые: признаки с сервера плюс свой список; справочник не пришёл — обойдёмся списком
            var (statuses, statusError) = await statusesTask;
            var closed = ClosedNames();
            closed.UnionWith(statuses.Where(s => s.IsFixed || s.IsFinal).Select(s => s.Name));

            // в окне без вопроса — ошибки целиком; в окне с вопросом — коротко (Brief): там главное — список переносимых
            // заявок, а многострочный ответ сервера оттеснил бы его вниз. Целиком ошибка всё равно в errors.log
            string Summary(Func<string, string> show) => $"Обновлено: {fresh.Count}"
                + (failed.Count > 0 ? $", не удалось: {failed.Count}. Первая ошибка — #{failed[0].Id}: {show(failed[0].Error)}" : "")
                + (statusError.Length > 0 ? $"\n\nСправочник статусов не получен — закрытые определены только по списку:\n{show(statusError)}" : "");
            var closedNow = ClosedToMove(fresh, closed);
            if (closedNow.Count == 0) { Report(RefreshTitle, Summary(e => e)); return; }
            AskMoveClosed(closedNow, $"{Summary(HttpIntraserviceClient.Brief)}\n\n");
        }
        finally { IsRefreshing = false; }
    }

    /// <summary>Перечитать карточки по одной, по 4 разом, и применить (Apply) — у F5 и автообновления. Fresh — перечитанные
    /// сейчас; Failed — номер и ошибка каждой неудачной, чтобы «не удалось: 3» было с причиной, а не гаданием.</summary>
    private async Task<(List<Ticket> Fresh, List<(int Id, string Error)> Failed)> RecheckAsync(HttpIntraserviceClient client,
        IReadOnlyList<Ticket> cards)
    {
        using var gate = new SemaphoreSlim(4);
        var fresh = new List<Ticket>();   // продолжения возвращаются в UI-поток, поэтому без блокировок
        var failed = new List<(int Id, string Error)>();
        await Task.WhenAll(cards.Select(async t =>
        {
            await gate.WaitAsync();
            try
            {
                var n = t.IntraserviceId!.Value;
                var r = await client.GetTaskAsync(n);
                if (r.Task is IntraserviceTask x) { Apply(t, n, x); fresh.Add(t); }
                else failed.Add((n, r.Error));
            }
            finally { gate.Release(); }
        }));
        return (fresh, failed);
    }

    /// <summary>Какие из только что перечитанных карточек закрыты в Интрасервисе и ждут переноса: всё ещё на доске
    /// (пока шли запросы, её могли удалить), не в «Готово» и не оставленные с этим статусом (Ticket.KeptOpenStatus) —
    /// иначе каждое F5 спрашивало бы о них снова.</summary>
    private List<Ticket> ClosedToMove(IEnumerable<Ticket> fresh, HashSet<string> closed)
    {
        var onBoard = AllTickets.ToHashSet();
        return fresh.Where(t => onBoard.Contains(t) && t.Status != TicketStatus.Done
            && t.ExternalStatus is { } st && closed.Contains(st) && t.KeptOpenStatus != st).ToList();
    }

    /// <summary>Вопрос «перенести закрытые в «Готово»?» — у F5 и по щелчку на уведомлении автообновления.
    /// Перечисляем, что именно переносим: «Да» на неизвестно что — не согласие. «Оставить» помнит карточка.</summary>
    private void AskMoveClosed(IReadOnlyList<Ticket> closedNow, string preface)
    {
        if (closedNow.Count == 0) return;
        var list = string.Join("\n", closedNow.Take(10).Select(t => $"{t.DisplayNumber}  {t.Title}"))
            + (closedNow.Count > 10 ? $"\n…и ещё {closedNow.Count - 10}" : "");
        var move = Views.AskWindow.Ask("Перенести закрытые в «Готово»?",
            $"{preface}Закрыты в Интрасервисе ({closedNow.Count}):\n{list}", "Перенести", "Оставить");
        if (!move)
        {
            foreach (var t in closedNow) t.KeptOpenStatus = t.ExternalStatus;
            ShowAutoSyncState();   // оставленные — не в счёт; перенос пересчитает сам (Recount)
            return;
        }

        // пока висел вопрос, доска жила дальше (вложенный цикл сообщений: автообновление, трей) —
        // карточку могли удалить, проверяем ещё раз
        var target = ColumnFor(TicketStatus.Done);
        var stillOnBoard = AllTickets.ToHashSet();
        foreach (var t in closedNow)
            if (stillOnBoard.Contains(t)) MoveTicket(t, target, afterMove: false);   // уже в «Готово» — MoveTicket не тронет
        AfterMove();   // одна перефильтровка и одно сохранение на всю пачку
    }

    private const string ImportTitle = "Импорт моих заявок";
    private const string RefreshTitle = "Обновление статусов";

    /// <summary>Итог импорта или обновления, либо его ошибка — окном: их запускают руками и ждут ответа.</summary>
    private static void Report(string heading, string text) => Views.AskWindow.Tell(heading, text);
}
