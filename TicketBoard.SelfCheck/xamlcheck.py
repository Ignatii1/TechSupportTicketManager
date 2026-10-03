#!/usr/bin/env python3
"""Проверка разметки окна без запуска WPF (на Linux окно не запускается): ключи ресурсов и пути привязок.

Что ломается только при открытии окна на Windows и не видно при сборке:
  - {StaticResource X} / {DynamicResource X}, которого нет ни в App.xaml, ни в Themes/*.xaml, ни в самом окне, —
    окно падает при создании;
  - {Binding Имя}, которого нет у viewmodel, — тихо ничего не показывает и не работает.

Запуск (из корня репозитория):
    python3 TicketBoard.SelfCheck/xamlcheck.py TicketBoard/Views/SearchWindow.xaml \\
        TicketBoard/ViewModels/SearchViewModel.cs SearchViewModel FoundTicketViewModel

Аргументы: окно; файл с viewmodel; класс, на который указывают привязки окна; (необязательно) класс строки — для привязок
внутри первого <DataTemplate>. Члены класса берутся из исходника: открытые свойства, [ObservableProperty] (поле _fooBar →
FooBar) и [RelayCommand] (метод Foo → FooCommand). Код возврата 1 — есть неизвестные ключи или привязки.
"""
import glob
import os
import re
import sys


def members(path: str, class_name: str) -> set[str]:
    src = open(path, encoding="utf-8").read()
    m = re.search(r"class\s+" + class_name + r"\b", src)
    if not m:
        sys.exit(f"класс {class_name} не найден в {path}")
    body = src[m.start():]
    nxt = re.search(r"\n(?:public|internal)\s+(?:sealed\s+)?(?:partial\s+)?class\s", body[10:])
    if nxt:
        body = body[: nxt.start() + 10]
    names = set(re.findall(r"public\s+(?:static\s+)?[\w<>\?,\.\[\]]+\s+(\w+)\s*(?:\{|=>)", body))
    names |= {n[1:2].upper() + n[2:] for n in re.findall(r"\[ObservableProperty[^\]]*\]\s*private\s+[\w<>\?,\.\[\]]+\s+(_\w+)", body, re.S)}
    names |= {n + "Command" for n in re.findall(r"\[RelayCommand[^\]]*\]\s*private\s+(?:async\s+)?(?:Task|void)\s+(\w+)\s*\(", body, re.S)}
    return names


def main() -> int:
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    xaml_path, vm_path, vm_class = sys.argv[1:4]
    row_class = sys.argv[4] if len(sys.argv) > 4 else None
    x = open(xaml_path, encoding="utf-8").read()

    # ресурсы приложения лежат рядом с проектом TicketBoard: App.xaml и Themes
    app_dir = os.path.join(os.path.dirname(os.path.abspath(xaml_path)), "..")
    defined = set(re.findall(r'x:Key="([^"]+)"', x))
    for f in [os.path.join(app_dir, "App.xaml")] + glob.glob(os.path.join(app_dir, "Themes", "*.xaml")):
        if os.path.exists(f):
            defined |= set(re.findall(r'x:Key="([^"]+)"', open(f, encoding="utf-8").read()))
    used = set(re.findall(r"\{(?:Static|Dynamic)Resource\s+([\w\.]+)\}", x))
    missing = sorted(k for k in used if k not in defined)

    vm_members = members(vm_path, vm_class)
    row_members = members(vm_path, row_class) if row_class else set()
    t0 = x.find("<DataTemplate")
    t1 = x.find("</DataTemplate>")
    bad = set()
    for m in re.finditer(r"\{Binding\s*([^,}\s]*)", x):
        path = m.group(1).strip()
        if not path or path.startswith("Path="):
            continue
        inside = row_class is not None and 0 <= t0 < m.start() < t1
        if path.startswith("DataContext."):          # команда окна из строки списка: RelativeSource к окну
            name, target, where = path.split(".")[1], vm_members, f"{vm_class} (через окно)"
        else:
            name = re.split(r"[.\[]", path)[0]
            target, where = (row_members, row_class) if inside else (vm_members, vm_class)
        if name and name not in target:
            bad.add((where, name))

    print(f"ресурсов в разметке: {len(used)}, неизвестных: {missing or 'нет'}")
    print(f"привязок к {vm_class}/{row_class or '-'}: неизвестных: {sorted(bad) or 'нет'}")
    return 1 if missing or bad else 0


if __name__ == "__main__":
    sys.exit(main())
