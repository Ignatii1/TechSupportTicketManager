#!/usr/bin/env python3
"""Проверка разметки без запуска WPF (на Linux окно не запускается): ключи ресурсов и пути привязок.

Что ломается только при открытии окна на Windows и не видно при сборке:
  - {StaticResource X}, которого нет ни в самом файле, ни в App.xaml, ни в Themes/*.xaml, — окно падает при создании.
    Ресурсы окна, куда вложен UserControl, его StaticResource не видит (они ищутся при загрузке его собственного XAML),
    поэтому считаются только свои ресурсы файла и ресурсы приложения. Свой ключ должен быть объявлен выше, чем
    используется: StaticResource вперёд не смотрит (DynamicResource — смотрит);
  - {Binding Путь}, которого нет у объекта данных, — тихо ничего не показывает и не работает.

Путь проверяется по звеньям: SelectedTicket.Title — свойство Title у типа свойства SelectedTicket (пока тип известен:
класс из исходников; коллекции и чужие типы — дальше не проверяются). Объект данных: вне шаблонов — viewmodel из
аргумента; в <DataTemplate DataType="{x:Type p:Класс}"> — этот класс; в шаблоне без DataType — класс строки из
аргумента, если дан, иначе не проверяется. {Binding DataContext.X, RelativeSource=…} — X у viewmodel; прочие привязки к
элементам (RelativeSource, ElementName, Source) не проверяются. Члены классов — из исходников проекта, все части
partial-класса: открытые свойства (internal привязка не видит), [ObservableProperty] (поле _fooBar → FooBar),
[RelayCommand] (метод Foo или FooAsync → FooCommand), параметры позиционного record, члены своих базовых классов.

Запуск (из корня репозитория):
    python3 TicketBoard.SelfCheck/xamlcheck.py TicketBoard/Views/SearchWindow.xaml SearchViewModel FoundTicketViewModel
    python3 TicketBoard.SelfCheck/xamlcheck.py TicketBoard/Views/MainWindow.xaml MainViewModel
    python3 TicketBoard.SelfCheck/xamlcheck.py --selftest      (сама проверка на образце с подложенными ошибками)
Прежняя форма с файлом viewmodel вторым аргументом тоже принимается (файл не нужен: классы ищутся по проекту).
Код возврата 1 — есть неизвестные ключи или привязки.
"""
import glob
import os
import re
import sys
import tempfile

TYPE = r"[\w<>\?,\.\[\]]+"
MODIFIERS = r"(?:(?:static|virtual|override|required|new|sealed|abstract|readonly|partial)\s+)*"


class Project:
    """Исходники C# проекта и члены их классов: имя → тип (строкой, None — команда или неизвестно)."""

    def __init__(self, root: str):
        self.sources = []
        for path in glob.glob(os.path.join(root, "**", "*.cs"), recursive=True):
            parts = path.split(os.sep)
            if "obj" not in parts and "bin" not in parts:
                with open(path, encoding="utf-8") as f:
                    self.sources.append(f.read())
        self._members: dict[str, dict[str, str | None] | None] = {}

    def members(self, cls: str) -> dict[str, str | None] | None:
        if cls in self._members:
            return self._members[cls]
        self._members[cls] = None   # от зацикливания на базовых классах
        found, names, bases = False, {}, []
        decl = re.compile(r"\b(?:class|record|struct)\s+" + re.escape(cls) + r"\b")
        for src in self.sources:
            for m in decl.finditer(src):
                found = True
                rest = src[m.end():]
                nxt = re.search(r"\n\s*(?:(?:public|internal|private|protected|file)\s+)*(?:static\s+|sealed\s+|abstract\s+|partial\s+)*"
                                r"(?:class|record|struct|enum|interface)\s+\w", rest)
                body = rest[: nxt.start()] if nxt else rest
                if params := re.match(r"\s*\(", body):         # позиционный record: его параметры — свойства
                    depth, i = 0, params.end() - 1
                    for i in range(params.end() - 1, len(body)):
                        depth += {"(": 1, ")": -1}.get(body[i], 0)
                        if depth == 0:
                            break
                    for param in split_top(body[params.end():i]):
                        if pm := re.match(r"\s*(?:\[[^\]]*\]\s*)*(" + TYPE + r")\s+(\w+)", param):
                            names[pm.group(2)] = pm.group(1)
                header = re.match(r"[^{;]*", body).group(0)
                if bm := re.search(r":\s*([\w\.]+)", header):
                    bases.append(bm.group(1).split(".")[-1])
                for t, n in re.findall(r"public\s+" + MODIFIERS + "(" + TYPE + r")\s+(\w+)\s*(?:\{|=>)", body):
                    names[n] = t
                for t, n in re.findall(r"\[ObservableProperty[^\]]*\]\s*(?:\[[^\]]*\]\s*)*private\s+(" + TYPE + r")\s+_?(\w+)\s*[;=]", body):
                    names[n[:1].upper() + n[1:]] = t
                for n in re.findall(r"\[RelayCommand[^\]]*\]\s*(?:\[[^\]]*\]\s*)*(?:(?:private|public|internal|protected)\s+)?"
                                    r"(?:async\s+)?(?:Task|void|Task<[^>]+>|ValueTask)\s+(\w+)\s*\(", body):
                    names[re.sub(r"Async$", "", n) + "Command"] = None
        if not found:
            return None
        for b in bases:
            for n, t in (self.members(b) or {}).items():
                names.setdefault(n, t)
        self._members[cls] = names
        return names


