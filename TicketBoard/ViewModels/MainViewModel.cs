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

public sealed partial class MainViewModel : ObservableObject
{
    private readonly TicketStore _store;
    private readonly AppSettings _settings;
    private readonly IntraserviceLinkParser _parser;
    private readonly DispatcherTimer _saveTimer;
    private readonly DispatcherTimer _ageTimer;
    private HttpIntraserviceClient? _intraservice; // null — API не настроен
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
    /// <summary>Кнопка «Поиск заявок» на доске — окно открывает App.</summary>
    public event Action? SearchRequested;

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
            c.Items.CollectionChanged += (_, _) => QueueRecount();

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

    /// <summary>Пороги, лимит, «скрыть готовые», клиент API, автообновление — при старте и после сохранения настроек.</summary>
    public void ApplySettings(HttpIntraserviceClient? intraservice)
    {
        _intraservice = intraservice;
        _me = null;                     // адрес или логин могли смениться — «кто я» спросим заново
        // сменили сервер или логин — в настройках или руками в settings.json между запусками: пропуски и «была моей»
        // относились к прежней учётной записи — заново, первым заходом, без уведомлений «передали» обо всех её заявках.
        if (_settings.AutoSyncAccount != _settings.AccountKey)
        {
            if (_settings.AutoSyncAccount is not null)
            {
                foreach (var t in AllTickets) t.AssignedToMe = null;
                _settings.AutoSyncSkipIds = null;
                ForgetExport();   // отметка выгрузки — тоже чужая
            }
            _settings.AutoSyncAccount = _settings.AccountKey;
            // сразу на диск: иначе смену логина руками между запусками не с чем сравнить. Только для настроенного адреса —
            // не прочитанный при запуске settings.json (в памяти дефолты, адреса нет) так не затрём
            if (_settings.IntraserviceBaseUrl.Trim().Length > 0)
                try { _settings.Save(App.DataDir); }
                catch { /* в памяти верно — запишется со следующей записью настроек */ }
        }
        TicketRules.OverdueDays = _settings.OverdueDays;
        TicketRules.OverloadLimit = _settings.WipLimit;
        ColumnFor(TicketStatus.InProgress).Hint = $"лимит {_settings.WipLimit}";
        OnPropertyChanged(nameof(HotkeyText));
        OnPropertyChanged(nameof(HideDoneDays));
        foreach (var t in AllTickets) t.RefreshAge();
        RefreshFilters();
        _commentCache.Clear();          // сменились адрес или логин — старая переписка не годится
        // выключили выгрузку — отметку забываем (включат снова — как в первый раз, с недели назад, как обещает подсказка)
        if (!_settings.AutoExportClosed && _settings.AutoExportSince is not null)
        {
            ForgetExport();
            try { _settings.Save(App.DataDir); }
            catch (Exception ex) { Log?.Invoke($"Не удалось сохранить settings.json: {ex}"); }   // в памяти верно — до следующей записи
        }
        // выгрузка не идёт (выключена, автообновление 0 минут, нет адреса) — заход, что ещё ждёт очереди, бросает, и в подсказке
        // трея её нет
        if (!ExportCanRun)
        {
            StopExportPass();
            AutoExportState = "";
        }
        LoadComments(SelectedTicket);
        ApplyAutoSync();
    }

    [RelayCommand]
    private void RequestSettings() => SettingsRequested?.Invoke();

    [RelayCommand]
    private void RequestSearch() => SearchRequested?.Invoke();

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
        if (id is int number) AllowAutoSync(new[] { number });   // вернули руками удалённую — автообновление снова её ведёт

        var rest = input;
        if (url.Length > 0) rest = rest.Replace(url, "");
        rest = rest.Trim().Trim('—', '-', '·', ' ');
        t.Title = rest.Length > 0 && !(id is not null && rest.TrimStart('#', '№', ' ') == id.ToString())
            ? rest
            : id is int n ? HttpIntraserviceClient.AutoTitle(n) : "Заявка";

