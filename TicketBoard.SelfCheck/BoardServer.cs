using System.Diagnostics;
using System.Net.Sockets;
using System.Text.Json;
using TicketBoard.Services;

namespace TicketBoard.SelfCheck;

/// <summary>Поддельный Интрасервис «моих заявок» для проверок доски (BoardCheck, CardCheck): заявки — статус, моя ли,
/// исполнители, Changed, переписка — меняет проверка между шагами, отвечает поток сервера (FakeIntraservice), всё под
/// одним замком. Я — «Я Сам» (№ 7), инициатор всех заявок — «Петрова А.». Запись в переписку двигает Changed заявки — так
/// автообновление узнаёт, что её пора перечитать.</summary>
internal sealed class BoardServer : IDisposable
{
    public const int MeId = 7;
    public const string MeName = "Я Сам";
    public const string Requester = "Петрова А.";
    public const int RequesterId = 50, OtherId = 60;
    public const string Phone = "+7 900 000-00-00";
    public const string Group = "Вторая линия";
    public const int Open = 31, Working = 32, Fixed = 29, Final = 30, Cancelled = 34;
    // «Выполнена» закрыта признаком IsFixed, «Закрыта» — IsFinal, «Отменена» — только по названию из ClosedStatusNames
    private const string Statuses = """[{"Id":31,"Name":"Открыта","IsFixed":false,"IsFinal":false},{"Id":32,"Name":"В работе"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта","IsFinal":true},{"Id":34,"Name":"Отменена"}]""";

    /// <summary>Заявка. Mine — я в исполнителях (попадает в список «мои открытые»); номера исполнителей в строке — по нему
    /// (я или OtherId), если не заданы ExecutorIdList. Creator/CreatorId — инициатор. NoIds — строка без номеров инициатора и
    /// исполнителей (как, возможно, у живого сервера). CardFails — карточка отвечает 500; LifetimeFails — переписка отвечает
    /// этим кодом (0 — как обычно).</summary>
    public sealed class FakeTask(int id, string name)
    {
        public int Id { get; } = id;
        public string Name { get; set; } = name;
        public string? Description { get; set; }
        public int StatusId { get; set; } = Open;
        public bool Mine { get; set; } = true;
        public string Executors { get; set; } = MeName;
        public int[]? ExecutorIdList { get; set; }
        public string Creator { get; set; } = Requester;
        public int CreatorId { get; set; } = RequesterId;
        public bool NoIds { get; set; }
        public bool CardFails { get; set; }
        public int LifetimeFails { get; set; }
        public DateTime Changed { get; set; }
        public List<Event> Events { get; } = new();
        /// <summary>Переписка длиннее страницы — Paginator скажет «есть ещё».</summary>
        public bool MoreEvents { get; set; }
        /// <summary>Переписку не отдаём (HTTP 403): у учётной записи к ней нет доступа.</summary>
        public bool EventsRefused { get; set; }
        /// <summary>Телефона инициатора в ответе нет — как у заявки, где его не заполнили.</summary>
        public bool NoPhone { get; set; }
    }

    /// <summary>Запись переписки: комментарий (Text) и/или статус на тот момент; IsPublic: false — внутренний комментарий.</summary>
    public sealed record Event(DateTime Date, string Editor, int EditorId, string? Text, int StatusId, bool IsPublic = true);

    private readonly object _lock = new();
    private readonly Dictionary<int, FakeTask> _tasks = new();
    private readonly List<(string Target, long At)> _asked = new();   // At — Stopwatch.GetTimestamp() на приходе
    private readonly List<string> _unexpected = new();
    private readonly TcpListener _listener;
    private DateTime _clock;
    private int _failList;
    private bool _brokenPaging;
    private bool _ignoreSort, _noChanged, _tiesDescending;
    private int _pageCap;
    private bool _emptyStatuses;
    private int _repeatRow;

    /// <summary>start — с какой минуты идут часы сервера (каждое изменение заявки — следующая минута).</summary>
    public BoardServer(DateTime? start = null)
    {
        _clock = start ?? new DateTime(2026, 10, 1, 9, 0, 0);
        (_listener, Port) = FakeIntraservice.Start(Respond);
    }

