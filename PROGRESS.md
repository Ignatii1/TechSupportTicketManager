# PROGRESS.md — state and open work

Read before starting, update before finishing. Code map: `AGENTS.md`. Past rounds and their reasons: `docs/HISTORY.md` —
add a short entry there when you finish; don't read it unless you need the why.

## Current state (2026-10-10)

- **`v0.14.0` released**, `main` = the release (0.14.0: the detail panel is its own component, `Views/TicketPanel.xaml`,
  with the same look and behaviour; roadmap stages 0 and 1). Tooling: SelfCheck by area, mutants in the repo (128 of
  128 caught by their own areas, rerun on the final runner rules), `coverage.py` (94%), a binding-aware `xamlcheck.py`,
  nullable warnings as build errors, LF pinned in `.gitattributes`. Board with drag&drop and keyboard; quick capture (hotkey, clipboard, bare
  ticket numbers); detail panel with notes and the ticket's Intraservice comments; the «Поиск заявок» window (any tickets by
  any filters, preview, copy, export — now also **all tickets the account can read**; Enter in the board's search box opens it); import of my open tickets; F5 refresh with an offer to move closed ones to «Готово»; API errors carry the server's
  own response; data next to the exe. Read-only towards Intraservice.
- **Claude via the clipboard (0.6.0):** the user has claude.ai **Pro only — no API key, ever**. Claude asks for data with a
  block of `TB …` lines, the user clicks Copy, TicketBoard answers into the clipboard, the user pastes. Off by default:
  Settings → Claude (+ «Скопировать инструкцию для Claude» for the claude.ai project). The 0.5.0 API-key chat page is gone.
- Confirmations and reports use the app's own dialog (`AskWindow`, 0.5.0) instead of the system MessageBox.
- **Auto-sync (0.7.0):** every 5 min (setting, 0 = off) the tray does what import and F5 do, without dialogs: new
  assignments → «Входящие» + notification, statuses → cards, closed → notification whose click asks the F5 question
  (once, when the status turns closed — also while the PC was off); the board title keeps counting closed cards not
  yet in «Готово». A new server URL or login restarts it as a first run.
  Adds only assignments made after its first run; never re-adds a card the user deleted (`AutoSyncSkipIds`).
- **Who's on the ticket (0.8.0):** the panel shows «Инициатор» and «Исполнители» (+ группа), refreshed by every sync;
  the relay's `TB ticket` includes them. The user asked for «Исполнители».
- **Unread comments (0.8.0):** auto-sync re-reads the comments of tickets whose `Changed` moved; comments by others
  after the last one the user saw light a badge on the card and raise a notification (click → the ticket). Seen = shown
  in the open panel of the active board window, or the user's own reply.
- **Ticket lifecycle (0.9.0):** a card in «Готово» whose ticket comes back into my open list (reopened, or given back to
  me) returns to «Входящие» with a notification; a ticket that leaves my list while still open → «больше не на вас»
  (card stays). Driven by `Ticket.AssignedToMe` + `AutoSyncRules.Track`; the account this memory belongs to is
  `AutoSyncAccount` in settings.json (a URL/login change, even by hand between runs, restarts it silently). The panel
  also shows the requester's phone and email under «Инициатор».
- **Export for the knowledge base (0.10.0):** the user is building a personal knowledge base (Obsidian vault in a private
  GitHub repo) to help solve tickets. One Markdown file per ticket (YAML properties, description, the whole conversation
  oldest-first) in `<folder>/tickets` + `_index.md`. Incremental: unchanged tickets are skipped without requests.
  Contacts are never exported. `Services/KnowledgeExport.cs`. 0.11.0: service, type, categories and group come from each
  changed ticket's card (`api/task/{id}`; the live list rows don't carry them), format 2.
