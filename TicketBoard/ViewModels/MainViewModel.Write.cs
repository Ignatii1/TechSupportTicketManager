using CommunityToolkit.Mvvm.Input;
using TicketBoard.Models;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Черновик комментария заявки: текст, вид (внутренний) и итог последней отправки — в памяти, у каждой заявки свой.</summary>
public sealed record CommentDraft(string Text, bool Internal, string Result);

/// <summary>Запись в Интрасервис (этап 8): комментарий из панели.</summary>
public sealed partial class MainViewModel
{
    // заглушка: проверки сначала красные
    public bool CanComment => false;
    public string CommentText { get; set; } = "";
    public bool CommentInternal { get; set; } = true;
    public string CommentResult => "";

    [RelayCommand]
    private Task SendComment() => Task.CompletedTask;
}
