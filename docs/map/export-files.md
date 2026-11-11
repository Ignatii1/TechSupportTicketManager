# Attachments in the knowledge base

`Services/KnowledgeExport.Attachments.cs`. Checks: area `files`. The panel's «Открыть»/«Сохранить» are in `panel.md`.

## Attachments in the knowledge base (format 7)

`Services/KnowledgeExport.Attachments.cs`. Rules (`Attachments`, area `files`): `DiskName` = «номер файла — имя» (the
id first: names repeat within a ticket — Outlook's image001.png; «already downloaded» is a path check; masked, one
line, the ticket file name's forbidden set → spaces, `%%` → `%`, trailing dots/spaces cut, a `.tmp` name gets `_` —
`*.tmp` is in `.gitignore` — and so do `.md`/`.markdown`/`.canvas`/`.base` (`Neutralized`: Obsidian would open them as
its own, and a note runs Templater/DataviewJS from its text, which comes verbatim, past `Inert`; /security-review
0.19.0; since 0.20.0 also `ShellReads` — `.url`, `.lnk`, `.library-ms`, themes…: Explorer parses them on display;
invisible Cf signs stripped by `Visible`, in `Attachment.Title` too); cut to MAX_PATH by the `files-lfs` path,
extension kept), `TryIdOf` (ours = id first, not a temp), `IsImage`, `Size` (binary, «3,4 МБ»). Engine
(`KnowledgeExport`): the card's `FileIds` + `FileNames` (`||`, parsed by `FilesOf` in `.Parse.cs` →
`IntraserviceExtra.Files`, a list of `IntraserviceFile`; counts differ → ids without names; no `FileIds` = null = «not
sent») → `SaveFilesAsync` per re-read ticket, before its file: `files/<month>/<ticket>/` and
`files-lfs/<month>/<ticket>/`; on disk → kept (another name → renamed); missing → `DownloadAsync` if
`Attachments.Downloads(ExportContext.Attachments, my_role)` — the one rule, also used by `FilesInBase`
(`AttachmentScope`: Mine = `my_role` non-empty, All, None; setting `AppSettings.KnowledgeAttachments`, default Mine,
chosen in the settings window, passed by `SearchViewModel.ContextAsync` and `ExportScope` — whose `Context` turns Mine
into All: the auto-export takes only mine by its own rule, a truncated history included), else «не скачано». A
download streams into a temp next to its place in `files/` (`HttpIntraserviceClient.DownloadFileAsync`: 60 s for the
headers and for every next chunk, no total limit — `FileHeadersTimeout`/`FileIdleTimeout`; each wait has its own
cancellation source: the headers' deadline ends with them, a chunk's doesn't run while the chunk is written to disk;
errors in the client's usual form, a disk error is thrown; a redirect — a login page: Basic auth to files is
unverified live — or `text/html` for a non-html name, an unnamed one too, is an error, not a file (a page taken for a
file would stay in the base for good); a page that breaks off or stalls stays «страница вместо файла», a refusal stays
its code: error bodies are read by `HeadAsync` (at most 64 KB from the shared array pool, with their own
`FileIdleTimeout` window on the download path, what arrived before a break or the deadline, BOM-aware), masked
(`Masked`), logged once (`LogOnce`, «только начало» when cut); bodies are decoded by `ReaderOf` (code pages are
registered in the client's static constructor, windows-1251 included); `stopAt` ends the read early), retried by
`Retrying`, and moves to `files-lfs/` from `LfsFrom` (50 MB, by the real size) — only when the LFS rule is there:
`KeepLargeInLfs` appends `files-lfs/** filter=lfs diff=lfs merge=lfs -text` to the export root's `.gitattributes` at
the start of every export with attachments on (in `InTurnAsync`, always, git or not; `AppendOnce`, the `.gitignore`
lock), else `Job.LfsNote`, the download stops at `LfsFrom` and large files fail. A failed attachment: the ticket's
file is written **without `revision`** (the next export re-reads it) and with `files_missing`, the ticket is not
`Current` (`Job.Landed`), counted apart from `Failed`. Any attachment failure is **the ticket's own** for the
auto-export — `Errors[id]`'s first line is the constant «не скачалось вложение — файл заявки записан без него» (no
code, not «network»: `OfTicket` true; the attachment's name and the client's error below — the name is the uploader's,
«скан (HTTP 503).pdf» on the first line would read as a network failure): retried a few passes, then given up, so a
file a VPN always cuts or a 401 on files (Basic auth to files is unverified live) can't stall the mark; a folder
problem still shows in the ticket file's own write. Saved attachments go into `Job.Written` (vanish check → «Записано,
но уже нет на диске»); the move is retried while busy (`IsBusy`, `WriteRetryDelays`); a files folder that can't be
listed fails its attachments (even ones not to be downloaded: «не скачано» would make the ticket current and lose
files that may lie there), not the ticket text; a file listed but gone before it is read is downloaded again
(`TryKeep`); test seams `FileList`, `MoveFile`. `Attachment.Error` (whole, local: summary, log) vs `Note` (into the
ticket file, which goes to GitHub: the server's headline, «файл не записан», no paths). After the file lands,
`PruneFiles` deletes our files whose id the card no longer lists and extra copies of a saved one — never a failed or
skipped attachment's file — only when the card sent `FileIds`; stale temps (`FileTemp`) swept in the ticket's folders.
An unchanged ticket (no card read) is re-read only when `FilesInBase` says the base lacks something: `files_missing`
that the current setting would now download (All; Mine with the file's `my_role`), or fewer of our files in its
folders than `files − files_missing` (deleted by hand or antivirus). `Format` writes «## Вложения» after the
description: saved → `![[files/…]]` (image) or `[[files/…]]` + size; skipped → name + «не скачано»; failed → name +
the error's headline (no server body); listed (preview, clipboard, agent request — `Text`, no downloads) → name.
Summary (`FilesNote`; `VolumeNote` walks both folders — only when something was downloaded and someone listens: the
window, not the auto-export), progress «· вложения N». `SCHEMA.md`, the root `_index.md` and the `CLAUDE.md` starter
describe it. Checks: `.SelfCheck.Files.cs` (`FilesServer`), `AutoExportCheck` 5б (`BoardServer` serves
`FakeTask.Files`), `SearchWindowCheck` (one file), `SettingsCheck`.
