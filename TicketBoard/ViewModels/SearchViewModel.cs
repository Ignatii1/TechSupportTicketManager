using System.Collections.ObjectModel;
using System.Diagnostics;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Строка результатов поиска. Всё, кроме «уже на доске», задаётся один раз при создании:
/// найденное на сервере мы не редактируем.</summary>
public sealed partial class FoundTicketViewModel : ObservableObject
{
    public FoundTicketViewModel(IntraserviceFound found, string url)
    {
        Id = found.Id;
        Title = found.Name;
        Status = found.Status;
        // подпись собирается здесь, а не через StringFormat в XAML: в разметке фигурные скобки внутри значения — лишний риск
        Creator = found.Creator?.Trim() is { Length: > 0 } c ? $"автор: {c}" : "";
        // даты приходят в часовом поясе пользователя Интрасервиса — показываем как есть, не переводя в местное время
        Created = found.Created is { } d ? d.ToString("dd.MM.yyyy") : "";
        Url = url;
    }

    public int Id { get; }
    public string Title { get; }
    public string Status { get; }
    /// <summary>«автор: Фамилия И.О.» или «» — сервер автора не прислал.</summary>
    public string Creator { get; }
    /// <summary>Дата создания, уже готовая к показу («» — сервер её не прислал).</summary>
    public string Created { get; }
    /// <summary>Адрес заявки или «», если базовый адрес не настроен.</summary>
    public string Url { get; }
    public string DisplayNumber => $"#{Id}";

    /// <summary>Такая заявка уже есть на доске: строка гасится, кнопки «на доску» нет.</summary>
    [ObservableProperty] private bool _onBoard;
}

/// <summary>Поиск заявок на сервере: Интрасервис ищет и по полям заявки, и по тексту всех её комментариев,
/// поэтому находит то, чего на доске нет. Результаты нигде не сохраняются — только на время жизни окна.</summary>
public sealed partial class SearchViewModel : ObservableObject
{
    private readonly MainViewModel _board;
    private readonly AppSettings _settings;
    private HttpIntraserviceClient? _intraservice; // null — API не настроен
    private CancellationTokenSource? _lookup;

    public ObservableCollection<FoundTicketViewModel> Results { get; } = new();

    [ObservableProperty] private string _query = "";
    /// <summary>Единственная строка состояния: «ищу…», «ничего не найдено», ошибка или сколько из скольких показано.</summary>
    [ObservableProperty] private string _message = "";
    [ObservableProperty] private bool _isBusy;

    public SearchViewModel(MainViewModel board, AppSettings settings, HttpIntraserviceClient? intraservice)
    {
        _board = board;
        _settings = settings;
        _intraservice = intraservice;
    }

    /// <summary>Настройки сохранены: новый клиент API и новый адрес — старые результаты больше не годятся.</summary>
    public void ApplySettings(HttpIntraserviceClient? intraservice)
    {
        _intraservice = intraservice;
        Reset("");
    }

    /// <summary>Окно открывают заново: показываем чистый лист, а не результаты прошлого поиска.</summary>
    public void Reset(string query)
    {
        _lookup?.Cancel();
        Query = query;
        Results.Clear();
        Message = "";
        IsBusy = false;
    }

    [RelayCommand]
    private async Task Search()
    {
        _lookup?.Cancel();
        var text = Query.Trim();
        Results.Clear();
        IsBusy = false;
        // на «ма» сервер вернёт пол-базы, и ждать этого придётся дольше всего; такой запрос никуда не отправляем
        if (text.Length < 3)
        {
            Message = "минимум 3 символа";
            return;
        }
        if (_intraservice is not { } client)
        {
            Message = "API не настроен";
            return;
        }

        var cts = _lookup = new CancellationTokenSource();
        IsBusy = true;
        Message = "ищу…";
        try
        {
            var r = await client.SearchAsync(text, cts.Token);
            if (cts.IsCancellationRequested) return; // ответ пришёл, но ищут уже другое
            Show(r);
        }
        catch (OperationCanceledException) { /* запустили новый поиск — старый результат не нужен */ }
        finally
        {
            if (!cts.IsCancellationRequested) IsBusy = false;
        }
    }

    private void Show(IntraserviceSearchResult r)
    {
        if (r.Error.Length > 0)
        {
            Message = r.Error;
            return;
        }

        var onBoard = _board.AllTickets.Where(t => t.IntraserviceId is not null).Select(t => t.IntraserviceId!.Value).ToHashSet();
        foreach (var f in r.Found)
            Results.Add(new FoundTicketViewModel(f, UrlFor(f.Id)) { OnBoard = onBoard.Contains(f.Id) });

        Message = Results.Count == 0 ? "ничего не найдено"
            : r.Total > Results.Count ? $"показаны первые {Results.Count} из {r.Total}"
            : $"найдено: {Results.Count}";
    }

    [RelayCommand]
    private void AddToBoard(FoundTicketViewModel? row)
    {
        if (row is null || row.OnBoard) return;
        // «уже на доске» посчитано, когда пришли результаты, а окно живёт дальше: заявку могли добавить и мимо него
        if (_board.AllTickets.Any(t => t.IntraserviceId == row.Id)) { row.OnBoard = true; return; }
        // тот же путь, что и у быстрого добавления: без адреса в тексте нет номера, поэтому отдаём хотя бы «#12345» —
        // парсер возьмёт номер, а название подставит ближайшая синхронизация
        _board.AddFromCapture(row.Url.Length > 0 ? $"{row.Url} {row.Title}" : $"#{row.Id}", TicketPriority.Mid);
        row.OnBoard = true;
    }

    [RelayCommand]
    private void OpenInBrowser(FoundTicketViewModel? row)
    {
        if (row is null || row.Url.Length == 0) return;
        try { Process.Start(new ProcessStartInfo(row.Url) { UseShellExecute = true }); }
        catch { /* нет браузера по умолчанию — окно из-за этого ронять не за что */ }
    }

    /// <summary>Адрес заявки; без настроенного базового адреса — пустая строка.
    /// ponytail: адрес заявки собирается в двух местах (ещё MainViewModel.TicketUrl) — поменяется путь, править оба.</summary>
    private string UrlFor(int id)
    {
        var b = _settings.IntraserviceBaseUrl.Trim().TrimEnd('/');
        return b.Length > 0 ? $"{b}/Task/View/{id}" : "";
    }
}
