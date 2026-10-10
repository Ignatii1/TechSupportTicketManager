using System.Diagnostics;

namespace TicketBoard.Services;

/// <summary>Отметка автовыгрузки: закрытые заявки, изменённые до Since, уже разобраны; Seen — номера разобранных в минуту
/// отметки (сервер сравнивает даты до минуты, и заявки этой минуты придут снова). Хранится в settings.json.</summary>
public sealed record ExportMark(DateTimeOffset Since, IReadOnlyList<int> Seen)
{
    /// <summary>Первое включение: неделя назад — то, что закрыли, пока выгрузки не было; старше — окно «Поиск заявок».</summary>
    public static ExportMark Start(DateTimeOffset now) => new(now.AddDays(-7), Array.Empty<int>());

    /// <summary>Строка уже разобрана: номер — среди разобранных минуты отметки, а изменение не позже отметки (изменилась ещё
    /// раз — разбирается заново).</summary>
    public bool Covers(IntraserviceFound row) => row.Changed is { } changed && changed <= Since && Seen.Contains(row.Id);

    /// <summary>Отметка после строк, разобранных подряд по порядку списка: изменение последней и номера её минуты (минута
    /// та же, что у прежней отметки, — и прежние номера).</summary>
    public ExportMark After(IReadOnlyList<IntraserviceFound> done)
    {
        if (done.Count == 0) return this;
        var since = done[^1].Changed!.Value;   // без даты строка не разбирается
        var minute = Minute(since);
        var seen = done.Where(r => r.Changed >= minute).Select(r => r.Id);
        if (Minute(Since) == minute) seen = seen.Concat(Seen);
        return new(since, seen.Distinct().Order().ToArray());
    }

    /// <summary>Та же отметка: и время, и номера минуты (у записи список сравнивается по ссылке).</summary>
    public bool SameAs(ExportMark other) => Since == other.Since && Seen.SequenceEqual(other.Seen);

    /// <summary>Минута, с которой сервер вернёт заявки по этой дате: сравнивает он до минуты.</summary>
    internal static DateTimeOffset Minute(DateTimeOffset d) => new(d.Year, d.Month, d.Day, d.Hour, d.Minute, 0, d.Offset);
}

/// <summary>Что выгружать автоматически: закрытые заявки, где я исполнитель, инициатор или участник. Задел на этап 7
/// («всё доступное»): другая область — другой отбор и другое правило «моё ли», ход выгрузки тот же.</summary>
public sealed record ExportScope(IReadOnlyCollection<int> ClosedStatusIds, IntraserviceUser Me)
{
    /// <summary>По возрастанию изменения, при равных — по номеру: отметка движется только вперёд.</summary>
    public const string Sort = "Changed asc, Id asc";

    /// <summary>Закрытые, изменённые с минуты отметки.</summary>
    public TaskQuery Query(ExportMark mark) => new(StatusIds: ClosedStatusIds, Changed: new DateSpan(mark.Since.LocalDateTime, null), Sort: Sort);

    /// <summary>Исполнитель или инициатор — видно по строке списка: по номерам, а нет их в строке — по именам.</summary>
    public bool MineByRow(IntraserviceFound row) =>
        (row.ExecutorIds is { } executors ? executors.Contains(Me.Id) : NamedIn(row.Executors))
        || (row.CreatorId is { } creator ? creator == Me.Id : NamedIn(row.Creator));

    /// <summary>Участник: в истории есть моё действие — комментарий или смена статуса (по номеру автора, нет номера — по имени).
    /// Моё имя в тексте чужого комментария — не участие.</summary>
    public bool MineByHistory(IEnumerable<IntraserviceEvent> events) =>
        events.Any(e => e.AuthorId is { } id ? id == Me.Id : IsMe(e.Author));

    /// <summary>Моё имя среди перечисленных через запятую («Иванов И. И., Петров П.»).</summary>
    private bool NamedIn(string? names) =>
        (names ?? "").Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).Any(IsMe);

    /// <summary>Это я по имени; моего имени сервер не прислал — ничьё имя не моё.</summary>
    private bool IsMe(string name) => Me.Name.Trim().Length > 0 && AutoSyncRules.SameName(name, Me.Name);
}

/// <summary>Итог захода автовыгрузки. Next — новая отметка; NotMine — разобранные «не мои» (номер → изменение), чтобы
/// не читать их историю снова, пока они не изменились; Mine — моих среди разобранных; Exported — файлов, что теперь
/// соответствуют серверу; More — порция кончилась, а список нет; Error — что не дало пройти дальше (отметка — перед этим).</summary>
public sealed record ClosedExportPass(ExportMark Next, IReadOnlyDictionary<int, DateTimeOffset> NotMine, int Mine, int Exported,
    bool More, string Error);

