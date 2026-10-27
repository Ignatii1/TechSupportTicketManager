using System.Diagnostics;

namespace TicketBoard.Services;

/// <summary>Отметка автовыгрузки: закрытые заявки, изменённые до Since, уже разобраны; Seen — номера разобранных в минуту
/// отметки (сервер сравнивает даты до минуты, и заявки этой минуты придут снова). Хранится в settings.json.
/// ponytail: время отметки — часы самого сервера: даты он отдаёт без пояса (живой, 2026-10-02: «2026-08-27T15:54:43.46»),
/// разбор их не сдвигает, и в ChangedMoreThan они уходят как есть. Отдай сервер /Date(ms)/ из другого пояса, чем у ПК, —
/// окно сдвинется на разницу: к востоку ранние строки придут снова (Covers пропустит их; столько, что заполнят страницу, —
/// ошибка «условие по дате», а у сервера без Paginator, что режет страницу короче запрошенной, — молчаливое «новых нет»:
/// полноты такой страницы не видно), к западу строки сдвига потеряются. И заявка, чьё изменение сервер покажет позже, чем через
/// минуту (долгая транзакция), останется позади отметки — как её отсёк бы и сам ChangedMoreThan; догонит окно «Поиск заявок».</summary>
public sealed record ExportMark(DateTimeOffset Since, IReadOnlyList<int> Seen)
{
    /// <summary>Первое включение: неделя назад — то, что закрыли, пока выгрузки не было; старше — окно «Поиск заявок».</summary>
    public static ExportMark Start(DateTimeOffset now) => new(now.AddDays(-7), Array.Empty<int>());

    /// <summary>Строка уже разобрана: изменена раньше минуты отметки (правильный сервер её и не вернул бы; вернул — у его
    /// пользователя другой часовой пояс) или номер — среди разобранных минуты отметки, а изменение не позже отметки
    /// (изменилась ещё раз — разбирается заново).</summary>
    public bool Covers(IntraserviceFound row) =>
        row.Changed is { } changed && (changed < Minute(Since) || changed <= Since && Seen.Contains(row.Id));

