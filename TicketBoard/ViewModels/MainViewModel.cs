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

public sealed record PriorityFilterItem(string Label, TicketPriority? Value);

/// <summary>Строка переписки в панели: комментарий из Интрасервиса и/или смена статуса.
/// Живёт только в памяти — в tickets.json не пишется и на Ticket не висит (там чужие имена и внутренние тексты).
/// Text — null, если это просто смена статуса; StatusChange — название нового статуса, только там, где он поменялся.</summary>
public sealed record CommentRow(string Author, DateTimeOffset? Date, string? Text, bool IsInternal, string? StatusChange);

public sealed partial class MainViewModel : ObservableObject
{
    private readonly TicketStore _store;
    private readonly AppSettings _settings;
    private readonly IntraserviceLinkParser _parser;
    private readonly DispatcherTimer _saveTimer;
    private readonly DispatcherTimer _ageTimer;
    private HttpIntraserviceClient? _intraservice; // null — API не настроен
    private bool _loaded;
    /// <summary>Переписка по номеру заявки: только в памяти, чистится при смене настроек.
    /// Со временем протухает — приложение живёт в трее сутками, а в заявку за это время успевают написать.</summary>
    private readonly Dictionary<int, (IReadOnlyList<CommentRow> Rows, bool HasMore, DateTimeOffset At)> _commentCache = new();
    private static readonly TimeSpan CommentCacheLife = TimeSpan.FromMinutes(2);
    private CancellationTokenSource? _commentsLookup;

    public static PriorityFilterItem[] PriorityFilters { get; } =
    {
        new("Все приоритеты", null),
        new("Высокий", TicketPriority.High),
        new("Средний", TicketPriority.Mid),
        new("Низкий", TicketPriority.Low),
    };
    public static TicketStatus[] Statuses { get; } = Enum.GetValues<TicketStatus>();

    public ObservableCollection<ColumnViewModel> Columns { get; }
    public string HotkeyText => _settings.Hotkey;
    public int HideDoneDays => _settings.HideDoneOlderThanDays;

