using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Пункт списка выбора в окне поиска: номер (0 — «любой» или «нет») и подпись.</summary>
public sealed record Choice(int Id, string Label);

/// <summary>Пункт списка «Статус»: «любой», «закрытые», «открытые» или один из статусов сервера (Kind One и его номер).</summary>
public sealed record StatusChoice(SearchStatus Kind, int Id, string Label);

/// <summary>Строка результатов поиска. Всё, кроме «уже на доске», задаётся один раз при создании:
/// найденное на сервере мы не редактируем.</summary>
public sealed partial class FoundTicketViewModel : ObservableObject
{
    public FoundTicketViewModel(IntraserviceFound found, string url)
    {
        Found = found;
        Id = found.Id;
        Title = found.Name;
        Status = found.Status;
        // подписи собираются здесь, а не через StringFormat в XAML: в разметке фигурные скобки внутри значения — лишний риск
        Service = string.Join(" · ", new[] { found.Extra?.Service, found.Extra?.Type }.Where(s => !string.IsNullOrWhiteSpace(s)));
        // даты приходят в часовом поясе пользователя Интрасервиса — показываем как есть, не переводя в местное время
        var parts = new List<string>();
        if (found.Creator?.Trim() is { Length: > 0 } creator) parts.Add($"автор: {creator}");
        if (found.Executors?.Trim() is { Length: > 0 } executors) parts.Add($"исполнители: {executors}");
        if (found.Created is { } created) parts.Add($"создана {created:dd.MM.yyyy}");
        if (found.Changed is { } changed) parts.Add($"изменена {changed:dd.MM.yyyy}");
        Details = string.Join(" · ", parts);
        Url = url;
    }

    /// <summary>Строка списка, как её прислал сервер, — из неё строятся просмотр и файлы.</summary>
    public IntraserviceFound Found { get; }
    public int Id { get; }
    public string Title { get; }
    public string Status { get; }
    /// <summary>«Сервис · тип» или «» — сервер ни того ни другого не прислал.</summary>
    public string Service { get; }
    /// <summary>«автор: … · исполнители: … · создана … · изменена …» — что из этого известно.</summary>
    public string Details { get; }
    /// <summary>Адрес заявки или «», если базовый адрес не настроен.</summary>
    public string Url { get; }
    public string DisplayNumber => $"#{Id}";

    /// <summary>Такая заявка уже есть на доске: строка гасится, кнопки «на доску» нет.</summary>
    [ObservableProperty] private bool _onBoard;
}

/// <summary>Окно «Поиск заявок»: условия (слова, статус, сервис, тип, исполнитель, заявитель, даты, сохранённый фильтр) →
/// список с сервера → просмотр заявки так, как её получит агент → в буфер обмена или файлами для базы знаний.
/// Условия запоминаются в settings.json. Список и просмотр нигде не сохраняются — только на время жизни окна. Окно прячется,
/// а не закрывается: идущая выгрузка продолжается. В Интрасервис ничего не пишет.</summary>
public sealed partial class SearchViewModel : ObservableObject
{
    /// <summary>Заявок на страницу списка и «ещё».</summary>
    private const int PageSize = 50;
    /// <summary>Больше этого в окно не набираем: дальше — уточнить условия или выгрузить файлами.</summary>
    private const int MaxShown = 1000;
    /// <summary>В буфер — не больше стольких заявок за раз: чат Claude всё равно не вместит больше.</summary>
    private const int MaxCopy = 30;

    /// <summary>Поля, от которых зависит, что нашлось: поменялись — «выгрузить найденные» ждёт нового поиска.</summary>
    private static readonly HashSet<string> QueryFields = new()
    {
        nameof(Words), nameof(Mine), nameof(Executor), nameof(Creator), nameof(SelectedStatus), nameof(SelectedService),
        nameof(WithChildren), nameof(SelectedType), nameof(SelectedSaved), nameof(IncludeArchived),
        nameof(CreatedFrom), nameof(CreatedTo), nameof(ChangedFrom), nameof(ChangedTo), nameof(ClosedFrom), nameof(ClosedTo),
    };

