# The Rust prototype

`rust/` — see also `rust/README.md` (files, commands). The C# app is its oracle: a port keeps the C# behaviour rule by
rule.

## The Rust prototype (`rust/`, 2026-11-06; layer 1 — client and rules — `rust-0.2.0`; layer 2.1 — the board writes — `rust-0.3.0`; layer 2.2 — F5, import, the quiet read — `rust-0.4.0`; layer 3.1 — the toolbar and the board's search — `rust-0.5.0`)

`rust/README.md` (files, commands). Layer 1: `src/intraservice/` (`parse.rs` = `.Parse.cs`, `html.rs` =
`HtmlToText`/`PageText`/`HtmlDecode` as hand-written scanners with .NET's regex semantics — keep them linear,
`errors.rs`, `charset.rs`, `query.rs` = `TaskQuery.ToUrl`, `mod.rs` = the client: `get`/`refusal`/`unparsed`, `Log`,
`my_open`), `src/rules.rs` (`AutoSyncRules`, `ClosedNames`), `src/link.rs` (`regex-lite`); a C# parsing change gets
its sample in `rust/tests/` too while both live. Data: `src/board.rs` (`tickets.json` in the C# format — every field
of `Ticket` in the order STJ writes them (`Notes` third: hand-written properties come before the generated ones —
checked against the real `TicketStore.Save`), unknown fields kept in `other` and written back, `Files: null` ≠ `[]`,
statuses read like `JsonStringEnumConverter` (any case, numbers), dates like STJ (`date` module), a
missing/repeated/non-Guid `Id` or note `Id` gets a new Guid (`new_guid`, from `RandomState`); rules copied from
`Ticket`/`Matches`/`ColumnViewModel`/`MainViewModel`: age, hidden «Готово», WIP, `MoveTo`, `Filter` +
`Ticket::matches` (3.1: `Matches` — words in title/description/number/url/notes, priority, old «Готово» hidden while
on; `visible`/`overloaded`/`hint` take it; in memory), `one_line` (titles), `Board` = owned tickets per column —
move/shift/insert/remove on top like `MoveTicket`/`Insert`, notes, `delete_question`; `load_tickets` decodes like
`File.ReadAllText` — BOM, U+FFFD), `src/store.rs` (`TicketStore`: `.tmp` + rename, a daily copy of the previous file
in `backups/`, 30 kept by name; `tickets_json` errs rather than write `[]`), `src/single.rs` (the mutex
`Local\TicketBoard.SingleInstance`: owner → writes; else, or the file unreadable, read-only and the mutex released so
TicketBoard.exe can start), `src/journal.rs` (`errors.log` like `App.AppendLog`, only when the board is ours — the
client's `Log`, save failures, the panic hook), `src/capture.rs` (`QuickCaptureViewModel` state + `AddFromCapture`),
`src/sync.rs` (`Apply`/`ApplyCard`), `src/refresh.rs` (2.2: `ImportMine`/`NewCard`, `RefreshAll`'s rules — what to
re-read, `ClosedNames` with the directory's flags, the summary, `ClosedToMove` in board order, the move question,
«Оставить»/«Перенести»), `src/hotkey.rs` (`HotkeyService.TryParse` names), `src/clipboard.rs` (Win32 text),
`src/settings.rs` (`settings.json` incl. comments and trailing commas — `strip_jsonc`, one pass), `src/dpapi.rs`
(CurrentUser, no entropy, base64). Network: `src/intraservice.rs` (`GET api/task/{id}?include=status`, ureq 3 +
native-tls with the Windows store — `RootCerts::PlatformVerifier`, never the bundled roots; redirects not followed;
errors in the C# phrases, bodies through `redact` — secrets and any `Basic <token>`; `system_proxy` reads the WinINet
registry, direct connection). Window: `ui/board.slint` (Slint 1.18, Fluent, colours from `Tokens.*.xaml`; drag & drop =
`DragArea`/`DropArea` with in-memory `DataTransfer` user data — always an in-window drag), `src/app.rs` (models,
selection, edits through `App::edit` with a redraw `Scope` — one card row or the board, saves 600 ms later
(`SAVE_DELAY`) and on exit, the right panel (`PanelData`, `panel-status` bound both ways), the capture window, threads
answer through a `Message` queue drained by `deliver`, links only http(s) via `ShellExecuteW`; 2.2: import and F5 on
threads (F5 4 at a time, the directory alongside, results in board order) with busy flags, the dialog `Ask`/`Dialog` —
a window that arrives while another is open waits in `waiting`; the quiet read — `look_up` on one restartable
`lookup_timer` (400 ms, `PANEL_PAUSE`), `card_read` = `CardReadAt` in memory (`CARD_FRESH`, marked at the start of ⟳
and of the quiet read, put back if the read fails — `Mark`), the answer dropped if the client changed meanwhile),
`src/main.rs` (window, `SystemTrayIcon`, the settings hotkey → quick capture when the board is ours,
Ctrl+Alt+Shift+Space → the board when read-only, a message box on panic). **Read-only writes nothing** — checked in
`tests/ui.rs` (the folder byte for byte, an unreadable file too). Tests: `tests/*.rs` (`cargo test`; the window
through `MinimalSoftwareWindow` with a queue as the event loop; Windows-only parts run on CI).

## Notes and gotchas

- **Rust mutants** (`cargo-mutants`, copy mode): `-C=--test -C=<name>` (or `-C=--lib`) per check — without it every
  mutant builds all 14 test targets and the example (~1 min instead of ~8 s); `CARGO_PROFILE_DEV_DEBUG=0` speeds
  linking; `--iterate` reruns only the survivors; `--cargo-test-arg` limits the run, not the build. Don't edit `rust/`
  while a run is going: each run copies the tree at its start, and a version bump mid-run made 60 of 62 mutants
  «unviable». Keep `CARGO_PROFILE_DEV_DEBUG=0` on every cargo call in the container, or `cargo clean` now and then:
  two debug-info settings side by side grew `rust/target` to 21 GB and a mutant run died of «No space left on device»
  (layer 2.1). Layers 1 and 2.1: the numbers are in `docs/HISTORY.md`.

## Layer 3.2: the search window (`src/search.rs`, `src/app/finder.rs`, `SearchWindow` in `ui/board.slint`)

Rules without the window — `src/search.rs` (checks: `tests/search.rs`, `tests/query.rs`, the parity tables
`search-dates` and `search-rows`):
- `parse_span(from, to, label)` = `TicketSearch.ParseSpan`/`TryDate`: the two date fields into a `DateSpan` (from the
  start of the first day to the start of the day after the last) or the error text. The notations are read by a small
  scanner with .NET's exact-parsing rules, not by chrono's `parse_from_str` (more lenient): `D.M.YYYY` (day and month
  1–2 digits), `DD.MM.YY` (00–49 → 20xx; .NET makes 50–99 19xx, which the 2000–2100 rule refuses), `YYYY-MM-DD`; ASCII
  digits only.
- `SearchFilter` (the form; `usual()` = «мои закрытые», the window's start; `quick(words)` = Enter on the board),
  `resolve(client, filter, statuses, closed)` = `TicketSearch.ResolveAsync` without service, type, saved filter and
  «one status» (step 3.3): the C# order of checks and requests, no ticket request before an error, the notes over the
  list. `Client::find_users` = `FindUsersAsync` (one page of max + 1), `Client::find_tasks` = the window's page (50,
  `include=status,service`, «ничего не найдено»), `TaskQuery::outside`/`outside_words` = the guard for a filter the
  server silently ignored.
- `Results` = `LoadPageAsync`: pages glued without repeats, total ≥ shown, «ещё» while a non-empty page leaves some
  unshown and fewer than 1000 are shown, the texts above the list (thousands with U+00A0, «1 000+»), the warning about
  rows outside the conditions; `service_of`/`details_of` = `FoundTicketViewModel`'s labels.

The window — `SearchWindow` in `ui/board.slint` (form on the left; the list, the texts above it and the preview on the
right) and `src/app/finder.rs` (a child module of `app`: it sees `State`). Opened by the toolbar button «Поиск заявок»
and the tray item (`open-search`: the window as it was; after a quick search, the conditions from before it — `usual`,
as `_quickMode`) and by Enter in the board's search box (`server-search`: `SearchFilter::quick`, search at once). It
starts with `SearchFilter::usual()`; nothing is remembered between runs (the prototype doesn't write `settings.json`).
Threads post `finder::Found` into the window's own inbox, drained by its `deliver`; an answer whose `generation`
(`preview_generation`) isn't current is dropped — a new search, «Показать ещё» and `use_client` (`forget_found`) start a
new one. The status directory is read once, for the first «открытые»/«закрытые» search. Preview = description +
`conversation_text` of the first lifetime page (the panel's renderer; the agent's Markdown comes with the export layer).
«+ На доску» = `AddKnown`: number, link, title, priority Mid → top of «Входящие», selected, ⟳ (`sync_card`); already on
the board — nothing; read-only — `ADD_READ_ONLY` above the list. Checked by `tests/finder.rs` (one test: the platform is
set once per process; the headless harness is shared with `tests/ui.rs` through `tests/common/window.rs`).