    [ObservableProperty] private Ticket? _selectedTicket;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(IsScrimVisible))]
    private bool _isPanelOpen;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(IsScrimVisible))]
    private bool _isOverlay;          // окно уже 1100 px — панель поверх доски + затемнение
    public bool IsScrimVisible => IsOverlay && IsPanelOpen;
    [ObservableProperty] private string _searchText = "";
    [ObservableProperty] private PriorityFilterItem _priorityFilter = PriorityFilters[0];
    [ObservableProperty] private bool _hideOldDone = true;
    [ObservableProperty] private string _newNoteText = "";
    [ObservableProperty] private bool _isBoardEmpty;
    /// <summary>Строка под статусом Интрасервиса в панели: «обновляю…», «заявка не найдена», …</summary>
    [ObservableProperty] private string _syncMessage = "";

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

    /// <summary>Окно просит показать quick capture (клавиша N, кнопка «+ Заявка», пустая доска).</summary>
    public event Action? CaptureRequested;
    /// <summary>Шестерёнка в заголовке окна — App открывает настройки.</summary>
    public event Action? SettingsRequested;

    public MainViewModel(TicketStore store, AppSettings settings, IntraserviceLinkParser parser, HttpIntraserviceClient? intraservice)
    {
        _store = store;
        _settings = settings;
        _parser = parser;
        _intraservice = intraservice;

        Columns = new()
        {
            new(TicketStatus.Inbox,      "Входящие",    OnDropped),
            new(TicketStatus.InProgress, "В работе",    OnDropped),
            new(TicketStatus.Waiting,    "Ждёт ответа", OnDropped),
            new(TicketStatus.Done,       "Готово",      OnDropped),
        };

        foreach (var t in _store.Load())
        {
            Track(t);
            ColumnFor(t.Status).Items.Add(t);
        }
        foreach (var c in Columns)
            c.Items.CollectionChanged += (_, _) => Application.Current?.Dispatcher.BeginInvoke(Recount, DispatcherPriority.Background);

        CommentsView = CollectionViewSource.GetDefaultView(Comments);
        CommentsView.Filter = o => ShowAllEvents || o is CommentRow { Text: not null };

        _saveTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(600) };
        _saveTimer.Tick += (_, _) => SaveNow();

        _ageTimer = new DispatcherTimer { Interval = TimeSpan.FromMinutes(10) };
        _ageTimer.Tick += (_, _) => { foreach (var t in AllTickets) t.RefreshAge(); RefreshFilters(); };
        _ageTimer.Start();

        _loaded = true;
        ApplySettings(intraservice);
    }

    /// <summary>Пороги, лимит, «скрыть готовые», клиент API — при старте и после сохранения настроек.</summary>
    public void ApplySettings(HttpIntraserviceClient? intraservice)
    {
        _intraservice = intraservice;
        TicketRules.OverdueDays = _settings.OverdueDays;
        TicketRules.OverloadLimit = _settings.WipLimit;
        ColumnFor(TicketStatus.InProgress).Hint = $"лимит {_settings.WipLimit}";
        OnPropertyChanged(nameof(HotkeyText));
        OnPropertyChanged(nameof(HideDoneDays));
        foreach (var t in AllTickets) t.RefreshAge();
        RefreshFilters();
        _commentCache.Clear();          // сменились адрес или логин — старая переписка не годится
        LoadComments(SelectedTicket);
    }

    [RelayCommand]
    private void RequestSettings() => SettingsRequested?.Invoke();

    public IEnumerable<Ticket> AllTickets => Columns.SelectMany(c => c.Items);

    // ---------- создание ----------

    [RelayCommand]
    private void RequestCapture() => CaptureRequested?.Invoke();

    /// <summary>Из окна быстрого добавления: ссылка и/или текст + приоритет.</summary>
    public Ticket AddFromCapture(string input, TicketPriority priority)
    {
        var t = new Ticket { Priority = priority };
        _parser.TryParse(input, out var url, out var id);
        t.Url = url.Length > 0 ? url : TicketUrl(id);
        t.IntraserviceId = id;

        var rest = input;
        if (url.Length > 0) rest = rest.Replace(url, "");
        rest = rest.Trim().Trim('—', '-', '·', ' ');
        t.Title = rest.Length > 0 && !(id is not null && rest.TrimStart('#', '№', ' ') == id.ToString())
            ? rest
            : id is int n ? $"Заявка #{n}" : "Заявка";

        Track(t);
        t.MarkAppear();
        ColumnFor(TicketStatus.Inbox).Items.Insert(0, t);
        ScheduleSave();
        SelectedTicket = t;
        if (id is not null && _intraservice is not null) _ = SyncAsync(t);
        return t;
    }

    /// <summary>Ввели только номер — ссылку собираем из базового адреса, иначе кнопка ↗ будет бесполезна.</summary>
    private string TicketUrl(int? id)
    {
        var b = _settings.IntraserviceBaseUrl.Trim().TrimEnd('/');
        return id is int n && b.Length > 0 ? $"{b}/Task/View/{n}" : "";
    }

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
            if (r.Task is IntraserviceTask x)
            {
                if (t.Title == $"Заявка #{n}" && x.Name.Length > 0) t.Title = x.Name;
                if (string.IsNullOrWhiteSpace(t.Description) && !string.IsNullOrEmpty(x.Description)) t.Description = x.Description;
                t.ExternalStatus = x.Status;
                t.LastSyncAt = DateTimeOffset.Now;
            }
        }
        if (SelectedTicket == t) SyncMessage = message;
    }

    // ---------- импорт моих заявок ----------

    /// <summary>Идёт импорт — кнопка на доске и пункт меню в трее на это время недоступны.</summary>
    [ObservableProperty] private bool _isImporting;

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
            if (_intraservice is not { } client) { Report("API не настроен: трей → Настройки…", MessageBoxImage.Warning); return; }

            var (userId, userError) = await client.GetCurrentUserIdAsync();
            if (userId is not int me) { Report(userError, MessageBoxImage.Warning); return; }

            var (statuses, statusError) = await client.GetStatusesAsync();
            if (statusError.Length > 0) { Report(statusError, MessageBoxImage.Warning); return; }
            // пустой справочник статусов и «все статусы закрытые» — разные беды: первая на сервере, вторую чинит сам пользователь
            if (statuses.Count == 0) { Report("сервер не вернул ни одного статуса заявок", MessageBoxImage.Warning); return; }

            // список правится руками, поэтому терпим и пустые строки, и лишние пробелы, и отсутствие самого списка
            var closed = (_settings.ClosedStatusNames ?? Array.Empty<string>())
                .Where(n => !string.IsNullOrWhiteSpace(n)).Select(n => n.Trim()).ToHashSet(StringComparer.OrdinalIgnoreCase);
            var openIds = statuses.Where(s => !s.IsFixed && !s.IsFinal && !closed.Contains(s.Name)).Select(s => s.Id).ToList();
            if (openIds.Count == 0)
            {
                Report("все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json", MessageBoxImage.Warning);
                return;
            }

            // ponytail: потолок 10 страниц по 200 — 2000 заявок; упрётся — добавить постраничную докачку.
            var rows = new List<IntraserviceFound>();
            var error = "";
            for (var page = 1; page <= ImportPages; page++)
            {
                var r = await client.GetExecutorTasksAsync(me, openIds, page);
                if (r.Error.Length > 0) { error = r.Error; break; }  // что успели забрать — всё равно добавим
                if (r.Found.Count == 0) break;
                rows.AddRange(r.Found);
                if (rows.Count >= r.Total) break;                    // забрали всё, что сервер обещал
            }
            if (rows.Count == 0 && error.Length > 0) { Report(error, MessageBoxImage.Warning); return; }
            if (rows.Count == 0) { Report("открытых заявок, где вы исполнитель, не нашлось", MessageBoxImage.Information); return; }

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

            Report($"Добавлено: {added}, уже было: {had}"
                + (error.Length > 0 ? $"\nЗагружены не все страницы: {error}" : ""), MessageBoxImage.Information);
        }
        finally { IsImporting = false; }
    }

    /// <summary>Итог импорта или его ошибка — окном, как подтверждение удаления: импорт запускают руками и ждут ответа.</summary>
    private static void Report(string text, MessageBoxImage icon) => MessageBox.Show(text, "Заявки", MessageBoxButton.OK, icon);

    // ---------- переписка ----------

    /// <summary>⟳ в заголовке секции: перечитать переписку с сервера, мимо кэша.</summary>
    [RelayCommand]
    private void RefreshComments() => LoadComments(SelectedTicket, force: true);

    /// <summary>Переписка выбранной заявки. Пауза 400 мс, чтобы бег стрелками по карточкам не слал запрос на каждую;
    /// предыдущий запрос отменяется; ответ пишем, только если заявка всё ещё выбрана. force — мимо кэша и без паузы.</summary>
    private async void LoadComments(Ticket? t, bool force = false)
    {
        _commentsLookup?.Cancel();
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
            ShowComments(cached.Rows, cached.HasMore);
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
            ShowComments(rows, r.HasMore);
        }
        catch (OperationCanceledException) { /* выбрали другую заявку — неважно */ }
    }

    private void ShowComments(IReadOnlyList<CommentRow> rows, bool truncated)
    {
        foreach (var r in rows) Comments.Add(r);
        HiddenEventsCount = rows.Count(r => r.Text is null);
        CommentsTruncated = truncated;
        CommentsMessage = rows.Count == 0 ? "переписки нет"
            : HiddenEventsCount == rows.Count ? "только смены статуса" : "";
        OnPropertyChanged(nameof(VisibleCommentsCount));
    }

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

    // ---------- перемещение ----------

    private void OnDropped(Ticket t, ColumnViewModel target)
    {
        t.MoveTo(target.Status);
        AfterMove();
    }

    [RelayCommand]
    private void MoveTo(TicketStatus status)
    {
        if (SelectedTicket is Ticket t) MoveTicket(t, ColumnFor(status));
    }

    /// <summary>Сдвинуть выбранную заявку на delta колонок (← / →).</summary>
    public void MoveSelected(int delta)
    {
        if (SelectedTicket is not Ticket t) return;
        var idx = Columns.IndexOf(ColumnFor(t.Status));
        var target = Math.Clamp(idx + delta, 0, Columns.Count - 1);
        if (target != idx) MoveTicket(t, Columns[target]);
    }

    /// <summary>Приоритет выбранной заявки с клавиатуры (1 / 2 / 3).</summary>
    public void SetSelectedPriority(TicketPriority p)
    {
        if (SelectedTicket is Ticket t) t.Priority = p; // сохранение и перефильтровка — в OnTicketChanged
    }

    private void MoveTicket(Ticket t, ColumnViewModel target)
    {
        if (t.Status == target.Status) return;
        foreach (var c in Columns) c.Items.Remove(t);
        target.Items.Insert(0, t);
        t.MoveTo(target.Status);
        AfterMove();
    }

    private void AfterMove()
    {
        OnPropertyChanged(nameof(SelectedStatus));
        ScheduleSave();
        RefreshFilters();
    }

    /// <summary>Статус выбранной заявки для ComboBox в панели: смена = перенос между колонками.</summary>
    public TicketStatus? SelectedStatus
    {
        get => SelectedTicket?.Status;
        set { if (value is TicketStatus s && SelectedTicket is Ticket t) MoveTicket(t, ColumnFor(s)); }
    }

    partial void OnSelectedTicketChanged(Ticket? value)
    {
        OnPropertyChanged(nameof(SelectedStatus));
        SyncMessage = "";
        LoadComments(value);
        if (value is not null) IsPanelOpen = true;
    }

    public ColumnViewModel ColumnFor(TicketStatus s) => Columns.First(c => c.Status == s);

    // ---------- заметки и прочее ----------

    [RelayCommand]
    private void AddNote()
    {
        if (SelectedTicket is null || string.IsNullOrWhiteSpace(NewNoteText)) return;
        SelectedTicket.Notes.Insert(0, new Note { Text = NewNoteText.Trim() });
        NewNoteText = "";
        ScheduleSave();
    }

    // ponytail: без подтверждения — заметка в одну строку, откат через ежедневный бэкап tickets.json
    [RelayCommand]
    private void DeleteNote(Note? note)
    {
        if (note is null || SelectedTicket is not Ticket t || !t.Notes.Remove(note)) return;
        ScheduleSave();
    }

    [RelayCommand]
    private void DeleteSelected()
    {
        if (SelectedTicket is not Ticket t) return;
        var ok = MessageBox.Show($"Удалить заявку «{t.Title}»?", "Заявки",
            MessageBoxButton.YesNo, MessageBoxImage.Question) == MessageBoxResult.Yes;
        if (!ok) return;
        foreach (var c in Columns) c.Items.Remove(t);
        SelectedTicket = null;
        IsPanelOpen = false;
        ScheduleSave();
    }

    [RelayCommand]
    private void OpenLink() => OpenTicketLink(SelectedTicket);

    [RelayCommand]
    private void OpenTicketLink(Ticket? t)
    {
        if (string.IsNullOrWhiteSpace(t?.Url)) return;
        try { Process.Start(new ProcessStartInfo(t.Url) { UseShellExecute = true }); }
        catch { /* кривая ссылка — молча */ }
    }

    [RelayCommand]
    private void CopyLink()
    {
        if (!string.IsNullOrWhiteSpace(SelectedTicket?.Url)) Clipboard.SetText(SelectedTicket.Url);
    }

    [RelayCommand]
    private void ClosePanel() => IsPanelOpen = false;

    public void TogglePanel()
    {
        if (SelectedTicket is null) return;
        IsPanelOpen = !IsPanelOpen;
    }

    // ---------- фильтры ----------

    partial void OnSearchTextChanged(string value) => RefreshFilters();
    partial void OnHideOldDoneChanged(bool value) => RefreshFilters();
    partial void OnPriorityFilterChanged(PriorityFilterItem value) => RefreshFilters();

    private void RefreshFilters()
    {
        if (!_loaded) return;
        foreach (var col in Columns)
        {
            col.View.Filter = o => o is Ticket t && Matches(t, col);
            col.View.Refresh();
        }
        ColumnFor(TicketStatus.Done).Hint = HideOldDone ? $"скрыты старше {_settings.HideDoneOlderThanDays} д" : "";
        Recount();
    }

    private void Recount()
    {
        foreach (var c in Columns) c.Recount();
        IsBoardEmpty = Columns.All(c => c.VisibleCount == 0);
        ColumnFor(TicketStatus.Inbox).ShowEmptyHint = !IsBoardEmpty && ColumnFor(TicketStatus.Inbox).VisibleCount == 0;
    }

    private bool Matches(Ticket t, ColumnViewModel col)
    {
        if (col.Status == TicketStatus.Done && HideOldDone
            && t.CompletedAt is DateTimeOffset done
            && done < DateTimeOffset.Now.AddDays(-_settings.HideDoneOlderThanDays))
            return false;

        if (PriorityFilter.Value is TicketPriority p && t.Priority != p) return false;

        if (string.IsNullOrWhiteSpace(SearchText)) return true;
        var q = SearchText.Trim();
        return t.Title.Contains(q, StringComparison.OrdinalIgnoreCase)
            || t.Description.Contains(q, StringComparison.OrdinalIgnoreCase)
            || t.DisplayNumber.Contains(q, StringComparison.OrdinalIgnoreCase)
            || t.Url.Contains(q, StringComparison.OrdinalIgnoreCase)
            || t.Notes.Any(n => n.Text.Contains(q, StringComparison.OrdinalIgnoreCase));
    }

    // ---------- сохранение ----------

    private void Track(Ticket t) => t.PropertyChanged += OnTicketChanged;

    private void OnTicketChanged(object? sender, PropertyChangedEventArgs e)
    {
        // косметика — не сохраняем
        if (e.PropertyName is nameof(Ticket.DaysInStatus) or nameof(Ticket.AgeState)
            or nameof(Ticket.AgeLabel) or nameof(Ticket.AgeText)) return;
        if (e.PropertyName is nameof(Ticket.Priority)) RefreshFilters();
        ScheduleSave();
    }

    private void ScheduleSave()
    {
        _saveTimer.Stop();
        _saveTimer.Start();
    }

    public void SaveNow()
    {
        _saveTimer.Stop();
        _store.Save(AllTickets.ToList());
    }
}
