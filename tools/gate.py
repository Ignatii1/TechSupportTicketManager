#!/usr/bin/env python3
"""The gate: every mechanical check of a change, one command, a short answer.

    python3 tools/gate.py                 # the sides (rust / cs) touched since origin/main, everything
    python3 tools/gate.py --quick         # no second clippy target, no Release build - the loop while working
    python3 tools/gate.py --rust --tests board ui      # only these cargo test targets
    python3 tools/gate.py --cs --areas board card      # only these SelfCheck areas
    python3 tools/gate.py --all           # both sides whatever changed

Prints one line per step and, for a failed step, the end of its output; whole logs go to a temp folder (named at the
end). Exit code 0 only when every step passed. Workers, reviewers and the lead run the same command - nobody reports
"tests pass" from memory.
"""
import argparse
import os
import subprocess
import sys
import tempfile
import time
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TEXT = (".cs", ".xaml", ".py", ".md", ".rs", ".slint", ".toml", ".yml", ".json")


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout


def changed(since):
    """Paths that differ from `since` (committed or not) plus untracked ones."""
    names = set(git("diff", "--name-only", since).split())
    names |= set(git("ls-files", "--others", "--exclude-standard").split())
    return sorted(names)


def hidden_characters():
    """Unicode Cf (RLO, zero-width...) in tracked or new text files: invisible in review - see AGENTS.md."""
    files = git("ls-files").split() + git("ls-files", "--others", "--exclude-standard").split()
    bad = []
    for name in files:
        path = ROOT / name
        if not name.endswith(TEXT) or not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            bad.append(f"{name}: not UTF-8")
            continue
        if any(unicodedata.category(c) == "Cf" for c in text):
            bad.append(f"{name}: a Unicode Cf character")
    return bad


def crlf(names):
    """The repo is LF everywhere (.gitattributes): mutant patterns and golden files are compared byte for byte."""
    bad = []
    for name in names:
        path = ROOT / name
        if name.endswith(TEXT) and path.is_file() and b"\r\n" in path.read_bytes():
            bad.append(f"{name}: CRLF line endings")
    return bad


class Gate:
    def __init__(self):
        self.logs = Path(tempfile.mkdtemp(prefix="gate-"))
        self.failed = []
        self.count = 0
        self.started = time.time()

    def step(self, name, command=None, cwd=None, check=None, summary=None):
        """Run a command (or a Python check returning a list of problems) and print its one line."""
        self.count += 1
        began = time.time()
        if check is not None:
            problems = check()
            ok, output = not problems, "\n".join(problems)
        else:
            env = dict(os.environ, CARGO_PROFILE_DEV_DEBUG="0", CARGO_TERM_COLOR="never", DOTNET_NOLOGO="1")
            run = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True)
            ok, output = run.returncode == 0, run.stdout + run.stderr
        log = self.logs / (name.replace(" ", "-").replace(":", "") + ".log")
        log.write_text(output, encoding="utf-8")
        note = summary(output) if ok and summary else ""
        print(f"{'ok  ' if ok else 'FAIL'} {name}{' - ' + note if note else ''} ({time.time() - began:.0f} s)", flush=True)
        if not ok:
            self.failed.append(name)
            tail = [line for line in output.splitlines() if line.strip()][-30:]
            print("\n".join("     | " + line for line in tail), flush=True)
        return ok

    def finish(self):
        took = time.time() - self.started
        if self.failed:
            print(f"GATE: FAIL - {', '.join(self.failed)} ({self.count} steps, {took:.0f} s; logs: {self.logs})")
            return 1
        print(f"GATE: OK ({self.count} steps, {took:.0f} s; logs: {self.logs})")
        return 0


def cargo_tests(output):
    passed = failed = 0
    for line in output.splitlines():
        if line.startswith("test result:"):
            words = line.split()
            passed += int(words[3])
            failed += int(words[5])
    return f"{passed} passed" + (f", {failed} failed" if failed else "")


def main():
    p = argparse.ArgumentParser(description="Every mechanical check of a change.")
    p.add_argument("--rust", action="store_true", help="the Rust prototype")
    p.add_argument("--cs", action="store_true", help="the C# app")
    p.add_argument("--all", action="store_true", help="both sides whatever changed")
    p.add_argument("--quick", action="store_true", help="skip the Windows clippy target and the Release build")
    p.add_argument("--tests", nargs="+", default=[], metavar="T", help="cargo test targets (default: all)")
    p.add_argument("--areas", nargs="+", default=[], metavar="A", help="SelfCheck areas (default: all)")
    p.add_argument("--since", default="origin/main", help="what the change is measured from (default: origin/main)")
    a = p.parse_args()

    names = changed(a.since)
    rust = a.rust or a.all or (not a.cs and any(n.startswith("rust/") for n in names))
    cs = a.cs or a.all or (not a.rust and any(n.startswith(("TicketBoard/", "TicketBoard.SelfCheck/")) for n in names))

    gate = Gate()
    gate.step("text: no hidden characters", check=hidden_characters)
    gate.step("text: LF line endings", check=lambda: crlf(names))

    if rust:
        cwd = ROOT / "rust"
        gate.step("rust: fmt", ["cargo", "fmt", "--check", "--", "--color", "never"], cwd)
        gate.step("rust: clippy", ["cargo", "clippy", "--all-targets", "--", "-D", "warnings"], cwd)
        if not a.quick:
            gate.step(
                "rust: clippy for Windows",
                ["cargo", "clippy", "--all-targets", "--target", "x86_64-pc-windows-gnu", "--", "-D", "warnings"],
                cwd,
            )
        targets = [arg for t in a.tests for arg in ("--test", t)]
        gate.step("rust: tests" + (f" ({' '.join(a.tests)})" if a.tests else ""), ["cargo", "test", *targets], cwd, summary=cargo_tests)

    if cs:
        if not a.quick:
            gate.step("cs: build", ["dotnet", "build", "-c", "Release", "-v", "q"], ROOT / "TicketBoard")
        areas = ["--", *a.areas] if a.areas else []
        last_line = lambda out: ([line for line in out.splitlines() if line.startswith("SelfCheck:")] or [""])[-1]
        gate.step(
            "cs: SelfCheck" + (f" ({' '.join(a.areas)})" if a.areas else ""),
            ["dotnet", "run", "--project", "TicketBoard.SelfCheck", *areas],
            ROOT,
            summary=last_line,
        )

    if not rust and not cs:
        print("     no code changed since " + a.since + " - text checks only (use --rust, --cs or --all to force)")
    return gate.finish()


if __name__ == "__main__":
    sys.exit(main())
