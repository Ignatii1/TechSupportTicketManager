# AGENTS.md — map for coding agents

Read this first, then `PROGRESS.md` (current state, open work) and `docs/ROADMAP.md` (the agreed plan and process). When you finish, update `PROGRESS.md` and add an entry to `docs/HISTORY.md`.

## What this is

**TicketBoard** («Заявки»): a personal Kanban tracker for [Intraservice](https://intraservice.ru) helpdesk tickets.
One user, one Windows PC. It lives in the tray; a global hotkey opens a quick-capture box; it reads ticket title/status
from the Intraservice REST API and **never writes to Intraservice**. All data is local JSON in the exe's own folder (`App.DataDir` = `AppContext.BaseDirectory`), so the app is portable.

- WPF, .NET 10 (`net10.0-windows`), MVVM via CommunityToolkit.Mvvm (`[ObservableProperty]`, `[RelayCommand]`).
- Libraries: WPF-UI (Fluent/Mica windows), gong-wpf-dragdrop, H.NotifyIcon.Wpf. Nothing else; don't add packages for what a few lines do.
- **Windows only, by decision.** Don't propose Avalonia/MAUI/Tauri/web ports.
- **UI text and code comments are Russian.** Keep that. Commit messages are English.
- Style: short, direct code, no one-implementation interfaces (an `IIntraserviceClient` was removed on purpose).
  `// ponytail: …` comments mark deliberate simplifications and name their ceiling; keep them honest.
- **How changes are made** (details: `docs/ROADMAP.md` → «Как работаем»). Tests first: the round's check list goes into
  its `docs/HISTORY.md` entry before the code, every new check is seen failing for the right reason, a bug gets a failing
  check before its fix, formats other programs read are compared with golden files, and new checks come with mutants
  they kill. New code: immutable values (`record`, `init`); no `null` from new APIs (nullable warnings are build errors);
  small objects that own their rule instead of managers and helpers; decorators only once there are two behaviours. Not
  taken from Elegant Objects: an interface per class, a ban on static pure functions (the rule classes are the most
  testable code here), a ban on properties (WPF binding and the JSON format live on them).

## Build and verify

The agent container builds the app and runs every parser check. Use that for each change; CI is for releases.

```
sudo apt-get update; sudo apt-get install -y dotnet-sdk-10.0   # once per container (`;`: update errors on dead PPAs)
cd TicketBoard && dotnet build -c Release    # ~15 s, compiles the WPF app on Linux (EnableWindowsTargeting)
cd .. && dotnet run --project TicketBoard.SelfCheck   # every check (~25 s); prints "SelfCheck: OK", exit 0
dotnet run --project TicketBoard.SelfCheck -- board card    # only these areas (names: Program.cs) — the fast TDD loop
python3 TicketBoard.SelfCheck/mutants/mutate.py board -j 3   # planted bugs the checks must catch (lists: mutants/*.py)
python3 TicketBoard.SelfCheck/coverage.py MainViewModel      # lines no check executes (+ their numbers for that file)
```

- **Mutants** (`TicketBoard.SelfCheck/mutants/`): a list per area — `(name, file, before, after)`, `before` found exactly
  once, byte for byte (the repo is LF everywhere — `.gitattributes`). The runner works in temp copies of the sources (the
  working tree is never touched, uncommitted edits included), first proves the unmutated copy green on every area (its
  timings set the limits), then builds and runs each mutant on its list's areas; a survivor is rerun on the other areas,
  so «ловит другая область» means fix the list's `AREAS`. A hang counts as caught; a run that never started is «ОШИБКА
  ЗАПУСКА», not a kill. Exit 1 on a survivor, a wrong area, a launch error, a stale pattern or a mutant that doesn't build. `--check` finds stale patterns in seconds after a refactor. 2026-10-10: 128 of 128.
- **Coverage** (`coverage.py`, dotnet-coverage pinned in `dotnet-tools.json`, a dev tool, not an app package): 94% of the
  app lines SelfCheck compiles (2026-10-10: 3119 of 3315, the rest listed per file); about 1100 lines on WPF types are
  not in SelfCheck at all (windows, tray, hotkey, converters) — the script lists those files. Coverage says a line ran,
  mutants say its result is checked.

- `TicketBoard.SelfCheck` compiles `Services/HttpIntraserviceClient*.cs`, `IntraserviceLinkParser.cs`, `AppSettings.cs`,
  `ClaudeRelay*.cs`, `AutoSyncRules.cs`, `FakeIntraservice.cs`, `KnowledgeExport*.cs`, `TaskQuery.cs`, `TicketSearch.cs`
  and the view models with what they need (`ViewModels/SearchViewModel.cs`, `MainViewModel*.cs`, `ColumnViewModel.cs`,
  `QuickCaptureViewModel.cs`, `SettingsViewModel.cs`, `Models/Ticket.cs`, `Services/TicketStore.cs`) into a console app; the relay, export, search, window and board checks run
  the real Intraservice client against a fake server on loopback (`FakeIntraservice`; the export ones do full exports into a temp
  folder — `KnowledgeExport.SelfCheck.Mass.cs` runs a 230-ticket server through pages, stop/resume, retries and breakers), `AppSettings.SelfCheck` round-trips `settings.json` in a temp folder. `SearchWindowCheck.cs` drives the real
  `SearchViewModel` (remembered conditions, lists, paging, stale flag, preview, clipboard, export, settings change).
  `BoardCheck.cs` drives the real `MainViewModel` against a server whose tickets change between steps: import, F5 with its
  question, and auto-sync passes (baseline, skip list, new / closed / reopened / reassigned, comments and «seen», requester
  reply, several events in one notification, a cut list, a dead server, settings saved or account changed mid-pass;
  every pass is checked for its expected outcome — a pass swallows its exceptions; and, on a board of its own, the
  per-pass limits: 20 re-reads and 10 comment reads, their rotation). `CardCheck.cs` drives what the user does by hand:
  quick capture (`QuickCaptureViewModel` + `AddFromCapture`), ⟳ of one card, people filled in for old cards, the comments
  panel (pause, cache, rows and status chips, hidden status-only rows, «seen»), notes, filters and column counters, the WIP
  limit, the actions behind the board keys (the key mapping itself, `MainWindow.OnPreviewKeyDown`, is not compiled here),
  card age, a board without API. `StoreCheck.cs` guards the user's data: `tickets.json` round trip, a corrupt file moved
  aside as it was, one backup a day (of the previous file), at most 30 — and the file format: a sample in today's field
  names must load whole and the written field set must equal it (renaming a persisted `Ticket` property fails here — it
  would drop that field from existing files; a new field goes into the sample on purpose). `SettingsCheck.cs` drives
  `SettingsViewModel`: fields, range/regex/URL errors, the http warning, saving, the password rule, «Проверить» against
  fake servers (ok, 401, a login page) — the hotkey parser itself is on WPF types and is stubbed (`HotkeyService.Parses`),
  and the password's DPAPI write runs only on Windows: there (CI) the check reads it back, on Linux it says «пропущено». A pause before a request is checked by when the request reached the fake server
  (`BoardServer.WaitedSince`), not by «nothing yet at this moment» — that flickers on a slow machine. Both board checks use one fake server, `BoardServer.cs` (tickets,
  comments, status changes, refused lifetimes, a list gate, request arrival times). A new self-check uses `CheckSet.cs`
  (`CheckSet` — named failures printed at once, one assert at the end; `TempDataDir` — its own data folder for
  `App.DataDir`, deleted before that assert). The window checks run on one
  thread (`SingleThread.cs`: its own SynchronizationContext, as on the WPF UI thread, which the board relies on — lists
  filled from parallel requests without locks) and fail if a board or window collection changes from another thread
  (`BoardCheck` also watches the board's properties, columns and cards); a pass's outcome is what `AutoSyncAsync`
  returns (`AutoSyncOutcome`: skipped / done / failed / dropped) — the title alone can't tell a dropped pass from a done one. What the view models take from the WPF side (data folder, `AskWindow` — answers set by
  the check, everything shown recorded —, clipboard, collection views, `Dispatcher.BeginInvoke` — deferred into that
  queue, timers that never tick by themselves: the check finds the board's in `DispatcherTimer.Created` and calls `Fire`,
  drag&drop interfaces — the stub moves nothing, `HotkeyService.TryParse` — answers `Parses`) is stubbed in `WpfStubs.cs` — if a view model starts using something new
  from the app, add the same stub there. A parsing change gets a sample in the matching
  `SelfCheck()` and must pass here before it is committed. It refuses to run in Release, where `[Conditional("DEBUG")]` would
  strip every check.
- The UI can't run on Linux. Say so rather than claiming a UI change works; the user tests on his Windows work PC. What
  can be checked without running it: the XAML compiles with the build, and `python3 TicketBoard.SelfCheck/xamlcheck.py
  <file.xaml> <ViewModelClass> [<RowClass>]` verifies that every `{StaticResource}` key exists where it is used (own
  resources in scope, declared above the use, or App/Themes) and that every `{Binding}` path exists link by link, through
  property types, template `DataType`s and all parts of a partial class (a missing key crashes the window on open; a
  wrong binding fails silently) — run it after touching a window or the panel; `--selftest` checks the checker.
  Rendering, layout and focus stay unverified.
- CI (`.github/workflows/build.yml`, windows-latest) runs SelfCheck and publishes the exe as a run artifact on pushes to
  `main`, PRs and manual runs. **The user tests only from GitHub Releases on his work PC**, so a finished round (built,
  SelfCheck green, reviewed) ends with a merge to `main` and a release; bump `<Version>` in `TicketBoard.csproj` in the
  same commit. **Tags can't be pushed from the agent container** (the git proxy refuses them) — dispatch `build.yml` with the
  `release_tag` input and GitHub creates the tag and the Release.

## Working with the user

- He is a developer and wants decisions made and explained, not asked — ask only at real forks. Say plainly what was not
  verified. Run `/code-review high` over the round before merging: it has caught a real bug every round so far.

## Working here without wasting context

- Compile and SelfCheck locally; dispatch CI only for a release. Every CI poll returns kilobytes of JSON.
- Read by range (`grep -n`, then `sed -n 'a,bp'`), not whole files. The big classes are split into partial files by
  responsibility — open the one you need.
- Commit bodies ≤ 5 lines: CI and release responses echo them back. The reasoning goes into `docs/HISTORY.md` once.
- Release link = `https://github.com/Ignatii1/TechSupportTicketManager/releases/tag/vX.Y.Z`; don't fetch the object.
- Subagents only for parallel work on disjoint files; ask them for reports of ≤ 30 lines.
- API reference: the user's `IntraService_API_v5_51.pdf`. Extract with `pip install pypdf cffi` (plain pypdf crashes here
  on a broken `cryptography`). `API-IDEAS.md` already maps the endpoints worth having.

