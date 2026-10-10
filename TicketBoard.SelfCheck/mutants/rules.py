"""Правила автообновления и чтения страниц (AutoSyncRules) — ловит их SelfCheck."""

AREAS = ['rules']

AUTOSYNCRULES = 'TicketBoard/Services/AutoSyncRules.cs'

MUTANTS = [
    ('cap early return', AUTOSYNCRULES,
     '            if (rows.Count >= maxRows) return (rows, total, false, "");   // набрали сколько просили — следующая страница не нужна\n',
     ''),
    ('dedupe', AUTOSYNCRULES,
     '                if (!seen.Add(f.Id)) continue;',
     '                seen.Add(f.Id);'),
    ('name normalization', AUTOSYNCRULES,
     'return string.Equals(Norm(a), Norm(b), StringComparison.OrdinalIgnoreCase);',
     'return string.Equals(a, b, StringComparison.OrdinalIgnoreCase);'),
    ('unknown requester decides nothing', AUTOSYNCRULES,
     'string.IsNullOrWhiteSpace(requester) ? null : fresh',
     'string.IsNullOrWhiteSpace(requester) ? fresh.FirstOrDefault() : fresh'),
]
