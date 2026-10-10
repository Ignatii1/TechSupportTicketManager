#!/usr/bin/env python3
"""Покрытие кода приложения самопроверками: какие строки не исполняет ни одна проверка.

  python3 TicketBoard.SelfCheck/coverage.py                  сборка, SelfCheck под dotnet-coverage, таблица по файлам
  python3 TicketBoard.SelfCheck/coverage.py MainViewModel    ещё и номера неисполненных строк файлов, в пути которых это

Инструмент — dotnet-coverage из манифеста dotnet-tools.json в корне репозитория (ставится сам: dotnet tool restore);
пакетом приложения он не становится. Считаются исходники приложения (TicketBoard/), не сами проверки. Файлы, которых в
SelfCheck нет вовсе (окна, трей, хоткей — всё на типах WPF), перечислены отдельно: их строки здесь не измерить.
Покрытие говорит, что строка исполнялась, а не что её результат проверен, — это меряют мутанты (mutants/mutate.py).
"""
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / 'TicketBoard'
DLL = ROOT / 'TicketBoard.SelfCheck' / 'bin' / 'Debug' / 'net10.0' / 'TicketBoard.SelfCheck.dll'


def run(cmd):
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f'{" ".join(cmd[:4])}…: код {r.returncode}\n{(r.stdout + r.stderr)[-3000:]}')
    return r.stdout


def ranges(numbers):
    """1,2,3,7,9,10 → «1-3, 7, 9-10»."""
    out, start, prev = [], None, None
    for n in sorted(numbers) + [None]:
        if start is not None and (n is None or n != prev + 1):
            out.append(f'{start}' if start == prev else f'{start}-{prev}')
            start = None
        if n is not None and start is None:
            start = n
        prev = n
    return ', '.join(out)


def is_check(rel):
    """Сами проверки, лежащие среди исходников приложения: образцы *.SelfCheck*.cs и поддельный сервер."""
    return '.SelfCheck' in rel.name or rel.name == 'FakeIntraservice.cs'


def code_lines(path):
    """Строки кода файла без пустых и комментариев — для файлов, которых нет в SelfCheck."""
    return sum(1 for line in path.read_text(encoding='utf-8').splitlines()
               if line.strip() and not line.strip().startswith(('//', '///', '/*', '*')))


def main():
    show = sys.argv[1:]
    run(['dotnet', 'tool', 'restore'])
    run(['dotnet', 'build', 'TicketBoard.SelfCheck', '-c', 'Debug', '-v', 'q', '-nologo'])
    with tempfile.TemporaryDirectory() as tmp:
        report = Path(tmp) / 'coverage.xml'
        run(['dotnet', 'dotnet-coverage', 'collect', f'dotnet "{DLL}"', '-f', 'cobertura', '-o', str(report)])
        tree = ET.parse(report)

    lines = {}   # файл → {номер строки: исполнялась ли}
    unresolved = 0
    bases = [Path(src.text) for src in tree.iter('source') if src.text]   # от них Cobertura считает относительные пути
    for cls in tree.iter('class'):
        path = Path(cls.get('filename', ''))
        if not path.is_absolute():   # от баз отчёта; при совпадении в нескольких — та, что в приложении
            found = [c for c in [b / path for b in bases] + [ROOT / path] if c.exists()]
            if not found:
                unresolved += 1
                continue
            path = next((c for c in found if APP in c.resolve().parents), found[0])
        try:
            rel = path.resolve().relative_to(APP)
        except ValueError:
            continue
        if not rel.parts or rel.parts[0] in ('obj', 'bin') or is_check(rel):
            continue
        hits = lines.setdefault(str(Path('TicketBoard') / rel), {})
        for line in cls.iter('line'):
            n = int(line.get('number'))
            hits[n] = hits.get(n, False) or int(line.get('hits', '0')) > 0

    if unresolved:
        print(f'Внимание: путей из отчёта нет на диске — {unresolved}, эти файлы не посчитаны')
    rows = sorted(((f, sum(h.values()), len(h)) for f, h in lines.items()), key=lambda r: (r[1] - r[2], r[0]))
    if not rows:
        sys.exit('В отчёте покрытия нет ни одного файла приложения — сверьте пути в нём с папкой TicketBoard/')
    width = max(len(f) for f, _, _ in rows)
    print(f'{"файл":<{width}}  исполнено  из     %   не исполнено')
    for f, covered, total in rows:
        print(f'{f:<{width}}  {covered:>9}  {total:>5}  {100 * covered / total:>3.0f}  {total - covered:>12}')
    covered, total = sum(r[1] for r in rows), sum(r[2] for r in rows)
    print(f'{"всего":<{width}}  {covered:>9}  {total:>5}  {100 * covered / total:>3.0f}  {total - covered:>12}')

    measured = {(ROOT / f).resolve() for f in lines}
    outside = sorted(p for p in APP.rglob('*.cs') if p.parts[len(APP.parts)] not in ('obj', 'bin')
                     and not is_check(p.relative_to(APP)) and p.resolve() not in measured)
    if outside:
        print(f'\nНет в SelfCheck (на типах WPF), строк кода {sum(code_lines(p) for p in outside)}: '
              + ', '.join(str(p.relative_to(APP)) for p in outside))

    for pattern in show:
        for f, hits in sorted(lines.items()):
            if pattern in f:
                missed = [n for n, hit in hits.items() if not hit]
                print(f'\n{f}: не исполнены строки {ranges(missed) or "—"}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