## Repository layout

```
README.md                  user guide (Russian): install, usage, settings, data files, troubleshooting
AGENTS.md                  this file
PROGRESS.md                current state and open work (short — read it)
API-IDEAS.md               what the Intraservice API offers, ranked; what's done is marked in PROGRESS
docs/HISTORY.md            past rounds and their reasons (read only when you need the why)
docs/ROADMAP.md            the agreed plan: stages in order, acceptance criteria, the process every stage goes through
TicketBoard.SelfCheck/     console app that runs the parser, relay, export, search, search-window, board, card, store and settings self-checks on Linux
                           (+ xamlcheck.py: resource keys and bindings of a XAML file; mutants/: planted bugs and their
                           runner; coverage.py: lines no check executes)
dotnet-tools.json          dev tools pinned for `dotnet tool restore` (dotnet-coverage)
LICENSE                    MIT
.github/workflows/build.yml
TicketBoard/
  README.md                developer notes (Russian): build, CI, short file list, Intraservice API notes
  TicketBoard.csproj       packages, Release publish settings
  app.manifest             PerMonitorV2 DPI, Win10/11
  App.xaml(.cs)            composition root and app-level concerns (see below)
  Models/Ticket.cs         Ticket, Note, TicketStatus/Priority/AgeState enums, TicketRules
  Services/                persistence, settings, hotkey, autostart, theme; Intraservice API = HttpIntraserviceClient
                           (HTTP + errors) + .Parse.cs (all JSON field names) + .SelfCheck.cs (samples)
  ViewModels/              MainViewModel (board; partials .Intraservice = sync/import/F5, .Comments = «Переписка»,
                           .AutoSync), ColumnViewModel, QuickCaptureViewModel, SearchViewModel (search + preview +
                           export), SettingsViewModel
  Views/                   MainWindow, TicketPanel (the detail panel, a UserControl), QuickCaptureWindow, SearchWindow,
                           SettingsWindow, AskWindow (XAML + thin code-behind)
  Themes/                  Tokens.Light/Dark.xaml (colors), Styles.xaml (shared styles, fonts, glyph)
  Converters/Converters.cs all XAML value converters
  Assets/                  app.ico, tray-light.ico, tray-dark.ico (embedded as WPF Resources)
```

