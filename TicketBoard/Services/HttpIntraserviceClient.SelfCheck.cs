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
        // заглушка без названия — не статус: по ней не решаем «закрыта или открыта»
        Debug.Assert(!IsResolvedStatus(Parse("""{"Task":{"Name":"C","StatusId":56}}""", 7)!.Status));   // что парсер и пишет
        Debug.Assert(!IsResolvedStatus("статус 56") && !IsResolvedStatus("") && !IsResolvedStatus(null)
            && IsResolvedStatus("Открыта") && IsResolvedStatus("статус уточняется"));
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
        // Карточка с живого сервера (2026-10-02; форма ответа, значения заменены): блоки рядом с Task — null, статус
        // строкой, сервис — ServiceName, тип — Type (не TypeName), пустые категории и группа — json null.
        var live = Parse("""{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"Categories":null,"CategoryIds":null,"Changed":"2026-08-27T15:54:43.460865","Created":"2026-08-07T10:15:08.408534","Creator":"Петрова Анна","Description":"строка 1\r\nстрока 2","ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"11, 12","Executors":"Иванов И. И., Сидоров С. С.","Id":700001,"Name":"Склад: удалить задания","ResolutionDateFact":"2026-08-24T17:07:00","ServiceCode":"005","ServiceId":844,"ServiceName":"Приложение на ТСД","ServicePath":"840|844|","StatusId":28,"StatusIsFinal":true,"StatusName":"Закрыта","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}""", 1);
        Debug.Assert(live is { Id: 700001, Status: "Закрыта", Executors: "Иванов И. И., Сидоров С. С.", ExecutorGroup: "" });
        Debug.Assert(live?.Extra is { Service: "Приложение на ТСД", Type: "Запрос на обслуживание", Categories: "" } lx
            && lx.Resolved == new DateTimeOffset(new DateTime(2026, 8, 24, 17, 7, 0)));
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
        Debug.Assert(life is not null && !life.Value.HasMore && life.Value.Paged && life.Value.Events.Count == 2);
        Debug.Assert(life?.Events[0] is { Author: "Администратор", Status: "Выполнена", Comment: null, IsPublic: null });
        Debug.Assert(life?.Events[0].Date == new DateTimeOffset(new DateTime(2015, 11, 12, 13, 44, 53)));
        Debug.Assert(life?.Events[1] is { Status: "Открыта", Comment: "Проверьте, пожалуйста", IsPublic: true, AuthorId: 43 });
        // Голый массив, дата в ISO, статуса 7 в ответе нет, пустой комментарий — это не комментарий.
        var bare = ParseLifetime("""[{"Date":"2015-10-29T13:51:14.023","Editor":"Иванов","StatusId":7,"Comments":"","IsPublic":false}]""");
        Debug.Assert(bare is not null && !bare.Value.HasMore && !bare.Value.Paged && bare.Value.Events.Count == 1);   // без Paginator
        Debug.Assert(bare?.Events[0] is { Author: "Иванов", Status: "статус 7", Comment: null, IsPublic: false, AuthorId: null });
        Debug.Assert(bare?.Events[0].Date == new DateTimeOffset(new DateTime(2015, 10, 29, 13, 51, 14, 23)));
        // Дата в формате WCF — миллисекунды от 1970 UTC, тот же момент, что и в примере выше.
        Debug.Assert(ParseLifetime("""[{"Date":"/Date(1447335893000)/","Editor":"Иванов","StatusId":7}]""")?.Events[0].Date
            == new DateTimeOffset(2015, 11, 12, 13, 44, 53, TimeSpan.Zero));
        // Пустая страница, но не последняя.
        var page2 = ParseLifetime("""{"TaskLifetimes":[],"Paginator":{"Page":2,"PageCount":3}}""");
        Debug.Assert(page2 is not null && page2.Value.HasMore && page2.Value.Events.Count == 0);
        // Paginator без номеров страниц ничего не говорит о следующей — это не «страниц больше нет».
        var countOnly = ParseLifetime("""{"TaskLifetimes":[],"Paginator":{"Count":120}}""");
        Debug.Assert(countOnly is not null && !countOnly.Value.HasMore && !countOnly.Value.Paged);
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
        // для выгрузки: сервис, тип, категории (строкой или массивом) и фактическая дата решения; нет полей — null
        var extra = ParseSearch("""{"Tasks":[{"Id":7,"Name":"C","ServiceName":"Принтеры","Type":"Инцидент","Categories":["Печать","HP"],"ResolutionDateFact":"20.09.2026 16:40:00"},{"Id":8,"Name":"D"}]}""");
        Debug.Assert(extra?.Found[0].Extra is { Service: "Принтеры", Type: "Инцидент", Categories: "Печать, HP" } x7
            && x7.Resolved == new DateTimeOffset(new DateTime(2026, 9, 20, 16, 40, 0)));
        Debug.Assert(extra is not null && extra.Value.Found[1].Extra is null);
        // список с include=service: у строки только ServiceId — название берём из блока Services; ServiceName в строке главнее
        var withServices = ParseSearch("""{"Tasks":[{"Id":7,"Name":"C","ServiceId":844,"Type":"Инцидент"},{"Id":8,"Name":"D","ServiceId":99},{"Id":9,"Name":"E","ServiceId":844,"ServiceName":"Свой"}],"Services":[{"Id":844,"Name":"Приложение на ТСД"}],"Paginator":{"Count":3,"Page":1,"PageCount":1}}""");
        Debug.Assert(withServices?.Found[0].Extra is { Service: "Приложение на ТСД", Type: "Инцидент" });
        Debug.Assert(withServices?.Found[1].Extra is null && withServices?.Found[2].Extra is { Service: "Свой" });   // сервиса 99 в блоке нет
        // Справочники: сервисы в обёртке с Paginator, фильтры голым массивом, сотрудники без обёртки; без номера и названия — пропуск
        var services = ParseRefs("""{"ServiceList":{"Services":[{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|","IsArchive":false},{"Id":840,"Name":"ТСД","Path":"840|","IsArchive":"True"},{"Id":9,"Name":""},{"Name":"без номера"}],"Paginator":{"Count":3,"Page":1,"PageCount":2}}}""", "Services", "ServiceList");
        Debug.Assert(services is { Total: 3, HasMore: true } s0 && s0.Items.Count == 2
            && s0.Items[0] == new IntraserviceRef(844, "Приложение на ТСД", "840|844|") && s0.Items[1] is { Id: 840, IsArchive: true });
        var filters = ParseRefs("""[{"Id":106,"IsCommon":false,"IsDefault":true,"Name":"1. Инциденты"},{"Id":107,"IsDefault":false,"Name":"2. Проблемы"}]""", "FilterView", "ArrayOfFilterView");
        Debug.Assert(filters is { Total: 2, HasMore: false } f0 && f0.Items[0].IsDefault && !f0.Items[1].IsDefault);
        var users = ParseRefs("""{"Users":[{"Id":45,"Name":"Иванов И."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}""", "Users", "UserList");
        Debug.Assert(users is { Total: 1, HasMore: false } u0 && u0.Items[0].Name == "Иванов И.");
        Debug.Assert(ParseRefs("""{"Message":"The request is invalid."}""", "Users", "UserList") is null);
        // Обёртка TaskList, Paginator'а нет: общее число — сколько пришло, статуса нет вовсе — пустая строка.
        var wrapped = ParseSearch("""{"TaskList":{"Tasks":[{"Id":7,"Name":"C"}]}}""");
        Debug.Assert(wrapped is not null && wrapped.Value.Total == 1 && wrapped.Value.Found[0].Status == "");
        // Счёт не заказан (count=false): вместо Count — HasNextPage, в Paginator или рядом со списком; не прислан — null
        Debug.Assert(found?.HasNext is null && wrapped?.HasNext is null);
        var noCount = ParseSearch("""{"Tasks":[{"Id":7,"Name":"C"}],"Paginator":{"Page":3,"PageSize":200,"CountOnPage":1,"HasNextPage":false}}""");
        Debug.Assert(noCount is { HasNext: false, Total: 1 });
        Debug.Assert(ParseSearch("""{"TaskList":{"Tasks":[{"Id":7,"Name":"C"}],"HasNextPage":"True"}}""") is { HasNext: true });
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

        // повторять ли запрос: сеть, срок и 5xx, 408, 429 проходят сами; логин, доступ, «нет такой» и непонятный ответ — нет.
        // Код — из первой строки: в теле ответа сервера на нижних строках может быть что угодно
        Debug.Assert(HttpCode("ошибка сервера (HTTP 503):\n{\"Message\":\"(HTTP 404)\"}") == 503 && HttpCode("сервер не ответил за 10 с") is null);
        Debug.Assert(IsTransient("сервер не ответил за 60 с") && IsTransient("сервер недоступен:\nNo such host") && IsTransient("соединение оборвалось:\nreset"));
        Debug.Assert(IsTransient("ошибка сервера (HTTP 500)") && IsTransient("ошибка сервера (HTTP 503):\n<html>") && IsTransient("ошибка сервера (HTTP 429)")
            && IsTransient("ошибка сервера (HTTP 408)"));
        Debug.Assert(!IsTransient("неверный логин или пароль (HTTP 401)") && !IsTransient("нет доступа (HTTP 403)") && !IsTransient("заявка не найдена (HTTP 404)")
            && !IsTransient("ошибка сервера (HTTP 400):\n{}") && !IsTransient("непонятный ответ сервера:\n(HTTP 500)") && !IsTransient(""));
    }
}