    public int Port { get; }
    public string Url => $"http://127.0.0.1:{Port}";

    /// <summary>Список закрытых (без ExecutorIds) отдаётся по убыванию изменения, что бы ни просили в sort.</summary>
    public bool IgnoreSort { get { lock (_lock) return _ignoreSort; } set { lock (_lock) _ignoreSort = value; } }

    /// <summary>Список закрытых — по возрастанию изменения, но равные — по убыванию номера (вторую часть sort сервер не соблюл).</summary>
    public bool TiesDescending { get { lock (_lock) return _tiesDescending; } set { lock (_lock) _tiesDescending = value; } }

    /// <summary>Эта заявка приходит в списке дважды подряд (сервер повторил строку); 0 — без повторов.</summary>
    public int RepeatRow { get { lock (_lock) return _repeatRow; } set { lock (_lock) _repeatRow = value; } }

    /// <summary>Справочник статусов — пустой список.</summary>
    public bool EmptyStatuses { get { lock (_lock) return _emptyStatuses; } set { lock (_lock) _emptyStatuses = value; } }

    /// <summary>Строк на страницу — не больше этого, сколько бы ни просили в pagesize; 0 — сколько просят.</summary>
    public int PageCap { get { lock (_lock) return _pageCap; } set { lock (_lock) _pageCap = value; } }

    /// <summary>Строки списка — без Changed.</summary>
    public bool NoChanged { get { lock (_lock) return _noChanged; } set { lock (_lock) _noChanged = value; } }

    /// <summary>Список заявок отвечает этим кодом; 0 — как обычно.</summary>
    public int FailList { get { lock (_lock) return _failList; } set { lock (_lock) _failList = value; } }

    /// <summary>Первая страница списка обещает больше, чем отдаёт, вторая — 503: список неполный.</summary>
    public bool BrokenPaging { get { lock (_lock) return _brokenPaging; } set { lock (_lock) _brokenPaging = value; } }

    /// <summary>Закрыты — список заявок ждёт, пока проверка их не откроет: настройки меняются ровно посреди захода, без гонки.</summary>
    public ManualResetEventSlim ListGate { get; } = new(true);

    /// <summary>Запросы, которых сервер не знает, — их быть не должно.</summary>
    public IReadOnlyList<string> Unexpected { get { lock (_lock) return _unexpected.ToList(); } }

    public void Dispose()
    {
        ListGate.Set();
        _listener.Stop();
    }

    public DateTime Tick() { lock (_lock) return _clock = _clock.AddMinutes(1); }

    public void Put(int id, string name, int status = Open, bool mine = true, string executors = MeName, string? description = null)
    {
        lock (_lock)
            _tasks[id] = new FakeTask(id, name) { StatusId = status, Mine = mine, Executors = executors, Description = description, Changed = Tick() };
    }

    public void Change(int id, Action<FakeTask> change)
    {
        lock (_lock)
        {
            change(_tasks[id]);
            _tasks[id].Changed = Tick();
        }
    }

    /// <summary>Комментарий в заявку — со статусом, какой у неё сейчас; дата — Changed заявки.</summary>
    public DateTime Comment(int id, string editor, int editorId, string text, bool isPublic = true)
    {
        lock (_lock)
        {
            var t = _tasks[id];
            var at = Tick();
            t.Events.Add(new(at, editor, editorId, text, t.StatusId, isPublic));
            t.Changed = at;
            return at;
        }
    }

    /// <summary>Смена статуса без комментария — тоже запись переписки.</summary>
    public DateTime StatusChange(int id, int status, string editor = "Система", int editorId = 1)
    {
        lock (_lock)
        {
            var t = _tasks[id];
            var at = Tick();
            t.StatusId = status;
            t.Events.Add(new(at, editor, editorId, null, status));
            t.Changed = at;
            return at;
        }
    }

    public DateTime ChangedOf(int id) { lock (_lock) return _tasks[id].Changed; }

    /// <summary>Поменять, не меняя заявку на сервере (Changed прежний): сбой и починка самого сервера — не изменение заявки.</summary>
    public void Quietly(int id, Action<FakeTask> change) { lock (_lock) change(_tasks[id]); }