/// <summary>Автовыгрузка закрытых заявок в базу знаний — один заход: закрытые, изменённые после отметки, по порядку
/// изменения; «моё ли» — по строке или по истории (ExportScope); мои — тем же движком, что окно «Поиск заявок»
/// (KnowledgeExport.ExportRowsAsync). Не больше budget заявок, которым нужен запрос; отметка сдвигается только за
/// разобранные подряд — сбой останавливает её на себе, следующий заход повторит.</summary>
public static class ClosedExport
{
    /// <summary>ponytail: столько заявок с запросом (история для разбора или выгрузка) за заход, по 5 минут — 360 в час.
    /// Догонять неделю большой компании окажется долго — поднять или ходить чаще. Самопроверка ставит меньше.</summary>
    internal static int Budget = 30;

    public static async Task<ClosedExportPass> RunAsync(HttpIntraserviceClient client, ExportScope scope, ExportMark mark,
        IReadOnlyDictionary<int, DateTimeOffset> notMine, string dir, Func<int, string> ticketUrl, int budget, CancellationToken ct)
    {
        var known = new Dictionary<int, DateTimeOffset>(notMine);
        var taken = new List<(IntraserviceFound Row, bool Mine)>();   // разобранные — по порядку списка
        var (cost, error, more) = (0, "", false);
        IntraserviceFound? previous = null;
        // одна страница за заход: следующий начнёт с новой отметки, и список, «поехавший» от чужих изменений, не теряет
        // строк между страницами. Дальше страницы — только если вся она уже разобрана (сотни заявок одной минуты)
        var pageSize = Math.Clamp(budget + mark.Seen.Count + 10, 20, HttpIntraserviceClient.ExecutorPageSize);
        for (var page = 1; ; page++)
        {
            var list = await client.GetTasksAsync(scope.Query(mark), page, ct, pageSize);
            if (list.Error.Length > 0) { error = list.Error; break; }
            // порядок и даты — до разбора: строка не по порядку, разобранная первой, увела бы отметку через более раннюю
            if (Disorder(list.Found, previous) is { Length: > 0 } wrong) { error = wrong; break; }
            previous = list.Found.LastOrDefault() ?? previous;
            var takenBefore = taken.Count;
            foreach (var row in list.Found)
            {
                var changed = row.Changed!.Value;
                if (mark.Covers(row)) continue;
                var mine = scope.MineByRow(row);
                var knownNotMine = !mine && known.TryGetValue(row.Id, out var at) && at == changed;
                if (!knownNotMine)
                {
                    if (cost == budget) { more = true; break; }
                    cost++;
                }
                if (!mine && !knownNotMine)
                {
                    var (events, lifetimeError, _) = await KnowledgeExport.ReadLifetimeAsync(client, row.Id, ct);
                    if (lifetimeError.Length > 0) { error = $"#{row.Id}: {lifetimeError}"; break; }
                    mine = scope.MineByHistory(events);
                    if (!mine) known[row.Id] = changed;
                }
                taken.Add((row, mine));
            }
            var full = list.Found.Count >= pageSize;
            more |= full && error.Length == 0;
            if (error.Length > 0 || taken.Count > takenBefore || !full) break;
        }

        var matched = taken.Where(t => t.Mine).Select(t => t.Row).ToList();
        var export = matched.Count > 0 ? await KnowledgeExport.ExportRowsAsync(client, matched, dir, ticketUrl, null, ct) : null;
        // отметка — за разобранными подряд: моя, чей файл не стал актуальным, останавливает её на себе
        var done = new List<IntraserviceFound>();
        foreach (var (row, mine) in taken)
        {
            if (mine && export?.Current.Contains(row.Id) != true)
            {
                if (error.Length == 0)
                    error = $"не выгружена #{row.Id}: " + (export is { FirstError.Length: > 0 } ? export.FirstError : export?.Error is { Length: > 0 } e ? e : "файл не записан");
                break;
            }
            done.Add(row);
        }
        var next = mark.After(done);
        var keep = known.Where(k => k.Value >= ExportMark.Minute(next.Since)).ToDictionary(k => k.Key, k => k.Value);
        return new ClosedExportPass(next, keep, matched.Count, export is null ? 0 : export.Created + export.Updated, more && error.Length == 0, error);
    }

    /// <summary>Что не так со страницей списка: строка без даты изменения или не по возрастанию (изменение, при равных —
    /// номер; before — последняя строка прошлой страницы). Пусто — всё в порядке.</summary>
    private static string Disorder(IReadOnlyList<IntraserviceFound> rows, IntraserviceFound? before)
    {
        foreach (var row in rows)
        {
            if (row.Changed is not { } changed)
                return $"в строке заявки #{row.Id} нет даты изменения (Changed) — не понять, докуда разобрано";
            if (before?.Changed is { } b && (changed < b || changed == b && row.Id <= before.Id))
                return $"сервер отдал список не по возрастанию изменения (#{before.Id}, затем #{row.Id}) — отметка перескочила бы заявки";
            before = row;
        }
        return "";
    }

