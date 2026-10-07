using System.Diagnostics;
using System.Globalization;

namespace TicketBoard.Services;

/// <summary>Какие заявки по статусу: любые; закрытые (в них решение) и открытые — по признакам статусов сервера и
/// ClosedStatusNames, как у импорта и F5; или один конкретный статус (SearchFilter.StatusId).</summary>
public enum SearchStatus { Any, Closed, Open, One }

/// <summary>Условия окна поиска так, как их видит человек; последние запоминаются в settings.json. Words — слова для поиска
/// Интрасервиса (в полях заявки и во всех её комментариях). Mine — исполнитель я; Executor, Creator — часть имени
/// сотрудника (ищется по справочнику сотрудников), вместе с Mine исполнитель — любой из них. ServiceId, TypeId — 0 это «любой»;
/// WithChildren — сервис вместе с вложенными. Даты — «дд.мм.гггг» как набраны, пусто — не задана, период включает оба дня
/// целиком. SavedFilterId — сохранённый фильтр веб-интерфейса, 0 — без него. IncludeArchived — и заявки архивных и
/// неактуальных сервисов. Limit — не больше стольких заявок при выгрузке найденного в файлы.</summary>
public sealed record SearchFilter(
    string Words = "", bool Mine = false, string Executor = "", string Creator = "",
    SearchStatus Status = SearchStatus.Any, int StatusId = 0,
    int ServiceId = 0, bool WithChildren = true, int TypeId = 0,
    string CreatedFrom = "", string CreatedTo = "", string ChangedFrom = "", string ChangedTo = "",
    string ClosedFrom = "", string ClosedTo = "",
    int SavedFilterId = 0, bool IncludeArchived = true, int Limit = 500);

/// <summary>Условия, превращённые в запрос к API (Query), и пояснения для человека (Notes): кто нашёлся по имени.</summary>
public sealed record ResolvedSearch(TaskQuery Query, IReadOnlyList<string> Notes);

/// <summary>От условий окна к запросу: «я» — номер из api/user, имена — номера из справочника сотрудников, «закрытые» —
/// номера статусов, сервис с вложенными — номера по Path, даты текстом — период. Одно правило и для списка, и для выгрузки.</summary>
public static class TicketSearch
{
    /// <summary>Сколько сотрудников может подойти под одно имя, чтобы искать по всем сразу; больше — имя надо уточнить.</summary>
    public const int MaxUsersPerName = 10;

    /// <summary>Слова короче — слишком много совпадений: сервер ищет по всем комментариям всех заявок.</summary>
    public const int MinWordsLength = 3;

    private static readonly string[] DateFormats = { "dd.MM.yyyy", "d.M.yyyy", "dd.MM.yy", "yyyy-MM-dd" };

