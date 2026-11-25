# The Rust prototype

`rust/` — see also `rust/README.md` (files, commands). The C# app is its oracle: a port keeps the C# behaviour rule by
rule.

## The Rust prototype (`rust/`, 2026-11-06; layer 1 — client and rules — `rust-0.2.0`; layer 2.1 — the board writes — `rust-0.3.0`; layer 2.2 — F5, import, the quiet read — `rust-0.4.0`; layer 3.1 — the toolbar and the board's search — `rust-0.5.0`; layer 3.2 — the search window — `rust-0.6.0`; layer 3.3 — its directories — `rust-0.7.0`; layer 4.1 — the agent text — `rust-0.8.0`; layer 4.2 — files of the knowledge base — `rust-0.9.0`; layer 4.3 — «Выгрузить найденные» and the table — `rust-0.10.0`)

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
`Local\TicketBoard.SingleInstance`: owner → writes; held by TicketBoard.exe or another prototype → `main.rs` shows
`BUSY` and doesn't start — one app at a time, 0.9.1; the file unreadable → read-only and the mutex released so
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

- **Keys under the Russian layout** (0.9.1, `src/keys.rs`, `tests/keys.rs`): Slint matches Ctrl+V/C/X/A/Z/Y by the key's
  text (`InternalKeyEvent::shortcut`: `"v"`), and the V key gives «м» under the Russian layout. Each window's root
  FocusScope (`capture-key-pressed`, before any field) sends Ctrl+«м/с/ч/ф/я/н» to `latin-key`, which dispatches the same
  key in Latin. A field outside that FocusScope (a popup window) gets none of it. The board's own keys already take both
  letters (`n`/`т`, `/`/`.`).
- **Repaint in place** (`tests/repaint.rs`): on Windows the software renderer draws only what changed into the reused
  buffer (age 1) or one of two swapped ones (age 2). The check runs three identical apps — from scratch, in place, two
  buffers — and compares pixels after each pick at 100/125/150 %. Choose the buffer type per frame by the buffer's age, as
  `i-slint-backend-winit` does: `set_repaint_buffer_type` clears the renderer's cache, so switching it to get a reference
  makes the next frame a full one and hides the bug.
- **The folder field** (`files::bad_folder_part`): a part ending in a space or a dot is refused by name — Windows trims
  them only from the last part of a path, so `create_dir_all` makes «папка» and then can't find «папка \внутри»
  (os error 3; `tests/export_rows.rs`, Windows only).

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
new one. The directories are read on the window's first show (layer 3.3, below). Preview = description +
`conversation_text` of the first lifetime page (the panel's renderer; the agent's Markdown comes with the export layer).
«+ На доску» = `AddKnown`: number, link, title, priority Mid → top of «Входящие», selected, ⟳ (`sync_card`); already on
the board — nothing; read-only — `ADD_READ_ONLY` above the list. Checked by `tests/finder.rs` (one test: the platform is
set once per process; the headless harness is shared with `tests/ui.rs` through `tests/common/window.rs`).

## Layer 3.3: the search window's directories (service, type, saved filter, one status)

Client — `src/intraservice/mod.rs` (checks: `tests/directories.rs`, the unit tests at the end of the file):
- `get_services` = `GetServicesAsync`: the full directory (pages of 1000, at most 5); refused or empty → the services I am
  assigned to (`for=filtertasks`); none → the services of the latest 1000 tickets (`get_task_services`: one page,
  `count=false`, on 400 again with the count; by name, no repeats; its own 30 s limit, and the timeout text names it).
  A failure that passes by itself, or 401, is an error without fallbacks; 500 is a refusal (the fallbacks go). Result
  `Services {items, error, note}`: the note says which list it is or why it is empty — the C# texts.
- `get_task_types` (with archived), `get_saved_filters` (`resource=task`, one page).

Rules — `src/search.rs` (parity tables `search-service-children`, `search-service-labels`): `service_with_children`
(by `Path`: «4|40» without a trailing bar is nested too), `service_label`, `type_label` («(архив) …»), `filter_label`
(«… (по умолчанию)»), `children_note`; `resolve` takes the service list and builds service/type/saved filter/«one
status» (`filterid` + `ROW_FIELDS`).

Window — `src/app/finder.rs`, `ui/board.slint` (checks: `tests/finder_lists.rs`, one `#[test]` with a `catch_unwind`
per scenario):
- The directories are read on the window's first show, all four at once (`read_references`: a thread per directory,
  joined — one `Found::References`); only the unread ones are asked again on the next show; while a read runs, another
  show asks nothing. An empty status list isn't kept (asked again); an empty service list is an answer.
