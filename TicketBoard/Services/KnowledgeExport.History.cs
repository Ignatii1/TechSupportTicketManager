using System.Diagnostics;

namespace TicketBoard.Services;

/// <summary>Что выгрузка знает о человеке и сервере, один раз на выгрузку: кто «я» (Me; null — не узнали: роль не пишется)
/// и какие названия статусов — закрытые сверх признаков сервера (ClosedNames — ClosedStatusNames из настроек); чьи вложения
/// качать (Attachments — настройка «Вложения в базу знаний»; не сказано — никаких).</summary>
public sealed record ExportContext(IntraserviceUser? Me, IReadOnlySet<string> ClosedNames, AttachmentScope Attachments = AttachmentScope.None)
{
    /// <summary>Ничего не знаем: роль пустая, закрытые — только по признакам сервера.</summary>
    public static readonly ExportContext Unknown = new(null, new HashSet<string>());
}

/// <summary>Что выгрузка говорит о заявке по её истории: кто участвовал, моя роль, как быстро отреагировали и решили, сколько
/// раз переоткрывали, сколько комментариев и чем закрыли. Только над уже прочитанными записями — без запросов.</summary>
public static class TicketHistory
{
    /// <summary>Participants — кто писал и менял статус, по числу таких действий (при равном — кто раньше); MyRole —
    /// «исполнитель», «инициатор», «участник» или «»; часы — null, когда нет одной из дат (не было реакции, нет даты решения
    /// и закрытия — как у разобранных полей ответа); ClosingComment — «» у открытой и когда закрывающего нет.</summary>
    public sealed record Facts(IReadOnlyList<string> Participants, string MyRole, double? FirstReactionHours, double? ResolveHours,
        int Reopened, int PublicComments, int InternalComments, string ClosingComment);

    /// <summary>Сколько знаков закрывающего комментария в свойствах и таблице: начало — обычно в нём и решение.</summary>
    public const int ClosingCommentChars = 300;

    public static Facts Of(IntraserviceFound f, IReadOnlyList<IntraserviceEvent> events, ExportContext context)
    {
        // по времени; при равном — как шло на сервере (присылает он свежие сверху — разворачиваем, как и файл заявки)
        var ordered = Enumerable.Reverse(events).OrderBy(e => e.Date ?? DateTimeOffset.MinValue).ToList();
        var actions = new List<(string Author, int At)>();
        string? previous = null;
        bool? closed = null;
        int reopened = 0, lastClosing = -1;
        for (var i = 0; i < ordered.Count; i++)
        {
            var e = ordered[i];
            var statusChanged = e.Status.Length > 0 && e.Status != previous;
            if (!string.IsNullOrWhiteSpace(e.Comment) || statusChanged) actions.Add((e.Author.Trim(), i));
            if (e.Status.Length == 0) continue;
            previous = e.Status;
            // закрытый — тем же правилом, что импорт, F5 и автовыгрузка: признак сервера или название из настроек
            var closedNow = e.Final || context.ClosedNames.Contains(e.Status);
            if (closed == true && !closedNow) reopened++;
            if (closedNow && closed != true) lastClosing = i;
            closed = closedNow;
        }
        var participants = actions.Where(a => a.Author.Length > 0).GroupBy(a => a.Author)
            .OrderByDescending(g => g.Count()).ThenBy(g => g.Min(a => a.At)).Select(g => g.Key).ToList();

        // инициатор записи — по номеру, без номеров — по имени
        bool ByInitiator(IntraserviceEvent e) => f.CreatorId is { } creator && e.AuthorId is { } author
            ? creator == author : f.Creator is { Length: > 0 } name && AutoSyncRules.SameName(e.Author, name);
        // реакция — первая запись не инициатора после самой первой: та — регистрация, её делает и диспетчер
        var reaction = ordered.Skip(1).FirstOrDefault(e => !ByInitiator(e));
        string? closing = null;
        if (closed == true)
            for (var i = lastClosing; i >= 0 && closing is null; i--)
                if (ordered[i] is { Comment: { } text, IsPublic: not false } e && !string.IsNullOrWhiteSpace(text) && !ByInitiator(e))
                    // Format уже скрыл пароли в тексте, но одна строка из нескольких может сложить новое «пароль: x» —
                    // скрываем ещё раз (лишнее скрытое лучше утёкшего)
                    closing = Begin(Secrets.Mask(KnowledgeExport.OneLine(text)));
        return new(participants, RoleOf(f, ordered, context), Hours(f.Created, reaction?.Date),
            Hours(f.Created, f.Extra?.Resolved ?? f.Extra?.Closed), reopened,
            ordered.Count(e => !string.IsNullOrWhiteSpace(e.Comment) && e.IsPublic != false),
            ordered.Count(e => !string.IsNullOrWhiteSpace(e.Comment) && e.IsPublic == false), closing ?? "");
    }

