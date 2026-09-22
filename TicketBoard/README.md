# TicketBoard — для разработчика

Как пользоваться приложением — в [README в корне репозитория](../README.md). Здесь — сборка и устройство.

WPF · .NET 10 (`net10.0-windows`) · [WPF-UI](https://github.com/lepoco/wpfui) (Fluent / Mica) · CommunityToolkit.Mvvm ·
gong-wpf-dragdrop · H.NotifyIcon.Wpf. Интерфейс и комментарии в коде — на русском.

## Сборка и запуск

Нужен .NET 10 SDK: https://dotnet.microsoft.com/download (без прав администратора —
[`dotnet-install.ps1 -Channel 10.0`](https://learn.microsoft.com/dotnet/core/tools/dotnet-install-script)).

```
cd TicketBoard
dotnet run                    # Debug: при старте ещё и самопроверка разбора ответов API и ссылок
dotnet publish -c Release     # один самодостаточный exe
```

Release-сборка: `bin\Release\net10.0-windows\win-x64\publish\TicketBoard.exe` — self-contained, single-file, ReadyToRun,
сжатый. .NET на машине пользователя не нужен.

Собирается и на Linux/macOS (`EnableWindowsTargeting`) — удобно для проверки компиляции, но запускать можно только на Windows.

## CI и выпуск версии

`.github/workflows/build.yml` (GitHub Actions, `windows-latest`):
- каждый push в `main` и каждый PR — сборка, exe в артефактах запуска (**Actions → запуск → Artifacts**);
- тег `v*` — ещё и GitHub Release с exe:
  ```
  git tag v0.1.0
  git push origin v0.1.0
  ```

## Устройство

```
App.xaml.cs                      старт: одна копия, тема, трей (значок с бейджем рисуется в RenderTrayIcon), хоткей,
                                 окно настроек, применение настроек на лету, лог ошибок
Models/Ticket.cs                 заявка, заметка, статусы, возраст в колонке; TicketRules — пороги из настроек
Services/TicketStore.cs          tickets.json: атомарная запись, бэкап раз в день (30 шт.), битый файл → .corrupt-…
Services/AppSettings.cs          settings.json; пароль Интрасервиса — DPAPI (CurrentUser)
Services/IntraserviceLinkParser  ссылка/номер заявки из текста (регулярка из настроек, голый номер), самопроверка — SelfCheck
ViewModels/SearchViewModel.cs    поиск на сервере: запрос, строки результата, «уже на доске», добавление на доску
Views/SearchWindow.xaml(.cs)     окно результатов поиска (Enter в поле поиска на доске)
Services/HttpIntraserviceClient.cs            REST API Интрасервиса: запросы и ошибки; null — API не настроен
Services/HttpIntraserviceClient.Parse.cs      разбор ответов — все имена полей API только здесь
Services/HttpIntraserviceClient.SelfCheck.cs  образцы ответов для разбора; гоняются ../TicketBoard.SelfCheck
Services/HotkeyService.cs        глобальный хоткей (RegisterHotKey)
Services/AutostartService.cs     автозапуск: HKCU\Software\Microsoft\Windows\CurrentVersion\Run
Services/TokenTheme.cs           подключает Themes/Tokens.*.xaml под тему; акцент — системный
ViewModels/MainViewModel.cs      доска: колонки, фильтры, перенос, заметки, автосохранение (600 мс)
ViewModels/MainViewModel.Intraservice.cs  синхронизация карточки, импорт «моих», F5 — обновить статусы
ViewModels/MainViewModel.Comments.cs      переписка выбранной заявки («Переписка» в панели)
ViewModels/ColumnViewModel.cs    колонка, счётчик, перегруз, приём drag&drop
ViewModels/QuickCaptureViewModel окно быстрого добавления, превью названия (пауза 400 мс, отмена прошлого запроса)
ViewModels/SettingsViewModel.cs  поля окна настроек и их проверка
Views/MainWindow.xaml(.cs)       доска, карточка, панель деталей, клавиатура, анимация появления карточки
Views/QuickCaptureWindow.xaml    окно быстрого добавления
Views/SettingsWindow.xaml        окно настроек
Themes/Tokens.Light|Dark.xaml    цвета из tokens.css макета
Themes/Styles.xaml               карточка, бейджи, чипы, kbd, кнопки, поля настроек
Converters/Converters.cs         мелкие конвертеры для XAML
```

Все настройки применяются без перезапуска: `App.ApplySettings` → `MainViewModel.ApplySettings` / `QuickCaptureViewModel.ApplySettings`.

## API Интрасервиса

По документации IntraService API v5.42 (на сайте PDF больше не отдаётся; копия —
[Wayback Machine](https://web.archive.org/web/20250808102006id_/https://intraservice.ru/upload/iblock/1ea/ktnvaryw9iceol8dz0cao6hhlnjgq8rj/IntraService_API_v5_42.pdf)):

- авторизация — только Basic (логин:пароль пользователя), токенов нет;
- `GET {адрес}/api/task/{id}?include=status`, `Accept: application/json`;
- поля: `Id`, `Name`, `Description`, `StatusId`, `StatusName`; блок `Statuses: [{Id, Name}]`;
- «Проверить подключение» — `GET {адрес}/api/taskstatus`.

**Не проверено на живом сервере.** В документации ответ на одну заявку показан только в XML; предполагается JSON
`{"Task": {...}, "Statuses": [...]}`, но `Parse` принимает и объект без обёртки. Описание считается HTML и сводится к тексту.
Все имена полей — только в разборщиках `HttpIntraserviceClient`. Когда будут настоящие ответы сервера — поправить их
и добавить образцы в `SelfCheck`.

## Дизайн

Макеты — Claude Design («Трекер заявок»), спецификация под WPF — `export/spec.md` в бандле. Бандл удалён из репозитория,
но есть в истории: `git show 579b8b9 --name-only`.

## Состояние и планы

Текущее состояние, открытые задачи и история изменений — в [PROGRESS.md](../PROGRESS.md).
Карта кода для агентов («что где менять») — в [AGENTS.md](../AGENTS.md).

## Как всё связано (перенесено из AGENTS.md)

`App.OnStartup` (`App.xaml.cs`) is the only place that creates and wires objects. There's no DI container.

1. Single-instance mutex `Local\TicketBoard.SingleInstance`. A second copy exits.
2. Global error handlers: `ShowError` (message box) and `LogError` (`errors.log`).
3. `AppSettings.Load` → `IntraserviceLinkParser(settings)` → `HttpIntraserviceClient.From(settings)` (`null` = API not configured) →
   `MainViewModel(TicketStore, settings, parser, client)`.
4. Theme: WPF-UI follows the system theme; `TokenTheme.Apply` swaps in `Tokens.Light/Dark.xaml` and replaces the accent with the system accent.
5. Creates the windows, then `SetupTray` and `SetupHotkey`. The main window is shown unless the app was started with `--minimized` (autostart).

Settings changes apply live. `SettingsWindow.Saved` → `App.ApplySettings` → new API client → `MainViewModel.ApplySettings` and
`QuickCaptureViewModel.ApplySettings` → tray redraw → hotkey re-register. The parser reads the regex from the shared
`AppSettings` instance on each call.

Data flow for a new ticket: hotkey → `QuickCaptureWindow.ShowCapture` (prefills a ticket URL from the clipboard) → Enter →
`MainViewModel.AddFromCapture` (parse URL/number, pick a title, insert into Inbox) → `SyncAsync` fills title, description and
external status from the API → `PropertyChanged` → debounced save (600 ms) → `TicketStore.Save`.

### Новая настройка — 5 мест

1. Add the property with a default to `AppSettings`.
2. In `SettingsViewModel`, add a string field, an `Error` field, validation in `On<Name>Changed`, a term in `IsValid`, and the assignment in `Save`.
3. Add the field to `Views/SettingsWindow.xaml`.
4. Consume it: read it in the relevant `ApplySettings` so it applies without a restart.
5. Add a row to the settings table in the root `README.md`.
