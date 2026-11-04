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
    public bool HasRecent => Recent.Count > 0;

    /// <summary>Новый статус; null — не выбран (статуса колонки нет среди разрешённых или на сервере).</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(CommentRequired), nameof(CanApply), nameof(CommentHint))]
    private IntraserviceStatus? _target;

    [ObservableProperty, NotifyPropertyChangedFor(nameof(CanApply))]
    private string _comment = "";

    /// <summary>Комментарий внутренний (видят только сотрудники). Снят: комментарий к смене статуса обычно для заявителя —
    /// решение, вопрос.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(KindText))]
    private bool _internal;

    public string Heading => $"Сменить статус заявки #{Ticket}?";

    /// <summary>У выбранного статуса комментарий обязателен (IsCommentRequired справочника).</summary>
    public bool CommentRequired => Target?.IsCommentRequired == true;

    /// <summary>«Сменить статус» нажимается: статус выбран, обязательный комментарий написан.</summary>
    public bool CanApply => Target is not null && (!CommentRequired || Comment.Trim().Length > 0);

    public string CommentHint => CommentRequired ? "Комментарий — обязателен для этого статуса" : "Комментарий — по желанию";

    public string KindText => Internal ? "Внутренний — увидят только сотрудники" : "Публичный — увидит инициатор";
}
