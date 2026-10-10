// Провал Debug.Assert роняет процесс с текстом проверки и ненулевым кодом (134 на Linux) — это и есть «тест упал».
#if !DEBUG
// [Conditional("DEBUG")] выкидывает проверки из Release целиком: здесь было бы «OK» без единой проверки.
Console.Error.WriteLine("SelfCheck запускается только в Debug: dotnet run --project TicketBoard.SelfCheck");
return 1;
#else
// Области по именам: без аргументов — все, с именами — только они (быстрый круг TDD, мутанты). Лямбды, а не группы
// методов: у SelfCheck() сервисов [Conditional("DEBUG")], делегат на такой метод не создать.
(string Name, Action Run)[] areas =
[
    ("client", () => TicketBoard.Services.HttpIntraserviceClient.SelfCheck()),
    ("link", () => TicketBoard.Services.IntraserviceLinkParser.SelfCheck()),
    ("relay", () => TicketBoard.Services.ClaudeRelay.SelfCheck()),
    ("rules", () => TicketBoard.Services.AutoSyncRules.SelfCheck()),
    ("settings-file", () => TicketBoard.Services.AppSettings.SelfCheck()),
    ("query", () => TicketBoard.Services.TaskQuery.SelfCheck()),
    ("resolve", () => TicketBoard.Services.TicketSearch.SelfCheck()),
    ("export", () => TicketBoard.Services.KnowledgeExport.SelfCheck()),
    ("search", TicketBoard.SelfCheck.SearchWindowCheck.Run),
    ("board", TicketBoard.SelfCheck.BoardCheck.Run),
    ("card", TicketBoard.SelfCheck.CardCheck.Run),
    ("store", TicketBoard.SelfCheck.StoreCheck.Run),
    ("settings", TicketBoard.SelfCheck.SettingsCheck.Run),
];
if (args.Except(areas.Select(a => a.Name)).ToList() is [_, ..] unknown)
{
    Console.Error.WriteLine($"Нет таких областей: {string.Join(", ", unknown)}. Есть: {string.Join(", ", areas.Select(a => a.Name))}");
    return 2;
}
// пауза перед проверкой последних записанных файлов (антивирус удаляет не сразу) — в проверках не нужна: удаляют они сами
TicketBoard.Services.KnowledgeExport.VanishWait = TimeSpan.FromMilliseconds(10);
foreach (var (name, run) in areas)
    if (args.Length == 0 || args.Contains(name)) run();
// частичный прогон не должен выглядеть как полный
Console.WriteLine(args.Length == 0 ? "SelfCheck: OK" : $"SelfCheck: OK — только {string.Join(", ", args)}");
return 0;
#endif
