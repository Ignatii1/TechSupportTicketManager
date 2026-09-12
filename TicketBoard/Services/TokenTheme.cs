using System.Windows;
using System.Windows.Media;
using Wpf.Ui.Appearance;

namespace TicketBoard.Services;

/// <summary>Подключает словарь токенов (Themes/Tokens.*.xaml) под текущую тему и подменяет акцент системным.</summary>
public static class TokenTheme
{
    private static ResourceDictionary? _current;

    public static void Apply(ApplicationTheme theme)
    {
        var dark = theme == ApplicationTheme.Dark;
        var dict = new ResourceDictionary
        {
            Source = new Uri(dark ? "pack://application:,,,/Themes/Tokens.Dark.xaml" : "pack://application:,,,/Themes/Tokens.Light.xaml", UriKind.Absolute)
        };

        // Акцент — системный (WPF-UI уже подобрал вариант под тему), токены #0067C0 / #4CC2FF — только запасные.
        var accent = ApplicationAccentColorManager.PrimaryAccent;
        dict["Accent"] = Freeze(new SolidColorBrush(accent));
        dict["AccentHover"] = Freeze(new SolidColorBrush(ApplicationAccentColorManager.SecondaryAccent));
        dict["AccentPressed"] = Freeze(new SolidColorBrush(ApplicationAccentColorManager.TertiaryAccent));
        dict["AccentTint"] = Freeze(new SolidColorBrush(Color.FromArgb(dark ? (byte)36 : (byte)26, accent.R, accent.G, accent.B)));

        var merged = Application.Current.Resources.MergedDictionaries;
        if (_current is not null) merged.Remove(_current);
        merged.Add(dict);
        _current = dict;
    }

    private static Brush Freeze(Brush b) { b.Freeze(); return b; }
}
