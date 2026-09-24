using System.Windows.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Автообновление: раз в AutoSyncMinutes тихо делает то же, что импорт и F5, только без окон. Новые заявки на
/// меня — во «Входящие», статусы — на карточки, о закрытых — уведомление; по щелчку — тот же вопрос, что у F5.
/// Сам ничего не переносит и в Интрасервис не пишет.</summary>
public sealed partial class MainViewModel
{
    /// <summary>Уведомление в трее: заголовок, текст и что сделать по щелчку (App сначала открывает доску).</summary>
    public event Action<string, string, Action?>? Notify;

    /// <summary>Запись в errors.log — App подставляет свой AppendLog.</summary>
    public Action<string>? Log { get; set; }

    /// <summary>Итог последнего автообновления для заголовка доски: «обновлено 12:05» или «не удалось обновить».</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(BoardTitle))] private string _autoSyncState = "";

    private DispatcherTimer? _autoSyncTimer;
    private bool _autoSyncing;
    private string _autoSyncError = "";   // в лог — только новая ошибка, а не одна и та же каждые 5 минут

    /// <summary>О закрытии этой заявки с этим статусом уже сказали — второй раз не надоедаем.</summary>
    private readonly HashSet<(int Id, string Status)> _closedNotified = new();

    /// <summary>ponytail: карточки, выпавшие из списка моих открытых, перечитываются по одной — не больше стольких за раз,
    /// самые давние первыми. Чужих заявок на доске станут десятки — перейти на один список по номерам.</summary>
    private const int AutoSyncRecheckLimit = 20;

    /// <summary>Из ApplySettings: включить, выключить или сменить период. Первый заход — вскоре после запуска.</summary>
    private void ApplyAutoSync()
    {
        _autoSyncTimer ??= NewAutoSyncTimer();
        _autoSyncTimer.Stop();
        if (_settings.AutoSyncMinutes <= 0 || _intraservice is null) { AutoSyncState = ""; return; }
        _autoSyncTimer.Interval = TimeSpan.FromSeconds(15);
        _autoSyncTimer.Start();
    }

    private DispatcherTimer NewAutoSyncTimer()
    {
        var timer = new DispatcherTimer();
        timer.Tick += async (_, _) =>
        {
            timer.Interval = TimeSpan.FromMinutes(Math.Max(1, _settings.AutoSyncMinutes));
            await AutoSyncAsync();
        };
        return timer;
    }

    private async Task AutoSyncAsync()
    {
        // ручные импорт и F5 идут со своими окнами — не мешаем; прошлый заход ещё идёт — тоже
        if (_autoSyncing || IsImporting || IsRefreshing || _intraservice is not { } client) return;
        _autoSyncing = true;
        try
        {
            var mine = await FetchMyOpenAsync(client);
            if (mine.Rows.Count == 0 && mine.Error.Length > 0) { AutoSyncFailed(mine.Error); return; }
            var now = DateTimeOffset.Now;
            var listed = new Dictionary<int, IntraserviceFound>();
            foreach (var f in mine.Rows) listed.TryAdd(f.Id, f);
            var cards = AllTickets.Where(t => t.IntraserviceId is not null).ToList();

            // мои открытые, что уже на доске, — по общему правилу синхронизации (Apply)
            foreach (var t in cards)
                if (listed.TryGetValue(t.IntraserviceId!.Value, out var f))
                    Apply(t, f.Id, new IntraserviceTask(f.Id, f.Name, f.Status, f.Description));

            // новые — во «Входящие», кроме удалённых с доски и открытых до первого автообновления (их приносит импорт)
            var onBoard = cards.Select(t => t.IntraserviceId!.Value).ToHashSet();
            var skip = AutoSyncSkip(listed.Keys, onBoard, complete: mine.Error.Length == 0);
            var inbox = ColumnFor(TicketStatus.Inbox);
            var added = new List<Ticket>();
            foreach (var id in AutoSyncRules.ToAdd(listed.Keys, onBoard, skip))
            {
                var t = NewCard(listed[id], now);
                t.MarkAppear();
                inbox.Items.Insert(0, t);
                added.Add(t);
            }
            if (added.Count > 0) ScheduleSave();

            // выпали из моих открытых — закрыты, переданы другому или вовсе не мои: перечитываем по одной
            var recheck = cards.Where(t => t.Status != TicketStatus.Done && !listed.ContainsKey(t.IntraserviceId!.Value))
                .OrderBy(t => t.LastSyncAt ?? DateTimeOffset.MinValue).Take(AutoSyncRecheckLimit).ToList();
            var fresh = new List<Ticket>();   // продолжения возвращаются в UI-поток, поэтому без блокировок
            using (var gate = new SemaphoreSlim(4))
                await Task.WhenAll(recheck.Select(async t =>
                {
                    await gate.WaitAsync();
                    try
                    {
                        var n = t.IntraserviceId!.Value;
                        if ((await client.GetTaskAsync(n)).Task is IntraserviceTask x) { Apply(t, n, x); fresh.Add(t); }
                    }
                    finally { gate.Release(); }
                }));

            // закрытые — по разу на номер и статус; сам ничего не переносит
            var closedNow = ClosedToMove(fresh, mine.Closed).Where(t => _closedNotified.Add((t.IntraserviceId!.Value, t.ExternalStatus!))).ToList();
            NotifyChanges(added, closedNow, mine.Closed);

            if (mine.Error.Length > 0) AutoSyncFailed(mine.Error);   // пришли не все страницы — что пришло, уже разобрано
            else
            {
                _autoSyncError = "";
                AutoSyncState = $"обновлено {now:HH:mm}";
            }
        }
        catch (Exception ex) { AutoSyncFailed(ex.ToString()); }   // по таймеру: ни окон, ни падений — заголовок и лог
        finally { _autoSyncing = false; }
    }

