using System.Windows.Controls;

namespace TicketBoard.Views;

/// <summary>Панель деталей заявки: только разметка, данные и команды — у MainViewModel окна (DataContext наследуется).</summary>
public partial class TicketPanel : UserControl
{
    public TicketPanel() => InitializeComponent();
}
