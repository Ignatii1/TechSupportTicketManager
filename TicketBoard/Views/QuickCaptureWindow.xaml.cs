using System.ComponentModel;
using System.Windows;
using System.Windows.Input;
using TicketBoard.Models;
using TicketBoard.Services;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;

namespace TicketBoard.Views;

public partial class QuickCaptureWindow : Wpf.Ui.Controls.FluentWindow
{
    private readonly MainViewModel _main;
    private readonly QuickCaptureViewModel _vm;
    private readonly IntraserviceLinkParser _parser;

    public QuickCaptureWindow(MainViewModel main, QuickCaptureViewModel vm, IntraserviceLinkParser parser)
    {
        _main = main;
        _vm = vm;
        _parser = parser;
        DataContext = vm;
        InitializeComponent();
        SystemThemeWatcher.Watch(this);
    }

    public void ShowCapture()
    {
        _vm.Reset();
        // Скопировал ссылку в браузере → хоткей → Enter. Самый частый сценарий.
        try
        {
            if (Clipboard.ContainsText() && _parser.TryParse(Clipboard.GetText(), out var url, out _))
                _vm.Text = url;
        }
        catch { /* буфер занят другим процессом — не страшно */ }

        Show();
        Activate();
        Input.Focus();
        Input.SelectAll();
    }

    public void ToggleCapture()
    {
        if (IsVisible) Hide(); else ShowCapture();
    }

    private void OnDeactivated(object? sender, EventArgs e) => Hide();

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

            case Key.Enter:
                var text = _vm.Text.Trim();
                if (text.Length > 0) _main.AddFromCapture(text, _vm.Priority);
                Hide();
                e.Handled = true;
                break;

            case Key.D1 or Key.NumPad1 when _vm.DigitsSetPriority:
                _vm.Priority = TicketPriority.Low;
                e.Handled = true;
                break;
            case Key.D2 or Key.NumPad2 when _vm.DigitsSetPriority:
                _vm.Priority = TicketPriority.Mid;
                e.Handled = true;
                break;
            case Key.D3 or Key.NumPad3 when _vm.DigitsSetPriority:
                _vm.Priority = TicketPriority.High;
                e.Handled = true;
                break;
        }
    }
}