        return Insert(t);
    }

    /// <summary>Карточка заявки, чьи номер, ссылка и название уже известны (окно поиска по серверу): без разбора текста —
    /// число в названии не перебьёт номер, а название видно сразу. Пустая ссылка — соберём из базового адреса.</summary>
    public Ticket AddKnown(int id, string url, string title, TicketPriority priority)
    {
        var t = new Ticket
        {
            Priority = priority,
            IntraserviceId = id,
            Url = url.Length > 0 ? url : TicketUrl(id),
            Title = string.IsNullOrWhiteSpace(title) ? HttpIntraserviceClient.AutoTitle(id) : title.Trim(),
        };
        AllowAutoSync(new[] { id });   // вернули руками удалённую — автообновление снова её ведёт
        return Insert(t);
    }

    /// <summary>Общий конец добавления: карточка — во «Входящие» сверху, выбрана, и если у неё есть номер — сразу синхронизация.</summary>
    private Ticket Insert(Ticket t)
    {
        Track(t);
        t.MarkAppear();
        ColumnFor(TicketStatus.Inbox).Items.Insert(0, t);
        ScheduleSave();
        SelectedTicket = t;   // панель прочитает карточку после паузы — к тому времени её читает синхронизация ниже (CardReadAt)
        if (t.IntraserviceId is not null && _intraservice is not null) _ = SyncAsync(t);
        return t;
    }

    /// <summary>Ввели только номер — ссылку собираем из базового адреса, иначе кнопка ↗ будет бесполезна.</summary>
    private string TicketUrl(int? id) => id is int n ? _settings.TicketUrl(n) : "";

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

    /// <summary>Перенос между колонками. afterMove: false — для пачки: перефильтровку и сохранение вызывающий делает сам, один раз.</summary>
    private void MoveTicket(Ticket t, ColumnViewModel target, bool afterMove = true)
    {
        if (t.Status == target.Status) return;
        foreach (var c in Columns) c.Items.Remove(t);
        target.Items.Insert(0, t);
        t.MoveTo(target.Status);
        if (afterMove) AfterMove();
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
        // поля карточки и вложения — из карточки доски сразу; саму карточку и размеры — после паузы, тихо (LookupPanel)
        ShowAttachments(value);
        LookupPanel(value);
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
        // в Интрасервис приложение не пишет — стоит сказать, что там заявка останется
        var text = t.IntraserviceId is null ? $"«{t.Title}»" : $"{t.DisplayNumber}  «{t.Title}»\n\nВ Интрасервисе она останется.";
        if (!Views.AskWindow.Ask("Удалить заявку с доски?", text, "Удалить", danger: true)) return;
        foreach (var c in Columns) c.Items.Remove(t);
        if (t.IntraserviceId is int n) SkipOnAutoSync(n);   // удалили — значит, не нужна: автообновление её не вернёт
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
        if (!string.IsNullOrWhiteSpace(SelectedTicket?.Url)) ClipboardWatcher.TrySetText(SelectedTicket.Url);   // буфер занят — не падаем
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

    private bool _recountQueued;

    /// <summary>Пересчёт ходит по всем карточкам, а импорт добавляет их по одной — склеиваем пачку изменений
    /// в один пересчёт, иначе на двух сотнях заявок доска встанет.</summary>
    private void QueueRecount()
    {
        if (_recountQueued || Application.Current?.Dispatcher is not { } dispatcher) return;
        _recountQueued = true;
        dispatcher.BeginInvoke(() => { _recountQueued = false; Recount(); }, DispatcherPriority.Background);
    }

    private void Recount()
    {
        foreach (var c in Columns) c.Recount();
        IsBoardEmpty = Columns.All(c => c.VisibleCount == 0);
        ColumnFor(TicketStatus.Inbox).ShowEmptyHint = !IsBoardEmpty && ColumnFor(TicketStatus.Inbox).VisibleCount == 0;
        ShowAutoSyncState();   // перенесли или удалили закрытую — счётчик в заголовке следом
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
            or nameof(Ticket.AgeLabel) or nameof(Ticket.AgeText) or nameof(Ticket.HasUnreadComments)
            or nameof(Ticket.ExecutorsText) or nameof(Ticket.CreatorContacts) or nameof(Ticket.CreatorRole)
            or nameof(Ticket.ServiceText) or nameof(Ticket.CardContext) or nameof(Ticket.FilesCount) or nameof(Ticket.HasFiles)) return;
        if (e.PropertyName is nameof(Ticket.Priority)) RefreshFilters();
        if (e.PropertyName is nameof(Ticket.ExternalStatus)) QueueRecount();   // счётчик закрытых в заголовке
        // у открытой карточки сменился список вложений (⟳, F5, тихое чтение) — в панель сразу, размеры новых — без паузы
        if (e.PropertyName is nameof(Ticket.Files) && sender is Ticket t && t == SelectedTicket)
        {
            ShowAttachments(t);
            LookupPanel(t, pause: false);
        }
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
