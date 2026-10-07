# AGENTS.md — map for coding agents

Read this first, then `PROGRESS.md` (current state, open work). When you finish, update `PROGRESS.md` and add an entry to `docs/HISTORY.md`.

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

## Build and verify

The agent container builds the app and runs every parser check. Use that for each change; CI is for releases.

```
sudo apt-get update; sudo apt-get install -y dotnet-sdk-10.0   # once per container (`;`: update errors on dead PPAs)
cd TicketBoard && dotnet build -c Release    # ~15 s, compiles the WPF app on Linux (EnableWindowsTargeting)
cd .. && dotnet run --project TicketBoard.SelfCheck   # runs every parser Debug.Assert; prints "SelfCheck: OK", exit 0
```

- `TicketBoard.SelfCheck` compiles `Services/HttpIntraserviceClient*.cs`, `IntraserviceLinkParser.cs`, `AppSettings.cs`,
  `ClaudeRelay*.cs`, `AutoSyncRules.cs`, `FakeIntraservice.cs`, `KnowledgeExport*.cs`, `TaskQuery.cs`, `TicketSearch.cs`
  and `ViewModels/SearchViewModel.cs` into a console app; the relay, export, search and window checks run the real
  Intraservice client against a fake server on loopback (`FakeIntraservice`; the export ones do full exports into a temp
  folder — `KnowledgeExport.SelfCheck.Mass.cs` runs a 230-ticket server through pages, stop/resume, retries and breakers), the settings check round-trips `settings.json` in a temp folder. `SearchWindowCheck.cs` drives the real
  `SearchViewModel` (remembered conditions, lists, paging, stale flag, preview, clipboard, export, settings change); what the
  view model takes from the WPF side (board, card, clipboard, data folder) is stubbed in `WpfStubs.cs` — if the view model
  starts using something new from the app, add the same stub there. A parsing change gets a sample in the matching
  `SelfCheck()` and must pass here before it is committed. It refuses to run in Release, where `[Conditional("DEBUG")]` would
  strip every check.