    /// <summary>Отметка после строк, разобранных подряд по порядку списка: изменение последней и номера её минуты (минута
    /// та же, что у прежней отметки, — и прежние номера).</summary>
    public ExportMark After(IReadOnlyList<IntraserviceFound> done)
    {
        if (done.Count == 0) return this;
        // без даты строка не разбирается; назад отметка не идёт: поздно показавшаяся строка минуты отметки бывает и раньше её
        var since = done[^1].Changed!.Value is var last && last > Since ? last : Since;
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

/// <summary>Что выгружать автоматически: закрытые заявки, где я исполнитель, инициатор, наблюдатель или участник. Задел на этап 7
/// («всё доступное»): другая область — другой отбор и другое правило «моё ли», ход выгрузки тот же. ClosedNames — закрытые
/// названия из настроек: по ним в файле считаются закрытия и переоткрытия (ExportContext). Attachments — чьи вложения
/// качать (настройка «Вложения в базу знаний»).</summary>
public sealed record ExportScope(IReadOnlyCollection<int> ClosedStatusIds, IntraserviceUser Me, IReadOnlySet<string> ClosedNames,
    AttachmentScope Attachments = AttachmentScope.None)
{
    /// <summary>Кто я, какие статусы закрытые и чьи вложения качать — для файла заявки. Автовыгрузка берёт только мои (своим
    /// правилом: и с урезанной историей, где роли в файле может не найтись), так что «моих» для неё — все.</summary>
    public ExportContext Context => new(Me, ClosedNames, Attachments == AttachmentScope.Mine ? AttachmentScope.All : Attachments);

    /// <summary>По возрастанию изменения, при равных — по номеру: отметка движется только вперёд.</summary>
    public const string Sort = TaskQuery.ChangedAscSort;

    /// <summary>Закрытые, изменённые с минуты отметки, — и у архивных и неактивных сервисов: без archive/inactive сервер их
    /// молча не отдаёт, и такая заявка не выгрузилась бы никогда.</summary>
    public TaskQuery Query(ExportMark mark) =>
        new(StatusIds: ClosedStatusIds, Changed: new DateSpan(mark.Since.LocalDateTime, null), IncludeArchived: true, Sort: Sort);

    /// <summary>Исполнитель, инициатор или наблюдатель — видно по строке списка: по номерам, а нет их в строке — по именам.</summary>
    public bool MineByRow(IntraserviceFound row) => new Myself(Me) is var i && (i.Executes(row) || i.Initiated(row) || i.Observes(row));

    /// <summary>Участник: в истории есть моё действие — комментарий или смена статуса (по номеру автора, нет номера — по имени).
    /// Моё имя в тексте чужого комментария — не участие.</summary>
    public bool MineByHistory(IEnumerable<IntraserviceEvent> events) => events.Any(new Myself(Me).Wrote);
}

/// <summary>Я в заявке — по номерам, а нет их — по именам: исполнитель, инициатор, наблюдатель, автор записи истории. Одно
/// правило у автовыгрузки («моё ли») и у роли в базе знаний (TicketHistory).</summary>
public sealed record Myself(IntraserviceUser Me)
{
    public bool Executes(IntraserviceFound row) => row.ExecutorIds is { } executors ? executors.Contains(Me.Id) : NamedIn(row.Executors);

    public bool Initiated(IntraserviceFound row) => row.CreatorId is { } creator ? creator == Me.Id : NamedIn(row.Creator);

    public bool Observes(IntraserviceFound row) => row.ObserverIds is { } observers ? observers.Contains(Me.Id) : NamedIn(row.Observers);

    public bool Wrote(IntraserviceEvent e) => e.AuthorId is { } id ? id == Me.Id : IsMe(e.Author);

    /// <summary>Моё имя среди перечисленных через запятую («Иванов И. И., Петров П.»).</summary>
    private bool NamedIn(string? names) =>
        (names ?? "").Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries).Any(IsMe);

    /// <summary>Это я по имени; моего имени сервер не прислал — ничьё имя не моё.</summary>
    private bool IsMe(string name) => Me.Name.Trim().Length > 0 && AutoSyncRules.SameName(name, Me.Name);
}

/// <summary>Заявка, на которой заходы спотыкаются: на каком её изменении, сколько заходов, с какого времени и последняя
/// ошибка.</summary>
public sealed record ExportFailure(int Id, DateTimeOffset Changed, int Times, DateTimeOffset First, string Error);

/// <summary>Заявки, на которых заходы спотыкаются из-за самих заявок (сервер отвечает ошибкой только на неё, к её истории
/// нет доступа, антивирус удаляет её файл): номер → неудача. Одна такая держала бы отметку на себе вечно, поэтому после
/// GiveUpTimes заходов подряд, но не раньше GiveUpAfter от первого, она пропускается: отметка идёт дальше, номер — в лог.
/// Сбои сети и учётной записи сюда не попадают (их пережидают, а не пропускают). В памяти: после перезапуска счёт заново.</summary>
public sealed record ExportFailures(IReadOnlyDictionary<int, ExportFailure> Items)
{
    public const int GiveUpTimes = 5;

    /// <summary>Не раньше: сбой сервера, при котором список отдаётся, а заявки — нет, столько обычно не длится. Самопроверка
    /// ставит ноль.</summary>
    internal static TimeSpan GiveUpAfter = TimeSpan.FromHours(1);

    public static ExportFailures None { get; } = new(new Dictionary<int, ExportFailure>());

    /// <summary>Пропустить: на этом же её изменении заходы спотыкались GiveUpTimes раз, и первый — не позже GiveUpAfter назад.</summary>
    public bool GivenUp(IntraserviceFound row, DateTimeOffset now) =>
        Items.TryGetValue(row.Id, out var f) && f.Changed == row.Changed && f.Times >= GiveUpTimes && now - f.First >= GiveUpAfter;

