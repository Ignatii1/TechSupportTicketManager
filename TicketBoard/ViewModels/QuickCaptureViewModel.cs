using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

public sealed partial class QuickCaptureViewModel : ObservableObject
{
    private readonly IntraserviceLinkParser _parser;

    public string HotkeyText { get; }

    [ObservableProperty] private string _text = "";
    [ObservableProperty] private TicketPriority _priority = TicketPriority.Mid;

    [ObservableProperty] private bool _hasNumber;
    [ObservableProperty] private string _numberText = "";
    [ObservableProperty] private string _hint = "";
    /// <summary>Превью названия из Интрасервиса — появится с API.</summary>
    [ObservableProperty] private string _preview = "Название подтянется из Интрасервиса";

    public QuickCaptureViewModel(IntraserviceLinkParser parser, AppSettings settings)
    {
        _parser = parser;
        HotkeyText = settings.Hotkey;
        Reset();
    }

    public void Reset()
    {
        Text = "";
        Priority = TicketPriority.Mid;
    }

    /// <summary>Цифры 1/2/3 меняют приоритет, только когда поле пустое или в нём распознанная ссылка.</summary>
    public bool DigitsSetPriority => Text.Length == 0 || HasNumber;

    partial void OnTextChanged(string value)
    {
        _parser.TryParse(value, out _, out var id);
        HasNumber = id is not null;
        NumberText = id is int n ? $"#{n}" : "";
        Hint = value.Length == 0
            ? "Ссылка вида …/Task/View/702180 распознаётся автоматически"
            : "Будет создана заявка с этим названием";
    }
}
