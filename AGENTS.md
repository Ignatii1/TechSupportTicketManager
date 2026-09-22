# AGENTS.md — map for coding agents

Read this first, then `PROGRESS.md` (current state, open work, history). When you finish a session, append to `PROGRESS.md`.

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

```
cd TicketBoard
dotnet build                  # compiles on Linux/macOS too (EnableWindowsTargeting) — compile check only
dotnet run                    # Windows only; Debug build also runs the SelfCheck()s at startup
dotnet publish -c Release     # single self-contained compressed exe → bin/Release/net10.0-windows/win-x64/publish/
```

- There's no test project. The only automated checks are `HttpIntraserviceClient.SelfCheck()` (Debug.Assert on `Parse`) and
  `IntraserviceLinkParser.SelfCheck()` (Debug.Assert on `TryParse`). If you change parsing, add a sample to the right one.
- The developer works on Linux (CachyOS), so **you can't run the UI there**. Say so rather than claiming a UI change works; the
  user tests on their Windows work PC.
- CI: `.github/workflows/build.yml` on `windows-latest` publishes the exe on every push to `main` and every PR (downloadable
  as a run artifact). A `v*` tag also creates a GitHub Release. Tag only when the user asks, and bump `<Version>` in
  `TicketBoard.csproj` in the same commit — that number is what the exe's file properties show.
  **Tags cannot be pushed from the agent container** (the git proxy refuses them; branches are fine). Release instead by
  dispatching `build.yml` with the `release_tag` input — GitHub creates the tag and the Release itself.

## Repository layout

```
README.md                  user guide (Russian): install, usage, settings, data files, troubleshooting
AGENTS.md                  this file
PROGRESS.md                state, open tasks, change log
LICENSE                    MIT
.github/workflows/build.yml
TicketBoard/
  README.md                developer notes (Russian): build, CI, short file list, Intraservice API notes
  TicketBoard.csproj       packages, Release publish settings
  app.manifest             PerMonitorV2 DPI, Win10/11
  App.xaml(.cs)            composition root and app-level concerns (see below)
  Models/Ticket.cs         Ticket, Note, TicketStatus/Priority/AgeState enums, TicketRules
  Services/                persistence, settings, Intraservice API, hotkey, autostart, theme
  ViewModels/              MainViewModel (board), ColumnViewModel, QuickCaptureViewModel, SettingsViewModel
  Views/                   MainWindow, QuickCaptureWindow, SettingsWindow (XAML + thin code-behind)
  Themes/                  Tokens.Light/Dark.xaml (colors), Styles.xaml (shared styles, fonts, glyph)
  Converters/Converters.cs all XAML value converters
  Assets/                  app.ico, tray-light.ico, tray-dark.ico (embedded as WPF Resources)
```

## How it fits together

`App.OnStartup` (`App.xaml.cs`) is the only place that creates and wires objects. There's no DI container.

1. Single-instance mutex `Local\TicketBoard.SingleInstance`. A second copy exits.
2. Global error handlers: `ShowError` (message box) and `LogError` (`errors.log`).
3. `AppSettings.Load` → `IntraserviceLinkParser(settings)` → `HttpIntraserviceClient.From(settings)` (`null` = API not configured) →
   `MainViewModel(TicketStore, settings, parser, client)`.
4. Theme: WPF-UI follows the system theme; `TokenTheme.Apply` swaps in `Tokens.Light/Dark.xaml` and replaces the accent with the system accent.
5. Creates the windows, then `SetupTray` and `SetupHotkey`. The main window is shown unless the app was started with `--minimized` (autostart).

Settings changes apply live. `SettingsWindow.Saved` → `App.ApplySettings` → new API client → `MainViewModel.ApplySettings` and
`QuickCaptureViewModel.ApplySettings` → tray redraw → hotkey re-register. The parser reads the regex from the shared
`AppSettings` instance on each call.