    /// <summary>После захода: прошедшие — из счёта вон; споткнувшиеся — ещё раз в счёт (изменилась с прошлого раза — счёт
    /// заново); до кого заход не дошёл — как были.</summary>
    public ExportFailures After(IEnumerable<(IntraserviceFound Row, string Error)> failed, IEnumerable<int> passed, DateTimeOffset now)
    {
        var items = new Dictionary<int, ExportFailure>(Items);
        foreach (var id in passed) items.Remove(id);
        foreach (var (row, error) in failed)
            items[row.Id] = Items.TryGetValue(row.Id, out var f) && f.Changed == row.Changed
                ? f with { Times = f.Times + 1, Error = error }
                : new ExportFailure(row.Id, row.Changed!.Value, 1, now, error);
        return new(items);
    }
}

/// <summary>Итог захода автовыгрузки. Next — новая отметка; NotMine — разобранные «не мои» (номер → изменение), чтобы
/// не читать их историю снова, пока они не изменились; Failures — на каких заявках заходы спотыкаются; Exported — файлов,
/// что теперь соответствуют серверу; More — порция кончилась, а список нет; Error — что не дало пройти дальше (отметка —
/// перед этим); GivenUp — пропущенные в этот заход после GiveUpTimes неудач (в лог).</summary>
public sealed record ClosedExportPass(ExportMark Next, IReadOnlyDictionary<int, DateTimeOffset> NotMine, ExportFailures Failures,
    int Exported, bool More, string Error, IReadOnlyList<ExportFailure> GivenUp);

/// <summary>Автовыгрузка закрытых заявок в базу знаний — один заход: закрытые, изменённые после отметки, по порядку
/// изменения; «моё ли» — по строке или по истории (ExportScope); мои — тем же движком, что окно «Поиск заявок»
/// (KnowledgeExport.ExportRowsAsync). Не больше budget заявок, которым нужен запрос; отметка сдвигается только за
/// разобранные подряд — сбой останавливает её на себе, следующий заход повторит.</summary>
public static class ClosedExport
{
    /// <summary>ponytail: столько заявок с запросом (история для разбора или выгрузка) за заход, по 5 минут — 360 в час.
    /// Догонять неделю большой компании окажется долго — поднять или ходить чаще. Самопроверка ставит меньше.</summary>
    internal static int Budget = 30;

    /// <summary>ponytail: больше строк на страницу документация не даёт (pagesize до 2000). Минута, в которой изменилось
    /// больше закрытых заявок, чем сервер отдаёт за раз, останавливает выгрузку с ошибкой «за раз» — тогда читать такую
    /// минуту постранично, сверяя, что список между страницами не сдвинулся.</summary>
    private const int MaxPage = 2000;

    private enum Verdict { Mine, NotMine, Failed, GivenUp }

