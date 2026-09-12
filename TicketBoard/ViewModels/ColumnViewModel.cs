using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Windows.Data;
using CommunityToolkit.Mvvm.ComponentModel;
using GongSolutions.Wpf.DragDrop;
using TicketBoard.Models;

namespace TicketBoard.ViewModels;

/// <summary>Колонка доски. Сама принимает drop карточек (gong-wpf-dragdrop) и сообщает наверх, что заявка переехала.</summary>
public sealed partial class ColumnViewModel : ObservableObject, IDropTarget
{
    private readonly Action<Ticket, ColumnViewModel> _onDropped;
    private readonly IDropTarget _default = new DefaultDropHandler();

    public TicketStatus Status { get; }
    public string Title { get; }
    public ObservableCollection<Ticket> Items { get; } = new();
    public ICollectionView View { get; }

    /// <summary>Сколько карточек видно после фильтров — это и показывает счётчик.</summary>
    [ObservableProperty, NotifyPropertyChangedFor(nameof(IsOverloaded))]
    private int _visibleCount;

    /// <summary>Курсор с карточкой над колонкой — подсветка drop-target.</summary>
    [ObservableProperty] private bool _isDragOver;

    /// <summary>«Входящих нет — всё в работе» (только для Входящих при непустой доске).</summary>
    [ObservableProperty] private bool _showEmptyHint;

    /// <summary>Подпись справа в заголовке: «лимит 5» / «скрыты старше 7 д».</summary>
    [ObservableProperty] private string _hint = "";

    public bool IsOverloaded => Status == TicketStatus.InProgress && VisibleCount > TicketRules.OverloadLimit;

    public ColumnViewModel(TicketStatus status, string title, Action<Ticket, ColumnViewModel> onDropped)
    {
        Status = status;
        Title = title;
        _onDropped = onDropped;
        View = CollectionViewSource.GetDefaultView(Items);
    }

    public void Recount() => VisibleCount = View.Cast<object>().Count();

    // ---- gong-wpf-dragdrop ----

    void IDropTarget.DragEnter(IDropInfo dropInfo)
    {
        _default.DragEnter(dropInfo);
        IsDragOver = true;
    }

    void IDropTarget.DragOver(IDropInfo dropInfo)
    {
        _default.DragOver(dropInfo);
        IsDragOver = true;
    }

    void IDropTarget.DragLeave(IDropInfo dropInfo)
    {
        _default.DragLeave(dropInfo);
        IsDragOver = false;
    }

    void IDropTarget.Drop(IDropInfo dropInfo)
    {
        IsDragOver = false;
        _default.Drop(dropInfo);
        if (dropInfo.Data is Ticket t) _onDropped(t, this);
    }
}
