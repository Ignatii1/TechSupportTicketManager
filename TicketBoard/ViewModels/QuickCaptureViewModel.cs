using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

public sealed partial class QuickCaptureViewModel : ObservableObject
{
    private readonly IntraserviceLinkParser _parser;
    private readonly AppSettings _settings;
    private HttpIntraserviceClient? _intraservice; // null — API не настроен
    private CancellationTokenSource? _lookup;
    private int? _lookupId;

    public string HotkeyText => _settings.Hotkey;

    [ObservableProperty] private string _text = "";
    [ObservableProperty] private TicketPriority _priority = TicketPriority.Mid;

    [ObservableProperty] private bool _hasNumber;
    [ObservableProperty] private string _numberText = "";
    [ObservableProperty] private string _hint = "";
    /// <summary>Название заявки из Интрасервиса по распознанному номеру (или «не найдена» / «сервер недоступен»).</summary>
    [ObservableProperty] private string _preview = "";

    public QuickCaptureViewModel(IntraserviceLinkParser parser, AppSettings settings, HttpIntraserviceClient? intraservice)
    {
        _parser = parser;
        _settings = settings;
        _intraservice = intraservice;
        Reset();
    }

    /// <summary>Настройки сохранены: новый клиент API, хоткей мог поменяться.</summary>
    public void ApplySettings(HttpIntraserviceClient? intraservice)
    {
        _intraservice = intraservice;
        _lookupId = null;
        OnPropertyChanged(nameof(HotkeyText));
    }

    public void Reset()
    {
        Text = "";
        Priority = TicketPriority.Mid;
    }

    private bool _hasUrl;

    /// <summary>Цифры 1/2/3 меняют приоритет, только когда в поле распознанная ссылка. В пустом поле — нет:
    /// с него начинается ввод номера, а «123456» иначе не набрать.
    /// ponytail: если будет мешать, добавить Ctrl+1/2/3 как приоритет в любом состоянии поля.</summary>
    public bool DigitsSetPriority => _hasUrl;

    partial void OnTextChanged(string value)
    {
        _hasUrl = _parser.TryParse(value, out _, out var id);
        HasNumber = id is not null;
        NumberText = id is int n ? $"#{n}" : "";
        Hint = value.Length == 0
            ? "Ссылка вида …/Task/View/702180 или просто номер заявки"
            : "Будет создана заявка с этим названием";
        if (id != _lookupId) LookupTitle(id);
    }

    /// <summary>Название по номеру: пауза 400 мс после ввода, предыдущий запрос отменяется. Ввод и Enter не ждут.</summary>
    private async void LookupTitle(int? id)
    {
        _lookup?.Cancel();
        _lookupId = id;
        Preview = "";
        if (id is not int n || _intraservice is not { } client) return;

        var cts = _lookup = new CancellationTokenSource();
        Preview = "Ищу в Интрасервисе…";
        try
        {
            await Task.Delay(400, cts.Token);
            var r = await client.GetTaskAsync(n, cts.Token);
            // ответ пришёл, но ввод уже другой — не пишем. Ошибка здесь — одной строкой (Brief): сырой ответ сервера
            // раздул бы окно быстрого добавления; целиком он виден в панели заявки и в errors.log
            if (!cts.IsCancellationRequested) Preview = r.Task?.Name ?? HttpIntraserviceClient.Brief(r.Error);
        }
        catch (OperationCanceledException) { /* ввели другое — неважно */ }
    }
}
