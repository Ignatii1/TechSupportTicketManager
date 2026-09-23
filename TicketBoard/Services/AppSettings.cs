using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TicketBoard.Services;

/// <summary>settings.json рядом с exe — правится руками, читается при старте.</summary>
public sealed class AppSettings
{
    /// <summary>Глобальный хоткей окна быстрого добавления. Формат: Ctrl+Shift+Space, Win+Alt+T, ...</summary>
    public string Hotkey { get; set; } = "Ctrl+Alt+Space"; // не Win+Shift+Space — это смена раскладки в Windows

    /// <summary>Регулярка, вытаскивающая номер заявки из ссылки или текста. Группа 1 — номер.
    /// По умолчанию: …/Task/View/702180, «#702180», «№ 702180». Если не совпало — берётся последнее число в ссылке.
    /// Поле целиком из 4–8 цифр считается номером мимо этой регулярки.</summary>
    public string IntraserviceIdPattern { get; set; } = @"(?:Task/View/|[#№]\s?)(\d{4,8})";

    /// <summary>API Интрасервиса: адрес сайта (https://helpdesk.company.ru) и логин/пароль пользователя — у API только базовая авторизация.
    /// Пусто — API выключен.</summary>
    public string IntraserviceBaseUrl { get; set; } = "";
    public string IntraserviceLogin { get; set; } = "";

    /// <summary>Пароль в памяти. В файл не пишется — только зашифрованным, см. IntraservicePasswordProtected.</summary>
    [JsonIgnore] public string IntraservicePassword { get; set; } = "";

    /// <summary>Пароль, зашифрованный DPAPI под текущего пользователя Windows (base64).
    /// Не расшифровался (другой пользователь / другая машина) — пароль пустой, API выключен.</summary>
    public string IntraservicePasswordProtected
    {
        get => Protect(IntraservicePassword);
        set => IntraservicePassword = Unprotect(value);
    }

    public int HideDoneOlderThanDays { get; set; } = 7;
    public int WipLimit { get; set; } = 5;

    /// <summary>Дней в колонке, после которых заявка считается просроченной (красный). За день до этого — жёлтый. «Готово» не подсвечивается.</summary>
    public int OverdueDays { get; set; } = 3;

    /// <summary>Названия статусов Интрасервиса, которые считаем закрытыми при импорте, сверх признаков
    /// «Заявка выполнена» и «Конечный». Правится руками в settings.json.</summary>
    public string[] ClosedStatusNames { get; set; } = { "Выполнена", "Ожидание ответа с автозакрытием", "Закрыта", "Отменена" };

    /// <summary>Отвечать Claude через буфер обмена (Services/ClaudeRelay.cs): скопированный блок «TB …» заменяется ответом.
    /// Выключено по умолчанию — без этого TicketBoard в буфер не заглядывает.</summary>
    public bool ClaudeRelayEnabled { get; set; }

    /// <summary>Ссылка на заявку в веб-интерфейсе Интрасервиса; без адреса — пусто.</summary>
    public string TicketUrl(int id)
    {
        var b = IntraserviceBaseUrl.Trim().TrimEnd('/');
        return b.Length > 0 ? $"{b}/Task/View/{id}" : "";
    }

    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() }
    };

    private static string Protect(string plain) => plain.Length == 0 ? ""
        : Convert.ToBase64String(ProtectedData.Protect(Encoding.UTF8.GetBytes(plain), null, DataProtectionScope.CurrentUser));

    private static string Unprotect(string? stored)
    {
        try
        {
            return string.IsNullOrEmpty(stored) ? ""
                : Encoding.UTF8.GetString(ProtectedData.Unprotect(Convert.FromBase64String(stored), null, DataProtectionScope.CurrentUser));
        }
        catch { return ""; }
    }

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
