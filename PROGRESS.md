# PROGRESS.md — state and open work

Read before starting, update before finishing. Code map: `AGENTS.md`. Past rounds and their reasons: `docs/HISTORY.md` —
add a short entry there when you finish; don't read it unless you need the why.

## Current state (2026-09-26)

- **`v0.9.0` released**, `main` = the release. Board with drag&drop and keyboard; quick capture (hotkey, clipboard, bare
  ticket numbers); detail panel with notes and the ticket's Intraservice comments; server-side search (Enter in the search
  box); import of my open tickets; F5 refresh with an offer to move closed ones to «Готово»; API errors carry the server's
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
- **Verified by the user against the live server:** credentials, ticket title and status by number, comments
  (`api/tasklifetime`), import of my tickets and F5 refresh (0.6.0, 2026-09-24); 0.7.0 runs in the user's daily work
  (2026-09-26, no details yet). Everything else under Open work below is built and compiled but not yet seen running.
  **Not confirmed against the live server:** the field names `Creator`, `Executors`, `ExecutorGroup`, `Changed` (task
  and list) and `EditorId` (lifetime) — from the doc; if `Changed` is missing, errors.log says so once.
- **Verification available to agents:** local `dotnet build` and `TicketBoard.SelfCheck` (every parser assert, and the
  Claude relay end to end against a fake Intraservice on loopback) — see `AGENTS.md`. CI builds on Windows and publishes
  releases. The WPF UI and the clipboard listener can only be checked by the user on Windows.

## Open work

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
- [ ] Спросить пользователя: карточке в «Ждёт ответа», где заявитель ответил, самой возвращаться «В работу»?
- [ ] Дальше по `API-IDEAS.md` (2.1 сроки и 2.2 приоритеты сняты — в компании не заполняются). Если копировать-вставлять
  станет утомительно — расширение браузера, которое по кнопке вставляет ответ TicketBoard в поле чата (тот же протокол
  `TB`, без автоотправки). Запись в Интрасервис — только по решению пользователя.
- [ ] Панель деталей — кандидат на отдельный UserControl (`MainWindow.xaml`, 513 строк). Отложено: привязки и фокус без
  Windows не проверить.

Known limitations (deliberate, revisit only if they cause problems):
- `errors.log` is never rotated (one sample per failing method per run keeps it small).
- A corrupt `settings.json` is moved aside to `settings.json.corrupt-<ts>` (0.7.0; it used to be overwritten with
  defaults at once, URL and password included); the app starts with defaults and says so in a tray warning + errors.log.
- The exe is unsigned, so SmartScreen and AppLocker can block it (documented in the README).
- The password is DPAPI-bound to the Windows user and machine; after moving to another PC it has to be re-entered.
- Claude relay: no persistent data, but anything Claude asks for is pasted into claude.ai by the user — same exposure as
  pasting tickets by hand (README says so).
- Agents can't delete remote branches here (the permission is refused), so merged `claude/*` branches stay until the
  user deletes them on GitHub.
