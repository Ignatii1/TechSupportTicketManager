# The Intraservice client: requests, errors, JSON, what the live server really does

`Services/HttpIntraserviceClient*.cs`. Checks: area `client` (samples in `.SelfCheck.cs`); the Rust copy is
`rust/src/intraservice/` — a parsing change gets its sample on both sides.

## Intraservice HTTP calls, error messages

`Services/HttpIntraserviceClient.cs` `GetTaskAsync`, `CheckAsync`, `GetAsync` (status code → Russian message). A
response that doesn't parse goes through `Unparsed(json)`, which writes the method name and the first 4000 chars of
the body to `errors.log` via `LogUnparsed` — route any new parse failure through it.

## Intraservice JSON field names

**only** `Services/HttpIntraserviceClient.Parse.cs`; a sample for every shape in
`HttpIntraserviceClient.SelfCheck.cs`, run by `TicketBoard.SelfCheck`

## Notes and gotchas

- API calls share one `HttpClient` (no client-wide timeout: each request has its own, 10 s, `GetAsync(timeout:)` — a
  list page of the export gets 60 s; a write 30 s, `WriteTimeout`) and return errors as strings, never exceptions. **A
  write is never retried** (`PutAsync` in `.Write.cs`, its own client without a connection pool — .NET itself replays
  a request that failed on a pooled connection): a read repeated is harmless, a comment repeated is two comments — an
  unanswered or 5xx write is `WriteOutcome.Unknown`, and the user decides after the conversation is re-read. Every
  body is decoded by `ReaderOf` (the BOM if there is one, else the header's charset if .NET reads it — `TryEncoding`:
  utf-7 it knows but refuses —, else UTF-8; not `ReadAsStringAsync`, which throws on an unknown charset); an error
  body is read only up to 64 KB (`HeadAsync`, after `ResponseHeadersRead`). **An error can be multi-line**: a short
  Russian phrase (+ `HTTP <code>`), then the evidence — the server's body via `Evidence` (the user wants raw
  responses) or the exception chain via `Reason`. `Redact` masks credentials; keep it on every new path. One-line
  spots use `Brief`; multi-line spots use the `SelectableText` style so the text can be copied.

- The API uses Basic auth only (no tokens). Warn on `http://` (that already exists); don't remove the DPAPI
  encryption.

- Response shapes are verified against the live server only where `PROGRESS.md` says so; the parsers tolerate
  several.

- **The live server is an ASP.NET Core build that validates parameter types**, not quite the PDF: `count=all`
  (documented) is rejected with `HTTP 400 {"errors":{"count":["The value 'all' is not valid."]}…}` — `count` is a bool
  there (2026-10-08). `FakeIntraservice` refuses it the same way for every scenario (`Refused`); add any other
  rejection seen live there too.

- **A filter value the live server can't read is dropped silently** — no 400: `StatusIds=<Id>` or
  `ChangedMoreThan=<a date in brackets>` returned the whole list from 2015 (2026-10-12). So a wrong date format shows
  only as rows outside the period — that is what `BeforeMark` (auto-export) and `TaskQuery.Outside`/`PastEnd` (search,
  export) are for. Live list rows carry `CreatorId`, `ExecutorIds` («a, b, c»), `Closed`, `ServiceId` (no name — the
  card has it), `Type`, zone-less dates with microseconds; `count` stops at 1000 and `HasNextPage` is null while
  counting; `pagesize=500` is honoured; `ChangedMoreThan` takes `yyyy-MM-dd HH:mm` and seconds too (compared to the
  second). Descriptions and comments come as plain text (`\r\n`): `HtmlToText` treats text as html only when it
  carries an element of a known tag (opening + the same closing, the closing on one line), a `<br>` or an `<img …>`
  (`IsHtml`, linear: the last closing of each name, then an opening of it before that — not a pair regex from each
  opening, quadratic on codes with many `<b>` and `</i>`; comments (`WithoutComments`) and style/script/head/title/pre
  blocks (`Blocks`) are linear too, and patterns that begin with whitespace start only at the first space of a run
  (`(?<!\s)`, else a long run is rescanned from every space); a lone «tag» turns up in plain text — marking codes
  `<p/7z>`, `<b>`, `<TTCx>`, mail `<i.ivanov@corp.ru>`, a mentioned `<div class=…>` or a quoted `</td>`); a tag ends
  with `>`, `/>` or attributes after a space (`TagEnd`), so `<p/7z>` is never a tag. Html is shown as a browser would
  (`HtmlText`; `Evidence` adds an error page's `<title>` unless one of the first three lines begins with it as a whole
  word — `PageText`): comments first, then styles, scripts and `<head>`; every markup stripped (tags with Word's
  `o:p`, `<!DOCTYPE>`, Outlook's `<![if …]>`), but a `<` not followed by a tag stays text; a block starts and ends a
  line (several in a row and the source newline after them — one break; a Word cell's own `<p>` stays on the cell's
  line); with paragraphs or a table (`HtmlBlocks`) source whitespace is one space except in `<pre>` (its lines kept,
  not its indentation — ponytail); without them source newlines are lines. Entities are decoded in both, and a line of
  (non-breaking) spaces is blank. Board cards cut by the old parser are repaired by `Apply` when they equal
  `CutDescription` (`LegacyText` of the raw server text, carried by `IntraserviceTask` and `IntraserviceFound`).
  Samples: «Живые ответы (2026-10-12)» in `HttpIntraserviceClient.SelfCheck`.