## How it fits together

`App.OnStartup` is the only composition root (no DI container); settings apply live through `App.ApplySettings`. The
startup order, the settings flow and the new-ticket data flow are in `TicketBoard/README.md` → «Как всё связано».

## "I want to change X" → go here

| Change | Where |
|---|---|
| Ticket fields / what gets saved | `Models/Ticket.cs`. `[ObservableProperty]` fields and public properties are serialized **by name** into `tickets.json`. Renaming breaks existing data. Computed properties need `[JsonIgnore]`. |
| Age badge logic (warn/overdue), day plurals | `Ticket.AgeState`, `AgeLabel`, `AgeText`, `Plural` in `Models/Ticket.cs`; thresholds in `TicketRules` (set from settings) |
| Columns (names, order, count) | `TicketStatus` enum (`Models/Ticket.cs`) + `Columns` list in the `MainViewModel` constructor + `StatusToTextConverter` + context menu in `MainWindow.xaml` (`TicketCard` template). Enum names are stored in JSON. |
| Moving tickets between columns | `MainViewModel.MoveTicket` / `MoveSelected` / `OnDropped` / `SelectedStatus`; drag&drop target is `ColumnViewModel` (`IDropTarget`) |
| Search and filters, hiding old Done cards | `MainViewModel.Matches`, `RefreshFilters`, `Recount` (checked by `TicketBoard.SelfCheck/CardCheck.cs`) |
| WIP limit / overload | `ColumnViewModel.IsOverloaded` (visible count) vs. `IsOverloadedTotal` (all cards, used by the tray) |
| Parsing a ticket number from a link or text | `Services/IntraserviceLinkParser.cs` (regex from settings, fallback to the last 4–8 digit number in the URL) |
| Title derivation for new tickets | `MainViewModel.AddFromCapture` |
| Intraservice HTTP calls, error messages | `Services/HttpIntraserviceClient.cs` `GetTaskAsync`, `CheckAsync`, `GetAsync` (status code → Russian message). A response that doesn't parse goes through `Unparsed(json)`, which writes the method name and the first 4000 chars of the body to `errors.log` via `LogUnparsed` — route any new parse failure through it. |
| Intraservice JSON field names | **only** `Services/HttpIntraserviceClient.Parse.cs`; a sample for every shape in `HttpIntraserviceClient.SelfCheck.cs`, run by `TicketBoard.SelfCheck` |
| Comments («Переписка») in the panel | `ViewModels/MainViewModel.Comments.cs` (`LoadComments` / `ToRows`) + `HttpIntraserviceClient.GetLifetimeAsync`; row template `CommentItem` in `Views/TicketPanel.xaml`, styles `CommentRow`/`Chip`/`IconToggle`. Never persisted: in memory + a 2-minute cache. |
| Unread comments (card badge, notification, «seen») | Counting rule: `AutoSyncRules.Unread` (+ SelfCheck; server dates only, own comments by `EditorId`, else by name). Auto-sync re-reads the lifetime only of cards whose `Changed` moved (`CheckCommentsAsync` in `.AutoSync.cs`; first sight = baseline, no request). Persisted on `Ticket`: `CommentsCheckedFor`, `CommentsSeenAt`, `UnreadComments`; in memory: `ServerChanged` (set by `Apply`). «Seen» = shown in the open panel of the active board window **and** a user action (selection, panel, ⟳, activation, any click/key on the board): `MarkCommentsSeen` in `.Comments.cs`, `IsBoardActive` + input hooks in `MainWindow`. Auto-sync never marks seen itself (`LoadComments(markSeen: false)`) and always notifies. Baselines from `Changed` get +1 s (`AutoSyncRules.SeenFrom`). Badge: `UnreadBadge` style in the `TicketCard` template. |
| Import of my tickets | `ViewModels/MainViewModel.Intraservice.cs` (`ImportMine`, `FetchMyOpenAsync`) + `HttpIntraserviceClient.GetCurrentUserAsync` / `GetStatusesAsync` / `GetExecutorTasksAsync`; paging and «is the list complete» — `AutoSyncRules.ReadAllPagesAsync` (+ SelfCheck); entry points in `App.SetupTray` and the toolbar in `MainWindow.xaml`; the closed-status names live in `AppSettings.ClosedStatusNames` |
| Refresh all cards («Обновить статусы», F5) | `MainViewModel.Intraservice.cs` (`RefreshAll`) (per-card `GetTaskAsync`, 4 at a time) + `Apply` (the shared "what a sync may overwrite" rule, also used by `SyncAsync`) + `ClosedNames` (shared with the import); entry points `App.SetupTray` and `MainWindow.OnPreviewKeyDown` |
| Search any tickets (filters, preview, copy, export) | Window: `Views/SearchWindow.xaml(.cs)` + `ViewModels/SearchViewModel.cs` (form → `TicketSearch.ResolveAsync` → `GetTasksAsync` pages of 50 + «ещё»; selection → preview/copy/export; `_wanted` = what to select in the lists, `_filling` = code, not the user, is filling them, `IsStale` = the form changed since the list). Opened from the toolbar (`MainViewModel.SearchRequested`), the tray and `Enter` in the board's search box (`MainWindow.ServerSearchRequested`, with the words, filters reset, remembered conditions not overwritten — `_skipRemember`; the form is put back to them when the hidden window is reopened — `_quickMode`, `OpenAsync(restoreQuick)`) — all wired in `App.OnStartup`. «+ На доску» goes through `MainViewModel.AddKnown` (id, url and title are known — no text parsing). «Не больше, заявок» (`Limit`; **0 = everything**, no upper ceiling) belongs to the export only: the search never validates it. An export of ≥ `SearchViewModel.ConfirmFrom` (2000) tickets asks first through the injectable `Confirm` callback (the window sets it to `AskWindow.Ask`, the harness leaves it null or answers); with no conditions at all the resolver adds the note «Условий нет…» (`TaskQuery.IsUnrestricted`). Query model + URL: `Services/TaskQuery.cs` (every list filter of the API doc, pp. 14-20: `ExecutorIds`/`CreatorIds`/`ServiceIds`/`TypeIds`/`StatusIds`, `search`, `Created`/`Changed`/`Closed` `…MoreThan`/`…LessThan`, `filterid`, `archive`+`inactive`, `Sort` = «Поле asc|desc» (default `Changed desc`; `StableSort` = `Created asc, Id asc` for reading everything); `Outside`/`PastEnd` = the guards for a silently ignored date filter). Form → query: `Services/TicketSearch.cs` (`SearchFilter` is what `AppSettings.LastSearch` stores; names → ids through `api/user?search=`, service + children by `Path`, closed/open statuses as in F5, dates `дд.мм.гггг` → period). Reference lists (services, task types, saved filters, users): `HttpIntraserviceClient.GetServicesAsync`/`GetTaskTypesAsync`/`GetSavedFiltersAsync`/`FindUsersAsync` + `ParseRefs`; services fall back (refused **or empty**) to `for=filtertasks`, then to the `Services` block of the latest 1000 tickets (`GetTaskServicesAsync`) — but not on a transient error or 401 (a partial list would stick for the session) — and `SearchViewModel.ServiceNote` under the field says which list it is or why it is empty; with a partial list (`_servicesPartial`) labels are flat (sorted by name); a remembered service missing from a partial or failed list stays as «Сервис №…» (`_referencesTried` tells «failed» from «not read yet», so it is set before the lists are filled) and a search with children says that children come only from that list; a 500 on the directory counts as a refusal (fallbacks run), and an error names both causes; the board's quick search (`StartWithAsync`) loads missing lists in the background; the other lists' failures go to `Notes` at the top of the conditions. Totals: `FoundNum` shows a capped total as «1 000+», the export button and the confirm question (`ConfirmTitle`/`ConfirmText`/`ExportCount`) don't invent a number then. The board's quick search and the relay's `TB search` still use `SearchAsync` (20 newest). |
| Export for the knowledge base (files for an agent / Obsidian) and the agent's text | Engine, split by responsibility: `Services/KnowledgeExport.cs` (the run: `BuildAsync` = one ticket as Markdown — the card via `GetTaskAsync` (service, type, categories, group: the live server leaves them out of list rows, merged by `WithDetails`) plus the full lifetime via `GetLifetimePageAsync` pages, the server's Paginator trusted when it has Page and PageCount — `IntraserviceLifetime.Paged`; an unreadable card or lifetime = no text — shared by the preview, the clipboard and the files; `ExportRowsAsync` = given rows; `RunAsync` = the list by a `TaskQuery`; the nested `Job` holds the counters, the circuit breaker and the progress/ETA line), `.Format.cs` (file name + Markdown text), `.Files.cs` (folders `tickets/<yyyy-MM>/` by the row's created date via `ShardOf`, `ScanNames` = file-name→id map without reading files, `IsCurrent` = the file's `format`+`changed` match the row, `MigrateFlat` = the 0.10–0.12 flat layout moves into months, month + root indexes). **`RunAsync(limit 0)` exports everything**: no ceiling; list pages of 200 (page 1 with the server's default count — the total for progress; a total equal to `HttpIntraserviceClient.CountCeiling` (1000) is only «that or more» (`Capped`): no «из»/ETA then, a rate instead; later pages `count=false` + `HasNextPage` — **never `count=all`**, the live server answers it with HTTP 400; 60 s timeout, `TaskQuery.StableSort` — creation date never changes, so the walk doesn't shift for hours) are exported (4 parallel) page by page before the next page is asked for — memory is one page + the name map + the `seen` id set. With `limit > 0` it is the first N by `Changed desc` (rows outside the period are skipped and don't count to the limit; the list is cut off past the period — `TaskQuery.PastEnd`). Stop/resume is just the incremental rule (unchanged files are skipped), so a stopped, crashed or tripped run continues where it was. `Retrying` repeats `HttpIntraserviceClient.IsTransient` errors after `RetryDelays` (2, 5, 15 s); the `Job` trips the whole run (`Fatal` text, atomically, cancels `Abort`) on HTTP 401, 30 transient failures in a row (network, timeouts, 502/503/504…), 100 HTTP 500s in a row (a 500 is more often one broken ticket than an outage: it gets one retry, not three, and a run must be able to pass a damaged stretch), 25 refusals (404/403…) with no success and no unchanged file, 10 write failures in a row. The next list page is requested while the current one is exported (`ahead`). **Second pass** (`everything` + sorted by creation + a clean first pass, when there were repeats at page boundaries — an unstable order loses a neighbour —, the exact count wasn't reached, or the count is capped «1000+», where a shift by a removed ticket can't be seen otherwise): the whole list again by `Changed desc`, only unseen tickets exported (`recovered`), with prefetch, its own stuck guard (`secondStuck`) and progress; notes say what it found, or that the server counts a ticket twice; the cut-at-ceiling check uses the first pass only (`firstSeen`). **Written files are verified**: `Job.Written` (id, path, write time) → `CheckWritten` between pages checks files at least `VanishWait` old (3 s; the self-check sets 10 ms), and at the end `SettleAsync` waits until the youngest is that old (Stop skips it) before `CheckWritten(all)` — vanished ones (antivirus quarantine) are named in the summary and make the run unfinished. A repeated page with no new tickets is «stuck» only when full; a repeated short page is the end of the list (a server clamping past the end), in both passes. The server's `HasNextPage` decides when it is sent; otherwise a short page ends the list only when the server sent an exact total (`total > r.Found.Count`). Names shrink to fit MAX_PATH (`PlaceOf`); a lifetime longer than `MaxLifetimePages` is marked in the file; selected rows scan only their month folders (`ScanNames(shards)`), so a duplicate in another month waits for the next full run. A 4xx on page 1 with the stable sort falls back to `Changed desc` (noted); a list that is not ascending, shifts, or repeats a page is noted/stopped. Files are found by the number the file name starts with (`IdInName`), but only those whose front matter has `source: intraservice` (`SourceName`) are ours — rewritten, deleted as stale/duplicates, migrated; a user's note with a ticket number in its name, a corrupt file and a user folder among the months (`IsShard`) are left alone (only a file exactly at the ticket's own path is overwritten); the list page takes one of the 4 request slots; `WriteAtomic` retries a write that Windows' antivirus/indexer holds for a moment (`IsBusy`: sharing/lock violation, access denied — 0.1 s, 0.3 s, 1 s); contacts never exported; `FormatVersion` — bump it when the file format changes. Self-check: `KnowledgeExport.SelfCheck.cs` (first/again/rename/period scenarios) and `.SelfCheck.Mass.cs` (`MassServer`: every scenario gets its own client path `/gN`, so requests that a cancelled run abandoned are not counted in the next one) against `FakeIntraservice`. The folder is `AppSettings.KnowledgeDir` (`KnowledgePath`). Extra row fields: `IntraserviceExtra` (service, type, categories, resolution date). |
| Background auto-sync (timer, new assignments, closed-ticket notifications) | `ViewModels/MainViewModel.AutoSync.cs` (`ApplyAutoSync`, `AutoSyncAsync` — `internal` and returns its `AutoSyncOutcome` so that `TicketBoard.SelfCheck/BoardCheck.cs` runs whole passes, like import and F5, against a fake server: a change here gets a step there) — reuses `FetchMyOpenAsync` / `NewCard` (import) and `Apply` / `ClosedToMove` / `AskMoveClosed` (F5) from `.Intraservice.cs`; what it never adds — `Services/AutoSyncRules.cs` (+ SelfCheck), persisted in `AppSettings.AutoSyncSkipIds` (reset with the lifecycle flags in `MainViewModel.ApplySettings` when `AppSettings.AutoSyncAccount` ≠ `AccountKey` — URL/login changed, also by hand between runs); notifications with click actions — `MainViewModel.Notify` → `App.ShowTrayNotification`; closed-but-not-moved counter in the board title — `ShowAutoSyncState`. It never moves a card into «Готово» by itself; the only moves it makes are a reopened «Готово» card back to «Входящие» and a «Ждёт ответа» card to «В работе» when the requester replies (rule `AutoSyncRules.RequesterReply` + SelfCheck: a new unread comment whose author's name equals the card's `Creator`; applied at the end of `AutoSyncAsync`, only to tickets in this pass's open list — so never closed or reassigned ones, and never on a stale card status; notification section `Answered`). |
| What a sync overwrites on a ticket | `MainViewModel.Intraservice.cs` → `Apply` (used by `SyncAsync`, `RefreshAll` and auto-sync — every N minutes on listed cards and rechecks): status, creator with phone/email, executors and executor group always (a field missing from the response keeps the old value), title only if it's still the auto `Заявка #N`, description only if empty. List rows go through `AsTask`. |
| Reopened / reassigned-away tickets | Rule: `AutoSyncRules.Track` (+ SelfCheck) over the persisted `Ticket.AssignedToMe` (null = not seen yet → baseline, no events). In `AutoSyncAsync`: listed cards in «Готово» that were not mine → moved to «Входящие» (`Lifecycle.Reopened`); cards that left a complete list and whose recheck says open → «больше не на вас» (`Reassigned`, card stays; an unresolved «статус N» decides nothing — `HttpIntraserviceClient.IsResolvedStatus`). Transitions are computed during the pass and applied only at its end, right before the notification. A pass is dropped (`StillCurrent`) only when the account changed or auto-sync was switched off mid-pass; a plain settings save no longer aborts it (it used to, and lost notifications for statuses already written). An account change resets `AssignedToMe` (`AutoSyncAccount` vs `AccountKey` in `MainViewModel.ApplySettings`). Notification sections: `NotifyChanges` / `AutoSyncNews`, list lines via `Lines`. |
| Creator / executors in the panel | Parsed in `.Parse.cs` (`Field`, `Names`), stored on `Ticket` (`Creator`, `CreatorPhone`, `CreatorEmail` → `CreatorContacts`, `Executors`, `ExecutorGroup`), rows 5–6 of the panel grid in `Views/TicketPanel.xaml` (`SelectableBody`/`SelectableText`). Cards without executors (before 0.8.0) or contacts (before 0.9.0) get the people fields quietly on first open (`FillPeopleAsync` → `ApplyPeople` only, status untouched; once per run). Also in the relay's `TB ticket` answer (`ClaudeRelay.People`). |
| Quick-capture behavior (keys 1/2/3, Enter, Esc, clipboard) | `Views/QuickCaptureWindow.xaml.cs` + `ViewModels/QuickCaptureViewModel.cs` (400 ms debounced title lookup; the hint starts as the empty-field one — `Reset` on the first open changes nothing) + `MainViewModel.AddFromCapture`; checked by `TicketBoard.SelfCheck/CardCheck.cs` |
| Board keyboard shortcuts | `MainWindow.OnPreviewKeyDown` (`Views/MainWindow.xaml.cs`); `Ctrl+Del` is also a `KeyBinding` in the XAML |
| Card look | `TicketCard` DataTemplate in `MainWindow.xaml` + `TicketCardItem`, `AgeBadge`, `PriorityChip` in `Themes/Styles.xaml` |
| Detail panel (right side) | `Views/TicketPanel.xaml` — a UserControl, markup only: the window's `MainViewModel` is its DataContext; its own converters and the `NoteItem`/`CommentItem` templates. It sits in `MainWindow.xaml`'s `PanelHost` border (width 0 → 380 px, shadow), which overlays the board below 1100 px (`OverlayBreakpoint`). |
| Card appear animation | `Ticket.MarkAppear/TakeAppear` (in memory, 500 ms freshness) + `MainWindow.OnCardLoaded` |
| Colors | `Themes/Tokens.Light.xaml` **and** `Tokens.Dark.xaml`; a new key must go in both. Accent brushes are overwritten at runtime by `TokenTheme`. |
| Fonts, shared control styles, app glyph | `Themes/Styles.xaml` (also the form styles `SectionTitle`/`FieldLabel`/`FieldNote` of the settings and export windows) |
| Tray icon, badge, tooltip, tray menu | `App.SetupTray`, `UpdateTrayIcon`, `RenderTrayIcon` (drawn at runtime; see the comments about HICON ownership) |
| Global hotkey | `Services/HotkeyService.cs` (`RegisterHotKey` on a message-only window; `TryParse` also validates the settings field) |
| Autostart | `Services/AutostartService.cs` (HKCU `...\Run`, adds `--minimized`) |
| Saving, backups, corrupt-file handling | `Services/TicketStore.cs` (tmp + rename, daily backup, keeps 30, corrupt → `.corrupt-<ts>`); checked by `TicketBoard.SelfCheck/StoreCheck.cs`, which also pins the file's field names |
| Where data lives | `App.DataDir` (one property, next to the exe) |
| Settings file, password encryption; adding a setting | `Services/AppSettings.cs` (DPAPI CurrentUser; the plain password is `[JsonIgnore]`; tmp + rename; `Load` moves a corrupt file aside and returns the reason, `App` shows it). A new setting touches 5 places — checklist in `TicketBoard/README.md` |
| Error log | `App.LogError` → `errors.log` next to the exe (no rotation) |
| Build and packaging | `TicketBoard.csproj` (Release group), `.github/workflows/build.yml` |
| Confirmations and messages (delete, F5 question, import/refresh reports) | `Views/AskWindow` — `Ask` / `Tell`. The system `MessageBox` stays only on the fatal paths in `App.xaml.cs` |
| Claude via the clipboard: request format, answer text, instructions for Claude | `Services/ClaudeRelay.cs` (`Parse`, `RunAsync`, `Format*`, `Instructions`) + `.SelfCheck.cs`; a new request = a `RelayVerb` + `Parse` + `RunOne` + `Instructions` + the table in the root README. Wiring: `App.ApplyRelay` / `OnClipboardChanged` / `BoardSnapshot`, `Services/ClipboardWatcher.cs`; setting `ClaudeRelayEnabled`, UI in `SettingsWindow`. The user has claude.ai Pro only — **no API key, ever** |

## Gotchas

- Windows hide instead of closing (`OnClosing` → `Cancel` + `Hide`), and `ShutdownMode="OnExplicitShutdown"`. The app only exits from tray → «Выход». `OnExit` calls `SaveNow()`.
- Every column change must go through `Ticket.MoveTo` so `StatusChangedAt`, `CompletedAt` and the animation stay correct. `MoveTicket` also moves the item between the columns' `Items` collections. On drag&drop, gong's default handler moves the item and `OnDropped` only calls `MoveTo`.
- `ColumnViewModel.Items` holds all cards and `View` is the filtered view. Counters show `VisibleCount`. The tray counts `Items`.
- Any `PropertyChanged` on a ticket schedules a save, except the age properties listed in `MainViewModel.OnTicketChanged`. Add new display-only notifying properties to that list.
- `TicketRules` is static and set by `MainViewModel.ApplySettings`. The model reads it directly.
- API calls share one `HttpClient` (no client-wide timeout: each request has its own, 10 s, `GetAsync(timeout:)` — a list page of the export gets 60 s) and return errors as strings, never exceptions. **An error can be
  multi-line**: a short Russian phrase (+ `HTTP <code>`), then the evidence — the server's body via `Evidence` (the user
  wants raw responses) or the exception chain via `Reason`. `Redact` masks credentials; keep it on every new path.
  One-line spots use `Brief`; multi-line spots use the `SelectableText` style so the text can be copied.
