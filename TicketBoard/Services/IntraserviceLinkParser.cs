using System.Diagnostics;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Находит в произвольном тексте ссылку и/или номер заявки (по регулярке из настроек).</summary>
public sealed class IntraserviceLinkParser(AppSettings settings)
{
    private static readonly Regex UrlRegex = new(@"https?://\S+", RegexOptions.Compiled | RegexOptions.IgnoreCase);
    private static readonly Regex LastNumber = new(@"(\d{4,8})(?!.*\d)", RegexOptions.Compiled);

    /// <summary>Возвращает true, если в тексте нашлась ссылка. Номер (id) может быть и без ссылки: «#702180» или просто «702180».
    /// Проверяйте именно возвращаемое значение, когда нужна ссылка: id без ссылки — обычное дело.</summary>
    public bool TryParse(string input, out string url, out int? id)
    {
        input ??= "";
        url = "";
        id = null;

        var m = UrlRegex.Match(input);
        if (m.Success) url = m.Value.TrimEnd('.', ',', ')', ';', '>');

        try
        {
            var mm = Regex.Match(input, settings.IntraserviceIdPattern, RegexOptions.IgnoreCase);
            if (mm.Success && int.TryParse(mm.Groups[1].Value, out var n)) id = n;
        }
        catch (ArgumentException) { /* кривая регулярка в настройках — ссылку всё равно берём */ }

        if (id is null && url.Length > 0)
        {
            var last = LastNumber.Match(url);
            if (last.Success && int.TryParse(last.Groups[1].Value, out var n)) id = n;
        }

        // В поле только номер — «702180». Проверяем всю строку целиком, иначе любое число
        // в обычном тексте («заменить картридж 12345») стало бы номером заявки.
        var bare = input.Trim();
        if (id is null && bare.Length is >= 4 and <= 8 && bare.All(char.IsAsciiDigit) && int.TryParse(bare, out var only))
            id = only;

        return url.Length > 0;
    }

    /// <summary>ponytail: самопроверка разбора, только в Debug (вызов в App.OnStartup). Регулярка берётся из настроек —
    /// здесь проверяется поведение с той, что стоит по умолчанию.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var p = new IntraserviceLinkParser(new AppSettings());
        // голый номер: ссылки нет, номер есть
        Debug.Assert(!p.TryParse("702180", out var u1, out var i1) && u1.Length == 0 && i1 == 702180);
        Debug.Assert(!p.TryParse("  702180  ", out _, out var i2) && i2 == 702180);
        // номер с решёткой и с текстом
        Debug.Assert(!p.TryParse("#702180", out _, out var i3) && i3 == 702180);
        Debug.Assert(!p.TryParse("#702180 не печатает", out _, out var i4) && i4 == 702180);
        // ссылка
        Debug.Assert(p.TryParse("https://help.local/Task/View/702180", out var u5, out var i5)
            && u5 == "https://help.local/Task/View/702180" && i5 == 702180);
        // число внутри текста номером не считается, короткое — тоже
        Debug.Assert(!p.TryParse("заменить картридж 12345", out _, out var i6) && i6 is null);
        Debug.Assert(!p.TryParse("123", out _, out var i7) && i7 is null);
    }
}
