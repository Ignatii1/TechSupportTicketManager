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
        // справочник статусов для колонок доски (этап 8, шаг 2)
        var (dir, dirPort) = FakeIntraservice.Start(t => t == "/api/taskstatus"
            ? (200, """[{"Id":31,"Name":"Открыта"},{"Id":32,"Name":"В работе"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":46,"Name":"Ожидание ответа пользователя","IsCommentRequired":true}]""")
            : (404, "{}"));
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

                // числа: в своих границах, пробелы по краям не мешают. Каждое — по отдельности, остальные верны: «сохранить
                // нельзя» должна давать именно ошибка этого поля
                foreach (var (set, error, value, expected, good) in new (Action<string>, Func<string>, string, string, string)[]
                {
                    (v => vm.WipLimit = v, () => vm.WipLimitError, "0", "Целое число от 1 до 50", "8"),
                    (v => vm.WipLimit = v, () => vm.WipLimitError, "51", "Целое число от 1 до 50", "8"),
                    (v => vm.WipLimit = v, () => vm.WipLimitError, " 50 ", "", "8"),
                    (v => vm.HideDoneDays = v, () => vm.HideDoneDaysError, "0", "", "14"),
                    (v => vm.HideDoneDays = v, () => vm.HideDoneDaysError, "366", "Целое число от 0 до 365", "14"),
                    (v => vm.OverdueDays = v, () => vm.OverdueDaysError, "0", "Целое число от 1 до 90", "4"),
                    (v => vm.OverdueDays = v, () => vm.OverdueDaysError, "три", "Целое число от 1 до 90", "4"),
                    (v => vm.AutoSyncMinutes = v, () => vm.AutoSyncMinutesError, "0", "", "10"),
                    (v => vm.AutoSyncMinutes = v, () => vm.AutoSyncMinutesError, "121", "Целое число от 0 до 120", "10"),
                })
                {
                    set(value);
                    Check($"«{value}» → «{expected}», сохранить {(expected == "" ? "можно" : "нельзя")}",
                        error() == expected && vm.IsValid == (expected == ""), () => $"{error()} · можно: {vm.IsValid}");
                    set(good);
                }
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

                // автовыгрузка: выключена по умолчанию; в пояснении — куда пойдут файлы (папка выгрузки «Поиска заявок»)
                Check("автовыгрузка выключена по умолчанию", !vm.AutoExportClosed);
                Check("…в пояснении — папка выгрузки", vm.AutoExportNote.Contains(Path.Combine(data.Path, "knowledge")), () => vm.AutoExportNote);

                // какие заявки (0.22.0): по умолчанию мои закрытые — при каждом автообновлении; зеркало — раз в час, вложения только
                // ваших, а всё, что раньше недели, — из «Поиска заявок» с подсказкой, как её настроить
                Check("охват — по умолчанию мои закрытые, выбор из трёх", vm.Reach.Reach == ExportReach.Mine && vm.ReachChoices.Count == 3
                    && vm.ReachChoices.Select(c => c.Reach).Distinct().Count() == 3, () => vm.Reach.Label);
                Check("…пояснение моих — при каждом автообновлении", vm.AutoExportNote.Contains("При каждом автообновлении")
                    && !vm.AutoExportNote.Contains("раз в час"), () => vm.AutoExportNote);
                var noteChanged = 0;
                vm.PropertyChanged += (_, e) => noteChanged += e.PropertyName == nameof(SettingsViewModel.AutoExportNote) ? 1 : 0;
                vm.Reach = vm.ReachChoices.Single(c => c.Reach == ExportReach.Closed);
                // 0.23.0: пользователь счёл, что «по 30 за заход» тысячи будут грузиться вечно, — пояснение говорит два шага: сначала
                // один раз полная выгрузка из «Поиска заявок» (без порций), потом зеркало раз в час берёт только изменившиеся
                Check("…все закрытые: сначала полная выгрузка («Закрытые», без порций), потом раз в час — только изменившиеся", noteChanged == 1
                    && vm.AutoExportNote.Contains("все закрытые заявки, что вам видны") && vm.AutoExportNote.Contains("Сначала один раз — полная выгрузка")
                    && vm.AutoExportNote.Contains("без порций") && vm.AutoExportNote.Contains("«Закрытые»") && vm.AutoExportNote.Contains("«Сбросить»")
                    && vm.AutoExportNote.Contains("раз в час берёт только изменившиеся") && vm.AutoExportNote.Contains("только последнюю неделю")
                    && vm.AutoExportNote.Contains("Вложения — только ваших") && vm.AutoExportNote.Contains(Path.Combine(data.Path, "knowledge")), () => vm.AutoExportNote);
                vm.Reach = vm.ReachChoices.Single(c => c.Reach == ExportReach.All);
                Check("…все заявки: и открытые, статус «Любой»", vm.AutoExportNote.Contains("все заявки, что вам видны, и открытые")
                    && vm.AutoExportNote.Contains("«Любой»") && vm.AutoExportNote.Contains("Сначала один раз — полная выгрузка"), () => vm.AutoExportNote);

                // запись в Интрасервис (этап 8): выключена, пока её не включили здесь
                Check("запись в Интрасервис — по умолчанию выключена", !vm.AllowWrite && !new AppSettings().AllowIntraserviceWrite);

                // вложения в базу: по умолчанию — моих заявок; выбор из трёх
                Check("вложения — по умолчанию моих заявок", vm.Attachments.Scope == AttachmentScope.Mine && vm.AttachmentChoices.Count == 3
                    && vm.AttachmentChoices.Select(c => c.Scope).Distinct().Count() == 3, () => vm.Attachments.Label);

                // запись: в тот же экземпляр и в settings.json
                (vm.Login, vm.RelayEnabled, vm.AutoExportClosed, vm.AllowWrite) = (" other ", false, true, true);
                vm.Attachments = vm.AttachmentChoices.Single(c => c.Scope == AttachmentScope.None);
                vm.Save("");
                Check("сохранено в настройки — обрезано и разобрано", settings is
                    {
                        Hotkey: "Ctrl+Alt+T", IntraserviceIdPattern: @"Task/View/(\d+)", HideDoneOlderThanDays: 14, WipLimit: 8, OverdueDays: 4,
                        AutoSyncMinutes: 10, IntraserviceBaseUrl: "https://hd.example.ru/", IntraserviceLogin: "other", ClaudeRelayEnabled: false,
                        AutoExportClosed: true, KnowledgeAttachments: AttachmentScope.None, AutoExportReach: ExportReach.All,
                        AllowIntraserviceWrite: true,
                    }, () => $"{settings.IntraserviceBaseUrl} · {settings.IntraserviceLogin} · {settings.Hotkey} · запись: {settings.AllowIntraserviceWrite}");
                Check("…и в settings.json", AppSettings.Load(data.Path, out var problem) is
                    { WipLimit: 8, AutoSyncMinutes: 10, IntraserviceLogin: "other", Hotkey: "Ctrl+Alt+T", ClaudeRelayEnabled: false, AutoExportClosed: true,
                      KnowledgeAttachments: AttachmentScope.None, AutoExportReach: ExportReach.All, AllowIntraserviceWrite: true }
                    && problem == "");
                Check("…и новое окно настроек её показывает", new SettingsViewModel(settings) is
                    { AutoExportClosed: true, Attachments.Scope: AttachmentScope.None, Reach.Reach: ExportReach.All, AllowWrite: true });

                // статус в Интрасервисе при переносе карточки (этап 8, шаг 2): выключен; у каждой колонки — «не менять»
                Check("статус при переносе — по умолчанию выключен, колонкам ничего не назначено", !vm.AllowStatus && !new AppSettings().AllowStatusWrite
                    && new AppSettings().StatusOnMove is null && vm.ColumnStatuses.Select(r => r.Title).SequenceEqual(new[] { "Входящие", "В работе", "Ждёт ответа", "Готово" })
                    && vm.ColumnStatuses.All(r => r.Selected.Id is null && r.Selected.Name == "не менять"),
                    () => string.Join(" | ", vm.ColumnStatuses.Select(r => $"{r.Title}: {r.Selected.Name}")));
                var mapped = new AppSettings
                {
                    IntraserviceBaseUrl = $"http://127.0.0.1:{dirPort}", IntraserviceLogin = "me", IntraservicePassword = "p",
                    StatusOnMove = new() { [Models.TicketStatus.Done] = new(29, "Выполнена"), [Models.TicketStatus.Inbox] = new(77, "Новая") },
                };
                var sv = new SettingsViewModel(mapped);
                pump.Watch(sv);
                StatusOption Option(Models.TicketStatus column) => sv.ColumnStatuses.Single(r => r.Column == column).Selected;
                Check("до чтения справочника — сохранённые статусы колонок видны под своими названиями", Option(Models.TicketStatus.Done) is { Id: 29, Name: "Выполнена" }
                    && Option(Models.TicketStatus.Inbox) is { Id: 77, Name: "Новая" }, () => $"{Option(Models.TicketStatus.Done)} · {Option(Models.TicketStatus.Inbox)}");
                await sv.LoadStatusesAsync();
                var options = sv.ColumnStatuses.Single(r => r.Column == Models.TicketStatus.Waiting).Options;
                Check("справочник прочитан — у колонок «не менять» и статусы сервера", sv.StatusesNote == ""
                    && options.Select(o => o.Name).SequenceEqual(new[] { "не менять", "Открыта", "В работе", "Выполнена", "Ожидание ответа пользователя" }),
                    () => $"{sv.StatusesNote} · {string.Join(", ", options.Select(o => o.Name))}");
                Check("…сохранённый — выбран; которого на сервере нет — остался под своим названием", Option(Models.TicketStatus.Done) is { Id: 29, Name: "Выполнена" }
                    && Option(Models.TicketStatus.Inbox) is { Id: 77, Name: "Новая" }
                    && sv.ColumnStatuses.Single(r => r.Column == Models.TicketStatus.Inbox).Options.Any(o => o.Id == 77),
                    () => $"{Option(Models.TicketStatus.Done)} · {Option(Models.TicketStatus.Inbox)}");
                var waiting = sv.ColumnStatuses.Single(r => r.Column == Models.TicketStatus.Waiting);
                waiting.Selected = waiting.Options.Single(o => o.Id == 46);
                var inboxRow = sv.ColumnStatuses.Single(r => r.Column == Models.TicketStatus.Inbox);
                inboxRow.Selected = inboxRow.Options[0];   // «не менять»
                sv.AllowStatus = true;
                try { sv.Save(""); }
                catch (NotSupportedException) when (!OperatingSystem.IsWindows()) { /* пароль — DPAPI, здесь её нет; правила применены до записи */ }
                Check("сохранено: выключатель и статусы колонок, «не менять» — без статуса", mapped.AllowStatusWrite && mapped.StatusOnMove is { Count: 2 } map
                    && map[Models.TicketStatus.Done] == new StatusRef(29, "Выполнена") && map[Models.TicketStatus.Waiting] == new StatusRef(46, "Ожидание ответа пользователя"),
                    () => string.Join(", ", (mapped.StatusOnMove ?? new()).Select(kv => $"{kv.Key}: {kv.Value}")));
                var noApi = new SettingsViewModel(new AppSettings());
                await noApi.LoadStatusesAsync();
                Check("без API — сказано, откуда статусы взять", noApi.StatusesNote.Contains("API"), () => noApi.StatusesNote);
                var refused = new SettingsViewModel(new AppSettings { IntraserviceBaseUrl = $"http://127.0.0.1:{denyPort}", IntraserviceLogin = "me", IntraservicePassword = "p" });
                await refused.LoadStatusesAsync();
                Check("справочник не прочитан — сказано почему", refused.StatusesNote.Contains("неверный логин или пароль"), () => refused.StatusesNote);

                // пароль: пустое поле — сохранённый не меняется, введённый — заменяет. В файл он идёт через DPAPI, которой
                // на Linux нет, — запись тогда падает, но правило применяется до неё (на Windows, в CI, запись проходит)
                void SaveWith(string password)
                {
                    try { vm.Save(password); }
                    catch (NotSupportedException) when (!OperatingSystem.IsWindows()) { /* DPAPI здесь нет */ }
                }
                settings.IntraservicePassword = "был";
                SaveWith("");
                Check("пустое поле пароля — сохранённый не меняется", settings.IntraservicePassword == "был");
                SaveWith("новый");
                Check("введённый пароль — заменяет", settings.IntraservicePassword == "новый");
                Check("открытым текстом пароль в settings.json не попадает",
                    !File.ReadAllText(AppSettings.PathFor(data.Path)).Contains("новый"));
                if (OperatingSystem.IsWindows())   // в CI: DPAPI есть — пароль записан зашифрованным и читается обратно
                    Check("пароль записан зашифрованным и читается", AppSettings.Load(data.Path, out _).IntraservicePassword == "новый");
                else checks.Skipped("пароль записан зашифрованным и читается", "DPAPI есть только в Windows — проверяется в CI");

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
                dir.Stop();
            }
        }
        checks.AssertAll();
    }
}
