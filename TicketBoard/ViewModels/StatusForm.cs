using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Окно «Сменить статус заявки #N?» (этап 8, шаг 2): что показать — название с сервера, нынешний статус, последние
/// записи переписки, разрешённые переходы, — и что человек выбрал. Всё, что окно решает, — здесь, без WPF.</summary>
public sealed partial class StatusForm : ObservableObject
{
    public StatusForm(int ticket, string title, string current, IReadOnlyList<IntraserviceStatus> choices,
        IntraserviceStatus? proposed, string note, IReadOnlyList<CommentRow> recent)
    {
        Ticket = ticket;
        Title = title;
        Current = current;
        Choices = choices;
        Note = note;
        Recent = recent;
        _target = proposed;
    }

    public int Ticket { get; }
    public string Title { get; }
    public string Current { get; }
    public IReadOnlyList<IntraserviceStatus> Choices { get; }
    public string Note { get; }
    public IReadOnlyList<CommentRow> Recent { get; }

    [ObservableProperty] private IntraserviceStatus? _target;
    [ObservableProperty] private string _comment = "";
    [ObservableProperty] private bool _internal;

    // заглушка: проверки сначала красные
    public bool CommentRequired => false;
    public bool CanApply => Target is not null;
}
