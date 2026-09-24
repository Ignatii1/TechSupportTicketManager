# API-IDEAS.md — what the Intraservice API makes possible

Read from *IntraService API v5.51* (82 pages, the PDF the user supplied 2026-09-21). Credentials are confirmed working
against the live server, so everything here is available — but **nothing below has been tried against the real server yet**,
and the doc is the only source. Ordered by how much it improves daily use, not by effort.

The app's rule today (`AGENTS.md`): **it never writes to Intraservice.** Tier 4 breaks that rule and needs the user's
explicit decision; tiers 1–3 do not.

## The two facts that unlock everything

- `GET /api/user?getcurrentuserinfo=true` → `Id`, `Login`, `Name`, `RoleType`, `UtcOffset`, **`DefaultTaskFilterId`**.
  One call at startup gives the "who am I" the app has never had.
- `GET /api/task?…` is a full query surface, not just a single-ticket lookup: filter, sort, page, pick fields, and pull
  related objects in one round trip. Everything in tier 1 is one variation of this call.

## Tier 1 — stops the board being typed by hand

**1.1 Import my tickets automatically.** `GET /api/task?ExecutorIds={myId}&StatusIds={open ids}&fields=…&include=status,priority`
Today every card is created by hand through quick capture. This fills the board by itself: everything assigned to me,
already titled, prioritised and statused. Quick capture stays for tickets that aren't mine yet.
Alternative shape: `GET /api/task?filterid={DefaultTaskFilterId}` — reuses the filter the user already built in the web UI
(«Мои заявки»), so the selection rule lives in Intraservice instead of in our settings. `GET /api/filter?resource=task`
lists the saved filters for a settings dropdown.
*Cost:* a real sync loop (match by `Id`, insert new, retire disappeared), the biggest item here. *Risk:* a wrong filter
floods the board — cap the page size and show what will be imported before the first run.

**1.2 Incremental refresh instead of manual «Обновить».** *Done in 0.7.0 differently:* every N minutes the app re-reads the
full list of my open tickets (the import query, already verified) instead of `ChangedMoreThan` (unverified: date format,
server time zone) — one or two requests per tick at this user's volume.  `GET /api/task?ExecutorIds={myId}&ChangedMoreThan={lastSync}`
Poll every few minutes and only touch what changed server-side. Cheap enough to run in the tray. This is the feature the
existing `PROGRESS.md` open-work item asks for, and the parameter that makes it affordable.
*Note:* there is no webhook or push for us — `/api/token` is Apple/Google device tokens for IntraVision's own mobile apps,
so polling is the only option. `count=false` returns just `HasNextPage` and skips counting 100k tickets.

**1.3 Know when a ticket is really done.** `GET /api/taskstatus` → each status carries `IsFixed` («Заявка выполнена»),
`IsFinal` («Конечный»), `IsDeadlineAccountingPaused`, `IsConcurrence`, `IsExternal`.
The app currently compares status *names* as strings. With these flags it can move a card to «Готово» by itself (or offer
to), and stop the age counter while the ticket is parked in a paused or external status — which is what «Ждёт ответа» is
trying to express by hand. Fetch once per session and cache.

## Tier 2 — the board tells the truth about urgency

> **2.1 and 2.2 are dropped (2026-09-22):** the user's company fills in neither deadlines nor priorities in
> Intraservice, so there is nothing to read. Kept below for reference only.

**2.1 Real deadlines instead of the days-in-column heuristic.** Ticket fields `Deadline`, `ReactionDate`,
`ReactionDateFact`, `ResolutionDateFact`, `ReactionOverdue`, `ResolutionOverdue`.
`TicketRules.OverdueDays` is a guess the user tuned by hand; Intraservice knows the actual SLA. The age badge becomes
«до 17:40» / «просрочена на 2 ч», and the red border means something real. Server-side variants exist too:
`GET /api/task?ResolutionOverdue=true`, `DeadlineLessThan={today 18:00}`.
*Decision needed:* keep the local counter as a fallback for tickets with no deadline, or drop it.

**2.2 Priorities from the server.** `GET /api/taskpriority` → `Id`, `Name`, `Description`, `Image16Url`.
Our three chips (Low/Mid/High) are local invention. Map them to the real priority list so the chip on the card matches
what the ticket actually has, and so `PriorityIds=…` filtering agrees with the board filter.

