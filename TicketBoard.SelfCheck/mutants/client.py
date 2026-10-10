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
]
