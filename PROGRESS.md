# PROGRESS.md — state and history

Shared log for agents and humans. **Read before starting; append before finishing.** The map of the code is in `AGENTS.md`.

Rules:
- Update **Current state** and **Open work** in place. Don't let them go stale.
- Add a **Log** entry at the top of the log for each session or merged change: date, what changed and why, what was verified
  and how, and what's left. Keep entries short; the details belong in commit messages.
- Say what you *didn't* verify. For example, UI changes can't be run on the Linux dev machine.

## Current state (2026-09-21)

- Version `v0.4.0` — everything through «Обновить статусы» (F5), the comment styling and unparsed-response logging (2026-09-22) is merged into `main` and released. CI builds the exe on
  every push to `main`; a `v*` tag also publishes a Release with the exe attached. `<Version>` in `TicketBoard.csproj`
  has to be bumped together with the tag — it is what the file properties of the exe show.
- Feature-complete for daily use:
  - four-column board with drag&drop, arrow-key and context-menu moves;
  - age badges (warn/overdue), WIP limit;
  - search, priority filter, hide old Done;
  - detail panel with inline edit and notes;
  - quick capture from a global hotkey (`Ctrl+Alt+Space`) with clipboard prefill;
  - tray icon with Inbox badge and overload dot, autostart;
  - settings window applied live;
  - local JSON storage **next to the exe** (copy the folder = move the app), atomic save, daily backups (30 kept)
    and corrupt-file quarantine;
  - errors.log.
- Intraservice API (read-only): title, description and status by ticket number, a connection check in settings.
  Written from the v5.42 docs; the user reports credentials work against the live server (2026-09-21), but the response
  shape is still unconfirmed against a captured payload.
- Verification so far: the CI build on Windows (no .NET SDK in the agent container any more — its download is blocked by
  network policy, so `workflow_dispatch` on the branch is the only compile check). `SelfCheck` covers `Parse` and `TryParse`,
  but both are `[Conditional("DEBUG")]`, so a Release CI build never runs them. There's no test project and there are no UI tests, so UI behavior is verified only by the user running
  it on Windows.

## Open work

- [ ] **Проверить на Windows импорт «мои заявки»** (ветка `claude/beautiful-albattani-pd6q7i`).
      1. Трей → «Импорт моих заявок» (и кнопка со стрелкой вниз в панели) приносит открытые заявки, где вы исполнитель,
         и говорит сколько.
      2. Ничего закрытого не приехало — ни «Выполнена», ни «Ожидание ответа с автозакрытием», ни «Закрыта», ни
         «Отменена». Приехало — посмотрите статус на карточке и допишите его в `ClosedStatusNames` в `settings.json`.
      3. Второй запуск подряд ничего не добавляет и не двигает карточки между колонками.
      4. Карточки, которые уже были на доске, сохранили колонку, заметки и приоритет.
      5. У привезённых карточек настоящее название, описание, статус Интрасервиса, ↗ открывает нужную страницу.
      6. Кнопка в панели не вылезает и не обрезается на узком окне (панель и так была почти полной, стало +36 px).
      7. При выключенном VPN или ненастроенном API — одно окно с текстом, без исключения.
- [ ] **Проверить на Windows переписку и поиск** — из прошлого раунда, в релизе v0.2.0 (см. запись от 21.09 ниже).
- [ ] **Выбрать, что делать из `API-IDEAS.md`** — разбор документации API v5.51 (82 стр.), возможности по убыванию пользы:
      автозаполнение доски своими заявками (`ExecutorIds`/`filterid`), инкрементальная синхронизация (`ChangedMoreThan`),
      автоперенос в «Готово» по признаку статуса `IsFixed`, реальные сроки вместо счётчика дней, комментарии из
      `/api/tasklifetime`. Запись в Интрасервис (комментарий, смена статуса) API позволяет, но это против правила
      «никогда не пишем» — нужно решение пользователя.
