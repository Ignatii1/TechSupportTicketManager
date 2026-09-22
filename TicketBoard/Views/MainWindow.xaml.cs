using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using TicketBoard.Models;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

public partial class MainWindow : Wpf.Ui.Controls.FluentWindow
{
    private const double OverlayBreakpoint = 1100;
    private readonly MainViewModel _vm;

    /// <summary>Enter в поле поиска: искать не по доске, а на сервере — Интрасервис ищет ещё и по комментариям.</summary>
    public event Action<string>? ServerSearchRequested;

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

    // ---------- появление карточки после переноса: 200 мс, opacity 0→1, сдвиг −4 → 0 ----------

    // Loaded приходит на каждый новый контейнер (старт, Refresh() фильтров) — анимирует только свежая отметка заявки.
    private void OnCardLoaded(object sender, RoutedEventArgs e)
    {
        // сначала ListBoxItem: копия шаблона в drag-adorner не должна съесть отметку
        if (sender is not FrameworkElement { DataContext: Ticket t } card
            || FindAncestor<ListBoxItem>(card) is not ListBoxItem item || !t.TakeAppear()) return;

        var shift = new TranslateTransform(0, -4);
        item.RenderTransform = item.RenderTransform is Transform rt && rt != Transform.Identity
            ? new TransformGroup { Children = { rt, shift } } : shift;
        item.Opacity = 0; // база = начало анимации, без кадра-вспышки до старта часов
        item.BeginAnimation(OpacityProperty, Decelerate(1));
        shift.BeginAnimation(TranslateTransform.YProperty, Decelerate(0));
    }

    // cubic-bezier(0,0,0,1) — WinUI decelerate
    private static DoubleAnimationUsingKeyFrames Decelerate(double to) => new()
    {
        KeyFrames = { new SplineDoubleKeyFrame(to, KeyTime.FromTimeSpan(TimeSpan.FromMilliseconds(200)), new KeySpline(0, 0, 0, 1)) },
    };

    // ---------- клавиатура: N · / · ← → · 1/2/3 · Enter · Esc · F5 ----------

    private void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        var ctrl = Keyboard.Modifiers.HasFlag(ModifierKeys.Control);
        var inText = Keyboard.FocusedElement is TextBoxBase or ComboBox;

        if (e.Key == Key.Escape)
        {
            if (inText)
            {
                if (SearchBox.IsKeyboardFocusWithin) SearchBox.Text = "";
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

        // F5 — не символ, поэтому работает и из поля ввода; Ctrl+F5 и Shift+F5 — нет: их жмут по привычке из браузера
        if (e.Key == Key.F5 && Keyboard.Modifiers == ModifierKeys.None)
        {
            if (_vm.RefreshAllCommand.CanExecute(null)) _vm.RefreshAllCommand.Execute(null);
            e.Handled = true;
            return;
        }

        // IsKeyboardFocusWithin, а не сравнение с Keyboard.FocusedElement: ui:TextBox — составной контрол
        if (e.Key == Key.Enter && SearchBox.IsKeyboardFocusWithin)
        {
            ServerSearchRequested?.Invoke(SearchBox.Text);
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
            case Key.D1 or Key.NumPad1 when !ctrl && _vm.SelectedTicket is not null:
                _vm.SetSelectedPriority(TicketPriority.Low);
                e.Handled = true;
                break;
            case Key.D2 or Key.NumPad2 when !ctrl && _vm.SelectedTicket is not null:
                _vm.SetSelectedPriority(TicketPriority.Mid);
                e.Handled = true;
                break;
            case Key.D3 or Key.NumPad3 when !ctrl && _vm.SelectedTicket is not null:
                _vm.SetSelectedPriority(TicketPriority.High);
                e.Handled = true;
                break;
            case Key.Delete when ctrl && _vm.SelectedTicket is not null:
                _vm.DeleteSelectedCommand.Execute(null); // спросит подтверждение
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
