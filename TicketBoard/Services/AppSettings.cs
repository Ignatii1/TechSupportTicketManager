using System.IO;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TicketBoard.Services;

/// <summary>%APPDATA%\TicketBoard\settings.json — правится руками, читается при старте.</summary>
public sealed class AppSettings
{
    /// <summary>Глобальный хоткей окна быстрого добавления. Формат: Ctrl+Shift+Space, Win+Alt+T, ...</summary>
    public string Hotkey { get; set; } = "Win+Shift+Space";

    /// <summary>Регулярка, вытаскивающая номер заявки из ссылки или текста. Группа 1 — номер.
    /// По умолчанию: …/Task/View/702180, «#702180», «№ 702180». Если не совпало — берётся последнее число в ссылке.</summary>
    public string IntraserviceIdPattern { get; set; } = @"(?:Task/View/|[#№]\s?)(\d{4,8})";

    /// <summary>Для API (потом): базовый адрес и токен.</summary>
    public string IntraserviceBaseUrl { get; set; } = "";
    public string IntraserviceApiToken { get; set; } = "";

    public int HideDoneOlderThanDays { get; set; } = 7;
    public int WipLimit { get; set; } = 5;

    /// <summary>Дней в колонке, после которых заявка считается просроченной (красный). За день до этого — жёлтый. «Готово» не подсвечивается.</summary>
    public int OverdueDays { get; set; } = 3;

    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() }
    };

    public static string PathFor(string dir) => Path.Combine(dir, "settings.json");

    public static AppSettings Load(string dir)
    {
        var path = PathFor(dir);
        if (File.Exists(path))
        {
            try { return JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(path), Json) ?? new(); }
            catch { /* битый файл — берём дефолты, файл не трогаем */ }
        }
        var s = new AppSettings();
        try { s.Save(dir); } catch { }
        return s;
    }

    public void Save(string dir)
    {
        Directory.CreateDirectory(dir);
        File.WriteAllText(PathFor(dir), JsonSerializer.Serialize(this, Json));
    }
}