- The API uses Basic auth only (no tokens). Warn on `http://` (that already exists); don't remove the DPAPI encryption.
- Response shapes are verified against the live server only where `PROGRESS.md` says so; the parsers tolerate several.
- **WPF-UI 4.3.0 `SymbolRegular` values above `0xFFFF` render blank, silently** (a cast truncates them) — e.g.
  `ArrowImport16`. Check the codepoint, not just the name, before using a symbol that isn't already in this repo.
- **WPF projects drop `System.IO` from the implicit usings** (it clashes with `System.Windows.Shapes.Path`) — add
  `using System.IO;` wherever you touch `File`, `Path` or `IOException`.
- **The agent's Write tool turns `\uXXXX` in file content into the character itself** (JSON-level unescaping) — it broke a
  comment once. Avoid such escapes in code you write, or `grep` for them afterwards.
- **A UserControl's `{StaticResource}` doesn't see the window's resources** (looked up while its own XAML loads, before
  it is in the window) — the window opens with a XamlParseException. Declare what it needs in its own resources or in
  `Themes/`; `xamlcheck.py` checks exactly this scope.
- `pkill -f <pattern>` kills your own shell when the pattern also occurs in the same command line — kill in a separate call.
- **The live server is an ASP.NET Core build that validates parameter types**, not quite the PDF: `count=all` (documented)
  is rejected with `HTTP 400 {"errors":{"count":["The value 'all' is not valid."]}…}` — `count` is a bool there (2026-10-08).
  `FakeIntraservice` refuses it the same way for every scenario (`Refused`); add any other rejection seen live there too.
- **`FakeIntraservice` serves one connection at a time.** A cancelled request leaves HttpClient's just-opened connection idle
  in its pool for a minute, and a server reading it blocked for that minute (a run of the self-check stalled 60 s, one
  time in five). It now drops a connection that sends no request within 2 s — keep that if you touch it. And a cancelled
  run's abandoned requests still reach the fake late: give each scenario its own client path prefix (`/gN`, see
  `MassServer`) instead of counting on a pause to let them drain.
- **Mutation scripts that `git checkout` a file discard your uncommitted edits to it** — commit first.
- Bulk changes to a column's `Items` fire `CollectionChanged` per item, and the handlers are expensive (tray icon
  re-render, full `Recount`). Both are coalesced through `QueueTrayUpdate` / `QueueRecount`; add new handlers the same way.

## Docs to keep in sync

- User-visible behavior → root `README.md` (Russian user guide; the keyboard, settings and error tables).
- Build, structure, API notes → `TicketBoard/README.md`.
- New file or moved responsibility → the table above.
- Always → `PROGRESS.md` (state, open work) and a short entry at the top of `docs/HISTORY.md`.