- The UI can't run on Linux. Say so rather than claiming a UI change works; the user tests on his Windows work PC. What
  can be checked without running it: the XAML compiles with the build, and `python3 TicketBoard.SelfCheck/xamlcheck.py
  <window.xaml> <ViewModel.cs> <ViewModelClass> [<RowClass>]` verifies that every `{StaticResource}` key exists and every
  `{Binding}` name is a member of the view model (a missing key crashes the window on open; a wrong binding fails silently) —
  run it after touching a window. Rendering, layout and focus stay unverified.
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
TicketBoard.SelfCheck/     console app that runs the parser, relay, export, search and search-window self-checks on Linux
                           (+ xamlcheck.py: resource keys and bindings of a window's XAML)
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
  Views/                   MainWindow, QuickCaptureWindow, SearchWindow, SettingsWindow, AskWindow
                           (XAML + thin code-behind)
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
| Search and filters, hiding old Done cards | `MainViewModel.Matches`, `RefreshFilters`, `Recount` |
| WIP limit / overload | `ColumnViewModel.IsOverloaded` (visible count) vs. `IsOverloadedTotal` (all cards, used by the tray) |
| Parsing a ticket number from a link or text | `Services/IntraserviceLinkParser.cs` (regex from settings, fallback to the last 4–8 digit number in the URL) |
| Title derivation for new tickets | `MainViewModel.AddFromCapture` |
| Intraservice HTTP calls, error messages | `Services/HttpIntraserviceClient.cs` `GetTaskAsync`, `CheckAsync`, `GetAsync` (status code → Russian message). A response that doesn't parse goes through `Unparsed(json)`, which writes the method name and the first 4000 chars of the body to `errors.log` via `LogUnparsed` — route any new parse failure through it. |
| Intraservice JSON field names | **only** `Services/HttpIntraserviceClient.Parse.cs`; a sample for every shape in `HttpIntraserviceClient.SelfCheck.cs`, run by `TicketBoard.SelfCheck` |
| Comments («Переписка») in the panel | `ViewModels/MainViewModel.Comments.cs` (`LoadComments` / `ToRows`) + `HttpIntraserviceClient.GetLifetimeAsync`; row template `CommentItem` in `MainWindow.xaml`, styles `CommentRow`/`Chip`/`IconToggle`. Never persisted: in memory + a 2-minute cache. |
| Unread comments (card badge, notification, «seen») | Counting rule: `AutoSyncRules.Unread` (+ SelfCheck; server dates only, own comments by `EditorId`, else by name). Auto-sync re-reads the lifetime only of cards whose `Changed` moved (`CheckCommentsAsync` in `.AutoSync.cs`; first sight = baseline, no request). Persisted on `Ticket`: `CommentsCheckedFor`, `CommentsSeenAt`, `UnreadComments`; in memory: `ServerChanged` (set by `Apply`). «Seen» = shown in the open panel of the active board window **and** a user action (selection, panel, ⟳, activation, any click/key on the board): `MarkCommentsSeen` in `.Comments.cs`, `IsBoardActive` + input hooks in `MainWindow`. Auto-sync never marks seen itself (`LoadComments(markSeen: false)`) and always notifies. Baselines from `Changed` get +1 s (`AutoSyncRules.SeenFrom`). Badge: `UnreadBadge` style in the `TicketCard` template. |
| Import of my tickets | `ViewModels/MainViewModel.Intraservice.cs` (`ImportMine`, `FetchMyOpenAsync`) + `HttpIntraserviceClient.GetCurrentUserAsync` / `GetStatusesAsync` / `GetExecutorTasksAsync`; paging and «is the list complete» — `AutoSyncRules.ReadAllPagesAsync` (+ SelfCheck); entry points in `App.SetupTray` and the toolbar in `MainWindow.xaml`; the closed-status names live in `AppSettings.ClosedStatusNames` |
| Refresh all cards («Обновить статусы», F5) | `MainViewModel.Intraservice.cs` (`RefreshAll`) (per-card `GetTaskAsync`, 4 at a time) + `Apply` (the shared "what a sync may overwrite" rule, also used by `SyncAsync`) + `ClosedNames` (shared with the import); entry points `App.SetupTray` and `MainWindow.OnPreviewKeyDown` |
| Search any tickets (filters, preview, copy, export) | Window: `Views/SearchWindow.xaml(.cs)` + `ViewModels/SearchViewModel.cs` (form → `TicketSearch.ResolveAsync` → `GetTasksAsync` pages of 50 + «ещё»; selection → preview/copy/export; `_wanted` = what to select in the lists, `_filling` = code, not the user, is filling them, `IsStale` = the form changed since the list). Opened from the toolbar (`MainViewModel.SearchRequested`), the tray and `Enter` in the board's search box (`MainWindow.ServerSearchRequested`, with the words, filters reset, remembered conditions not overwritten — `_skipRemember`; the form is put back to them when the hidden window is reopened — `_quickMode`, `OpenAsync(restoreQuick)`) — all wired in `App.OnStartup`. «+ На доску» goes through `MainViewModel.AddKnown` (id, url and title are known — no text parsing). «Не больше, заявок» (`Limit`; **0 = everything**, no upper ceiling) belongs to the export only: the search never validates it. An export of ≥ `SearchViewModel.ConfirmFrom` (2000) tickets asks first through the injectable `Confirm` callback (the window sets it to `AskWindow.Ask`, the harness leaves it null or answers); with no conditions at all the resolver adds the note «Условий нет…» (`TaskQuery.IsUnrestricted`). Query model + URL: `Services/TaskQuery.cs` (every list filter of the API doc, pp. 14-20: `ExecutorIds`/`CreatorIds`/`ServiceIds`/`TypeIds`/`StatusIds`, `search`, `Created`/`Changed`/`Closed` `…MoreThan`/`…LessThan`, `filterid`, `archive`+`inactive`, `Sort` = «Поле asc|desc» (default `Changed desc`; `StableSort` = `Created asc, Id asc` for reading everything); `Outside`/`PastEnd` = the guards for a silently ignored date filter). Form → query: `Services/TicketSearch.cs` (`SearchFilter` is what `AppSettings.LastSearch` stores; names → ids through `api/user?search=`, service + children by `Path`, closed/open statuses as in F5, dates `дд.мм.гггг` → period). Reference lists (services, task types, saved filters, users): `HttpIntraserviceClient.GetServicesAsync`/`GetTaskTypesAsync`/`GetSavedFiltersAsync`/`FindUsersAsync` + `ParseRefs`. The board's quick search and the relay's `TB search` still use `SearchAsync` (20 newest). |
| Export for the knowledge base (files for an agent / Obsidian) and the agent's text | Engine, split by responsibility: `Services/KnowledgeExport.cs` (the run: `BuildAsync` = one ticket as Markdown — the card via `GetTaskAsync` (service, type, categories, group: the live server leaves them out of list rows, merged by `WithDetails`) plus the full lifetime via `GetLifetimePageAsync` pages, the server's Paginator trusted when it has Page and PageCount — `IntraserviceLifetime.Paged`; an unreadable card or lifetime = no text — shared by the preview, the clipboard and the files; `ExportRowsAsync` = given rows; `RunAsync` = the list by a `TaskQuery`; the nested `Job` holds the counters, the circuit breaker and the progress/ETA line), `.Format.cs` (file name + Markdown text), `.Files.cs` (folders `tickets/<yyyy-MM>/` by the row's created date via `ShardOf`, `ScanNames` = file-name→id map without reading files, `IsCurrent` = the file's `format`+`changed` match the row, `MigrateFlat` = the 0.10–0.12 flat layout moves into months, month + root indexes). **`RunAsync(limit 0)` exports everything**: no ceiling; list pages of 200 (`count=all`, 60 s timeout, `TaskQuery.StableSort` — creation date never changes, so the walk doesn't shift for hours) are exported (4 parallel) page by page before the next page is asked for — memory is one page + the name map + the `seen` id set. With `limit > 0` it is the first N by `Changed desc` (rows outside the period are skipped and don't count to the limit; the list is cut off past the period — `TaskQuery.PastEnd`). Stop/resume is just the incremental rule (unchanged files are skipped), so a stopped, crashed or tripped run continues where it was. `Retrying` repeats `HttpIntraserviceClient.IsTransient` errors after `RetryDelays` (2, 5, 15 s); the `Job` trips the whole run (`Fatal` text, cancels `Abort`) on HTTP 401, 30 transient failures in a row, 25 refusals (404/403…) with no success, 10 write failures in a row. A 4xx on page 1 with the stable sort falls back to `Changed desc` (noted); a list that is not ascending, shifts, or repeats a page is noted/stopped. Files are found by the number the file name starts with (`IdInName`), so `tickets/` belongs to the export; contacts never exported; `FormatVersion` — bump it when the file format changes. Self-check: `KnowledgeExport.SelfCheck.cs` (first/again/rename/period scenarios) and `.SelfCheck.Mass.cs` (`MassServer`: every scenario gets its own client path `/gN`, so requests that a cancelled run abandoned are not counted in the next one) against `FakeIntraservice`. The folder is `AppSettings.KnowledgeDir` (`KnowledgePath`). Extra row fields: `IntraserviceExtra` (service, type, categories, resolution date). |
| Background auto-sync (timer, new assignments, closed-ticket notifications) | `ViewModels/MainViewModel.AutoSync.cs` (`ApplyAutoSync`, `AutoSyncAsync`) — reuses `FetchMyOpenAsync` / `NewCard` (import) and `Apply` / `ClosedToMove` / `AskMoveClosed` (F5) from `.Intraservice.cs`; what it never adds — `Services/AutoSyncRules.cs` (+ SelfCheck), persisted in `AppSettings.AutoSyncSkipIds` (reset with the lifecycle flags in `MainViewModel.ApplySettings` when `AppSettings.AutoSyncAccount` ≠ `AccountKey` — URL/login changed, also by hand between runs); notifications with click actions — `MainViewModel.Notify` → `App.ShowTrayNotification`; closed-but-not-moved counter in the board title — `ShowAutoSyncState`. It never moves a card into «Готово» by itself; the only moves it makes are a reopened «Готово» card back to «Входящие» and a «Ждёт ответа» card to «В работе» when the requester replies (rule `AutoSyncRules.RequesterReply` + SelfCheck: a new unread comment whose author's name equals the card's `Creator`; applied at the end of `AutoSyncAsync`, only to tickets in this pass's open list — so never closed or reassigned ones, and never on a stale card status; notification section `Answered`). |
| What a sync overwrites on a ticket | `MainViewModel.Intraservice.cs` → `Apply` (used by `SyncAsync`, `RefreshAll` and auto-sync — every N minutes on listed cards and rechecks): status, creator with phone/email, executors and executor group always (a field missing from the response keeps the old value), title only if it's still the auto `Заявка #N`, description only if empty. List rows go through `AsTask`. |
| Reopened / reassigned-away tickets | Rule: `AutoSyncRules.Track` (+ SelfCheck) over the persisted `Ticket.AssignedToMe` (null = not seen yet → baseline, no events). In `AutoSyncAsync`: listed cards in «Готово» that were not mine → moved to «Входящие» (`Lifecycle.Reopened`); cards that left a complete list and whose recheck says open → «больше не на вас» (`Reassigned`, card stays; an unresolved «статус N» decides nothing — `HttpIntraserviceClient.IsResolvedStatus`). Transitions are computed during the pass and applied only at its end, right before the notification. A pass is dropped (`StillCurrent`) only when the account changed or auto-sync was switched off mid-pass; a plain settings save no longer aborts it (it used to, and lost notifications for statuses already written). An account change resets `AssignedToMe` (`AutoSyncAccount` vs `AccountKey` in `MainViewModel.ApplySettings`). Notification sections: `NotifyChanges` / `AutoSyncNews`, list lines via `Lines`. |
| Creator / executors in the panel | Parsed in `.Parse.cs` (`Field`, `Names`), stored on `Ticket` (`Creator`, `CreatorPhone`, `CreatorEmail` → `CreatorContacts`, `Executors`, `ExecutorGroup`), rows 5–6 of the panel grid in `MainWindow.xaml` (`SelectableBody`/`SelectableText`). Cards without executors (before 0.8.0) or contacts (before 0.9.0) get the people fields quietly on first open (`FillPeopleAsync` → `ApplyPeople` only, status untouched; once per run). Also in the relay's `TB ticket` answer (`ClaudeRelay.People`). |
| Quick-capture behavior (keys 1/2/3, Enter, Esc, clipboard) | `Views/QuickCaptureWindow.xaml.cs` + `ViewModels/QuickCaptureViewModel.cs` (400 ms debounced title lookup) |
| Board keyboard shortcuts | `MainWindow.OnPreviewKeyDown` (`Views/MainWindow.xaml.cs`); `Ctrl+Del` is also a `KeyBinding` in the XAML |
| Card look | `TicketCard` DataTemplate in `MainWindow.xaml` + `TicketCardItem`, `AgeBadge`, `PriorityChip` in `Themes/Styles.xaml` |
| Detail panel (right side) | `MainWindow.xaml`, the `PanelHost` border. It overlays the board below 1100 px (`OverlayBreakpoint`). |
| Card appear animation | `Ticket.MarkAppear/TakeAppear` (in memory, 500 ms freshness) + `MainWindow.OnCardLoaded` |
| Colors | `Themes/Tokens.Light.xaml` **and** `Tokens.Dark.xaml`; a new key must go in both. Accent brushes are overwritten at runtime by `TokenTheme`. |
| Fonts, shared control styles, app glyph | `Themes/Styles.xaml` (also the form styles `SectionTitle`/`FieldLabel`/`FieldNote` of the settings and export windows) |
| Tray icon, badge, tooltip, tray menu | `App.SetupTray`, `UpdateTrayIcon`, `RenderTrayIcon` (drawn at runtime; see the comments about HICON ownership) |
| Global hotkey | `Services/HotkeyService.cs` (`RegisterHotKey` on a message-only window; `TryParse` also validates the settings field) |
| Autostart | `Services/AutostartService.cs` (HKCU `...\Run`, adds `--minimized`) |
| Saving, backups, corrupt-file handling | `Services/TicketStore.cs` (tmp + rename, daily backup, keeps 30, corrupt → `.corrupt-<ts>`) |
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
- `pkill -f <pattern>` kills your own shell when the pattern also occurs in the same command line — kill in a separate call.
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
