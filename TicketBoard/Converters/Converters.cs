using System.Globalization;
using System.Windows;
using System.Windows.Data;
using TicketBoard.Models;

namespace TicketBoard.Converters;

public sealed class NullToVisibilityConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c) => value is null ? Visibility.Collapsed : Visibility.Visible;
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}

public sealed class InverseBoolToVisibilityConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c) => value is true ? Visibility.Collapsed : Visibility.Visible;
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}

/// <summary>Пустая строка → Collapsed.</summary>
public sealed class StringToVisibilityConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c) => string.IsNullOrEmpty(value as string) ? Visibility.Collapsed : Visibility.Visible;
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}

/// <summary>true → Opacity из параметра (например 0.35), иначе 1.</summary>
public sealed class BoolToOpacityConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c)
        => value is true && double.TryParse(p?.ToString(), NumberStyles.Float, CultureInfo.InvariantCulture, out var o) ? o : 1.0;
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}

/// <summary>Enum ↔ bool для RadioButton: IsChecked = (value == parameter).</summary>
public sealed class EnumEqualsConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c)
        => value is not null && p is string s && string.Equals(value.ToString(), s, StringComparison.OrdinalIgnoreCase);
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c)
        => value is true && p is string s ? Enum.Parse(t, s, ignoreCase: true) : Binding.DoNothing;
}

public sealed class PriorityToTextConverter : IValueConverter
{
    /// <summary>Название приоритета — одно на карточку, панель и мост для Claude.</summary>
    public static string Text(TicketPriority priority) => priority switch
    {
        TicketPriority.Low => "Низкий",
        TicketPriority.Mid => "Средний",
        TicketPriority.High => "Высокий",
        _ => ""
    };

    public object Convert(object? value, Type t, object? p, CultureInfo c) => value is TicketPriority priority ? Text(priority) : "";
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}

public sealed class StatusToTextConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c) => value switch
    {
        TicketStatus.Inbox => "Входящие",
        TicketStatus.InProgress => "В работе",
        TicketStatus.Waiting => "Ждёт ответа",
        TicketStatus.Done => "Готово",
        _ => ""
    };
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}

/// <summary>AgeState → кисть текста «В колонке» в панели.</summary>
public sealed class AgeStateToBrushConverter : IValueConverter
{
    public object Convert(object? value, Type t, object? p, CultureInfo c)
    {
        var key = value switch
        {
            AgeState.Overdue => "CriticalFg",
            AgeState.Warn => "CautionFg",
            _ => "TextPrimary"
        };
        return Application.Current.TryFindResource(key) ?? DependencyProperty.UnsetValue;
    }
    public object ConvertBack(object? value, Type t, object? p, CultureInfo c) => throw new NotSupportedException();
}
