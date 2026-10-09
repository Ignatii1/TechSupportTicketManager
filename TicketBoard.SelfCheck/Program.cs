// Провал Debug.Assert роняет процесс с текстом проверки и ненулевым кодом (134 на Linux) — это и есть «тест упал».
#if !DEBUG
// [Conditional("DEBUG")] выкидывает проверки из Release целиком: здесь было бы «OK» без единой проверки.
Console.Error.WriteLine("SelfCheck запускается только в Debug: dotnet run --project TicketBoard.SelfCheck");
return 1;
#else
// пауза перед проверкой последних записанных файлов (антивирус удаляет не сразу) — в проверках не нужна: удаляют они сами
TicketBoard.Services.KnowledgeExport.VanishWait = TimeSpan.FromMilliseconds(10);
TicketBoard.Services.HttpIntraserviceClient.SelfCheck();
TicketBoard.Services.IntraserviceLinkParser.SelfCheck();
TicketBoard.Services.ClaudeRelay.SelfCheck();
TicketBoard.Services.AutoSyncRules.SelfCheck();
TicketBoard.Services.AppSettings.SelfCheck();
TicketBoard.Services.TaskQuery.SelfCheck();
TicketBoard.Services.TicketSearch.SelfCheck();
TicketBoard.Services.KnowledgeExport.SelfCheck();
TicketBoard.SelfCheck.SearchWindowCheck.Run();
TicketBoard.SelfCheck.BoardCheck.Run();
TicketBoard.SelfCheck.CardCheck.Run();
Console.WriteLine("SelfCheck: OK");
return 0;
#endif
