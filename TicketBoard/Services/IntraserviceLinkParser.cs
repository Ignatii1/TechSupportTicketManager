using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Находит в произвольном тексте ссылку и/или номер заявки (по регулярке из настроек).</summary>
public sealed class IntraserviceLinkParser(AppSettings settings)
{
    private static readonly Regex UrlRegex = new(@"https?://\S+", RegexOptions.Compiled | RegexOptions.IgnoreCase);
    private static readonly Regex LastNumber = new(@"(\d{4,8})(?!.*\d)", RegexOptions.Compiled);

    /// <summary>Есть ли в тексте ссылка (url) и удалось ли распознать номер (id). Номер может быть и без ссылки: «#702180».</summary>
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
        return url.Length > 0;
    }
}