    /// <summary>Запрос по условиям; ошибка (не пуста) — понятный текст для окна, запрос не получен. Дешёвые проверки — до
    /// первого обращения к серверу. statuses и services — справочники, уже загруженные окном.</summary>
    public static async Task<(ResolvedSearch? Resolved, string Error)> ResolveAsync(HttpIntraserviceClient client, SearchFilter f,
        IReadOnlyList<IntraserviceStatus> statuses, IReadOnlyList<IntraserviceRef> services, IReadOnlySet<string> closedNames,
        CancellationToken ct)
    {
        var words = (f.Words ?? "").Trim();
        if (words.Length is > 0 and < MinWordsLength) return (null, $"Слова — минимум {MinWordsLength} символа");
        // «Не больше, заявок» к поиску не относится — это потолок выгрузки в файлы; его проверяет выгрузка, а не поиск

        var (created, error) = ParseSpan(f.CreatedFrom, f.CreatedTo, "Создана");
        if (error.Length > 0) return (null, error);
        (var changed, error) = ParseSpan(f.ChangedFrom, f.ChangedTo, "Изменена");
        if (error.Length > 0) return (null, error);
        (var closed, error) = ParseSpan(f.ClosedFrom, f.ClosedTo, "Закрыта");
        if (error.Length > 0) return (null, error);

        List<int>? statusIds = null;
        if (f.Status == SearchStatus.One)
        {
            if (f.StatusId <= 0) return (null, "Выберите статус");
            statusIds = new() { f.StatusId };
        }
        else if (f.Status is SearchStatus.Closed or SearchStatus.Open)
        {
            if (statuses.Count == 0) return (null, "Список статусов не загрузился — «закрытые» и «открытые» определить нечем; выберите статус «Любой»");
            var wantClosed = f.Status == SearchStatus.Closed;
            statusIds = statuses.Where(s => (s.IsFixed || s.IsFinal || closedNames.Contains(s.Name)) == wantClosed).Select(s => s.Id).ToList();
            if (statusIds.Count == 0)
                return (null, wantClosed ? "Сервер не назвал ни одного закрытого статуса" : "Все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json");
        }

        List<int>? serviceIds = null;
        if (f.ServiceId > 0) serviceIds = f.WithChildren ? ServiceWithChildren(services, f.ServiceId) : new() { f.ServiceId };

        var notes = new List<string>();
        var executors = new List<int>();
        if (f.Mine)
        {
            var (me, meError) = await client.GetCurrentUserAsync(ct);
            if (me is null) return (null, meError);
            executors.Add(me.Id);
        }
        if (!string.IsNullOrWhiteSpace(f.Executor))
        {
            var (ids, peopleError) = await FindPeopleAsync(client, f.Executor.Trim(), "Исполнитель", notes, ct);
            if (peopleError.Length > 0) return (null, peopleError);
            executors.AddRange(ids);
        }
        List<int>? creators = null;
        if (!string.IsNullOrWhiteSpace(f.Creator))
        {
            var (ids, peopleError) = await FindPeopleAsync(client, f.Creator.Trim(), "Заявитель", notes, ct);
            if (peopleError.Length > 0) return (null, peopleError);
            creators = ids;
        }

        var query = new TaskQuery(
            ExecutorIds: executors.Count > 0 ? executors.Distinct().ToList() : null, StatusIds: statusIds,
            Search: words.Length > 0 ? words : null, CreatorIds: creators, ServiceIds: serviceIds,
            TypeIds: f.TypeId > 0 ? new[] { f.TypeId } : null, Created: created, Changed: changed, Closed: closed,
            FilterId: f.SavedFilterId > 0 ? f.SavedFilterId : null, IncludeArchived: f.IncludeArchived);
        // ничего не выбрано — это не «пусто», а все заявки; выгрузка всех (Limit 0) так и задаётся
        if (query.IsUnrestricted) notes.Add("Условий нет: ищу среди всех заявок, которые видит ваша учётная запись");
        return (new(query, notes), "");
    }

    /// <summary>Номера сотрудников, чьё имя (логин, почта…) содержит text. Никого или слишком много — ошибка: молча искать
    /// «по всем Ивановым» или «ни по кому» значило бы показать не то, что просили.</summary>
    private static async Task<(List<int> Ids, string Error)> FindPeopleAsync(HttpIntraserviceClient client, string text, string label,
        List<string> notes, CancellationToken ct)
    {
        var (users, total, error) = await client.FindUsersAsync(text, MaxUsersPerName, ct);
        if (error.Length > 0) return (new(), $"{label} «{text}»: не удалось найти сотрудника\n{error}");
        if (users.Count == 0) return (new(), $"{label} «{text}»: такого сотрудника не нашёл — достаточно части фамилии, но верной");
        if (users.Count > MaxUsersPerName || total > MaxUsersPerName)
            return (new(), $"{label} «{text}»: подходит больше {MaxUsersPerName} сотрудников — уточните");
        notes.Add(users.Count == 1 ? $"{label}: {users[0].Name}"
            : $"{label} «{text}»: подошли {users.Count} — {string.Join(", ", users.Select(u => u.Name))}");
        return (users.Select(u => u.Id).ToList(), "");
    }

