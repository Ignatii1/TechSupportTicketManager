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
  `ClaudeRelay*.cs` and `AutoSyncRules.cs` into a console app; the relay check runs the real Intraservice client against a fake server on
  loopback, the settings check round-trips `settings.json` in a temp folder. A parsing change gets a sample in the matching `SelfCheck()` and must pass here before it is
  committed. It refuses to run in Release, where `[Conditional("DEBUG")]` would strip every check.
- The UI can't run on Linux. Say so rather than claiming a UI change works; the user tests on his Windows work PC.
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
TicketBoard.SelfCheck/     console app that runs the parser and Claude-relay self-checks on Linux
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
  ViewModels/              MainViewModel (board; partials .Intraservice = sync/import/F5, .Comments = «Переписка»),
                           ColumnViewModel, QuickCaptureViewModel, SearchViewModel, SettingsViewModel
  Views/                   MainWindow, QuickCaptureWindow, SearchWindow, SettingsWindow, AskWindow (XAML + thin code-behind)
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
| Server-side search | `ViewModels/SearchViewModel.cs` + `Views/SearchWindow.xaml` + `HttpIntraserviceClient.SearchAsync`; triggered by `MainWindow.ServerSearchRequested`, wired in `App.OnStartup` |
| Background auto-sync (timer, new assignments, closed-ticket notifications) | `ViewModels/MainViewModel.AutoSync.cs` (`ApplyAutoSync`, `AutoSyncAsync`) — reuses `FetchMyOpenAsync` / `NewCard` (import) and `Apply` / `ClosedToMove` / `AskMoveClosed` (F5) from `.Intraservice.cs`; what it never adds — `Services/AutoSyncRules.cs` (+ SelfCheck), persisted in `AppSettings.AutoSyncSkipIds` (reset by `SettingsViewModel.Save` when the URL or login changes); notifications with click actions — `MainViewModel.Notify` → `App.ShowTrayNotification`; closed-but-not-moved counter in the board title — `ShowAutoSyncState`. It never moves a card by itself. |
| What a sync overwrites on a ticket | `MainViewModel.Intraservice.cs` → `Apply` (used by `SyncAsync`, `RefreshAll` and auto-sync — every N minutes on listed cards and rechecks): status, creator with phone/email, executors and executor group always (a field missing from the response keeps the old value), title only if it's still the auto `Заявка #N`, description only if empty. List rows go through `AsTask`. |
| Reopened / reassigned-away tickets | Rule: `AutoSyncRules.Track` (+ SelfCheck) over the persisted `Ticket.AssignedToMe` (null = not seen yet → baseline, no events). In `AutoSyncAsync`: listed cards in «Готово» that were not mine → moved to «Входящие» (`Lifecycle.Reopened`); cards that left a complete list and whose recheck says open → «больше не на вас» (`Reassigned`, card stays). Rechecks of cards that just left the list go first. An account change (`AppSettings.AccountKey`, `MainViewModel.ApplySettings`) resets `AssignedToMe`. Notification sections: `NotifyChanges` / `AutoSyncNews`. |
| Creator / executors in the panel | Parsed in `.Parse.cs` (`Field`, `Names`), stored on `Ticket` (`Creator`, `CreatorPhone`, `CreatorEmail` → `CreatorContacts`, `Executors`, `ExecutorGroup`), rows 5–6 of the panel grid in `MainWindow.xaml` (`SelectableBody`/`SelectableText`). Cards from before 0.8.0 are filled quietly on first open (`FillPeopleAsync`, once per run). Also in the relay's `TB ticket` answer (`ClaudeRelay.People`). |
| Quick-capture behavior (keys 1/2/3, Enter, Esc, clipboard) | `Views/QuickCaptureWindow.xaml.cs` + `ViewModels/QuickCaptureViewModel.cs` (400 ms debounced title lookup) |
| Board keyboard shortcuts | `MainWindow.OnPreviewKeyDown` (`Views/MainWindow.xaml.cs`); `Ctrl+Del` is also a `KeyBinding` in the XAML |
| Card look | `TicketCard` DataTemplate in `MainWindow.xaml` + `TicketCardItem`, `AgeBadge`, `PriorityChip` in `Themes/Styles.xaml` |
| Detail panel (right side) | `MainWindow.xaml`, the `PanelHost` border. It overlays the board below 1100 px (`OverlayBreakpoint`). |
| Card appear animation | `Ticket.MarkAppear/TakeAppear` (in memory, 500 ms freshness) + `MainWindow.OnCardLoaded` |
| Colors | `Themes/Tokens.Light.xaml` **and** `Tokens.Dark.xaml`; a new key must go in both. Accent brushes are overwritten at runtime by `TokenTheme`. |
| Fonts, shared control styles, app glyph | `Themes/Styles.xaml` |
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
- API calls share one `HttpClient` (10 s timeout) and return errors as strings, never exceptions. **An error can be
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
- Bulk changes to a column's `Items` fire `CollectionChanged` per item, and the handlers are expensive (tray icon
  re-render, full `Recount`). Both are coalesced through `QueueTrayUpdate` / `QueueRecount`; add new handlers the same way.

## Docs to keep in sync

- User-visible behavior → root `README.md` (Russian user guide; the keyboard, settings and error tables).
- Build, structure, API notes → `TicketBoard/README.md`.
- New file or moved responsibility → the table above.
- Always → `PROGRESS.md` (state, open work) and a short entry at the top of `docs/HISTORY.md`.
