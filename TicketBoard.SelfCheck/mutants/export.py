"""Выгрузка в базу знаний (KnowledgeExport) — ловит её SelfCheck против поддельного сервера."""

AREAS = ['export']

KNOWLEDGEEXPORT_FILES = 'TicketBoard/Services/KnowledgeExport.Files.cs'
KNOWLEDGEEXPORT = 'TicketBoard/Services/KnowledgeExport.cs'

MUTANTS = [
    ('file names not recognised', KNOWLEDGEEXPORT_FILES,
     'IdInName.Match(fileName) is { Success: true } m',
     'IdInName.Match(fileName) is { Success: true, Index: > 0 } m'),
    ('tmp cleanup', KNOWLEDGEEXPORT_FILES,
     'TryDelete(tmp);',
     ''),
    ('card merged into the file', KNOWLEDGEEXPORT,
     'Format(WithDetails(row, task), events',
     'Format(row, events'),
    ('card failure not reported', KNOWLEDGEEXPORT,
     'if (details.Task is not { } task) return ("", details.Error);',
     'if (details.Task is not { } task) return ("", "");'),
    ('per-field merge', KNOWLEDGEEXPORT,
     'Extra = new(card?.Service ?? row?.Service, card?.Type ?? row?.Type, card?.Categories ?? row?.Categories,\n                card?.Resolved ?? row?.Resolved),',
     'Extra = card ?? row,'),
    ('export skips outside rows', KNOWLEDGEEXPORT,
     'if (listQuery.Outside(f)) { outside++; continue; }',
     'if (false) { outside++; continue; }'),
]