Data flow for a new ticket: hotkey → `QuickCaptureWindow.ShowCapture` (prefills a ticket URL from the clipboard) → Enter →
`MainViewModel.AddFromCapture` (parse URL/number, pick a title, insert into Inbox) → `SyncAsync` fills title, description and
external status from the API → `PropertyChanged` → debounced save (600 ms) → `TicketStore.Save`.

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
| Intraservice JSON field names | **only** `HttpIntraserviceClient.Parse` / `ParseLifetime` / `ParseSearch`, plus samples in `SelfCheck` |
| Comments («Переписка») in the panel | `MainViewModel.LoadComments` / `ToRows` + `HttpIntraserviceClient.GetLifetimeAsync`; row template `CommentItem` in `MainWindow.xaml`, styles `CommentRow`/`Chip`/`IconToggle`. Never persisted: in memory + a 2-minute cache. |
| Import of my tickets | `MainViewModel.ImportMine` + `HttpIntraserviceClient.GetCurrentUserIdAsync` / `GetStatusesAsync` / `GetExecutorTasksAsync`; entry points in `App.SetupTray` and the toolbar in `MainWindow.xaml`; the closed-status names live in `AppSettings.ClosedStatusNames` |
| Refresh all cards («Обновить статусы», F5) | `MainViewModel.RefreshAll` (per-card `GetTaskAsync`, 4 at a time) + `Apply` (the shared "what a sync may overwrite" rule, also used by `SyncAsync`) + `ClosedNames` (shared with the import); entry points `App.SetupTray` and `MainWindow.OnPreviewKeyDown` |
| Server-side search | `ViewModels/SearchViewModel.cs` + `Views/SearchWindow.xaml` + `HttpIntraserviceClient.SearchAsync`; triggered by `MainWindow.ServerSearchRequested`, wired in `App.OnStartup` |
| What a sync overwrites on a ticket | `MainViewModel.Apply` (used by both `SyncAsync` and `RefreshAll`): status always, title only if it's still the auto `Заявка #N`, description only if empty |
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
| Settings file, password encryption | `Services/AppSettings.cs` (DPAPI CurrentUser; the plain password is `[JsonIgnore]`) |
| Error log | `App.LogError` → `errors.log` next to the exe (no rotation) |
| Build and packaging | `TicketBoard.csproj` (Release group), `.github/workflows/build.yml` |

### Adding a new setting (touches 5 places)

1. Add the property with a default to `AppSettings`.
2. In `SettingsViewModel`, add a string field, an `Error` field, validation in `On<Name>Changed`, a term in `IsValid`, and the assignment in `Save`.
3. Add the field to `Views/SettingsWindow.xaml`.
4. Consume it: read it in the relevant `ApplySettings` so it applies without a restart.
5. Add a row to the settings table in the root `README.md`.

## Gotchas

- Windows hide instead of closing (`OnClosing` → `Cancel` + `Hide`), and `ShutdownMode="OnExplicitShutdown"`. The app only exits from tray → «Выход». `OnExit` calls `SaveNow()`.
- Every column change must go through `Ticket.MoveTo` so `StatusChangedAt`, `CompletedAt` and the animation stay correct. `MoveTicket` also moves the item between the columns' `Items` collections. On drag&drop, gong's default handler moves the item and `OnDropped` only calls `MoveTo`.
- `ColumnViewModel.Items` holds all cards and `View` is the filtered view. Counters show `VisibleCount`. The tray counts `Items`.
- Any `PropertyChanged` on a ticket schedules a save, except the age properties listed in `MainViewModel.OnTicketChanged`. Add new display-only notifying properties to that list.
- `TicketRules` is static and set by `MainViewModel.ApplySettings`. The model reads it directly.
- Async API calls go through a shared `HttpClient` with a 10 s timeout. Errors come back as short Russian strings, not exceptions. Keep secrets out of those messages.
- The API uses Basic auth only (no tokens). Warn on `http://` (that already exists); don't remove the DPAPI encryption.
- The Intraservice response format is **unverified** against a real server (see `PROGRESS.md`).
- **WPF-UI 4.3.0 `SymbolRegular` entries above `0xFFFF` do not render.** `SymbolIcon` goes through
  `Encoding.Unicode.GetString(BitConverter.GetBytes((int)icon))`, which truncates a 5-hex-digit codepoint to garbage —
  the icon comes out blank with no error anywhere. `ArrowImport16` (`0xF0384`) is one of them; a whole block at the end
  of the enum is. Check the codepoint, not just the name, before using a symbol that isn't already in this repo.
- Bulk changes to a column's `Items` fire `CollectionChanged` per item, and the handlers are expensive (tray icon
  re-render, full `Recount`). Both are coalesced through `QueueTrayUpdate` / `QueueRecount`; add new handlers the same way.

## Docs to keep in sync

- User-visible behavior → root `README.md` (Russian user guide; the keyboard, settings and error tables).
- Build, structure, API notes → `TicketBoard/README.md`.
- New file or moved responsibility → the table above.
- Always → an entry in `PROGRESS.md`.
