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

    /// <summary>Что синхронизация меняет в карточке: статус Интрасервиса — всегда; название — только пока оно
    /// автоматическое «Заявка #N»; описание — только пустое. Колонку, заметки и приоритет не трогает.</summary>
    private static void Apply(Ticket t, int n, IntraserviceTask x)
    {
        if (t.Title == $"Заявка #{n}" && x.Name.Length > 0) t.Title = x.Name;
        if (string.IsNullOrWhiteSpace(t.Description) && !string.IsNullOrEmpty(x.Description)) t.Description = x.Description;
        t.ExternalStatus = x.Status;
        t.LastSyncAt = DateTimeOffset.Now;
    }

    /// <summary>Названия закрытых статусов из настроек. Список правится руками, поэтому терпим пустые строки,
    /// лишние пробелы и отсутствие самого списка.</summary>
    private HashSet<string> ClosedNames() => (_settings.ClosedStatusNames ?? Array.Empty<string>())
        .Where(n => !string.IsNullOrWhiteSpace(n)).Select(n => n.Trim()).ToHashSet(StringComparer.OrdinalIgnoreCase);

    // ---------- импорт моих заявок ----------

    /// <summary>Идёт импорт — кнопка на доске и пункт меню в трее на это время недоступны.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(BoardTitle))] private bool _isImporting;

    /// <summary>Заголовок окна. Импорт и обновление идут секунды, а итог приходит окном в конце — без подсказки
    /// кажется, что ничего не происходит.</summary>
    public string BoardTitle => IsImporting ? "Заявки — импорт…" : IsRefreshing ? "Заявки — обновляю статусы…" : "Заявки";

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

            var (userId, userError) = await client.GetCurrentUserIdAsync();
            if (userId is not int me) { Report(ImportTitle, userError); return; }

            var (statuses, statusError) = await client.GetStatusesAsync();
            if (statusError.Length > 0) { Report(ImportTitle, statusError); return; }
            // пустой справочник статусов и «все статусы закрытые» — разные беды: первая на сервере, вторую чинит сам пользователь
            if (statuses.Count == 0) { Report(ImportTitle, "сервер не вернул ни одного статуса заявок"); return; }

            var closed = ClosedNames();
            var openIds = statuses.Where(s => !s.IsFixed && !s.IsFinal && !closed.Contains(s.Name)).Select(s => s.Id).ToList();
            if (openIds.Count == 0)
            {
                Report(ImportTitle, "все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json");
                return;
            }

            // ponytail: потолок 10 страниц по 200 — 2000 заявок; упрётся — добавить постраничную докачку.
            var rows = new List<IntraserviceFound>();
            var error = "";
            var total = 0;
            for (var page = 1; page <= ImportPages; page++)
            {
                var r = await client.GetExecutorTasksAsync(me, openIds, page);
                if (r.Error.Length > 0) { error = r.Error; break; }  // что успели забрать — всё равно добавим
                if (r.Found.Count == 0) break;
                rows.AddRange(r.Found);
                total = Math.Max(total, r.Total);
                if (rows.Count >= r.Total) break;                    // забрали всё, что сервер обещал
            }
            if (rows.Count == 0 && error.Length > 0) { Report(ImportTitle, error); return; }
            if (rows.Count == 0) { Report(ImportTitle, "открытых заявок, где вы исполнитель, не нашлось"); return; }

            var onBoard = AllTickets.Where(x => x.IntraserviceId is not null).Select(x => x.IntraserviceId!.Value).ToHashSet();
            var inbox = ColumnFor(TicketStatus.Inbox);
            var now = DateTimeOffset.Now;
            int added = 0, had = 0;
            foreach (var f in rows)
            {
                if (!onBoard.Add(f.Id)) { had++; continue; }   // уже на доске (или пришла дважды) — не трогаем её
                // все поля — до Track: иначе каждое присваивание заведёт таймер сохранения
                var t = new Ticket
                {
                    Title = f.Name,
                    IntraserviceId = f.Id,
                    Url = TicketUrl(f.Id),
                    Description = f.Description ?? "",
                    ExternalStatus = f.Status,
                    LastSyncAt = now,
                    Priority = TicketPriority.Mid,   // приоритеты сервера пока не переносим
                };
                Track(t);
                inbox.Items.Add(t);   // без MarkAppear: полсотни карточек, влетающих разом, — шум, а не подсказка
                added++;
            }
            if (added > 0) ScheduleSave();   // одно сохранение на весь импорт, а не на каждую заявку

            // молчаливый обрыв хуже недогруза: и ошибка, и упёршийся потолок страниц должны быть видны
            var partial = error.Length > 0 ? $"\nЗагружены не все страницы: {error}"
                : rows.Count < total ? $"\nВзяты первые {rows.Count} из {total} — запустите импорт ещё раз"
                : "";
            Report(ImportTitle, $"Добавлено: {added}, уже было: {had}{partial}");
        }
        finally { IsImporting = false; }
    }

    // ---------- обновить статусы всех карточек ----------

    /// <summary>Идёт обновление — пункт в трее и F5 на это время недоступны.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(BoardTitle))] private bool _isRefreshing;

    private bool CanRefresh() => !IsRefreshing;

    partial void OnIsRefreshingChanged(bool value) => RefreshAllCommand.NotifyCanExecuteChanged();

    /// <summary>Перечитывает из Интрасервиса все карточки с номером, кроме «Готово», и предлагает перенести в «Готово»
    /// те, что там уже закрыты, — по тому же правилу, что и импорт: признаки статуса плюс ClosedStatusNames.
    /// Импорт — половина петли: после него доска сама не обновляется, и закрытая днём заявка висела бы «В работе».
    /// Только по запросу (трей, F5); в Интрасервис ничего не пишет; без согласия ничего не переносит.</summary>
    /// <summary>Закрытые в Интрасервисе карточки, о которых на этом запуске сказали «не переносить» — иначе каждое F5
    /// спрашивало бы о них снова. Ключ — номер и статус: заявку переоткроют и снова закроют — спросим снова.</summary>
    private readonly HashSet<(int Id, string Status)> _keptOpen = new();

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
            using var gate = new SemaphoreSlim(4);
            var fresh = new List<Ticket>();   // обновлённые сейчас: по статусу недельной давности переносить нельзя
            var failed = 0;                   // продолжения возвращаются в UI-поток, поэтому без блокировок
            var firstError = "";              // «не удалось: 3» без причины — это опять гадание
            await Task.WhenAll(cards.Select(async t =>
            {
                await gate.WaitAsync();
                try
                {
                    var n = t.IntraserviceId!.Value;
                    var r = await client.GetTaskAsync(n);
                    if (r.Task is IntraserviceTask x) { Apply(t, n, x); fresh.Add(t); }
                    else if (failed++ == 0) firstError = $"#{n}: {r.Error}";
                }
                finally { gate.Release(); }
            }));
            _commentCache.Clear();   // статусы сменились — переписка, скорее всего, тоже

            // закрытые: признаки с сервера плюс свой список; справочник не пришёл — обойдёмся списком
            var (statuses, statusError) = await statusesTask;
            var closed = ClosedNames();
            closed.UnionWith(statuses.Where(s => s.IsFixed || s.IsFinal).Select(s => s.Name));

            // в окне без вопроса — ошибки целиком; в окне с вопросом — коротко (Brief): там главное — список переносимых
            // заявок, а многострочный ответ сервера оттеснил бы его вниз. Целиком ошибка всё равно в errors.log
            string Summary(Func<string, string> show) => $"Обновлено: {fresh.Count}"
                + (failed > 0 ? $", не удалось: {failed}. Первая ошибка — {show(firstError)}" : "")
                + (statusError.Length > 0 ? $"\n\nСправочник статусов не получен — закрытые определены только по списку:\n{show(statusError)}" : "");
            // пока шли запросы, карточку могли удалить или перенести в «Готово» руками — в списке её быть не должно
            var onBoard = AllTickets.ToHashSet();
            var closedNow = fresh.Where(t => onBoard.Contains(t) && t.Status != TicketStatus.Done
                && t.ExternalStatus is { } st && closed.Contains(st)
                && !_keptOpen.Contains((t.IntraserviceId!.Value, st))).ToList();
            if (closedNow.Count == 0) { Report(RefreshTitle, Summary(e => e)); return; }

            // перечисляем, что именно предлагаем перенести: «Да» на неизвестно что — не согласие
            var list = string.Join("\n", closedNow.Take(10).Select(t => $"{t.DisplayNumber}  {t.Title}"))
                + (closedNow.Count > 10 ? $"\n…и ещё {closedNow.Count - 10}" : "");
            var move = Views.AskWindow.Ask("Перенести закрытые в «Готово»?",
                $"{Summary(HttpIntraserviceClient.Brief)}\n\nЗакрыты в Интрасервисе ({closedNow.Count}):\n{list}",
                "Перенести", "Оставить");
            if (!move)
            {
                foreach (var t in closedNow) _keptOpen.Add((t.IntraserviceId!.Value, t.ExternalStatus!));
                return;
            }

            // пока висел вопрос, доска жила дальше (вложенный цикл сообщений: фоновые синхронизации, трей) —
            // карточку могли удалить, проверяем ещё раз
            var target = ColumnFor(TicketStatus.Done);
            var stillOnBoard = AllTickets.ToHashSet();
            foreach (var t in closedNow)
                if (stillOnBoard.Contains(t)) MoveTicket(t, target, afterMove: false);   // уже в «Готово» — MoveTicket не тронет
            AfterMove();   // одна перефильтровка и одно сохранение на всю пачку
        }
        finally { IsRefreshing = false; }
    }

    private const string ImportTitle = "Импорт моих заявок";
    private const string RefreshTitle = "Обновление статусов";

    /// <summary>Итог импорта или обновления, либо его ошибка — окном: их запускают руками и ждут ответа.</summary>
    private static void Report(string heading, string text) => Views.AskWindow.Tell(heading, text);
}
