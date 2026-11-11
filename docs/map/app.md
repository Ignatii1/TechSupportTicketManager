# The app shell: tray, hotkey, autostart, data files, settings, log, build, the Claude relay

`App.xaml.cs`, `Services/` (store, settings, hotkey, autostart), `Services/ClaudeRelay*.cs`. Checks: areas `store`,
`settings`, `relay`.

## Tray icon, badge, tooltip, tray menu

`App.SetupTray`, `UpdateTrayIcon`, `RenderTrayIcon` (drawn at runtime; see the comments about HICON ownership)

## Global hotkey

`Services/HotkeyService.cs` (`RegisterHotKey` on a message-only window; `TryParse` also validates the settings field)

## Autostart

`Services/AutostartService.cs` (HKCU `...\Run`, adds `--minimized`)

## Saving, backups, corrupt-file handling

`Services/TicketStore.cs` (tmp + rename, daily backup, keeps 30, corrupt → `.corrupt-<ts>`); checked by
`TicketBoard.SelfCheck/StoreCheck.cs`, which also pins the file's field names

## Where data lives

`App.DataDir` (one property, next to the exe)

## Settings file, password encryption; adding a setting

`Services/AppSettings.cs` (DPAPI CurrentUser; the plain password is `[JsonIgnore]`; tmp + rename; `Load` moves a
corrupt file aside and returns the reason, `App` shows it). A new setting touches 5 places — checklist in
`TicketBoard/README.md`

## Error log

`App.LogError` → `errors.log` next to the exe (no rotation)

## Build and packaging

`TicketBoard.csproj` (Release group), `.github/workflows/build.yml`

## Claude via the clipboard: request format, answer text, instructions for Claude

`Services/ClaudeRelay.cs` (`Parse`, `RunAsync`, `Format*`, `Instructions`) + `.SelfCheck.cs`; a new request = a
`RelayVerb` + `Parse` + `RunOne` + `Instructions` + the table in the root README. Wiring: `App.ApplyRelay` /
`OnClipboardChanged` / `BoardSnapshot`, `Services/ClipboardWatcher.cs`; setting `ClaudeRelayEnabled`, UI in
`SettingsWindow`. The user has claude.ai Pro only — **no API key, ever**
