# PROGRESS.md — state and open work

Read before starting, update before finishing. Code map: `AGENTS.md`. Past rounds and their reasons: `docs/HISTORY.md` —
add a short entry there when you finish; don't read it unless you need the why.

## Current state (2026-09-22)

- **`v0.5.0` released**, `main` = the release. Board with drag&drop and keyboard; quick capture (hotkey, clipboard, bare
  ticket numbers); detail panel with notes and the ticket's Intraservice comments; server-side search (Enter in the search
  box); import of my open tickets; F5 refresh with an offer to move closed ones to «Готово»; API errors carry the server's
  own response; data next to the exe. Read-only towards Intraservice.
- **New in 0.5.0:** confirmations and reports in the app's own dialog (`AskWindow`) instead of the system MessageBox;
  **the Claude bridge** — a chat page served on `127.0.0.1` where Claude (Opus 5, via the user's API key, from Brave)
  searches and reads tickets through TicketBoard. Off by default: Settings → Claude.
- **Verified by the user against the live server:** credentials, ticket title and status by number, comments
  (`api/tasklifetime`). Everything else under Open work below is built and compiled but not yet seen running.
- **Verification available to agents:** local `dotnet build`, `TicketBoard.SelfCheck` (every parser assert, and the
  bridge over a real loopback socket) and `TicketBoard.SelfCheck/page/run.sh` (the chat page end to end in headless
  Chromium, Anthropic API mocked) — see `AGENTS.md`. CI builds on Windows and publishes releases. The WPF UI itself can
  only be checked by the user on Windows; the page has never talked to the real Anthropic API.

## Open work

- [ ] **Не проверено на Windows** (v0.4.1):
  - поиск на сервере: `/` → слово → `Enter` открывает окно; слово, которое есть только в комментарии, находится;
  - импорт «моих заявок»: закрытое не приезжает (приехало — дописать статус в `ClosedStatusNames`), второй запуск ничего
    не добавляет, кнопка со стрелкой в панели не обрезается на узком окне;
  - F5: в вопросе список закрытых, перенос только по «Перенести», «Оставить» помнится до перезапуска;
  - ошибки API с настоящим ответом сервера — как выглядят, копируются ли;
  - переписка с акцентным рельсом — в обеих темах.
- [ ] **Не проверено на Windows** (v0.5.0):
  - `AskWindow`: удаление (`Ctrl+Del`, меню карточки) — окно по центру доски, красная «Удалить», `Enter`/`Esc`; из трея
    (доска скрыта) F5 и импорт — окно по центру экрана и не прячется за другими; длинный текст прокручивается и копируется;
  - мост: галочка → «Сохранить» → нет запроса брандмауэра; ссылка из «Копировать» открывается в Brave, статус
    «Интрасервис подключён»; живой вопрос с настоящим ключом — поиск, заявка, переписка, цена под ответом; порт занят —
    уведомление в трее.
- [ ] Дальше по `API-IDEAS.md` (2.1 сроки и 2.2 приоритеты сняты — в компании не заполняются). Мост — кандидаты, если
  пригодится: история разговоров между перезагрузками страницы, выбор модели. Запись в Интрасервис — только по решению
  пользователя.
- [ ] Панель деталей — кандидат на отдельный UserControl (`MainWindow.xaml`, 513 строк). Отложено: привязки и фокус без
  Windows не проверить.

Known limitations (deliberate, revisit only if they cause problems):
- `errors.log` is never rotated (one sample per failing method per run keeps it small).
- A corrupt `settings.json` silently falls back to defaults and isn't overwritten until the next save.
- The exe is unsigned, so SmartScreen and AppLocker can block it (documented in the README).
- The password and the bridge key are DPAPI-bound to the Windows user and machine; after moving to another PC the
  password has to be re-entered and the bridge link is new.
- The Anthropic API key lives in the browser's localStorage for the page's origin (plain text in the Brave profile).
- Agents can't delete remote branches here (the permission is refused), so merged `claude/*` branches stay until the
  user deletes them on GitHub.
