using System.Text.RegularExpressions;
using CommunityToolkit.Mvvm.ComponentModel;
using TicketBoard.Services;

namespace TicketBoard.ViewModels;

/// <summary>Окно настроек: поля — строками, ошибка — сразу под полем. Сохраняет в тот же экземпляр AppSettings.</summary>
public sealed partial class SettingsViewModel : ObservableObject
{
    private readonly AppSettings _settings;

    [ObservableProperty] private string _hotkey = "";
    [ObservableProperty] private string _hotkeyError = "";
    [ObservableProperty] private string _idPattern = "";
    [ObservableProperty] private string _idPatternError = "";
    [ObservableProperty] private string _hideDoneDays = "";
    [ObservableProperty] private string _hideDoneDaysError = "";
    [ObservableProperty] private string _wipLimit = "";
    [ObservableProperty] private string _wipLimitError = "";
    [ObservableProperty] private string _overdueDays = "";
    [ObservableProperty] private string _overdueDaysError = "";
    [ObservableProperty] private string _autoSyncMinutes = "";
    [ObservableProperty] private string _autoSyncMinutesError = "";
    [ObservableProperty] private string _baseUrl = "";
    [ObservableProperty] private string _baseUrlError = "";
    [ObservableProperty] private string _baseUrlWarning = "";
    [ObservableProperty] private string _login = "";
    [ObservableProperty] private string _checkResult = "";
    [ObservableProperty] private string _saveError = "";
    [ObservableProperty] private bool _relayEnabled;

    public string PasswordPlaceholder => _settings.IntraservicePassword.Length > 0 ? "сохранён — пусто, чтобы не менять" : "Пароль";

    public SettingsViewModel(AppSettings settings)
    {
        _settings = settings;
        Hotkey = settings.Hotkey;
        IdPattern = settings.IntraserviceIdPattern;
        HideDoneDays = settings.HideDoneOlderThanDays.ToString();
        WipLimit = settings.WipLimit.ToString();
        OverdueDays = settings.OverdueDays.ToString();
        AutoSyncMinutes = settings.AutoSyncMinutes.ToString();
        BaseUrl = settings.IntraserviceBaseUrl;
        Login = settings.IntraserviceLogin;
        RelayEnabled = settings.ClaudeRelayEnabled;
    }

    // ---------- проверка ----------

    partial void OnHotkeyChanged(string value) =>
        HotkeyError = HotkeyService.TryParse(value, out _, out _) ? "" : "Не понял хоткей. Пример: Ctrl+Alt+Space, Ctrl+Alt+T";

    partial void OnIdPatternChanged(string value)
    {
        try { IdPatternError = new Regex(value).GetGroupNumbers().Length > 1 ? "" : @"Нужна группа 1 с номером, например (\d{4,8})"; }
        catch (ArgumentException ex) { IdPatternError = ex.Message; }
    }

    partial void OnHideDoneDaysChanged(string value) => HideDoneDaysError = RangeError(value, 0, 365);
    partial void OnWipLimitChanged(string value) => WipLimitError = RangeError(value, 1, 50);
    partial void OnOverdueDaysChanged(string value) => OverdueDaysError = RangeError(value, 1, 90);
    partial void OnAutoSyncMinutesChanged(string value) => AutoSyncMinutesError = RangeError(value, 0, 120);

    partial void OnBaseUrlChanged(string value)
    {
        var url = value.Trim();
        BaseUrlError = url.Length == 0 || HttpIntraserviceClient.IsValidUrl(url) ? "" : "Нужен полный адрес: https://helpdesk.company.ru";
        BaseUrlWarning = BaseUrlError.Length == 0 && HttpIntraserviceClient.IsHttp(url) ? "По http пароль уходит открытым текстом — лучше https" : "";
    }

    private static string RangeError(string text, int min, int max) =>
        int.TryParse(text.Trim(), out var n) && n >= min && n <= max ? "" : $"Целое число от {min} до {max}";

    public bool IsValid =>
        (HotkeyError + IdPatternError + HideDoneDaysError + WipLimitError + OverdueDaysError + AutoSyncMinutesError + BaseUrlError).Length == 0;

    // ---------- действия ----------

    /// <summary>Записать в настройки и settings.json. Пустой newPassword — пароль не меняется.</summary>
    public void Save(string newPassword)
    {
        _settings.Hotkey = Hotkey.Trim();
        _settings.IntraserviceIdPattern = IdPattern;
        _settings.HideDoneOlderThanDays = int.Parse(HideDoneDays.Trim());
        _settings.WipLimit = int.Parse(WipLimit.Trim());
        _settings.OverdueDays = int.Parse(OverdueDays.Trim());
        _settings.AutoSyncMinutes = int.Parse(AutoSyncMinutes.Trim());
        _settings.IntraserviceBaseUrl = BaseUrl.Trim();   // сменили сервер или логин — память автообновления
        _settings.IntraserviceLogin = Login.Trim();       // сбросит MainViewModel.ApplySettings (AutoSyncAccount)
        if (newPassword.Length > 0) _settings.IntraservicePassword = newPassword;
        _settings.ClaudeRelayEnabled = RelayEnabled;
        _settings.Save(App.DataDir);
    }

    /// <summary>Проверить адрес и логин/пароль из формы (пароль — введённый или сохранённый).</summary>
    public async Task CheckAsync(string typedPassword)
    {
        var url = BaseUrl.Trim();
        var password = typedPassword.Length > 0 ? typedPassword : _settings.IntraservicePassword;
        if (url.Length == 0 || BaseUrlError.Length > 0) { CheckResult = "Укажите адрес Интрасервиса"; return; }
        if (Login.Trim().Length == 0 || password.Length == 0) { CheckResult = "Нужны логин и пароль"; return; }

        CheckResult = "Проверяю…";
        var error = await new HttpIntraserviceClient(url, Login.Trim(), password).CheckAsync();
        CheckResult = error.Length == 0 ? "Подключение есть" : $"Не удалось: {error}";
    }
}