- [ ] **Verify the Intraservice API on a real server.** Capture real `api/task/{id}?include=status` and `api/taskstatus`
      responses, fix `HttpIntraserviceClient.Parse` if needed, and add the responses as samples in `SelfCheck`.
      Blocked on the user providing server access or a response sample.
- [ ] (after the API check) Refresh statuses automatically, and suggest moving tickets closed in Intraservice to «Готово».
      Today a refresh happens only on add and from the context menu.

Known limitations (deliberate, revisit only if they cause problems):
- `errors.log` is never rotated. It could be trimmed at startup if it grows.
- A corrupt `settings.json` silently falls back to defaults and isn't overwritten until the next save.
- The exe is unsigned, so SmartScreen and AppLocker can block it (documented in the README).
- The password is DPAPI-bound to the Windows user and machine. After moving to another PC it has to be re-entered.

## Log (newest first)

### 2026-09-22 (ночь) — ошибки API с настоящим ответом сервера
- Просьба пользователя (он сам разработчик): не пересказывать ошибку, а показывать, что ответил сервер, — и не
  заставлять лезть в папку за `errors.log`. Лог остаётся.
- `GetAsync` раньше выбрасывал тело любого не-2xx ответа, а это ровно та часть, где сервер объясняет причину. Теперь
  сообщение — «короткая фраза (HTTP код)» и под ней тело ответа; сетевая ошибка — с цепочкой исключений («The SSL
  connection could not be established → The remote certificate is invalid…»); неразобранный ответ — с самим ответом.
  Json и xml показываются байт в байт, html-страница (IIS, прокси, страница входа) — видимым текстом без стилей и
  скриптов, с пометкой. В сообщении — первые 1000 символов, в логе — 4000, по одному образцу на метод и на код HTTP.
- Все четыре места, где показывается ошибка (синхронизация и переписка в панели, окно поиска, проверка в настройках),
  теперь выделяемые — новый стиль `SelectableText`. В окнах-сообщениях `Ctrl+C` и так копирует всё. Итог «Обновить
  статусы» называет первую ошибку, а не только их число. Окно быстрого добавления берёт только первую строку.
- Попутно: `GetAsync` ловит `IOException` (оборванное соединение посреди чтения тела) — раньше оно пролетало мимо всех
  обработчиков.
- По итогам `/code-review high` (десять находок, все приняты): **без `using System.IO` сборка бы упала** — WPF-проект
  убирает `System.IO` из неявных using (конфликт с `System.Windows.Shapes.Path`); **Basic-токен из тела ответа
  вычищается** — страница-трассировка прокси или IIS может повторить заголовок `Authorization`, а это логин и пароль
  в base64, которые иначе оказались бы на экране, в буфере обмена и в `errors.log`; щелчок по выделяемому тексту
  ошибки больше не глушит клавиши доски (поле только для чтения — не «поле ввода»); проверка подключения тоже
  показывает ответ; в html-странице заголовки и строки таблиц — с новой строки; лог — по образцу на метод и код, а не
  на код вообще; в однострочных местах — `Brief` (фраза и причина, без висящего двоеточия); в вопросе «перенести?»
  ошибки коротко, чтобы Win32 не обрезал высокое окно вместе со списком и вопросом.
- Проверено: сборка на CI (после исправлений); новые assert в `SelfCheck` посчитаны руками. Не проверено: как это
  выглядит с настоящим ответом сервера.

### 2026-09-22 (вечер) — «Обновить статусы» и неразобранные ответы в лог
- **Обновить статусы** (`F5`, трей). Импорт — половина петли: после него карточки сами не обновлялись, и закрытая днём
  заявка висела бы «В работе» до ручного «Обновить из Интрасервиса» по каждой. `RefreshAll` перечитывает все карточки
  с номером вне «Готово» (по 4 запроса разом), применяет те же правила, что и одиночная синхронизация, и спрашивает,
  перенести ли в «Готово» закрытые — закрытость по тому же правилу, что у импорта (`IsFixed`/`IsFinal` +
  `ClosedStatusNames`). Правила «что можно перезаписать» (`Apply`) и «какие статусы закрыты» (`ClosedNames`) вынесены
  в одно место, чтобы одиночная синхронизация, импорт и обновление не разъехались.
