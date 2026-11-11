---
name: round
description: Run a development round as a team — you plan, slice and accept; cheaper models write the checks and the code; other models review; you do the final check. Use for any change bigger than a quick fix (a feature, a port of a C# area, a bug whose cause is unclear).
argument-hint: [what to build or fix]
---

# A round, run as a team

You are the lead: the strongest model in the session. Your context and your judgment are the scarce resources. Spend
them on the design, the decisions and the final check; hand the volume to cheaper models, whose work is checked three
times before you accept it — by scripts, by other models, by you.

The request: $ARGUMENTS

## Who does what

| Role | Runs on | Does | Never |
|---|---|---|---|
| **lead** (you) | the session's model | design and check list, task cards, decisions at forks, triage of findings, the final check, commits, release, the report | accepts a claim without its evidence |
| **tester** | sonnet | the checks of a card, written from the card and the oracle and proved red | takes expected values from an implementation |
| **builder** | sonnet | the code that makes them green, inside the card's files | edits a check |
| **reviewer** | opus | one lens over the diff: correctness, parity, tests or security | edits, builds, hears the author's explanation |
| **gate** | scripts | `tools/gate.py` (lint, build, checks, hidden characters), `tools/mutants.py` | — |

Why this shape. The one who writes the code doesn't write its checks: checks written after the code mirror it. The one
who reviews is a different model and reads blind: it can't inherit the author's blind spot or be talked into it. And
whatever a script can decide is decided by a script before any model spends tokens on it.

## The steps

1. **Understand.** `PROGRESS.md`, the roadmap entry, the map file of the area (`docs/map/`). For a port the C# source
   is the oracle: read it yourself where the rules are subtle; for wide reading send `Explore` agents and ask for rules
   with `file:line`, not summaries of files.
2. **Design.** The round's entry at the top of `docs/HISTORY.md`: what and why, the decisions, the check list — before
   any code, as always here. That entry is the plan of record: it survives context compaction, the cards don't.
   For a port of a pure rule, start its **parity table** now (`rust/tests/parity/<name>.tsv`: inputs, and the answers
   the C# code gives them — `docs/map/selfcheck.md` → «Parity tables»): register the C# function in
   `ParityCheck.Tables`, write the inputs that the check list calls for, let the C# code fill the answers. What you
   learn from those answers belongs in the design — .NET rarely does exactly what the code reads like.
3. **Slice into cards** ([card.md](card.md) is the template). A good card is one behaviour with a named oracle, files
   nobody else touches, an API you have fixed, and a command that runs its checks. If you can't write its acceptance
   line, the design isn't finished. Keep for yourself: whatever is faster to do than to brief; markup that is judged by
   screenshots; wiring that crosses several cards; every decision about what the product should do.
4. **Per card: tester, then builder.** Spawn the tester with the card. Look at its checks before the builder starts —
   do they pin the card, or something easier? Then the builder, with the same card and the tester's report. One card at
   a time in this working tree: two builders break each other's builds. Parallel cards are worth a worktree
   (`isolation: "worktree"`) only when each is big enough to pay for a cold build.
5. **Gate.** Run `python3 tools/gate.py` yourself, then `python3 tools/mutants.py --since <base>` — and wait for it:
   its result goes to the reviewer of the tests lens. A red gate goes back to the builder with the output. A surviving
   mutant goes to the tester as «this bug passes your checks», unless it is equivalent — then the code usually has a
   branch that can't matter: simplify it. A card that comes back wrong twice is yours: re-slice it or do it — the same
   brief a third time won't give a different answer.
6. **Review.** Commit first (reviewers read `git diff <base>..HEAD`). Choose the lenses the diff deserves and spawn
   them in one message so they run in parallel; each gets its lens, the diff range and the card — not the builder's
   report. **tests** — always: in the pilot it found what the mutant tool can't generate (bounds passed at a call
   site); give it the mutants' summary so it looks past what the tool already tried. **parity** — a port whose rule
   the parity table can't hold whole (state, order of calls, texts built from parts), or whose C# depends on the
   machine. **security** — network, files, secrets, anything the server's text can reach. **correctness** — state,
   timing, concurrency. Nobody edits `rust/` while reviewers read or mutants run.
7. **Triage.** Each finding: accept, reject (with the reason, one line, for the record), or — unsure — ask the tester to
   reproduce it. An accepted finding becomes a failing check first (tester), then a fix (builder), then the gate again.
8. **Final check — yours alone.** `/code-review high` over the round's diff: you read every line, after everyone else
   has. Run the gate yourself on the final tree (no `--quick`). Go down the check list: each item has a check that was
   seen red. For a port, compare the two or three riskiest rules with the C# yourself. Name what stays unverified
   (Windows, the live server, the look).
9. **Close.** Docs (`AGENTS.md` → «Docs to keep in sync»), commit, CI, merge and release as `AGENTS.md` says. Add the
   retro to the round's HISTORY entry: per role — cards, returns, findings accepted and rejected, what you redid
   yourself. That line is how this process is tuned: cards too big, a lens that never finds anything, a role that needs
   a stronger or a cheaper model.

## Rules of evidence

- A result is a command and its output. «Tests pass» in a report is a claim until you have run the gate.
- A finding is a line, a scenario and a quote. A reviewer's confidence is not evidence; a reproduced failure is.
- A check that was never red proves nothing. A mutant that survives is a missing check or an equivalent mutant — say
  which, and why.
- For a port, an expected value is what the C# code answered, not what someone read out of it: the parity table's last
  cell is written by `dotnet run --project TicketBoard.SelfCheck -- parity --write` and by nothing else.

## Spending

- Every agent starts cold: it reads `AGENTS.md`, the card, and what the card points to. Brief it completely the first
  time — goal, oracle with line numbers, API, files, command, what not to touch. A second message costs more than a
  longer first one.
- Don't spawn for what a few tool calls of your own would settle, and don't split one small job between agents.
- **What it costs (the pilot, 2026-11-11):** a card of 60 lines of code took four agents — tester, builder, two
  reviewers — about 70 thousand tokens each, 286 thousand together, and the two reviewers ran into the subscription's
  5-hour limit. An agent's cost is mostly its cold start, not its work. So: a card earns the whole team when there is
  something to read — an area to port, 200+ lines, rules nobody has written down. A small card is done solo — checks
  first, the gate, mutants — with one reviewer of the tests lens at the end. A fix round after the review (a few rows,
  a rename) is the lead's own: say so in the retro.
- The subscription's limit is shared by everyone in the session. Before a round with many cards, say what it will
  roughly cost; when an agent stops on the limit, its work is saved — resume it with `SendMessage` after the reset
  instead of spawning a new one.
- Reports are capped at 30 lines: the conclusion and its evidence, not the story.

## Spawning

`Agent` with `subagent_type: "tester"`, `"builder"` or `"reviewer"`; the model comes from the role file in
`.claude/agents/`. Wait for a tester or a builder (`run_in_background: false`) when the next step needs the result and
nothing else can be done meanwhile; reviewers go together, in the background. If a session doesn't list those types,
spawn `general-purpose` with `model: "sonnet"` (tester, builder) or `"opus"` (reviewer) and start the prompt with «Read
`.claude/agents/<role>.md` and act as that role».
