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
    ('the revision to the second', F,
     '=> changed.UtcTicks.ToString(CultureInfo.InvariantCulture);',
     '=> (changed.UtcTicks / TimeSpan.TicksPerSecond).ToString(CultureInfo.InvariantCulture);'),
    ('the revision in local ticks', F,
     '=> changed.UtcTicks.ToString(CultureInfo.InvariantCulture);',
     '=> changed.Ticks.ToString(CultureInfo.InvariantCulture);'),
    ('a zone-less date read as local', F,
     'DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.None, out date);',
     'DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out date);'),
]
