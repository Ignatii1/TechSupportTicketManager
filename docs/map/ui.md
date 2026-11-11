# Windows, look, keys, dialogs

`Views/`, `Themes/`, `Converters/`. Nothing here runs on Linux: the build compiles the XAML, `xamlcheck.py` checks
keys and bindings, the rest is the user's eyes on Windows.

## Board keyboard shortcuts

`MainWindow.OnPreviewKeyDown` (`Views/MainWindow.xaml.cs`); `Ctrl+Del` is also a `KeyBinding` in the XAML

## Card look

`TicketCard` DataTemplate in `MainWindow.xaml` + `TicketCardItem`, `AgeBadge`, `PriorityChip` in `Themes/Styles.xaml`

## Detail panel (right side)

`Views/TicketPanel.xaml` — a UserControl, markup only: the window's `MainViewModel` is its DataContext; its own
converters and the `NoteItem`/`CommentItem` templates. It sits in `MainWindow.xaml`'s `PanelHost` border (width 0 →
380 px, shadow), which overlays the board below 1100 px (`OverlayBreakpoint`).

## Card appear animation

`Ticket.MarkAppear/TakeAppear` (in memory, 500 ms freshness) + `MainWindow.OnCardLoaded`

## Colors

`Themes/Tokens.Light.xaml` **and** `Tokens.Dark.xaml`; a new key must go in both. Accent brushes are overwritten at
runtime by `TokenTheme`.

## Fonts, shared control styles, app glyph

`Themes/Styles.xaml` (also the form styles `SectionTitle`/`FieldLabel`/`FieldNote` of the settings and export windows)

## Confirmations and messages (delete, F5 question, import/refresh reports)

`Views/AskWindow` — `Ask` / `Tell`. The system `MessageBox` stays only on the fatal paths in `App.xaml.cs`

## Notes and gotchas

- The UI can't run on Linux. Say so rather than claiming a UI change works; the user tests on his Windows work PC.
  What can be checked without running it: the XAML compiles with the build, and `python3
  TicketBoard.SelfCheck/xamlcheck.py <file.xaml> <ViewModelClass> [<RowClass>]` verifies that every `{StaticResource}`
  key exists where it is used (own resources in scope, declared above the use, or App/Themes) and that every
  `{Binding}` path exists link by link, through property types, template `DataType`s and all parts of a partial class
  (a missing key crashes the window on open; a wrong binding fails silently) — run it after touching a window or the
  panel; `--selftest` checks the checker. Rendering, layout and focus stay unverified.

- Windows hide instead of closing (`OnClosing` → `Cancel` + `Hide`), and `ShutdownMode="OnExplicitShutdown"`. The
  app only exits from tray → «Выход». `OnExit` calls `SaveNow()`.

- **WPF-UI 4.3.0 `SymbolRegular` values above `0xFFFF` render blank, silently** (a cast truncates them) — e.g.
  `ArrowImport16`. Check the codepoint, not just the name, before using a symbol that isn't already in this repo.

- **A UserControl's `{StaticResource}` doesn't see the window's resources** (looked up while its own XAML loads,
  before it is in the window) — the window opens with a XamlParseException. Declare what it needs in its own resources
  or in `Themes/`; `xamlcheck.py` checks exactly this scope.
