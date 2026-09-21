# PROGRESS.md — state and history

Shared log for agents and humans. **Read before starting; append before finishing.** The map of the code is in `AGENTS.md`.

Rules:
- Update **Current state** and **Open work** in place. Don't let them go stale.
- Add a **Log** entry at the top of the log for each session or merged change: date, what changed and why, what was verified
  and how, and what's left. Keep entries short; the details belong in commit messages.
- Say what you *didn't* verify. For example, UI changes can't be run on the Linux dev machine.

## Current state (2026-09-21)

- Version `v0.1.0` is tagged at `3133130` (head of `main`). CI builds the exe on every push; the tag creates a Release.
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

- [ ] **Проверить на Windows переписку и поиск** (ветка `claude/beautiful-albattani-pd6q7i`). Ни одна из этих двух
      возможностей не проверялась на живом сервере — форма json для `tasklifetime` в документации вообще не приведена.
      1. `/` → слово → **Enter** открывает окно поиска. Самое вероятное место тихой поломки: раньше проверка фокуса
         сравнивалась с `Keyboard.FocusedElement`, теперь `IsKeyboardFocusWithin` (`ui:TextBox` — составной контрол).
      2. Искать слово, которое есть только внутри комментария, а не в названии — это и есть доказательство, что сервер
         ищет по переписке.
      3. Найденная заявка, которая уже на доске, погашена и подписана «уже на доске»; ↗ и двойной клик открывают
         нужную страницу; «+ На доску» заводит карточку, название подтягивается само.
      4. Выбрать карточку с номером — переписка появляется примерно за секунду. Пробежать стрелками по нескольким
         карточкам: запросы не копятся, под карточкой никогда не оказывается чужая переписка.
      5. Записи без комментария спрятаны, глаз их показывает, число в подсказке совпадает.
      6. Даты и авторы непустые и похожи на правду. **Пустые даты — пришлите один сырой ответ сервера**: значит формат
         даты не из трёх разобранных.
      7. Комментарий, помеченный в вебе как внутренний, показан с плашкой «внутр.».
      8. Отключить VPN и щёлкнуть карточку: в секции «сервер недоступен», без окна с исключением. И `tickets.json` не
         меняет время изменения, пока щёлкаешь по карточкам, — переписка никуда не сохраняется.
      9. Обе темы и панель в режиме наложения (окно уже 1100 px).
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