    private readonly MainViewModel _board;
    private readonly AppSettings _settings;
    private HttpIntraserviceClient? _intraservice; // null — API не настроен
    private CancellationTokenSource? _lookup;      // идущий поиск
    private CancellationTokenSource? _previewRun;  // идущий просмотр
    private CancellationTokenSource? _work;        // идущая выгрузка или подготовка текста для буфера
    private ResolvedSearch? _last;                 // условия, по которым получен список на экране
    private int _page;                             // сколько страниц списка уже показано
    private bool _referencesTried;                 // хоть раз пытались: поиск сам за ними больше не ходит, чтобы не тянуть каждый раз
    private Task? _referencesRun;                  // идущая загрузка справочников — одна на всех, кто её ждёт
    // справочники; null — не прочитан (ещё или не вышло): окно при открытии перечитывает только такие
    private IReadOnlyList<IntraserviceStatus>? _statuses;
    private IReadOnlyList<IntraserviceRef>? _services, _types, _saved;
    private bool AllReferences => _statuses is not null && _services is not null && _types is not null && _saved is not null;
    private SearchFilter _wanted = new();          // что выбрать в списках: запомненное и то, что человек выбрал сам
    private bool _filling;                         // списки и выбор в них заполняет код, а не человек
    private bool _skipRemember;                    // ближайший поиск — «быстрый» с доски: запомненные условия не трогает
    private string _account;                       // учётная запись, которой принадлежат справочники и список на экране
    private IReadOnlyList<FoundTicketViewModel> _selected = Array.Empty<FoundTicketViewModel>();
    private readonly Dictionary<int, (DateTimeOffset? Changed, string Text)> _texts = new();   // просмотр: не ходить за тем же дважды

    public ObservableCollection<FoundTicketViewModel> Results { get; } = new();
    public ObservableCollection<StatusChoice> StatusChoices { get; } = new();
    public ObservableCollection<Choice> ServiceChoices { get; } = new();
    public ObservableCollection<Choice> TypeChoices { get; } = new();
    public ObservableCollection<Choice> SavedChoices { get; } = new();

