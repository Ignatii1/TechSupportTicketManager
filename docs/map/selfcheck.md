# Verification: what SelfCheck runs, the fake server, mutants, coverage

`TicketBoard.SelfCheck/`. One command for everything mechanical: `python3 tools/gate.py` (see `AGENTS.md`).

## How it works

- `TicketBoard.SelfCheck` compiles `Services/HttpIntraserviceClient*.cs`, `IntraserviceLinkParser.cs`,
  `AppSettings.cs`, `ClaudeRelay*.cs`, `AutoSyncRules.cs`, `FakeIntraservice.cs`, `KnowledgeExport*.cs`,
  `ClosedExport.cs`, `TaskQuery.cs`, `TicketSearch.cs` and the view models with what they need
  (`ViewModels/SearchViewModel.cs`, `MainViewModel*.cs`, `ColumnViewModel.cs`, `QuickCaptureViewModel.cs`,
  `SettingsViewModel.cs`, `Models/Ticket.cs`, `Services/TicketStore.cs`) into a console app; the relay, export,
  search, window and board checks run the real Intraservice client against a fake server on loopback
  (`FakeIntraservice`; the export ones do full exports into a temp folder — `KnowledgeExport.SelfCheck.Mass.cs` runs a
  230-ticket server through pages, stop/resume, retries and breakers; `KnowledgeExport.SelfCheck.Files.cs` —
  attachments: the fake serves files, whole, cut mid-body or stalled — `FakeFile`), `AppSettings.SelfCheck`
  round-trips `settings.json` in a temp folder. `SearchWindowCheck.cs` drives the real `SearchViewModel` (remembered
  conditions, lists, paging, stale flag, preview, clipboard, export, settings change). `BoardCheck.cs` drives the real
  `MainViewModel` against a server whose tickets change between steps: import, F5 with its question, and auto-sync
  passes (baseline, skip list, new / closed / reopened / reassigned, comments and «seen», requester reply, several
  events in one notification, a cut list, a dead server, settings saved or account changed mid-pass; every pass is
  checked for its expected outcome — a pass swallows its exceptions; and, on a board of its own, the per-pass limits:
  20 re-reads and 10 comment reads, their rotation). `CardCheck.cs` drives what the user does by hand: quick capture
  (`QuickCaptureViewModel` + `AddFromCapture`), ⟳ of one card, the quiet card read on open, the comments panel (pause,
  cache, rows and status chips, hidden status-only rows, «seen»), notes, filters and column counters, the WIP limit,
  the actions behind the board keys (the key mapping itself, `MainWindow.OnPreviewKeyDown`, is not compiled here),
  card age, a board without API. `PanelCheck.cs` (area `panel`) drives the panel's card fields and attachments: ⟳ and
  the quiet read (pause, freshness, arrowing, status untouched, auto-sync re-reads the open card, F5 reads it once),
  sizes (Range, once per run, a cancelled probe asked again, no length), «Открыть» (the per-server cache, read-only,
  temp then move, busy-move and 503 retries, runnables and shell-read types shown in the folder, Windows' own danger
  list via the stub, invisible signs), «Сохранить» (dialog name, copy or download, cancel, 404, the copy not
  read-only), an account change vs. a plain settings save, a board without API; the mark of the web is checked on
  Windows only (CI). `CommentCheck.cs` (area `comment`, 0.24.0) drives the write: the switch, the field only with a
  number and API, the preview, the whole request (method, path, Basic, `Device-Name`/`Device-Version`, the exact JSON
  body with `Changed` as the card sent it), the conversation check before the preview (new entries, nothing on screen
  yet, another card selected meanwhile), the switch or the account changed while the preview is open (a plain settings
  save is not that), 409/403/500/a stall, no retries, drafts per ticket, text typed during the send kept, the
  requester's title folded in the preview; the client's outcomes at the end (204, 400, 409, a closed port, a dropped
  connection, a followed redirect, an html page with 200 — `FakeIntraservice` answers a write with code 0 by dropping
  it, 3xx with `Location`, a body starting with `<` as html). `StatusCheck.cs` (area `status`, 0.25.0) drives the
  status on a hand move: the switch, a column without a status, every hand path (drop — through the column's
  `IDropTarget.Drop` with a fake `IDropInfo`, ←/→, the card menu, the panel's column) and the moves that never write
  (a reorder within a column, auto-sync's reopen, F5's closed move); the window (`StatusForm`: number, the server's
  title, the current status, the three latest comments, the transitions from the rights, the column's status proposed
  or a note why not — not allowed, another server's mapping), the exact body, the card re-read (no «закрыта»
  notification about one's own act), the required comment, «Только на доске», already in the status, rights absent or
  the include refused (400), moved back / back-and-forth / deleted while the card was read, two cards — windows one at
  a time (the stub `StatusWindow.Answer` returns a task that keeps the window «open»), card, directory or lifetime
  unread, no transitions, 409/403/500/a stall, the switch or account changed while the window is open. `StoreCheck.cs`
  guards the user's data: `tickets.json` round trip, a corrupt file moved aside as it was, one backup a day (of the
  previous file), at most 30 — and the file format: a sample in today's field names must load whole and the written
  field set must equal it (renaming a persisted `Ticket` property fails here — it would drop that field from existing
  files; a new field goes into the sample on purpose). `SettingsCheck.cs` drives `SettingsViewModel`: fields,
  range/regex/URL errors, the http warning, saving, the password rule, «Проверить» against fake servers (ok, 401, a
  login page) — the hotkey parser itself is on WPF types and is stubbed (`HotkeyService.Parses`), and the password's
  DPAPI write runs only on Windows: there (CI) the check reads it back, on Linux it says «пропущено».
  `AutoExportCheck.cs` drives `MainViewModel.AutoExportAsync` against `BoardServer` (its closed listing: `StatusIds` +
  `ChangedMoreThan` + `sort=Changed asc, Id asc`; row ids optional — `NoIds`; `CardFails`, `LifetimeFails` (a code),
  `IgnoreSort`, `TiesDescending`, `RepeatRow`, `NoChanged`, `PageCap`, `IgnoreChanged`, `ChangedSkew`, `NoPaginator`,
  `EmptyStatuses`, `Quietly`/`SetChanged` for server faults and exact dates): whose tickets go, the mark and its
  minute, budgets of 3, list, card and history failures, a broken ticket skipped after five passes
  (`ExportFailures.GiveUpAfter` set to zero there, `KnowledgeExport.RetryDelays` to zeros where a 503 is retried)
  while a network failure never is, a skipped ticket behind a failing one, a minute larger than a page, switched off,
  off-and-on or account changed mid-pass, a folder that can't be created, `.gitignore` in a repository's root or
  subfolder, before the folder exists and when it can't be written, the tray line; the pure rules are
  `ClosedExport.SelfCheck()`, both in the area `autoexport`. `MirrorCheck.cs` (area `mirror`, 0.22.0) drives the same
  pass with the mirror reaches on a `BoardServer` of its own: what goes (others' tickets too), the exact requests of a
  pass, the hour's rest and «догоняю», current files free after a reset (the mine reach too), widening/narrowing the
  reach and a mark without one, a failed list resting, the pass time in the tray line. A pause before a request is
  checked by when the request reached the fake server (`BoardServer.WaitedSince`), not by «nothing yet at this moment»
  — that flickers on a slow machine. Both board checks use one fake server, `BoardServer.cs` (tickets, comments,
  status changes, refused lifetimes, a list gate, request arrival times; a write — any non-GET request — is
  «unexpected» unless the check sets `AcceptWrites`, so every board, panel, auto-export and mirror check proves it
  writes nothing; `Writes` records them whole, `WriteAnswer`/`WriteWait` fake refusals and stalls; a write with
  `StatusId` sets the status — only into `FakeTask.ToStatuses` if set (403), into `Waits` (46, `IsCommentRequired`)
  only with a comment (400); the card with `include=…usertaskrights` carries `UserTaskRights.ToStatuses`,
  `RefuseRights` answers it 400, `StatusesFail` breaks the directory; `FakeIntraservice.Start` passes a request with a
  body to its `write` handler as a `FakeRequest`). A new self-check uses `CheckSet.cs` (`CheckSet` — named failures
  printed at once, one assert at the end; `TempDataDir` — its own data folder for `App.DataDir`, deleted before that
  assert). The window checks run on one thread (`SingleThread.cs`: its own SynchronizationContext, as on the WPF UI
  thread, which the board relies on — lists filled from parallel requests without locks) and fail if a board or window
  collection changes from another thread (`BoardCheck` also watches the board's properties, columns and cards); a
  pass's outcome is what `AutoSyncAsync` returns (`AutoSyncOutcome`: skipped / done / failed / dropped) — the title
  alone can't tell a dropped pass from a done one. What the view models take from the WPF side (data folder,
  `AskWindow` — answers set by the check, everything shown recorded —, clipboard, collection views,
  `Dispatcher.BeginInvoke` — deferred into that queue, timers that never tick by themselves: the check finds the
  board's in `DispatcherTimer.Created` and calls `Fire`, drag&drop interfaces — the stub moves nothing,
  `HotkeyService.TryParse` — answers `Parses`, `FileShell` — records what was opened, shown in the folder, asked to
  save; answers `SaveAnswer`, `OpenAnswer`, `Dangerous`) is stubbed in `WpfStubs.cs` — if a view model starts using
  something new from the app, add the same stub there. A parsing change gets a sample in the matching `SelfCheck()`
  and must pass here before it is committed. It refuses to run in Release, where `[Conditional("DEBUG")]` would strip
  every check.

- **Mutants** (`TicketBoard.SelfCheck/mutants/`): a list per area — `(name, file, before, after)`, `before` found
  exactly once, byte for byte (the repo is LF everywhere — `.gitattributes`). The runner works in temp copies of the
  sources (the working tree is never touched, uncommitted edits included), first proves the unmutated copy green on
  the lists' areas (its time ×4 is the limit), then builds and runs each mutant on its list's areas; a survivor is
  rerun on the other areas (measured green unmutated once per run, at the first survivor) and a failure there counts
  only if it repeats — so «ловит другая область» means fix `AREAS`. A hang counts as caught; a run that never started
  is «ОШИБКА ЗАПУСКА», not a kill. Exit 1 on a survivor, a wrong area, a launch error, a stale pattern or a mutant
  that doesn't build. `--check` finds stale patterns in seconds after a refactor. 2026-10-11: 288 of 288; 0.15.1 round
  4: 341 of 343 (both survivors fixed); 0.15.2, client/export/autoexport/query: 225 of 225; 0.16.0,
  ask/card/client/export: 193 of 193; 0.16.1, ask/card/export: 143 of 143; 0.17.0, table/export/autoexport: 197 of
  197; 0.17.1, card/board: 98 of 98. 0.18.0: history/secrets 28 of 28, ask/card/client/export/table/autoexport/search
  369 of 369; after the two reviews, search/card/board/autoexport/history/secrets/ask 262 of 262. 0.18.1: dates/table
  43 of 43 (+ 1 removed as equivalent on a UTC machine), the export date mutants 4 of 4. 0.19.0:
  files/client/autoexport/search/settings/export 382 of 382 (one caught only by another area — a check added in
  `export`); after review rounds 7–10, files/client 168 of 168, then client 72 of 72 and the error-body files mutants
  5 of 5 on the final code. 0.20.0: panel/store/card/client/files 265 of 266 (the survivor — a pause no check measured
  — fixed); after review rounds 1–2, panel/files 157 of 157 and 171 of 172 (the survivor — a card read slower than the
  panel's pause — fixed); on the final code panel 53 of 53, the cache-name files mutants 3 of 3. 0.21.0 (observers):
  client/history/query/resolve/table 157 of 157, search 34 of 35 (fixed), the observer mutants of
  autoexport/export/board 13 of 13; after two reviews panel/search/resolve 104 of 105 (the survivor — a pending
  selection lookup read the card for the pass — fixed). 0.22.0 (mirror): mirror/settings 37 of 38 (the survivor — a
  current file judged by the setting instead of the mirror's rule — fixed; «all tickets still by closed statuses» is
  caught only by the rules in `autoexport`: the wiring passes no closed ids to All anyway); after review round 1,
  mirror/autoexport 126 of 126 and the unchanged-rule mutants 8 of 9 (the survivor — our file under another name —
  fixed); after round 2, resolve/autoexport/mirror 144 of 144, the unchanged-rule mutants 10 of 10. 0.23.0 («В базу
  знаний»): card/settings 75 of 75; after review round 1, card/ask/settings 104 of 104; on the final code, card/ask 89
  of 89 (one first didn't build — the mutant's own fault, fixed) and the engine's queue/staleness mutants 10 of 10.
  0.24.0 (the comment, the first write): comment/client/settings 146 of 146; after review round 1, comment 51 of 51 on
  the final code, then the preview mutants (after /security-review) 6 of 6 and the dropped-connection one 1 of 1;
  after review round 2, comment 63 of 65 and files 2 of 2 (both survivors — weak checks, fixed), then 2 of 2 and the
  send mutants 4 of 4. 0.25.0 (the status): status/settings/comment/client 218 of 220 (both survivors — weak checks,
  fixed); after two reviews the new and fixed mutants 18 of 18, the window's labels and the broken rights card 6 of 6.

- **Coverage** (`coverage.py`, dotnet-coverage pinned in `dotnet-tools.json`, a dev tool, not an app package): 94%
  of the app lines SelfCheck compiles (2026-10-10: 3119 of 3315, the rest listed per file); about 1100 lines on WPF
  types are not in SelfCheck at all (windows, tray, hotkey, converters) — the script lists those files. Coverage says
  a line ran, mutants say its result is checked.

- **`FakeIntraservice` serves one connection at a time.** A cancelled request leaves HttpClient's just-opened
  connection idle in its pool for a minute, and a server reading it blocked for that minute (a run of the self-check
  stalled 60 s, one time in five). It now drops a connection that sends no request within 2 s — keep that if you touch
  it. And a cancelled run's abandoned requests still reach the fake late: give each scenario its own client path
  prefix (`/gN`, see `MassServer`) instead of counting on a pause to let them drain.

- **Mutation scripts that `git checkout` a file discard your uncommitted edits to it** — commit first.

## Parity tables (C# answers the Rust port is checked against)

`rust/tests/parity/<name>.tsv` — one table per ported pure rule: cells separated by tabs, the inputs first, the answer
of the C# code last; lines starting with `#` are comments. `TicketBoard.SelfCheck/ParityCheck.cs` (area `parity`) holds
`Tables`: name → the number of inputs and the C# call that answers them; the area fails if a row's answer isn't what
the C# code gives now, if a cell isn't in the canonical form, if a table has no entry in `Tables` or an entry has no
file. `dotnet run --project TicketBoard.SelfCheck -- parity --write` rewrites the answers (a row may come without
one) and the form of the cells — after adding inputs or changing the rule; nobody types an answer by hand. Cell form
(`Escape`/`Unescape`): a backslash doubled; tab, line feed, carriage return as a backslash and `t`, `n`, `r`; anything
that isn't printable ASCII or Russian text, and a space at a cell's edge, as a backslash, `u` and four hex digits. The
Rust side reads a table with `common::parity(name)` (rows unescaped, with their line numbers) and compares with
`common::parity_mismatches(name, |inputs| …)`, which returns every differing row at once (`rust/tests/parity.rs` checks
the reader itself). The tables live under `rust/` because cargo-mutants copies only that tree. First table:
`search-dates` — `TicketSearch.ParseSpan` (the fields «с» and «по» and the field's label → `OK <from> <to>` or
`ERR <text>`).