    /// <summary>Правила без сервера: запрос, «моё ли» по строке и по истории, отметка и её минута. Ход захода против
    /// поддельного сервера — TicketBoard.SelfCheck/AutoExportCheck.cs.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var me = new IntraserviceUser(7, "Я Сам");
        var scope = new ExportScope(new[] { 29, 30 }, me);
        var at = new DateTime(2026, 10, 1, 12, 5, 30);
        static IntraserviceFound Row(int id, DateTime? changed = null, int? creatorId = null, int[]? executorIds = null,
            string? creator = null, string? executors = null) =>
            new(id, "x", "Закрыта", creator, null, Executors: executors, Changed: changed is { } c ? new DateTimeOffset(c) : null,
                CreatorId: creatorId, ExecutorIds: executorIds);

        // запрос: закрытые статусы, изменённые с минуты отметки (сервер сравнивает до минуты), по возрастанию изменения
        var mark = new ExportMark(new DateTimeOffset(at), new[] { 5 });
        var url = scope.Query(mark).ToUrl(1, 60);
        Debug.Assert(url == "api/task?StatusIds=29,30&ChangedMoreThan=2026-10-01%2012%3A05&include=status&sort=Changed%20asc,%20Id%20asc&pagesize=60&page=1", url);

        // по строке: исполнитель и инициатор по номерам; номеров нет — по именам (без учёта регистра и лишних пробелов)
        Debug.Assert(scope.MineByRow(Row(1, executorIds: new[] { 9, 7 })) && scope.MineByRow(Row(2, creatorId: 7, executorIds: new[] { 9 })),
            "исполнитель и инициатор по номерам");
        Debug.Assert(!scope.MineByRow(Row(3, creatorId: 5, executorIds: new[] { 9 }, creator: "Я Сам", executors: "Я Сам")),
            "номера есть — имена не решают");
        Debug.Assert(scope.MineByRow(Row(4, executors: "Иванов И., я  сам")) && scope.MineByRow(Row(5, creator: "Я Сам")),
            "номеров нет — по именам");
        Debug.Assert(!scope.MineByRow(Row(6, executors: "Иванов И.", creator: "Петрова А.")), "чужая по именам");
        Debug.Assert(!(scope with { Me = new IntraserviceUser(7, "") }).MineByRow(Row(7, executors: "", creator: "")),
            "моего имени сервер не прислал — пустое с пустым не совпадает");

        // по истории: любое моё действие — комментарий или смена статуса; по номеру, нет номера — по имени
        static IntraserviceEvent E(string author, int? id, string? text = "текст") =>
            new(DateTimeOffset.Now, author, "Закрыта", text, true, id);
        Debug.Assert(scope.MineByHistory(new[] { E("Петрова А.", 5), E("Я Сам", 7, null) }), "смена статуса — тоже участие");
        Debug.Assert(scope.MineByHistory(new[] { E("я сам", null) }), "без номера — по имени");
        Debug.Assert(!scope.MineByHistory(new[] { E("Петрова А.", 5, "Я Сам, посмотрите") }), "моё имя в тексте чужого комментария — не участие");
        Debug.Assert(!scope.MineByHistory(new[] { E("Я Сам", 8) }), "номер автора чужой — имя не решает");
        Debug.Assert(!scope.MineByHistory(Array.Empty<IntraserviceEvent>()), "пустая история");

        // отметка: разобранные строки её минуты пропускаются, пока не изменились
        Debug.Assert(mark.Covers(Row(5, at)) && mark.Covers(Row(5, at.AddSeconds(-20))), "разобранная в минуту отметки");
        Debug.Assert(!mark.Covers(Row(5, at.AddSeconds(1))), "изменилась ещё раз — разбирается заново");
        Debug.Assert(!mark.Covers(Row(6, at)) && !mark.Covers(Row(5)), "не разобранная и без даты — не пропускаются");
        // после разобранных: время последней и номера её минуты; минута та же — прежние номера остаются
        var next = mark.After(new[] { Row(6, at.AddSeconds(10)), Row(8, at.AddSeconds(20)) });
        Debug.Assert(next.Since == new DateTimeOffset(at.AddSeconds(20)) && next.Seen.SequenceEqual(new[] { 5, 6, 8 }),
            $"та же минута: {next.Since:HH:mm:ss} [{string.Join(",", next.Seen)}]");
        var later = next.After(new[] { Row(9, at.AddMinutes(1)), Row(10, at.AddMinutes(2).AddSeconds(5)) });
        Debug.Assert(later.Since == new DateTimeOffset(at.AddMinutes(2).AddSeconds(5)) && later.Seen.SequenceEqual(new[] { 10 }),
            $"минута сменилась — только её номера: {later.Since:HH:mm:ss} [{string.Join(",", later.Seen)}]");
        Debug.Assert(mark.After(Array.Empty<IntraserviceFound>()).SameAs(mark) && !next.SameAs(mark), "SameAs");
        Debug.Assert(ExportMark.Start(new DateTimeOffset(at)).Since == new DateTimeOffset(at.AddDays(-7)), "первое включение — неделя назад");
    }
}
