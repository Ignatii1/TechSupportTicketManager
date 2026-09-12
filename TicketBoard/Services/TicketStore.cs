using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;
using TicketBoard.Models;

namespace TicketBoard.Services;

/// <summary>Хранит заявки в %APPDATA%\TicketBoard\tickets.json: атомарная запись + ежедневный бэкап.</summary>
public sealed class TicketStore
{
    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        Converters = { new JsonStringEnumConverter() }
    };

    public string DataDir { get; }
    public string FilePath => Path.Combine(DataDir, "tickets.json");
    public string BackupDir => Path.Combine(DataDir, "backups");

    public TicketStore(string dataDir)
    {
        DataDir = dataDir;
        Directory.CreateDirectory(DataDir);
    }

    public List<Ticket> Load()
    {
        if (!File.Exists(FilePath)) return new();
        try
        {
            return JsonSerializer.Deserialize<List<Ticket>>(File.ReadAllText(FilePath), Json) ?? new();
        }
        catch (JsonException)
        {
            // Битый файл откладываем в сторону, не затираем — чтобы можно было восстановить руками.
            File.Move(FilePath, FilePath + $".corrupt-{DateTime.Now:yyyyMMdd-HHmmss}", overwrite: true);
            return new();
        }
    }

    public void Save(IReadOnlyCollection<Ticket> tickets)
    {
        Directory.CreateDirectory(DataDir);
        BackupOncePerDay();

        var tmp = FilePath + ".tmp";
        File.WriteAllText(tmp, JsonSerializer.Serialize(tickets, Json));
        File.Move(tmp, FilePath, overwrite: true);
    }

    private void BackupOncePerDay()
    {
        if (!File.Exists(FilePath)) return;
        Directory.CreateDirectory(BackupDir);
        var today = Path.Combine(BackupDir, $"tickets-{DateTime.Now:yyyy-MM-dd}.json");
        if (File.Exists(today)) return;
        File.Copy(FilePath, today);

        // держим последние 30 бэкапов
        foreach (var old in Directory.GetFiles(BackupDir, "tickets-*.json").OrderByDescending(f => f).Skip(30))
            try { File.Delete(old); } catch { }
    }
}
