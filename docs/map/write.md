# Writes into Intraservice: the comment and the status

The only two writes, both off by default, never retried. `ViewModels/MainViewModel.Write.cs`, `.Status.cs`,
`Services/HttpIntraserviceClient.Write.cs`. Checks: areas `comment`, `status`.

## Comment into the ticket from the panel (0.24.0, stage 8 step 1 — the only write)

Switch `AppSettings.AllowIntraserviceWrite` («Разрешить комментарии в заявки из панели»,
`SettingsViewModel.AllowWrite`, section «Запись в Интрасервис» of `SettingsWindow`). Field in `Views/TicketPanel.xaml`
above «Переписка» (visible by `CanComment`: switch + API + a number; never locked while sending — a read-only box
would let the board's single-key shortcuts fire). Logic: `ViewModels/MainViewModel.Write.cs` — `CommentDraft` (text,
internal — the default, result; per ticket id in `_drafts`, in memory only, forgotten on an account change —
`ForgetDrafts`; `CommentText`/`CommentInternal`/`CommentResult` read the selected ticket's draft, `NotifyDraft` on
selection and `ApplySettings`), `SendComment` (`AllowConcurrentExecutions` — one ticket's send doesn't lock the button
for the others; `_sending` — the same ticket twice doesn't go): the card (`ChangedText` — `Changed` as the server sent
it, `.Parse.cs`) then the lifetime; the draft taken again (edited during the check — that goes); another card selected
meanwhile, or rows other than those on screen (`_commentsShownFor`, `Comments.SequenceEqual`) → not sent, the fresh
rows shown (`ShowFresh` in `.Comments.cs`; only hidden status-only rows new → the message names the eye button);
`AskWindow.Ask` preview (number, the kind first, the server's title folded to one line — the requester writes it, the
whole text); after it the switch and the `AccountKey` again (a plain settings save makes a new client of the same
account — not a reason); `HttpIntraserviceClient.CommentAsync` (`Services/HttpIntraserviceClient.Write.cs`: `PUT
api/task/{id}` through its own pool-free `WriteHttp` + `Connection: close` (.NET replays a request that failed on a
pooled connection before the first response byte — a second comment), JSON `Comment`/`IsPrivateComment`/`Changed`,
`Device-Name: TicketBoard`, `Device-Version`; `IntraserviceWrite` = `WriteOutcome` Done / Refused (4xx, no connection
— `HttpRequestError` connect/name/TLS/proxy) / Unknown (5xx, timeout, broken, a followed redirect at any status —
`Redirected`, the final path isn't ours, a trailing «/» is; and a 2xx `text/html` page — `IsPage`, `PageAsync`: the
rules shared with `DownloadFileAsync`), `Conflict` = 409; `RefusalAsync(what:)` phrases); Done clears the draft unless
it was edited meanwhile; Unknown → `Log` (number and reason, never the text); Done/Unknown/409 →
`_commentCache.Remove` + `LoadComments(force)`. Checked by `CommentCheck.cs` (area `comment`).

## Status in Intraservice on a hand move (0.25.0, stage 8 step 2)

Switch `AppSettings.AllowStatusWrite` + `StatusOnMove` (column → `StatusRef(Id, Name)`, null = «не менять»), chosen in
`SettingsWindow` («Запись в Интрасервис»: `SettingsViewModel.AllowStatus`, `ColumnStatuses` — a `ColumnStatusRow` per
column, `StatusOption`, filled from the server's directory by `LoadStatusesAsync` on the window's `Loaded` — only
while `AllowStatus` is on, or once it is switched on (`OnAllowStatusChanged`), one read per window (`StatusesRead`); a
saved status missing on the server stays under its name). Hand moves only: `MainViewModel.MoveByHand` (card menu
`MoveTo`, `MoveSelected`, `SelectedStatus`) and `OnDropped` (from ≠ to — a reorder is a Drop too) call `OfferStatus`;
auto-sync and F5 move through `MoveTicket` and never offer. Logic: `ViewModels/MainViewModel.Status.cs` —
`OfferStatusAsync` (`_offering` — one offer per (ticket, column); `_statusGate` — windows one at a time, held through
the write; `Gone` — moved on or deleted meanwhile → nothing); `GetTaskWithRightsAsync`
(`include=status,usertaskrights`, a 400 → the plain card, rights unknown) → already in the status → nothing; the
directory (`IsCommentRequired`) and the lifetime, read together; choices = the rights' `ToStatuses` (`ParseRights`: no
block or a null list = unknown → the whole directory; an empty list = none) minus the current; proposed = the column's
status if it is on the server under the same name and allowed, else a note; `StatusForm` (`ViewModels/StatusForm.cs`:
`CommentRequired`, `CanApply`, `KindText`; public by default) → `Views.StatusWindow.AskAsync`
(`Views/StatusWindow.xaml`; Enter is a new line, the button is a click); after it the card still in that column and on
the board, the switch and the account again — else not sent, said; `ChangeStatusAsync` (`.Write.cs`: `StatusId`, the
comment through `WithComment` only when not empty, `Changed` through `Body` — both shared with `CommentAsync`);
whatever the outcome — the card re-read (`Apply`), comments dropped from the cache; Done → `SyncMessage`;
Refused/Unknown → `AskWindow.Tell` (the panel may be closed after a drag), Unknown also to the log. Column titles:
`TicketStatusText.Of` (`Models/Ticket.cs`) — the board's columns, the converter and the settings. Checked by
`StatusCheck.cs` (area `status`).