- Lists (`show_choices` = `FillChoices`): status «Любой», «Закрытые», «Открытые», then each server status; a partial
  service list without indents. Choices travel by number: `form_of` reads the indexes through the `Finder` lists,
  `set_form` = `SelectWanted` (not in the list, or 0 → the first row). After a directory answer the form is put back.
- Failures: «Не загрузились: статусы (…), типы (…). Поиск работает и без них.» above the conditions, in the C# order (the
  order the answer is handled in); under the service field — what the list is or why it is empty.
- A search reads the statuses itself when it needs them and they aren't there (it doesn't wait for the window's read —
  unlike C#, which awaits the running read; the price is a second `/api/taskstatus` when both run) and fills the list;
  a late failure of the window's read then changes nothing. The quick search from the board doesn't wait either.
- `use_client` (`forget_found`) drops the lists, the failures and the notes, ignores the old client's answers
  (`references_generation`) and drops the numbers of the remembered conditions (service, type, filter; one status →
  «Любой»), keeping «Закрытые»/«Открытые» — as C# `ApplySettings`.
- Mutants: `tools/mutants.py` runs `cargo test --lib` too (the `#[cfg(test)]` modules inside `src`) for `src/app.rs`
  and `src/intraservice/mod.rs`.

## Layer 4.1: the ticket's text as the agent gets it (`src/export.rs`; the search window's preview and «Копировать»)

