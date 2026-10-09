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
    public const string Phone = "+7 900 000-00-00";
    public const string Group = "Вторая линия";
    public const int Open = 31, Working = 32, Fixed = 29, Final = 30, Cancelled = 34;
    // «Выполнена» закрыта признаком IsFixed, «Закрыта» — IsFinal, «Отменена» — только по названию из ClosedStatusNames
    private const string Statuses = """[{"Id":31,"Name":"Открыта","IsFixed":false,"IsFinal":false},{"Id":32,"Name":"В работе"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта","IsFinal":true},{"Id":34,"Name":"Отменена"}]""";

    /// <summary>Заявка. Mine — я в исполнителях (попадает в список «мои открытые»).</summary>
    public sealed class FakeTask(int id, string name)
    {
        public int Id { get; } = id;
        public string Name { get; set; } = name;
        public string? Description { get; set; }
        public int StatusId { get; set; } = Open;
        public bool Mine { get; set; } = true;
        public string Executors { get; set; } = MeName;
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
    private DateTime _clock = new(2026, 10, 1, 9, 0, 0);
    private int _failList;
    private bool _brokenPaging;

    public BoardServer() => (_listener, Port) = FakeIntraservice.Start(Respond);

    public int Port { get; }
    public string Url => $"http://127.0.0.1:{Port}";

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
    public int Count(string prefix) { lock (_lock) return _asked.Count(a => a.Target.StartsWith(prefix)); }
    public string Last(string prefix) { lock (_lock) return _asked.LastOrDefault(a => a.Target.StartsWith(prefix)).Target ?? ""; }

    /// <summary>Через сколько после since (Stopwatch.GetTimestamp()) пришёл первый такой запрос; null — не приходил. Паузу
    /// перед запросом так видно и на медленной машине: задержка потока делает ответ только позже, а не раньше.</summary>
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
            if (target == "/api/taskstatus") return (200, Statuses);
            if (target.StartsWith("/api/task?"))
            {
                if (_failList > 0) return (_failList, "{\"Message\":\"Service Unavailable\"}");
                var q = Query(target);
                var page = int.Parse(q.GetValueOrDefault("page", "1"));
                var size = int.Parse(q.GetValueOrDefault("pagesize", "50"));
                var statusIds = q.GetValueOrDefault("StatusIds", "").Split(',', StringSplitOptions.RemoveEmptyEntries).Select(int.Parse).ToHashSet();
                var mine = q.GetValueOrDefault("ExecutorIds") == $"{MeId}";
                var rows = _tasks.Values.Where(t => mine && t.Mine && statusIds.Contains(t.StatusId))
                    .OrderByDescending(t => t.Changed).ThenByDescending(t => t.Id).ToList();
                if (_brokenPaging)
                    return page == 1 ? (200, List(rows, rows.Count + 300, 1, 2)) : (503, "{\"Message\":\"Service Unavailable\"}");
                return (200, List(rows.Skip((page - 1) * size).Take(size), rows.Count, page, Math.Max(1, (rows.Count + size - 1) / size)));
            }
            if (target.StartsWith("/api/task/") && int.TryParse(target["/api/task/".Length..].Split('?')[0], out var id))
                return _tasks.TryGetValue(id, out var t) ? (200, $"{{\"Task\":{Row(t)},\"Statuses\":{Statuses}}}") : (404, "{}");
            if (target.StartsWith("/api/tasklifetime?") && int.TryParse(Query(target).GetValueOrDefault("taskid"), out var lid))
                return !_tasks.TryGetValue(lid, out var l) ? (404, "{}") : l.EventsRefused ? (403, "{}") : (200, Lifetime(l));
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

    private static string Row(FakeTask t) =>
        $"{{\"Id\":{t.Id},\"Name\":{J(t.Name)},\"StatusId\":{t.StatusId},\"Creator\":{J(Requester)},"
        + (t.NoPhone ? "" : $"\"CreatorPhone\":{J(Phone)},")
        + (t.Description is null ? "" : $"\"Description\":{J(t.Description)},")
        + $"\"Executors\":{J(t.Executors)},\"ExecutorGroup\":{J(Group)},\"Created\":\"2026-09-01T10:00:00\",\"Changed\":\"{t.Changed:s}\"}}";

    private static string List(IEnumerable<FakeTask> rows, int count, int page, int pages) =>
        $"{{\"Tasks\":[{string.Join(",", rows.Select(Row))}],\"Statuses\":{Statuses},"
        + $"\"Paginator\":{{\"Count\":{count},\"Page\":{page},\"PageCount\":{pages}}}}}";

    /// <summary>Переписка, свежие сверху (lastcommentsontop=true), с блоком статусов — названия по StatusId записи.</summary>
    private static string Lifetime(FakeTask t) =>
        "{\"TaskLifetimeList\":{\"TaskLifetimes\":[" + string.Join(",", t.Events.OrderByDescending(e => e.Date).Select(e =>
            $"{{\"Date\":\"{e.Date:s}\",\"Editor\":{J(e.Editor)},\"EditorId\":{e.EditorId},"
            + (e.Text is null ? "" : $"\"Comments\":{J(e.Text)},")
            + $"\"IsPublic\":{(e.IsPublic ? "true" : "false")},\"StatusId\":{e.StatusId}}}"))
        + $"],\"Statuses\":{Statuses},\"Paginator\":{{\"Page\":1,\"PageCount\":{(t.MoreEvents ? 2 : 1)}}}}}}}";
}
