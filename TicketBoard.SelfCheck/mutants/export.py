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
    ('vanished counted as current', KNOWLEDGEEXPORT,
     'Current = job.Current.Except(vanished).ToHashSet(),',
     'Current = job.Current.ToHashSet(),'),
    ('per-ticket error not kept', KNOWLEDGEEXPORT,
     '            Errors[id] = error;\n',
     ''),
    ('write failure without its error', KNOWLEDGEEXPORT,
     '            Errors[id] = $"файл не записан: {message}";\n',
     ''),
    ('vanished without its error', KNOWLEDGEEXPORT,
     'Errors = job.Errors.Concat(vanished.Select(id => KeyValuePair.Create(id, "файл записан, но пропал с диска — похоже, его удалил антивирус")))',
     'Errors = job.Errors.Concat(Array.Empty<int>().Select(id => KeyValuePair.Create(id, "")))'),
]
