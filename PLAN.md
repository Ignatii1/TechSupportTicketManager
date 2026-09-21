# PLAN.md — work queue for the 2026-09-21 round

Working doc for agents. Delete this file when the round is merged; the record goes to `PROGRESS.md`.
Read `AGENTS.md` first. Everything below is verified against the code at `1015ca4`.

## Status before the round

- Branch `claude/beautiful-albattani-pd6q7i` (= `main` + 2 commits) **is not merged into `main`.**
  It already contains **T1 (delete notes)** and **T5 (keys 1/2/3 on the board)**. The user tested a `main` build,
  which is why both looked missing. Exe with them: Actions → run `34753438355` → Artifacts.
- API credentials work against the live Intraservice server (user confirmed 2026-09-21). The
  "response format unverified" caveat in `PROGRESS.md` can be relaxed once `GetTaskAsync` is seen returning a real title.
- No .NET SDK in the agent container and the download is blocked by network policy → **nothing compiles locally.**
  The only compile check is CI: push a branch, then dispatch `build.yml` on it (workflow_dispatch is enabled).

## Tasks

**Done inline on 2026-09-21 (XAML only, no logic), unverified on Windows:**
- **T3** — note timestamp and note text now share a baseline: both `TextBlock`s in the `NoteItem` template got
  `LineHeight="20" LineStackingStrategy="BlockLineHeight"`, so the 12 px date no longer rides above the 14 px text.
- **T4** — the «Статус» ComboBox lost its hard `Height="28" MinHeight="28"` (which fought WPF-UI's taller template and
  pinned the value top-left) and gained `HorizontalContentAlignment="Center" VerticalContentAlignment="Center"`.
  Horizontal centring is what the user asked for; it is a one-attribute revert. `Padding="8,0,28,0"` kept as written —
  the right padding clears the chevron, so a short value may read a few px left of true centre. Tune only if the user says so.
- **T7** — `IntraserviceLinkParser.TryParse` now takes a field that is nothing but 4–8 digits as the ticket number (the
  settings regex is untouched, so a number inside ordinary text is still ignored); `MainViewModel.AddFromCapture` builds
  `{base}/Task/View/{id}` for it so ↗ works; `QuickCaptureViewModel.DigitsSetPriority` now keys off a recognised **link**
  instead of a recognised number, otherwise the `1` in `702180` would have been swallowed as a priority change.
  A Debug-only `IntraserviceLinkParser.SelfCheck()` covers the cases and runs from `App.OnStartup`.
- **T6** — the quick-capture window was pinned at `Height="160"` while its content measures ~145 px plus whatever the
  caption reserves at the user's DPI, so the Grid compressed and the footer drew over the priority chips. Now
  `SizeToContent="Height"` with no fixed `Height`/`MinHeight`, and the priority row is `Auto` instead of a hard 24 px.
  If `SizeToContent` misbehaves under Mica/`ExtendsContentIntoTitleBar`, fall back to a fixed taller window (~196).

T1 and T5 need no code. Everything else is grouped into three agents whose file sets do not overlap.

### T1 — delete notes ✔ done, unverified
Code is on the branch (`MainViewModel.DeleteNote` + hover ✕ in the `NoteItem` template, style `NoteDeleteButton`).
Action: user verifies on Windows. No agent.

### T5 — keys 1/2/3 on the board ✔ done, unverified
Code is on the branch (`MainWindow.OnPreviewKeyDown` → `MainViewModel.SetSelectedPriority`).
Action: user verifies on Windows. No agent.

---

### Agent A — dropped, T7 done inline on 2026-09-21

### Agent B — dropped, T3 and T4 done inline on 2026-09-21

### Agent C — portable data folder (T2)

Owns: `App.xaml.cs` (the `DataDir` property only), `README.md` → the «Данные» / files section,
`TicketBoard/README.md` → the short file list / notes.

Today everything lives in `%APPDATA%\TicketBoard` (`tickets.json`, `settings.json`, `backups/`, `errors.log`), resolved once
in `App.DataDir`. The user wants "copy a file and it works on the other machine".

Ship the small version: a pointer file next to the exe.
- `datadir.txt` next to the exe (`AppContext.BaseDirectory`, which is the exe folder for our single-file publish):
  absent → today's `%APPDATA%\TicketBoard` (unchanged, no migration); present and empty → the exe's own folder
  (USB-stick / portable mode); present with a path → that path (OneDrive, a network share, a synced folder).
