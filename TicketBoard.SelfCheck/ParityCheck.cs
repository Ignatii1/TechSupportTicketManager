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
        // текст → он же с паролями под «[скрыт]» (`Secrets.Mask`)
        ["export-mask"] = (1, c => Secrets.Mask(c[0])),
        // текст заявки или комментария → как он ляжет в Markdown (`KnowledgeExport.Escape`)
        ["export-escape"] = (1, c => KnowledgeExport.Escape(c[0])),
        // строка списка (ответ api/task, берётся первая; «-» — заявка известна только карточкой, `RowOf`), карточка (ответ
        // api/task/{id}), страница истории (ответ api/tasklifetime), я («номер|имя»; «-» — не узнали), закрытые названия
        // статусов через «|», урезана ли история (да/нет) → текст заявки для агента, как в файле базы знаний
        // (`Format(WithDetails(строка, карточка), …)`); выгружено — ExportedAt, ссылка — https://is.example/Task/View/{номер};
        // что-то не разобрано — «НЕ РАЗОБРАН»
        ["export-text"] = (6, c => ExportText(c, null)),
        // номер, название, сколько знаков названия можно → имя файла заявки (`KnowledgeExport.FileName`)
        ["export-names"] = (3, c => KnowledgeExport.FileName(int.Parse(c[0], CultureInfo.InvariantCulture), c[1],
            int.Parse(c[2], CultureInfo.InvariantCulture))),
        // папка tickets (строкой — важна только длина), строка списка (ответ api/task, первая) → «месяц|имя файла» (`PlaceOf`)
        ["export-place"] = (2, c => HttpIntraserviceClient.ParseSearch(c[1]) is { Found.Count: > 0 } r
            && KnowledgeExport.PlaceOf(c[0], r.Found[0]) is var (shard, rel) ? $"{shard}|{Path.GetFileName(rel)}" : "НЕ РАЗОБРАН"),
        // текст файла → его свойства строками «ключ=значение» в порядке файла (`ReadProps`, `Unquote`); не свойства — «НЕТ»
        ["export-props"] = (1, ExportProps),
        // файлы базы ({"месяц": {"имя.md": "текст"}}) → оглавления месяцев и общее после `WriteIndexes` (дата — «ДАТА»)
        ["export-index"] = (1, ExportIndex),
        // номер файла, имя, сколько знаков можно → имя вложения на диске (`Attachments.DiskName`)
        ["export-disk-names"] = (3, c => Attachments.DiskName(int.Parse(c[0], CultureInfo.InvariantCulture), c[1],
            int.Parse(c[2], CultureInfo.InvariantCulture))),
        // байты → размер для человека (`Attachments.Size`)
        ["export-size"] = (1, c => Attachments.Size(long.Parse(c[0], CultureInfo.InvariantCulture))),
        // как export-text, но шестая ячейка — что с вложениями карточки, по порядку, через «|»: «saved=путь=байты»
        // (лежит в базе), «skipped» (не скачано), «failed=пояснение» (не скачалось), пусто — только названы; история целая
        ["export-file-text"] = (6, c => ExportText(c, c[5])),
        // замеры «мс:заявок» через пробел, по порядку, одному `RateMeter` (RateWarmup — 5 с) → его ответы через пробел («R»)
        ["export-rate"] = (1, c =>
        {
            var meter = new KnowledgeExport.RateMeter();
            return string.Join(" ", c[0].Split(' ').Select(m => m.Split(':')).Select(p => meter.PerSecond(
                long.Parse(p[0], CultureInfo.InvariantCulture), int.Parse(p[1], CultureInfo.InvariantCulture))
                .ToString("R", CultureInfo.InvariantCulture)));
        }),
        // секунды → срок для человека (`HumanSpan(TimeSpan.FromSeconds(…))`)
        ["export-span"] = (1, c => KnowledgeExport.HumanSpan(TimeSpan.FromSeconds(double.Parse(c[0], CultureInfo.InvariantCulture)))),
        // легло, прошло, всего, не выгружено, скорость (заявок в секунду) → «по скольким спросили скорость|строка хода»
        // (`ProgressLine`, вложений 0; не спросили — «-»)
        ["export-progress"] = (5, c =>
        {
            int N(int i) => int.Parse(c[i], CultureInfo.InvariantCulture);
            var asked = "-";
            var line = KnowledgeExport.ProgressLine(N(0), N(1), N(2), N(3), 0, count =>
            {
                asked = count.ToString(CultureInfo.InvariantCulture);
                return double.Parse(c[4], CultureInfo.InvariantCulture);
            });
            return $"{asked}|{line}";
        }),
    };

    private static string ExportProps(string[] c)
    {
        var file = Path.GetTempFileName();
        try
        {
            File.WriteAllText(file, c[0], new UTF8Encoding(false));
            return KnowledgeExport.ReadProps(file) is { } props ? string.Join("\n", props.Select(kv => $"{kv.Key}={kv.Value}")) : "НЕТ";
        }
        finally { File.Delete(file); }
    }

    private static string ExportIndex(string[] c)
    {
        var dir = Directory.CreateTempSubdirectory("tb-parity-index-").FullName;
        try
        {
            using var doc = System.Text.Json.JsonDocument.Parse(c[0]);
            var months = new List<string>();
            var tickets = Path.Combine(dir, KnowledgeExport.TicketsFolder);
            foreach (var month in doc.RootElement.EnumerateObject())
            {
                months.Add(month.Name);
                Directory.CreateDirectory(Path.Combine(tickets, month.Name));
                foreach (var f in month.Value.EnumerateObject())
                    File.WriteAllText(Path.Combine(tickets, month.Name, f.Name), f.Value.GetString(), new UTF8Encoding(false));
            }
            // таблица и SCHEMA.md пишутся тут же — их сверяет своя таблица (4.3), здесь только оглавления
            KnowledgeExport.WriteIndexes(dir, months);
            var sb = new StringBuilder();
            foreach (var m in KnowledgeExport.InMonthOrder(months))
            {
                var index = Path.Combine(tickets, m, KnowledgeExport.IndexFile);
                sb.Append($"== {m}/{KnowledgeExport.IndexFile} ==\n").Append(File.Exists(index) ? File.ReadAllText(index) : "(нет)\n");
            }
            var root = Path.Combine(dir, KnowledgeExport.IndexFile);
            sb.Append($"== {KnowledgeExport.IndexFile} ==\n").Append(File.Exists(root) ? File.ReadAllText(root) : "(нет)\n");
            return System.Text.RegularExpressions.Regex.Replace(sb.ToString(), @"обновлено \d\d\.\d\d\.\d{4} \d\d:\d\d", "обновлено ДАТА");
        }
        finally { try { Directory.Delete(dir, true); } catch (IOException) { } }
    }

    /// <summary>Когда «выгружено» в таблице export-text: со смещением, чтобы ответ не зависел от пояса машины (даты сервера
    /// в таблице — тоже со смещением).</summary>
    private static readonly DateTimeOffset ExportedAt = new(2026, 11, 17, 10, 0, 0, TimeSpan.FromHours(3));

    private static string ExportText(string[] c, string? files)
    {
        // испорченный JSON — исключение, его Run пишет как «НЕ РАЗОБРАН»
        var card = HttpIntraserviceClient.Parse(c[1], 0);
        var life = HttpIntraserviceClient.ParseLifetime(c[2]);
        var list = c[0] == "-" ? null : HttpIntraserviceClient.ParseSearch(c[0]);
        if (card is null || life is null || (c[0] != "-" && list is not { Found.Count: > 0 })) return "НЕ РАЗОБРАН";
        var row = list is { } l ? l.Found[0] : KnowledgeExport.RowOf(card);
        var me = c[3] == "-" ? null : new IntraserviceUser(int.Parse(c[3].Split('|')[0], CultureInfo.InvariantCulture), c[3].Split('|', 2)[1]);
        var closed = c[4].Split('|', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var full = KnowledgeExport.WithDetails(row, card);
        var states = string.IsNullOrEmpty(files) ? [] : files.Split('|');
        var attachments = files is null ? null : (full.Extra?.Files ?? []).Select((file, i) => (i < states.Length ? states[i].Split('=') : []) switch
        {
            ["saved", var path, var size] => new Attachment(file, AttachmentState.Saved, path, long.Parse(size, CultureInfo.InvariantCulture)),
            ["skipped"] => new Attachment(file, AttachmentState.Skipped),
            ["failed", var note] => new Attachment(file, AttachmentState.Failed, Note: note),
            _ => new Attachment(file),
        }).ToList();
        return KnowledgeExport.Format(full, life.Value.Events.ToList(), $"https://is.example/Task/View/{card.Id}",
            ExportedAt, files is null && c[5] == "да", new ExportContext(me, closed), attachments);
    }

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
