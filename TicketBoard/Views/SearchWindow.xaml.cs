using System.ComponentModel;
using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using Microsoft.Win32;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

/// <summary>Окно «Поиск заявок». Крестик и Esc прячут окно — идущая выгрузка продолжается.</summary>
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

    /// <summary>Открыть окно. С запросом (из поля поиска на доске) — условия по умолчанию, слова из поля, поиск сразу; без —
    /// как было закрыто.</summary>
    public void ShowSearch(string? query)
    {
        // CenterOwner без владельца не работает; из трея окно доски может быть ещё не показано — тогда без владельца
        if (Owner is null && Application.Current?.MainWindow is Window main && !ReferenceEquals(main, this) && main.IsVisible)
        {
            try { Owner = main; }
            catch (InvalidOperationException) { /* владельца ещё не показывали — откроемся без него */ }
        }

        Show();
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        Activate();
        WordsBox.Focus();
        WordsBox.SelectAll();
        // короткие слова поиск отобьёт сам — «минимум 3 символа», а не пустое окно
        _ = query is null ? _vm.OpenAsync() : _vm.StartWithAsync(query);
    }

    // Крестик — не выход: окно прячется, как и остальные.
    protected override void OnClosing(CancelEventArgs e)
    {
        e.Cancel = true;
        Hide();
    }

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape) return;
        Hide();
        e.Handled = true;
    }

    /// <summary>Enter в любом поле условий — искать. На кнопке и в списке Enter остаётся обычным: нажатием кнопки.</summary>
    private void OnFilterKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter || e.OriginalSource is not DependencyObject source || FindAncestor<TextBox>(source) is null) return;
        _vm.SearchCommand.Execute(null);
        e.Handled = true;
    }

    /// <summary>ui:TextBox — составной контрол, поэтому ищем TextBox вверх по дереву, а не сравниваем с источником события.</summary>
    private static T? FindAncestor<T>(DependencyObject? from) where T : DependencyObject
    {
        for (var d = from; d is not null; d = d is Visual or System.Windows.Media.Media3D.Visual3D ? VisualTreeHelper.GetParent(d) : LogicalTreeHelper.GetParent(d))
            if (d is T found) return found;
        return null;
    }

    private void OnSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        var rows = ResultsList.SelectedItems.OfType<FoundTicketViewModel>().ToList();
        _vm.SetSelection(rows, e.AddedItems.OfType<FoundTicketViewModel>().LastOrDefault());
    }

    /// <summary>Двойной клик по строке — открыть заявку в браузере; на кнопках строки — нет, у них своё действие.</summary>
    private void OnRowDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (e.OriginalSource is not DependencyObject source || FindAncestor<ButtonBase>(source) is not null) return;
        if (ItemsControl.ContainerFromElement(ResultsList, source) is ListBoxItem { DataContext: FoundTicketViewModel row })
            _vm.OpenInBrowserCommand.Execute(row);
    }

    private void OnChooseFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog
        {
            Title = "Куда выгружать заявки",
            InitialDirectory = Directory.Exists(_vm.Folder.Trim()) ? _vm.Folder.Trim() : App.DataDir,
        };
        if (dialog.ShowDialog(this) == true) _vm.Folder = dialog.FolderName;
    }
}