- **Неразобранный ответ сервера — в `errors.log`.** Семь вызовов написаны по документации, где у половины json не
  показан вовсе. Раньше на «непонятный ответ сервера» оставалось просить пользователя самому достать сырой json на
  рабочем ноутбуке; теперь имя метода и первые 4000 символов ответа ложатся в лог. Логина и пароля в теле нет —
  они в заголовке. Подключено через `[CallerMemberName]`, поэтому все 13 мест поменялись без правки сигнатур.
- Пока идут импорт или обновление, это видно в заголовке окна: итог приходит окном в конце, и без подсказки секунды
  ожидания выглядели как «ничего не произошло».
- **Проверено:** сборка на CI. **Не проверено:** всё на живом сервере; сколько времени занимает обновление на реальной доске.

### 2026-09-22 (вечер) — переписку видно, что она чужая
- Пользователь проверил v0.3.0 на рабочем ноутбуке: комментарии из Интрасервиса приходят и показываются правильно,
  но глазом не отличаются от собственных заметок. Так и было: строка переписки заливалась `SubtleFill` — это 4%
  чёрного, — а рельс слева рисовался `ControlStrokeBottom`, 16% чёрного в 2 px. Рядом с плоской заметкой — ничто.
- Теперь строка переписки: рельс 3 px цветом акцента, заливка `AccentTint`, отступ 12 px слева. Заметки остались
  плоскими и вровень с краем, так что два списка расходятся и по цвету, и по левой границе. Автор — полужирным.
- В заголовке «Заметки» приписка «не уходят в Интрасервис»: приложение теперь много общается с сервером, и стоит
  сразу снять вопрос, куда попадают заметки.
- Проверено: сборка на CI. Не проверено: как это выглядит на самом деле — нужен взгляд пользователя, в обеих темах.

### 2026-09-22 — импорт заявок, где я исполнитель (API-IDEAS 1.1)
- Трей → «Импорт моих заявок» и кнопка в панели: `GET api/user?getcurrentuserinfo=true` → `GET api/taskstatus` →
  `GET api/task?ExecutorIds=…&StatusIds=…` постранично. Всё попадает во «Входящие», только вручную, только вниз.
- «Открытая» заявка — это не только признаки `IsFixed`/`IsFinal`: пользователь считает закрытым и «Ожидание ответа
  с автозакрытием», а такой статус сервер не пометит. Поэтому признаки **плюс** список названий в новой настройке
  `ClosedStatusNames` (правится руками в `settings.json`, дефолт — четыре названия от пользователя).
- Импорт идемпотентен: заявка, которая уже на доске, только считается в «уже было» — её не трогают вовсе.
- По итогам `/code-review high` (три находки, все исправлены): пересчёт колонок и перерисовка значка в трее теперь
  склеиваются (`QueueRecount` / `QueueTrayUpdate`) — раньше каждая добавленная заявка перерисовывала значок целиком,
  и импорт на 300 заявок подвесил бы UI; упёршийся потолок страниц больше не выглядит как полный импорт.
- Найдено попутно и записано в `AGENTS.md`: в WPF-UI 4.3.0 элементы `SymbolRegular` со значением выше `0xFFFF`
  (например `ArrowImport16` = `0xF0384`) не рисуются вообще — библиотека режет такой код при преобразовании в строку.
  Кнопка была бы пустой, и никакой ошибки. Взяли `ArrowDownload16`, глиф проверен прямо в шрифте.
- **Проверено:** сборка Release на Windows CI после каждого шага. **Не проверено:** ничего на живом сервере и ни один
  пиксель — ни `getcurrentuserinfo`, ни `api/taskstatus` в документации вообще не показаны в json, только xml.