- **Search any tickets (0.12.0):** the user expects read-only access to all tickets soon and asked for full search + download
  (2026-10-03). Window «Поиск заявок» (toolbar button, tray, Enter in the board's search box — the old search and export
  windows are gone): conditions (words, «я»/name of executor, requester name, status any/closed/open/one, service with
  children, task type, created/changed/closed periods, saved web filter, include archived/inactive services) → list of 50
  with «ещё» → preview of the exact Markdown an agent gets → «Копировать» (clipboard, ≤ 30 tickets) or files (found or
  selected, into the knowledge folder). Parameter names are from `IntraService_API_v5_51.pdf` pp. 14-20 (the user supplied
  the PDF 2026-10-03). Conditions are remembered in `settings.json` (`LastSearch`; the old `LastExport` is ignored).
  `Services/TaskQuery.cs`, `TicketSearch.cs`, `ViewModels/SearchViewModel.cs`. Not in the window, by decision: executor
  group (the API lists groups only per service), categories, priorities (the company doesn't fill them) — a saved filter
  from the web UI covers all of those.
- **Export of everything (0.13.0):** the user got read-only rights to all tickets specifically for a mass export and asked
  whether the program lets him pick tickets he isn't executor of (2026-10-07). Search always did (no conditions = every
  ticket the account sees); the export did not (cap 2000, 11 list pages, whole list in memory, one flat folder, one huge
  index). Now «Не больше, заявок» **0 = all**, no ceiling: list pages of 200 sorted `Created asc, Id asc` are exported page
  by page (memory = a page + file names), files go to `tickets/<yyyy-MM>/` with an index per month and one in the root, the
  old flat folder moves itself, a run of ≥ 2000 asks first, progress shows «Выгружено X из N · осталось ~…». Stop / crash /
  PC asleep → the next run with the same conditions skips what is current (that is the whole resume mechanism); failed
  requests are retried (2, 5, 15 s) and the run trips on 401, 30 network failures in a row, 25 refusals with no success,
  10 write failures in a row. A list page has 60 s instead of 10 (per-request timeouts now). Read the why in HISTORY.
- **Export of «my tickets» on the live server (2026-10-09) → 0.13.2:** the user saw «Найдено: 260», «По отбору — 259: новых
  файлов 259 … Список закончился раньше, чем обещал сервер (259 из 260)», and only 245 files on disk (also in a fresh
  Downloads folder). Code can't lose files in a fresh folder (unique names, no deletes), so the 14 are most likely removed
  after writing — antivirus quarantine is the suspect, unconfirmed. 0.13.2: every written file is checked a page later and
  at the end (after a 3 s wait), the summary names the vanished ones; a second list pass (Changed desc, only unseen
  tickets exported) runs when the first saw repeats at page boundaries, fell short of an exact count, or the count is
  «1 000+» (always then: a shift can't be seen otherwise). The missing 260th is either a boundary loss (fixed by the
  pass) or an inflated count (now said so).
