using System.ComponentModel;
using System.IO;
using System.Windows;
using System.Windows.Input;
using Microsoft.Win32;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

/// <summary>Окно выгрузки для базы знаний. Крестик и Esc прячут окно — идущая выгрузка продолжается.</summary>
public partial class ExportWindow : Wpf.Ui.Controls.FluentWindow
{
    private readonly ExportViewModel _vm;

    public ExportWindow(ExportViewModel vm)
    {
        _vm = vm;
        DataContext = vm;
        InitializeComponent();
        SystemThemeWatcher.Watch(this);
    }

    public void ShowExport()
    {
        // CenterOwner без владельца не работает; из трея окно доски может быть ещё не показано — тогда без владельца
        if (Owner is null && Application.Current?.MainWindow is Window main && !ReferenceEquals(main, this) && main.IsVisible)
        {
            try { Owner = main; }
            catch (InvalidOperationException) { }
        }
        Show();
        if (WindowState == WindowState.Minimized) WindowState = WindowState.Normal;
        Activate();
    }

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
