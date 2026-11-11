# AGENTS.md — the core every agent reads

This file is loaded into every session and every subagent, so it holds only what all of them need. The rules of an
area — where the code is, what it must keep doing, which check pins it — live in `docs/map/<area>.md`: **read the map
file of an area before you change it** (index below). The lead of a round also reads `PROGRESS.md` (state, open work)
and `docs/ROADMAP.md` (the agreed plan), and at the end updates `PROGRESS.md` and adds an entry at the top of
`docs/HISTORY.md`. A worker with a task card reads the card and what it points to, nothing more.

## What this is

**TicketBoard** («Заявки»): a personal Kanban tracker for [Intraservice](https://intraservice.ru) helpdesk tickets.
One user, one Windows PC. It lives in the tray; a global hotkey opens a quick-capture box; it reads ticket title/status
from the Intraservice REST API and **writes two things there: a comment from the panel** (0.24.0) **and a status when a
card is moved by hand** (0.25.0) — both off by default, each after a check and a preview, never retried; nothing else
writes: not auto-sync's or F5's moves, import, export or the Claude relay (`docs/map/write.md`). All data is local JSON
in the exe's own folder (`App.DataDir` = `AppContext.BaseDirectory`), so the app is portable.

**A Rust prototype lives in `rust/`** (since 2026-11-06; the user's reason: size, memory, speed; pre-releases `rust-*`,
not the product yet). The C# app is its oracle: a port keeps the C# behaviour rule by rule. It writes `tickets.json`
only while it holds TicketBoard.exe's single-instance mutex (else read-only), and never writes Intraservice or
`settings.json` (`docs/map/rust.md`, `rust/README.md`).

- WPF, .NET 10 (`net10.0-windows`), MVVM via CommunityToolkit.Mvvm (`[ObservableProperty]`, `[RelayCommand]`).
- Libraries: WPF-UI (Fluent/Mica windows), gong-wpf-dragdrop, H.NotifyIcon.Wpf. Nothing else; don't add packages for
  what a few lines do. Rust: Slint, ureq, serde, chrono, `windows` — the same rule.
- **Windows only, by decision.** Don't propose Avalonia/MAUI/Tauri/web ports.
- **UI text and code comments are Russian.** Keep that. Commit messages are English.
- Style: short, direct code, no one-implementation interfaces (an `IIntraserviceClient` was removed on purpose).
  `// ponytail: …` comments mark deliberate simplifications and name their ceiling; keep them honest.
- New code: immutable values (`record`, `init`); no `null` from new APIs (nullable warnings are build errors; a parsed
  response field the server may leave out stays nullable = «not sent»); small objects that own their rule instead of
  managers and helpers; decorators only once there are two behaviours. Not taken from Elegant Objects: an interface per
  class, a ban on static pure functions (the rule classes are the most testable code here), a ban on properties (WPF
  binding and the JSON format live on them).

## Lines nobody crosses

- **No API key, ever**: the user has claude.ai Pro only. Nothing here may call a model API or need a key.
- Intraservice is written only by the two switches above. The Rust prototype doesn't write it at all, and writes
  nothing anywhere while read-only.
- Credentials: the password stays DPAPI-encrypted; anything that can carry server text, a header or a URL goes through
  `Redact` / `redact` before it reaches a message, a log or a file. Contacts are never exported.
- Committed samples are anonymized: no real logins, passwords, names or server addresses.
- `tickets.json` and `settings.json` are read by both apps and by old versions: a persisted name is never renamed.

## How a round runs

A change bigger than a quick fix runs as a team: `/round <what to do>` (the playbook is `.claude/skills/round/SKILL.md`,
the roles are `.claude/agents/*.md`).

| Role | Model | Does | Never |
|---|---|---|---|
| lead | the session's — the strongest | design and check list in `docs/HISTORY.md`, task cards, decisions, triage, the final check, commits and release | accepts a claim without its evidence |
| tester | sonnet | a card's checks, from the card and the oracle, each seen red — before the code | takes expected values from an implementation |
| builder | sonnet | the code that makes them green, inside the card's files | edits a check |
| reviewer | opus | one lens over the diff — correctness, parity, tests or security — blind to the author's reasoning | edits or builds |
| gate | scripts | `tools/gate.py`, `tools/mutants.py`: what a script can decide, no model decides | — |

Order: design → cards → tester → builder → gate → reviewers → triage → **the lead's final check** → docs, merge, release.
What holds for everyone, team or solo (details: `docs/ROADMAP.md` → «Как работаем»):

- Tests first. The round's check list goes into its `docs/HISTORY.md` entry before the code; every new check is seen
  failing for the right reason; a bug gets a failing check before its fix; formats other programs read are compared
  with golden files; new checks come with mutants they kill.
- A result is a command and its output. A finding is a line, a scenario and a quote. A check that was never red proves
  nothing; a surviving mutant is a missing check or an equivalent mutant — say which.
- For a port, expected values come from running the C# code on the samples, not from reading it.
- A small fix (cause known, one or two files) is done solo: the failing check, the fix, the gate.

## Build and verify

The agent container builds both apps and runs every check. Use that for each change; CI is for releases.

```
python3 tools/gate.py                          # everything mechanical for the sides touched since origin/main:
                                               # hidden characters, LF, fmt, clippy (Linux + Windows target),
                                               # cargo test, dotnet build -c Release, SelfCheck
python3 tools/gate.py --quick --rust --tests board ui     # the loop while working: only these cargo test targets
python3 tools/gate.py --quick --cs --areas board card     # … only these SelfCheck areas (names: Program.cs)
python3 tools/mutants.py --since <base>                   # Rust: planted bugs in the lines changed since <base>
python3 TicketBoard.SelfCheck/mutants/mutate.py board -j 3   # C#: planted bugs, a list per area (mutants/*.py)
python3 TicketBoard.SelfCheck/coverage.py MainViewModel      # C#: lines no check executes
python3 TicketBoard.SelfCheck/xamlcheck.py <file.xaml> <ViewModelClass> [<RowClass>]   # resource keys and bindings
sudo apt-get update; sudo apt-get install -y dotnet-sdk-10.0   # once per container (`;`: update errors on dead PPAs)
```

What the gate runs, by hand: `cd TicketBoard && dotnet build -c Release`; `dotnet run --project TicketBoard.SelfCheck
[-- area …]` (prints «SelfCheck: OK»; refuses to run in Release); `cd rust && cargo fmt --check && cargo clippy
--all-targets [--target x86_64-pc-windows-gnu] && cargo test [--test name]` — with `CARGO_PROFILE_DEV_DEBUG=0` on every
cargo call (two debug-info settings side by side once grew `rust/target` to 21 GB). What each check drives, the fake
server, how the mutant runners work: `docs/map/selfcheck.md`.

- **The UI can't run on Linux.** Say so rather than claiming a UI change works; the user tests on his Windows work PC.
  The XAML compiles with the build, `xamlcheck.py` checks keys and bindings, the Rust window is tested headless and
  `rust/examples/shot.rs` renders PNGs of it. Rendering on Windows, focus and the tray stay unverified — name them.
- CI (`.github/workflows/build.yml`, windows-latest) runs SelfCheck and publishes the exe as a run artifact on pushes to
  `main`, PRs and manual runs. **The user tests only from GitHub Releases on his work PC**, so a finished round (built,
  SelfCheck green, reviewed) ends with a merge to `main` and a release; bump `<Version>` in `TicketBoard.csproj` in the
  same commit. **Tags can't be pushed from the agent container** (the git proxy refuses them) — dispatch `build.yml` with the
  `release_tag` input and GitHub creates the tag and the Release.

## Working with the user

- He is a developer and wants decisions made and explained, not asked — ask only at real forks. Say plainly what was not
  verified.
- Every round is reviewed before it is merged: by the reviewers of the round, then by the lead — `/code-review high`
  over the round's diff is the lead's final check (it has caught a real bug every round so far). A solo fix gets at
  least that.

## Working here without wasting context

- Compile and check locally; dispatch CI only for a release. Every CI poll returns kilobytes of JSON.
- Read by range (`grep -n`, then `sed -n 'a,bp'`), not whole files. The big classes are split into partial files by
  responsibility — open the one you need.
- Commit bodies ≤ 5 lines: CI and release responses echo them back. The reasoning goes into `docs/HISTORY.md` once.
- Release link = `https://github.com/Ignatii1/TechSupportTicketManager/releases/tag/vX.Y.Z`; don't fetch the object.
- Every subagent starts cold and reads this file — which is why it is short: a rule that only one area needs goes into
  that area's map file. Brief a subagent completely the first time; reports ≤ 30 lines.
- `docs/HISTORY.md` is half a megabyte: `grep` it for the round you need, never read it whole.
- API reference: the user's `IntraService_API_v5_51.pdf`. Extract with `pip install pypdf cffi` (plain pypdf crashes here
  on a broken `cryptography`). `API-IDEAS.md` already maps the endpoints worth having.

