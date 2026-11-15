using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text;
using TicketBoard.Services;
using TicketBoard.ViewModels;

namespace TicketBoard.SelfCheck;

/// <summary>Таблицы сверки (<c>rust/tests/parity/*.tsv</c>): входы и ответ кода C# на них. По ним проверяется перенос на
/// Rust (<c>rust/tests/common</c> → <c>parity_mismatches</c>), а здесь каждая таблица сверяется с самим кодом C# — так
/// образец не может разойтись с приложением, и ожидаемые значения никто не выводит чтением кода. Строка — ячейки через
/// табуляцию, последняя — ответ; <c>#</c> в начале — пояснение. В ячейке всё, чего не видно или нет на клавиатуре, —
/// знаками: обратная черта удвоена, табуляция и переводы строк — чертой с буквой, остальное — чертой, «u» и четырьмя
/// шестнадцатеричными цифрами (и пробел на краю ячейки: редактор срезал бы его; и «#» в её начале). <c>-- parity --write</c> переписывает
/// ответы и приводит ячейки к этому виду — после правки входов или самих правил.</summary>
internal static class ParityCheck
{
    /// <summary>Таблица → сколько в строке входов и ответ кода C# на них (входы уже без экранирования).</summary>
    private static readonly Dictionary<string, (int Inputs, Func<string[], string> Answer)> Tables = new()
    {
        // поля «с» и «по» формы поиска и подпись поля → период или текст ошибки
        ["search-dates"] = (3, c =>
        {
            var (span, error) = TicketSearch.ParseSpan(c[0], c[1], c[2]);
            return error != "" ? "ERR " + error : $"OK {Moment(span.From)} {Moment(span.To)}";
        }),
        // справочник сервисов (ответ api/service) и номер → номера «с вложенными» через запятую (`ServiceWithChildren`);
        // ответ не разобран — «НЕ РАЗОБРАН»
        ["search-service-children"] = (2, c => HttpIntraserviceClient.ParseRefs(c[0], "Services", "ServiceList") is { } r
            ? string.Join(",", TicketSearch.ServiceWithChildren(r.Items, int.Parse(c[1], CultureInfo.InvariantCulture)))
            : "НЕ РАЗОБРАН"),
        // справочник сервисов и «полный ли он» (да/нет) → подписи списка выбора через « ‖ » (`ServiceLabel`; неполный — без
        // отступов, путь отброшен, как в `SearchViewModel.FillChoices`)
        ["search-service-labels"] = (2, c => HttpIntraserviceClient.ParseRefs(c[0], "Services", "ServiceList") is { } r
            ? string.Join(" ‖ ", r.Items.Select(s => TicketSearch.ServiceLabel(c[1] == "нет" ? s with { Path = null } : s)))
            : "НЕ РАЗОБРАН"),
        // страница списка заявок (ответ api/task) → подписи строк окна поиска: «сервис · тип | автор · исполнители · даты»,
        // строки через « ‖ »; ответ не разобран — «НЕ РАЗОБРАН»
        ["search-rows"] = (1, c => HttpIntraserviceClient.ParseSearch(c[0]) is { } r
            ? string.Join(" ‖ ", r.Found.Select(f => new FoundTicketViewModel(f, "")).Select(v => $"{v.Service} | {v.Details}"))
            : "НЕ РАЗОБРАН"),
    };