The text is the knowledge-base file's text (format 8, `FORMAT_VERSION`) — C# `KnowledgeExport.Format.cs`, `.Secrets.cs`,
`.History.cs`, `ClosedExport.cs` `Myself`. Checks: `tests/export.rs` and the parity tables `export-mask`,
`export-escape`, `export-text` (the C# code writes the answers; server dates carry an offset, so no answer depends on the
machine's zone); `tests/export_read.rs` (the read).
- `mask` = `Secrets.Mask`, `escape` = `Escape` (+ `inert`, `tags`, `containers`), `one_line`, `machine_date`/`revision`
  (`MachineDate`), `facts`/`role_of` (`TicketHistory`), `with_details`/`row_of`, `format`. No regex: small scanners over
  `char`; lengths and cuts in UTF-16 units, as `string.Length` (the 20-unit gap of `mask`, the 300-unit closing comment).
- .NET classes by hand: `letter` = `\p{L}` (BMP only — .NET sees a sign beyond BMP as two halves; Alphabetic minus
  numbers, circled letters and a few marks), `is_word` = `\b`'s word sign; a doubtful sign is NOT a word sign, so a
  password next to it is hidden (only the hiding direction may differ from C#: «⚠️Пароль: 1» is hidden here, not there).
  Deliberate differences, all towards the safe side: `visible` also strips invisible signs beyond BMP (tag characters —
  C# `Attachments.Visible` tests UTF-16 units and keeps them: a gap in the C# app); a fence after an invisible sign or
  after a non-ASCII list number isn't escaped (it isn't a fence in Markdown either; C#'s culture-aware `StartsWith`
  escapes it).
- `Client::read_lifetime(id, max_pages)` = `ReadLifetimeAsync`; `export::build` = `BuildAsync` (card, then all pages,
  then `format(with_details(row, card))`; no retries — those belong to the file export, 4.2).

Window (`src/app/finder.rs`, checks `tests/finder_copy.rs`): the preview of the selected row is that text
(«Не удалось прочитать заявку:\n…» when it can't be read); «Копировать» puts the selected row's text into the clipboard
(`clipboard::set_text` on Windows; `App::use_clipboard` in checks) — the text on screen is reused while the row has the
same, present `changed` and the text is younger than 3 min (`TextTtl`; `App::use_text_ttl` in checks); «кто я» is read
once per client (a failed read is asked again), closed names come from the settings; `use_client` stops a running copy
(«Остановлено, в буфер ничего не положено.»). One selected row only (C# copies up to 30) — multi-select comes with 4.2.

## Layer 4.2: «Выгрузить выбранные» — the knowledge base's files (`src/export/files.rs`, `src/export/run.rs`)

One knowledge base for both apps: the prototype writes exactly what TicketBoard.exe writes, and each sees the other's
files as its own and current (the parity review walked renamed tickets, other months, older formats, attachments
downloaded by C#, user notes, corrupt files, MAX_PATH). Writes only while the board is its own (`store` is Some); a
read-only board refuses («Доска только для чтения: …») and «Открыть» creates nothing.
- `files.rs` (checks `tests/export_files.rs`, parity tables `export-names`, `export-place`, `export-props`,
  `export-index` (C# builds the indexes in a temp folder; the date is masked), `export-disk-names`, `export-size`,
  `export-file-text`): `file_name`/`place_of`/`shard_of` (`tickets/<yyyy-MM>/<id> — <title>.md`, MAX_PATH 250 in UTF-16
  units), `read_props`/`unquote`, ours = `source: intraservice`, current = `format` + `revision` + attachments in the
  base (`files_in_base`), scan by the `^\d+( — .*)?\.md$` pattern (a number beyond i32 still counts in the root index,
  as C#), attachments already in `files/`/`files-lfs/` linked with their size (renamed to today's disk name), others «не
  скачано» — **nothing is downloaded** (4.4); `write_atomic` (temp `.<4 hex>.tmp` beside, 100/300/1000 ms on a busy
  file; the retry is checked through `write_atomic_with`, which takes the write, the rename and the pauses — a busy file
  can't be made on Linux), temps older than 10 min swept, `*.tmp` appended once to `.gitignore` inside a git repository
  (read leniently: cp1251, BOM), `write_indexes` (month and root `_index.md`, then the table and the guides — 4.3). Ignore-case = ordinal (per-unit simple upper
  case), as .NET `OrdinalIgnoreCase`: σ/ς and the Kelvin sign decide the same as C#.
- `run.rs` (checks `tests/export_rows.rs`, unit tests at its end; parity tables `export-rate` (`RateMeter`),
  `export-span` (`HumanSpan`), `export-progress` (`ProgressLine` — C# took it out of `Job.Report` for the table) in
  `tests/export_progress.rs`): `export_rows` = `ExportRowsAsync` — 4 workers, unchanged skipped, card +
  whole lifetime with retries (`Options`: 2, 5, 15 s; 500 once), the breaker (401; 30 network, 100 × 500, 25 refusals
  without a success, 10 unwritten files — in a row), progress ≤ 4 a second («Выгружено N из M · … · осталось ~…»),
  stop, written files checked after 3 s («Проверяю, что файлы на месте…»), indexes at the end; `summary` = C#
  `Summary`. Deliberate differences: no writer queue (one export at a time in the window), no `MigrateFlat`, no
  «Смотрю, что уже выгружено…» line, a request in flight isn't aborted by «Остановить» (it finishes), a root index isn't
  rewritten when a month folder can't be listed, a case-only rename of an attachment is left as is.
- Window (`src/app/finder.rs`, checks `tests/finder_export.rs`): several rows (click, Ctrl, Shift, Ctrl+Shift adds; the
  preview — the row just added, else the last still picked), «Копировать (N)» ≤ 30, four at once, «кто я» once;
  «Выгрузить выбранные (N)», the folder from `KnowledgeDir` (empty — `knowledge` beside the data; edits live until exit),
  «Открыть» through the shell (not a program named explorer), «Остановить»; a client change stops copy and export and
  doesn't keep the old server's user. «Остановить» during copy says «Останавливаю — дочитываю начатое…» and keeps the
  window busy until the reads in flight end (ureq can't abort them; C# cancels), then «Остановлено, в буфер ничего не
  положено.»; copy's «Первая ошибка» is the first to arrive (C# `errors.Add`), the texts stay in row order; a click on
  the row whose preview failed reads it again, on a shown or loading one — doesn't.

## Layer 4.3: «Выгрузить найденные» and the base's table (`src/export/table.rs`, `export_found` in `src/export/run.rs`)

- `table.rs` = `KnowledgeExport.Table.cs` (checks `tests/export_table.rs`; parity tables `table-cell`, `table-records`,
  `table-row`, `table-date`, `export-table` (files of the base → `tickets.csv` byte for byte), `table-schema`,
  `table-agent`): `tickets.csv` — BOM, `;`, CRLF, the C# columns in order, `cell` (`'` before `= + - @` and a leading
  tab — formula injection; `;`, a quote or a line break → in quotes), `records` (the reader of the old table; an unclosed quote → the table is rebuilt
  whole), `read_date` (only the export's own form `гггг?мм?дд[(T| )чч:мм…][±ч[ч][[:]мм]|Z]` — ponytail; C#
  `TryParse` takes more), `row` (front matter → line; lists `["a", "b"]` → «a, b»). `write`: months touched by this run
  or newer than the table are re-read, the others are copied from the old table as they are; old table missing, other
  columns or broken → built whole; a file in its place that isn't ours → left alone; the table not behind → not written;
  its mtime = the moment its write began (`files::write_atomic_from_start`), so a file that lands during the write makes
  the next export re-read that month; stop or a tripped breaker (`write_indexes` takes «cancelled?», the run passes
  `Job::cancelled` — C# `Abort`) → not built (the old one stays, the next export catches up); a busy error while building
  (a month or the old table held by Excel, an antivirus) → built again after 100/300/1000 ms, as C# `WriteAtomic`; built
  as a string in memory (ponytail: tens of MB at tens of thousands of tickets). `write_guides`: `SCHEMA.md` (columns + rules, no date inside)
  rewritten when its text differs and it is ours (`SCHEMA_MARK`) — the user's own is left with a note; `CLAUDE.md` only
  when there is none (the user's own is never touched).
- `files.rs::write_indexes` (indexes → table → guides; the first failure is the note) takes `months_on_disk`
  (`MonthOnDisk{name, count, newest}`) so the table knows which months moved; when the months can't be listed, the root
  index, the table and the guides stay as they were (C# writes them from an empty list; checked through
  `write_indexes_with` — an unreadable folder can't be made as root).
- `run.rs::export_found` = `RunInTurnAsync` (checks `tests/export_found.rs` — the C# `Mass` scenarios against a Rust
  `MassServer`; parity table `export-found-notes` = `FoundNotes`, which C# took out of `RunInTurnAsync` for it,
  behaviour unchanged): list pages of `LIST_PAGE_SIZE` (200) with `get_task_list` (60 s, `Options.list_timeout`), page 1
  counted then `count=false` (a refused one → that page counted again: `counting_worked`), the sort fallback on page 1
  only (`fallback_worked`; HTTP ≥ 400 except 401/403/408/429), the next page asked while this one's cards are exported,
  list pages and card requests share one FIFO gate of 4 places; `TaskQuery::past_end`/`outside` guard a date filter the
  server ignored; `limit` 0 = all; a repeated page → `stuck`; a list shorter than promised → the second pass in another
  order (all: `DEFAULT_SORT`; with a limit: `CHANGED_ASC_SORT`; the first pass of «all» is `STABLE_SORT`) (`second_long`
  when it runs past `limit + 200`), its fresh tickets newest first, `overflow` beyond the limit; the summary =
  `summary(…, "По отбору")` + `found_notes`. `invalid_limit` (< 0) answers before anything starts. ponytail:
  «Остановить» waits for a list page already asked (ureq can't abort it; up to `list_timeout`, 60 s) — C# cancels it; as
  in 4.2, a request in flight finishes.
- Window (`src/app/finder.rs`, `src/search.rs`, checks `tests/finder_found.rs`, parity tables `export-found-label`,
  `export-found-confirm`, `export-found-tail` — C# `SearchViewModel.ExportFoundLabelOf`/`LimitOf`/`FoundTail` taken out
  for them): «Не больше, заявок» (`DEFAULT_LIMIT` 500; `limit_of` — ASCII digits only, else −1 → `invalid_limit`'s text;
  not a search condition: the list doesn't go stale; but part of the form, as C# `SearchFilter.Limit`: «Очистить» and
  the board's quick search set 500, the reopened window brings back the last person's search with its limit —
  `last_limit`, a non-number keeps the old), «Выгрузить найденные (N)» (`found_label`; «1 000+» stays a word, not a
  number), enabled when not busy, searched, not stale, total > 0, no question open (`can_export_found` =
  `CanExportFound`); ≥ `CONFIRM_FROM` (2000) to export (`export_count`; unknown total → the limit, 0 → all) → the
  question in the window (`asking`: «Выгрузить» and Enter / «Отмена» and Esc → «Выгрузка отменена.»; modal like
  `AskWindow`: the window's key handler swallows the other keys except Ctrl+A/C for its text, «Копировать» doesn't
  start, a new search drops the question); the board, the client and the folder are checked again on «Выгрузить»
  (`export_target`, shared with «Выгрузить выбранные»); the total is the one at the click; the summary's tail «Всего
  найдено N, взяты первые M…» when the limit cut it (`found_tail`); a client change drops the question.