## Repository layout

```
README.md                  user guide (Russian): install, usage, settings, data files, troubleshooting
AGENTS.md                  this file
PROGRESS.md                current state and open work
API-IDEAS.md               what the Intraservice API offers, ranked; what's done is marked in PROGRESS
docs/map/                  the rules of each area (index below)
docs/HISTORY.md            past rounds and their reasons (grep it when you need the why)
docs/ROADMAP.md            the agreed plan: stages in order, acceptance criteria, the process every stage goes through
.claude/agents/            the roles of a round: tester, builder, reviewer
.claude/skills/round/      the lead's playbook (`/round`) and the task card template
tools/                     gate.py (every mechanical check), mutants.py (Rust mutants for the changed lines)
TicketBoard.SelfCheck/     console app that runs the C# checks on Linux (+ xamlcheck.py, mutants/, coverage.py)
dotnet-tools.json          dev tools pinned for `dotnet tool restore` (dotnet-coverage)
.github/workflows/build.yml   the C# app: SelfCheck, exe, a release by `release_tag` (v*)
.github/workflows/rust.yml    the Rust prototype: cargo test, exe, a pre-release by `release_tag` (rust-*)
rust/                      the Rust prototype (Slint): src/, ui/board.slint, tests/ (cargo test; the window headless),
                           examples/shot.rs (PNG of the windows)
TicketBoard/
  README.md                developer notes (Russian): build, CI, short file list, Intraservice API notes
  TicketBoard.csproj       packages, Release publish settings
  App.xaml(.cs)            composition root and app-level concerns
  Models/Ticket.cs         Ticket, Note, TicketStatus/Priority/AgeState enums, TicketRules
  Services/                persistence, settings, hotkey, autostart, theme; the Intraservice client; export; rules
  ViewModels/              MainViewModel (board; partials by responsibility), StatusForm, ColumnViewModel,
                           QuickCaptureViewModel, SearchViewModel, SettingsViewModel
  Views/                   MainWindow, TicketPanel, QuickCaptureWindow, SearchWindow, SettingsWindow, AskWindow,
                           StatusWindow (XAML + thin code-behind)
  Themes/, Converters/, Assets/   colors and shared styles, value converters, icons
```

