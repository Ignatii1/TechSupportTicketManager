#!/usr/bin/env python3
"""Mutants for the lines a round changed (the Rust prototype): do the checks notice a planted bug?

    python3 tools/mutants.py --since <ref>            # every changed file of rust/src, each against its own checks
    python3 tools/mutants.py --since <ref> --list     # only count them
    python3 tools/mutants.py --since <ref> --file src/board.rs --iterate   # one file; only what survived last time

Runs cargo-mutants in copies of the tree (the working tree is not touched) - but each run copies the tree at its
start, so don't edit rust/ while it is going. A survivor means a missing check (give it to the tester) or an equivalent
mutant (say why in docs/HISTORY.md). Exit code 1 on a survivor.

The C# side has hand-written lists per area: python3 TicketBoard.SelfCheck/mutants/mutate.py <areas> -j 3
"""
import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST = ROOT / "rust"

# which `cargo test` targets pin a source file (without -C every mutant builds all test targets: ~1 min instead of ~8 s)
CHECKS = {
    "src/app.rs": ["ui"],
    "src/board.rs": ["board"],
    "src/intraservice/mod.rs": ["intraservice"],
    "src/intraservice/charset.rs": ["intraservice"],
    "src/intraservice/errors.rs": ["errors"],
    "src/intraservice/html.rs": ["html"],
    "src/intraservice/parse.rs": ["parse"],
    "src/intraservice/query.rs": ["query"],
}
# no check can run these on Linux: the event loop, Win32 calls
SKIPPED = {"src/main.rs": "needs a real event loop", "src/clipboard.rs": "Windows only", "src/lib.rs": "module list"}


def checks_of(source):
    if source in CHECKS:
        return CHECKS[source]
    stem = Path(source).stem
    return [stem] if (RUST / "tests" / f"{stem}.rs").exists() else []


def lines(path):
    return [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()] if path.exists() else []


def main():
    p = argparse.ArgumentParser(description="Mutants for the changed lines of rust/src.")
    p.add_argument("--since", required=True, help="the commit the round started from")
    p.add_argument("--file", nargs="+", default=[], help="only these files (src/...)")
    p.add_argument("--list", action="store_true", help="count the mutants, run nothing")
    p.add_argument("--iterate", action="store_true", help="only the mutants that survived the previous run (same --out)")
    p.add_argument("-j", type=int, default=4, help="copies in parallel (default 4)")
    p.add_argument("--out", help="where cargo-mutants keeps its results (default: a new temp folder)")
    a = p.parse_args()

    env = dict(os.environ, CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never")
    diff = subprocess.run(["git", "diff", "--relative", a.since, "--", "src"], cwd=RUST, capture_output=True, text=True).stdout
    sources = sorted({line[6:] for line in diff.splitlines() if line.startswith("+++ b/src/")})
    if a.file:
        sources = [s for s in sources if s in a.file]
    if not sources:
        print(f"MUTANTS: nothing changed in rust/src since {a.since}")
        return 0
    out = Path(a.out) if a.out else Path(tempfile.mkdtemp(prefix="mutants-"))
    out.mkdir(parents=True, exist_ok=True)
    patch = out / "round.diff"
    patch.write_text(diff, encoding="utf-8")

    missed_total, rows = [], []
    for source in sources:
        if source in SKIPPED:
            rows.append((source, f"skipped: {SKIPPED[source]}"))
            continue
        base = ["cargo", "mutants", "--in-diff", str(patch), "-f", source]
        if a.list:
            listed = subprocess.run([*base, "--list"], cwd=RUST, env=env, capture_output=True, text=True).stdout
            rows.append((source, f"{len([m for m in listed.splitlines() if m.strip()])} mutants, checks: {' '.join(checks_of(source)) or 'ALL (no test target of its own)'}"))
            continue
        where = out / source.replace("/", "_")
        only = [arg for t in checks_of(source) for arg in ("-C=--test", f"-C={t}")]
        command = [*base, "-j", str(a.j), *only, "-o", str(where), *(["--iterate"] if a.iterate else [])]
        print(f"... {source} against {' '.join(checks_of(source)) or 'all checks'}", flush=True)
        run = subprocess.run(command, cwd=RUST, env=env, capture_output=True, text=True)
        (out / (source.replace("/", "_") + ".log")).write_text(run.stdout + run.stderr, encoding="utf-8")
        results = where / "mutants.out"
        caught, missed, timeout, unviable = (lines(results / f"{kind}.txt") for kind in ("caught", "missed", "timeout", "unviable"))
        if run.returncode not in (0, 2, 3) or not results.exists():
            # 2 = survivors, 3 = timeouts; anything else is a run that never tested (a broken baseline, no disk)
            rows.append((source, f"RUN FAILED (exit {run.returncode}) - see {out / (source.replace('/', '_') + '.log')}"))
            missed_total.append(f"{source}: the run failed")
            continue
        # a hang counts as caught (the check noticed by not finishing), as in the C# runner
        rows.append((source, f"{len(caught) + len(timeout)} of {len(caught) + len(timeout) + len(missed)} caught" + (f", {len(unviable)} don't build" if unviable else "")))
        missed_total += missed

    for source, text in rows:
        print(f"{source:34} {text}")
    for m in missed_total:
        print("  SURVIVED " + m)
        # the line the planted bug is in - what the tester needs to write the missing check
        where = m.split(":")
        if len(where) > 2 and where[1].isdigit() and (RUST / where[0]).is_file():
            source = (RUST / where[0]).read_text(encoding="utf-8").splitlines()
            if int(where[1]) <= len(source):
                print("           | " + source[int(where[1]) - 1].strip())
    print(f"MUTANTS: {'FAIL - ' + str(len(missed_total)) + ' survived' if missed_total else 'OK'} (results: {out})")
    return 1 if missed_total else 0


if __name__ == "__main__":
    sys.exit(main())