- **First live run of the search window (2026-10-08) → 0.13.1:** the user reported «ошибка сервера (HTTP 400):
  {"errors":{"count":["The value 'all' is not valid."]},…}» on «Найти» and an empty «Сервис» list. The live server is an
  ASP.NET Core build that binds `count` as a bool (the PDF's `count=all` is refused). 0.13.1: the search uses the default
  count — a total of exactly 1000 (the doc's ceiling) shows as «1 000+», and the export button, its question and the
  progress don't invent a number then («Выгружено X · ~N в минуту»); export pages after the first ask `count=false` and go
  by `HasNextPage` (a 400 on that repeats the page with the count). Services: the full list refused **or empty** → the ones
  the user is assigned to → the services of the 1000 latest tickets (a transient error or 401 retries at the next open
  instead); the note under «Сервис» says which list it is and why (the actual reason on the user's server is still
  unknown); other lists' failures now show at the top of the conditions. An export whose list ends exactly at the counting
  ceiling without the server ever sending `HasNextPage` is reported unfinished with a note (likely cut by the server).
  `FakeIntraservice` refuses `count=all` like the live server.
- **Requester reply (0.11.0):** a new unread comment by the ticket's requester (matched by name) moves a «Ждёт ответа»
  card back to «В работе» with an «Ответ инициатора» notification — the user asked for it (2026-10-02). Not for closed
  or no-longer-mine tickets; a colleague's or the system's comment only lights the badge; a reply already read in the
  panel leaves the card where it is.
- **Verified by the user against the live server:** credentials, ticket title and status by number, comments
  (`api/tasklifetime`), import of my tickets and F5 refresh (0.6.0, 2026-09-24); 0.7.0 runs in the user's daily work
  (2026-09-26, no details yet). The 0.10.0 export runs on the live server (2026-10-02, sample: closed #692784 with 12
  lifetime records): list rows carry `Creator`, `Executors`, `Created`, `Changed`, `ResolutionDateFact`, `Description`;
  lifetime records — author, status changes and comments, oldest-first order right. The user's raw `api/task/692784`
  response (2026-10-02) confirmed the card's shape and names: `ServiceName`, `Type` (not `TypeName`), `Categories`,
  `ExecutorGroup`, `ResolutionDateFact`, `Changed`, `StatusName` — sample in `HttpIntraserviceClient.SelfCheck`
  (values replaced). Everything else under Open work below is built and compiled but not yet seen running.
  **Not confirmed against the live server:** `EditorId` (lifetime) — from the doc. For the export also: `search`
  combined with `ExecutorIds`/`StatusIds` in one query, `page=` on `api/tasklifetime` (the sample fit on one page).
  **0.12.0, all from the doc and unseen live:** the new list conditions (`CreatorIds`, `ServiceIds`, `TypeIds`, the date
  ones — sent as `yyyy-MM-dd HH:mm`, the doc's example format; a wrong format shows as the window's «сервер не применил
  условие по дате» warning, and the export drops such rows), ~~`count=all`~~ (refused live, 0.13.1), `include=status,service` and the
  `Services` block of a list (service names in the results), `archive=true&inactive=true`, `filterid` together with an
  explicit `fields=` list, and the reference lists `api/service`, `api/tasktype`, `api/filter?resource=task`,
  `api/user?search=` (response wrappers are guessed and tolerated: bare array, `{"Users": […]}`, `{"UserList": {…}}`).
- **Verification available to agents:** local `dotnet build` and `TicketBoard.SelfCheck` (every parser assert, and the
  Claude relay, the export, the search window and — since 2026-10-09 — the board's import, F5 and auto-sync passes with
  their limits, the card and panel by hand (quick capture, ⟳, comments, notes, filters, the actions behind the keys), the
  ticket store (backups, corrupt file, the file's field names) and the settings form end to end against a
  fake Intraservice on loopback, with the real view models) — see `AGENTS.md`. CI builds on Windows and
  publishes releases. The WPF UI, the tray, notifications and their clicks, timers and the clipboard listener can only be
  checked by the user on Windows.

## Open work

- **The agreed plan is `docs/ROADMAP.md`** (updated 2026-10-10 with the user's answers; **work starts on his go-ahead**):
  0 — process (TDD made explicit, mutants into the repo, coverage, analyzers), 1 — the detail panel as its own component
  (0.14.0), R — raw responses (Monday; the URL list is in the plan), 2 — auto-export of closed tickets where I am
  executor, initiator or participant (0.15.0), 3 — the knowledge base made for the agent: `SCHEMA.md`, `tickets.csv`, a
  starter `CLAUDE.md`, a measured question set (0.16.0), 4 — «Спросить базу знаний» from the panel (0.16.x), 5 —
  attachments and the full card (0.17.0), 6 — enrichment by agents and local models, 7 — export of everything readable,
  8 — writes to Intraservice (1.0). The knowledge base is the user's private GitHub repo, worked on by agents (Claude Code
  in the cloud now; Claude Desktop and an RTX 3060 on his personal PC); ticket texts in the cloud are allowed.
- The user's answers (2026-10-10): Obsidian Git pushes the vault to GitHub every minute and the export folder is the
  repo root, so TicketBoard never runs git; attachments get no size cap (files from 50 MB go to Git LFS, stage 5);
  «participant» = any action of mine in the history. Monday: the raw responses (R) and tickets for the question set.
- Not covered by any check (from `coverage.py`, 2026-10-10): drag&drop of cards (`ColumnViewModel` as `IDropTarget`,
  half of the file) — the gong interfaces are stubbed, so a check can build an `IDropInfo` and drive `DragOver`/`Drop`;
  `AppSettings` load fallbacks and parts of the HTTP error paths. Candidates for checks when those areas are touched.
- 2026-10-10, the user: tray notifications of auto-sync do arrive; clicking them hasn't been tried yet (0.7.0 checklist).
- The board's logic in the items below (what a pass adds, notifies, moves or leaves; import; F5 and its question; quick
  capture, the panel's comments and people, filters and counters) is now checked by `BoardCheck` and `CardCheck` on every
  SelfCheck run. What stays for Windows is what only Windows shows: the 15 s / N-minute timers firing, tray notifications
  and whether their clicks arrive, rendering of the card badge and panel, focus and the input hooks that mark comments
  seen, the key mapping itself (`MainWindow.OnPreviewKeyDown`), drag&drop, the hotkey parser (on WPF types) and the
  password's DPAPI encryption in the settings window.
- [ ] **Не проверено на Windows** (v0.14.0), панель деталей — теперь отдельный компонент, должна быть точно как раньше:
  открывается выбором карточки, `Enter` открывает и закрывает, `Esc` и крестик закрывают; выезд справа за 0,2 с —
  содержимое стоит на месте и открывается справа налево, как было (не «едет»); уже 1100 px — поверх доски с тенью и
  затемнением, щелчок по затемнению закрывает; название, ссылка и описание правятся; статус (список) и приоритет (кнопки) меняются; люди и
  контакты выделяются и копируются; заметка — `Enter`, крестик удаления при наведении; переписка — глаз и ⟳, чипы
  «внутр.» и статуса; обе темы.
- [ ] **Не проверено на Windows** (v0.13.3): первое открытие быстрого добавления после запуска (буфер без ссылки) — под
  полем подсказка «Ссылка вида …/Task/View/702180 или просто номер заявки» (до 0.13.3 там было пусто до первого ввода).
- [ ] **Не проверено на Windows** (v0.4.1):
  - поиск на сервере: `/` → слово → `Enter` открывает окно; слово, которое есть только в комментарии, находится;
  - ошибки API с настоящим ответом сервера — как выглядят, копируются ли;
  - переписка с акцентным рельсом — в обеих темах.
- [ ] **Не проверено на Windows** (v0.5.0):
  - `AskWindow`: удаление (`Ctrl+Del`, меню карточки) — окно по центру доски, красная «Удалить», `Enter`/`Esc`; из трея
    (доска скрыта) F5 и импорт — окно по центру экрана и не прячется за другими; длинный текст прокручивается и копируется.
- [ ] **Не проверено на Windows** (v0.6.0), Claude через буфер: галочка → «Сохранить»; инструкция копируется и
  вставляется в проект claude.ai; Claude пишет блок `TB …` → «Copy» → уведомление «Ответ для Claude — в буфере» →
  `Ctrl+V` вставляет ответ; обычное копирование (текст, файлы, картинки) ничего не вызывает; галочка снята — буфер
  не слушается. Дальше — как Claude справляется с протоколом на живых вопросах (правится `ClaudeRelay.Instructions`).
- [ ] **Не проверено на Windows** (v0.7.0), автообновление: через ~15 с после запуска в заголовке «обновлено ЧЧ:ММ»;
  первый запуск ничего не добавляет; новая заявка на вас → во «Входящих» + уведомление, щелчок открывает доску;
  закрыли заявку в Интрасервисе → уведомление, щелчок → вопрос «Перенести закрытые…»; удалённая карточка не
  возвращается; 0 в настройках — заголовок без «обновлено»; щелчок по уведомлению вообще доходит (Windows 10/11);
  закрытая, но не перенесённая карточка — «закрыты в Интрасервисе: 1 — F5» в заголовке, пропадает после переноса
  и после «Оставить» (и не возвращается после перезапуска);
  опечатка в settings.json → уведомление «Настройки не прочитаны» и файл `settings.json.corrupt-…`.
- [ ] **Не проверено на Windows** (v0.8.0): в панели «Инициатор» и «Исполнители» (+ «группа: …»), у старой карточки —
  после первого открытия; имена выделяются и копируются. Новые комментарии: кто-то пишет в заявку с доски → через ≤ 5 мин
  синий значок с числом на карточке + уведомление «Новый комментарий в #N» с началом текста; щелчок открывает заявку,
  значок гаснет; свой ответ в веб-интерфейсе значок гасит; доска в трее или за браузером — значок остаётся до открытия;
  карточка уже открыта в панели — новый комментарий появляется в переписке, значок гаснет от щелчка по доске;
  бейдж узкий, не на всю строку; `TB ticket` показывает инициатора и исполнителей.
- [ ] **Не проверено на Windows** (v0.9.0): заявку из «Готово» переоткрыли в Интрасервисе → через ≤ 5 мин карточка во
  «Входящих» сверху + уведомление «Заявку открыли снова»; вернул её в «Готово» руками — там и остаётся; сняли вас с
  заявки (открытой) → уведомление «Заявка больше не на вас» с новым исполнителем, карточка на месте; первый запуск
  0.9.0 таких уведомлений не шлёт; под «Инициатором» — телефон · почта (если заполнены), выделяются.
- [ ] **Не проверено на Windows** (v0.11.0): первая выгрузка после обновления переписывает все файлы, в них `service:` и
  `type:` (у #692784 — «Приложение Mobile Mark», «Запрос на обслуживание»), в шапке «Сервис: … · тип: …»; повторная —
  снова «без изменений». Карточка в «Ждёт ответа», инициатор ответил → через ≤ 5 мин она «В работе» сверху,
  уведомление «Ответ инициатора в #N» с его словами, щелчок открывает заявку; комментарий коллеги — только значок;
  «спасибо, можно закрывать» и заявку закрыли — карточка не переезжает «В работу».
- [ ] **Не проверено на Windows и на живом сервере** (v0.13.2): выгрузка «моих» ещё раз в пустую папку — что в итоге:
  «Записано, но уже нет на диске: N (#…)» (тогда в журнале антивируса — эти файлы; добавить папку в исключения) или нет;
  «Второй проход по списку нашёл заявок, которых не было в первом: 1…» или «Сервер насчитал 260, а разных заявок в списке
  259…» — прислать итог целиком. Сколько файлов на самом деле: «Проводник» → папка `tickets` → «Свойства» (минус по
  одному `_index.md` на месяц) или корневой `_index.md` («заявок: N»).
- [ ] **Не проверено на Windows и на живом сервере** (v0.13.1, после отказа сервера на `count=all`): «Найти» без ошибки;
  «Найдено» — точное число или «1 000+» (сказать, какое: так видно, считает ли сервер по умолчанию только до тысячи);
  под «Сервис» — подпись: нет её — пришёл весь справочник; «…здесь те, на которые вы назначены» / «…здесь сервисы последних
  N заявок» / «Список сервисов не загрузился: …» — прислать текст (и `errors.log`); выгрузка всех идёт дальше второй
  страницы (`count=false`; если в итоге «Сервер не принял список без счёта» — тоже прислать), ход «Выгружено X из N ·
  осталось ~…» или «Выгружено X · ~N в минуту»; вопрос перед выгрузкой «1 000+» — «Выгрузить все найденные заявки?».
- [ ] **Не проверено на Windows и на живом сервере** (v0.13.0), выгрузка всех: «Сбросить» → «Найти» — над списком «Условий
  нет: ищу среди всех заявок…», «Найдено» — ожидаемое число или «1 000+» (первая страница списка — 60 с на ответ); «Не больше, заявок» `0` → на кнопке «Выгрузить найденные (N)» → вопрос (от 2000) с
  числами; идёт: «Выгружено X из N · осталось ~…» (оценка по последним полуминутам — после запуска появляется не сразу),
  файлы в `tickets\ГГГГ-ММ\`, оглавления месяцев и общее; «Остановить» → повторная выгрузка с теми же условиями идёт
  быстро по готовому (только страницы списка) и продолжает; старая плоская `tickets\` переезжает по месяцам
  (в итоге «Файлы прежней раскладки переложены…»); **сервер принял сортировку `Created asc, Id asc`** (иначе в итоге
  «Сервер не принял сортировку…» — прислать, что окно пишет, и ответ сервера из `errors.log`) и действительно отдаёт по
  возрастанию создания (иначе «отдал список не по дате создания»); сколько заявок в секунду на живом сервере (от этого
  зависит «часы»); как ведут себя Проводник и Obsidian на сотнях тысяч файлов; память приложения на очень большой папке
  (имена всех файлов держатся в словаре). Ушло ли что-то в `errors.log`.
- [ ] Выгрузка всех — `tickets.csv`: этап 3 в `docs/ROADMAP.md`. Пропуск запроса карточки, если когда-нибудь строки
  списка начнут нести сервис, тип, категории и дату решения (сейчас по ним не отличить «нет» от «не прислали»).
- [ ] **Ждём от пользователя** (пришлёт в понедельник, с работы): сырые ответы `…/api/task?pagesize=2` (какие поля в
  строке списка: есть ли `ServiceId`, `Type`) и `…/api/filter?resource=task` (форма списка фильтров) — закрепить образцы
  в `SelfCheck` (этап R в `docs/ROADMAP.md`).
- [ ] **Не проверено на Windows** (v0.12.0), окно «Поиск заявок» (кнопка «документ с лупой» на доске, трей, `Enter` в поле
  поиска на доске): открывается без ошибки (иначе — ключ ресурса, см. `xamlcheck.py`); списки сервисов, типов, сохранённых
  фильтров и статусов заполняются, запомненные условия на месте после перезапуска; поиск «Я + Закрытые» даёт то же, что
  выгрузка 0.11.0; слова, фамилия исполнителя и заявителя (несколько Ивановых — над списком написано, кого нашли),
  сервис с вложенными, тип, периоды дат (граница «по» включает последний день), сохранённый фильтр; найдено N (или
  «1 000+») — и N верное; «Показать ещё»; у строк сервис · тип; множественный выбор `Ctrl`/`Shift`; просмотр под списком —
  текст как в файле; «Копировать» → вставить в чат Claude (выбрано 1–30); «Выгрузить выбранные» и «найденные» в папку
  Obsidian, повторная — «без изменений»; поменял условие после поиска — «найденные» выключено до нового «Найти»; «Остановить»;
  Esc прячет окно, выгрузка идёт дальше (а при открытом выпадающем списке — закрывает список, не окно); двойной клик по
  строке — браузер, «+ На доску» (карточка с названием сразу); поиск с доски не меняет «привычные» условия; на кнопке
  «Выгрузить найденные» видно «500 из 1200», когда найдено больше потолка; Enter в тексте о не загрузившихся справочниках
  не запускает поиск. Если после ввода дат висит
  жёлтое «сервер не применил условие по дате» — формат даты в запросе не тот: прислать, что окно пишет, и ответ сервера
  из `errors.log`.
- [ ] База знаний (план пользователя — модель с базой знаний разбирает новые заявки), автовыгрузка закрытых, пункты
  `API-IDEAS.md` (2.1 сроки и 2.2 приоритеты сняты — в компании не заполняются), панель деталей отдельным компонентом —
  этапы 1–6 в `docs/ROADMAP.md`; там же «потом / на решение пользователя» (запись в Интрасервис, Claude Desktop + MCP,
  база целиком в проекте claude.ai). Если копировать-вставлять станет утомительно — расширение браузера, которое по
  кнопке вставляет ответ TicketBoard в поле чата (тот же протокол `TB`, без автоотправки).

Known limitations (deliberate, revisit only if they cause problems):
- `errors.log` is never rotated (one sample per failing method per run keeps it small).
- A corrupt `settings.json` is moved aside to `settings.json.corrupt-<ts>` (0.7.0; it used to be overwritten with
  defaults at once, URL and password included); the app starts with defaults and says so in a tray warning + errors.log.
- Lifetime records are de-duplicated by (date, author, status, comment) in the export (a safeguard against a server that
  ignores `page`): two identical posts by one author within a second become one in the file.
- The exe is unsigned, so SmartScreen and AppLocker can block it (documented in the README).
- The password is DPAPI-bound to the Windows user and machine; after moving to another PC it has to be re-entered.
- Claude relay: no persistent data, but anything Claude asks for is pasted into claude.ai by the user — same exposure as
  pasting tickets by hand (README says so).
- Agents can't delete remote branches here (the permission is refused), so merged `claude/*` branches stay until the
  user deletes them on GitHub.