def split_top(text: str) -> list[str]:
    """Части через запятую верхнего уровня: запятые внутри <…>, (…), {…} не делят."""
    parts, depth, start = [], 0, 0
    for i, ch in enumerate(text):
        depth += {"<": 1, "(": 1, "{": 1, ">": -1, ")": -1, "}": -1}.get(ch, 0)
        if ch == "," and depth == 0:
            parts.append(text[start:i])
            start = i + 1
    parts.append(text[start:])
    return parts


def type_class(t: str | None) -> str | None:
    """Класс, у которого искать следующее звено пути: Ticket? → Ticket; коллекции и прочее обобщённое — не дальше."""
    if not t or "<" in t or "[" in t:
        return None
    return t.rstrip("?").split(".")[-1]


def bindings(x: str):
    """Каждая {Binding …}: (позиция, путь, остальные аргументы)."""
    for m in re.finditer(r"\{Binding\b", x):
        depth, i = 0, m.start()
        for i in range(m.start(), len(x)):
            depth += {"{": 1, "}": -1}.get(x[i], 0)
            if depth == 0:
                break
        args = [a.strip() for a in split_top(x[m.end():i])]
        path = next((a[5:].strip() for a in args if a.startswith("Path=")), None)
        if path is None and args and args[0] and "=" not in args[0]:
            path = args[0]
        named = {a.split("=", 1)[0].strip() for a in args if "=" in a}
        yield m.start(), path or "", named


def templates(x: str):
    """Области <DataTemplate>: (начало, конец, класс из DataType или None, вложен ли в другой шаблон)."""
    out, stack = [], []
    for m in re.finditer(r"<DataTemplate\b([^>]*)>|</DataTemplate>", x):
        if m.group(0).startswith("</"):
            if stack:
                start, cls, nested = stack.pop()
                out.append((start, m.end(), cls, nested))
        elif not m.group(1).rstrip().endswith("/"):
            dt = re.search(r'DataType="\{x:Type\s+(?:\w+:)?(\w+)\}"', m.group(1))
            stack.append((m.start(), dt.group(1) if dt else None, bool(stack)))
    return out


def app_root(xaml_path: str) -> str:
    """Папка проекта: ближайшая вверх, где лежит App.xaml."""
    d = os.path.dirname(os.path.abspath(xaml_path))
    while not os.path.exists(os.path.join(d, "App.xaml")):
        parent = os.path.dirname(d)
        if parent == d:
            sys.exit(f"App.xaml не найден выше {xaml_path}")
        d = parent
    return d


def check(xaml_path: str, vm_class: str, row_class: str | None = None) -> tuple[list[str], list[tuple[str, str]]]:
    """(неизвестные ключи ресурсов, [(класс, неизвестное звено пути)])."""
    root = app_root(xaml_path)
    with open(xaml_path, encoding="utf-8") as f:
        x = f.read()

    app_keys = set()
    for path in [os.path.join(root, "App.xaml")] + glob.glob(os.path.join(root, "Themes", "*.xaml")):
        with open(path, encoding="utf-8") as f:
            app_keys |= set(re.findall(r'x:Key="([^"]+)"', f.read()))
    own = {}
    for m in re.finditer(r'x:Key="([^"]+)"', x):
        own.setdefault(m.group(1), m.start())
    missing = set()
    for m in re.finditer(r"\{(Static|Dynamic)Resource\s+([\w\.]+)\}", x):
        kind, key = m.groups()
        if key in app_keys:
            continue
        if key not in own:
            missing.add(key)
        elif kind == "Static" and own[key] > m.start():
            missing.add(f"{key} (объявлен ниже, чем используется)")

    project = Project(root)
    if project.members(vm_class) is None:
        sys.exit(f"класс {vm_class} не найден в исходниках {root}")
    regions = templates(x)
    bad = set()
    for pos, path, named in bindings(x):
        if not path or named & {"ElementName", "Source"}:
            continue
        if "RelativeSource" in named:
            if not path.startswith("DataContext."):
                continue
            cls, path = vm_class, path[len("DataContext."):]
        else:
            inner = [r for r in regions if r[0] < pos < r[1]]
            region = max(inner, key=lambda r: r[0]) if inner else None
            cls = vm_class if region is None else region[2] or (row_class if not region[3] else None)
        for step in re.split(r"\.", path):
            name = re.sub(r"\[.*?\]", "", step).strip("()")
            if not cls or not name:
                break
            known = project.members(cls)
            if known is None:
                break
            if name not in known:
                bad.add((cls, name))
                break
            cls = type_class(known[name])
    return sorted(missing), sorted(bad)