    public static async Task<ClosedExportPass> RunAsync(HttpIntraserviceClient client, ExportScope scope, ExportMark mark,
        IReadOnlyDictionary<int, DateTimeOffset> notMine, ExportFailures failures, string dir, Func<int, string> ticketUrl,
        int budget, DateTimeOffset now, CancellationToken ct)
    {
        ClosedExportPass Stopped(string error) => new(mark, notMine, failures, 0, false, error, Array.Empty<ExportFailure>());

        // одна страница за заход, одним запросом — снимок: читай мы дальше, заявка, изменённая между страницами, сдвинула
        // бы список, и строка со следующей страницы ускользнула бы на прочитанную. Страница — порция, номера минуты
        // отметки (они придут снова) и запас; следующий заход начнёт с новой отметки
        var pageSize = Math.Clamp(budget + mark.Seen.Count + 10, 20, MaxPage);
        var list = await client.GetTasksAsync(scope.Query(mark), 1, ct, pageSize, timeout: KnowledgeExport.ListTimeout);
        if (list.Error.Length > 0) return Stopped(list.Error);
        // порядок и даты — до разбора: строка не по порядку, разобранная первой, увела бы отметку через более раннюю
        if (Disorder(list.Found) is { Length: > 0 } wrong) return Stopped(wrong);
        // вся страница раньше отметки, и это не сдвиг поясов (страница полная или уходит дальше него) — сервер не применил
        // условие по дате (формат на живом сервере не проверен): так и сказать, а не молча пропускать страницу за страницей
        // или вечно отвечать «новых нет»; ранние строки среди поздних и короткую страницу сдвига пропускает Covers
        var beyond = list.Found.Count >= pageSize || list.HasNext == true || list.Total > list.Found.Count
            || HttpIntraserviceClient.Capped(list.Total);
        if (BeforeMark(list.Found, mark, beyond) is { Length: > 0 } early) return Stopped(early);

        var known = new Dictionary<int, DateTimeOffset>(notMine);
        var rows = new List<(IntraserviceFound Row, Verdict Verdict)>();   // разобранные — по порядку списка
        var missed = new Dictionary<int, (string Message, bool OfTicket)>();
        var (cost, more, listed) = (0, false, new HashSet<int>());
        foreach (var row in list.Found)
        {
            if (mark.Covers(row) || !listed.Add(row.Id)) continue;
            if (failures.GivenUp(row, now)) { rows.Add((row, Verdict.GivenUp)); continue; }
            var changed = row.Changed!.Value;
            var mine = scope.MineByRow(row);
            var knownNotMine = !mine && known.TryGetValue(row.Id, out var at) && at == changed;
            // ponytail: «моя» по строке тратит место в порции, даже если её файл уже верен (что верен, видно только выгрузке):
            // после сброса отметки (выключили и включили, сменили папку) уже выгруженные догоняются по Budget за заход
            if (!knownNotMine)
            {
                if (cost == budget) { more = true; break; }
                cost++;
            }
            if (!mine && !knownNotMine)
            {
                var (events, lifetimeError, truncated) = await KnowledgeExport.ReadLifetimeAsync(client, row.Id, ct);
                if (lifetimeError.Length > 0)
                {
                    missed[row.Id] = ($"#{row.Id}: история не читается — {lifetimeError}", OfTicket(lifetimeError));
                    rows.Add((row, Verdict.Failed));
                    if (missed[row.Id].OfTicket) continue;
                    break;   // сеть или учётная запись: остальные упадут так же — до следующего захода
                }
                // история обрезана (длиннее, чем читаем) — моё действие могло остаться в непрочитанной, ранней части: выгружаем,
                // а не записываем в «не мои» навсегда (лишний файл лучше потерянной заявки)
                mine = truncated || scope.MineByHistory(events);
                if (!mine) known[row.Id] = changed;
            }
            rows.Add((row, mine ? Verdict.Mine : Verdict.NotMine));
        }

        var matched = rows.Where(r => r.Verdict == Verdict.Mine).Select(r => r.Row).ToList();
        var export = matched.Count > 0 ? await KnowledgeExport.ExportRowsAsync(client, matched, dir, ticketUrl, null, ct, context: scope.Context) : null;
        // файл не записался: временный рядом записался, а на месте файла — папка или файл только для чтения — беда этой
        // заявки, в счёт; не записался и временный — беда папки (диск полон, нет доступа): ждём, не пропускаем
        foreach (var row in matched.Where(r => export?.Current.Contains(r.Id) != true))
            missed[row.Id] = export is not null && export.Errors.TryGetValue(row.Id, out var e)
                ? ($"не выгружена #{row.Id}: {e}", export.NotWritten.Contains(row.Id) ? export.PathBlocked.Contains(row.Id) : OfTicket(e))
                // своей ошибки нет — выгрузку остановил выключатель (логин, сеть): причина — в её итоге, и она не в счёт
                : ($"не выгружена #{row.Id}: {(export?.Error is { Length: > 0 } summary ? summary : "файл не записан")}", false);

        // отметка — за разобранными подряд: неразобранная или моя, чей файл не стал актуальным, останавливает её на себе
        var done = rows.TakeWhile(r => !missed.ContainsKey(r.Row.Id)).Select(r => r.Row).ToList();
        var next = mark.After(done);
        var error = rows.FirstOrDefault(r => missed.ContainsKey(r.Row.Id)) is { Row: { } first } ? missed[first.Id].Message : "";
        if (error.Length == 0 && beyond && rows.Count == 0)
            error = $"в минуту {ExportMark.Minute(mark.Since):dd.MM.yyyy HH:mm} изменилось больше закрытых заявок, чем сервер отдаёт "
                + $"за раз ({list.Found.Count}), и все отданные уже разобраны — отметка дальше не сдвинется";
        var keep = known.Where(k => k.Value >= ExportMark.Minute(next.Since)).ToDictionary(k => k.Key, k => k.Value);
        // пропущенная остаётся в счёте, пока отметка её не прошла (впереди споткнулась другая): следующий заход пропустит
        // её снова, без запросов, а в лог она попадёт один раз — когда отметка пройдёт
        var nextFailures = failures.After(
            rows.Where(r => missed.TryGetValue(r.Row.Id, out var m) && m.OfTicket).Select(r => (r.Row, missed[r.Row.Id].Message)),
            rows.Where((r, i) => !missed.ContainsKey(r.Row.Id) && (r.Verdict != Verdict.GivenUp || i < done.Count)).Select(r => r.Row.Id), now);
        return new ClosedExportPass(next, keep, nextFailures, export is null ? 0 : export.Created + export.Updated,
            (more || beyond) && error.Length == 0, error,
            rows.Take(done.Count).Where(r => r.Verdict == Verdict.GivenUp).Select(r => failures.Items[r.Row.Id]).ToList());
    }

