using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.CompilerServices;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace TicketBoard.Services;

/// <summary>Самопроверка разбора на образцах из документации. Запускается в Debug при старте приложения и командой dotnet run --project TicketBoard.SelfCheck (см. AGENTS.md).</summary>
public sealed partial class HttpIntraserviceClient
{
    /// <summary>ponytail: самопроверка разбора на образцах формы из документации (v5.42 и v5.51), только в Debug
    /// (вызов в App.OnStartup). Реальные ответы сервера появятся — добавить их сюда же.</summary>
    [Conditional("DEBUG")]
    internal static void SelfCheck()
    {
        Debug.Assert(Parse("""{"Task":{"Id":159,"Name":"Принтер","Description":"<p>a &laquo;b&raquo;</p><p>c<br/>d</p>","StatusId":31},"Statuses":[{"Id":31,"Name":"Открыта"}]}""", 1)
            == new IntraserviceTask(159, "Принтер", "Открыта", "a «b»\nc\nd"));
        Debug.Assert(Parse("""{"Id":162,"Name":"D","StatusName":"Выполнена"}""", 1) is { Id: 162, Status: "Выполнена", Description: null });
        Debug.Assert(Parse("""{"Task":{"Name":"C","StatusId":56}}""", 7) is { Id: 7, Status: "статус 56" });
        Debug.Assert(Parse("""{"Message":"The request is invalid."}""", 1) is null);
        // Кто подал и кто работает: исполнители строкой через запятую (как в веб-интерфейсе) — к одному виду; поля нет — null,
        // пусто или json null — "" (синхронизация отличает «не прислали» от «никого»). Changed — та же дата, что везде.
        Debug.Assert(Parse("""{"Task":{"Id":5,"Name":"N","Creator":" Сидоров С. ","Executors":"Иванов И. И.,Петров П.;  ","ExecutorGroup":"Первая линия","Changed":"26.09.2026 10:15:00"}}""", 5)
            is { Creator: "Сидоров С.", Executors: "Иванов И. И., Петров П.", ExecutorGroup: "Первая линия" } t5
            && t5.Changed == new DateTimeOffset(new DateTime(2026, 9, 26, 10, 15, 0)));
        Debug.Assert(Parse("""{"Id":5,"Name":"N","Executors":null,"ExecutorGroup":null}""", 5)
            is { Creator: null, Executors: "", ExecutorGroup: "", Changed: null, CreatorPhone: null, CreatorEmail: null });
        // как связаться с подавшим — строки как есть (без пробелов по краям); json null — «пусто»
        Debug.Assert(Parse("""{"Id":5,"Name":"N","CreatorPhone":" +7 (495) 123-45-67 ","CreatorEmail":null}""", 5)
            is { CreatorPhone: "+7 (495) 123-45-67", CreatorEmail: "" });
        // исполнители массивом — строками или объектами с Name
        Debug.Assert(Parse("""{"Id":5,"Name":"N","Executors":["Иванов",{"Id":2,"Name":"Петров"},{"Id":3},""]}""", 5)
            ?.Executors == "Иванов, Петров");

        // Жизненный цикл: пример из документации (стр. 65-66), переведённый в json. Первая запись — просто смена
        // статуса, без ключа Comments; во второй комментарий и признак «виден клиенту» строкой.
        var life = ParseLifetime("""
            {"TaskLifetimeList":{"TaskLifetimes":[
              {"Date":"12.11.2015 13:44:53","EditorId":43,"Editor":"Администратор","StatusId":29},
              {"Date":"11.11.2015 15:24:10","EditorId":43,"Editor":"Администратор","StatusId":31,"Comments":"<p>Проверьте, пожалуйста</p>","IsPublic":"True"}],
              "Statuses":[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}],
              "Paginator":{"Count":2,"Page":1,"PageCount":1,"PageSize":25,"CountOnPage":2}}}
            """);
        Debug.Assert(life is not null && !life.Value.HasMore && life.Value.Events.Count == 2);
        Debug.Assert(life?.Events[0] is { Author: "Администратор", Status: "Выполнена", Comment: null, IsPublic: null });
        Debug.Assert(life?.Events[0].Date == new DateTimeOffset(new DateTime(2015, 11, 12, 13, 44, 53)));
        Debug.Assert(life?.Events[1] is { Status: "Открыта", Comment: "Проверьте, пожалуйста", IsPublic: true, AuthorId: 43 });
        // Голый массив, дата в ISO, статуса 7 в ответе нет, пустой комментарий — это не комментарий.
        var bare = ParseLifetime("""[{"Date":"2015-10-29T13:51:14.023","Editor":"Иванов","StatusId":7,"Comments":"","IsPublic":false}]""");
        Debug.Assert(bare is not null && !bare.Value.HasMore && bare.Value.Events.Count == 1);
        Debug.Assert(bare?.Events[0] is { Author: "Иванов", Status: "статус 7", Comment: null, IsPublic: false, AuthorId: null });
        Debug.Assert(bare?.Events[0].Date == new DateTimeOffset(new DateTime(2015, 10, 29, 13, 51, 14, 23)));
        // Дата в формате WCF — миллисекунды от 1970 UTC, тот же момент, что и в примере выше.
        Debug.Assert(ParseLifetime("""[{"Date":"/Date(1447335893000)/","Editor":"Иванов","StatusId":7}]""")?.Events[0].Date
            == new DateTimeOffset(2015, 11, 12, 13, 44, 53, TimeSpan.Zero));
        // Пустая страница, но не последняя.
        var page2 = ParseLifetime("""{"TaskLifetimes":[],"Paginator":{"Page":2,"PageCount":3}}""");
        Debug.Assert(page2 is not null && page2.Value.HasMore && page2.Value.Events.Count == 0);
        Debug.Assert(ParseLifetime("""{"Message":"The request is invalid."}""") is null);

        // Поиск: форма ответа из документации (стр. 16-17) — Tasks + Statuses + Paginator. Строка без Name пропадает,
        // общее число берём из Paginator.Count, а не из длины списка.
        var found = ParseSearch("""
            {"Tasks":[{"Id":159,"Name":"Принтер","StatusId":31,"Created":"26.11.2015 16:18:06","Creator":"Администратор"},
              {"Id":160,"StatusId":31},
              {"Id":161,"Name":"Сканер","StatusName":"В работе"}],
              "Statuses":[{"Id":31,"Name":"Открыта"}],
              "Paginator":{"Count":137,"Page":1,"PageCount":7,"PageSize":20,"CountOnPage":3}}
            """);
        Debug.Assert(found is not null && found.Value.Total == 137 && found.Value.Found.Count == 2);
        Debug.Assert(found?.Found[0] is { Id: 159, Name: "Принтер", Status: "Открыта", Creator: "Администратор" });
        Debug.Assert(found?.Found[0].Created == new DateTimeOffset(new DateTime(2015, 11, 26, 16, 18, 6)));
        Debug.Assert(found?.Found[1] is { Id: 161, Status: "В работе", Creator: null, Executors: null, Changed: null });
        // строка списка несёт исполнителей и Changed — импорт и автообновление берут их отсюда, без запроса на заявку
        Debug.Assert(ParseSearch("""{"Tasks":[{"Id":7,"Name":"C","Executors":"Иванов","ExecutorGroup":"ИТ","Changed":"2026-09-26T10:15:00","CreatorEmail":"a@b.ru"}]}""")
            ?.Found[0] is { Executors: "Иванов", ExecutorGroup: "ИТ", Changed: not null, CreatorEmail: "a@b.ru", CreatorPhone: null });
        // Обёртка TaskList, Paginator'а нет: общее число — сколько пришло, статуса нет вовсе — пустая строка.
        var wrapped = ParseSearch("""{"TaskList":{"Tasks":[{"Id":7,"Name":"C"}]}}""");
        Debug.Assert(wrapped is not null && wrapped.Value.Total == 1 && wrapped.Value.Found[0].Status == "");
        Debug.Assert(ParseSearch("""{"Message":"The request is invalid."}""") is null);
        // Описание в списке — тот же html из редактора, что и в карточке: чистим его так же.
        Debug.Assert(ParseSearch("""{"Tasks":[{"Id":7,"Name":"C","Description":"<p>a &laquo;b&raquo;</p><p>c<br/>d</p>"}]}""")
            ?.Found[0].Description == "a «b»\nc\nd");

        // Текущий пользователь: пример из документации (стр. 57), переведённый в json. Корень там назван
        // <CurrenUserInfo> — буква «t» потеряна в самой документации, поэтому понимаем оба написания и голый объект.
        Debug.Assert(ParseCurrentUser("""
            {"CompanyId":30,"DefaultTaskFilterId":106,"Email":"test@test.ru","Id":1,"IsArchive":false,"Language":"ru",
             "Login":"admin","Name":"Администратор","RoleId":37,"RoleType":1,"UtcOffset":"+03:00"}
            """) == new IntraserviceUser(1, "Администратор"));
        Debug.Assert(ParseCurrentUser("""{"CurrenUserInfo":{"Id":1,"Login":"admin","Name":"Администратор","RoleType":1}}""")?.Id == 1);
        Debug.Assert(ParseCurrentUser("""{"CurrentUserInfo":{"Id":44,"Login":"test1"}}""") == new IntraserviceUser(44, ""));
        Debug.Assert(ParseCurrentUser("""{"Message":"The request is invalid."}""") is null);

        // Статусы: пример из документации (стр. 39), переведённый в json. Голый массив; признаки приходят и
        // булевыми, и строкой; строка без номера пропадает, остальные читаются.
        var statuses = ParseStatuses("""
            [{"Id":31,"Name":"Открыта","IsCommentRequired":false,"IsFinal":false,"IsFixed":false,"IsInitial":true},
             {"Id":29,"Name":"Выполнена","IsFixed":"True","IsFinal":"False"},
             {"Id":30,"Name":"Закрыта","IsFinal":true},
             {"Name":"Без номера","IsFixed":true}]
            """);
        Debug.Assert(statuses is { Count: 3 });
        Debug.Assert(statuses?[0] == new IntraserviceStatus(31, "Открыта", false, false));
        Debug.Assert(statuses?[1] == new IntraserviceStatus(29, "Выполнена", true, false));
        Debug.Assert(statuses?[2] is { Id: 30, Name: "Закрыта", IsFixed: false, IsFinal: true });
        // Обёртки: из xml-документации в лоб и «как блок Statuses в ответе по заявкам».
        Debug.Assert(ParseStatuses("""{"ArrayOfTaskStatusView":{"TaskStatusView":[{"Id":7,"Name":"Открыта"}]}}""") is { Count: 1 });
        Debug.Assert(ParseStatuses("""{"Statuses":[{"Id":7,"Name":"Открыта"}]}""") is { Count: 1 });
        Debug.Assert(ParseStatuses("""{"Message":"The request is invalid."}""") is null);

        // Сырой ответ в сообщении: json — как есть; html — видимым текстом, без стилей; длинное — обрезано.
        Debug.Assert(Evidence(""" {"Message":"The request is invalid."} """) == """{"Message":"The request is invalid."}""");
        var page = Evidence("<html><head><title>401 - Unauthorized</title><style>body{color:red}</style></head><body><p>Access denied</p></body></html>");
        Debug.Assert(page == "(html-страница, показан её текст)\n401 - Unauthorized\nAccess denied");
        Debug.Assert(Evidence("Authorization: Basic aXZhbm92Om15aXZhbm92MjM=") == "Authorization: Basic ***");
        Debug.Assert(Brief("сервер недоступен:\nNo such host is known\nещё") == "сервер недоступен — No such host is known");
        Debug.Assert(Brief("нет доступа (HTTP 403)") == "нет доступа (HTTP 403)" && Brief("") == "");
        Debug.Assert(Evidence("<Task><Id>1</Id></Task>") == "<Task><Id>1</Id></Task>");   // xml — не html, как есть
        Debug.Assert(Evidence(new string('x', 5000)).Length == ShownChars + 2);
    }
}
