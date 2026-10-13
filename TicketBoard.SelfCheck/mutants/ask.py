"""Запрос агенту «Спросить базу знаний» (KnowledgeAsk) — ловит его SelfCheck: эталон текста и обрезка."""

AREAS = ['ask']

ASK = 'TicketBoard/Services/KnowledgeExport.Ask.cs'

MUTANTS = [
    ('no link line', ASK,
     '(url.Length > 0 ? $"Ссылка: {url}\\n" : "")',
     '""'),
    ('a file in the base said to be missing', ASK,
     'var where = inBase ? $"В базе знаний: {file}" : ',
     'var where = '),
    ('a long ticket not cut', ASK,
     '            + Cut(ticket, MaxTicketChars);',
     '            + ticket;'),
    ('the cut keeps only the head', ASK,
     '…\\n\\n" + text[tail..];',
     '…\\n\\n";'),
    ('the head cut mid-line', ASK,
     "text.LastIndexOf('\\n', max * 2 / 3) is var h && h > 0 ? h + 1 : max * 2 / 3;",
     'max * 2 / 3;'),
    ('the tail cut mid-line', ASK,
     "text.IndexOf('\\n', text.Length - (max - head)) is var t && t >= 0 && t + 1 < text.Length ? t + 1 : text.Length - (max - head);",
     'text.Length - (max - head);'),
    ('the mark counts wrong', ASK,
     '[вырезано {tail - head} знаков',
     '[вырезано {tail} знаков'),
]