    /// <summary>Изменение заявки — ровно в это время (часы сервера не двигаются): заявки одной минуты, давние заявки.</summary>
    public void SetChanged(int id, DateTime at) { lock (_lock) _tasks[id].Changed = at; }
    public int Count(string prefix) { lock (_lock) return _asked.Count(a => a.Target.StartsWith(prefix)); }
    public string Last(string prefix) { lock (_lock) return _asked.LastOrDefault(a => a.Target.StartsWith(prefix)).Target ?? ""; }

    /// <summary>Через сколько после since (Stopwatch.GetTimestamp()) пришёл первый такой запрос; null — не приходил. Паузу
    /// перед запросом так видно и на медленной машине: задержка потока делает приход только позже, а не раньше. Отметка —
    /// когда сервер прочитал запрос, а обслуживает он по одному соединению: мерить, только когда других запросов нет, иначе
    /// посланный сразу запрос, простояв в очереди, сошёл бы за отложенный.</summary>
    public TimeSpan? WaitedSince(long since, string prefix)
    {
        lock (_lock)
            return _asked.Where(a => a.At >= since && a.Target.StartsWith(prefix)).Select(a => (long?)a.At).FirstOrDefault() is long at
                ? Stopwatch.GetElapsedTime(since, at) : null;
    }

    /// <summary>Дата сервера как её разберёт клиент: без пояса — местное время.</summary>
    public static DateTimeOffset At(DateTime d) => new(d);

    private (int, string) Respond(string target)
    {
        lock (_lock) _asked.Add((target, Stopwatch.GetTimestamp()));
        if (target.StartsWith("/api/task?")) ListGate.Wait(TimeSpan.FromSeconds(10));
        lock (_lock)
        {
            if (target == "/api/user?getcurrentuserinfo=true") return (200, $"{{\"Id\":{MeId},\"Name\":\"{MeName}\"}}");
            if (target == "/api/taskstatus") return (200, _emptyStatuses ? "[]" : Statuses);
            if (target.StartsWith("/api/task?"))
            {
                if (_failList > 0) return (_failList, "{\"Message\":\"Service Unavailable\"}");
                var q = Query(target);
                var page = int.Parse(q.GetValueOrDefault("page", "1"));
                var size = int.Parse(q.GetValueOrDefault("pagesize", "50"));
                if (_pageCap > 0) size = Math.Min(size, _pageCap);
                var statusIds = q.GetValueOrDefault("StatusIds", "").Split(',', StringSplitOptions.RemoveEmptyEntries).Select(int.Parse).ToHashSet();
                List<FakeTask> rows;
                if (q.ContainsKey("ExecutorIds"))   // мои открытые — импорт и автообновление
                {
                    var mine = q["ExecutorIds"] == $"{MeId}";
                    rows = _tasks.Values.Where(t => mine && t.Mine && statusIds.Contains(t.StatusId))
                        .OrderByDescending(t => t.Changed).ThenByDescending(t => t.Id).ToList();
                }
                else   // закрытые, изменённые с минуты отметки (сервер сравнивает до минуты) — автовыгрузка
                {
                    DateTime? since = q.TryGetValue("ChangedMoreThan", out var cm)
                        ? DateTime.ParseExact(cm, "yyyy-MM-dd HH:mm", System.Globalization.CultureInfo.InvariantCulture) : null;
                    var found = _tasks.Values.Where(t => statusIds.Contains(t.StatusId) && (since is null || t.Changed >= since));
                    rows = (q.GetValueOrDefault("sort") == "Changed asc, Id asc" && !_ignoreSort
                        ? _tiesDescending ? found.OrderBy(t => t.Changed).ThenByDescending(t => t.Id) : found.OrderBy(t => t.Changed).ThenBy(t => t.Id)
                        : found.OrderByDescending(t => t.Changed).ThenByDescending(t => t.Id)).ToList();
                }
                if (rows.FindIndex(t => t.Id == _repeatRow) is >= 0 and var at)
                    rows.Insert(at + 1, rows[at]);
                if (_brokenPaging)
                    return page == 1 ? (200, List(rows, rows.Count + 300, 1, 2)) : (503, "{\"Message\":\"Service Unavailable\"}");
                return (200, List(rows.Skip((page - 1) * size).Take(size), rows.Count, page, Math.Max(1, (rows.Count + size - 1) / size)));
            }
            if (target.StartsWith("/api/task/") && int.TryParse(target["/api/task/".Length..].Split('?')[0], out var id))
                return !_tasks.TryGetValue(id, out var t) ? (404, "{}") : t.CardFails ? (500, "{\"Message\":\"Internal Server Error\"}")
                    : (200, $"{{\"Task\":{Row(t, withChanged: true)},\"Statuses\":{Statuses}}}");
            if (target.StartsWith("/api/tasklifetime?") && int.TryParse(Query(target).GetValueOrDefault("taskid"), out var lid))
                return !_tasks.TryGetValue(lid, out var l) ? (404, "{}") : l.EventsRefused ? (403, "{}")
                    : l.LifetimeFails > 0 ? (l.LifetimeFails, "{\"Message\":\"сбой\"}") : (200, Lifetime(l));
            _unexpected.Add(target);
            return (404, "{}");
        }
    }

