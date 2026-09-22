// Провал Debug.Assert роняет процесс с текстом проверки и ненулевым кодом (134 на Linux) — это и есть «тест упал».
#if !DEBUG
// [Conditional("DEBUG")] выкидывает проверки из Release целиком: здесь было бы «OK» без единой проверки.
Console.Error.WriteLine("SelfCheck запускается только в Debug: dotnet run --project TicketBoard.SelfCheck");
return 1;
#else
using TicketBoard.Services;

if (args is ["bridge", var key])
{
    // Для page/run.sh: настоящий мост на 127.0.0.1:47822 и настоящий клиент Интрасервиса против поддельного сервера
    // (page/fake-intraservice.mjs на 47899) и доски из одной карточки.
    var settings = new AppSettings { IntraserviceBaseUrl = "http://127.0.0.1:47899" };
    var client = new HttpIntraserviceClient(settings.IntraserviceBaseUrl, "user", "pass");
    IReadOnlyList<BridgeCard> board = new[]
    {
        new BridgeCard(702180, "Принтер в бухгалтерии", "В работе", "высокий", "Открыта", 4, settings.TicketUrl(702180),
            "Не печатает", new[] { new BridgeNote(DateTimeOffset.Now, "Звонил Петровой") }),
    };
    using var bridge = new ClaudeBridge(47822, key, settings, () => client, () => Task.FromResult(board), "0.0.0-check");
    bridge.Start();
    Console.WriteLine("bridge ready on 47822");
    await Task.Delay(Timeout.Infinite);
}

HttpIntraserviceClient.SelfCheck();
IntraserviceLinkParser.SelfCheck();
ClaudeBridge.SelfCheck();
Console.WriteLine("SelfCheck: OK");
return 0;
#endif
