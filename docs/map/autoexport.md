# Auto-export of closed tickets and the mirror

`Services/ClosedExport.cs`, `ViewModels/MainViewModel.AutoExport.cs`. Checks: areas `autoexport`, `mirror`.

## Auto-export of closed tickets to the knowledge base

**Reach (0.22.0)**: `ExportReach` Mine / Closed / All — `AppSettings.AutoExportReach`, «Какие» in `SettingsWindow`
(`SettingsViewModel.ReachChoices`, the note per choice); Closed and All are the **mirror**: every row is taken
(`ExportScope.TakesByRow` — no history reads), All lists without `StatusIds` (and asks no status directory),
attachments only mine whatever the setting says but None (`ExportScope.Context`; Mine reach turns Mine into All as
before), page = 500 + the mark minute's ids in every reach (`ClosedExport.PageSize`, `PageRows`), the mirror rests
`ClosedExport.MirrorEvery` (59 min from the pass's **start** — the timer's ticks drift by seconds, a full hour from
the end would skip the next tick; the check sets its own) only after a pass that caught up without errors — cut by the
portion or failed = the next auto-sync tick, like the mine reach — (`ClosedExport.Due` over
`MainViewModel._exportRest`, in memory, with the reach it was computed for), the mark remembers its reach
(`AppSettings.AutoExportMarkReach`, null = Mine as 0.21 wrote; `ClosedExport.MarkServes`, an explicit switch, not the
enum's order: a wider reach forgets the mark — a week back —, a narrower keeps it; `KeepMark` writes the pass's own
reach), the tray line shows the pass time from 1 s minus the wait for the writer turn (`ClosedExport.Took` — seconds,
then `HumanSpan`; `ExportResult.Waited` from `InTurnAsync`, `ClosedExportPass.Waited`). **Rows whose file is already
current cost nothing**, in every reach (`KnowledgeExport.Unchanged` — a lazy predicate: a month folder is listed by
name once, when the pass reaches its first row, a row's file read only then, the pass's token, any exception but
cancellation = «not current»; the export's own skip rule, `OursOf` in `.Files.cs`, shared with `ExportCoreAsync`;
read-only, outside the writer's turn — the export scans again in its turn; whose attachments must be there comes from
`ExportScope.CurrentFor`: a row taken by its row — the context's, as the export would; the mine reach's other rows —
by the role in their file (Mine): a stranger's file with «не скачано» is current, no history read; `Verdict.Current` =
done for the mark). The mirror never deletes files. Checked by `TicketBoard.SelfCheck/MirrorCheck.cs` (area `mirror`,
its own `BoardServer`, which lists every status when `StatusIds` is absent). The mine reach:
`Services/ClosedExport.cs` — `ExportMark` (the mark: `Changed` of the last ticket done in order + ids of its minute,
since the server compares dates to the minute; it never moves back; rows changed before its minute count as done —
`Covers` — a correct server wouldn't send them, a time-zone-skewed one does; first enable = a week ago), `ExportScope`
(query: closed `StatusIds` — `IntraserviceStatus.ClosedBy`, the rule shared with import, F5 and search —,
`ChangedMoreThan`, `archive=true&inactive=true` (otherwise the server silently leaves out archived/inactive services),
`sort=Changed asc, Id asc`; mine = executor/initiator/observer by row ids (`ObserverIds` since 0.21.0 — live rows
carry it), by names when the row has no ids, else any my action in the history — `EditorId`, else name), `RunAsync`
(**one list request per pass** — a snapshot: reading on, a ticket changed between pages would shift a row past us;
page = 500 + the mark minute's ids, up to 2000 (API max); a full page of covered rows = error «за раз», the ceiling;
order checked before anything is taken — by `Changed` only, ties in any id order; at most `Budget` (30) tickets
needing a request; known not-mine with the same `Changed` and rows with a current file cost nothing; a history that
fails for the ticket's own reason (`OfTicket`: 4xx but 401/408/429, 500, unparsed, a file that vanished after writing)
is skipped over, a network/login failure ends the pass; a file that couldn't be written (`ExportResult.NotWritten`)
counts as the ticket's only when its temp file was written and the final move failed — something sits at the file's
own path (`PathBlocked`, `KnowledgeExport.PathBlockedException` from `WriteAtomic`, whose temp name is new on every
attempt, `X.md.1a2b.tmp`, so a stray `X.md.tmp` can't fake a folder problem — and so `SweepTemps` removes our temps
(`TicketTemp`/`IndexTemp`: a ticket number or `_index` before `.md.xxxx.tmp` — a bare `X.md.tmp` may be an editor's,
it stays) older than 10 minutes, on every export job (jobs run only when there is something to write — or, while a
pass is held by a broken folder, every pass, touching the same folder its export touches anyway): the export root
(index temps only) and `tickets/` when the job starts (`Job.Begin` — inside the writer's turn, never throwing; a bad
limit is refused before the queue), the month folders as batches reach them (`Job.SweepMonth`); a month whose index is
rewritten without rows there keeps such a leftover until an export reaches it); a temp file that can't be written is
the folder's (disk full) and the pass just waits; mine exported by `KnowledgeExport.ExportRowsAsync`, whose
`ExportResult.Current` = ids whose file is now current and `Errors` = each failed ticket's own error; the mark moves
only past tickets done in a row), `ExportFailures` (a ticket failing for its own reason `GiveUpTimes` (5) passes, the
first `GiveUpAfter` (1 h) ago, on the same `Changed` is skipped; it stays counted until the mark passes it, and only
then is it in `GivenUp` (logged once); in memory). Why `Changed` and not `Closed`: the live card had no
`Closed`/`CreatorId` (docs/HISTORY.md, 2026-10-10). Wiring: `ViewModels/MainViewModel.AutoExport.cs`
(`AutoExportAsync` after each auto-sync pass on its timer, while `ExportCanRun` (on, auto-sync > 0, a client — one
property for the guard, `StillExporting` and `ApplySettings`); a pass first keeps the start mark on a first run
(`KeepMark` — a week back from enabling, not from the first good pass) and creates the folder in the pool — one that
can't be made (a network drive off VPN) = «не удалось» before any request; the export itself runs in `Task.Run` — it
writes files and month indexes synchronously; `ClosedExport.RunAsync` stops when the whole page is before the mark
minute and is full or reaches further back than any time-zone skew, 2 days (`BeforeMark`: the server ignored
`ChangedMoreThan`; a short page within the skew is just skipped), and counts a truncated history as mine; any
exception → «не удалось» + log, never a dialog; `StillExporting` = the pass's own stop token not cancelled, same
account and folder, still on — `ForgetExport` cancels `_exportStop` (`StopExportPass`, also called by `ApplySettings`
whenever export can't run — off, auto-sync 0, no client), the token the pass runs with (each pass makes its own), so a
pass waiting for the writer behind a long search-window export gives up instead of writing later — and stays given up
even if export is switched back on before it returns; a pass waiting behind the search window when that window changes
the folder still writes into the folder it started with (correct files there; its mark isn't kept, the next pass
starts over in the new folder); mark in `AppSettings.AutoExportSince`/`AutoExportSeen` + the folder setting it belongs
to, `AutoExportDir` (the setting as typed, `""` = `knowledge` next to the exe, so moving the app keeps it) — another
folder = `ForgetExport` (a week back, into the new one); the search window's export changes the same setting only when
the export actually runs — after the 2000+ question and once it has its writer turn (`SearchViewModel.WriteAsync`
calls `RememberFolder` on the window's thread once the engine's `started` has fired — judged by `IsCompleted`, not by
which task finished first; stopped while waiting, a folder that can't be created or where no file lands = setting
untouched; `RememberFolder` never throws — a settings.json that can't be saved (any exception, DPAPI included) is a
line of the summary; `Summary` names failed tickets without guessing cause or cure («Не выгружены: N — повторная
выгрузка попробует их снова» + the first error: an unread ticket, a refusal, a locked or unwritable file all land
there, and a retry fixes some of them, not all); the default folder is stored as `""` however typed) — and then says
so when auto-export runs and its mark belongs to another folder; folders compare by `AppSettings.SameFolder` — full
paths, `""` = `knowledge` next to the exe, no trailing separator; `ForgetExport` — on an account change and when the
toggle is off, called from `MainViewModel.ApplySettings`, which also clears `AutoExportState` (→ tray tooltip in
`App.UpdateTrayIcon`, clamped to 127 chars) when export can't run; `*.tmp` appended to the export folder's own
`.gitignore` when it or a parent holds `.git`, a folder or a file — the rule is `KnowledgeExport.IgnoreTempsInGit`,
without memory (a deleted `.gitignore` is rewritten next pass), applied with the folder at the start of a pass (in the
pool) and by the engine before the first file of any export, the search window's too, under one lock (both may run at
once); the auto-export's failure is logged once per run per distinct text (it names the path) and retried silently, an
export's goes into its summary (`Job.GitNote`)), toggle `AutoExportClosed` in `SettingsWindow`. Checked by
`TicketBoard.SelfCheck/AutoExportCheck.cs`.
