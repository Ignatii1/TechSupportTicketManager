using System.Diagnostics;

namespace TicketBoard.Services;

/// <summary>Запрос агенту над базой знаний — «Спросить базу знаний» в панели: номер, название, ссылка, где заявка в базе
/// (или будет), задача и полный текст заявки в формате базы. Как искать — в CLAUDE.md базы, не здесь.</summary>
public static class KnowledgeAsk
{
    /// <summary>Сколько знаков текста заявки кладётся в запрос: больше в чат не влезет вместе с ответом.</summary>
    public const int MaxTicketChars = 60_000;

    /// <summary>Запрос целиком. file — путь файла заявки от корня базы («tickets/2026-10/…md»); inBase — он уже есть (иначе —
    /// куда его положит выгрузка). ticket — текст заявки в формате базы; длинный обрезается (Cut).</summary>
    public static string Request(int id, string title, string url, string file, bool inBase, string ticket)
    {
        var where = inBase ? $"В базе знаний: {file}" : $"В базе знаний её ещё нет (будет: {file}) — полный текст ниже.";
        return $"Заявка #{id} — {title}\n"
            + (url.Length > 0 ? $"Ссылка: {url}\n" : "")
            + where + "\n\n"
            + "Найди в базе знаний похожие решённые заявки (как искать — в CLAUDE.md базы; нет его — в папке tickets/ по сервису, "
            + "типу, словам названия и кодам ошибок) и ответь:\n"
            + "1. Похожие заявки, 3–5 лучших: номер, ссылка, чем похожа.\n"
            + "2. Как их решали — по шагам.\n"
            + "3. Что сделать с этой заявкой.\n"
            + "4. Черновик ответа пользователю.\n"
            + "Похожих нет — так и скажи, не придумывай.\n\n"
            + "Текст заявки в формате базы:\n\n"
            + Cut(ticket, MaxTicketChars);
    }

    /// <summary>Не длиннее max: середина вырезается по границам строк — начало (карточка и описание) и конец (свежие записи)
    /// важнее; на месте выреза — сколько вырезано.</summary>
    public static string Cut(string text, int max)
    {
        if (text.Length <= max) return text;
        var head = text.LastIndexOf('\n', max * 2 / 3) is var h && h > 0 ? h + 1 : max * 2 / 3;
        var tail = text.IndexOf('\n', text.Length - (max - head)) is var t && t >= 0 && t + 1 < text.Length ? t + 1 : text.Length - (max - head);
        return text[..head] + $"\n… [вырезано {tail - head} знаков из середины — заявка целиком в файле базы и по ссылке] …\n\n" + text[tail..];
    }

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // эталон: маленькая заявка, ещё не в базе
        var request = Request(712182, "Не проводится реализация", "https://helpdesk.example/Task/View/712182",
            "tickets/2026-10/712182 — Не проводится реализация.md", inBase: false, "---\nid: 712182\n---\n\n# Не проводится реализация\n\nпрошу помочь");
        Debug.Assert(request == """
            Заявка #712182 — Не проводится реализация
            Ссылка: https://helpdesk.example/Task/View/712182
            В базе знаний её ещё нет (будет: tickets/2026-10/712182 — Не проводится реализация.md) — полный текст ниже.

            Найди в базе знаний похожие решённые заявки (как искать — в CLAUDE.md базы; нет его — в папке tickets/ по сервису, типу, словам названия и кодам ошибок) и ответь:
            1. Похожие заявки, 3–5 лучших: номер, ссылка, чем похожа.
            2. Как их решали — по шагам.
            3. Что сделать с этой заявкой.
            4. Черновик ответа пользователю.
            Похожих нет — так и скажи, не придумывай.

            Текст заявки в формате базы:

            ---
            id: 712182
            ---

            # Не проводится реализация

            прошу помочь
            """, request);
        // уже в базе — её путь; ссылки нет — и строки нет
        var known = Request(5, "Принтер", "", "tickets/2026-09/5 — Принтер.md", inBase: true, "текст");
        Debug.Assert(known.StartsWith("Заявка #5 — Принтер\nВ базе знаний: tickets/2026-09/5 — Принтер.md\n\n", StringComparison.Ordinal), known);
        // длинная — обрезана посередине: начало (описание) и конец (свежие записи) целы, пометка — сколько вырезано
        var lines = string.Concat(Enumerable.Range(0, 300).Select(i => $"строка {i:D3} " + new string('x', 90) + "\n"));
        var cut = Cut(lines, 10_000);
        var mark = System.Text.RegularExpressions.Regex.Match(cut, @"\n… \[вырезано (\d+) знаков из середины[^\]]*\] …\n\n");
        var removed = mark.Success ? int.Parse(mark.Groups[1].Value) : -1;
        Debug.Assert(cut.StartsWith("строка 000 ", StringComparison.Ordinal) && cut.EndsWith("строка 299 " + new string('x', 90) + "\n", StringComparison.Ordinal)
            && cut.Length <= 10_000 + mark.Length && cut.Length - mark.Length + removed == lines.Length, cut.Length + " / " + removed);
        // режется по строкам: каждая оставленная — целая
        Debug.Assert(cut.Replace(mark.Value, "\n").Split('\n').All(l => l.Length == 0 || l.StartsWith("строка ", StringComparison.Ordinal) && l.Length == 101),
            "режется по строкам");
        Debug.Assert(Cut("коротко", 10_000) == "коротко");
        // в запросе текст — уже обрезанный
        Debug.Assert(Request(1, "t", "", "f", false, lines + lines + lines + lines + lines + lines + lines + lines).Length < MaxTicketChars + 2_000);
    }
}