### 2026-09-21 (вечер) — переписка из Интрасервиса и поиск на сервере
- **Переписка в панели.** `GET /api/tasklifetime` — комментарии и смены статуса по заявке, своя секция под «Заметками».
  Не один общий список с заметками намеренно: API отдаёт даты в часовом поясе пользователя Интрасервиса, а заметки
  живут в поясе компьютера — вперемешку они молча сортировались бы неправильно. Записи без комментария (большинство)
  спрятаны за кнопкой-глазом. Нигде не сохраняется: в памяти плюс кэш на 2 минуты, так что `tickets.json` от кликов по
  карточкам не переписывается и чужие имена с внутренними текстами на диск не попадают.
- **Поиск на сервере.** `Enter` в поле поиска → `GET /api/task?search=…`, который ищет и по полям заявки, и по тексту
  всех комментариев. Результаты в отдельном окне: ↗ в браузер, «+ На доску» через тот же путь, что и быстрое
  добавление. Заявка, уже лежащая на доске, погашена.
- Попутно исправлено: проверка фокуса поля поиска (`Keyboard.FocusedElement` → `IsKeyboardFocusWithin`, `ui:TextBox`
  составной — старая проверка, похоже, не срабатывала никогда, `Esc` в поиске тоже её использовал).
- **Как делалось:** разбор документации и план — отдельным агентом-планировщиком, три шага реализации — тремя агентами
  с непересекающимися наборами файлов (клиент → панель ‖ поиск), затем `/code-review high` по всему диапазону:
  четыре находки, три исправлены (дубль карточки при повторном добавлении, пустое окно на коротком запросе, вечный кэш
  переписки), четвёртая — эта самая документация.
- **Проверено:** сборка Release на Windows CI после каждого шага. Разбор дат прогнан вручную: формат из документации
  (`12.11.2015`) разбирается раньше общего, иначе инвариантная культура читает его как 11 декабря.
- **Не проверено:** всё, что касается живого сервера и любой UI. Форма json для `tasklifetime` в документации
  отсутствует (там xml), поэтому оба разборщика терпят три вида обёрток и пропускают негодные строки вместо падения.

### 2026-09-21 — семь замечаний после первого запуска на Windows
- **Заметки удаляются** (✕ в строке, появляется по наведению) и **приоритет меняется клавишами 1/2/3 на доске** — это было
  сделано ещё 13-го, но лежало в неслитой ветке, поэтому в сборке из `main` не работало. Ничего не переделывалось.
- **Данные переехали к exe** (`App.DataDir` = `AppContext.BaseDirectory`): папка с программой копируется целиком на другой
  ПК или флешку. Выбран этот вариант, а не `datadir.txt`, потому что данных у пользователя ещё нет и переносить нечего.
  Цена — exe должен лежать там, куда есть запись; при старте это проверяется и выдаётся одно понятное сообщение вместо
  падения сохранения раз в полсекунды. Пароль по-прежнему не переносится (DPAPI, пользователь+машина) — описано в README.
- **Голый номер заявки** в быстром добавлении: `702180` без решётки распознаётся, если в поле нет ничего кроме цифр
  (4–8). Регулярка из настроек не трогалась, иначе число в тексте («картридж 12345») стало бы номером. Ссылка для такой
  заявки собирается из базового адреса. Добавлена `IntraserviceLinkParser.SelfCheck`.
- **Цифры 1/2/3 в быстром добавлении теперь работают только при распознанной ссылке.** Раньше — «поле пустое или есть
  номер», и с вводом голого номера первая цифра номера уходила бы в приоритет: `123456` превращалось в `456`.
- **Вёрстка:** дата и текст заметки на одной базовой линии (`BlockLineHeight`); значение «Статуса» по центру (убрана
  жёсткая высота 28, которая ломала шаблон WPF-UI); окно быстрого добавления подгоняется по содержимому
  (`SizeToContent`) — раньше футер накрывал кнопки приоритета.
- **Проверено:** сборка Release на Windows CI после каждого коммита (последняя — run `35627811908`); `/code-review high`
  по всему диапазону, шесть находок, пять исправлено (шестая — миграция данных из `%APPDATA%` — отклонена: данных нет);
  разбор ссылок дополнительно прогнан на эквивалентной модели регулярок вне C#.
