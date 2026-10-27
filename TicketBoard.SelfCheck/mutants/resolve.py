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
     's.ClosedBy(closedNames) == wantClosed',
     '(s.IsFixed || s.IsFinal) == wantClosed'),
    # 0.21.0: наблюдатель
    ('observer me ignored', TICKETSEARCH,
     '            if (f.ObserverMine) observers.Add(me.Id);\n',
     ''),
    ('observer me needs mine', TICKETSEARCH,
     'if (f.Mine || f.ObserverMine)',
     'if (f.Mine)'),
    ('observer name as executor', TICKETSEARCH,
     '"Наблюдатель", notes, ct);\n            if (peopleError.Length > 0) return (null, peopleError);\n            observers.AddRange(ids);',
     '"Наблюдатель", notes, ct);\n            if (peopleError.Length > 0) return (null, peopleError);\n            executors.AddRange(ids);'),
    ('observers not in the query', TICKETSEARCH,
     'ObserverIds: observers.Count > 0 ? observers.Distinct().ToList() : null);',
     'ObserverIds: null);'),
    ('both «я» not said', TICKETSEARCH,
     '        if (f.Mine && f.ObserverMine)\n            notes.Add(',
     '        if (false)\n            notes.Add('),
]
