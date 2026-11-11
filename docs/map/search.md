# The search window

`Views/SearchWindow.xaml(.cs)`, `ViewModels/SearchViewModel.cs`, `Services/TaskQuery.cs`, `Services/TicketSearch.cs`.
Checks: areas `search`, `query`, `resolve`.

## Search any tickets (filters, preview, copy, export)

Window: `Views/SearchWindow.xaml(.cs)` + `ViewModels/SearchViewModel.cs` (form → `TicketSearch.ResolveAsync` →
`GetTasksAsync` pages of 50 + «ещё»; selection → preview/copy/export; `_wanted` = what to select in the lists,
`_filling` = code, not the user, is filling them, `IsStale` = the form changed since the list). Opened from the
toolbar (`MainViewModel.SearchRequested`), the tray and `Enter` in the board's search box
(`MainWindow.ServerSearchRequested`, with the words, filters reset, remembered conditions not overwritten —
`_skipRemember`; the form is put back to them when the hidden window is reopened — `_quickMode`,
`OpenAsync(restoreQuick)`) — all wired in `App.OnStartup`. «+ На доску» goes through `MainViewModel.AddKnown` (id, url
and title are known — no text parsing). «Не больше, заявок» (`Limit`; **0 = everything**, no upper ceiling) belongs to
the export only: the search never validates it. An export of ≥ `SearchViewModel.ConfirmFrom` (2000) tickets asks first
through the injectable `Confirm` callback (the window sets it to `AskWindow.Ask`, the harness leaves it null or
answers); with no conditions at all the resolver adds the note «Условий нет…» (`TaskQuery.IsUnrestricted`). Query
model + URL: `Services/TaskQuery.cs` (every list filter of the API doc, pp. 14-20:
`ExecutorIds`/`CreatorIds`/`ServiceIds`/`TypeIds`/`StatusIds`, `ObserverIds` (0.21.0, the form's «Наблюдатель» —
`SearchFilter.ObserverMine`/`Observer`; ticking observer «Я» clears executor «Я», which is on by default and ANDs with
it — `OnObserverMineChanged`, not while the code fills the form, `_applying`; both ticked by hand → a note from
`TicketSearch.ResolveAsync`, and so does an observer name with executor «Я» (0.22.0: only my tickets with that
observer); unseen live, so `Outside` also skips a row whose `ObserverIds` lack every asked one, and `OutsideWords`
names the condition in the export's summary and the window's warning; `RowFields` carries `ObserverIds,Observers` for
saved filters), `search`, `Created`/`Changed`/`Closed` `…MoreThan`/`…LessThan`, `filterid`, `archive`+`inactive`,
`Sort` = «Поле asc|desc» (default `Changed desc`; `StableSort` = `Created asc, Id asc` for reading everything);
`Outside`/`PastEnd` = the guards for a silently ignored date filter). Form → query: `Services/TicketSearch.cs`
(`SearchFilter` is what `AppSettings.LastSearch` stores; names → ids through `api/user?search=`, service + children by
`Path`, closed/open statuses as in F5, dates `дд.мм.гггг` → period). Reference lists (services, task types, saved
filters, users): `HttpIntraserviceClient.GetServicesAsync`/`GetTaskTypesAsync`/`GetSavedFiltersAsync`/`FindUsersAsync` +
`ParseRefs`; services fall back (refused **or empty**) to `for=filtertasks`, then to the `Services` block of the
latest 1000 tickets (`GetTaskServicesAsync`) — but not on a transient error or 401 (a partial list would stick for the
session) — and `SearchViewModel.ServiceNote` under the field says which list it is or why it is empty; with a partial
list (`_servicesPartial`) labels are flat (sorted by name); a remembered service missing from a partial or failed list
stays as «Сервис №…» (`_referencesTried` tells «failed» from «not read yet», so it is set before the lists are filled)
and a search with children says that children come only from that list; a 500 on the directory counts as a refusal
(fallbacks run), and an error names both causes; the board's quick search (`StartWithAsync`) loads missing lists in
the background; the other lists' failures go to `Notes` at the top of the conditions. Totals: `FoundNum` shows a
capped total as «1 000+», the export button and the confirm question (`ConfirmTitle`/`ConfirmText`/`ExportCount`)
don't invent a number then. The board's quick search and the relay's `TB search` still use `SearchAsync` (20 newest).
