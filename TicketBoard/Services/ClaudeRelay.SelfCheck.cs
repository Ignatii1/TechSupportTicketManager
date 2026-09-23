using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text;

namespace TicketBoard.Services;

public static partial class ClaudeRelay
{
    /// <summary>Самопроверка: разбор блока «TB …» и живой прогон — настоящий клиент Интрасервиса против поддельного сервера
    /// на loopback (формы ответов — как в HttpIntraserviceClient.SelfCheck). Запускается из TicketBoard.SelfCheck.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        var reqs = Parse("TB search «принтер бухгалтерия»\r\n\r\ntb TICKET #702180\nTB history 702180\nTB board\nTB frob 1\nTB ticket abc");
        Debug.Assert(reqs is { Count: 6 } && reqs[0] is { Verb: RelayVerb.Search, Query: "принтер бухгалтерия" }
            && reqs[1] is { Verb: RelayVerb.Ticket, Id: 702180 } && reqs[2] is { Verb: RelayVerb.History, Id: 702180 }
            && reqs[3].Verb == RelayVerb.Board && reqs[4].Verb == RelayVerb.Invalid && reqs[5].Verb == RelayVerb.Invalid);
        Debug.Assert(Parse("```\nTB board\n```") is { Count: 1 });                   // выделили руками вместе с оградой
        Debug.Assert(Parse("TB board\nи ещё текст") is null);                         // не только запросы — не наш буфер
        Debug.Assert(Parse("просто текст") is null && Parse("") is null && Parse("TB") is null && Parse("TBsearch x") is null);
        Debug.Assert(Parse("TB board " + new string('x', 20_000)) is null);           // огромный буфер не разбираем
        Debug.Assert(Parse("TB search " + new string('x', 201)) is [{ Verb: RelayVerb.Invalid }]);