    /// <summary>Сервис и все вложенные в него: у вложенного в Path («840|844|») есть номер этого сервиса. Так условие
    /// «сервис» не зависит от того, берёт ли сервер вложенные сам.</summary>
    internal static List<int> ServiceWithChildren(IReadOnlyList<IntraserviceRef> services, int id)
    {
        var ids = new List<int> { id };
        foreach (var s in services)
            if (s.Id != id && s.Path is { Length: > 0 } path && ("|" + path).Contains($"|{id}|")) ids.Add(s.Id);
        return ids;
    }

    /// <summary>Название сервиса для списка выбора: вложенные — со сдвигом по глубине, архивные — с пометкой.</summary>
    public static string ServiceLabel(IntraserviceRef s)
    {
        var depth = s.Path is { Length: > 0 } path ? path.Split('|', StringSplitOptions.RemoveEmptyEntries).Length - 1 : 0;
        return new string(' ', Math.Max(0, depth) * 3) + (s.IsArchive ? "(архив) " : "") + s.Name;
    }

    /// <summary>Период из двух дат «дд.мм.гггг» (пусто — граница не задана): с начала первого дня до начала дня после
    /// последнего. Граница «по» уходит серверу как «меньше или равна» (док., стр. 18): с 23:59 теряется последняя минута
    /// дня, с полуночью следующего дня лишь заявка ровно в 00:00:00 попадёт лишней — это куда реже.</summary>
    internal static (DateSpan Span, string Error) ParseSpan(string? from, string? to, string label)
    {
        if (!TryDate(from, out var a)) return (default, $"{label}: дата «{from!.Trim()}» не понятна — нужно дд.мм.гггг, год 2000–2100");
        if (!TryDate(to, out var b)) return (default, $"{label}: дата «{to!.Trim()}» не понятна — нужно дд.мм.гггг, год 2000–2100");
        if (a is { } start && b is { } end && start > end) return (default, $"{label}: начало периода позже его конца");
        return (new(a?.Date, b?.Date.AddDays(1)), "");
    }

    /// <summary>Пусто — подходит (даты нет); непусто — только если читается как дата и год разумный: у краёв диапазона
    /// DateTime арифметика с днём (граница «по» — следующая полночь, допуск в сутки) бросала бы исключение.</summary>
    private static bool TryDate(string? text, out DateTime? date)
    {
        date = null;
        if (string.IsNullOrWhiteSpace(text)) return true;
        if (!DateTime.TryParseExact(text.Trim(), DateFormats, CultureInfo.InvariantCulture, DateTimeStyles.None, out var d)
            || d.Year is < 2000 or > 2100) return false;
        date = d;
        return true;
    }

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // даты: период с начала первого дня до начала следующего за последним; пусто — не задано; разные записи одной даты
        var (span, spanError) = ParseSpan("01.02.2026", "28.2.2026", "Создана");
        Debug.Assert(spanError == "" && span.From == new DateTime(2026, 2, 1) && span.To == new DateTime(2026, 3, 1));
        Debug.Assert(ParseSpan(" ", null, "Создана") is { Span.IsEmpty: true, Error: "" });
        Debug.Assert(ParseSpan("", "2026-03-05", "Создана").Span is { From: null, To: { } t } && t == new DateTime(2026, 3, 6));
        Debug.Assert(ParseSpan("31.02.2026", "", "Закрыта").Error.StartsWith("Закрыта: дата «31.02.2026»"));   // такого дня нет
        Debug.Assert(ParseSpan("01.03.2026", "01.02.2026", "Изменена").Error.Contains("позже"));
        // края диапазона DateTime: арифметика с днём там бросает исключение — такие даты просто не принимаются
        Debug.Assert(ParseSpan("", "31.12.9999", "Создана").Error.Contains("не понятна") && ParseSpan("01.01.0001", "", "Создана").Error.Contains("не понятна"));
        Debug.Assert(ParseSpan("01.01.2000", "31.12.2100", "Создана").Error == "");
        Debug.Assert(ParseSpan("05.03.2026", "05.03.2026", "Изменена").Error == "");   // один день — тоже период