    /// <summary>Сбой из-за самой заявки — в счёт её пропуска: отказ 4xx (кроме 401, 408, 429), 500, ответ, который не
    /// разобрался, файл, который записался и пропал (антивирус — по её содержимому). Не её вина: сеть, сроки, 502–504 и
    /// неверный логин. Незаписанный файл решает RunAsync: в счёт — только если мешает место самого файла
    /// (ExportResult.PathBlocked); не записался и временный рядом — беда папки, не в счёт.</summary>
    internal static bool OfTicket(string error) =>
        HttpIntraserviceClient.HttpCode(error) == 500 || !HttpIntraserviceClient.IsTransient(error) && HttpIntraserviceClient.HttpCode(error) != 401;

    /// <summary>Часы двух поясов расходятся не больше чем на 26 часов (UTC−12 и UTC+14) — с запасом: строки раньше отметки
    /// дальше этого сдвигом поясов не объяснить.</summary>
    private static readonly TimeSpan MaxSkew = TimeSpan.FromDays(2);

    /// <summary>Вся страница раньше минуты отметки (строки по возрастанию: последняя раньше — и все), и она полная (дальше
    /// есть ещё) или её первая строка раньше отметки дальше, чем бывает сдвиг поясов: сервер не применил ChangedMoreThan, и к
    /// новым заявкам не пробиться. Раньше лишь часть или короткая страница в пределах сдвига (новых пока нет, а сервер со
    /// сдвигом отдал прежние) — такие строки пропускает Covers. Пусто — в порядке.</summary>
    internal static string BeforeMark(IReadOnlyList<IntraserviceFound> rows, ExportMark mark, bool full) =>
        rows.Count > 0 && rows[^1].Changed < ExportMark.Minute(mark.Since) && (full || rows[0].Changed < mark.Since - MaxSkew)
            ? $"сервер вернул только заявки, изменённые раньше отметки ({mark.Since:dd.MM.yyyy HH:mm}; последняя — #{rows[^1].Id}, "
              + $"{rows[^1].Changed:dd.MM.yyyy HH:mm}): похоже, не применил условие по дате (ChangedMoreThan); отметка стоит"
            : "";

