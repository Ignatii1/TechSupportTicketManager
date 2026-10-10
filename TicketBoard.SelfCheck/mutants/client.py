"""Разбор ответов Интрасервиса (HttpIntraserviceClient) — ловят образцы в его SelfCheck."""

AREAS = ['client']

HTTPINTRASERVICECLIENT_PARSE = 'TicketBoard/Services/HttpIntraserviceClient.Parse.cs'

MUTANTS = [
    ('paged needs numbers', HTTPINTRASERVICECLIENT_PARSE,
     'return (events, pages is { } p && p.Page < p.Pages, pages is not null);',
     'return (events, pages is { } p && p.Page < p.Pages, Paginator(u.Blocks) is not null);'),
    ('Type, not TypeName', HTTPINTRASERVICECLIENT_PARSE,
     'Field(t, "Type"), Names(t, "Categories")',
     'Field(t, "TypeName"), Names(t, "Categories")'),
    ('service name from block', HTTPINTRASERVICECLIENT_PARSE,
     'services?.GetValueOrDefault(serviceId)',
     'null'),
    ('refs skip nameless rows', HTTPINTRASERVICECLIENT_PARSE,
     'Str(r, "Name")?.Trim() is { Length: > 0 })',
     'Str(r, "Name")?.Trim() is not null)'),
    ('executor ids only as array', HTTPINTRASERVICECLIENT_PARSE,
     "{ ValueKind: JsonValueKind.String } v => v.GetString()!.Split(',', ';'),\n            { ValueKind: JsonValueKind.Array } a => a.EnumerateArray().Select(i => i.ValueKind switch\n            {\n                JsonValueKind.Number",
     "{ ValueKind: JsonValueKind.Array } a => a.EnumerateArray().Select(i => i.ValueKind switch\n            {\n                JsonValueKind.Number"),
    ('creator id not read', HTTPINTRASERVICECLIENT_PARSE,
     'CreatorId: Int(t, "CreatorId"), ExecutorIds',
     'CreatorId: null, ExecutorIds'),
]
