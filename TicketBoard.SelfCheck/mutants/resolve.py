"""Условия поиска → запрос (TicketSearch) — ловит его SelfCheck."""

AREAS = ['resolve']

TICKETSEARCH = 'TicketBoard/Services/TicketSearch.cs'

MUTANTS = [
    ('children by path with pipes', TICKETSEARCH,
     '("|" + path).Contains($"|{id}|")',
     '("|" + path).Contains($"{id}")'),
    ('span end is end of day', TICKETSEARCH,
     'b?.Date.AddDays(1)), "");',
     'b?.Date), "");'),
    ('too many users is an error', TICKETSEARCH,
     'if (users.Count > MaxUsersPerName || total > MaxUsersPerName)',
     'if (false)'),
    ('no user is an error', TICKETSEARCH,
     'if (users.Count == 0) return',
     'if (false) return'),
    ('closed = flags or names', TICKETSEARCH,
     's.IsFixed || s.IsFinal || closedNames.Contains(s.Name)',
     's.IsFixed || s.IsFinal'),
]