- Wrap it so a bad path cannot break startup: any exception → fall back to `%APPDATA%`, and report the fallback through the
  existing `LogError`. This runs before the error handlers are installed, so keep it exception-free by construction.
- No settings-window field, no migration wizard, no file watcher. Keep `DataDir` a single resolved-once static.
- **DPAPI caveat, must be documented, must not be "fixed":** the Intraservice password is encrypted to the Windows user +
  machine, so a copied `settings.json` will not carry the password to another PC — it has to be re-entered there. Everything
  else (`tickets.json`, backups) is plain JSON and copies fine. Do not weaken the encryption to make this nicer.
- README: what to copy (`tickets.json`, optionally `settings.json`), where it lives, how `datadir.txt` works, and the
  password caveat. The tray menu already has «Открыть папку с данными», so no new UI.

## Rules for every agent

1. `AGENTS.md` rules hold: Russian UI text and code comments, no new NuGet packages, no interfaces with one implementation,
   `// ponytail:` on deliberate shortcuts naming their ceiling. Commit messages are English.
2. **Do not run any git command and do not touch `PROGRESS.md` or `PLAN.md`.** The orchestrator commits and writes the log;
   three agents editing the top of the log would conflict every time.
3. Stay inside your file list. If a fix needs a file another agent owns, stop and report it instead of editing it.
4. You cannot build or run anything: no .NET SDK, the SDK download is blocked, and the UI is Windows-only. Say plainly what
   you did not verify. Do not claim a UI change works.
5. Re-read your own diff adversarially before reporting — a XAML typo costs a whole CI round trip.
6. Report: what changed and where, what you chose when the brief offered a choice, what you could not verify, and anything
   suspicious you left alone.

## Review process

Gate 1 — orchestrator read, per agent, before its work is committed
- Diff matches the brief and the owned file list; nothing else moved.
- `AGENTS.md` conventions: Russian comments/UI strings, no new package, docs table still accurate.
- XAML by eye: every `StaticResource` key exists (`Themes/Styles.xaml`), every `DynamicResource` colour key exists in
  **both** `Tokens.Light.xaml` and `Tokens.Dark.xaml`, `RelativeSource` ancestors are real, `Grid.Row`/`Grid.Column` fit the
  definitions.
- C# by eye: nullable-clean, no new `async void`, no secret in a user-facing string, saves still ride the existing
  `OnTicketChanged` → `ScheduleSave` path.

Gate 2 — compile, on the integration branch
- Commit all three agents' work to `claude/beautiful-albattani-pd6q7i`, push, then dispatch `build.yml` on that branch
  (`actions_run_trigger` → `run_workflow`, ref = the branch) and wait for the job. Red = fix before anything else.

Gate 3 — automated review
- Run `/code-review high` over the round's diff (`main..claude/beautiful-albattani-pd6q7i`). Fix confirmed findings;
  for anything rejected, say why in one line.
- Re-dispatch `build.yml` after fixes. Green build is the entry condition for Gate 4.

Gate 4 — the user on Windows (the only real test)
Hand over the CI artifact link with this checklist:
1. Select a card, press `1` / `2` / `3` → the priority chip changes; the change survives a restart.
2. Typing in the search box still types digits; `Esc` still leaves the field.
3. Hover a note → ✕ appears at the right; click → the note disappears, the card's note counter drops, survives a restart.
4. The note timestamp and the note text sit on the same line.
5. The «Статус» value is centred in its box; the dropdown still opens and changing it still moves the card.
6. `Ctrl+Alt+Space`: the priority chips are fully visible, nothing overlaps the Enter / hotkey footer.
7. In quick capture type `702180` (no `#`): the `#702180` chip appears, the title comes from Intraservice, Enter creates the
   card with the real title, and ↗ opens the right page. Typing the number must not jump the priority around.
8. `1` / `2` / `3` still set the priority when the field is empty and when it holds a pasted link.
9. Put an empty `datadir.txt` next to the exe, restart → data now lives next to the exe; delete it, restart → the old
   `%APPDATA%` data is back.

Gate 5 — merge
Only when Gates 2–4 pass: append the round to `PROGRESS.md` (what changed, what was verified and how, what is still
unverified), delete `PLAN.md`, merge to `main`, and tag only if the user asks.