**2.3 More on the card, free of charge.** Already in the same response: `Creator` (who asked), `CreatorPhone`,
`CreatorEmail`, `CreatorPosition`, `CreatorCompanyName`, `ServiceName`, `Type`, `ExecutorGroup`, `Categories`, `Assets`.
"Who is this for and how do I reach them" without opening the browser. Pick two or three for the card, the rest for the
detail panel.

## Tier 3 — context without leaving the app

**3.1 The ticket's real history and comments.** `GET /api/tasklifetime?taskid={id}&include=status&lastcommentsontop=true`
Returns every change with `Date`, `Editor`, `StatusId`, `Comments`, `IsPublic`, plus file add/remove events. This is where
Intraservice comments live. Shown under the local notes, the panel becomes the whole story of the ticket. Paged, so fetch
one page on demand — not during the background sync.

**3.2 Find a ticket that isn't on the board.** `GET /api/task?search={text}` — the doc says `search` covers the fields
marked searchable **and all comments of the ticket**, with the same rules as the web UI. Our search box only sees local
cards; this would let it fall through to the server and offer to add what it finds.

**3.3 Attachments.** `FileNames` / `FileIds` on the ticket, and the per-file GET in «Получение файла, привязанного к
заявке». Listing the names in the panel is cheap; downloading is a separate, bigger decision (where to put them).

**3.4 Logged time.** `GET /api/taskexpenses?taskid={id}` → minutes, date, who, comment; the ticket itself carries `Hours`
and `Price`. Only worth building if the user actually logs time.

## Tier 4 — writing back (needs the "read-only" rule to be lifted)

The API supports all of this; the app's own rule currently forbids it. In rough order of usefulness:

**4.1 Comment from the app.** `PUT /api/task/{id}` with `{"Comment": "…", "IsPrivateComment": false}`.
The natural pair to local notes: write once, and it lands in the ticket where colleagues see it.

**4.2 Move the ticket when the card moves.** `PUT /api/task/{id}` with `{"StatusId": …}`. Dragging a card to «Готово»
would close the ticket for real. Guard rails: `UserTaskRights.ToStatuses` (`include=USERTASKRIGHTS`) lists exactly which
statuses this user may move the ticket to, and `StatusIsCommentRequired` says when a comment is mandatory.

**4.3 Log time.** `POST /api/taskexpenses` — minutes + comment, per ticket.

**4.4 Create a ticket.** `POST /api/task` (needs `Name`, `ServiceId`, `StatusId`, `PriorityId`, `TypeId`; get sane
defaults from `GET /api/newtask?serviceid=…&tasktypeid=…`). Turns quick capture into real intake. The heaviest item and
the least aligned with what this app is for.

**Whenever writes happen:** send back the `Changed` value received with the ticket. Intraservice then answers `409 Conflict`
with who changed it and when, instead of silently overwriting a colleague's edit.

## Tier 5 — plumbing worth doing with any of the above

- `fields=Id,Name,StatusId,PriorityId,Deadline,Changed,…` — ask only for what gets stored. Smaller responses, faster sync.
- `include=STATUS,PRIORITY,USER,SERVICE` — names resolved in the same response instead of N extra calls.
- `pagesize` (max 2000, default 25) + the `Paginator` block — required for a first import; the current client ignores paging.
- `X-API-Version` response header — log it once; makes "which version is this server" answerable.
- `GET /api/settings?keys=…` — system settings by key (e.g. `maxfilesize`), only interesting if files get implemented.
- Device headers `Device-Name` / `Device-Version` are accepted on every request and show up in the ticket's audit trail.
  Free, and makes "changed by TicketBoard" visible on the Intraservice side — worth sending as soon as anything is written.

## Not available, so don't plan for it

- **No push, no webhooks, no long-poll.** Sync is polling, full stop.
- **No DELETE** in the API at all ("в настоящее время не реализовано").
- The response format for everything above is still unverified against the real server — `HttpIntraserviceClient.Parse`
  was written from the v5.42 doc. **First task of any of these: capture one real response and put it in `SelfCheck`.**

## Suggested order if the user wants a roadmap

1. `getcurrentuserinfo` + `/api/taskstatus` cached at startup (tiny, unlocks 1.1/1.3).
2. Auto-import by filter or `ExecutorIds` (1.1) with a preview before the first run.
3. Background incremental sync on `ChangedMoreThan` (1.2), reusing the existing `SyncAsync` path.
4. Auto-«Готово» via `IsFixed` (1.3).
5. Lifecycle comments in the panel (3.1).
6. Only then decide about writes (tier 4).
