#!/usr/bin/env python3
"""Мутанты: подложенные ошибки, которые обязаны ловить самопроверки. Покрытие говорит, какие строки исполнялись;
мутант — заметит ли проверка, что строка стала неправильной. Выживший мутант — дыра в проверках.

Запуск из любой папки:
  python3 TicketBoard.SelfCheck/mutants/mutate.py                  все списки
  python3 TicketBoard.SelfCheck/mutants/mutate.py board card       только эти списки (имена файлов рядом, без .py)
  python3 TicketBoard.SelfCheck/mutants/mutate.py -k reopened      мутанты, в имени которых есть это
  python3 TicketBoard.SelfCheck/mutants/mutate.py -j 3             три копии исходников параллельно (по умолчанию 2)
  python3 TicketBoard.SelfCheck/mutants/mutate.py --check          только найти образцы в коде, без сборки (секунды)

Список — модуль рядом: AREAS (области SelfCheck, см. Program.cs) и MUTANTS — кортежи (имя, файл, было, стало[,
области]); «было» должно встречаться в файле ровно один раз. Рабочую папку скрипт не трогает: каждый поток подменяет
файлы в своей копии TicketBoard/ и TicketBoard.SelfCheck/ во временной папке, так что прерванный прогон не оставит
подменённый код (берётся рабочая папка как есть, с незакоммиченным). Сначала в каждой копии — прогон без подмены: не
зелёный — мерить нечего. Выживший в своих областях прогоняется ещё раз по всем: так видно, что его ловит другая область
(поправить AREAS списка), а не что его не ловит никто.
Код возврата: 0 — убиты все; 1 — есть выжившие, несобравшиеся или устаревшие (образец не найден) мутанты.
"""
import argparse
import concurrent.futures as cf
import importlib.util
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PROJECT = 'TicketBoard.SelfCheck'
DLL = Path(PROJECT, 'bin', 'Debug', 'net10.0', 'TicketBoard.SelfCheck.dll')
COPIED = ['TicketBoard', 'TicketBoard.SelfCheck']
ROOT_FILES = ['Directory.Build.props', 'Directory.Build.targets', 'Directory.Packages.props', 'global.json',
              'NuGet.config', 'nuget.config', '.editorconfig']
IGNORED = shutil.ignore_patterns('bin', 'obj', '.vs', '__pycache__', '*.user')


@dataclass(frozen=True)
class Mutant:
    list: str
    name: str
    path: str
    old: str
    new: str
    areas: tuple


@dataclass(frozen=True)
class Verdict:
    mutant: Mutant
    kind: str       # killed / crashed / timeout / survived / wrong-area / not-built
    reason: str

    @property
    def killed(self):
        return self.kind in ('killed', 'crashed', 'timeout', 'wrong-area')


SAYS = {'killed': 'убит проверкой', 'crashed': 'убит падением', 'timeout': 'убит зависанием',
        'wrong-area': 'ловит другая область', 'survived': 'ВЫЖИЛ', 'not-built': 'НЕ СОБРАЛСЯ'}


def load(names):
    files = sorted(p for p in HERE.glob('*.py') if p.name != Path(__file__).name)
    known = [p.stem for p in files]
    if unknown := [n for n in names if n not in known]:
        sys.exit(f'Нет таких списков: {", ".join(unknown)}. Есть: {", ".join(known)}')
    mutants = []
    for p in files:
        if names and p.stem not in names:
            continue
        spec = importlib.util.spec_from_file_location(f'mutants_{p.stem}', p)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for item in module.MUTANTS:
            name, path, old, new, *rest = item
            areas = tuple(rest[0]) if rest else tuple(module.AREAS)
            mutants.append(Mutant(p.stem, name, path, old, new, areas))
    names_seen = {}
    for m in mutants:
        if (m.list, m.name) in names_seen:
            sys.exit(f'{m.list}: имя мутанта повторяется — «{m.name}»')
        names_seen[(m.list, m.name)] = m
    return mutants


def stale(mutants):
    """Мутанты, чей образец не встречается в коде ровно один раз: код поменялся — список устарел."""
    out = []
    for m in mutants:
        f = ROOT / m.path
        count = f.read_text(encoding='utf-8').count(m.old) if f.exists() else -1
        if count != 1:
            out.append((m, 'файла нет' if count < 0 else f'образец найден {count} раз'))
    return out


def run(cmd, cwd, timeout):
    """(код, вывод) или (None, вывод) при превышении времени."""
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, r.stdout + r.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b'') + (e.stderr or b'')
        return None, out.decode('utf-8', 'replace') if isinstance(out, bytes) else out


def build(work):
    # предупреждения не роняют сборку мутанта: «if (false)» даёт «недостижимый код», а мерить надо проверки, не компилятор
    return run(['dotnet', 'build', PROJECT, '-c', 'Debug', '-v', 'q', '-nologo', '--no-restore',
                '-p:TreatWarningsAsErrors=false', '-p:WarningsAsErrors='], work, 900)


def selfcheck(work, areas, timeout):
    return run(['dotnet', str(DLL), *areas], work, timeout)


def first(out, *marks):
    lines = out.splitlines()
    for i, line in enumerate(lines):
        if any(mark in line for mark in marks):
            more = lines[i + 1].strip() if 'Assertion failed' in line and i + 1 < len(lines) else ''
            return (line.strip() + (' ' + more if more else ''))[:160]
    return ''


def judge(code, out):
    """Что сказал прогон SelfCheck: None — прошёл (мутант выжил)."""
    if code is None:
        return 'timeout', 'не уложился во время'
    if code == 0:
        return None
    if reason := first(out, 'не прошло:', 'Assertion failed'):
        return 'killed', reason
    return 'crashed', first(out, 'Unhandled exception', 'Exception') or (out.strip().splitlines() or [''])[-1][:160]


