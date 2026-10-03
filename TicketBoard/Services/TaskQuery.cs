using System.Diagnostics;
using System.Globalization;
using System.Text;

namespace TicketBoard.Services;

/// <summary>Период по дате, границы включительно; null — граница не задана. Обе границы у сервера включительные: «…MoreThan»
/// — «больше или равна», «…LessThan» — «меньше или равна» (док., стр. 18). Даты — в часовом поясе пользователя Интрасервиса
/// (так их ждёт API, док., стр. 10): человек вводит их по своему времени, туда же они и уходят.</summary>
public readonly record struct DateSpan(DateTime? From, DateTime? To)
{
    public bool IsEmpty => From is null && To is null;
}

/// <summary>Отбор заявок для api/task: всё, чем список заявок умеет фильтровать (док., стр. 14-20), склеивается по «И».
/// null или пусто — условия нет; ничего не задано — все заявки, что видит пользователь. Номера в списках — через запятую,
/// сервер берёт заявки, где подходит хотя бы один из них. Created, Changed, Closed — когда создана, изменена, закрыта.
/// FilterId — сохранённый фильтр веб-интерфейса (отбор, которого здесь нет, собирают там один раз). IncludeArchived —
/// и заявки архивных и неактуальных сервисов: по умолчанию сервер их не отдаёт.</summary>
public sealed record TaskQuery(
    IReadOnlyCollection<int>? ExecutorIds = null, IReadOnlyCollection<int>? StatusIds = null, string? Search = null,
    IReadOnlyCollection<int>? CreatorIds = null, IReadOnlyCollection<int>? ServiceIds = null, IReadOnlyCollection<int>? TypeIds = null,
    DateSpan Created = default, DateSpan Changed = default, DateSpan Closed = default,
    int? FilterId = null, bool IncludeArchived = false)
{
    /// <summary>Поля строки для запроса с сохранённым фильтром: он сам задаёт, какие поля вернуть, а нам нужны эти (имена —
    /// из части документации «поля для списка», стр. 10-12). ponytail: на живом сервере запрос с filterid и fields не
    /// проверен; не примет — окно покажет ответ сервера, и тогда править этот список по нему.</summary>
    private const string RowFields = "Id,Name,Description,StatusId,Created,Changed,Creator,CreatorId,Executors,ExecutorIds,"
        + "ExecutorGroup,ExecutorGroupId,ServiceId,TypeId,Type,Categories,ResolutionDateFact,Closed";

    /// <summary>Адрес страницы списка. detailed — для окна поиска: общее число точное (count=all: без него сервер считает
    /// не больше тысячи, док., стр. 14) и названия сервисов в том же ответе (include=service).</summary>
    public string ToUrl(int page, int pageSize, bool detailed = false)
    {
        var url = new StringBuilder("api/task?");
        void Ids(string key, IReadOnlyCollection<int>? ids)
        {
            if (ids is { Count: > 0 }) url.Append(key).Append('=').Append(string.Join(",", ids)).Append('&');
        }
        void Span(string key, DateSpan span)
        {
            if (span.From is { } from) url.Append(key).Append("MoreThan=").Append(Uri.EscapeDataString(Stamp(from))).Append('&');
            if (span.To is { } to) url.Append(key).Append("LessThan=").Append(Uri.EscapeDataString(Stamp(to))).Append('&');
        }

        Ids("ExecutorIds", ExecutorIds);
        Ids("StatusIds", StatusIds);
        Ids("CreatorIds", CreatorIds);
        Ids("ServiceIds", ServiceIds);
        Ids("TypeIds", TypeIds);
        if (!string.IsNullOrWhiteSpace(Search)) url.Append("search=").Append(Uri.EscapeDataString(Search.Trim())).Append('&');
        Span("Created", Created);
        Span("Changed", Changed);
        Span("Closed", Closed);
        if (FilterId is int filter) url.Append("filterid=").Append(filter).Append("&fields=").Append(RowFields).Append('&');
        if (IncludeArchived) url.Append("archive=true&inactive=true&");
        // ponytail: без fields — ответ жирнее, зато не упадёт на незнакомом имени поля; появится нужда экономить трафик — добавить fields и проверить на живом сервере.
        url.Append(detailed ? "include=status,service&count=all&" : "include=status&");
        url.Append("sort=Changed%20desc&pagesize=").Append(pageSize).Append("&page=").Append(Math.Max(1, page));
        return url.ToString();
    }

    /// <summary>Дата для запроса: единственный формат из примеров документации с временем («2015-11-13 10:00»).</summary>
    private static string Stamp(DateTime d) => d.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);

    /// <summary>Строка явно не из периода создания или изменения: значит, сервер условие по дате не применил (формат даты в
    /// запросе на живом сервере не проверен). С допуском в сутки — на часовые пояса и округление: ловим только очевидное.
    /// Остальные условия по строке не проверить — в ней нет нужных полей.</summary>
    public bool Outside(IntraserviceFound f) => Violates(Created, f.Created) || Violates(Changed, f.Changed);

    private static bool Violates(DateSpan span, DateTimeOffset? value) =>
        value is { } v && ((span.From is { } from && v.DateTime < from.AddDays(-1)) || (span.To is { } to && v.DateTime > to.AddDays(1)));

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        // без условий — тот же адрес, что был у списка до этого отбора: импорт и автообновление не меняются
        Debug.Assert(new TaskQuery().ToUrl(1, 200) == "api/task?include=status&sort=Changed%20desc&pagesize=200&page=1");
        Debug.Assert(new TaskQuery(ExecutorIds: new[] { 7 }, StatusIds: new[] { 29, 30 }).ToUrl(2, 200)
            == "api/task?ExecutorIds=7&StatusIds=29,30&include=status&sort=Changed%20desc&pagesize=200&page=2");
        // пустые списки и пробельные слова — не условия; страница не меньше первой
        Debug.Assert(new TaskQuery(ExecutorIds: Array.Empty<int>(), Search: "  ").ToUrl(0, 50)
            == "api/task?include=status&sort=Changed%20desc&pagesize=50&page=1");

        // всё сразу: имена параметров — по документации (стр. 18-20), даты со временем, слова — в процентах, запрос окна поиска
        var full = new TaskQuery(new[] { 7, 9 }, new[] { 29 }, "принтер C# & .NET", new[] { 5 }, new[] { 844, 845 }, new[] { 1009 },
            Created: new(new DateTime(2026, 1, 1), new DateTime(2026, 1, 31, 23, 59, 0)),
            Changed: new(new DateTime(2026, 2, 3, 4, 5, 0), null), Closed: new(null, new DateTime(2026, 12, 31, 23, 59, 0)),
            FilterId: 45, IncludeArchived: true);
        Debug.Assert(full.ToUrl(3, 50, detailed: true) == "api/task?ExecutorIds=7,9&StatusIds=29&CreatorIds=5&ServiceIds=844,845&TypeIds=1009"
            + "&search=%D0%BF%D1%80%D0%B8%D0%BD%D1%82%D0%B5%D1%80%20C%23%20%26%20.NET"
            + "&CreatedMoreThan=2026-01-01%2000%3A00&CreatedLessThan=2026-01-31%2023%3A59&ChangedMoreThan=2026-02-03%2004%3A05"
            + "&ClosedLessThan=2026-12-31%2023%3A59&filterid=45&fields=" + RowFields
            + "&archive=true&inactive=true&include=status,service&count=all&sort=Changed%20desc&pagesize=50&page=3");

        // сервер не применил условие по дате — строки вне периода видны; в пределах суток от границы — не считаем
        var span = new TaskQuery(Changed: new(new DateTime(2026, 6, 1), new DateTime(2026, 6, 30, 23, 59, 0)),
            Created: new(new DateTime(2026, 1, 1), null));
        IntraserviceFound Row(DateTime? created, DateTime? changed) => new(1, "N", "Закрыта", null,
            created is { } c ? new DateTimeOffset(c) : null, Changed: changed is { } h ? new DateTimeOffset(h) : null);
        Debug.Assert(!span.Outside(Row(new DateTime(2026, 3, 1), new DateTime(2026, 6, 15))));
        Debug.Assert(span.Outside(Row(new DateTime(2026, 3, 1), new DateTime(2026, 8, 1))));      // изменена позже периода
        Debug.Assert(span.Outside(Row(new DateTime(2025, 5, 1), new DateTime(2026, 6, 15))));     // создана раньше периода
        Debug.Assert(!span.Outside(Row(new DateTime(2025, 12, 31, 20, 0, 0), new DateTime(2026, 7, 1, 3, 0, 0))));   // на границе: допуск
        Debug.Assert(!span.Outside(Row(null, null)) && !new TaskQuery().Outside(Row(new DateTime(1999, 1, 1), null)));
    }
}