    private static string Moment(DateTime? d) => d?.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture) ?? "-";

    public static void Run(bool write, [CallerFilePath] string here = "")
    {
        var dir = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(here)!, "..", "rust", "tests", "parity"));
        var check = new CheckSet("parity");

        // сам вид ячеек: туда и обратно, и что испорченная ячейка — ошибка, а не молчаливый пропуск
        var odd = "a\\b\tc\nd\re " + (char)0xA0 + (char)0xFF10 + "«я»—";
        check.Check("ячейка: туда и обратно", Unescape(Escape(odd)) == odd, () => Escape(odd));
        check.Check("ячейка: вид", Escape(" x ") == "\\u0020x\\u0020" && Escape("\\\t\n\r") == "\\\\\\t\\n\\r" && Escape("" + (char)0xA0) == "\\u00A0" && Escape("") == "");
        check.Check("ячейка: «#» в начале — знаком, иначе строка сошла бы за пояснение", Escape("#x#") == "\\u0023x#" && Unescape(Escape("#x#")) == "#x#");
        check.Check("ячейка: незнакомый знак после черты — ошибка", Throws(() => Unescape("\\x")) && Throws(() => Unescape("\\u12")) && Throws(() => Unescape("\\")));

        var files = Directory.Exists(dir) ? Directory.GetFiles(dir, "*.tsv").OrderBy(f => f, StringComparer.Ordinal).ToList() : [];
        foreach (var name in Tables.Keys)
            check.Check($"таблица {name}.tsv есть", files.Any(f => Path.GetFileNameWithoutExtension(f) == name), () => dir);
        foreach (var file in files)
        {
            var name = Path.GetFileNameWithoutExtension(file);
            if (!Tables.TryGetValue(name, out var table))
            {
                check.Check($"{name}.tsv: код C# для таблицы назван в ParityCheck.Tables", false);
                continue;
            }
            var lines = File.ReadAllText(file, Encoding.UTF8).Split('\n');
            var rows = 0;
            var changed = 0;
            for (var i = 0; i < lines.Length; i++)
            {
                var line = lines[i];
                if (line.Length == 0 || line.StartsWith('#')) continue;
                rows++;
                var cells = line.Split('\t');
                string where = $"{name}.tsv, строка {i + 1}";
                // при записи ответа может ещё не быть; при сверке строка обязана быть полной
                if (cells.Length != table.Inputs + 1 && !(write && cells.Length == table.Inputs))
                {
                    check.Check($"{where}: ячеек {table.Inputs} и ответ", false, () => line);
                    continue;
                }
                string[] inputs;
                try { inputs = cells.Take(table.Inputs).Select(Unescape).ToArray(); }
                catch (FormatException e)
                {
                    check.Check($"{where}: ячейка читается", false, () => e.Message);
                    continue;
                }
                // ответ, который сервер прислал не json-ом, клиент C# ловит (JsonException → «ответ не разобран»): так и здесь
                string answer;
                try { answer = table.Answer(inputs); }
                catch (System.Text.Json.JsonException) { answer = "НЕ РАЗОБРАН"; }
                var right = string.Join('\t', inputs.Select(Escape).Append(Escape(answer)));
                if (right == line) continue;
                changed++;
                if (write) lines[i] = right;
                else check.Check($"{where}: совпадает с ответом кода C#", false, () => $"в таблице: {line}\nкод C#:   {right}\n(переписать ответы: dotnet run --project TicketBoard.SelfCheck -- parity --write)");
            }
            check.Check($"{name}.tsv: строки есть", rows > 0);
            if (write)
            {
                if (changed > 0) File.WriteAllText(file, string.Join('\n', lines), new UTF8Encoding(false));
                Console.WriteLine($"  parity: {name}.tsv — строк {rows}, переписано {changed}");
            }
        }
        check.AssertAll();
    }

    private static bool Throws(Action act)
    {
        try { act(); }
        catch (FormatException) { return true; }
        return false;
    }

    /// <summary>Что в ячейке пишется как есть: печатные знаки ASCII, русские буквы и знаки русского текста.</summary>
    private static bool Plain(char c) => c is >= ' ' and <= '~' or >= 'А' and <= 'я' or 'Ё' or 'ё' or '«' or '»' or '—' or '–' or '…' or '№';

    internal static string Escape(string text)
    {
        var s = new StringBuilder();
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            if (c == '\\') s.Append("\\\\");
            else if (c == '\t') s.Append("\\t");
            else if (c == '\n') s.Append("\\n");
            else if (c == '\r') s.Append("\\r");
            // «#» в начале ячейки: с неё строка читалась бы как пояснение
            else if (!Plain(c) || c == ' ' && (i == 0 || i == text.Length - 1) || c == '#' && i == 0) s.Append("\\u").Append(((int)c).ToString("X4", CultureInfo.InvariantCulture));
            else s.Append(c);
        }
        return s.ToString();
    }

    internal static string Unescape(string cell)
    {
        var s = new StringBuilder();
        for (var i = 0; i < cell.Length; i++)
        {
            if (cell[i] != '\\')
            {
                s.Append(cell[i]);
                continue;
            }
            var next = ++i < cell.Length ? cell[i] : '\0';
            if (next == '\\') s.Append('\\');
            else if (next == 't') s.Append('\t');
            else if (next == 'n') s.Append('\n');
            else if (next == 'r') s.Append('\r');
            else if (next == 'u' && i + 4 < cell.Length && ushort.TryParse(cell.AsSpan(i + 1, 4), NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var code))
            {
                s.Append((char)code);
                i += 4;
            }
            else throw new FormatException($"после обратной черты — не \\, t, n, r и не u с четырьмя цифрами: «{cell}»");
        }
        return s.ToString();
    }
}