class Copy:
    """Копия исходников во временной папке: здесь подменяются файлы, здесь собирается и идёт SelfCheck."""

    def __init__(self):
        self.dir = Path(tempfile.mkdtemp(prefix='tb-mutants-'))
        for name in COPIED:
            shutil.copytree(ROOT / name, self.dir / name, ignore=IGNORED)
        for name in ROOT_FILES:
            if (ROOT / name).exists():
                shutil.copy2(ROOT / name, self.dir / name)

    def baseline(self, areas):
        code, out = run(['dotnet', 'build', PROJECT, '-c', 'Debug', '-v', 'q', '-nologo'], self.dir, 900)
        if code != 0:
            return None, 'без подмены не собирается:\n' + out[-3000:]
        started = time.monotonic()
        code, out = selfcheck(self.dir, areas, 900)
        if code != 0:
            return None, 'без подмены SelfCheck не зелёный — мерить нечего:\n' + out[-3000:]
        return time.monotonic() - started, ''

    def mutate(self, m, timeout, all_areas):
        f = self.dir / m.path
        orig = f.read_bytes()
        try:
            f.write_bytes(orig.replace(m.old.encode('utf-8'), m.new.encode('utf-8'), 1))
            code, out = build(self.dir)
            if code != 0:
                return Verdict(m, 'not-built', first(out, 'error') or out.strip()[-160:])
            judged = judge(*selfcheck(self.dir, m.areas, timeout))
            if judged:
                return Verdict(m, *judged)
            if set(m.areas) >= set(all_areas):
                return Verdict(m, 'survived', f'области: {", ".join(m.areas)}')
            judged = judge(*selfcheck(self.dir, [], timeout * 3))
            if judged:
                return Verdict(m, 'wrong-area', f'не в {", ".join(m.areas)}: {judged[1]}')
            return Verdict(m, 'survived', 'полный прогон тоже зелёный')
        finally:
            f.write_bytes(orig)

    def remove(self):
        shutil.rmtree(self.dir, ignore_errors=True)


AREAS_ALL = ['client', 'link', 'relay', 'rules', 'settings-file', 'query', 'resolve', 'export', 'search', 'board',
             'card', 'store', 'settings']


def main():
    ap = argparse.ArgumentParser(description='Мутанты для TicketBoard.SelfCheck (см. начало файла).')
    ap.add_argument('lists', nargs='*', help='списки (имена файлов рядом без .py); без них — все')
    ap.add_argument('-k', dest='substring', default='', help='только мутанты, в имени которых есть это')
    ap.add_argument('-j', dest='jobs', type=int, default=2, help='копий исходников параллельно (по умолчанию 2)')
    ap.add_argument('--check', action='store_true', help='только найти образцы в коде, без сборки')
    args = ap.parse_args()

    mutants = [m for m in load(args.lists) if args.substring in m.name]
    if bad := [a for m in mutants for a in m.areas if a not in AREAS_ALL]:
        sys.exit(f'Нет таких областей SelfCheck: {", ".join(sorted(set(bad)))} (см. Program.cs)')
    old = stale(mutants)
    for m, why in old:
        print(f'УСТАРЕЛ  {m.list} · {m.name}: {m.path} — {why}')
    todo = [m for m in mutants if m not in {o for o, _ in old}]
    if args.check:
        print(f'Образцы на месте: {len(todo)} из {len(mutants)}')
        return 1 if old else 0
    if not todo:
        print('Мутантов нет.')
        return 1 if old else 0

    jobs = max(1, min(args.jobs, len(todo)))
    areas = sorted({a for m in todo for a in m.areas}, key=AREAS_ALL.index)
    print(f'Мутантов: {len(todo)}, копий: {jobs}, области: {", ".join(areas)}', flush=True)
    copies = []
    try:
        with cf.ThreadPoolExecutor(jobs) as pool:
            copies = list(pool.map(lambda _: Copy(), range(jobs)))
            baselines = list(pool.map(lambda c: c.baseline(areas), copies))
        if errors := [e for t, e in baselines if t is None]:
            print(errors[0])
            return 1
        timeout = max(120, 4 * max(t for t, _ in baselines))
        print(f'Без подмены — зелёный ({max(t for t, _ in baselines):.0f} с); предел на мутанта {timeout:.0f} с', flush=True)

        pending = queue.Queue()
        for m in todo:
            pending.put(m)
        results, lock = [], threading.Lock()

        def work(copy):
            while True:
                try:
                    m = pending.get_nowait()
                except queue.Empty:
                    return
                v = copy.mutate(m, timeout, AREAS_ALL)
                with lock:
                    results.append(v)
                    mark = '' if v.kind in ('killed', 'crashed', 'timeout') else '   <<<<<<'
                    print(f'[{len(results)}/{len(todo)}] {m.list} · {m.name}: {SAYS[v.kind]}{mark} | {v.reason}', flush=True)

        with cf.ThreadPoolExecutor(jobs) as pool:
            list(pool.map(work, copies))
    finally:
        for c in copies:
            c.remove()

    killed = [v for v in results if v.killed]
    counted = [v for v in results if v.kind != 'not-built']
    print(f'\nУбито {len(killed)} из {len(counted)}' + (f' ({100 * len(killed) / len(counted):.0f}%)' if counted else ''))
    for kind in ('survived', 'not-built', 'wrong-area'):
        for v in (v for v in results if v.kind == kind):
            print(f'  {SAYS[kind]}: {v.mutant.list} · {v.mutant.name} — {v.reason}')
    if old:
        print(f'  устаревших (образец не найден): {len(old)}')
    return 0 if not old and all(v.killed for v in results) else 1


if __name__ == '__main__':
    sys.exit(main())