        // сервис с вложенными — по Path; соседний номер с общей цифрой не подходит; архивный и глубокий — в списке выбора
        var tree = new IntraserviceRef[]
        {
            new(840, "ТСД", "840|"), new(844, "Приложение на ТСД", "840|844|"), new(850, "Mobile Mark", "840|844|850|"),
            new(84, "Другой", "84|"), new(9, "Почта", "9|", IsArchive: true),
        };
        Debug.Assert(ServiceWithChildren(tree, 840).SequenceEqual(new[] { 840, 844, 850 }));
        Debug.Assert(ServiceWithChildren(tree, 844).SequenceEqual(new[] { 844, 850 }) && ServiceWithChildren(tree, 84).SequenceEqual(new[] { 84 }));
        Debug.Assert(ServiceLabel(tree[2]) == "      Mobile Mark" && ServiceLabel(tree[4]) == "(архив) Почта" && ServiceLabel(new(1, "Без пути")) == "Без пути");

        // живое: «я», имена по справочнику сотрудников, статусы, сервис — и то, что в итоге уходит серверу
        var asked = new List<string>();
        var (listener, port) = FakeIntraservice.Start(target =>
        {
            asked.Add(target);
            if (target.StartsWith("/api/user?getcurrentuserinfo=true")) return (200, """{"Id":7,"Name":"Я Сам"}""");
            if (target.StartsWith("/api/user?")) return FakeUsers(target);
            return (200, """{"Tasks":[],"Paginator":{"Count":0,"Page":1,"PageCount":1}}""");
        });
        try
        {
            var client = new HttpIntraserviceClient($"http://127.0.0.1:{port}", "u", "p");
            var statuses = new IntraserviceStatus[] { new(31, "Открыта", false, false), new(29, "Выполнена", true, false), new(30, "Закрыта", false, false) };
            var closedNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "Закрыта" };
            (ResolvedSearch? Resolved, string Error) Resolve(SearchFilter f) =>
                ResolveAsync(client, f, statuses, tree, closedNames, CancellationToken.None).GetAwaiter().GetResult();

            // всё сразу: я и ещё Максимов — исполнители (я не задваиваюсь), заявитель по двум Ивановым, закрытые, сервис с вложенными
            var all = Resolve(new(Words: " принтер ", Mine: true, Executor: "максимов", Creator: "иванов", Status: SearchStatus.Closed,
                ServiceId: 840, TypeId: 1009, CreatedFrom: "01.01.2026", ChangedTo: "31.12.2026", SavedFilterId: 45, Limit: 100));
            Debug.Assert(all.Error == "" && all.Resolved is not null);
            var r = all.Resolved!;
            Debug.Assert(r.Query.ExecutorIds!.SequenceEqual(new[] { 7, 38472 }) && r.Query.CreatorIds!.SequenceEqual(new[] { 5, 6 })
                && r.Query.StatusIds!.SequenceEqual(new[] { 29, 30 }) && r.Query.ServiceIds!.SequenceEqual(new[] { 840, 844, 850 })
                && r.Query.TypeIds!.SequenceEqual(new[] { 1009 }) && r.Query.Search == "принтер" && r.Query.FilterId == 45
                && r.Query.Created.From == new DateTime(2026, 1, 1) && r.Query.Changed.To == new DateTime(2027, 1, 1));
            Debug.Assert(r.Notes.SequenceEqual(new[] { "Исполнитель: Максимов М. С.", "Заявитель «иванов»: подошли 2 — Иванов А., Иванов Б." }));
            // и это уходит серверу как есть: списком заявок через тот же адрес, что и у окна поиска
            client.GetTasksAsync(r.Query, 1, detailed: true).GetAwaiter().GetResult();
            Debug.Assert(asked[^1].Contains("ExecutorIds=7,38472&StatusIds=29,30&CreatorIds=5,6&ServiceIds=840,844,850&TypeIds=1009&search=")
                && asked[^1].Contains("&CreatedMoreThan=2026-01-01%2000%3A00&ChangedLessThan=2027-01-01%2000%3A00&filterid=45&")
                && asked[^1].Contains("&include=status,service&count=all&"));
            // без условий — запрос без условий (последние заявки), «я» и сотрудников не ищем
            asked.Clear();
            Debug.Assert(Resolve(new()) is { Error: "", Resolved.Query: { ExecutorIds: null, StatusIds: null, Search: null, ServiceIds: null } } && asked.Count == 0);
            // и об этом сказано: «ничего не выбрано» — не пусто, а все заявки учётной записи (с них начинается выгрузка всех)
            Debug.Assert(Resolve(new()).Resolved!.Notes is [var everything] && everything.StartsWith("Условий нет"));
            Debug.Assert(Resolve(new(Limit: 0, IncludeArchived: false)).Resolved!.Notes.Count == 1 && Resolve(new(Words: "принтер")).Resolved!.Notes.Count == 0);
            Debug.Assert(Resolve(new(Status: SearchStatus.Closed)).Resolved!.Notes.Count == 0 && Resolve(new(SavedFilterId: 45)).Resolved!.Notes.Count == 0);