`App.OnStartup` is the only composition root (no DI container); settings apply live through `App.ApplySettings`. The
startup order, the settings flow and the new-ticket data flow are in `TicketBoard/README.md` → «Как всё связано».

## Where the rules are

Each file answers «I want to change X → go here» for its area, with the rules the code must keep and the checks that
pin them. They are long on purpose — read the section you need (`grep -n '^## ' docs/map/<file>`).

| Map file | Covers |
|---|---|
| `docs/map/board.md` | ticket fields and what is saved (`tickets.json` names), age badge, columns, moves (`Ticket.MoveTo`), the board's filters, WIP limit, link parsing, titles, quick capture |
| `docs/map/intraservice.md` | the HTTP client and its error texts, JSON field names (only in `.Parse.cs`), html → text, what the live server really does and silently ignores |
| `docs/map/sync.md` | import of my tickets, F5, background auto-sync, what a sync overwrites on a ticket, reopened / reassigned tickets, unread comments |
| `docs/map/panel.md` | the detail panel: «Переписка», creator and executors, card fields, attachments (open, save, the danger rules) |
| `docs/map/write.md` | the two writes: a comment from the panel, a status on a hand move |
| `docs/map/search.md` | the search window: form → query (`TaskQuery`, `TicketSearch`), lists, preview, export from it |
| `docs/map/export.md` | the knowledge-base export: engine, file format and escaping, history facts, hidden passwords, `tickets.csv` and `SCHEMA.md`, «В базу знаний», «Спросить базу знаний» |
| `docs/map/export-files.md` | attachments in the knowledge base |
| `docs/map/autoexport.md` | auto-export of closed tickets, the mirror, the mark |
| `docs/map/ui.md` | windows, card look, colors, fonts, keys, dialogs; what can be checked without running the UI; WPF gotchas |
| `docs/map/app.md` | tray, hotkey, autostart, saving and backups, settings and the password, error log, packaging, the Claude relay |
| `docs/map/rust.md` | the Rust prototype file by file, its mutant runner |
| `docs/map/selfcheck.md` | what `TicketBoard.SelfCheck` compiles and what each check file drives, the fake server, the C# mutant runner and its history, coverage |

## Gotchas that bite everywhere

- **The agent's Write, Edit and Bash tools turn `\uXXXX` in their text into the character itself** (JSON-level
  unescaping) — it broke a comment once and in 0.20.0 put real RLO and zero-width characters into C# tests and the README
  (invisible in review: «Trojan Source»). Write such an escape from a script as `chr(92) + 'u202E'`, and after touching
  tests with them check that no tracked file holds a Unicode Cf character: `python3 -c "import unicodedata,subprocess;
  print([f for f in subprocess.check_output(['git','ls-files']).decode().split() if f.endswith(('.cs','.xaml','.py','.md'))
  and any(unicodedata.category(c)=='Cf' for c in open(f,encoding='utf-8').read())])"` must print `[]`.
- **WPF projects drop `System.IO` from the implicit usings** (it clashes with `System.Windows.Shapes.Path`) — add
  `using System.IO;` wherever you touch `File`, `Path` or `IOException`.
- `pkill -f <pattern>` kills your own shell when the pattern also occurs in the same command line — kill in a separate call.
- **Don't edit `rust/` while a mutant run is going**: each run copies the tree at its start. And mutation scripts that
  `git checkout` a file discard your uncommitted edits to it — commit first.
- A check that waits «a bit» and then looks is a flake on a slow machine: check a pause by when the request reached the
  fake server, wait for a condition, or assert state.

## Docs to keep in sync

- User-visible behavior → root `README.md` (Russian user guide; the keyboard, settings and error tables).
- Build, structure, API notes → `TicketBoard/README.md`; the Rust side → `rust/README.md`.
- New file, moved responsibility, a new rule of an area → that area's file in `docs/map/` (a new area → the index above).
- A rule every agent needs → here, and keep this file under 20 KB.
- Always → `PROGRESS.md` (state, open work) and a short entry at the top of `docs/HISTORY.md`.
