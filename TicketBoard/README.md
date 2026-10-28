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

## Claude через буфер обмена

У пользователя подписка claude.ai (Pro), ключа API нет и не будет; интернет на рабочем ПК — только в браузере.
claude.ai не может позвать TicketBoard, TicketBoard не может позвать claude.ai — поэтому канал между ними буфер обмена,
а ход обмена ведёт Claude:

```
claude.ai: Claude пишет блок «TB search …/TB ticket N/TB history N/TB board»
   └─ пользователь: Copy ─▶ ClipboardWatcher (WM_CLIPBOARDUPDATE) ─▶ App.OnClipboardChanged
        ─▶ ClaudeRelay.Parse ─▶ RunAsync (HttpIntraserviceClient + App.BoardSnapshot, по 4 запроса одновременно)
        ─▶ ответ в буфер + уведомление в трее ─▶ пользователь: Ctrl+V, Enter ─▶ Claude решает, хватит ли
```

- Протокол Claude узнаёт из `ClaudeRelay.Instructions` — их копирует кнопка в настройках, пользователь вставляет
  в инструкции проекта на claude.ai. Поменял формат запросов — поменяй и инструкцию, и таблицу в корневом README.
- `Parse` берёт текст, только если **каждая** непустая строка — `TB …` (заглавными; ограды ``` пропускаются) и есть хоть
  один понятный запрос; иначе буфер не наш и не трогается. Непонятная строка рядом с понятными — `Invalid`: Claude
  получает «не понял» и исправляется. Свой ответ (начинается с «TicketBoard →») `Parse` не узнаёт — круга не будет.
- До чтения текста `ClipboardWatcher.HasPlainText` смотрит только список форматов: файлы, Office (он рисует текст по
  запросу — секунды на большой таблице) и данные менеджеров паролей (`ExcludeClipboardContentFromMonitorProcessing`)
  не читаются вовсе.
- Пока ответ собирается, буфер могли поменять: по `GetClipboardSequenceNumber` ответ тогда не пишется (чужое не
  затираем), а после — буфер смотрится ещё раз (следующий блок `TB` выполнится). Сняли галочку — не пишется тоже.
- Ответ — текст для чата, не JSON: коротко, со ссылками на заявки; длинное обрезается с явной пометкой, весь ответ —
  до 60 000 символов (длинную вставку claude.ai делает вложением). Доска — как её видит пользователь: старое «Готово»
  только числом.
- Отвергнуто (подробнее — `docs/HISTORY.md`, v0.6.0): коннектор claude.ai (его зовёт облако Anthropic — нужен вход из
  интернета во внутреннюю сеть), расширение браузера поверх claude.ai (установка, хрупкая вёрстка, автоотправка — это уже
  автоматизация потребительского приложения), страница с ключом API (v0.5.0 — ключа у пользователя нет).
- Проверка: `TicketBoard.SelfCheck` — разбор блоков и живой прогон `RunAsync` с настоящим `HttpIntraserviceClient`
  против поддельного сервера на loopback. `ClipboardWatcher` и обработчик в `App` — только на Windows.

## Устройство
## Устройство

```
App.xaml.cs                      старт: одна копия, тема, трей (значок с бейджем рисуется в RenderTrayIcon), хоткей,
                                 окно настроек, применение настроек на лету, лог ошибок
Models/Ticket.cs                 заявка, заметка, статусы, возраст в колонке; TicketRules — пороги из настроек
Services/TicketStore.cs          tickets.json: атомарная запись, бэкап раз в день (30 шт.), битый файл → .corrupt-…
Services/AppSettings.cs          settings.json (атомарная запись, битый → .corrupt-…); пароль Интрасервиса — DPAPI (CurrentUser)
Services/ClaudeRelay.cs          ответы Claude через буфер: разбор блока «TB …», запросы, текст ответа, инструкция
Services/ClaudeRelay.SelfCheck.cs  разбор и живой прогон против поддельного Интрасервиса; гоняется ../TicketBoard.SelfCheck
Services/ClipboardWatcher.cs     изменения буфера обмена (AddClipboardFormatListener на message-only окне)
Views/AskWindow.xaml(.cs)        вопрос/сообщение в стиле приложения вместо системного MessageBox
Services/IntraserviceLinkParser  ссылка/номер заявки из текста (регулярка из настроек, голый номер), самопроверка — SelfCheck
ViewModels/SearchViewModel.cs    окно «Поиск заявок»: условия → список → просмотр → буфер обмена / файлы; условия и папка — в settings.json
Views/SearchWindow.xaml(.cs)     разметка окна (кнопка на доске, трей, Enter в поле поиска на доске)
Services/TaskQuery.cs            отбор заявок для api/task: все фильтры списка из документации и адрес запроса (+ самопроверка)
Services/TicketSearch.cs         условия окна (SearchFilter) → запрос: «я», имена → номера, статусы, сервис с вложенными, даты
Services/KnowledgeExport.cs      ход выгрузки: BuildAsync (заявка Markdown-ом: карточка + вся переписка), RunAsync (по отбору, 0 — все:
                                 страница списка → сразу файлы, повторы, выключатель) и ExportRowsAsync (по готовому списку)
Services/KnowledgeExport.Format.cs  имя файла и текст заявки (свойства YAML, описание, переписка по времени); MachineDate —
                                    один вид дат для программ в свойствах и tickets.csv (область dates)
Services/KnowledgeExport.Files.cs   папки tickets/ГГГГ-ММ, поиск прежних файлов, переезд плоской раскладки 0.10–0.12, _index.md
Services/KnowledgeExport.Table.cs   tickets.csv (колонки — один список, по месяцам: тронутое и отставшее — из файлов, прочее —
                                 из прежней таблицы), SCHEMA.md, заготовка CLAUDE.md; эталоны в его SelfCheck (область table)
Services/KnowledgeExport.Ask.cs     «Спросить базу знаний»: текст запроса агенту, обрезка длинной заявки (область ask)
Services/KnowledgeExport.History.cs что видно по истории заявки (формат 5): участники, моя роль, часы реакции и решения,
                                    переоткрытия, комментарии, закрывающий (область history); ExportContext — кто я и закрытые статусы
Services/KnowledgeExport.Secrets.cs пароли в тексте → «[скрыт]» для выгрузки, запроса агенту и ответов Claude (область secrets)
Services/KnowledgeExport.Attachments.cs вложения в базе (формат 7): имя на диске, картинки, размер — Attachments (область files);
                                    files/ и files-lfs/ (от 50 МБ — Git LFS), правило в .gitattributes, убранные с заявки файлы;
                                    правила панели (0.20.0): что Windows запускает (Runs), что Проводник читает (ShellReads → «_»),
                                    имя для сохранения и в папке временных, пометка «из интернета» (Zone.Identifier)
Services/FileShell.cs               что вложениям панели нужно от Windows: открыть, показать в Проводнике, «Сохранить как»,
                                    AssocIsDangerous; в самопроверке — заглушка (WpfStubs.cs)
Services/ClosedExport.cs            автовыгрузка закрытых: отметка (ExportMark), «моё ли» (ExportScope), один заход (RunAsync)
Services/KnowledgeExport.SelfCheck.cs       имена файлов, экранирование и живые выгрузки против поддельного Интрасервиса
Services/KnowledgeExport.SelfCheck.Mass.cs  выгрузка всех: поддельный сервер на 230 заявок — страницы, остановка и продолжение,
                                 повторы, выключатели, сортировка, плоская папка, период
Services/KnowledgeExport.SelfCheck.Files.cs вложения: поддельный сервер отдаёт файлы — побайтно, files-lfs, правило LFS раньше
                                 крупного, повтор без запросов, убранные и переименованные, «всех»/«никаких», обрыв, тишина, стоп
Services/FakeIntraservice.cs     поддельный Интрасервис на loopback для самопроверок (релей, выгрузка, поиск, окно, доска)
../TicketBoard.SelfCheck/SearchWindowCheck.cs  окно поиска без WPF: сценарии на настоящем SearchViewModel
../TicketBoard.SelfCheck/BoardCheck.cs         доска без WPF: импорт, F5 и заходы автообновления (и их лимиты) на настоящем
                                 MainViewModel против сервера, где заявки меняются между шагами
../TicketBoard.SelfCheck/AutoExportCheck.cs    автовыгрузка закрытых против BoardServer: чьё, отметка и её минута, порции, сбои, смена
                                 учётной записи посреди захода, .gitignore, строка для трея
../TicketBoard.SelfCheck/CardCheck.cs          карточка и панель руками: быстрое добавление, ⟳, люди старых карточек,
                                 переписка, заметки, фильтры и счётчики, клавиши, возраст, доска без API
../TicketBoard.SelfCheck/BoardServer.cs        поддельный Интрасервис «моих заявок» для обеих (и для панели: файлы, сбои, Range)
../TicketBoard.SelfCheck/PanelCheck.cs         панель (область panel): поля карточки, тихое чтение, вложения — размеры, «Открыть»,
                                 «Сохранить», папка временных, повторы, ярлыки, смена учётной записи
../TicketBoard.SelfCheck/StoreCheck.cs         tickets.json: запись и чтение, бэкапы, битый файл, имена полей (формат файла)
../TicketBoard.SelfCheck/SettingsCheck.cs      окно настроек: ошибки под полями, запись, пароль, «Проверить» (хоткей — заглушка)
../TicketBoard.SelfCheck/CheckSet.cs           общее для самопроверок: список непрошедших, своя папка данных
../TicketBoard.SelfCheck/SingleThread.cs       один поток для проверок окон и доски, как UI-поток WPF; считает изменения из чужого
../TicketBoard.SelfCheck/WpfStubs.cs           что viewmodel'и берут у WPF и приложения: диалоги, таймеры (тикает проверка),
                                 Dispatcher, представления коллекций, буфер, папка данных, разбор хоткея (ответ задаёт проверка)
../TicketBoard.SelfCheck/xamlcheck.py          ключи ресурсов (в своей области) и пути привязок по звеньям в XAML окна или панели
../TicketBoard.SelfCheck/mutants/              подложенные ошибки по областям и их прогон во временных копиях: ловят ли их проверки
../TicketBoard.SelfCheck/coverage.py           какие строки приложения не исполняет ни одна проверка (dotnet-coverage)
Services/HttpIntraserviceClient.cs            REST API Интрасервиса: запросы и ошибки; null — API не настроен
Services/HttpIntraserviceClient.Parse.cs      разбор ответов — все имена полей API только здесь
Services/HttpIntraserviceClient.SelfCheck.cs  образцы ответов для разбора; гоняются ../TicketBoard.SelfCheck
Services/HotkeyService.cs        глобальный хоткей (RegisterHotKey)
Services/AutostartService.cs     автозапуск: HKCU\Software\Microsoft\Windows\CurrentVersion\Run
Services/TokenTheme.cs           подключает Themes/Tokens.*.xaml под тему; акцент — системный
ViewModels/MainViewModel.cs      доска: колонки, фильтры, перенос, заметки, автосохранение (600 мс)
ViewModels/MainViewModel.Intraservice.cs  синхронизация карточки, импорт «моих», F5 — обновить статусы
ViewModels/MainViewModel.AutoExport.cs    автовыгрузка после захода автообновления: мои закрытые или зеркало (все закрытые, все заявки — раз в час), отметка с охватом в settings.json, строка для трея, *.tmp в .gitignore
ViewModels/MainViewModel.Ask.cs           база знаний из панели: «Спросить базу знаний» (запрос агенту — в буфер) и «В базу знаний» (заявку — в базу сейчас)
ViewModels/MainViewModel.AutoSync.cs      автообновление по таймеру: те же импорт и F5 без окон, уведомления в трей
Services/AutoSyncRules.cs        что автообновление не добавляет (AutoSyncSkipIds), листание списка до конца, счёт новых
                                 комментариев (Unread) и «снова открыли / передали» (Track) — с самопроверкой
ViewModels/MainViewModel.Comments.cs      переписка выбранной заявки («Переписка» в панели)
ViewModels/MainViewModel.Attachments.cs   вложения панели: тихое чтение карточки, размеры, «Открыть» (папка временных), «Сохранить»
ViewModels/ColumnViewModel.cs    колонка, счётчик, перегруз, приём drag&drop
ViewModels/QuickCaptureViewModel окно быстрого добавления, превью названия (пауза 400 мс, отмена прошлого запроса)
ViewModels/SettingsViewModel.cs  поля окна настроек и их проверка
Views/MainWindow.xaml(.cs)       доска, карточка, рамка выезда панели деталей, клавиатура, анимация появления карточки
Views/TicketPanel.xaml(.cs)      панель деталей заявки (UserControl): поля, люди, вложения, заметки, переписка; данные — MainViewModel окна
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
- «Проверить подключение» — `GET {адрес}/api/taskstatus`;
- вложение — `GET {адрес}/api/taskfile/{номер файла}` (стр. 33): живой отдаёт файл сразу (2026-10-12); номера — `FileIds`
  карточки, имена — `FileNames` через `||` (стр. 10; `Files` устарело). Размера в карточке нет — он известен после
  скачивания (`HttpIntraserviceClient.DownloadFileAsync`: потоком, 60 с на заголовки и на каждый следующий кусок).
  Панель (0.20.0) узнаёт размер запросом с `Range: bytes=0-0`: понимающий сервер отвечает 206 одним байтом и длиной в
  `Content-Range` (`*` — длина неизвестна), пустой файл — 416 с `bytes */0`; не понимающий шлёт файл — читается первый
  кусок, длина из `Content-Length`, соединение закрывается (`MaxResponseDrainSize = 0`), а не дочитывается. Понимает ли
  Range живой сервер — **не проверено**: работает и так, и так.

Карточка заявки проверена на живом сервере (2026-10-02, ответ без `include`): `{"Task": {...}, "Statuses": null,
"Services": null, "Users": null, …}` — соседние блоки без `include` приходят `null`, статус есть и строкой (`StatusName`).
Поля: `Id`, `Name`, `Description` (текст с `\r\n`), `StatusId`, `StatusName`, `Created`, `Changed` (ISO без пояса),
`Creator`, `CreatorPhone`, `CreatorEmail`, `Executors` (строкой через запятую), `ExecutorIds`, `ExecutorGroup`,
`ServiceName`, `Type` (не `TypeName`), `Categories`, `ResolutionDateFact`, `ObserverIds` и `Observers` (наблюдатели — и в
строках списка, 2026-10-12); пустое — `null`. Строки списка
(`GET api/task?…`) несут `Creator`, `Executors`, `Created`, `Changed`, `ResolutionDateFact`, `Description`, но не сервис
и тип — выгрузка берёт их из карточки. Образец этой формы — в `HttpIntraserviceClient.SelfCheck`. Все имена полей — только
в разборщиках `HttpIntraserviceClient`.

Живой сервер — сборка на ASP.NET Core и проверяет типы параметров строже PDF (2026-10-08): `count=all` (стр. 14) — HTTP 400
`{"errors":{"count":["The value 'all' is not valid."]}, …}`, `count` у него логический. Поэтому список заявок просится со
счётом по умолчанию (по документации — до 1000: ровно 1000 показывается как «1 000+») или `count=false` (страницы
выгрузки после первой; конец — `HasNextPage`, а без него — пустая страница). Справочник сервисов без `for` отдаётся только
с правом на просмотр списка сервисов; у учётной записи пользователя список оказался пустым (почему — пока неизвестно),
поэтому есть запасные: `for=filtertasks`, затем блок `Services` страницы 1000 последних заявок. Такой отказ проверки
параметров повторён в `FakeIntraservice.Refused` — новый, увиденный на живом сервере, добавлять туда же.

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
