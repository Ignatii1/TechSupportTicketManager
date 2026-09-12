using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using TicketBoard.Models;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

public partial class MainWindow : Wpf.Ui.Controls.FluentWindow
{
    private const double OverlayBreakpoint = 1100;
    private readonly MainViewModel _vm;

    public MainWindow(MainViewModel vm)
    {
        _vm = vm;
        DataContext = vm;
        InitializeComponent();
        SystemThemeWatcher.Watch(this); // светлая/тёмная вслед за системой
    }

    public void ShowAndActivate()
    {
        if (!IsVisible) Show();
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        Activate();
    }

    // Крестик — не выход, а в трей. Выход только из меню трея.
    protected override void OnClosing(CancelEventArgs e)
    {
        e.Cancel = true;
        Hide();
    }

    private void OnSizeChanged(object sender, SizeChangedEventArgs e) => _vm.IsOverlay = ActualWidth < OverlayBreakpoint;

    private void OnScrimClick(object sender, MouseButtonEventArgs e) => _vm.ClosePanelCommand.Execute(null);

    // ---------- выделение: четыре списка, одна выбранная заявка ----------

    private void OnColumnSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ListBox list || list.SelectedItem is not Ticket t) return;
        _vm.SelectedTicket = t;
        foreach (var other in FindChildren<ListBox>(this))
            if (other != list) other.SelectedItem = null;
    }

    private void SelectInLists(Ticket? t, bool focus)
    {
        foreach (var list in FindChildren<ListBox>(this))
        {
            var mine = t is not null && list.Items.Contains(t);
            list.SelectedItem = mine ? t : null;
            if (mine && focus)
            {
                list.UpdateLayout();
                list.ScrollIntoView(t!);
                (list.ItemContainerGenerator.ContainerFromItem(t) as ListBoxItem)?.Focus();
            }
        }
    }

    private void OnCardRightClick(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<ListBoxItem>(e.OriginalSource as DependencyObject) is ListBoxItem item)
            item.IsSelected = true;
    }

    private void OnCardDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (FindAncestor<ListBoxItem>(e.OriginalSource as DependencyObject) is not null)
            _vm.OpenLinkCommand.Execute(null);
    }

    // ---------- клавиатура: N · / · ← → · Enter · Esc ----------

    private void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        var ctrl = Keyboard.Modifiers.HasFlag(ModifierKeys.Control);
        var inText = Keyboard.FocusedElement is TextBoxBase or ComboBox;

        if (e.Key == Key.Escape)
        {
            if (inText)
            {
                if (ReferenceEquals(Keyboard.FocusedElement, SearchBox)) SearchBox.Text = "";
                FocusSelectedCard();
            }
            else if (_vm.IsPanelOpen)
            {
                _vm.ClosePanelCommand.Execute(null);
            }
            else
            {
                _vm.SelectedTicket = null;
                SelectInLists(null, focus: false);
            }
            e.Handled = true;
            return;
        }

        if (ctrl && e.Key == Key.F)
        {
            FocusSearch();
            e.Handled = true;
            return;
        }
        if (ctrl && e.Key == Key.N)
        {
            _vm.RequestCaptureCommand.Execute(null);
            e.Handled = true;
            return;
        }

        if (inText) return; // в полях ввода одиночные клавиши — это текст

        switch (e.Key)
        {
            case Key.N:
                _vm.RequestCaptureCommand.Execute(null);
                e.Handled = true;
                break;
            case Key.OemQuestion or Key.Divide:
                FocusSearch();
                e.Handled = true;
                break;
            case Key.Left or Key.Right when _vm.SelectedTicket is not null:
                _vm.MoveSelected(e.Key == Key.Right ? +1 : -1);
                SelectInLists(_vm.SelectedTicket, focus: true);
                e.Handled = true;
                break;
            case Key.Enter when _vm.SelectedTicket is not null:
                _vm.TogglePanel();
                e.Handled = true;
                break;
        }
    }

    private void FocusSearch()
    {
        SearchBox.Focus();
        SearchBox.SelectAll();
    }

    private void FocusSelectedCard()
    {
        if (_vm.SelectedTicket is Ticket t) SelectInLists(t, focus: true);
        else Keyboard.ClearFocus();
    }

    // ---------- helpers ----------

    private static IEnumerable<T> FindChildren<T>(DependencyObject root) where T : DependencyObject
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(root); i++)
        {
            var child = VisualTreeHelper.GetChild(root, i);
            if (child is T hit) yield return hit;
            foreach (var deeper in FindChildren<T>(child)) yield return deeper;
        }
    }

    private static T? FindAncestor<T>(DependencyObject? node) where T : DependencyObject
    {
        while (node is not null and not T)
            node = node is Visual or System.Windows.Media.Media3D.Visual3D ? VisualTreeHelper.GetParent(node) : LogicalTreeHelper.GetParent(node);
        return node as T;
    }
}
