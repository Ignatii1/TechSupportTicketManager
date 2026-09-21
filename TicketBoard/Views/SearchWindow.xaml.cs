using System.ComponentModel;
using System.Windows;
using System.Windows.Input;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

public partial class SearchWindow : Wpf.Ui.Controls.FluentWindow
{
    private readonly SearchViewModel _vm;

    public SearchWindow(SearchViewModel vm)
    {
        _vm = vm;
        DataContext = vm;
        InitializeComponent();
        SystemThemeWatcher.Watch(this);
    }

    /// <summary>Открыть окно с запросом из поля поиска на доске и сразу поискать.</summary>
    public void ShowSearch(string query)
    {
        _vm.Reset((query ?? "").Trim());
        // CenterOwner без владельца не работает; к этому моменту главное окно уже показано — поиск открывается из него
        if (Owner is null && Application.Current?.MainWindow is Window main && !ReferenceEquals(main, this))
        {
            try { Owner = main; }
            catch (InvalidOperationException) { /* владельца ещё не показывали — откроемся без него */ }
        }

        Show();
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        Activate();
        QueryBox.Focus();
        QueryBox.SelectAll();
        _vm.SearchCommand.Execute(null); // короткий запрос команда отобьёт сама — «минимум 3 символа», а не пустое окно
    }

    // Крестик — не выход: окно прячется, как и остальные.
    protected override void OnClosing(CancelEventArgs e)
    {
        e.Cancel = true;
        Hide();
    }

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        switch (e.Key)
        {
            case Key.Escape:
                Hide();
                e.Handled = true;
                break;

            // IsKeyboardFocusWithin, а не сравнение с Keyboard.FocusedElement: ui:TextBox — составной контрол.
            // На кнопке в строке результата Enter остаётся нажатием кнопки.
            case Key.Enter when QueryBox.IsKeyboardFocusWithin:
                _vm.SearchCommand.Execute(null);
                e.Handled = true;
                break;
        }
    }

    /// <summary>Двойной клик по строке — открыть заявку в браузере.</summary>
    private void OnRowClick(object sender, MouseButtonEventArgs e)
    {
        if (e.ClickCount == 2 && sender is FrameworkElement { DataContext: FoundTicketViewModel row })
            _vm.OpenInBrowserCommand.Execute(row);
    }
}