    private static Dictionary<string, string> Query(string target)
    {
        var q = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var at = target.IndexOf('?');
        if (at < 0) return q;
        foreach (var pair in target[(at + 1)..].Split('&', StringSplitOptions.RemoveEmptyEntries))
        {
            var kv = pair.Split('=', 2);
            q[kv[0]] = kv.Length > 1 ? Uri.UnescapeDataString(kv[1]) : "";
        }
        return q;
    }

    private static string J(string? s) => JsonSerializer.Serialize(s);

    /// <summary>Строка заявки, как её отдаёт сервер: номера исполнителей — строкой через «, », как в живой карточке.</summary>
    private static string Row(FakeTask t, bool withChanged) =>
        $"{{\"Id\":{t.Id},\"Name\":{J(t.Name)},\"StatusId\":{t.StatusId},\"Creator\":{J(t.Creator)},"
        + (t.NoIds ? "" : $"\"CreatorId\":{t.CreatorId},\"ExecutorIds\":{J(string.Join(", ", t.ExecutorIdList ?? (t.Mine ? new[] { MeId } : new[] { OtherId })))},")
        + (t.NoPhone ? "" : $"\"CreatorPhone\":{J(Phone)},")
        + (t.Description is null ? "" : $"\"Description\":{J(t.Description)},")
        + $"\"Executors\":{J(t.Executors)},\"ExecutorGroup\":{J(Group)},\"Created\":\"2026-09-01T10:00:00\""
        + (withChanged ? $",\"Changed\":\"{t.Changed:yyyy-MM-ddTHH:mm:ss.fffffff}\"" : "") + "}";

    private string List(IEnumerable<FakeTask> rows, int count, int page, int pages) =>
        $"{{\"Tasks\":[{string.Join(",", rows.Select(r => Row(r, !_noChanged)))}],\"Statuses\":{Statuses},"
        + $"\"Paginator\":{{\"Count\":{count},\"Page\":{page},\"PageCount\":{pages}}}}}";

    /// <summary>Переписка, свежие сверху (lastcommentsontop=true), с блоком статусов — названия по StatusId записи.</summary>
    private static string Lifetime(FakeTask t) =>
        "{\"TaskLifetimeList\":{\"TaskLifetimes\":[" + string.Join(",", t.Events.OrderByDescending(e => e.Date).Select(e =>
            $"{{\"Date\":\"{e.Date:s}\",\"Editor\":{J(e.Editor)},\"EditorId\":{e.EditorId},"
            + (e.Text is null ? "" : $"\"Comments\":{J(e.Text)},")
            + $"\"IsPublic\":{(e.IsPublic ? "true" : "false")},\"StatusId\":{e.StatusId}}}"))
        + $"],\"Statuses\":{Statuses},\"Paginator\":{{\"Page\":1,\"PageCount\":{(t.MoreEvents ? 2 : 1)}}}}}}}";
}
