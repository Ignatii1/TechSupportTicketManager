// Провал Debug.Assert роняет процесс с текстом проверки и ненулевым кодом (134 на Linux) — это и есть «тест упал».
#if !DEBUG
// [Conditional("DEBUG")] выкидывает проверки из Release целиком: здесь было бы «OK» без единой проверки.
Console.Error.WriteLine("SelfCheck запускается только в Debug: dotnet run --project TicketBoard.SelfCheck");
return 1;
#else
TicketBoard.Services.HttpIntraserviceClient.SelfCheck();
TicketBoard.Services.IntraserviceLinkParser.SelfCheck();
Console.WriteLine("SelfCheck: OK");
return 0;
#endif