            // ошибки понятны и приходят до запроса заявок: короткие слова, нет такого сотрудника, слишком много, не тот статус
            Debug.Assert(Resolve(new(Words: "аб")).Error.Contains("минимум 3"));
            Debug.Assert(Resolve(new(Creator: "никого")).Error.StartsWith("Заявитель «никого»: такого сотрудника не нашёл"));
            Debug.Assert(Resolve(new(Executor: "много")).Error.Contains("больше 10 сотрудников"));
            Debug.Assert(Resolve(new(Status: SearchStatus.One)).Error == "Выберите статус");
            Debug.Assert(Resolve(new(Limit: 0)) is { Error: "", Resolved: not null });   // потолок выгрузки поиску не мешает
            Debug.Assert(Resolve(new(ChangedFrom: "вчера")).Error.StartsWith("Изменена: дата «вчера»"));
            Debug.Assert(ResolveAsync(client, new(Status: SearchStatus.Open), Array.Empty<IntraserviceStatus>(), tree, closedNames, CancellationToken.None)
                .GetAwaiter().GetResult().Error.Contains("не загрузился"));
            // открытые — те, что не закрыты ни признаком сервера, ни списком из настроек
            Debug.Assert(Resolve(new(Status: SearchStatus.Open)).Resolved!.Query.StatusIds!.SequenceEqual(new[] { 31 }));
            Debug.Assert(Resolve(new(Status: SearchStatus.One, StatusId: 31)).Resolved!.Query.StatusIds!.SequenceEqual(new[] { 31 }));
        }
        finally { listener.Stop(); }
    }

    /// <summary>Сотрудники поддельного сервера: по «максимов» один, по «иванов» два, по «много» одиннадцать, по другому — никого.</summary>
    private static (int Code, string Json) FakeUsers(string target)
    {
        static string Page(IEnumerable<(int Id, string Name)> users) =>
            "{\"Users\":[" + string.Join(",", users.Select(u => $"{{\"Id\":{u.Id},\"Name\":\"{u.Name}\"}}")) + "],\"Paginator\":{\"Count\":"
            + users.Count() + ",\"Page\":1,\"PageCount\":1}}";
        var search = Uri.UnescapeDataString(target.Split("search=")[1].Split('&')[0]);
        return (200, search switch
        {
            "максимов" => Page(new[] { (38472, "Максимов М. С.") }),
            "иванов" => Page(new[] { (5, "Иванов А."), (6, "Иванов Б.") }),
            "много" => Page(Enumerable.Range(100, 11).Select(i => (i, $"Сотрудник {i}"))),
            _ => Page(Array.Empty<(int, string)>()),
        });
    }
}
