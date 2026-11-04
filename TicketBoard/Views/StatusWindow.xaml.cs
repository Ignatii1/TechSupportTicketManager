using System.Windows;
using System.Windows.Input;
using TicketBoard.ViewModels;
using Wpf.Ui.Appearance;
using Wpf.Ui.Controls;

namespace TicketBoard.Views;

/// <summary>«Сменить статус заявки #N?» при переносе карточки руками (этап 8, шаг 2). Всё, что окно решает, — в StatusForm;
/// здесь только кнопки и Esc.</summary>
public partial class StatusWindow : FluentWindow
{
    private readonly StatusForm _form;

    private StatusWindow(StatusForm form)
    {
        _form = form;
        DataContext = form;
        InitializeComponent();
        SystemThemeWatcher.Watch(this);
        // как у AskWindow: над окном, из которого перенесли; доска скрыта — по центру экрана и с кнопкой на панели задач
        if (AskWindow.OwnerCandidate() is { } owner) Owner = owner;
        else { WindowStartupLocation = WindowStartupLocation.CenterScreen; ShowInTaskbar = true; }
        Loaded += (_, _) => CommentBox.Focus();
    }

    /// <summary>true — «Сменить статус»; «Только на доске», Esc и закрытие окна — false. Задача — чтобы самопроверка могла
    /// держать окно «открытым»: ShowDialog и так ждёт, пока его закроют.</summary>
    public static Task<bool> AskAsync(StatusForm form) => Task.FromResult(new StatusWindow(form).ShowDialog() == true);

    private void OnApply(object sender, RoutedEventArgs e)
    {
        if (_form.CanApply) DialogResult = true;
    }

    private void OnBoardOnly(object sender, RoutedEventArgs e) => DialogResult = false;

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Escape || TargetBox.IsDropDownOpen) return;   // Esc в открытом списке закрывает список, не окно
        Close();   // DialogResult остаётся null — «Только на доске»
        e.Handled = true;
    }
}
