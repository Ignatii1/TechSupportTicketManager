#!/usr/bin/env python3
"""Мутанты для строк, которые изменил раунд (прототип на Rust): заметят ли проверки подложенную ошибку?

    python3 tools/mutants.py --since <ref>            # каждый изменённый файл rust/src - против своих проверок
    python3 tools/mutants.py --since <ref> --list     # только сосчитать
    python3 tools/mutants.py --since <ref> --file src/board.rs --iterate   # один файл; только выжившие в прошлый раз

cargo-mutants работает в копиях дерева (рабочее не трогает), но копию снимает в начале прогона - пока он идёт,
rust/ не править. Новый файл, ещё не добавленный в git, проверяется целиком. Выживший - это недостающая проверка
(отдать тестировщику) или равнозначный мутант (сказать почему в docs/HISTORY.md; обычно такую ветку кода можно
упростить). Код возврата 1 - есть выживший или прогон не состоялся. Строки вывода - английские: их цитируют роли.

У C# списки мутантов написаны руками, по областям: python3 TicketBoard.SelfCheck/mutants/mutate.py <areas> -j 3
"""
import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST = ROOT / "rust"

# какие цели cargo test держат файл (без -C каждый мутант собирает все цели проверок: минута вместо восьми секунд)
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
# это на Linux не запустить ни одной проверкой: цикл событий, вызовы Win32
SKIPPED = {"src/main.rs": "needs a real event loop", "src/clipboard.rs": "Windows only", "src/lib.rs": "module list"}


def git(*args):
    run = subprocess.run(["git", *args], cwd=RUST, capture_output=True, text=True)
    if run.returncode != 0:
        sys.exit(f"MUTANTS: FAIL - git {' '.join(args)}: {run.stderr.strip() or 'exit ' + str(run.returncode)}")
    return run.stdout


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
    # неизвестная точка отсчёта дала бы «ничего не менялось» и зелёный итог
    git("rev-parse", "--verify", "--quiet", a.since + "^{commit}")
    diff = git("diff", "--relative", a.since, "--", "src")
    in_diff = {line[6:] for line in diff.splitlines() if line.startswith("+++ b/src/")}
    # новый файл, которого git ещё не знает, в diff не попадает - его мутанты берутся по всему файлу
    untracked = {name for name in git("ls-files", "--others", "--exclude-standard", "--", "src").split("\n") if name.endswith(".rs")}
    sources = sorted(in_diff | untracked)
    if a.file:
        unknown = [f for f in a.file if f not in sources]
        if unknown:
            print(f"MUTANTS: FAIL - not among the files changed since {a.since}: {', '.join(unknown)} (changed: {', '.join(sources) or 'none'})")
            return 1
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
        base = ["cargo", "mutants", "-f", source, *([] if source in untracked else ["--in-diff", str(patch)])]
        whole = " (a new file: all of it)" if source in untracked else ""
        if a.list:
            listed = subprocess.run([*base, "--list"], cwd=RUST, env=env, capture_output=True, text=True).stdout
            rows.append((source, f"{len([m for m in listed.splitlines() if m.strip()])} mutants{whole}, checks: {' '.join(checks_of(source)) or 'ALL (no test target of its own)'}"))
            continue
        where = out / source.replace("/", "_")
        only = [arg for t in checks_of(source) for arg in ("-C=--test", f"-C={t}")]
        command = [*base, "-j", str(a.j), *only, "-o", str(where), *(["--iterate"] if a.iterate else [])]
        print(f"... {source}{whole} against {' '.join(checks_of(source)) or 'all checks'}", flush=True)
        run = subprocess.run(command, cwd=RUST, env=env, capture_output=True, text=True)
        log = out / (source.replace("/", "_") + ".log")
        log.write_text(run.stdout + run.stderr, encoding="utf-8")
        results = where / "mutants.out"
        if run.returncode == 0 and not results.exists():
            # в изменённых строках нечего подменять (комментарий, use, константа) - cargo-mutants ничего не запускает
            rows.append((source, "no mutants in the changed lines"))
            continue
        if run.returncode not in (0, 2, 3) or not results.exists():
            # 2 - есть выжившие, 3 - зависшие; прочее - прогон, который ничего не проверил (сломанная основа, нет места)
            rows.append((source, f"RUN FAILED (exit {run.returncode}) - see {log}"))
            missed_total.append(f"{source}: the run failed")
            continue
        caught, missed, timeout, unviable = (lines(results / f"{kind}.txt") for kind in ("caught", "missed", "timeout", "unviable"))
        # зависший мутант считается пойманным (проверка заметила тем, что не кончилась) - как у прогона C#
        rows.append((source, f"{len(caught) + len(timeout)} of {len(caught) + len(timeout) + len(missed)} caught" + (f", {len(unviable)} don't build" if unviable else "")))
        missed_total += missed

    for source, text in rows:
        print(f"{source:34} {text}")
    for m in missed_total:
        print("  SURVIVED " + m)
        # строка, в которую подложена ошибка, - то, что нужно тестировщику для недостающей проверки
        where = m.split(":")
        if len(where) > 2 and where[1].isdigit() and (RUST / where[0]).is_file():
            source = (RUST / where[0]).read_text(encoding="utf-8").splitlines()
            if int(where[1]) <= len(source):
                print("           | " + source[int(where[1]) - 1].strip())
    print(f"MUTANTS: {'FAIL - ' + str(len(missed_total)) + ' survived' if missed_total else 'OK'} (results: {out})")
    return 1 if missed_total else 0


if __name__ == "__main__":
    sys.exit(main())
