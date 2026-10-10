using TicketBoard.Services;
using TicketBoard.ViewModels;

namespace TicketBoard.SelfCheck;

/// <summary>Самопроверка окна настроек (SettingsViewModel): поля из настроек, ошибки под полями (числа в границах,
/// регулярка с группой номера, полный адрес, предупреждение про http), «можно сохранить», запись в settings.json, подсказка
/// в поле пароля и «Проверить» против поддельных серверов. Сам разбор хоткея — на типах WPF (HotkeyService.TryParse):
/// здесь вместо него заглушка с ответом от проверки, и проверяется окно вокруг разбора, а не разбор. Пароль в файл
/// пишется через DPAPI — его запись здесь не проверить (Linux), только «пусто — не меняется».</summary>
internal static class SettingsCheck
{
    public static void Run() => SingleThread.Run(RunAsync);

    private static async Task RunAsync(SingleThread pump)
    {
        var checks = new CheckSet("настройки");
        void Check(string name, bool ok, Func<string>? details = null) => checks.Check(name, ok, details);

        // «Проверить» ходит в api/taskstatus: этот сервер отвечает как надо, второй — 401, третий — не API
        var (ok, okPort) = FakeIntraservice.Start(t => t == "/api/taskstatus" ? (200, """[{"Id":31,"Name":"Открыта"}]""") : (404, "{}"));
        var (deny, denyPort) = FakeIntraservice.Start(t => (401, ""));
        var (page, pagePort) = FakeIntraservice.Start(t => (200, "<html><body>Вход в систему</body></html>"));
        var parsesWas = HotkeyService.Parses;
        using (var data = new TempDataDir("tb-settings"))
        {
            try
            {
                var settings = new AppSettings { IntraserviceBaseUrl = "https://hd.example.ru", IntraserviceLogin = "me", WipLimit = 7, ClaudeRelayEnabled = true };
                var vm = new SettingsViewModel(settings);
                pump.Watch(vm);
                Check("поля — из настроек, ошибок нет", vm is
                    {
                        Hotkey: "Ctrl+Alt+Space", IdPattern: @"(?:Task/View/|[#№]\s?)(\d{4,8})", HideDoneDays: "7", WipLimit: "7", OverdueDays: "3",
                        AutoSyncMinutes: "5", BaseUrl: "https://hd.example.ru", Login: "me", RelayEnabled: true, IsValid: true,
                        HotkeyError: "", IdPatternError: "", BaseUrlError: "", BaseUrlWarning: "",
                    });

                // числа: в своих границах, пробелы по краям не мешают
                foreach (var (set, error, value, expected) in new (Action<string>, Func<string>, string, string)[]
                {
                    (v => vm.WipLimit = v, () => vm.WipLimitError, "0", "Целое число от 1 до 50"),
                    (v => vm.WipLimit = v, () => vm.WipLimitError, "51", "Целое число от 1 до 50"),
                    (v => vm.WipLimit = v, () => vm.WipLimitError, " 50 ", ""),
                    (v => vm.HideDoneDays = v, () => vm.HideDoneDaysError, "0", ""),
                    (v => vm.HideDoneDays = v, () => vm.HideDoneDaysError, "366", "Целое число от 0 до 365"),
                    (v => vm.OverdueDays = v, () => vm.OverdueDaysError, "0", "Целое число от 1 до 90"),
                    (v => vm.OverdueDays = v, () => vm.OverdueDaysError, "три", "Целое число от 1 до 90"),
                    (v => vm.AutoSyncMinutes = v, () => vm.AutoSyncMinutesError, "0", ""),
                    (v => vm.AutoSyncMinutes = v, () => vm.AutoSyncMinutesError, "121", "Целое число от 0 до 120"),
                })
                {
                    set(value);
                    Check($"«{value}» → «{expected}»", error() == expected, () => error());
                }
                Check("ошибка в числе — сохранить нельзя", !vm.IsValid);
                (vm.WipLimit, vm.HideDoneDays, vm.OverdueDays, vm.AutoSyncMinutes) = ("8", "14", "4", "10");
                Check("числа поправили — можно", vm.IsValid);

                // регулярка номера: должна разбираться и иметь группу с номером
                vm.IdPattern = "(";
                Check("кривая регулярка — текст ошибки разбора", vm.IdPatternError.Length > 0 && !vm.IsValid);
                vm.IdPattern = @"\d{4,8}";
                Check("без группы — что нужно", vm.IdPatternError == @"Нужна группа 1 с номером, например (\d{4,8})");
                vm.IdPattern = @"Task/View/(\d+)";
                Check("с группой — без ошибки", vm.IdPatternError == "" && vm.IsValid);

                // адрес: полный; http — можно, но с предупреждением
                vm.BaseUrl = "helpdesk";
                Check("не адрес — что нужно", vm.BaseUrlError == "Нужен полный адрес: https://helpdesk.company.ru" && vm.BaseUrlWarning == "" && !vm.IsValid);
                vm.BaseUrl = "http://hd";
                Check("http — без ошибки, с предупреждением", vm.BaseUrlError == "" && vm.BaseUrlWarning == "По http пароль уходит открытым текстом — лучше https");
                vm.BaseUrl = " https://hd.example.ru/ ";
                Check("https — ни ошибки, ни предупреждения", vm.BaseUrlError == "" && vm.BaseUrlWarning == "" && vm.IsValid);

                // хоткей — что скажет разбор (здесь заглушка)
                HotkeyService.Parses = _ => false;
                vm.Hotkey = "Ctrl+Alt+Пробел";
                Check("хоткей не понят — пример под полем, сохранить нельзя",
                    vm.HotkeyError == "Не понял хоткей. Пример: Ctrl+Alt+Space, Ctrl+Alt+T" && !vm.IsValid);
                HotkeyService.Parses = _ => true;
                vm.Hotkey = " Ctrl+Alt+T ";
                Check("понят — без ошибки", vm.HotkeyError == "" && vm.IsValid);

                // запись: в тот же экземпляр и в settings.json
                (vm.Login, vm.RelayEnabled) = (" other ", false);
                vm.Save("");
                Check("сохранено в настройки — обрезано и разобрано", settings is
                    {
                        Hotkey: "Ctrl+Alt+T", IntraserviceIdPattern: @"Task/View/(\d+)", HideDoneOlderThanDays: 14, WipLimit: 8, OverdueDays: 4,
                        AutoSyncMinutes: 10, IntraserviceBaseUrl: "https://hd.example.ru/", IntraserviceLogin: "other", ClaudeRelayEnabled: false,
                    }, () => $"{settings.IntraserviceBaseUrl} · {settings.IntraserviceLogin} · {settings.Hotkey}");
                Check("…и в settings.json", AppSettings.Load(data.Path, out var problem) is
                    { WipLimit: 8, AutoSyncMinutes: 10, IntraserviceLogin: "other", Hotkey: "Ctrl+Alt+T", ClaudeRelayEnabled: false } && problem == "");

                // пароль: пустое поле — сохранённый не меняется, введённый — заменяет. В файл он идёт через DPAPI, которой
                // на Linux нет, — запись тогда падает, но правило применяется до неё (на Windows, в CI, запись проходит)
                void SaveWith(string password)
                {
                    try { vm.Save(password); }
                    catch (NotSupportedException) { /* DPAPI здесь нет */ }
                }
                settings.IntraservicePassword = "был";
                SaveWith("");
                Check("пустое поле пароля — сохранённый не меняется", settings.IntraservicePassword == "был");
                SaveWith("новый");
                Check("введённый пароль — заменяет", settings.IntraservicePassword == "новый");

                // подсказка в поле пароля
                Check("пароль сохранён — подсказка, что пустое поле его не меняет", vm.PasswordPlaceholder == "сохранён — пусто, чтобы не менять");
                Check("пароля нет — просто «Пароль»", new SettingsViewModel(new AppSettings()).PasswordPlaceholder == "Пароль");

                // «Проверить»
                var fresh = new SettingsViewModel(new AppSettings());
                pump.Watch(fresh);
                await fresh.CheckAsync("p");
                Check("адреса нет — просим его", fresh.CheckResult == "Укажите адрес Интрасервиса");
                fresh.BaseUrl = "helpdesk";
                await fresh.CheckAsync("p");
                Check("адрес кривой — тоже", fresh.CheckResult == "Укажите адрес Интрасервиса");
                fresh.BaseUrl = $"http://127.0.0.1:{okPort}";
                await fresh.CheckAsync("p");
                Check("логина нет — нужны логин и пароль", fresh.CheckResult == "Нужны логин и пароль");
                fresh.Login = "me";
                await fresh.CheckAsync("");
                Check("пароля нет ни в поле, ни сохранённого — тоже", fresh.CheckResult == "Нужны логин и пароль");
                var checking = fresh.CheckAsync("p");
                Check("пока проверяем — так и сказано", fresh.CheckResult == "Проверяю…");
                await checking;
                Check("сервер ответил — подключение есть", fresh.CheckResult == "Подключение есть", () => fresh.CheckResult);
                var saved = new AppSettings { IntraservicePassword = "сохранённый" };
                var withSaved = new SettingsViewModel(saved) { BaseUrl = $"http://127.0.0.1:{okPort}", Login = "me" };
                await withSaved.CheckAsync("");
                Check("поле пароля пустое — проверяем с сохранённым", withSaved.CheckResult == "Подключение есть", () => withSaved.CheckResult);
                fresh.BaseUrl = $"http://127.0.0.1:{denyPort}";
                await fresh.CheckAsync("p");
                Check("401 — неверный логин или пароль", fresh.CheckResult == "Не удалось: неверный логин или пароль (HTTP 401)", () => fresh.CheckResult);
                fresh.BaseUrl = $"http://127.0.0.1:{pagePort}";
                await fresh.CheckAsync("p");
                Check("ответила не API (страница входа) — так и сказано, с ответом",
                    fresh.CheckResult.StartsWith("Не удалось: ответ не похож на API Интрасервиса") && fresh.CheckResult.Contains("Вход в систему"),
                    () => fresh.CheckResult);

                Check("окно менялось только в своём потоке", pump.OffThread == 0, () => $"изменений из чужого потока: {pump.OffThread}");
            }
            finally
            {
                HotkeyService.Parses = parsesWas;
                ok.Stop();
                deny.Stop();
                page.Stop();
            }
        }
        checks.AssertAll();
    }
}
