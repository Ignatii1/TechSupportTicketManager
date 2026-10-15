using System.Diagnostics;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Пароли в тексте заявки — скрыть: выгрузка уходит в GitHub (Obsidian Git), запрос агенту — в облако, а в
/// комментариях их пишут («Логин: …, Пароль: …»).</summary>
public static class Secrets
{
    public const string Hidden = "[скрыт]";

    /// <summary>Слово «пароль» (и как его пишут: password, pwd, pass, пин-код), через не больше 20 знаков без знаков
    /// препинания — «:» или «=», и следующее слово — оно и скрывается. Узко нарочно: «пароль сброшен, логин: ivanov» и
    /// «сменил пароль» — обычный текст. ponytail: «пароль 12345» без двоеточия не скрывается — шире шаблон испортил бы текст.</summary>
    private static readonly Regex Password = new(
        @"(?<key>\b(?:пароль|пароля|паролем|пароли|password|passwd|pwd|pass|пин-код|пин|pin)\b[^:=\n.,;!?]{0,20}?[:=][ \t]*)\S+",
        RegexOptions.IgnoreCase | RegexOptions.CultureInvariant | RegexOptions.Compiled);

    public static string Mask(string text) => Password.Replace(text, m => m.Groups["key"].Value + Hidden);

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // живой случай: учётки ТСД с паролями — пароль скрыт, логин на месте
        Debug.Assert(Mask("Логин: ТСД_Щербинка_3\r\nПароль: 17092026") == "Логин: ТСД_Щербинка_3\r\nПароль: [скрыт]", Mask("Логин: ТСД_Щербинка_3\r\nПароль: 17092026"));
        foreach (var (text, masked) in new[]
        {
            ("password=Qwerty123", "password=[скрыт]"),
            ("Пароль от wifi: guest2026 — до пятницы", "Пароль от wifi: [скрыт] — до пятницы"),
            ("ПИН-код: 1234", "ПИН-код: [скрыт]"),
            ("pwd: s3cr3t; логин admin", "pwd: [скрыт] логин admin"),
            ("PASS : x1", "PASS : [скрыт]"),
            ("пароля: abc", "пароля: [скрыт]"),
        })
            Debug.Assert(Mask(text) == masked, $"{text} → {Mask(text)}");
        // без двоеточия или знака равенства — обычный текст, не трогаем; «passport» — не «pass»
        foreach (var text in new[] { "пароль сброшен, логин: ivanov", "Сменил пароль, всё работает", "passport: 4510 123456", "пароль 12345", "" })
            Debug.Assert(Mask(text) == text, $"{text} → {Mask(text)}");
        // скрытое скрыть ещё раз — то же самое
        Debug.Assert(Mask(Mask("Пароль: 1")) == "Пароль: [скрыт]");
    }
}
