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

/// <summary>Строка переписки в панели: комментарий из Интрасервиса и/или смена статуса.
/// Живёт только в памяти — в tickets.json не пишется и на Ticket не висит (там чужие имена и внутренние тексты).
/// Text — null, если это просто смена статуса; StatusChange — название нового статуса, только там, где он поменялся.</summary>
public sealed record CommentRow(string Author, DateTimeOffset? Date, string? Text, bool IsInternal, string? StatusChange);

/// <summary>Переписка выбранной заявки из Интрасервиса — секция «Переписка» в панели. Только в памяти.</summary>
public sealed partial class MainViewModel
{
    /// <summary>Переписка по номеру заявки: только в памяти, чистится при смене настроек.
    /// Со временем протухает — приложение живёт в трее сутками, а в заявку за это время успевают написать.</summary>
    private readonly Dictionary<int, (IReadOnlyList<CommentRow> Rows, bool HasMore, DateTimeOffset At)> _commentCache = new();
    private static readonly TimeSpan CommentCacheLife = TimeSpan.FromMinutes(2);
    private CancellationTokenSource? _commentsLookup;

    // ---- переписка выбранной заявки (секция «Переписка» в панели) ----

    public ObservableCollection<CommentRow> Comments { get; } = new();
    /// <summary>Вид с фильтром по ShowAllEvents — как ColumnViewModel.View у колонки.</summary>
    public ICollectionView CommentsView { get; }
    /// <summary>Сколько строк видно сейчас — число рядом с заголовком секции.</summary>
    public int VisibleCommentsCount => Comments.Count - (ShowAllEvents ? 0 : HiddenEventsCount);
    /// <summary>Строка под заголовком переписки: «загружаю…», «сервер недоступен», «переписки нет».</summary>
    [ObservableProperty] private string _commentsMessage = "";
    /// <summary>Показывать и записи без комментария (голые смены статуса) — кнопка-глаз.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(VisibleCommentsCount))]
    private bool _showAllEvents;
    /// <summary>Сколько записей без комментария сейчас скрыто.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(VisibleCommentsCount))]
    private int _hiddenEventsCount;
    /// <summary>Записей на сервере больше, чем влезло на страницу.</summary>
    [ObservableProperty] private bool _commentsTruncated;

    /// <summary>Доска перед глазами — активное окно (MainWindow). В трее, свёрнута или за другими окнами — false:
    /// выбранная карточка остаётся выбранной, но её переписку никто не видит, и новые комментарии в ней не прочитаны.</summary>
    [ObservableProperty] private bool _isBoardActive;

    /// <summary>Чья переписка сейчас показана — загрузилась без ошибки; пока грузится — null.</summary>
    private Ticket? _commentsShownFor;

    // ---------- переписка ----------

    /// <summary>⟳ в заголовке секции: перечитать переписку с сервера, мимо кэша.</summary>
    [RelayCommand]
    private void RefreshComments() => LoadComments(SelectedTicket, force: true);

    /// <summary>Переписка выбранной заявки. Пауза 400 мс, чтобы бег стрелками по карточкам не слал запрос на каждую;
    /// предыдущий запрос отменяется; ответ пишем, только если заявка всё ещё выбрана. force — мимо кэша и без паузы.
    /// markSeen: false — показали не по действию человека (автообновление), новые комментарии ещё не прочитаны.</summary>
    private async void LoadComments(Ticket? t, bool force = false, bool markSeen = true)
    {
        _commentsLookup?.Cancel();
        _commentsShownFor = null;
        Comments.Clear();
        HiddenEventsCount = 0;
        CommentsTruncated = false;
        CommentsMessage = "";
        OnPropertyChanged(nameof(VisibleCommentsCount));

        if (t is null) return;
        if (t.IntraserviceId is not int n) { CommentsMessage = "у заявки нет номера"; return; }
        if (_intraservice is not { } client) { CommentsMessage = "API не настроен"; return; }
        if (!force && _commentCache.TryGetValue(n, out var cached) && DateTimeOffset.Now - cached.At < CommentCacheLife)
        {
            ShowComments(t, cached.Rows, cached.HasMore, markSeen);
            return;
        }

        var cts = _commentsLookup = new CancellationTokenSource();
        CommentsMessage = "загружаю…";
        try
        {
            if (!force) await Task.Delay(400, cts.Token); // по кнопке ждать нечего: нажали осознанно
            var r = await client.GetLifetimeAsync(n, cts.Token);
            if (cts.IsCancellationRequested || SelectedTicket != t) return; // пока ходили, выбрали другую заявку
            if (r.Error.Length > 0) { CommentsMessage = r.Error; return; }

            var rows = ToRows(r.Events);
            _commentCache[n] = (rows, r.HasMore, DateTimeOffset.Now);
            ShowComments(t, rows, r.HasMore, markSeen);
        }
        catch (OperationCanceledException) { /* выбрали другую заявку — неважно */ }
    }

    private void ShowComments(Ticket t, IReadOnlyList<CommentRow> rows, bool truncated, bool markSeen)
    {
        foreach (var r in rows) Comments.Add(r);
        HiddenEventsCount = rows.Count(r => r.Text is null);
        CommentsTruncated = truncated;
        CommentsMessage = rows.Count == 0 ? "переписки нет"
            : HiddenEventsCount == rows.Count ? "только смены статуса" : "";
        OnPropertyChanged(nameof(VisibleCommentsCount));
        _commentsShownFor = t;
        if (markSeen) MarkCommentsSeen();
    }

    /// <summary>Переписка выбранной заявки на экране — доска активна, панель открыта, загрузилась — и человек что-то сделал
    /// (выбрал карточку, открыл панель или окно, ⟳, щёлкнул или нажал клавишу на доске), значит, прочитана: бейдж гаснет,
    /// «видел до» — дата самого нового комментария (дата сервера, как у AutoSyncRules.Unread).</summary>
    public void MarkCommentsSeen()
    {
        if (!IsBoardActive || !IsPanelOpen || _commentsShownFor is not { } t || t != SelectedTicket) return;
        var newest = Comments.Where(c => c.Text is not null).Max(c => c.Date);
        if (newest is not null && (t.CommentsSeenAt is null || newest > t.CommentsSeenAt)) t.CommentsSeenAt = newest;
        t.UnreadComments = 0;
    }

    partial void OnIsBoardActiveChanged(bool value) => MarkCommentsSeen();
    partial void OnIsPanelOpenChanged(bool value) => MarkCommentsSeen();

    /// <summary>События API → строки панели: свежие сверху, чип статуса — только там, где статус отличается
    /// от следующей (более старой) записи. Сортировка устойчивая: не разобрались даты — останется порядок сервера.</summary>
    private static IReadOnlyList<CommentRow> ToRows(IReadOnlyList<IntraserviceEvent> events)
    {
        var sorted = events.OrderByDescending(e => e.Date ?? DateTimeOffset.MinValue).ToList();
        var rows = new List<CommentRow>(sorted.Count);
        for (var i = 0; i < sorted.Count; i++)
        {
            var e = sorted[i];
            var changed = e.Status.Length > 0 && (i + 1 == sorted.Count || sorted[i + 1].Status != e.Status);
            rows.Add(new(e.Author.Length > 0 ? e.Author : "—", e.Date,
                string.IsNullOrWhiteSpace(e.Comment) ? null : e.Comment,
                e.IsPublic == false, changed ? e.Status : null));
        }
        return rows;
    }

    partial void OnShowAllEventsChanged(bool value)
    {
        CommentsView.Refresh();
        HiddenEventsCount = Comments.Count(c => c.Text is null);
    }
}
