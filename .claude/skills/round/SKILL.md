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
3. **Slice into cards** ([card.md](card.md) is the template). A good card is one behaviour with a named oracle, files
   nobody else touches, an API you have fixed, and a command that runs its checks. If you can't write its acceptance
   line, the design isn't finished. Keep for yourself: whatever is faster to do than to brief; markup that is judged by
   screenshots; wiring that crosses several cards; every decision about what the product should do.
4. **Per card: tester, then builder.** Spawn the tester with the card. Look at its checks before the builder starts —
   do they pin the card, or something easier? Then the builder, with the same card and the tester's report. One card at
   a time in this working tree: two builders break each other's builds. Parallel cards are worth a worktree
   (`isolation: "worktree"`) only when each is big enough to pay for a cold build.
5. **Gate.** Run `python3 tools/gate.py` yourself; start `python3 tools/mutants.py --since <base>` in the background.
   A red gate goes back to the builder with the output. A surviving mutant goes to the tester as «this bug passes your
   checks». A card that comes back wrong twice is yours: re-slice it or do it — the same brief a third time won't give
   a different answer.
6. **Review.** Choose the lenses the diff deserves — a port: parity and tests; network, files or secrets: security;
   state, timing or concurrency: correctness — and spawn those reviewers in one message so they run in parallel. Each
   gets its lens, the diff range and the card. Not the builder's report.
7. **Triage.** Each finding: accept, reject (with the reason, one line, for the record), or — unsure — ask the tester to
   reproduce it. An accepted finding becomes a failing check first (tester), then a fix (builder), then the gate again.
8. **Final check — yours alone.** Read the whole diff once, top to bottom. Run the gate yourself on the final tree. Go
   down the check list: each item has a check that was seen red. For a port, compare the two or three riskiest rules
   with the C# yourself. Name what stays unverified (Windows, the live server, the look).
9. **Close.** Docs (`AGENTS.md` → «Docs to keep in sync»), commit, CI, merge and release as `AGENTS.md` says. Add the
   retro to the round's HISTORY entry: per role — cards, returns, findings accepted and rejected, what you redid
   yourself. That line is how this process is tuned: cards too big, a lens that never finds anything, a role that needs
   a stronger or a cheaper model.

## Rules of evidence

- A result is a command and its output. «Tests pass» in a report is a claim until you have run the gate.
- A finding is a line, a scenario and a quote. A reviewer's confidence is not evidence; a reproduced failure is.
- A check that was never red proves nothing. A mutant that survives is a missing check or an equivalent mutant — say
  which, and why.

## Spending

- Every agent starts cold: it reads `AGENTS.md`, the card, and what the card points to. Brief it completely the first
  time — goal, oracle with line numbers, API, files, command, what not to touch. A second message costs more than a
  longer first one.
- Don't spawn for what a few tool calls of your own would settle, and don't split one small job between agents. A
  round of one small card may be cheaper done solo with reviewers at the end; say so in the retro when it is.
- Reports are capped at 30 lines: the conclusion and its evidence, not the story.

## Spawning

`Agent` with `subagent_type: "tester"`, `"builder"` or `"reviewer"`; the model comes from the role file in
`.claude/agents/`. Wait for a tester or a builder (`run_in_background: false`) when the next step needs the result and
nothing else can be done meanwhile; reviewers go together, in the background. If a session doesn't list those types,
spawn `general-purpose` with `model: "sonnet"` (tester, builder) or `"opus"` (reviewer) and start the prompt with «Read
`.claude/agents/<role>.md` and act as that role».
