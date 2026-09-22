# PROGRESS.md — state and open work

Read before starting, update before finishing. Code map: `AGENTS.md`. Past rounds and their reasons: `docs/HISTORY.md` —
add a short entry there when you finish; don't read it unless you need the why.

## Current state (2026-09-22)

- **`v0.4.1` released**, `main` = the release. Board with drag&drop and keyboard; quick capture (hotkey, clipboard, bare
  ticket numbers); detail panel with notes and the ticket's Intraservice comments; server-side search (Enter in the search
  box); import of my open tickets; F5 refresh with an offer to move closed ones to «Готово»; API errors carry the server's
  own response; data next to the exe. Read-only towards Intraservice.
- **Verified by the user against the live server:** credentials, ticket title and status by number, comments
  (`api/tasklifetime`). Everything else under Open work below is built and compiled but not yet seen running.
- **Verification available to agents:** local `dotnet build` and `TicketBoard.SelfCheck` (every parser assert, executed) —
  see `AGENTS.md`. CI builds on Windows and publishes releases. The UI itself can only be checked by the user on Windows.

## Open work

- [ ] **Не проверено на Windows** (всё уже в v0.4.1):
  - поиск на сервере: `/` → слово → `Enter` открывает окно; слово, которое есть только в комментарии, находится;
  - импорт «моих заявок»: закрытое не приезжает (приехало — дописать статус в `ClosedStatusNames`), второй запуск ничего
    не добавляет, кнопка со стрелкой в панели не обрезается на узком окне;
  - F5: в вопросе список закрытых, перенос только по «Да», «Нет» помнится до перезапуска;
  - ошибки API с настоящим ответом сервера — как выглядят, копируются ли;
  - переписка с акцентным рельсом — в обеих темах.
- [ ] Дальше по `API-IDEAS.md`: настоящие сроки вместо счётчика дней (2.1), приоритеты с сервера (2.2); мост, чтобы Claude
  мог искать по заявкам (CLI-режим поиска — см. `docs/HISTORY.md`, 21.09). Запись в Интрасервис — только по решению
  пользователя.
- [ ] Панель деталей — кандидат на отдельный UserControl (`MainWindow.xaml`, 513 строк). Отложено: привязки и фокус без
  Windows не проверить.

Known limitations (deliberate, revisit only if they cause problems):
- `errors.log` is never rotated (one sample per failing method per run keeps it small).
- A corrupt `settings.json` silently falls back to defaults and isn't overwritten until the next save.
- The exe is unsigned, so SmartScreen and AppLocker can block it (documented in the README).
- The password is DPAPI-bound to the Windows user and machine; after moving to another PC it has to be re-entered.
