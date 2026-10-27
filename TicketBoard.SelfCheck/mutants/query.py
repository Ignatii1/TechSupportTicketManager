"""Адрес списка заявок (TaskQuery) — ловит его SelfCheck."""

AREAS = ['query']

TASKQUERY = 'TicketBoard/Services/TaskQuery.cs'

MUTANTS = [
    ('date format needs time', TASKQUERY,
     'd.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture)',
     'd.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture)'),
    ('outside tolerance', TASKQUERY,
     'v.DateTime < from.AddDays(-1)',
     'v.DateTime < from'),
    ('archive flags', TASKQUERY,
     'if (IncludeArchived) url.Append("archive=true&inactive=true&");',
     ''),
    ('count=all sent again', TASKQUERY,
     'url.Append(detailed ? "include=status,service&" : "include=status&");',
     'url.Append(detailed ? "include=status,service&count=all&" : "include=status&");'),
    ('list not cut past the period', TASKQUERY,
     'DefaultSort => Changed.From is { } from && f.Changed is { } changed && changed.DateTime < from.AddDays(-1),',
     'DefaultSort => false,'),
    # 0.21.0: наблюдатель
    ('observer filter not sent', TASKQUERY,
     '        Ids("ObserverIds", ObserverIds);\n',
     ''),
    ('observers unrestricted', TASKQUERY,
     '&& ObserverIds is not { Count: > 0 } && Created.IsEmpty',
     '&& Created.IsEmpty'),
    ('observer guard off', TASKQUERY,
     '\n        || (ObserverIds is { Count: > 0 } wanted && f.ObserverIds is { } observers && !observers.Any(wanted.Contains));',
     ';'),
    ('a row without observers counted outside', TASKQUERY,
     'f.ObserverIds is { } observers && !observers',
     '(f.ObserverIds ?? Array.Empty<int>()) is var observers && !observers'),
    ('saved filter without observers', TASKQUERY,
     ',Closed,ObserverIds,Observers";',
     ',Closed";'),
    ('outside words ignore observers', TASKQUERY,
     '(Created.IsEmpty && Changed.IsEmpty, ObserverIds is { Count: > 0 }) switch',
     '(Created.IsEmpty && Changed.IsEmpty, false) switch'),
    ('outside words ignore the period', TASKQUERY,
     '(Created.IsEmpty && Changed.IsEmpty, ObserverIds is { Count: > 0 }) switch',
     '(true, ObserverIds is { Count: > 0 }) switch'),
]
