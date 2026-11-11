# The board: tickets, columns, filters, quick capture

`Models/Ticket.cs`, `ViewModels/MainViewModel.cs`, `ColumnViewModel.cs`, `QuickCaptureViewModel.cs`. Checks: areas
`board`, `card`, `store`.

## Ticket fields / what gets saved

`Models/Ticket.cs`. `[ObservableProperty]` fields and public properties are serialized **by name** into
`tickets.json`. Renaming breaks existing data. Computed properties need `[JsonIgnore]`.

## Age badge logic (warn/overdue), day plurals

`Ticket.AgeState`, `AgeLabel`, `AgeText`, `Plural` in `Models/Ticket.cs`; thresholds in `TicketRules` (set from
settings)

## Columns (names, order, count)

`TicketStatus` enum and its titles `TicketStatusText.Of` (`Models/Ticket.cs`; the `MainViewModel` constructor builds
`Columns` from the enum, `StatusToTextConverter` and the settings' column statuses use the same titles) + context menu
in `MainWindow.xaml` (`TicketCard` template). Enum names are stored in JSON (`tickets.json`, and `StatusOnMove`'s keys
in `settings.json`).

## Moving tickets between columns

`MainViewModel.MoveTicket` / `MoveSelected` / `OnDropped` / `SelectedStatus`; drag&drop target is `ColumnViewModel`
(`IDropTarget`)

## Search and filters, hiding old Done cards

`MainViewModel.Matches`, `RefreshFilters`, `Recount` (checked by `TicketBoard.SelfCheck/CardCheck.cs`)

## WIP limit / overload

`ColumnViewModel.IsOverloaded` (visible count) vs. `IsOverloadedTotal` (all cards, used by the tray)

## Parsing a ticket number from a link or text

`Services/IntraserviceLinkParser.cs` (regex from settings, fallback to the last 4–8 digit number in the URL)

## Title derivation for new tickets

`MainViewModel.AddFromCapture`

## Quick-capture behavior (keys 1/2/3, Enter, Esc, clipboard)

`Views/QuickCaptureWindow.xaml.cs` + `ViewModels/QuickCaptureViewModel.cs` (400 ms debounced title lookup; the hint
starts as the empty-field one — `Reset` on the first open changes nothing) + `MainViewModel.AddFromCapture`; checked
by `TicketBoard.SelfCheck/CardCheck.cs`

## Notes and gotchas

- Every column change must go through `Ticket.MoveTo` so `StatusChangedAt`, `CompletedAt` and the animation stay
  correct. `MoveTicket` also moves the item between the columns' `Items` collections. On drag&drop, gong's default
  handler moves the item and `OnDropped` only calls `MoveTo`.

- `ColumnViewModel.Items` holds all cards and `View` is the filtered view. Counters show `VisibleCount`. The tray
  counts `Items`.

- Any `PropertyChanged` on a ticket schedules a save, except the age properties listed in
  `MainViewModel.OnTicketChanged`. Add new display-only notifying properties to that list.

- `TicketRules` is static and set by `MainViewModel.ApplySettings`. The model reads it directly.

- Bulk changes to a column's `Items` fire `CollectionChanged` per item, and the handlers are expensive (tray icon
  re-render, full `Recount`). Both are coalesced through `QueueTrayUpdate` / `QueueRecount`; add new handlers the same
  way.
