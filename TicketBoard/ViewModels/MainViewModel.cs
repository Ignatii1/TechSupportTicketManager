using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Windows;
using System.Windows.Threading;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

public sealed record PriorityFilterItem(string Label, TicketPriority? Value);

public sealed partial class MainViewModel : ObservableObject
{
    private readonly TicketStore _store;
    private readonly AppSettings _settings;
    private readonly IntraserviceLinkParser _parser;
    private readonly DispatcherTimer _saveTimer;
    private readonly DispatcherTimer _ageTimer;
    private IIntraserviceClient _intraservice;
    private bool _loaded;

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

    /// <summary>Окно просит показать quick capture (клавиша N, кнопка «+ Заявка», пустая доска).</summary>
    public event Action? CaptureRequested;
    /// <summary>Шестерёнка в заголовке окна — App открывает настройки.</summary>
    public event Action? SettingsRequested;

    public MainViewModel(TicketStore store, AppSettings settings, IntraserviceLinkParser parser, IIntraserviceClient intraservice)
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

        _saveTimer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(600) };
        _saveTimer.Tick += (_, _) => SaveNow();

        _ageTimer = new DispatcherTimer { Interval = TimeSpan.FromMinutes(10) };
        _ageTimer.Tick += (_, _) => { foreach (var t in AllTickets) t.RefreshAge(); RefreshFilters(); };
        _ageTimer.Start();

        _loaded = true;
        ApplySettings(intraservice);
    }

    /// <summary>Пороги, лимит, «скрыть готовые», клиент API — при старте и после сохранения настроек.</summary>
    public void ApplySettings(IIntraserviceClient intraservice)
    {
        _intraservice = intraservice;
        TicketRules.OverdueDays = _settings.OverdueDays;
        TicketRules.OverloadLimit = _settings.WipLimit;
        TicketRules.HideDoneDays = _settings.HideDoneOlderThanDays;
        ColumnFor(TicketStatus.InProgress).Hint = $"лимит {_settings.WipLimit}";
        OnPropertyChanged(nameof(HotkeyText));
        OnPropertyChanged(nameof(HideDoneDays));
        foreach (var t in AllTickets) t.RefreshAge();
        RefreshFilters();
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
        t.Url = url;
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
        if (id is not null && _intraservice is not NullIntraserviceClient) _ = SyncAsync(t);
        return t;
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
        else if (_intraservice is NullIntraserviceClient) message = "API не настроен: трей → Настройки…";
        else
        {
            var r = await _intraservice.GetTaskAsync(n);
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
