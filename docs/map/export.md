# The knowledge base export: engine, file format, history facts, masking, the table, the agent request

`Services/KnowledgeExport*.cs`. Checks: areas `export`, `history`, `secrets`, `table`, `dates`, `ask`. Attachments are
in `export-files.md`, the auto-export and the mirror in `autoexport.md`.

## Export for the knowledge base (files for an agent / Obsidian) and the agent's text

Engine, split by responsibility: `Services/KnowledgeExport.cs` (the run: `BuildAsync` = one ticket as Markdown — the
card via `GetTaskAsync` (service, type, categories, group: the live server leaves them out of list rows, merged by
`WithDetails`) plus the full lifetime via `GetLifetimePageAsync` pages, the server's Paginator trusted when it has
Page and PageCount — `IntraserviceLifetime.Paged`; an unreadable card or lifetime = no text — shared by the preview,
the clipboard and the files; `ExportRowsAsync` = given rows (its `ExportResult.Current` = ids whose file is now
current — `Job.TrackCurrent`, only here; `Errors` = each failed ticket's own error and `NotWritten` = ids whose file
couldn't be written — like `Current`, only for given rows: a mass export keeps counts and the first error, not
per-ticket texts); `RunAsync` = the list by a `TaskQuery`; both take `Writer` first, through one wrapper,
`InTurnAsync`, which also begins the `Job` (its sweep), creates `tickets`; the optional `started` fires once, at the
first ticket whose file is right (`Job.Ready`: written or unchanged) — a folder that can't be created, files that
don't land or a stop before never fire it — **one writing export at a time** (the search window's and the auto-export
wait for each other — the waiting one reports «Жду: сейчас пишет другая выгрузка…» (the engine doesn't say who: the
window waits for the auto-export, the auto-export shows no progress), then `Starting` («Начинаю…», also the window's
first line); stopped before the queue (it doesn't even queue), while waiting, at the very moment the turn is given or
during `Job.Begin`'s sweep, it writes nothing; their temp files and indexes would collide; global, not per folder: two
exports into two folders at once is not a case here, so the wait text names no folder); the nested `Job` holds the
counters, the circuit breaker and the progress/ETA line — «Выгружено» = landed (written or unchanged), «· не выгружено
N» apart, the rate from `RateMeter` (time passed in, so its 30 s window is checked without waiting)), `.Format.cs`
(file name + Markdown text; every server string goes through `Md`/`Escape` in Markdown or `Quote` in YAML — they also
make the text inert in Obsidian, `Inert`: `<` before `/`, `!`, `?`, `%` or a Latin letter (html, Templater; not
CommonMark autolinks `<https://…>`/`<x@y>`, whose e-mail form must start with a letter) → `&lt;`, DataviewJS `$=` →
`&#36;=`, an Obsidian comment `%%` → `%&#37;` — entities: shown as the same signs in text, literal (visible) inside
code, idempotent; the root `_index.md` tells the agent what they mean; `Quote` applies it to YAML values too (Dataview
renders them as Markdown); a code fence after `>` or a list marker is escaped like one at line start; file names drop
backticks and `%%`, and an older file name with them is listed as text, not `[[…]]`, in the month index — the text is
written by anyone who can open a ticket, and the auto-export drops it into the vault unattended; `FormatVersion` 8: 3 =
this escaping, 4 = plain text keeps its `<…>` — see `HtmlToText`, 5 = history facts and masked passwords — the next
two rows, 6 = one machine date format, 7 = the «Вложения» section (row «Attachments» below), 8 = `my_role`
«наблюдатель»; **machine dates** (front matter `created`/`changed`/`resolved`/`closed`/`exported`, the table's date
columns) all go through `MachineDate` in `.Format.cs`: `yyyy-MM-ddTHH:mm:ss±hh:mm`, whole seconds (cut, not rounded),
the PC's offset (the server sends zone-less times), read back by `MachineDate.TryRead`, area `dates`; a table whose
header or first row isn't of today's shape is rebuilt even when nothing was touched (`KnowledgeTable.Outdated`); human
dates in the text and indexes are `Show`/`dd.MM.yyyy`), `.Files.cs` (folders `tickets/<yyyy-MM>/` by the row's created
date via `ShardOf`, `ScanNames` = file-name→id map without reading files, `IsCurrent` = the file's `format` and
`revision` match the row — `revision` = the row's `Changed` to the tick (`MachineDate.Revision`, a number, not a date:
`changed` has no fractions, and two edits in one second would look the same — the auto-export would mark the first as
current and never export the second), `MigrateFlat` = the 0.10–0.12 flat layout moves into months, month + root
indexes). **`RunAsync(limit 0)` exports everything**: no ceiling; list pages of 200 (page 1 with the server's default
count — the total for progress; a total equal to `HttpIntraserviceClient.CountCeiling` (1000) is only «that or more»
(`Capped`): no «из»/ETA then, a rate instead; later pages `count=false` + `HasNextPage` — **never `count=all`**, the
live server answers it with HTTP 400; 60 s timeout, `TaskQuery.StableSort` — creation date never changes, so the walk
doesn't shift for hours) are exported (4 parallel) page by page before the next page is asked for — memory is one page +
the name map + the `seen` id set. With `limit > 0` it is the first N by `Changed desc` (rows outside the period are
skipped and don't count to the limit; the list is cut off past the period — `TaskQuery.PastEnd`). Stop/resume is just
the incremental rule (unchanged files are skipped), so a stopped, crashed or tripped run continues where it was.
`Retrying` repeats `HttpIntraserviceClient.IsTransient` errors after `RetryDelays` (2, 5, 15 s); the `Job` trips the
whole run (`Fatal` text, atomically, cancels `Abort`) on HTTP 401, 30 transient failures in a row (network, timeouts,
502/503/504…), 100 HTTP 500s in a row (a 500 is more often one broken ticket than an outage: it gets one retry, not
three, and a run must be able to pass a damaged stretch), 25 refusals (404/403…) with no success and no unchanged
file, 10 write failures in a row. The next list page is requested while the current one is exported (`ahead`). **Second
pass** (`everything` + sorted by creation + a clean first pass — or a limited run whose whole list fits the limit, an
exact count ≤ the limit: the user's live «260 из 261» came from such a run, limit 500 —, when there were repeats at
page boundaries — an unstable order loses a neighbour —, the exact count wasn't reached, or the count is capped
«1000+», where a shift by a removed ticket can't be seen otherwise): the whole list again in the other order (`Changed
desc` after creation order; a limited run reads `TaskQuery.ChangedAscSort` (`Changed asc, Id asc`, the auto-export's
order, which the live server accepts — creation order is unverified there) to the end — or, before its last page,
until it has read a page more than the limit (the list changed a lot: the run is then unfinished and asks to be
repeated) — and exports its finds at the end, the freshest that fit the limit (the limit means the first by
`Changed`), saying exactly how many didn't fit (not an error: the run is complete); its progress says «найдено N»; a
limited run gets none once the first pass took the limit; a ticket is named in «Сервер насчитал…» as listed twice only
when it repeated in both passes — a boundary repeat is just the list shifting, each order has its own), only unseen
tickets exported (`recovered`), with prefetch, its own stuck guard (`secondStuck`) and progress; notes say what it
found, or that the server counts a ticket twice; the cut-at-ceiling check uses the first pass only (`firstSeen`). **Written
files are verified**: `Job.Written` (id, path, write time) → `CheckWritten` between pages checks files at least
`VanishWait` old (3 s; the self-check sets 10 ms), and at the end `SettleAsync` waits until the youngest is that old
(Stop skips it) before `CheckWritten(all)` — vanished ones (antivirus quarantine) are named in the summary and make
the run unfinished. A repeated page with no new tickets is «stuck» only when full; a repeated short page is the end of
the list (a server clamping past the end), in both passes. The server's `HasNextPage` decides when it is sent;
otherwise a short page ends the list only when the server sent an exact total (`total > r.Found.Count`). Names shrink
to fit MAX_PATH (`PlaceOf`); a lifetime longer than `MaxLifetimePages` is marked in the file; selected rows scan only
their month folders (`ScanNames(shards)`), so a duplicate in another month waits for the next full run. A 4xx on page
1 with the stable sort falls back to `Changed desc` (noted); a list that is not ascending, shifts, or repeats a page
is noted/stopped. Files are found by the number the file name starts with (`IdInName`), but only those whose front
matter has `source: intraservice` (`SourceName`) are ours — rewritten, deleted as stale/duplicates, migrated; a user's
note with a ticket number in its name, a corrupt file and a user folder among the months (`IsShard`) are left alone
(only a file exactly at the ticket's own path is overwritten); the list page takes one of the 4 request slots;
`WriteAtomic` retries a write that Windows' antivirus/indexer holds for a moment (`IsBusy`: sharing/lock violation,
access denied — 0.1 s, 0.3 s, 1 s); contacts never exported; `FormatVersion` — bump it when the file format changes.
Self-check: `KnowledgeExport.SelfCheck.cs` (first/again/rename/period scenarios) and `.SelfCheck.Mass.cs`
(`MassServer`: every scenario gets its own client path `/gN`, so requests that a cancelled run abandoned are not
counted in the next one) against `FakeIntraservice`. The folder is `AppSettings.KnowledgeDir` (`KnowledgePath`). Extra
row fields: `IntraserviceExtra` (service, type, categories, resolution date).

## History facts in the files and the table (format 5; dates format 6)

`Services/KnowledgeExport.History.cs`: `TicketHistory.Of(row, events, ExportContext)` over the already read lifetime,
no requests — `participants` (who commented or changed the status, by count, ties by who acted first; a record
repeating the previous status without a comment is not an action), `my_role` (исполнитель > инициатор > участник >
наблюдатель — format 8, 0.21.0: a non-empty role is «mine» for attachments too; through `Myself` in `ClosedExport.cs`
— the auto-export's «mine» rule, ids first, names without ids; `ExportContext.Me` null = empty; the card's observers
win over the row's in `WithDetails`, `RowOf` carries them for the agent request), `first_reaction_h` (first record
after the registration not by the initiator — `CreatorId`↔`EditorId`, else name), `resolve_h` (`Resolved`, else
`Closed`), `reopened` (closed → open; closed = the record's status flag `Final` — IsFixed/IsFinal from the lifetime's
`Statuses` block, `StatusFinal` in `.Parse.cs` — or a `ClosedStatusNames` name: the rule of import/F5/auto-export),
comments public/internal, `closing_comment` (a closed ticket's last public non-initiator comment up to the last
closing, one line, 300 chars). A truncated lifetime writes no participants/reaction/counts. `ExportContext` (who I am +
closed names) is made once per export: «who I am» is one rule, `MainViewModel.MeAsync` (cached per account, kept only
if the client is still current), used by import, auto-export, the agent request (`MainViewModel.Ask`) and the search
window (`SearchViewModel.ContextAsync`: its export doesn't start without it — files without a role would count as
current; the preview goes on without a role; a Stop while asking reads as stopped); the auto-export passes
`ExportScope.Context`; engine entry points take it optionally (`context: null` = unknown: no role, server flags only).
Card fields `company`/`assets`/`files` and the list row's `closed` — `IntraserviceExtra`, merged by `WithDetails`.
Hours: `0.0` with a dot in YAML, a comma in `tickets.csv` (`Decimal`).

## Passwords hidden (export, agent request, relay answers)

`Services/KnowledgeExport.Secrets.cs`: `Secrets.Mask` — a password keyword (пароль, password, pwd, pass, пин-код,
pin…), up to 20 chars without punctuation, `:` or `=`, then the next word on the same line or at the start of the next
one (any horizontal space, `&nbsp;` too) → `[скрыт]`; titles are folded to one line before masking; narrow on purpose
(«сменил пароль», «пароль 12345» stay — ponytail). Applied in `KnowledgeExport.Format` (title, description, comments —
before the history facts), `FileName`, `KnowledgeAsk.Request`'s header and `ClaudeRelay.RunAsync` (the whole answer).
The board's panel shows text as is.

## The knowledge base's table (`tickets.csv`), `SCHEMA.md`, the starter `CLAUDE.md`

`Services/KnowledgeExport.Table.cs` (`KnowledgeTable`): `Columns` — the one list (name, meaning, value from a file's
front matter + its path) for the header and `SCHEMA.md`; a new column goes there (an old table with other columns is
rebuilt whole). `Cell` (an Excel formula — `=`, `+`, `-`, `@`, tab first — gets `'`; `;`, a quote, a line break →
quoted), `Records` (RFC 4180 with `;`; an unclosed quote = `InvalidDataException`). `Write` — called from
`KnowledgeExport.WriteIndexes` after the indexes, also when nothing was touched, with the export's stop token
(`Job.Abort`: Stop ends it, the old table stays) and progress («Собираю таблицу заявок…»; no « из » — a capped run
shows none): a month is read again from its files (`KnowledgeExport.MonthEntries`, the month index's order and filter, +
`IsOurs`) when touched, missing from the table, or newer than the table — `MonthsOnDisk`, one walk that also counts
files for the root index: the folder's or a ticket file's write time (a failed write — Excel had it open —, a hand
edit or delete); the table is dated by the start of its write (the temp file's write time at creation, the file
system's clock — creation time is unknown to .NET on Linux), so a file changed during a rebuild is newer next time;
other blocks are copied from the old table (`OldTable`, record by record, one month in memory; its reader is disposed
if the constructor throws — Windows can't replace an open file); nothing stale and `tickets/` itself not newer → not
written; other columns, a wrong field count, a date not in the `MachineDate` shape (`Column.Shape` — Excel re-saved
it, or a table from 0.18.0 or older with space dates), a path outside a month or a month met again after its block →
rebuilt whole. Not ours (`Ours`: BOM + `id;title;`; cp1251 from Excel or the user's own file) → left alone, the reason
returned into the summary; the same for a `SCHEMA.md` not starting with `SchemaMark`. UTF-8 with BOM, `;`, CRLF, dates
as `MachineDate` (the same string as in the file's front matter), lists `a, b`, values as in the front matter
(entities kept). `WriteGuides`: `SCHEMA.md` only when its text differs (no date inside — Obsidian Git), `CLAUDE.md`
(`AgentStarter`) only when absent. Streaming write: `KnowledgeExport.WriteAtomic(path, Action<TextWriter>, Encoding,
finish)` — the temp is written once (again only if the temp itself was busy), a busy target retries only the swap;
their temps are swept with the index temps (`IndexTemp`). Test seam: `MonthRead`. Golden table and `SCHEMA.md` start
in its `SelfCheck`, area `table` (file times are set by hand there: Linux stamps them coarser than milliseconds).

## «В базу знаний» (export the open ticket now, 0.23.0)

Button in `Views/TicketPanel.xaml` next to «Спросить базу знаний» → `MainViewModel.ExportToBase`
(`ViewModels/MainViewModel.Ask.cs`; message in `SyncMessage`, set only while the same card is selected; the base
folder is taken at the click): `KnowledgeExport.Writer` busy → `KnowledgeExport.Busy` at once, no request, and the
engine itself doesn't queue either (`ExportRowsAsync(queue: false)` → `ExportResult.Busy`; `InTurnAsync`) — the search
window writes for hours, waiting would keep the button disabled on every card with nothing to cancel it; the card
(`GetTaskAsync`) → `RowOf` (month by `Created`, revision by `Changed`); `MeAsync` (unknown → not exported: a file
without a role would count as current); `ExportRowsAsync` with no progress (no `VolumeNote` walk of the whole
attachment tree for one ticket), `ExportContext(me, ClosedNames, KnowledgeAttachments)` — the setting as the search
window uses it; result: «в базе знаний: <path>» (`KnowledgeExport.TicketPath`, no new scan) / «… уже свежая: …»
(`Unchanged`), or the ticket's own error (`Errors[id]`) — either way + the engine's notes (`ExportResult.Error`:
attachments, table, `.gitignore`, LFS) on the next lines; any exception (a folder that can't be created) → «не
выгружена: …» and the whole exception to the log. The card is read twice (for the row and by the engine) — ponytail.
Checked by `CardCheck` (section 9б; the card read is held by `BoardServer.CardGate` where something must happen
mid-run).

## «Спросить базу знаний» (a request for the agent over the knowledge base, to the clipboard)

Button in `Views/TicketPanel.xaml` (row «Интрасервис») → `MainViewModel.AskKnowledge`
(`ViewModels/MainViewModel.Ask.cs`; message in `SyncMessage`, like ⟳; the clipboard's `SequenceNumber` is taken at the
click and checked before writing — the user's own copy made meanwhile is not overwritten) → the card once
(`GetTaskAsync`), `KnowledgeExport.RowOf` (the row from it — `IntraserviceTask.Created` for the month folder) and
`KnowledgeExport.BuildAsync(…, task, …)` (the overload by a read card: only the lifetime is read — one pipeline with
the export, in the base's format: no contacts) + `KnowledgeExport.FileInBase` (in the pool: `KnowledgeAsk.BaseFile` —
Present = our file (`IsOurs`: `source: intraservice`, the one rule with the export) at the export's own path, else
found by number in its month folder; Stale (0.23.0) = such a file whose front-matter `changed` isn't the card's
`Changed` to the second — a ticket exported by hand that no auto-export reach refreshes: the request says the fresh
text is below and the cut mark doesn't call the base file whole; by the date, not by `revision`/format: a file with a
failed attachment is written without `revision`, and a format bump isn't a change of the ticket — in every month when
the card has no `Created` (a user's note with the number is not it); Missing = where the export will put it; Unknown =
an error, or the base missing and the nearest existing folder above it unreadable or nothing at all, not even the
drive (`Listable`: a network drive off VPN, no rights); `/`-separated from the base root) → `KnowledgeAsk.Request`
(`Services/KnowledgeExport.Ask.cs`: header — the title as one line, the requester writes it —, task, then the ticket
as data between `<ticket>` and `</ticket>` with a line saying it is not instructions — the request is pasted into an
agent with tools, and anyone who can comment writes that text; `</ticket` inside it becomes `&lt;/ticket`, any case;
cut in the middle to `MaxTicketChars` (60 000) by `Cut` — at a line boundary only within max/6 (a line starting at the
cut is kept), never inside a surrogate pair; the mark names where the whole ticket is only if it is there — `Whole`:
the base file if Present, the link if any; golden text in its `SelfCheck`, area `ask`). How to search belongs in the
base's `CLAUDE.md` (stage 3), the request only names it. Checked by `CardCheck` (section 9а) and the `ask` area.