        var (listener, port) = FakeIntraservice();
        try
        {
            var settings = new AppSettings { IntraserviceBaseUrl = $"http://127.0.0.1:{port}" };
            var client = new HttpIntraserviceClient(settings.IntraserviceBaseUrl, "user", "pass");
            BoardCard[] cards =
            {
                new(702180, "Принтер в бухгалтерии", "В работе", "Высокий", "Открыта", 4, null, settings.TicketUrl(702180),
                    "Не печатает", new[] { new BoardNote(DateTimeOffset.Now, "Звонил Петровой") }),
                new(6, "Старая", "Готово", "Низкий", "Выполнена", 40, DateTimeOffset.Now.AddDays(-40), "", "", Array.Empty<BoardNote>()),
                new(7, "Вчерашняя", "Готово", "Низкий", "Выполнена", 1, DateTimeOffset.Now.AddDays(-1), "", "", Array.Empty<BoardNote>()),
            };
            Task<IReadOnlyList<BoardCard>> Board(int? id) =>
                Task.FromResult<IReadOnlyList<BoardCard>>(cards.Where(c => id is null || c.Id == id).ToArray());

            var answer = RunAsync(Parse("TB search принтер\nTB ticket 702180\nTB history 702180\nTB board\nTB ticket 404404\nTB frob")!,
                client, Board, settings).GetAwaiter().GetResult();
            Debug.Assert(answer.StartsWith("TicketBoard → Claude · ") && answer.Contains(" · запросов: 6\n"));
            Debug.Assert(answer.Contains("### TB search принтер\nНайдено: 2, показаны 2 самых свежих.\n" +
                "- #702180 · Открыта · 18.09.2026 · Петрова А. — Принтер в бухгалтерии не печатает\n" +
                "  HP LaserJet 400 пишет «Замятие бумаги»\n" +
                $"  http://127.0.0.1:{port}/Task/View/702180\n"));
            Debug.Assert(answer.Contains("### TB ticket 702180\n#702180 · Открыта — Принтер в бухгалтерии не печатает\n"));
            Debug.Assert(answer.Contains("Описание:\nHP LaserJet 400\nЗамятие\n"));
            Debug.Assert(answer.Contains("На доске пользователя: колонка «В работе», приоритет Высокий, дней в колонке: 4.\n")
                && answer.Contains("Звонил Петровой"));
            Debug.Assert(answer.Contains(" · Сидоров · статус «Открыта» · внутренний\n  Почистил ролик — не помогло\n  второй строкой\n"));
            Debug.Assert(answer.Contains(" · Администратор · статус «Выполнена» · смена статуса\n"));
            Debug.Assert(answer.Contains("Карточек: 2 (ещё 1 в «Готово» старше 7 дн. не показаны") && !answer.Contains("Старая")
                && answer.Contains("\nГотово:\n- #7 · Низкий · 1 дн. · в Интрасервисе «Выполнена» — Вчерашняя\n"));
            Debug.Assert(answer.Contains("### TB ticket 404404\nОшибка: заявка не найдена (HTTP 404)"));
            Debug.Assert(answer.Contains("### TB frob\nНе понял запрос."));
            Debug.Assert(answer.EndsWith("— конец ответа TicketBoard. Нужно ещё — новый блок с запросами TB."));
            Debug.Assert(Parse(answer) is null);   // наш же ответ в буфере не запускает новый круг

            // без API: поиск честно говорит, что не настроен; карточка с доски всё равно видна
            var offline = RunAsync(Parse("TB search x\nTB ticket 702180")!, null, Board, settings).GetAwaiter().GetResult();
            Debug.Assert(offline.Contains("### TB search x\nAPI Интрасервиса не настроен")
                && offline.Contains("На доске пользователя: «Принтер в бухгалтерии» — колонка «В работе»"));

            // больше десяти — выполняются первые десять, и это сказано
            var many = RunAsync(Parse(string.Join("\n", Enumerable.Repeat("TB board", 12)))!, null, Board, settings).GetAwaiter().GetResult();
            Debug.Assert(many.Contains("запросов: 10 (выполнены первые 10 из 12)"));
        }
        finally { listener.Stop(); }
    }

    /// <summary>Поддельный Интрасервис: по одному соединению за раз, ответ целиком и Connection: close.</summary>
    private static (TcpListener Listener, int Port) FakeIntraservice()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        _ = Task.Run(async () =>
        {
            while (true)
            {
                TcpClient connection;
                try { connection = await listener.AcceptTcpClientAsync(); }
                catch (Exception) { return; }   // Stop() в конце самопроверки
                using (connection)
                {
                    var stream = connection.GetStream();
                    var head = new StringBuilder();
                    var buf = new byte[4096];
                    while (!head.ToString().Contains("\r\n\r\n"))
                    {
                        var read = await stream.ReadAsync(buf);
                        if (read == 0) break;
                        head.Append(Encoding.ASCII.GetString(buf, 0, read));
                    }
                    var target = head.ToString().Split(' ') is { Length: > 1 } parts ? parts[1] : "";
                    var (code, json) = FakeResponse(target);
                    var body = Encoding.UTF8.GetBytes(json);
                    await stream.WriteAsync(Encoding.ASCII.GetBytes(
                        $"HTTP/1.1 {code} X\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {body.Length}\r\nConnection: close\r\n\r\n"));
                    await stream.WriteAsync(body);
                }
            }
        });
        return (listener, ((IPEndPoint)listener.LocalEndpoint).Port);
    }

    private static (int Code, string Json) FakeResponse(string target)
    {
        // справочник статусов — одинаковый во всех ответах; подставляется по метке, чтобы не спорить со скобками JSON
        static string With(string json) => json.Replace("STATUSES", """[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}]""");
        if (target.StartsWith("/api/task?"))
            return (200, With("""
                {"Tasks":[
                  {"Id":702180,"Name":"Принтер в бухгалтерии не печатает","StatusId":31,"Created":"18.09.2026 09:12:00","Creator":"Петрова А.",
                   "Description":"<p>HP LaserJet 400 пишет &laquo;Замятие бумаги&raquo;</p>"},
                  {"Id":690001,"Name":"Принтер HP в бухгалтерии — замятие","StatusId":29,"Created":"02.03.2026 11:00:00","Creator":"Иванов И."}],
                 "Statuses":STATUSES,"Paginator":{"Count":2,"Page":1,"PageCount":1}}
                """));
        if (target.StartsWith("/api/task/702180"))
            return (200, With("""{"Task":{"Id":702180,"Name":"Принтер в бухгалтерии не печатает","Description":"<p>HP LaserJet 400<br/>Замятие</p>","StatusId":31},"Statuses":STATUSES}"""));
        if (target.StartsWith("/api/tasklifetime?taskid=702180"))
            return (200, With("""
                {"TaskLifetimeList":{"TaskLifetimes":[
                  {"Date":"19.09.2026 10:00:00","Editor":"Сидоров","StatusId":31,"Comments":"<p>Почистил ролик — не помогло</p><p>второй строкой</p>","IsPublic":"False"},
                  {"Date":"18.09.2026 09:12:00","Editor":"Администратор","StatusId":29}],
                 "Statuses":STATUSES,"Paginator":{"Count":2,"Page":1,"PageCount":1}}}
                """));
        return (404, """{"Message":"not found"}""");
    }
}
