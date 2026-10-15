"""Пароли скрыты (Secrets.Mask) — в выгрузке, запросе агенту и ответах Claude через буфер; ловят самопроверки Secrets и
ClaudeRelay."""

AREAS = ['secrets', 'relay']

S = 'TicketBoard/Services/KnowledgeExport.Secrets.cs'
RELAY = 'TicketBoard/Services/ClaudeRelay.cs'

MUTANTS = [
    ('a value on the next line kept', S,
     r'[ \t]*(?:\r?\n[ \t]*)?)\S+",',
     r'[ \t]*)\S+",'),
    ('a blank line crossed', S,
     r'(?:\r?\n[ \t]*)?)\S+",',
     r'\s*)\S+",'),
    ('no colon needed', S,
     r'{0,20}?[:=][ \t]*',
     r'{0,20}?[:=]?[ \t]*'),
    ('passport taken for pass', S,
     r'|pin)\b[^:=',
     r'|pin)[^:='),
    ('punctuation crossed', S,
     r'[^:=\n.,;!?]{0,20}?',
     r'[^:=\n]{0,20}?'),
    ('case matters', S,
     'RegexOptions.IgnoreCase | RegexOptions.CultureInvariant',
     'RegexOptions.CultureInvariant'),
    ('relay answers not masked', RELAY,
     'var answer = Secrets.Mask(sb.ToString());',
     'var answer = sb.ToString();'),
]
