using System.Diagnostics;
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

    /// <summary>Автообновление: раз в столько минут новые заявки на меня — во «Входящие», статусы — на карточки,
    /// о закрытых — уведомление (MainViewModel.AutoSync.cs). 0 — выключено.</summary>
    public int AutoSyncMinutes { get; set; } = 5;

    /// <summary>Служебное, правится само: номера, которые автообновление не добавляет на доску, — удалённые с доски
    /// и открытые на момент первого автообновления (прошлое приносит импорт). null — автообновление ещё не запускалось.</summary>
    public int[]? AutoSyncSkipIds { get; set; }

    /// <summary>Отвечать Claude через буфер обмена (Services/ClaudeRelay.cs): скопированный блок «TB …» заменяется ответом.
    /// Выключено по умолчанию — без этого TicketBoard в буфер не заглядывает.</summary>
    public bool ClaudeRelayEnabled { get; set; }

    /// <summary>Какая это учётная запись: адрес без «/» на конце и логин, без учёта регистра (null — из settings.json,
    /// поправленного руками). Сменилась — всё, что автообновление помнило о «моих» заявках, относится к прежней.</summary>
    [JsonIgnore] public string AccountKey =>
        $"{(IntraserviceBaseUrl ?? "").Trim().TrimEnd('/')}|{(IntraserviceLogin ?? "").Trim()}".ToLowerInvariant();

    /// <summary>Ссылка на заявку в веб-интерфейсе Интрасервиса; без адреса — пусто.</summary>
    public string TicketUrl(int id)
    {
        var b = IntraserviceBaseUrl.Trim().TrimEnd('/');
        return b.Length > 0 ? $"{b}/Task/View/{id}" : "";
    }

    private static readonly JsonSerializerOptions Json = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
        // файл правят руками — лишняя запятая или комментарий не повод сбрасывать настройки
        AllowTrailingCommas = true,
        ReadCommentHandling = JsonCommentHandling.Skip,
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

    /// <summary>Файл есть, но не прочитан — в памяти дефолты. Первая запись не затирает его, а отодвигает в .unread-….</summary>
    private bool _unread;

    /// <summary>Настройки из settings.json; нет файла — дефолты и новый файл. problem — почему взяты дефолты при
    /// существующем файле (для лога и уведомления); пусто — всё в порядке.</summary>
    public static AppSettings Load(string dir, out string problem)
    {
        problem = "";
        var path = PathFor(dir);
        if (File.Exists(path))
        {
            // в тексте проблемы суть — первой: уведомление режется с конца, а подробности всё равно в errors.log
            static AppSettings Unread() => new() { _unread = true };
            string text;
            try { text = ReadText(path); }
            catch (Exception ex)
            {
                problem = $"settings.json не прочитан — перезапустите приложение; пока настройки по умолчанию, файл не затирается. {ex.Message}";
                return Unread();
            }
            try { return JsonSerializer.Deserialize<AppSettings>(text, Json) ?? new(); }
            catch (JsonException ex)
            {
                // битый (опечатка при правке руками) — в сторону, как tickets.json: дефолты ниже иначе затёрли бы его
                // вместе с адресом, логином и паролем
                var aside = $"{path}.corrupt-{DateTime.Now:yyyyMMdd-HHmmss}";
                try { File.Move(path, aside, overwrite: true); }
                catch
                {
                    problem = $"settings.json испорчен — пока настройки по умолчанию, файл не затирается. {ex.Message}";
                    return Unread();
                }
                problem = $"settings.json испорчен — настройки сброшены, старый файл: {Path.GetFileName(aside)}. {ex.Message}";
            }
            catch (Exception ex)   // не разбор, а что-то ещё — не повод не запуститься
            {
                problem = $"settings.json не прочитан — пока настройки по умолчанию, файл не затирается. {ex.Message}";
                return Unread();
            }
        }
        var s = new AppSettings();
        try { s.Save(dir); } catch { }
        return s;
    }

    /// <summary>Антивирус или синхронизация держат файл в момент запуска — подождём немного, а не сбросим настройки.</summary>
    private static string ReadText(string path)
    {
        for (var attempt = 1; ; attempt++)
            try { return File.ReadAllText(path); }
            catch (IOException) when (attempt < 3) { Thread.Sleep(200); }
    }

    /// <summary>Через временный файл и замену, как tickets.json: settings.json пишет и автообновление, а сбой посреди
    /// записи оставил бы обрезанный файл — Load молча взял бы дефолты, и адрес, логин и пароль пропали бы.</summary>
    public void Save(string dir)
    {
        Directory.CreateDirectory(dir);
        var path = PathFor(dir);
        if (_unread && File.Exists(path)) File.Move(path, $"{path}.unread-{DateTime.Now:yyyyMMdd-HHmmss}", overwrite: true);
        _unread = false;
        File.WriteAllText(path + ".tmp", JsonSerializer.Serialize(this, Json));
        File.Move(path + ".tmp", path, overwrite: true);
    }

    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var dir = Path.Combine(Path.GetTempPath(), $"tb-selfcheck-{Guid.NewGuid():N}");
        Directory.CreateDirectory(dir);
        try
        {
            // битый файл не затирается дефолтами: уходит в .corrupt-…, на его месте — файл с дефолтами
            File.WriteAllText(PathFor(dir), "{ \"IntraserviceBaseUrl\": \"https://hd\", ");
            var s = Load(dir, out var problem);
            Debug.Assert(s.IntraserviceBaseUrl == "" && problem.Contains("corrupt-")
                && Directory.GetFiles(dir, "settings.json.corrupt-*").Length == 1);
            Debug.Assert(Load(dir, out problem).AutoSyncSkipIds is null && problem == "");   // и файл с дефолтами читается

            // правка руками: лишняя запятая и комментарий — не порча
            File.WriteAllText(PathFor(dir), "{\n  // мой сервер\n  \"IntraserviceBaseUrl\": \"https://hd\",\n  \"WipLimit\": 7,\n}");
            Debug.Assert(Load(dir, out problem) is { IntraserviceBaseUrl: "https://hd", WipLimit: 7 } && problem == "");

            // запись через временный файл: читается обратно, временного не остаётся
            s.IntraserviceBaseUrl = "https://hd";
            s.AutoSyncSkipIds = new[] { 5 };
            s.Save(dir);
            var back = Load(dir, out problem);
            Debug.Assert(back.IntraserviceBaseUrl == "https://hd" && back.AutoSyncSkipIds is [5] && !File.Exists(PathFor(dir) + ".tmp"));
        }
        finally { Directory.Delete(dir, recursive: true); }
    }
}