SELFTEST_VM = """
public partial class Vm : ObservableObject
{
    [ObservableProperty] private Item? _selected;
    public string Title { get; set; } = "";
    [RelayCommand] private async Task SaveAsync() { await Task.Yield(); }
}
public partial class Vm
{
    public ObservableCollection<Item> Items { get; } = new();
}
public sealed record Item(string Name, int Count)
{
    public Sub Child => new();
}
public class Sub
{
    public string Deep { get; set; } = "";
    internal string Hidden { get; set; } = "";
}
"""

SELFTEST_VIEW = """<UserControl x:Class="T.View" xmlns:vm="clr-namespace:T">
    <UserControl.Resources>
        <Style x:Key="Local" />
        <DataTemplate x:Key="Row" DataType="{x:Type vm:Item}">
            <StackPanel>
                <TextBlock Text="{Binding Name}" />
                <TextBlock Text="{Binding Title}" />
                <Button Command="{Binding DataContext.SaveCommand, RelativeSource={RelativeSource AncestorType=Window}}" />
                <Button Command="{Binding DataContext.Nope, RelativeSource={RelativeSource AncestorType=Window}}" />
            </StackPanel>
        </DataTemplate>
    </UserControl.Resources>
    <StackPanel Style="{StaticResource Local}">
        <TextBlock Text="{Binding Title}" Style="{StaticResource AppStyle}" Foreground="{DynamicResource Brush1}" />
        <TextBlock Text="{Binding Path=Title, Mode=OneWay}" />
        <TextBlock Text="{Binding Selected.Name, Converter={StaticResource Local}}" />
        <TextBlock Text="{Binding Selected.Child.Deep}" />
        <Button Command="{Binding SaveCommand}" />
        <ItemsControl ItemsSource="{Binding Items}" ItemTemplate="{StaticResource Row}" />
        <TextBlock Text="{Binding Titel}" />
        <TextBlock Text="{Binding Selected.Nmae}" />
        <TextBlock Text="{Binding Selected.Child.Hidden}" />
        <Button Command="{Binding Save}" />
        <Border Visibility="{Binding IsMouseOver, RelativeSource={RelativeSource AncestorType=Border}}" />
        <TextBlock Text="{Binding Text, ElementName=Box}" />
        <TextBlock Style="{StaticResource Missing}" />
        <TextBlock Style="{StaticResource Later}" Background="{DynamicResource AlsoLater}" />
        <StackPanel.Resources><Style x:Key="Later" /><SolidColorBrush x:Key="AlsoLater" /></StackPanel.Resources>
    </StackPanel>
</UserControl>
"""


def selftest() -> int:
    """Образец с заранее известными ошибками: найдено должно быть ровно то, что подложено, — ни больше, ни меньше."""
    with tempfile.TemporaryDirectory() as root:
        os.makedirs(os.path.join(root, "Themes"))
        os.makedirs(os.path.join(root, "Views"))
        files = {
            "App.xaml": '<Application><Application.Resources><Style x:Key="AppStyle" /></Application.Resources></Application>',
            "Themes/T.xaml": '<ResourceDictionary><SolidColorBrush x:Key="Brush1" /></ResourceDictionary>',
            "Vm.cs": SELFTEST_VM,
            "Views/View.xaml": SELFTEST_VIEW,
        }
        for name, text in files.items():
            with open(os.path.join(root, name), "w", encoding="utf-8") as f:
                f.write(text)
        missing, bad = check(os.path.join(root, "Views", "View.xaml"), "Vm")
    want_missing = ["Later (объявлен ниже, чем используется)", "Missing"]
    want_bad = [("Item", "Nmae"), ("Item", "Title"), ("Sub", "Hidden"), ("Vm", "Nope"), ("Vm", "Save"), ("Vm", "Titel")]
    ok = missing == want_missing and sorted(bad) == want_bad
    print("xamlcheck --selftest:", "OK" if ok else f"НЕ ТО\n  ключи: {missing}\n  ждали: {want_missing}\n  привязки: {sorted(bad)}\n  ждали: {want_bad}")
    return 0 if ok else 1


def main() -> int:
    args = sys.argv[1:]
    if args == ["--selftest"]:
        return selftest()
    if len(args) > 1 and args[1].endswith(".cs"):     # прежняя форма: окно, файл viewmodel, класс[, класс строки]
        args = [args[0]] + args[2:]
    if len(args) not in (2, 3):
        print(__doc__)
        return 2
    missing, bad = check(*args)
    print(f"{args[0]}: неизвестных ключей ресурсов: {missing or 'нет'}")
    print(f"{args[0]}: неизвестных привязок: {[f'{c}.{n}' for c, n in bad] or 'нет'}")
    return 1 if missing or bad else 0


if __name__ == "__main__":
    sys.exit(main())
