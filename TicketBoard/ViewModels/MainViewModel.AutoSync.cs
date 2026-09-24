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

    /// <summary>Итог последнего автообновления для заголовка доски: «обновлено 12:05» (и сколько карточек закрыты в
    /// Интрасервисе, но не в «Готово») или «не удалось обновить».</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(BoardTitle))] private string _autoSyncState = "";

    private DispatcherTimer? _autoSyncTimer;
    private bool _autoSyncing;
    private string _autoSyncError = "";   // в лог — только новая ошибка, а не одна и та же каждые 5 минут
    private DateTimeOffset? _autoSyncAt;              // последний удачный заход; null — заголовок не про «обновлено»
    private HashSet<string> _autoSyncClosed = new();  // закрытые статусы с того захода — для счётчика в заголовке

    /// <summary>ponytail: карточки, выпавшие из списка моих открытых, перечитываются по одной — не больше стольких за раз,
    /// по кругу. Чужих заявок на доске станут десятки — перейти на один список по номерам.</summary>
    private const int AutoSyncRecheckLimit = 20;

    /// <summary>Когда автообновление последний раз пробовало перечитать заявку (номер → время). Очередь — по попыткам, а не
    /// по LastSyncAt: удалённая на сервере заявка не перечитается никогда и иначе вечно стояла бы первой, оттесняя другие.</summary>
    private readonly Dictionary<int, DateTimeOffset> _rechecked = new();

    /// <summary>Номера, чей сбой перечитывания уже в логе: пишем, когда заявка начала сбоить, а не каждый заход.</summary>
    private readonly HashSet<int> _recheckFailed = new();

    /// <summary>Из ApplySettings: включить, выключить или сменить период. Первый заход — вскоре после запуска.</summary>
    private void ApplyAutoSync()
    {
        _autoSyncTimer ??= NewAutoSyncTimer();
        _autoSyncTimer.Stop();
        if (_settings.AutoSyncMinutes <= 0 || _intraservice is null) { (_autoSyncAt, AutoSyncState) = (null, ""); return; }
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
            if (!StillCurrent(client)) return;
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
            var skip = AutoSyncSkip(listed.Keys, onBoard, mine.Complete);
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

            // выпали из моих открытых — закрыты, переданы другому или вовсе не мои: перечитываем по одной, по кругу
            var recheck = cards.Where(t => t.Status != TicketStatus.Done && !listed.ContainsKey(t.IntraserviceId!.Value))
                .OrderBy(t => _rechecked.GetValueOrDefault(t.IntraserviceId!.Value, DateTimeOffset.MinValue))
                .ThenBy(t => t.LastSyncAt ?? DateTimeOffset.MinValue)
                .Take(AutoSyncRecheckLimit).ToList();
            foreach (var t in recheck) _rechecked[t.IntraserviceId!.Value] = now;
            var before = recheck.ToDictionary(t => t, t => t.ExternalStatus ?? "");
            var (fresh, failed) = await RecheckAsync(client, recheck);
            if (!StillCurrent(client)) return;
            LogRecheckFailures(fresh, failed);

            // о закрытой — уведомление один раз, когда статус стал закрытым (в том числе пока компьютер был выключен), а
            // не каждый заход, пока она висит закрытой; пропустили его — остаётся счётчик в заголовке. Идёт F5 — он
            // спрашивает о них сам. Сам ничего не переносит
            var closedNow = IsRefreshing ? new List<Ticket>()
                : ClosedToMove(fresh, mine.Closed).Where(t => !mine.Closed.Contains(before[t])).ToList();
            NotifyChanges(added, closedNow, mine.Closed);

            if (mine.Error.Length > 0) AutoSyncFailed(mine.Error);   // пришли не все страницы — что пришло, уже разобрано
            else
            {
                (_autoSyncError, _autoSyncAt, _autoSyncClosed) = ("", now, mine.Closed);
                ShowAutoSyncState();
            }
        }
        catch (Exception ex) { AutoSyncFailed(ex.ToString()); }   // по таймеру: ни окон, ни падений — заголовок и лог
        finally { _autoSyncing = false; }
    }

    /// <summary>Пока шли запросы, сохранили настройки (другой клиент) или выключили автообновление — итог захода не
    /// применяем: ни уведомлений, ни «обновлено» в заголовке, который ApplyAutoSync только что очистил.</summary>
    private bool StillCurrent(HttpIntraserviceClient client) => ReferenceEquals(_intraservice, client) && _settings.AutoSyncMinutes > 0;

    /// <summary>Одно уведомление на заход: у Windows щелчок приходит без указания, по какому уведомлению, — два подряд
    /// перепутали бы действия. По щелчку App открывает доску; есть закрытые — ещё и вопрос F5 о переносе.</summary>
    private void NotifyChanges(IReadOnlyList<Ticket> added, IReadOnlyList<Ticket> closedNow, HashSet<string> closed)
    {
        if (added.Count == 0 && closedNow.Count == 0) return;
        var title = added.Count > 0 && closedNow.Count > 0 ? $"Новые заявки на вас: {added.Count}, закрыты: {closedNow.Count}"
            : added.Count > 0 ? (added.Count == 1 ? "Новая заявка на вас" : $"Новые заявки на вас: {added.Count}")
            : closedNow.Count == 1 ? "Заявка закрыта в Интрасервисе" : $"Закрыты в Интрасервисе: {closedNow.Count}";
        // что сделает щелчок — первой строкой: длинный текст обрезается с конца
        var text = closedNow.Count == 0 ? Titles(added)
            : $"Щёлкните, чтобы перенести в «Готово»:\n{Titles(closedNow)}" + (added.Count > 0 ? $"\nНовые: {Titles(added)}" : "");
        Notify?.Invoke(title, text, closedNow.Count == 0 ? null
            // к щелчку что-то могли уже перенести руками; идёт F5 — у него свой такой же вопрос
            : () => { if (!IsRefreshing) AskMoveClosed(ClosedToMove(closedNow, closed), ""); });
    }

    /// <summary>Заголовок после удачного захода: когда обновлено и сколько карточек закрыты в Интрасервисе, но не в «Готово»
    /// и не оставлены — уведомление о них могли пропустить. Зовётся и после переносов и смены статусов (Recount),
    /// и после «Оставить» (AskMoveClosed), иначе сразу после F5 висело бы «закрыты: 2».</summary>
    private void ShowAutoSyncState()
    {
        if (_autoSyncAt is not { } at) return;
        var waiting = ClosedToMove(AllTickets.Where(t => t.IntraserviceId is not null), _autoSyncClosed).Count;
        AutoSyncState = $"обновлено {at:HH:mm}" + (waiting > 0 ? $" · закрыты в Интрасервисе: {waiting} — F5" : "");
    }

    private void AutoSyncFailed(string error)
    {
        (_autoSyncAt, AutoSyncState) = (null, "не удалось обновить");
        if (error == _autoSyncError) return;   // сервер лежит — одна запись в логе, а не каждые 5 минут
        _autoSyncError = error;
        Log?.Invoke($"Автообновление: {error}");
    }

    /// <summary>Не перечиталась — в лог со своей ошибкой, но по разу на заявку, пока она снова не перечитается: удалённая
    /// на сервере иначе писала бы в errors.log каждые 5 минут. Заголовок доски не трогаем — список моих открытых пришёл.</summary>
    private void LogRecheckFailures(IReadOnlyList<Ticket> fresh, IReadOnlyList<(int Id, string Error)> failed)
    {
        foreach (var t in fresh) _recheckFailed.Remove(t.IntraserviceId!.Value);
        var first = failed.Where(f => _recheckFailed.Add(f.Id)).ToList();
        if (first.Count > 0)
            Log?.Invoke("Автообновление: не удалось перечитать\n" + string.Join("\n", first.Select(f => $"#{f.Id}: {f.Error}")));
    }

    /// <summary>Номера, которые автообновление не добавляет (правило — AutoSyncRules.Skip); изменились — в settings.json.</summary>
    private HashSet<int> AutoSyncSkip(IReadOnlyCollection<int> listed, IReadOnlySet<int> onBoard, bool complete)
    {
        var saved = _settings.AutoSyncSkipIds;
        var (skip, persist) = AutoSyncRules.Skip(saved, listed, onBoard, complete);
        if (persist && (saved is null || !skip.SetEquals(saved))) SaveSkip(skip);
        return skip;
    }

    /// <summary>Карточку удалили с доски — автообновление не вернёт её, пока заявка открыта.</summary>
    private void SkipOnAutoSync(int id)
    {
        if (_settings.AutoSyncSkipIds is not { } saved || saved.Contains(id)) return;   // ещё не запускалось — первый запуск учтёт сам
        SaveSkip(saved.Append(id).ToHashSet());
    }

    /// <summary>Заявки вернули на доску руками (импорт, быстрое добавление) — автообновление снова их ведёт.
    /// settings.json — один раз на пачку и только если что-то поменялось.</summary>
    private void AllowAutoSync(IEnumerable<int> ids)
    {
        if (_settings.AutoSyncSkipIds is not { } saved) return;
        var skip = saved.ToHashSet();
        var count = skip.Count;
        skip.ExceptWith(ids);
        if (skip.Count < count) SaveSkip(skip);
    }

    private void SaveSkip(HashSet<int> skip)
    {
        _settings.AutoSyncSkipIds = skip.Order().ToArray();
        try { _settings.Save(App.DataDir); }
        catch (Exception ex) { Log?.Invoke($"Не удалось сохранить settings.json: {ex}"); }   // в памяти список верный — до следующего раза
    }

    /// <summary>Три строки на список: у Windows под весь текст уведомления 255 символов.</summary>
    private static string Titles(IReadOnlyList<Ticket> list) =>
        string.Join("\n", list.Take(3).Select(t => $"{t.DisplayNumber} {(t.Title.Length > 60 ? t.Title[..60] + "…" : t.Title)}"))
        + (list.Count > 3 ? $"\n…и ещё {list.Count - 3}" : "");
}
