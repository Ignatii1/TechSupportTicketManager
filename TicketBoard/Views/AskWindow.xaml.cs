using System.Windows;
using System.Windows.Input;
using Wpf.Ui.Appearance;
using Wpf.Ui.Controls;

namespace TicketBoard.Views;

/// <summary>Вопрос «да/нет» или сообщение в стиле приложения — вместо системного MessageBox, который выглядел чужим.
/// Enter — главная кнопка, Esc — отмена. Системный MessageBox остался там, где своё окно может не подняться:
/// нет доступа к папке при старте и необработанное исключение (App.xaml.cs).</summary>
public partial class AskWindow : FluentWindow
{
    private AskWindow(string heading, string text, string yes, string? no, bool danger)
    {
        InitializeComponent();
        SystemThemeWatcher.Watch(this);
        HeadingText.Text = heading;
        MessageText.Text = text;
        YesButton.Content = yes;
        if (danger) YesButton.Appearance = ControlAppearance.Danger;
        if (no is null) NoButton.Visibility = Visibility.Collapsed;
        else NoButton.Content = no;

        // над окном, из которого спросили; из трея (доска скрыта или свёрнута) — по центру экрана и с кнопкой на панели
        // задач, чтобы вопрос не потерялся за чужими окнами
        if (OwnerCandidate() is { } owner) Owner = owner;
        else { WindowStartupLocation = WindowStartupLocation.CenterScreen; ShowInTaskbar = true; }
        Loaded += (_, _) => YesButton.Focus();
    }

    /// <summary>true — нажата главная кнопка (или Enter); Esc, «нет» и закрытие окна — false.</summary>
    public static bool Ask(string heading, string text, string yes, string no = "Отмена", bool danger = false) =>
        new AskWindow(heading, text, yes, no, danger).ShowDialog() == true;

    /// <summary>Сообщение с одной кнопкой. Текст можно выделить и скопировать — в ошибках API там ответ сервера.</summary>
    public static void Tell(string heading, string text) => new AskWindow(heading, text, "OK", null, false).ShowDialog();

    private void OnYes(object sender, RoutedEventArgs e) => DialogResult = true;

    private void OnNo(object sender, RoutedEventArgs e) => DialogResult = false;

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape) return;
        Close();   // DialogResult остаётся null — для Ask это «нет»
        e.Handled = true;
    }

    /// <summary>Активное окно приложения, иначе доска. Не годятся скрытые и свёрнутые (свёрнутый владелец прячет вопрос
    /// вместе с собой, а ShowDialog при этом блокирует всё остальное) и быстрое добавление — оно прячется само, едва
    /// потеряет фокус. Подходящего нет — вопрос без владельца, по центру экрана и с кнопкой на панели задач.</summary>
    private static Window? OwnerCandidate()
    {
        var shown = Application.Current.Windows.OfType<Window>()
            .Where(w => w.IsVisible && w.WindowState != WindowState.Minimized && w is not AskWindow and not QuickCaptureWindow)
            .ToList();
        return shown.FirstOrDefault(w => w.IsActive) ?? shown.FirstOrDefault(w => w == Application.Current.MainWindow);
    }
}
