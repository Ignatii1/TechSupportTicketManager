"""Пароли скрыты (Secrets.Mask) — в выгрузке, запросе агенту и ответах Claude через буфер; ловят самопроверки Secrets и
ClaudeRelay."""

AREAS = ['secrets', 'relay']

S = 'TicketBoard/Services/KnowledgeExport.Secrets.cs'
RELAY = 'TicketBoard/Services/ClaudeRelay.cs'

MUTANTS = [
    ('a value on the next line kept', S,
     r'[^\S\r\n]*(?:\r?\n[^\S\r\n]*)?)\S+",',
     r'[^\S\r\n]*)\S+",'),
    ('a non-breaking space ends the key', S,
     r'[:=][^\S\r\n]*(?:',
     r'[:=][ \t]*(?:'),
    ('a non-breaking space on the next line', S,
     r'(?:\r?\n[^\S\r\n]*)?)',
     r'(?:\r?\n[ \t]*)?)'),
    ('a blank line crossed', S,
     r'(?:\r?\n[^\S\r\n]*)?)\S+",',
     r'\s*)\S+",'),
    ('no colon needed', S,
     r'{0,20}?[:=][^\S\r\n]*',
     r'{0,20}?[:=]?[^\S\r\n]*'),
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
