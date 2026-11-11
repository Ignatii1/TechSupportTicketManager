# Sync: import, F5, auto-sync, unread comments, reopened and reassigned tickets

`ViewModels/MainViewModel.Intraservice.cs`, `.AutoSync.cs`, `Services/AutoSyncRules.cs`. Checks: area `board`
(`BoardCheck.cs`).

## Import of my tickets

`ViewModels/MainViewModel.Intraservice.cs` (`ImportMine`, `FetchMyOpenAsync`) +
`HttpIntraserviceClient.GetCurrentUserAsync` / `GetStatusesAsync` / `GetExecutorTasksAsync`; paging and «is the list
complete» — `AutoSyncRules.ReadAllPagesAsync` (+ SelfCheck); entry points in `App.SetupTray` and the toolbar in
`MainWindow.xaml`; the closed-status names live in `AppSettings.ClosedStatusNames`

## Refresh all cards («Обновить статусы», F5)

`MainViewModel.Intraservice.cs` (`RefreshAll`) (per-card `GetTaskAsync`, 4 at a time) + `Apply` (the shared "what a
sync may overwrite" rule, also used by `SyncAsync`) + `ClosedNames` (shared with the import); entry points
`App.SetupTray` and `MainWindow.OnPreviewKeyDown`

## Background auto-sync (timer, new assignments, closed-ticket notifications)

`ViewModels/MainViewModel.AutoSync.cs` (`ApplyAutoSync`, `AutoSyncAsync` — `internal` and returns its
`AutoSyncOutcome` so that `TicketBoard.SelfCheck/BoardCheck.cs` runs whole passes, like import and F5, against a fake
server: a change here gets a step there) — reuses `FetchMyOpenAsync` / `NewCard` (import) and `Apply` / `ClosedToMove`
/ `AskMoveClosed` (F5) from `.Intraservice.cs`; what it never adds — `Services/AutoSyncRules.cs` (+ SelfCheck),
persisted in `AppSettings.AutoSyncSkipIds` (reset with the lifecycle flags in `MainViewModel.ApplySettings` when
`AppSettings.AutoSyncAccount` ≠ `AccountKey` — URL/login changed, also by hand between runs); notifications with click
actions — `MainViewModel.Notify` → `App.ShowTrayNotification`; closed-but-not-moved counter in the board title —
`ShowAutoSyncState`. It never moves a card into «Готово» by itself; the only moves it makes are a reopened «Готово»
card back to «Входящие» and a «Ждёт ответа» card to «В работе» when the requester replies (rule
`AutoSyncRules.RequesterReply` + SelfCheck: a new unread comment whose author's name equals the card's `Creator`;
applied at the end of `AutoSyncAsync`, only to tickets in this pass's open list — so never closed or reassigned ones,
and never on a stale card status; notification section `Answered`).

## What a sync overwrites on a ticket

`MainViewModel.Intraservice.cs` → `Apply` (used by `SyncAsync`, `RefreshAll` and auto-sync — every N minutes on listed
cards and rechecks): status, creator with phone/email, executors and executor group always (a field missing from the
response keeps the old value), title only if it's still the auto `Заявка #N`, description only if empty or equal to
`CutDescription` (what the pre-0.15.1 parser made of the same server text — repaired once, hand edits never match);
then `ApplyCard`: people, position, and — only from a real card (`Extra` not null; list rows have none) — company,
service, type, categories, assets, the files list (written only when it differs) and `CardReadAt` (set before the
fields, so the files handler doesn't read the card a second time). List rows go through `AsTask`.

## Reopened / reassigned-away tickets

Rule: `AutoSyncRules.Track` (+ SelfCheck) over the persisted `Ticket.AssignedToMe` (null = not seen yet → baseline, no
events). In `AutoSyncAsync`: listed cards in «Готово» that were not mine → moved to «Входящие» (`Lifecycle.Reopened`);
cards that left a complete list and whose recheck says open → «больше не на вас» (`Reassigned`, card stays; an
unresolved «статус N» decides nothing — `HttpIntraserviceClient.IsResolvedStatus`). Transitions are computed during
the pass and applied only at its end, right before the notification. A pass is dropped (`StillCurrent`) only when the
account changed or auto-sync was switched off mid-pass; a plain settings save no longer aborts it (it used to, and
lost notifications for statuses already written). An account change resets `AssignedToMe` (`AutoSyncAccount` vs
`AccountKey` in `MainViewModel.ApplySettings`). Notification sections: `NotifyChanges` / `AutoSyncNews`, list lines
via `Lines`.

## Unread comments (card badge, notification, «seen»)

Counting rule: `AutoSyncRules.Unread` (+ SelfCheck; server dates only, own comments by `EditorId`, else by name).
Auto-sync re-reads the lifetime only of cards whose `Changed` moved (`CheckCommentsAsync` in `.AutoSync.cs`; first
sight = baseline, no request). Persisted on `Ticket`: `CommentsCheckedFor`, `CommentsSeenAt`, `UnreadComments`; in
memory: `ServerChanged` (set by `Apply`). «Seen» = shown in the open panel of the active board window **and** a user
action (selection, panel, ⟳, activation, any click/key on the board): `MarkCommentsSeen` in `.Comments.cs`,
`IsBoardActive` + input hooks in `MainWindow`. Auto-sync never marks seen itself (`LoadComments(markSeen: false)`) and
always notifies. Baselines from `Changed` get +1 s (`AutoSyncRules.SeenFrom`). Badge: `UnreadBadge` style in the
`TicketCard` template.