    /// <summary>Моя роль — первая подходящая: исполнитель, инициатор, участник (моё действие в истории). Кто я — не знаем: «».</summary>
    private static string RoleOf(IntraserviceFound f, IReadOnlyList<IntraserviceEvent> events, ExportContext context) =>
        context.Me is not { } me ? "" : new Myself(me) is var i && i.Executes(f) ? "исполнитель" : i.Initiated(f) ? "инициатор"
            : events.Any(i.Wrote) ? "участник" : "";

    /// <summary>Часы между датами — с одной цифрой после запятой; какой-то даты нет — null.</summary>
    private static double? Hours(DateTimeOffset? from, DateTimeOffset? to) =>
        from is { } a && to is { } b ? Math.Round((b - a).TotalHours, 1, MidpointRounding.AwayFromZero) : null;

    /// <summary>Начало текста — до ClosingCommentChars знаков (суррогатная пара не рвётся), обрезанное — с «…».</summary>
    private static string Begin(string text) => text.Length <= ClosingCommentChars ? text
        : text[..(char.IsHighSurrogate(text[ClosingCommentChars - 1]) ? ClosingCommentChars - 1 : ClosingCommentChars)] + "…";

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        static DateTimeOffset At(int day, int hour, int minute, int second = 0) => new(2026, 10, day, hour, minute, second, TimeSpan.FromHours(3));
        static IntraserviceEvent E(DateTimeOffset at, string author, int id, string status = "", string? comment = null, bool? isPublic = null,
            bool final = false) => new(at, author, status, comment, isPublic, id, final);
        // по живой истории 715007, обезличенной: регистрация диспетчером, передача, ответы, закрытие, переоткрытие, закрытие
        // статусом, закрытым только по названию из настроек
        var row = new IntraserviceFound(715007, "Настроить ТСД", "Закрыта по названию", "Петров Пётр", At(14, 11, 2),
            Executors: "Иванов Иван, Сидоров Сидор", Extra: new("Склад", "Запрос", null, At(15, 9, 2), Closed: At(15, 9, 14)),
            CreatorId: 100, ExecutorIds: new[] { 200, 300 });
        var history = new List<IntraserviceEvent>
        {
            E(At(15, 9, 14), "Сидоров Сидор", 300, "Закрыта по названию"),
            E(At(15, 9, 5), "Петров Пётр", 100, comment: "Спасибо, работает", isPublic: true),          // инициатор — не закрывающий
            E(At(15, 9, 0), "Сидоров Сидор", 300, comment: "Проверить завтра", isPublic: false),          // внутренний — тоже
            E(At(15, 8, 50), "Сидоров Сидор", 300, comment: "Перенастроил принтер", isPublic: true),
            E(At(14, 17, 0), "Петров Пётр", 100, "Открыта", "Не работает", true),
            E(At(14, 16, 0), "Иванов Иван", 200, "Выполнена", "Настроил ТСД, логин: ТСД_3", true, final: true),
            E(At(14, 15, 22), "Сидоров Сидор", 300, comment: "Внутренняя заметка", isPublic: false),
            E(At(14, 15, 13), "Петров Пётр", 100, comment: "Спасибо", isPublic: true),
            E(At(14, 12, 6, 7), "Иванов Иван", 200, "Ожидание ответа", "Давай попробуем как в прошлый раз", true),
            E(At(14, 11, 40), "Диспетчерова Дарья", 600, "Передана исполнителю"),   // статус тот же (сменила исполнителя) — не действие
            E(At(14, 11, 32), "Диспетчерова Дарья", 600, "Передана исполнителю"),
            E(At(14, 11, 2), "Диспетчер Денис", 500, "Открыта"),
        };   // как присылает сервер — свежие сверху
        var closedNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "Закрыта по названию" };
        ExportContext As(int id, string name) => new(new IntraserviceUser(id, name), closedNames);
        var facts = Of(row, history, As(200, "Иванов Иван"));
        // участники — по числу действий (комментарий или смена статуса), при равном — кто раньше
        Debug.Assert(facts.Participants.SequenceEqual(new[] { "Сидоров Сидор", "Петров Пётр", "Иванов Иван", "Диспетчер Денис", "Диспетчерова Дарья" }),
            string.Join(", ", facts.Participants));
        // роль: исполнитель раньше инициатора, инициатор раньше участника; не моя или не знаем, кто я, — пусто
        Debug.Assert(facts.MyRole == "исполнитель" && Of(row, history, As(100, "Петров Пётр")).MyRole == "инициатор"
            && Of(row, history, As(500, "Диспетчер Денис")).MyRole == "участник" && Of(row, history, As(999, "Чужой")).MyRole == ""
            && Of(row, history, new ExportContext(null, closedNames)).MyRole == ""
            && Of(row with { CreatorId = 200 }, history, As(200, "Иванов Иван")).MyRole == "исполнитель", facts.MyRole);   // и то и другое
        // без номеров в строке — по именам
        var named = row with { CreatorId = null, ExecutorIds = null };
        Debug.Assert(Of(named, history, As(0, "Сидоров Сидор")).MyRole == "исполнитель" && Of(named, history, As(0, "петров  пётр")).MyRole == "инициатор");
        // первая реакция — не регистрация (её делает и диспетчер) и не инициатор: передача через полчаса
        Debug.Assert(facts.FirstReactionHours == 0.5, $"{facts.FirstReactionHours}");
        var byCreator = history.Select(e => e.Author == "Диспетчерова Дарья" ? e with { Author = "Петров Пётр", AuthorId = 100 } : e).ToList();
        Debug.Assert(Of(row, byCreator, As(200, "Иванов Иван")).FirstReactionHours == 1.1
            && Of(named, byCreator, As(200, "Иванов Иван")).FirstReactionHours == 1.1, "реакция инициатора (по номеру и по имени) — не реакция");
        // решено через 22 часа (дата решения), нет её — до закрытия
        Debug.Assert(facts.ResolveHours == 22.0 && Of(row with { Extra = row.Extra! with { Resolved = null } }, history, As(200, "Иванов Иван")).ResolveHours == 22.2,
            $"{facts.ResolveHours}");
        // переоткрыли раз: «Выполнена» (признак сервера) → «Открыта»; закрыли статусом, закрытым по названию из настроек
        Debug.Assert(facts.Reopened == 1, $"{facts.Reopened}");
        Debug.Assert(facts.PublicComments == 6 && facts.InternalComments == 2, $"{facts.PublicComments} / {facts.InternalComments}");
        // закрывающий — последний публичный не инициатора до последнего закрытия
        Debug.Assert(facts.ClosingComment == "Перенастроил принтер", facts.ClosingComment);
        // без названия из настроек последний статус не закрытый: заявка открыта — закрывающего нет, а переоткрытие то же
        var serverOnly = Of(row, history, new ExportContext(new IntraserviceUser(200, "Иванов Иван"), new HashSet<string>()));
        Debug.Assert(serverOnly.ClosingComment == "" && serverOnly.Reopened == 1, serverOnly.ClosingComment);
        // закрывающий — одной строкой, длинный обрезан
        var longText = string.Join("\r\n", Enumerable.Range(0, 60).Select(i => $"шаг {i:D2} сделан"));
        var longClosing = Of(row, history.Select(e => e.Comment == "Перенастроил принтер" ? e with { Comment = longText } : e).ToList(), As(200, "Иванов Иван"));
        Debug.Assert(longClosing.ClosingComment is { Length: ClosingCommentChars + 1 } c && c.EndsWith('…') && !c.Contains('\n') && c.StartsWith("шаг 00 сделан шаг 01"),
            longClosing.ClosingComment);
        // обрезка не рвёт суррогатную пару: знак за границей уходит целиком
        var smile = char.ConvertFromUtf32(0x1F600);
        Debug.Assert(Begin(new string('я', ClosingCommentChars - 1) + smile + "хвост") == new string('я', ClosingCommentChars - 1) + "…");
        // истории нет — пусто, но не ошибка
        Debug.Assert(Of(row, Array.Empty<IntraserviceEvent>(), As(200, "Иванов Иван")) is { Participants.Count: 0, FirstReactionHours: null, Reopened: 0, ClosingComment: "" });
    }
}
