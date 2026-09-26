using System.Windows.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Автообновление: раз в AutoSyncMinutes тихо делает то же, что импорт и F5, только без окон. Новые заявки на
/// меня — во «Входящие», статусы — на карточки, о закрытых — уведомление; по щелчку — тот же вопрос, что у F5; новые
/// чужие комментарии — бейдж на карточке и уведомление. Сам ничего не переносит и в Интрасервис не пишет.</summary>
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

    /// <summary>ponytail: переписку перечитываем только у заявок, чей Changed сдвинулся, — не больше стольких за заход,
    /// по 4 разом, остальные — в следующий. Меняющихся за 5 минут заявок станут десятки — поднять.</summary>
    private const int AutoSyncCommentLimit = 10;

    /// <summary>Номера, чья ошибка чтения переписки уже в логе, — как _recheckFailed.</summary>
    private readonly HashSet<int> _commentsFailed = new();

    /// <summary>В списке заявок нет Changed — новые комментарии не отследить; в лог об этом — раз за запуск.</summary>
    private bool _noChangedLogged;

    /// <summary>Показать заявку на доске — по щелчку на уведомлении: MainWindow выделяет карточку в колонке.</summary>
    public event Action<Ticket>? RevealRequested;

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
                    Apply(t, f.Id, AsTask(f));
            if (!_noChangedLogged && mine.Rows.Count > 0 && mine.Rows.All(f => f.Changed is null))
            {
                _noChangedLogged = true;
                Log?.Invoke("Автообновление: в списке заявок нет поля Changed — новые комментарии не отслеживаются");
            }

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

            var commented = await CheckCommentsAsync(client, cards);
            if (!StillCurrent(client)) return;
            NotifyChanges(added, closedNow, commented, mine.Closed);

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

    /// <summary>Новые комментарии в заявках на доске. Changed сдвинулся с прошлой проверки — перечитываем переписку и
    /// пересчитываем бейдж (правило — AutoSyncRules.Unread); первая встреча с заявкой — только отсчёт, без запроса.
    /// Возвращает заявки, где чужих комментариев прибавилось, с этими комментариями (свежие первыми) — для уведомления.</summary>
    private async Task<List<(Ticket Ticket, IReadOnlyList<IntraserviceEvent> Comments)>> CheckCommentsAsync(
        HttpIntraserviceClient client, IReadOnlyList<Ticket> cards)
    {
        var due = new List<(Ticket Ticket, DateTimeOffset Changed)>();
        foreach (var t in cards)
        {
            if (t.ServerChanged is not { } changed || t.CommentsCheckedFor == changed) continue;
            if (t.CommentsCheckedFor is null)
            {
                t.CommentsSeenAt ??= changed;   // всё, что было до этого Changed, — прочитано
                t.CommentsCheckedFor = changed;
            }
            else due.Add((t, changed));
        }

        var news = new List<(Ticket, IReadOnlyList<IntraserviceEvent>)>();   // продолжения — в UI-потоке, без блокировок
        using var gate = new SemaphoreSlim(4);
        await Task.WhenAll(due.OrderBy(d => d.Ticket.CommentsCheckedFor).Take(AutoSyncCommentLimit).Select(async d =>
        {
            await gate.WaitAsync();
            try
            {
                var (t, n) = (d.Ticket, d.Ticket.IntraserviceId!.Value);
                var r = await client.GetLifetimeAsync(n);
                if (r.Error.Length > 0)
                {
                    // CommentsCheckedFor прежний — в следующий заход попробуем снова; в лог — раз, пока не пройдёт
                    if (_commentsFailed.Add(n)) Log?.Invoke($"Автообновление: не удалось прочитать переписку #{n}: {r.Error}");
                    return;
                }
                _commentsFailed.Remove(n);
                _commentCache[n] = (ToRows(r.Events), r.HasMore, DateTimeOffset.Now);   // панель покажет свежую без запроса
                var before = t.UnreadComments;
                var (count, seen, unread) = AutoSyncRules.Unread(r.Events, t.CommentsSeenAt, _me);
                (t.CommentsSeenAt, t.UnreadComments, t.CommentsCheckedFor) = (seen, count, d.Changed);
                if (count > before && AllTickets.Contains(t)) news.Add((t, unread.Take(count - before).ToList()));
                if (count > 0 && SelectedTicket == t) LoadComments(t);   // открыта — показать сразу (из кэша)
            }
            finally { gate.Release(); }
        }));
        return news;
    }

    /// <summary>Одно уведомление на заход: у Windows щелчок приходит без указания, по какому уведомлению, — два подряд
    /// перепутали бы действия. Щелчок: есть закрытые — вопрос F5 о переносе; иначе — показать заявку с новым
    /// комментарием или единственную новую. App перед этим открывает доску.</summary>
    private void NotifyChanges(IReadOnlyList<Ticket> added, IReadOnlyList<Ticket> closedNow,
        IReadOnlyList<(Ticket Ticket, IReadOnlyList<IntraserviceEvent> Comments)> commented, HashSet<string> closed)
    {
        var kinds = new[] { added.Count, closedNow.Count, commented.Count }.Count(n => n > 0);
        if (kinds == 0) return;
        var title = kinds > 1
            ? "Заявки — " + string.Join(" · ", new[]
            {
                added.Count > 0 ? $"новые: {added.Count}" : null,
                closedNow.Count > 0 ? $"закрыты: {closedNow.Count}" : null,
                commented.Count > 0 ? $"с комментариями: {commented.Count}" : null,
            }.OfType<string>())
            : added.Count > 0 ? (added.Count == 1 ? "Новая заявка на вас" : $"Новые заявки на вас: {added.Count}")
            : closedNow.Count > 0 ? (closedNow.Count == 1 ? "Заявка закрыта в Интрасервисе" : $"Закрыты в Интрасервисе: {closedNow.Count}")
            : commented.Count == 1 ? $"Новый комментарий в {commented[0].Ticket.DisplayNumber}"
            : $"Новые комментарии в заявках: {commented.Count}";

        // что сделает щелчок — первой строкой: длинный текст обрезается с конца
        var parts = new List<string>();
        if (closedNow.Count > 0) parts.Add($"Щёлкните, чтобы перенести в «Готово»:\n{Titles(closedNow)}");
        if (commented.Count > 0) parts.Add(CommentLines(commented));
        if (added.Count > 0) parts.Add((parts.Count > 0 ? "Новые: " : "") + Titles(added));

        Action? onClick = closedNow.Count > 0
            // к щелчку что-то могли уже перенести руками; идёт F5 — у него свой такой же вопрос
            ? () => { if (!IsRefreshing) AskMoveClosed(ClosedToMove(closedNow, closed), ""); }
            : commented.Count > 0 ? () => Reveal(commented[0].Ticket)
            : added.Count == 1 ? () => Reveal(added[0])
            : null;
        Notify?.Invoke(title, string.Join("\n", parts), onClick);
    }

    /// <summary>Выбрать заявку и открыть панель; MainWindow выделит карточку. Пока висело уведомление, её могли удалить.</summary>
    private void Reveal(Ticket t)
    {
        if (!AllTickets.Contains(t)) return;
        SelectedTicket = t;
        IsPanelOpen = true;
        RevealRequested?.Invoke(t);
    }

    /// <summary>Самый свежий новый комментарий каждой заявки: «#123 Иванов: текст…» — до трёх заявок.</summary>
    private static string CommentLines(IReadOnlyList<(Ticket Ticket, IReadOnlyList<IntraserviceEvent> Comments)> commented) =>
        string.Join("\n", commented.Take(3).Select(c => $"{c.Ticket.DisplayNumber} {c.Comments[0].Author}: {OneLine(c.Comments[0].Comment!, 80)}"))
        + (commented.Count > 3 ? $"\n…и ещё {commented.Count - 3}" : "");

    /// <summary>Текст одной строкой, не длиннее max: переводы строк и повторные пробелы — в один пробел.</summary>
    private static string OneLine(string s, int max)
    {
        var line = string.Join(" ", s.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
        return line.Length > max ? line[..max] + "…" : line;
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