    /// <summary>Что не так со страницей списка: строка без даты изменения или не по возрастанию изменения. Равные — в любом
    /// порядке номеров: вторую часть сортировки (Id) сервер вправе не соблюсти, а отметке она не нужна. Пусто — всё в порядке.</summary>
    internal static string Disorder(IReadOnlyList<IntraserviceFound> rows)
    {
        IntraserviceFound? before = null;
        foreach (var row in rows)
        {
            if (row.Changed is not { } changed)
                return $"в строке заявки #{row.Id} нет даты изменения (Changed) — не понять, докуда разобрано";
            if (before?.Changed is { } b && changed < b)
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
        var scope = new ExportScope(new[] { 29, 30 }, me, new HashSet<string>());
        // вложения: автовыгрузка берёт только мои (своим правилом, с урезанной историей тоже), так что «моих» — это все её
        Debug.Assert(new ExportScope(new[] { 29 }, me, new HashSet<string>(), AttachmentScope.Mine).Context.Attachments == AttachmentScope.All
            && new ExportScope(new[] { 29 }, me, new HashSet<string>(), AttachmentScope.None).Context.Attachments == AttachmentScope.None
            && scope.Context.Attachments == AttachmentScope.None, "вложения автовыгрузки");
        var at = new DateTime(2026, 10, 1, 12, 5, 30);
        static IntraserviceFound Row(int id, DateTime? changed = null, int? creatorId = null, int[]? executorIds = null,
            string? creator = null, string? executors = null) =>
            new(id, "x", "Закрыта", creator, null, Executors: executors, Changed: changed is { } c ? new DateTimeOffset(c) : null,
                CreatorId: creatorId, ExecutorIds: executorIds);

        // запрос: закрытые статусы, изменённые с минуты отметки (сервер сравнивает до минуты), по возрастанию изменения
        var mark = new ExportMark(new DateTimeOffset(at), new[] { 5 });
        var url = scope.Query(mark).ToUrl(1, 60);
        // и архивные с неактивными сервисами: без этих двух сервер их молча не отдаёт (HISTORY, 0.12.0) — закрытая заявка такого
        // сервиса иначе не выгрузилась бы никогда, отметка ушла бы за неё
        Debug.Assert(url == "api/task?StatusIds=29,30&ChangedMoreThan=2026-10-01%2012%3A05&archive=true&inactive=true&include=status&sort=Changed%20asc,%20Id%20asc&pagesize=60&page=1", url);

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
        Debug.Assert(!(scope with { Me = new IntraserviceUser(7, "") }).MineByHistory(new[] { E("", null) }),
            "моего имени сервер не прислал — событие без автора и номера не моё");

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
        // строка раньше минуты отметки — уже разобрана (правильный сервер её и не вернул бы; вернул — сдвиг поясов): пропуск
        Debug.Assert(mark.Covers(Row(99, at.AddMinutes(-1))), "раньше минуты отметки — пропускается");
        // поздно показавшаяся строка минуты отметки, раньше самой отметки: отметка назад не идёт, номера минуты — вместе
        var late = mark.After(new[] { Row(11, at.AddSeconds(-20)) });
        Debug.Assert(late.Since == new DateTimeOffset(at) && late.Seen.SequenceEqual(new[] { 5, 11 }) && late.Covers(Row(5, at)),
            $"отметка назад не идёт: {late.Since:HH:mm:ss} [{string.Join(",", late.Seen)}]");
        // «сервер не применил условие по дате» — только когда раньше отметки вся страница; часть — пропускается
        Debug.Assert(BeforeMark(new[] { Row(1, at.AddDays(-3)), Row(2, at.AddMinutes(-2)) }, mark, full: true).Contains("ChangedMoreThan"),
            "полная страница — вся раньше");
        Debug.Assert(BeforeMark(new[] { Row(1, at.AddMinutes(-2)), Row(2, at.AddMinutes(1)) }, mark, full: true) == "", "раньше — только часть");
        // страница не полная, и вся раньше (новых нет, а сервер со сдвигом отдал прежние) — не ошибка: пропустятся, «новых нет»
        Debug.Assert(BeforeMark(new[] { Row(1, at.AddMinutes(-2)) }, mark, full: false) == "", "короткая страница ранних — не ошибка");
        Debug.Assert(BeforeMark(new[] { Row(1, at.AddHours(-26)), Row(2, at.AddMinutes(-2)) }, mark, full: false) == "",
            "короткая страница на 26 часов раньше — крайний сдвиг поясов, не ошибка");
        // короткая, но её первая строка раньше отметки дальше любого сдвига поясов (сервер без Paginator режет страницу сам) —
        // сдвигом это не объяснить: условие не применено
        Debug.Assert(BeforeMark(new[] { Row(1, at.AddDays(-3)), Row(2, at.AddMinutes(-2)) }, mark, full: false).Contains("ChangedMoreThan"),
            "короткая страница из давних — не применил условие");
        Debug.Assert(ExportMark.Start(new DateTimeOffset(at)).Since == new DateTimeOffset(at.AddDays(-7)), "первое включение — неделя назад");

        // порядок страницы: по возрастанию изменения; равные — в любом порядке номеров; без даты — не разобрать
        Debug.Assert(Disorder(new[] { Row(3, at), Row(2, at), Row(4, at.AddSeconds(1)) }) == "", "равные — в любом порядке номеров");
        Debug.Assert(Disorder(new[] { Row(3, at.AddSeconds(1)), Row(4, at) }).Contains("не по возрастанию"), "раньше после позже");
        Debug.Assert(Disorder(new[] { Row(3, at), Row(4) }).Contains("нет даты"), "строка без даты");

        // в счёт пропуска — сбои самой заявки; сеть, сроки, 502–504 и логин — нет
        foreach (var e in new[] { "ошибка сервера (HTTP 500):\n{}", "нет доступа (HTTP 403)", "заявка не найдена (HTTP 404)",
                     "сервер ответил, но не так (HTTP 200): разбор не удался", "файл записан, но пропал с диска — похоже, его удалил антивирус" })
            Debug.Assert(OfTicket(e), e);
        foreach (var e in new[] { "сервер не ответил за 10 с", "сервер недоступен: connection refused", "соединение оборвалось",
                     "сервер перегружен (HTTP 503)", "шлюз (HTTP 502)", "срок шлюза (HTTP 504)", "неверный логин или пароль (HTTP 401)",
                     "слишком много запросов (HTTP 429)" })
            Debug.Assert(!OfTicket(e), e);

        // пропуск: GiveUpTimes заходов подряд на том же изменении, и первый — не меньше GiveUpAfter назад
        var t0 = new DateTimeOffset(at);
        var broken = Row(40, at);
        var failures = ExportFailures.None;
        for (var i = 0; i < ExportFailures.GiveUpTimes; i++)
            failures = failures.After(new[] { (broken, $"сбой {i}") }, Array.Empty<int>(), t0.AddMinutes(5 * i));
        Debug.Assert(failures.Items[40] is { Times: ExportFailures.GiveUpTimes, Error: "сбой 4" } f40 && f40.First == t0, "счёт и последняя ошибка");
        Debug.Assert(!failures.GivenUp(broken, t0.AddMinutes(59)) && failures.GivenUp(broken, t0.AddHours(1)),
            "пять заходов — но не раньше часа от первого");
        Debug.Assert(!failures.GivenUp(Row(40, at.AddMinutes(1)), t0.AddHours(2)), "заявка изменилась — пробуем снова");
        var four = Enumerable.Range(0, ExportFailures.GiveUpTimes - 1).Aggregate(ExportFailures.None,
            (acc, i) => acc.After(new[] { (broken, "сбой") }, Array.Empty<int>(), t0.AddMinutes(i)));
        Debug.Assert(!four.GivenUp(broken, t0.AddDays(1)), "на заход меньше — хоть сутки спустя, пробуем");
        var changedAgain = failures.After(new[] { (Row(40, at.AddMinutes(1)), "новый сбой") }, Array.Empty<int>(), t0.AddHours(2));
        Debug.Assert(changedAgain.Items[40] is { Times: 1, Error: "новый сбой" } c40 && c40.First == t0.AddHours(2), "изменилась — счёт заново");
        Debug.Assert(!failures.After(Array.Empty<(IntraserviceFound, string)>(), new[] { 40 }, t0).Items.ContainsKey(40), "прошла — из счёта вон");
        Debug.Assert(failures.After(Array.Empty<(IntraserviceFound, string)>(), new[] { 41 }, t0).Items[40].Times == ExportFailures.GiveUpTimes,
            "не дошли до неё — счёт как был");
        Debug.Assert(ExportFailures.None.Items.Count == 0 && !ExportFailures.None.GivenUp(broken, t0.AddDays(1)), "пусто — ничего не пропускаем");
    }
}
