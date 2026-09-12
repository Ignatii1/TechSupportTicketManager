using System.Windows;
using System.Windows.Input;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

public partial class SettingsWindow : Wpf.Ui.Controls.FluentWindow
{
    private readonly SettingsViewModel _vm;

    /// <summary>Настройки записаны в settings.json — App применяет их без перезапуска.</summary>
    public event Action? Saved;

    public SettingsWindow(AppSettings settings)
    {
        _vm = new SettingsViewModel(settings);
        DataContext = _vm;
        InitializeComponent();
        SystemThemeWatcher.Watch(this);
    }

    private void OnSave(object sender, RoutedEventArgs e)
    {
        if (!_vm.IsValid)
        {
            _vm.SaveError = "Исправьте поля с ошибками";
            return;
        }
        try { _vm.Save(PasswordInput.Password); }
        catch (Exception ex)
        {
            _vm.SaveError = $"Не удалось сохранить: {ex.Message}";
            return;
        }
        Saved?.Invoke();
        Close();
    }

    private void OnCancel(object sender, RoutedEventArgs e) => Close();

    private async void OnCheck(object sender, RoutedEventArgs e)
    {
        CheckButton.IsEnabled = false;
        try { await _vm.CheckAsync(PasswordInput.Password); }
        finally { CheckButton.IsEnabled = true; }
    }

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape) return;
        Close();
        e.Handled = true;
    }
}