    /// <summary>Одно уведомление на заход: у Windows щелчок приходит без указания, по какому уведомлению, — два подряд
    /// перепутали бы действия. По щелчку App открывает доску; есть закрытые — ещё и вопрос F5 о переносе.</summary>
    private void NotifyChanges(IReadOnlyList<Ticket> added, IReadOnlyList<Ticket> closedNow, HashSet<string> closed)
    {
        if (added.Count == 0 && closedNow.Count == 0) return;
        var title = added.Count > 0 && closedNow.Count > 0 ? $"Новые заявки на вас: {added.Count}, закрыты: {closedNow.Count}"
            : added.Count > 0 ? (added.Count == 1 ? "Новая заявка на вас" : $"Новые заявки на вас: {added.Count}")
            : closedNow.Count == 1 ? "Заявка закрыта в Интрасервисе" : $"Закрыты в Интрасервисе: {closedNow.Count}";
        var text = added.Count == 0 ? "" : Titles(added);
        if (closedNow.Count > 0)
            text += (text.Length > 0 ? "\nЗакрыты: " : "") + $"{Titles(closedNow)}\nЩёлкните, чтобы перенести в «Готово»";
        Notify?.Invoke(title, text, closedNow.Count == 0 ? null
            // к щелчку что-то могли уже перенести руками; идёт F5 — у него свой такой же вопрос
            : () => { if (!IsRefreshing) AskMoveClosed(ClosedToMove(closedNow, closed), ""); });
    }

    private void AutoSyncFailed(string error)
    {
        AutoSyncState = "не удалось обновить";
        if (error == _autoSyncError) return;   // сервер лежит — одна запись в логе, а не каждые 5 минут
        _autoSyncError = error;
        Log?.Invoke($"Автообновление: {error}");
    }

    /// <summary>Номера, которые автообновление не добавляет (правило — AutoSyncRules.Skip); изменились — в settings.json.</summary>
    private HashSet<int> AutoSyncSkip(IReadOnlyCollection<int> listed, IReadOnlySet<int> onBoard, bool complete)
    {
        var saved = _settings.AutoSyncSkipIds;
        var skip = AutoSyncRules.Skip(saved, listed, onBoard, complete);
        if (saved is null || !skip.SetEquals(saved)) SaveSkip(skip);
        return skip;
    }

    /// <summary>Карточку удалили с доски — автообновление не вернёт её, пока заявка открыта.</summary>
    private void SkipOnAutoSync(int id)
    {
        if (_settings.AutoSyncSkipIds is not { } saved || saved.Contains(id)) return;   // ещё не запускалось — первый запуск учтёт сам
        SaveSkip(saved.Append(id).ToHashSet());
    }

    /// <summary>Заявку вернули на доску руками (импорт, быстрое добавление) — автообновление снова её ведёт.</summary>
    private void AllowAutoSync(int id)
    {
        if (_settings.AutoSyncSkipIds is { } saved && saved.Contains(id)) SaveSkip(saved.Where(x => x != id).ToHashSet());
    }

    private void SaveSkip(HashSet<int> skip)
    {
        _settings.AutoSyncSkipIds = skip.Order().ToArray();
        try { _settings.Save(App.DataDir); }
        catch (Exception ex) { Log?.Invoke($"Не удалось сохранить settings.json: {ex}"); }   // в памяти список верный — до следующего раза
    }

    /// <summary>Три строки для уведомления: Windows режет текст после ~250 символов.</summary>
    private static string Titles(IReadOnlyList<Ticket> list) =>
        string.Join("\n", list.Take(3).Select(t => $"{t.DisplayNumber} {(t.Title.Length > 60 ? t.Title[..60] + "…" : t.Title)}"))
        + (list.Count > 3 ? $"\n…и ещё {list.Count - 3}" : "");
}
