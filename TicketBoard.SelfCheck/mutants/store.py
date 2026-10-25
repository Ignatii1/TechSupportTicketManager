"""Хранилище заявок tickets.json: запись, бэкапы, битый файл, формат — ловит StoreCheck."""

AREAS = ['store']

TICKET = 'TicketBoard/Models/Ticket.cs'
TICKETSTORE = 'TicketBoard/Services/TicketStore.cs'

MUTANTS = [
    # этап 5б: поля карточки и вложения — в tickets.json под своими именами
    ('creator company renamed', TICKET,
     'private string? _creatorCompany;',
     'private string? _creatorOrganization;'),
    ('files renamed', TICKET,
     'private IReadOnlyList<TicketFile>? _files;',
     'private IReadOnlyList<TicketFile>? _attachments;'),
    ('a file name renamed', TICKET,
     'public sealed record TicketFile(int Id, string? Name);',
     'public sealed record TicketFile(int Id, string? FileName);'),
    ('no backup', TICKETSTORE,
     '        BackupOncePerDay();\n',
     ''),
    ('backup rewritten every save', TICKETSTORE,
     '        if (File.Exists(today)) return;\n        File.Copy(FilePath, today);',
     '        File.Copy(FilePath, today, overwrite: true);'),
    ('all backups kept', TICKETSTORE,
     '.Skip(30)',
     '.Skip(300)'),
    ('corrupt file deleted', TICKETSTORE,
     'File.Move(FilePath, FilePath + $".corrupt-{DateTime.Now:yyyyMMdd-HHmmss}", overwrite: true);',
     'File.Delete(FilePath);'),
    ('enums as numbers', TICKETSTORE,
     'Converters = { new JsonStringEnumConverter() }',
     'Converters = { }'),
    ('nulls written', TICKETSTORE,
     'DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,',
     ''),
    ('persisted field renamed', TICKET,
     '[ObservableProperty] private string? _executorGroup;',
     '[ObservableProperty] [property: System.Text.Json.Serialization.JsonPropertyName("Group")] private string? _executorGroup;'),
    ('in-memory field persisted', TICKET,
     '[JsonIgnore] public DateTimeOffset? ServerChanged',
     'public DateTimeOffset? ServerChanged'),
    ('backup on every save', TICKETSTORE,
     'var today = Path.Combine(BackupDir, $"tickets-{DateTime.Now:yyyy-MM-dd}.json");',
     'var today = Path.Combine(BackupDir, $"tickets-{DateTime.Now:yyyy-MM-dd-HHmmssfffffff}.json");'),
    ('note field renamed', TICKET,
     'public string Text { get; init; } = "";',
     '[System.Text.Json.Serialization.JsonPropertyName("Body")] public string Text { get; init; } = "";'),
]
