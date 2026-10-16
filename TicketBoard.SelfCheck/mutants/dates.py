"""Даты для программ (MachineDate): один вид в свойствах файла и в tickets.csv — ловит самопроверка MachineDate."""

AREAS = ['dates']

F = 'TicketBoard/Services/KnowledgeExport.Format.cs'

MUTANTS = [
    ('fractions of a second kept', F,
     '''date.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)''',
     '''date.ToString("yyyy-MM-dd'T'HH:mm:ss.FFFFFFFzzz", CultureInfo.InvariantCulture)'''),
    ('the offset dropped', F,
     '''date.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)''',
     '''date.ToString("yyyy-MM-dd'T'HH:mm:ss", CultureInfo.InvariantCulture)'''),
    ('converted to UTC', F,
     '''date.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)''',
     '''date.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)'''),
    ('a space instead of T', F,
     '''date.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)''',
     '''date.ToString("yyyy-MM-dd HH:mm:sszzz", CultureInfo.InvariantCulture)'''),
    ('seconds rounded, not cut', F,
     '''date.ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)''',
     '''date.AddMilliseconds(500).ToString("yyyy-MM-dd'T'HH:mm:sszzz", CultureInfo.InvariantCulture)'''),
    ('the same tick, not the same second', F,
     'public static bool Same(DateTimeOffset a, DateTimeOffset b) => Second(a) == Second(b);',
     'public static bool Same(DateTimeOffset a, DateTimeOffset b) => a.UtcTicks == b.UtcTicks;'),
    ('the same minute', F,
     'date.UtcTicks / TimeSpan.TicksPerSecond;',
     'date.UtcTicks / TimeSpan.TicksPerMinute;'),
    ('local seconds compared across offsets', F,
     'date.UtcTicks / TimeSpan.TicksPerSecond;',
     'date.Ticks / TimeSpan.TicksPerSecond;'),
]
