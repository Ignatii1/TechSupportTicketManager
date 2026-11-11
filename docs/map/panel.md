# The detail panel: comments, people, card fields, attachments

`Views/TicketPanel.xaml`, `ViewModels/MainViewModel.Comments.cs`, `.Attachments.cs`. Checks: areas `card`, `panel`.

## Comments («Переписка») in the panel

`ViewModels/MainViewModel.Comments.cs` (`LoadComments` / `ToRows`) + `HttpIntraserviceClient.GetLifetimeAsync`; row
template `CommentItem` in `Views/TicketPanel.xaml`, styles `CommentRow`/`Chip`/`IconToggle`. Never persisted: in
memory + a 2-minute cache. The open panel is re-read whenever its ticket's history is re-read or dropped — an
auto-sync pass (from the cache it just wrote, only when the rows differ — `SequenceEqual`, so scrolling survives a
field edit; never «seen»; own comments too: they are never unread), F5 (`RefreshAll`, forced, only for a card it read)
and the card's ⟳ (`RefreshFromIntraservice`, forced, only when the card was read — `SyncAsync` returns it; not from
`SyncAsync` itself: adding a card calls it right after the selection started its own load); the open card goes first
in a pass's 10 lifetime reads.

## Creator / executors in the panel

Parsed in `.Parse.cs` (`Field`, `Names`), stored on `Ticket` (`Creator`, `CreatorPhone`, `CreatorEmail` →
`CreatorContacts`, `Executors`, `ExecutorGroup`), rows 5–6 of the panel grid in `Views/TicketPanel.xaml`
(`SelectableBody`/`SelectableText`), position and company under the creator (`CreatorRole`). The open card is re-read
quietly (next row), so old cards get people and fields on first open. Also in the relay's `TB ticket` answer
(`ClaudeRelay.People`).

## Card fields 2.3 and the panel's attachments (0.20.0)

Fields on `Ticket` (`CreatorPosition`, `CreatorCompany`, `Service`, `TaskType`, `Categories`, `Assets`, `Files` =
`TicketFile(Id, Name)`; computed `CreatorRole`, `ServiceText`, `CardContext` — the card's caption line —,
`FilesCount`/`HasFiles` — the paperclip), written by `ApplyCard`; `CreatorPosition` parsed in `.Parse.cs`. Panel rows
7–9 (Сервис, Категории, Оборудование) and the «Вложения» section (`AttachmentRow` template) in
`Views/TicketPanel.xaml`; the caption and paperclip in the `TicketCard` template. Logic:
`ViewModels/MainViewModel.Attachments.cs` — `LookupPanel` (async void, cancelled by the next selection: a 400 ms
pause, then `ReadCardQuietlyAsync` unless `CardReadAt` is fresher than `CardFresh`, 2 min; status untouched; then
`ProbeSizesAsync`: `DownloadFileAsync(stopAt: 1, rangeTo: 0)`, `SizeAsked` set only after the answer),
`RereadOpenCard` (auto-sync, when the open card's `Changed` moved; with the panel closed it only drops `CardReadAt`,
and opening the panel runs `LookupPanel` — `OnIsPanelOpenChanged`; a failed read in `ReadCardQuietlyAsync` or
`SyncAsync` puts the previous `CardReadAt` back), `ForgetPanelLookups` (only when `AccountKey` changed; busy rows
kept), `OpenAttachment` (`FetchAsync` → `FilesFolder` = `%TEMP%\TicketBoard\files\<8 hex of SHA-256 of the base
URL>\<ticket>\` + `Attachments.CachePath` = «id — SaveName», cut by MAX_PATH, `ShellReads` neutralized again after the
cut (a cut name can end otherwise than the original); read-only after download; `Attachments.Runs` of the original or
of the cached name, or `FileShell.IsDangerous` → shown in Explorer, never run; else `FileShell.Open`, a warning when
the mark didn't stick), `SaveAttachment` (`FileShell.TryAskSavePath(SaveName)`, a copy in `Task.Run` or
`DownloadToAsync` there, read-only cleared, the mark written). `DownloadToAsync`: a temp next to the target,
`KnowledgeExport.Retrying` (network/5xx), the mark on the temp, `KnowledgeExport.MoveWhenFreeAsync` (busy retries; the
`MoveFile` seam). Rules in `Services/KnowledgeExport.Attachments.cs`: `Runnable` (Outlook's level-1 list in full +
extras), `ShellReads` (`.url`, `.lnk`, `.library-ms`, themes… → «_» in `SaveName` and `DiskName`: Explorer parses them
on display — NTLM leak), `WindowsName`, `Visible` (Unicode Cf stripped — RLO), `MarkFromInternet`/`IsMarked`
(Zone.Identifier, Windows only). Windows calls: `Services/FileShell.cs` (ShellExecute, `explorer /select`,
`SaveFileDialog`, `AssocIsDangerous`). Checked by `PanelCheck.cs` (area `panel`) and the rules in
`Attachments.SelfCheck` (area `files`); the client's Range/416 in `KnowledgeExport.SelfCheck.Files.cs` (area
`export`).