- **Не проверено:** ничего из UI. Нужен прогон по списку в «Open work» на Windows.

### 2026-09-13 — приоритет с клавиатуры, удаление заметок
- `1` / `2` / `3` на доске меняют приоритет выбранной заявки (`MainWindow.OnPreviewKeyDown` → `MainViewModel.SetSelectedPriority`).
  Раньше эти клавиши работали только в окне быстрого добавления, на доске не делали ничего.
- Крестик в строке заметки (виден при наведении на строку) удаляет её: `MainViewModel.DeleteNote`, стиль `NoteDeleteButton`.
  Без подтверждения — откат через ежедневный бэкап `tickets.json`.
- Проверено: сборка `dotnet publish -c Release` на CI (Windows) — зелёная (локально собрать не вышло: в контейнере агента нет
  .NET SDK, загрузка закрыта сетевой политикой). Поведение в UI не проверялось — нужен запуск на Windows.

### 2026-09-13 — agent docs
- Added `AGENTS.md` (code map, "change X → file" table, gotchas) and this `PROGRESS.md`. `CLAUDE.md` imports `AGENTS.md`.
- The todo checklist moved here from `TicketBoard/README.md`, so there's a single list.
- No code changes.

### 2026-09-12 — v0.1.0: CI, packaging, docs (`65fc377`, `3133130`, `5e79953`)
- GitHub Actions on `windows-latest`: exe artifact on each push/PR, Release on `v*` tags. Actions bumped off Node 20.
- Dropped the redundant `System.Security.Cryptography.ProtectedData` package (it ships with Windows Desktop and caused NU1510).
- The single-file exe is compressed (156 → 67 MB).
- Default hotkey changed to `Ctrl+Alt+Space`, because `Win+Shift+Space` is the Windows input-language switch.
- `Ctrl+Del` delete wired up (it was documented but never worked).
- Unobserved task exceptions are now shown. All errors are appended to `errors.log`.
- Docs split: root `README.md` is the user guide, `TicketBoard/README.md` holds the developer notes.

### 2026-09-12 — refactor (`33980d6`)
- Removed `IIntraserviceClient` and `NullIntraserviceClient`; a nullable `HttpIntraserviceClient?` is used instead (`null` = not configured).
- Removed the unused `TicketRules.HideDoneDays`.
- Added the Debug-only `SelfCheck` for `Parse`.

### 2026-09-12 — Intraservice API + settings window (`281769a`, `62ae2a9`, merge `01c4050`)
- `HttpIntraserviceClient`: Basic auth, `GET api/task/{id}?include=status`, 10 s timeout, errors mapped to short messages.
- Settings: login plus a DPAPI-encrypted password. The old `IntraserviceApiToken` was dropped (old files still load). Warning on http.
- Quick capture shows the ticket title (debounced). New tickets get title, description and status. Context menu «Обновить из Интрасервиса».
- `SettingsWindow` with inline validation and «Проверить подключение». Changes apply live on save.
- Merge fixes: the tray overload dot redraws when the limit changes; a stale lookup result is ignored.

### 2026-09-12 — tray badge (`a201c25`, merge `c8193e2`)
- The tray icon is rendered at runtime: a theme glyph, an Inbox count badge ("9+" cap) and a WIP overload dot. Counts ignore filters.
- The `System.Drawing.Icon` is built from a stream so it owns its HICON. H.NotifyIcon's `GeneratedIconSource` leaks handles.

### 2026-09-12 — card appear animation (`f1911c0`)
- 200 ms fade and slide on move or add, driven by an in-memory timestamp consumed once in `OnCardLoaded`.

### 2026-09-12 — initial import (`3d74d26` … `9ac0d5d`)
- Design handoff bundle from Claude Design («Трекер заявок»). It's removed from the tree but still in history: `git show 579b8b9 --name-only`.
- WPF sources unpacked into `TicketBoard/`.
