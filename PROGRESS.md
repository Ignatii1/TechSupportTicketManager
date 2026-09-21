# PROGRESS.md — state and history

Shared log for agents and humans. **Read before starting; append before finishing.** The map of the code is in `AGENTS.md`.

Rules:
- Update **Current state** and **Open work** in place. Don't let them go stale.
- Add a **Log** entry at the top of the log for each session or merged change: date, what changed and why, what was verified
  and how, and what's left. Keep entries short; the details belong in commit messages.
- Say what you *didn't* verify. For example, UI changes can't be run on the Linux dev machine.

## Current state (2026-09-13)

- Version `v0.1.0` is tagged at `3133130` (head of `main`). CI builds the exe on every push; the tag creates a Release.
- Feature-complete for daily use:
  - four-column board with drag&drop, arrow-key and context-menu moves;
  - age badges (warn/overdue), WIP limit;
  - search, priority filter, hide old Done;
  - detail panel with inline edit and notes;
  - quick capture from a global hotkey (`Ctrl+Alt+Space`) with clipboard prefill;
  - tray icon with Inbox badge and overload dot, autostart;
  - settings window applied live;
  - local JSON storage with atomic save, daily backups (30 kept) and corrupt-file quarantine;
  - errors.log.
- Intraservice API (read-only): title, description and status by ticket number, a connection check in settings.
  **Written from the v5.42 docs, never tested against a live server.**
- Verification so far: compile checks on Linux during development, and the CI build on Windows. `SelfCheck` covers `Parse`
  on doc-shaped samples. There's no test project and there are no UI tests, so UI behavior is verified only by the user running
  it on Windows.

## Open work

**Round 2026-09-21 — план и ревью в `PLAN.md`.** Семь замечаний после первого запуска на Windows: удаление заметок (T1)
и приоритет по 1/2/3 (T5) уже лежат в ветке `claude/beautiful-albattani-pd6q7i` и ждут проверки; остальное (T2 — переносимая
папка данных, T3 — базовая линия в строке заметки, T4 — центровка значения «Статус», T6 — перекрытие приоритетов футером в
быстром добавлении, T7 — ввод голого номера заявки) расписано по трём агентам.

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
