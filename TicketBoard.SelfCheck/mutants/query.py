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
]