    // ----- условия -----
    [ObservableProperty] private string _words = "";
    [ObservableProperty] private bool _mine;
    [ObservableProperty] private string _executor = "";
    [ObservableProperty] private string _creator = "";
    [ObservableProperty] private StatusChoice? _selectedStatus;
    [ObservableProperty] private Choice? _selectedService;
    [ObservableProperty] private bool _withChildren = true;
    [ObservableProperty] private Choice? _selectedType;
    [ObservableProperty] private Choice? _selectedSaved;
    [ObservableProperty] private string _createdFrom = "";
    [ObservableProperty] private string _createdTo = "";
    [ObservableProperty] private string _changedFrom = "";
    [ObservableProperty] private string _changedTo = "";
    [ObservableProperty] private string _closedFrom = "";
    [ObservableProperty] private string _closedTo = "";
    [ObservableProperty] private bool _includeArchived = true;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(ExportFoundLabel))] private string _limit = "500";

    // ----- ход и итоги -----
    /// <summary>Строка состояния поиска: «ищу…», «ничего не найдено», ошибка или сколько из скольких показано.</summary>
    [ObservableProperty] private string _message = "";
    /// <summary>Какие справочники не загрузились (поиск работает и без них) — под условиями.</summary>
    [ObservableProperty] private string _notes = "";
    /// <summary>Кто нашёлся по имени («Исполнитель: Иванов И.», «Заявитель «иванов»: подошли 2 — …») — над списком: так
    /// видно, если под одно имя подошло несколько человек и поиск стал шире.</summary>
    [ObservableProperty] private string _matched = "";
    /// <summary>Сервер, похоже, не применил условие по дате — результаты включают лишнее.</summary>
    [ObservableProperty] private string _warning = "";
    [ObservableProperty] private bool _isBusy;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(ExportFoundLabel)), NotifyCanExecuteChangedFor(nameof(ExportFoundCommand))]
    private int _total;
    [ObservableProperty] private bool _hasMore;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(StaleHint)), NotifyCanExecuteChangedFor(nameof(ExportFoundCommand))]
    private bool _isStale;

    // ----- просмотр, буфер, выгрузка -----
    [ObservableProperty, NotifyPropertyChangedFor(nameof(PreviewHint))] private string _previewTitle = "";
    [ObservableProperty] private string _previewText = "";
    [ObservableProperty] private bool _isPreviewBusy;
    [ObservableProperty, NotifyPropertyChangedFor(nameof(CopyLabel), nameof(ExportSelectedLabel)),
     NotifyCanExecuteChangedFor(nameof(CopyCommand), nameof(ExportSelectedCommand))]
    private int _selectedCount;
    [ObservableProperty] private string _folder = "";
    /// <summary>Ход и итог копирования и выгрузки — выделяется и копируется.</summary>
    [ObservableProperty] private string _workMessage = "";
    [ObservableProperty, NotifyCanExecuteChangedFor(nameof(CopyCommand), nameof(ExportFoundCommand), nameof(ExportSelectedCommand), nameof(StopCommand))]
    private bool _isWorking;

    /// <summary>Подсказка вместо пустого просмотра: пока ничего не выбрано.</summary>
    public string PreviewHint => PreviewTitle.Length == 0 ? "Выберите заявку в списке — здесь будет её текст, таким, каким он уйдёт агенту." : "";

    /// <summary>Условия поменяли, а список на экране — прежний: подсказка над ним («выгрузить найденные» до нового поиска выключено).</summary>
    public string StaleHint => IsStale && Results.Count > 0 ? "Условия изменены — нажмите «Найти»: список ниже ещё по прежним" : "";

    public string CopyLabel => SelectedCount > 1 ? $"Копировать ({SelectedCount})" : "Копировать";
    public string ExportSelectedLabel => SelectedCount > 0 ? $"Выгрузить выбранные ({SelectedCount})" : "Выгрузить выбранные";
    public string ExportFoundLabel
    {
        get
        {
            if (Total <= 0) return "Выгрузить найденные";
            var take = ExportLimit is >= 1 and var n ? Math.Min(n, KnowledgeExport.MaxLimit) : KnowledgeExport.MaxLimit;
            return take < Total ? $"Выгрузить найденные ({take} из {Total})" : $"Выгрузить найденные ({Total})";
        }
    }

    /// <summary>«Не больше, заявок» числом; не число — -1.</summary>
    private int ExportLimit => int.TryParse(Limit.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var n) ? n : -1;

    public SearchViewModel(MainViewModel board, AppSettings settings, HttpIntraserviceClient? intraservice)
    {
        _board = board;
        _settings = settings;
        _intraservice = intraservice;
        _account = settings.AccountKey;
        Folder = settings.KnowledgePath(App.DataDir);
        FillReferences();   // пока пустые: «любой» и «нет»; настоящие — при первом показе окна
        ApplyFilter(settings.LastSearch ?? new SearchFilter(Mine: true, Status: SearchStatus.Closed));
    }

    /// <summary>Поменялось то, от чего зависит найденное, — список на экране больше не отвечает условиям. Выбор в списках
    /// запоминается в _wanted: настоящие списки приходят позже, и заполнение их кодом (_filling) не должно ни терять
    /// запомненное, ни выдавать себя за правку условий.</summary>
    protected override void OnPropertyChanged(PropertyChangedEventArgs e)
    {
        base.OnPropertyChanged(e);
        if (_filling || e.PropertyName is not { } name) return;
        if (QueryFields.Contains(name)) IsStale = true;
        switch (name)
        {
            case nameof(SelectedStatus) when SelectedStatus is { } s: _wanted = _wanted with { Status = s.Kind, StatusId = s.Id }; break;
            case nameof(SelectedService) when SelectedService is { } s: _wanted = _wanted with { ServiceId = s.Id }; break;
            case nameof(SelectedType) when SelectedType is { } t: _wanted = _wanted with { TypeId = t.Id }; break;
            case nameof(SelectedSaved) when SelectedSaved is { } f: _wanted = _wanted with { SavedFilterId = f.Id }; break;
        }
    }

    /// <summary>Настройки сохранены: новый клиент API. Тот же сервер и логин (поменяли хоткей, пароль…) — справочники и
    /// список на экране годятся, идущая выгрузка доходит старым клиентом: она уже на полпути. Другой сервер или логин —
    /// всё прежнее чужое: поиск и просмотр останавливаются, список, справочники и тексты забываются.</summary>
    public void ApplySettings(HttpIntraserviceClient? intraservice)
    {
        _intraservice = intraservice;
        var account = _settings.AccountKey;
        if (account == _account) return;
        _account = account;
        _lookup?.Cancel();
        _previewRun?.Cancel();
        _work?.Cancel();   // выгрузка и копирование идут со старого сервера: чужие заявки в папку не пишем
        _referencesRun = null;
        _referencesTried = false;
        _statuses = null;
        _services = _types = _saved = null;
        // номера сервиса, типа, фильтра и статуса — того сервера: на другом это другие сущности. Слова, имена и даты остаются
        _wanted = _wanted with { Status = _wanted.Status == SearchStatus.One ? SearchStatus.Any : _wanted.Status, StatusId = 0,
            ServiceId = 0, TypeId = 0, SavedFilterId = 0 };
        FillReferences();   // пустые списки: в окне сразу «любой», а не пункты прежнего сервера
        _texts.Clear();
        IsBusy = false;
        IsPreviewBusy = false;
        ClearResults();
        Message = "";
        Notes = "";
    }

    /// <summary>Окно открыли с кнопки или из трея: условия — как были. Справочники читаются при открытии, пока все не
    /// прочитаны (не вышло из-за сети — следующее открытие попробует снова).</summary>
    public async Task OpenAsync()
    {
        if (AllReferences || _intraservice is not { } client) return;
        await LoadReferencesAsync(client);
    }

    /// <summary>Окно открыли из поля поиска на доске: остальные условия — по умолчанию (иначе запомненное «мои закрытые»
    /// сузило бы поиск по слову, и нашлось бы не то), слова — из поля, поиск сразу.</summary>
    public async Task StartWithAsync(string words)
    {
        ApplyFilter(new SearchFilter(Words: words.Trim()));
        await OpenAsync();
        _skipRemember = true;   // привычные условия (в settings.json) остаются теми, с которыми окно откроют в следующий раз
        await SearchCommand.ExecuteAsync(null);
    }

    // ---------- справочники ----------

    /// <summary>Справочники читаются одним заходом, сколько бы мест их ни ждало (открытие окна и поиск).</summary>
    private Task LoadReferencesAsync(HttpIntraserviceClient client) =>
        _referencesRun is { IsCompleted: false } running ? running : _referencesRun = ReadReferencesAsync(client);

    /// <summary>Статусы, сервисы, типы и сохранённые фильтры, которых ещё нет, — запросами сразу. Не загрузился какой-то —
    /// окно работает без него, а что именно, видно под условиями; следующее открытие окна спросит только про него.
    /// Пока грузили, сменили сервер или логин — ответы чужие, не берём; поменяли лишь клиент (пароль, хоткей) — берём.</summary>
    private async Task ReadReferencesAsync(HttpIntraserviceClient client)
    {
        var account = _account;
        var statuses = _statuses is null ? client.GetStatusesAsync() : null;
        var services = _services is null ? client.GetServicesAsync() : null;
        var types = _types is null ? client.GetTaskTypesAsync() : null;
        var saved = _saved is null ? client.GetSavedFiltersAsync() : null;
        await Task.WhenAll(new Task?[] { statuses, services, types, saved }.OfType<Task>());
        if (account != _account) return;

        var failed = new List<string>();
        if (statuses is not null)
        {
            if (statuses.Result.Error.Length > 0) failed.Add($"статусы ({Brief(statuses.Result.Error)})");
            else _statuses = statuses.Result.Statuses;
        }
        if (services is not null)
        {
            if (services.Result.Error.Length > 0) failed.Add($"сервисы ({Brief(services.Result.Error)})");
            else _services = services.Result.Items;
        }
        if (types is not null)
        {
            if (types.Result.Error.Length > 0) failed.Add($"типы ({Brief(types.Result.Error)})");
            else _types = types.Result.Items;
        }
        if (saved is not null)
        {
            if (saved.Result.Error.Length > 0) failed.Add($"сохранённые фильтры ({Brief(saved.Result.Error)})");
            else _saved = saved.Result.Items;
        }
        FillReferences();
        _referencesTried = true;
        Notes = failed.Count > 0 ? "Не загрузились: " + string.Join(", ", failed) + ". Поиск работает и без них." : "";
    }

    private static string Brief(string error) => error.Split('\n')[0].Trim();

    /// <summary>Списки выбора — из прочитанных справочников (непрочитанные пусты: остаётся «любой»), выбор — по _wanted.</summary>
    private void FillReferences()
    {
        var was = _filling;
        _filling = true;
        try { FillChoices(); }
        finally { _filling = was; }
    }

    private void FillChoices()
    {
        StatusChoices.Clear();
        StatusChoices.Add(new(SearchStatus.Any, 0, "Любой"));
        StatusChoices.Add(new(SearchStatus.Closed, 0, "Закрытые"));
        StatusChoices.Add(new(SearchStatus.Open, 0, "Открытые"));
        foreach (var s in _statuses ?? Array.Empty<IntraserviceStatus>()) StatusChoices.Add(new(SearchStatus.One, s.Id, s.Name));

        ServiceChoices.Clear();
        ServiceChoices.Add(new(0, "— любой —"));
        foreach (var s in _services ?? Array.Empty<IntraserviceRef>()) ServiceChoices.Add(new(s.Id, TicketSearch.ServiceLabel(s)));

        TypeChoices.Clear();
        TypeChoices.Add(new(0, "— любой —"));
        foreach (var t in _types ?? Array.Empty<IntraserviceRef>()) TypeChoices.Add(new(t.Id, (t.IsArchive ? "(архив) " : "") + t.Name));

        SavedChoices.Clear();
        SavedChoices.Add(new(0, "— нет —"));
        foreach (var f in _saved ?? Array.Empty<IntraserviceRef>()) SavedChoices.Add(new(f.Id, f.IsDefault ? f.Name + " (по умолчанию)" : f.Name));

        SelectWanted();
    }

    // ---------- условия ----------

    private SearchFilter CurrentFilter() => new(
        Words: Words.Trim(), Mine: Mine, Executor: Executor.Trim(), Creator: Creator.Trim(),
        Status: SelectedStatus?.Kind ?? SearchStatus.Any, StatusId: SelectedStatus?.Id ?? 0,
        ServiceId: SelectedService?.Id ?? 0, WithChildren: WithChildren, TypeId: SelectedType?.Id ?? 0,
        CreatedFrom: CreatedFrom.Trim(), CreatedTo: CreatedTo.Trim(), ChangedFrom: ChangedFrom.Trim(), ChangedTo: ChangedTo.Trim(),
        ClosedFrom: ClosedFrom.Trim(), ClosedTo: ClosedTo.Trim(),
        SavedFilterId: SelectedSaved?.Id ?? 0, IncludeArchived: IncludeArchived,
        // не число — то же, что число вне пределов: текст ошибки один, у KnowledgeExport.InvalidLimit
        Limit: int.TryParse(Limit.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var limit) ? limit : -1);

    private void ApplyFilter(SearchFilter f)
    {
        Words = f.Words ?? "";
        Mine = f.Mine;
        Executor = f.Executor ?? "";
        Creator = f.Creator ?? "";
        WithChildren = f.WithChildren;
        CreatedFrom = f.CreatedFrom ?? "";
        CreatedTo = f.CreatedTo ?? "";
        ChangedFrom = f.ChangedFrom ?? "";
        ChangedTo = f.ChangedTo ?? "";
        ClosedFrom = f.ClosedFrom ?? "";
        ClosedTo = f.ClosedTo ?? "";
        IncludeArchived = f.IncludeArchived;
        Limit = f.Limit.ToString(CultureInfo.InvariantCulture);
        _wanted = f;
        SelectWanted();
    }

    /// <summary>Выбрать в списках то, что в _wanted; чего в списке нет (удалили, не загрузилось), — «любой».</summary>
    private void SelectWanted()
    {
        var f = _wanted;
        var was = _filling;
        _filling = true;
        try
        {
            SelectedStatus = StatusChoices.FirstOrDefault(c => c.Kind == f.Status && (f.Status != SearchStatus.One || c.Id == f.StatusId))
                ?? StatusChoices.FirstOrDefault();
            SelectedService = ServiceChoices.FirstOrDefault(c => c.Id == f.ServiceId) ?? ServiceChoices.FirstOrDefault();
            SelectedType = TypeChoices.FirstOrDefault(c => c.Id == f.TypeId) ?? TypeChoices.FirstOrDefault();
            SelectedSaved = SavedChoices.FirstOrDefault(c => c.Id == f.SavedFilterId) ?? SavedChoices.FirstOrDefault();
        }
        finally { _filling = was; }
    }

    [RelayCommand]
    private void Clear()
    {
        ApplyFilter(new SearchFilter());
        IsStale = true;   // условия сброшены, а список на экране — по прежним
        Message = "";
    }

    // ---------- поиск ----------

    // свой отмена-и-заново через _lookup: пока идёт один поиск, команда принимает следующий (по умолчанию — нет)
    [RelayCommand(AllowConcurrentExecutions = true)]
    private async Task Search()
    {
        _lookup?.Cancel();
        if (_intraservice is not { } client)
        {
            Message = "API не настроен: трей → Настройки…";
            return;
        }

        var cts = _lookup = new CancellationTokenSource();
        var remember = !_skipRemember;
        _skipRemember = false;
        ClearResults();
        IsBusy = true;
        Message = "ищу…";
        Warning = "";
        try
        {
            // справочники: ни разу не пробовали — читаем; не вышло со статусами, а «закрытые» и «открытые» без них не собрать, —
            // пробуем снова (сеть могла вернуться); остальное окно перечитает при следующем открытии
            if (!_referencesTried || (_statuses is null && SelectedStatus is { Kind: SearchStatus.Closed or SearchStatus.Open }))
                await LoadReferencesAsync(client);
            if (cts.IsCancellationRequested) return;
            var filter = CurrentFilter();
            var (resolved, error) = await TicketSearch.ResolveAsync(client, filter, _statuses ?? Array.Empty<IntraserviceStatus>(),
                _services ?? Array.Empty<IntraserviceRef>(), _settings.ClosedNames(), cts.Token);
            if (cts.IsCancellationRequested) return; // ответ пришёл, но ищут уже другое
            if (resolved is null)
            {
                Message = error;
                return;
            }
            _last = resolved;
            _page = 0;
            // пока разбирались с именами, условия могли поменять: «устарел» — по сравнению с теми, по которым ищем
            IsStale = CurrentFilter() with { Limit = filter.Limit } != filter;
            Matched = string.Join("\n", resolved.Notes);
            if (remember) Remember(filter);
            ExportFoundCommand.NotifyCanExecuteChanged();
            await LoadPageAsync(client, cts);
        }
        catch (OperationCanceledException) { /* запустили новый поиск — старый результат не нужен */ }
        finally
        {
            if (ReferenceEquals(_lookup, cts)) IsBusy = false;
        }
    }

    /// <summary>Ещё одна страница того же списка.</summary>
    [RelayCommand(AllowConcurrentExecutions = true)]
    private async Task ShowMore()
    {
        if (_intraservice is not { } client || _last is null || !HasMore) return;
        _lookup?.Cancel();
        var cts = _lookup = new CancellationTokenSource();
        IsBusy = true;
        try { await LoadPageAsync(client, cts); }
        catch (OperationCanceledException) { }
        finally
        {
            if (ReferenceEquals(_lookup, cts)) IsBusy = false;
        }
    }

    private async Task LoadPageAsync(HttpIntraserviceClient client, CancellationTokenSource cts)
    {
        var r = await client.GetTasksAsync(_last!.Query, _page + 1, cts.Token, pageSize: PageSize, notFound: "ничего не найдено", detailed: true);
        if (cts.IsCancellationRequested) return;
        if (r.Error.Length > 0)
        {
            Message = r.Error;
            return;
        }

        _page++;
        var onBoard = _board.AllTickets.Where(t => t.IntraserviceId is not null).Select(t => t.IntraserviceId!.Value).ToHashSet();
        var shown = Results.Select(x => x.Id).ToHashSet();
        foreach (var f in r.Found)
            if (shown.Add(f.Id))   // список мог сдвинуться между страницами — заявка на стыке пришла бы дважды
                Results.Add(new FoundTicketViewModel(f, _settings.TicketUrl(f.Id)) { OnBoard = onBoard.Contains(f.Id) });

        Total = Math.Max(r.Total, Results.Count);
        HasMore = r.Found.Count > 0 && Results.Count < Total && Results.Count < MaxShown;
        var outside = r.Found.Count(_last.Query.Outside);
        if (outside > 0)
            Warning = $"Сервер вернул заявки вне выбранного периода ({outside} из {r.Found.Count} на странице): условие по дате он, похоже, не применил — в списке лишнее";
        Message = Results.Count == 0 ? "Ничего не найдено"
            : Total > Results.Count ? $"Найдено: {Total} · показано {Results.Count}" + (Results.Count >= MaxShown ? " — уточните условия или выгрузите файлами" : "")
            : $"Найдено: {Total}";
    }

    private void ClearResults()
    {
        Results.Clear();
        _last = null;
        _page = 0;
        Total = 0;
        HasMore = false;
        IsStale = false;
        Warning = "";
        Matched = "";
        SetSelection(Array.Empty<FoundTicketViewModel>(), null);
        ExportFoundCommand.NotifyCanExecuteChanged();
    }

    /// <summary>Условия — в settings.json: окно откроется с ними, выгрузка повторится тем же отбором. Номера из выпадающих
    /// списков берём из _wanted, а не из того, что сейчас выбрано: справочник мог не загрузиться, и тогда в списке только
    /// «любой» — запомненные сервис, тип и фильтр от этого стираться не должны.</summary>
    private void Remember(SearchFilter filter)
    {
        // «не больше, заявок» не число — прежнее запомненное: поиску это поле не мешает, а в файле ему нечего делать
        var limit = filter.Limit is >= 1 and <= KnowledgeExport.MaxLimit ? filter.Limit : _settings.LastSearch?.Limit ?? 500;
        _settings.LastSearch = filter with { Status = _wanted.Status, StatusId = _wanted.StatusId, ServiceId = _wanted.ServiceId,
            TypeId = _wanted.TypeId, SavedFilterId = _wanted.SavedFilterId, Limit = limit };
        TrySave();
    }

    private void TrySave()
    {
        try { _settings.Save(App.DataDir); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { /* не запомнится — поиску не мешает */ }
    }

    // ---------- строки: выбор, просмотр ----------

    /// <summary>Выбрали строки списка (окно сообщает из SelectionChanged). Просмотр — самой последней из выбранных.</summary>
    public void SetSelection(IReadOnlyList<FoundTicketViewModel> rows, FoundTicketViewModel? focused)
    {
        _selected = rows;
        SelectedCount = rows.Count;
        var shown = focused is not null && rows.Contains(focused) ? focused : rows.LastOrDefault();
        _previewRun?.Cancel();
        if (shown is null)
        {
            IsPreviewBusy = false;
            PreviewTitle = "";
            PreviewText = "";
            return;
        }
        _ = ShowPreviewAsync(shown);
    }

    private async Task ShowPreviewAsync(FoundTicketViewModel row)
    {
        PreviewTitle = $"{row.DisplayNumber} {row.Title}";
        if (_intraservice is not { } client)
        {
            PreviewText = "";
            return;
        }
        var cts = _previewRun = new CancellationTokenSource();
        PreviewText = "";
        IsPreviewBusy = true;
        try
        {
            var (text, error) = await TextOfAsync(client, row, cts.Token);
            if (cts.IsCancellationRequested) return;   // уже смотрят другую
            PreviewText = error.Length > 0 ? $"Не удалось прочитать заявку:\n{error}" : text;
        }
        catch (OperationCanceledException) { }
        finally
        {
            if (ReferenceEquals(_previewRun, cts)) IsPreviewBusy = false;
        }
    }

    /// <summary>Текст заявки для агента — тот же, что ляжет в файл. Прочитанное раз держим, пока заявка не менялась: просмотр
    /// и буфер обмена не ходят на сервер за тем же дважды.</summary>
    private async Task<(string Text, string Error)> TextOfAsync(HttpIntraserviceClient client, FoundTicketViewModel row, CancellationToken ct)
    {
        // без даты изменения «не менялась» не доказать — читаем заново
        if (row.Found.Changed is { } changed && _texts.TryGetValue(row.Id, out var known) && known.Changed == changed) return (known.Text, "");
        var (text, error) = await KnowledgeExport.BuildAsync(client, row.Found, row.Url, ct);
        if (error.Length == 0 && row.Found.Changed is not null)
        {
            if (_texts.Count >= 60) _texts.Clear();
            _texts[row.Id] = (row.Found.Changed, text);
        }
        return (text, error);
    }

    // ---------- действия над строками ----------

    [RelayCommand]
    private void AddToBoard(FoundTicketViewModel? row)
    {
        if (row is null || row.OnBoard) return;
        // «уже на доске» посчитано, когда пришли результаты, а окно живёт дальше: заявку могли добавить и мимо него
        if (_board.AllTickets.Any(t => t.IntraserviceId == row.Id)) { row.OnBoard = true; return; }
        // номер, ссылка и название известны — карточка строится из них напрямую, без разбора текста регулярками из настроек
        _board.AddKnown(row.Id, row.Url, row.Title, TicketPriority.Mid);
        row.OnBoard = true;
    }

    [RelayCommand]
    private void OpenInBrowser(FoundTicketViewModel? row)
    {
        if (row is null || row.Url.Length == 0) return;
        try { Process.Start(new ProcessStartInfo(row.Url) { UseShellExecute = true }); }
        catch { /* нет браузера по умолчанию — окно из-за этого ронять не за что */ }
    }

    // ---------- буфер обмена и файлы ----------

    private bool CanWork() => !IsWorking;
    private bool CanWorkOnSelected() => !IsWorking && SelectedCount > 0;
    private bool CanExportFound() => !IsWorking && _last is not null && !IsStale && Total > 0;

    /// <summary>Начало копирования или выгрузки: новый токен остановки и «занято».</summary>
    private CancellationTokenSource BeginWork(string message)
    {
        var run = _work = new CancellationTokenSource();
        IsWorking = true;
        WorkMessage = message;
        return run;
    }

    private void EndWork(CancellationTokenSource run)
    {
        IsWorking = false;
        if (ReferenceEquals(_work, run)) _work = null;
        run.Dispose();
    }

    /// <summary>Выбранные заявки — текстом в буфер обмена, одна за другой: вставить в чат с агентом.</summary>
    [RelayCommand(CanExecute = nameof(CanWorkOnSelected))]
    private async Task Copy()
    {
        if (_intraservice is not { } client) { WorkMessage = "API не настроен: трей → Настройки…"; return; }
        var rows = _selected.ToList();
        if (rows.Count > MaxCopy)
        {
            WorkMessage = $"В буфер — не больше {MaxCopy} заявок за раз: в чат больше не поместится. Для большего — выгрузка файлами.";
            return;
        }

        var run = BeginWork($"Готовлю текст: 0 из {rows.Count}…");
        try
        {
            var texts = new string?[rows.Count];
            var errors = new List<string>();
            var done = 0;
            using var gate = new SemaphoreSlim(4);   // по 4 запроса разом, как у F5: сервер общий
            // продолжения идут в потоке окна: SynchronizationContext захвачен (запущено командой из UI)
            await Task.WhenAll(rows.Select(async (row, i) =>
            {
                await gate.WaitAsync(run.Token);
                try
                {
                    var (text, error) = await TextOfAsync(client, row, run.Token);
                    if (error.Length > 0) errors.Add($"{row.DisplayNumber}: {error}"); else texts[i] = text;
                }
                finally
                {
                    gate.Release();
                    done++;
                    WorkMessage = $"Готовлю текст: {done} из {rows.Count}…";
                }
            }));

            var ready = texts.Where(t => t is not null).ToList();
            var failed = errors.Count > 0 ? $"\nНе прочитались: {errors.Count}. Первая ошибка — {errors[0]}" : "";
            if (ready.Count == 0) { WorkMessage = "Ничего не скопировано." + failed; return; }
            var all = string.Join("\n\n", ready);
            WorkMessage = ClipboardWatcher.TrySetText(all)
                ? $"В буфере обмена — заявок: {ready.Count}, {all.Length.ToString("N0", CultureInfo.CurrentCulture)} символов. Вставьте в чат." + failed
                : "Буфер обмена занят другой программой — нажмите «Копировать» ещё раз.";
        }
        catch (OperationCanceledException) { WorkMessage = "Остановлено, в буфер ничего не положено."; }
        catch (Exception ex) { WorkMessage = $"Не вышло: {ex.Message}"; }
        finally { EndWork(run); }
    }

    /// <summary>Всё найденное по этим условиям (не больше «Не больше, заявок») — файлами в папку для базы знаний.</summary>
    [RelayCommand(CanExecute = nameof(CanExportFound))]
    private async Task ExportFound()
    {
        if (_intraservice is not { } client || _last is not { } resolved) return;
        var limit = ExportLimit;
        if (KnowledgeExport.InvalidLimit(limit) is { } badLimit) { WorkMessage = badLimit; return; }
        if (FolderOrNull() is not { } dir) return;

        var run = BeginWork("Начинаю…");
        var progress = new Progress<string>(m => { if (IsWorking && !run.IsCancellationRequested) WorkMessage = m; });
        try
        {
            var r = await Task.Run(() => KnowledgeExport.RunAsync(client, resolved.Query, limit, dir, _settings.TicketUrl, progress, run.Token));
            WorkMessage = Summary(r, "По отбору")
                + (limit < Total ? $"\nВсего найдено {Total}, взяты первые {limit}: больше не позволяет «Не больше, заявок» (до {KnowledgeExport.MaxLimit})." : "");
        }
        catch (Exception ex) { WorkMessage = Failure(ex, dir); }
        finally { EndWork(run); }
    }

    /// <summary>Только выбранные в списке — файлами в ту же папку.</summary>
    [RelayCommand(CanExecute = nameof(CanWorkOnSelected))]
    private async Task ExportSelected()
    {
        if (_intraservice is not { } client) return;
        var rows = _selected.Select(r => r.Found).ToList();
        if (FolderOrNull() is not { } dir) return;

        var run = BeginWork("Начинаю…");
        var progress = new Progress<string>(m => { if (IsWorking && !run.IsCancellationRequested) WorkMessage = m; });
        try
        {
            var r = await Task.Run(() => KnowledgeExport.ExportRowsAsync(client, rows, dir, _settings.TicketUrl, progress, run.Token));
            WorkMessage = Summary(r, "Выбрано");
        }
        catch (Exception ex) { WorkMessage = Failure(ex, dir); }
        finally { EndWork(run); }
    }

    /// <summary>Папка из поля — полный путь; запоминается в settings.json («knowledge» рядом с exe хранится пустой строкой:
    /// переедет вместе с exe). null — путь не годится, причина уже в сообщении.</summary>
    private string? FolderOrNull()
    {
        var dir = Folder.Trim();
        if (!Path.IsPathFullyQualified(dir))
        {
            WorkMessage = "Папка — полный путь, например D:\\Obsidian\\База\\Заявки";
            return null;
        }
        _settings.KnowledgeDir = string.Equals(Path.GetFullPath(dir), Path.GetFullPath(Path.Combine(App.DataDir, "knowledge")),
            StringComparison.OrdinalIgnoreCase) ? "" : dir;
        TrySave();
        return dir;
    }

    [RelayCommand(CanExecute = nameof(IsWorking))]
    private void Stop()
    {
        _work?.Cancel();
        WorkMessage = "Останавливаю — дописываю начатое…";
    }

    [RelayCommand]
    private void OpenFolder()
    {
        var dir = Folder.Trim();
        if (!Path.IsPathFullyQualified(dir)) return;
        try
        {
            Directory.CreateDirectory(dir);
            Process.Start(new ProcessStartInfo(dir) { UseShellExecute = true });
        }
        catch (Exception ex) { WorkMessage = $"Не открыть папку: {ex.Message}"; }
    }

    private static string Failure(Exception ex, string dir) => ex is IOException or UnauthorizedAccessException
        ? $"Не удалось записать в папку {dir}:\n{ex.Message}"
        : $"Выгрузка сорвалась: {ex.Message}";

    private static string Summary(ExportResult r, string what) =>
        r.Found == 0 ? (r.Error.Length > 0 ? r.Error : "Под отбор не попало ни одной заявки.")
        : $"Готово. {what} — {r.Found}: новых файлов {r.Created}, обновлено {r.Updated}, без изменений {r.Unchanged}."
          + (r.Failed > 0 ? $"\nНе прочитались: {r.Failed} — выгрузите ещё раз, дочитаются. Первая ошибка — {r.FirstError}" : "")
          + (r.Error.Length > 0 ? $"\n{r.Error}" : "");
}
