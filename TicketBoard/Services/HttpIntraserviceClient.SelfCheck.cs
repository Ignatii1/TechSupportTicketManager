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
            == new IntraserviceTask(159, "Принтер", "Открыта", "a «b»\nc\nd", CutDescription: "a «b»\nc\nd"));
        Debug.Assert(Parse("""{"Id":162,"Name":"D","StatusName":"Выполнена"}""", 1) is { Id: 162, Status: "Выполнена", Description: null });
        Debug.Assert(Parse("""{"Task":{"Name":"C","StatusId":56}}""", 7) is { Id: 7, Status: "статус 56" });
        // заглушка без названия — не статус: по ней не решаем «закрыта или открыта»
        Debug.Assert(!IsResolvedStatus(Parse("""{"Task":{"Name":"C","StatusId":56}}""", 7)!.Status));   // что парсер и пишет
        Debug.Assert(!IsResolvedStatus("статус 56") && !IsResolvedStatus("") && !IsResolvedStatus(null)
            && IsResolvedStatus("Открыта") && IsResolvedStatus("статус уточняется"));
        Debug.Assert(Parse("""{"Message":"The request is invalid."}""", 1) is null);
        // заявка без названия (из почты без темы) — «Заявка #N», как в списке, а не «сервер ответил не так»: иначе её не выгрузить;
        // пустая обёртка — не заявка
        Debug.Assert(Parse("""{"Task":{"Id":163,"StatusId":31},"Statuses":[{"Id":31,"Name":"Открыта"}]}""", 163) is { Id: 163, Name: "Заявка #163", Status: "Открыта" });
        Debug.Assert(Parse("""{"Task":{"Id":164,"Name":null}}""", 164) is { Id: 164, Name: "Заявка #164" });
        Debug.Assert(Parse("""{"Task":{}}""", 1) is null);
        // «Заявка #N» — по запрошенному номеру (по нему доска узнаёт своё название по умолчанию), а номер 0 — не заявка
        Debug.Assert(Parse("""{"Task":{"Id":200,"StatusId":31}}""", 5) is { Id: 200, Name: "Заявка #5" });
        Debug.Assert(Parse("""{"Task":{"Id":0}}""", 5) is null);
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

        // Поиск: форма ответа из документации (стр. 16-17) — Tasks + Statuses + Paginator. Строка без Name — «Заявка #N», а не
        // пропуск (автовыгрузка иначе прошла бы мимо неё навсегда); общее число берём из Paginator.Count, а не из длины списка.
        var found = ParseSearch("""
            {"Tasks":[{"Id":159,"Name":"Принтер","StatusId":31,"Created":"26.11.2015 16:18:06","Creator":"Администратор"},
              {"Id":160,"StatusId":31},
              {"Id":161,"Name":"Сканер","StatusName":"В работе"}],
              "Statuses":[{"Id":31,"Name":"Открыта"}],
              "Paginator":{"Count":137,"Page":1,"PageCount":7,"PageSize":20,"CountOnPage":3}}
            """);
        Debug.Assert(found is not null && found.Value.Total == 137 && found.Value.Found.Count == 3);
        Debug.Assert(found?.Found[1] is { Id: 160, Name: "Заявка #160" });
        Debug.Assert(found?.Found[0] is { Id: 159, Name: "Принтер", Status: "Открыта", Creator: "Администратор" });
        Debug.Assert(found?.Found[0].Created == new DateTimeOffset(new DateTime(2015, 11, 26, 16, 18, 6)));
        Debug.Assert(found?.Found[2] is { Id: 161, Status: "В работе", Creator: null, Executors: null, Changed: null });
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
        // номера инициатора и исполнителей — для «моё ли» автовыгрузки: исполнители строкой через «, » (так в живой карточке,
        // 2026-10-02) или массивом; пустая строка — исполнителей нет; полей нет — null, тогда сверяются имена
        var ids = ParseSearch("""{"Tasks":[{"Id":7,"Name":"C","CreatorId":5,"ExecutorIds":"11, 12"},{"Id":8,"Name":"D","ExecutorIds":[13,14]},{"Id":9,"Name":"E","ExecutorIds":""},{"Id":10,"Name":"F"}]}""");
        Debug.Assert(ids?.Found[0] is { CreatorId: 5, ExecutorIds: [11, 12] } && ids?.Found[1] is { CreatorId: null, ExecutorIds: [13, 14] }
            && ids?.Found[2].ExecutorIds is [] && ids?.Found[3] is { CreatorId: null, ExecutorIds: null }, "номера инициатора и исполнителей в строке списка");
        // Справочники: сервисы в обёртке с Paginator, фильтры голым массивом, сотрудники без обёртки; без номера и названия — пропуск
        var services = ParseRefs("""{"ServiceList":{"Services":[{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|","IsArchive":false},{"Id":840,"Name":"ТСД","Path":"840|","IsArchive":"True"},{"Id":9,"Name":""},{"Name":"без номера"}],"Paginator":{"Count":3,"Page":1,"PageCount":2}}}""", "Services", "ServiceList");
        Debug.Assert(services is { Total: 3, HasMore: true } s0 && s0.Items.Count == 2
            && s0.Items[0] == new IntraserviceRef(844, "Приложение на ТСД", "840|844|") && s0.Items[1] is { Id: 840, IsArchive: true });
        var filters = ParseRefs("""[{"Id":106,"IsCommon":false,"IsDefault":true,"Name":"1. Инциденты"},{"Id":107,"IsDefault":false,"Name":"2. Проблемы"}]""", "FilterView", "ArrayOfFilterView");
        Debug.Assert(filters is { Total: 2, HasMore: false } f0 && f0.Items[0].IsDefault && !f0.Items[1].IsDefault);
        var users = ParseRefs("""{"Users":[{"Id":45,"Name":"Иванов И."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}""", "Users", "UserList");
        Debug.Assert(users is { Total: 1, HasMore: false } u0 && u0.Items[0].Name == "Иванов И.");
        Debug.Assert(ParseRefs("""{"Message":"The request is invalid."}""", "Users", "UserList") is null);
        // Сервисы из списка заявок (include=service): только блок Services; строки — заявки, их не путать с сервисами
        var taskServices = ParseTaskServices("""{"TaskList":{"Tasks":[{"Id":7,"Name":"C","ServiceId":844},{"Id":8,"Name":"D","ServiceId":850}],"Services":[{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"},{"Id":850,"Name":"Принтеры"},{"Id":9}]}}""");
        Debug.Assert(taskServices is { Rows: 2 } ts && ts.Services.Count == 2 && ts.Services[0] == new IntraserviceRef(844, "Приложение на ТСД", "840|844|"));
        Debug.Assert(ParseTaskServices("""{"Tasks":[{"Id":7,"Name":"C"}]}""") is { Rows: 1, Services.Count: 0 });   // блока нет — сервисов нет
        Debug.Assert(ParseTaskServices("""[{"Id":7,"Name":"Заявка, а не сервис"}]""") is null && ParseTaskServices("""{"Message":"x"}""") is null);
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

        // Живые ответы (2026-10-12; форма ответа, значения заменены). Строка списка: даты без пояса, с микросекундами; номера
        // инициатора и исполнителей; Closed, ServiceId, Type; блоки рядом пустые (без include); счёт упирается в тысячу, а
        // HasNextPage при счёте — null
        var liveList = ParseSearch("""{"Tasks":[{"Assets":null,"Categories":null,"Changed":"2026-10-12T11:13:02.945214","Closed":"2026-10-12T11:13:02.894457","Created":"2026-10-12T07:53:54.984246","Creator":"Петрова Анна Сергеевна","EditorId":41404,"CreatorIP":"10.0.0.1","CreatorId":41404,"Data":"<field id=\"1564\"></field><field id=\"3061\">SHIPMENT</field>","Deadline":"2026-10-12T11:15:00","Description":"прошу помочь\r\nс реализацией","Evaluation":"Отлично","EvaluationId":4,"ExecutorGroup":null,"ExecutorGroupId":null,"Files":"800001| Screenshot 2026-10-12 8 52 05.png,800002| реализация.png","Hours":null,"Id":700101,"IsMassIncident":false,"Name":"не проводится реализация","ParentId":null,"Price":null,"PriorityId":12,"ReactionDate":"2026-10-12T09:15:00","ReactionDateFact":"2026-10-12T07:55:00","ReactionOverdue":false,"ResolutionDateFact":"2026-10-12T09:15:00","ResolutionOverdue":false,"ServiceId":69,"ServiceStage":null,"ServiceStageId":null,"StatusId":28,"TaskRepeatRuleId":null,"Type":"Запрос на обслуживание","TypeId":1009,"ExecutorIds":"40620, 18621, 40536","Executors":"Куратор 1С, Куратор ФТО, Сидорова Мария Ивановна","ObserverIds":"32275, 40536","Observers":"Кузнецова Ольга Петровна, Сидорова Мария Ивановна","CoordinatorIds":"","Coordinators":"","FileIds":"800001, 800002"},{"Assets":null,"Categories":null,"Changed":"2026-10-12T11:10:32.789819","Closed":null,"Created":"2026-10-12T10:31:08.270431","Creator":"Смирнова Елена Викторовна","EditorId":32275,"CreatorId":28471,"Description":"Добрый день! Просьба подцепить спеццены","ExecutorGroup":null,"Files":null,"Id":700102,"Name":"Не прикрепляются спец.цены","ServiceId":69,"StatusId":38,"Type":"Запрос на обслуживание","TypeId":1009,"ExecutorIds":"32275","Executors":"Кузнецова Ольга Петровна","ObserverIds":"30415","Observers":"Орлова Ксения Александровна","CoordinatorIds":"","Coordinators":"","FileIds":""}],"Priorities":[],"Services":[],"Statuses":[],"Users":[],"Paginator":{"Count":1000,"Page":1,"PageCount":500,"PageSize":2,"CountOnPage":2,"HasNextPage":null}}""");
        Debug.Assert(liveList is { Total: 1000, HasNext: null } ll && Capped(ll.Total) && ll.Found.Count == 2
            && ll.Found[0] is { Id: 700101, CreatorId: 41404, ExecutorIds: [40620, 18621, 40536], Executors: "Куратор 1С, Куратор ФТО, Сидорова Мария Ивановна",
                Description: "прошу помочь\nс реализацией", Extra: { Type: "Запрос на обслуживание", Categories: "" } }
            && ll.Found[0].Changed == new DateTimeOffset(new DateTime(2026, 10, 12, 11, 13, 2).AddTicks(9_452_140))
            && ll.Found[1] is { Id: 700102, CreatorId: 28471, ExecutorIds: [32275] }, "живая строка списка");
        // Справочник статусов — голый массив с признаками булевыми; закрытые по правилу программы: по признакам и по названию
        var liveStatuses = ParseStatuses("""[{"Id":31,"Name":"Открыта","Description":"Заявка для регистрации","Image16Url":"https://hd/img/statuses/actual16.png","IsCommentRequired":false,"IsFinal":false,"IsFixed":false,"IsInitial":true,"IsConcurrence":false,"IsExternal":false,"IsDeadlineAccountingPaused":false,"Changed":"2016-02-08T17:04:58.76"},{"Id":29,"Name":"Выполнена","IsFinal":false,"IsFixed":true},{"Id":28,"Name":"Закрыта","IsFinal":true,"IsFixed":true},{"Id":30,"Name":"Отменена","IsFinal":true,"IsFixed":false},{"Id":46,"Name":"Ожидание ответа пользователя","IsCommentRequired":true,"IsFinal":false,"IsFixed":false},{"Id":55,"Name":"Отклонена","IsFinal":true,"IsFixed":false},{"Id":88,"Name":"Утверждена ТК","IsFinal":true,"IsFixed":true},{"Id":120,"Name":"Ожидание ответа с автозакрытием","IsFinal":false,"IsFixed":false}]""");
        Debug.Assert(liveStatuses?.Where(s => s.ClosedBy(new AppSettings().ClosedNames())).Select(s => s.Id).Order().SequenceEqual(new[] { 28, 29, 30, 55, 88, 120 }) == true,
            "живые статусы: закрытые — 28, 29, 30, 55, 88 по признакам и 120 по названию");
        // Текущий пользователь — голый объект; история — обёртка TaskLifetimes с пустыми блоками и Paginator (страница — 25);
        // EditorId записи — номер пользователя: по нему «где я участвовал»; первая запись — смена статуса со списком людей
        Debug.Assert(ParseCurrentUser("""{"Id":40800,"UtcOffset":"+03:00","Language":"Ru","Email":"ivanov@example.ru","IsArchive":false,"Login":"ivanovii","MobilePhone":null,"Name":"Иванов Иван Иванович","Phone":null,"Position":"Специалист технической поддержки","RoleId":55,"RoleType":1,"Comments":"ООО Пример / ИТ / ","CompanyId":179,"DefaultTaskFilterId":null,"Phones":[]}""")
            == new IntraserviceUser(40800, "Иванов Иван Иванович"));
        var liveLife = ParseLifetime("""{"TaskLifetimes":[{"Date":"2026-10-02T14:00:48.041767","EditorId":40800,"Editor":"Иванов Иван Иванович","Executors":"Петров Пётр Петрович, Иванов Иван Иванович","Participants":"Сидорова Мария Ивановна","StatusId":31},{"Date":"2026-10-02T14:02:02.137104","EditorId":33736,"Editor":"Кузнецова Ольга Петровна","StatusId":38},{"Comments":"Добрый день! \r\n\r\nНе могли бы вы связаться с поставщиком? \r\n\r\nСпасибо. ","IsPublic":true,"Date":"2026-10-02T14:02:22.482802","EditorId":40800,"Editor":"Иванов Иван Иванович","StatusId":46},{"Comments":"Добрый день! \r\nда, свяжусь.","IsPublic":true,"Date":"2026-10-02T14:07:07.241319","EditorId":26512,"Editor":"Сидорова Мария Ивановна"}],"Priorities":[],"Services":[],"Statuses":[],"Paginator":{"Count":4,"Page":1,"PageCount":1,"PageSize":25,"CountOnPage":4,"HasNextPage":null}}""");
        Debug.Assert(liveLife is { HasMore: false, Paged: true } lf && lf.Events.Count == 4 && lf.Events[0] is { AuthorId: 40800, Comment: null }
            && lf.Events[2] is { AuthorId: 40800, IsPublic: true, Comment: "Добрый день! \n\nНе могли бы вы связаться с поставщиком? \n\nСпасибо." }
            && lf.Events[3] is { AuthorId: 26512, Author: "Сидорова Мария Ивановна" }, "живая история");
        // Карточка: контакты подавшего (CreatorPhone — null, почта есть), сервис и статус названиями, Closed у открытой — null
        var liveCard = Parse("""{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"AssetIds":null,"Assets":null,"Categories":null,"CategoryIds":null,"Changed":"2026-10-12T09:42:10.851778","Closed":null,"Created":"2026-10-06T14:41:43.085385","Creator":"Иванов Иван Иванович","CreatorCompanyName":"Отдел поддержки","CreatorEmail":"ivanov@example.ru","CreatorId":40800,"CreatorMobilePhone":null,"CreatorPhone":null,"CreatorPosition":"Специалист технической поддержки","Description":"Просьба помочь.\r\n\r\nПримеры:\r\n0004607055280006797622IV8","EditorId":27699,"ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"38472, 40800, 17122","Executors":"Петров Пётр Петрович, Иванов Иван Иванович, Сидоров Сергей Сергеевич","Files":"ассортимент.xlsx","FileNames":"ассортимент.xlsx","FileIds":"691552","Id":712182,"Name":"Не проводятся упаковки","ObserverIds":"14361, 26181","Observers":"Орлова Ксения Александровна, Белова Анна Сергеевна","ServiceId":843,"ServiceName":"Система учёта маркировки","ServicePath":"840|843|","StatusId":46,"StatusIsFinal":false,"StatusIsFixed":false,"StatusName":"Ожидание ответа пользователя","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}""", 712182);
        Debug.Assert(liveCard is { Id: 712182, Status: "Ожидание ответа пользователя", Creator: "Иванов Иван Иванович", CreatorEmail: "ivanov@example.ru", CreatorPhone: "",
                ExecutorGroup: "", Description: "Просьба помочь.\n\nПримеры:\n0004607055280006797622IV8", Extra: { Service: "Система учёта маркировки", Type: "Запрос на обслуживание" } }
            && liveCard.Changed == new DateTimeOffset(new DateTime(2026, 10, 12, 9, 42, 10).AddTicks(8_517_780)), "живая карточка");
        // Описания и комментарии живой сервер шлёт простым текстом, а в нём «<» и «>» — знаки: «остаток < 5 шт, заказ > 10»,
        // коды маркировки (…KW<, …r!>n). Снимать «теги» в простом тексте — значит молча терять текст между ними; html (из
        // редактора) узнаётся по разметке — там теги снимаются, а знаки «<» и «>» в нём и так записаны сущностями
        Debug.Assert(ParseLifetime("""[{"Date":"2026-10-12T10:00:00","Editor":"И","Comments":"остаток < 5 шт, заказ > 10 шт"}]""")?.Events[0].Comment
            == "остаток < 5 шт, заказ > 10 шт", "простой текст со знаками < и >");
        Debug.Assert(Parse("""{"Task":{"Id":9,"Name":"КМ","Description":"0104606056177299215Rbk\"rR!>ncE;BuBoKW<\r\n0104606056177299215dTPSOa:HbJ0CmP*8GHO\r\n01046060561772992<TTCx>9"}}""", 9)?.Description
            == "0104606056177299215Rbk\"rR!>ncE;BuBoKW<\n0104606056177299215dTPSOa:HbJ0CmP*8GHO\n01046060561772992<TTCx>9", "коды маркировки целиком");
        Debug.Assert(ParseLifetime("""[{"Date":"2026-10-12T10:00:00","Editor":"И","Comments":"<div>a &lt; b</div><div>c</div>"}]""")?.Events[0].Comment == "a < b\nc",
            "html из редактора — теги сняты, сущности раскрыты");
        // тег узнаётся, только если за именем — пробел, «/» или «>», и в одной строке: почта в угловых скобках (живая, 2026-10-12,
        // имя и адрес изменены: «<300@corp.ru>») и код маркировки «<P» с переводом строки — не теги
        string? Text(string raw) => ParseLifetime($$"""[{"Date":"2026-10-12T10:00:00","Editor":"И","Comments":{{JsonSerializer.Serialize(raw)}}}]""")?.Events[0].Comment;
        // (пробел в конце строки — как в живых комментариях: в простом тексте он остаётся, html-разбор его бы срезал)
        Debug.Assert(Text("Иванов <i.ivanov@corp.ru> \r\nостаток < 5 шт, заказ > 10 шт") == "Иванов <i.ivanov@corp.ru> \nостаток < 5 шт, заказ > 10 шт", "почта с буквы");
        Debug.Assert(Text("Пользователь Петрова Анна <300@corp.ru>") == "Пользователь Петрова Анна <300@corp.ru>", "почта с цифры");
        Debug.Assert(Text("0104606056177299215Rbk<P\r\n0104606056177299215dTPS>Oa") == "0104606056177299215Rbk<P\n0104606056177299215dTPS>Oa", "<P через строку");
        // сущности в простом тексте раскрываются, как прежде
        Debug.Assert(Text("ООО &quot;Ромашка&quot; &amp; Ко") == "ООО \"Ромашка\" & Ко", "сущности в простом тексте");
        // html: одни <br>, заглавные теги с атрибутами, письмо Outlook (стиль, условный комментарий и заголовок окна в <head> —
        // не текст: браузер их не показывает), таблица — ячейки через пробел, строки — с новой строки
        Debug.Assert(Text("строка1<br>строка2") == "строка1\nстрока2", "html из одних br");
        // и в html снимаются только теги: «<», записанный как есть, — текст
        Debug.Assert(Text("<p>остаток < 5 шт, заказ > 10</p>") == "остаток < 5 шт, заказ > 10", "html со знаком <");
        Debug.Assert(Text("<P class=MsoNormal>Текст</P><P>ещё</P>") == "Текст\nещё", "заглавные теги с атрибутами");
        Debug.Assert(Text("<html><head><title>Письмо</title><style>p.MsoNormal{margin:0cm}</style><!--[if gte mso 9]><xml><o:OfficeDocumentSettings></o:OfficeDocumentSettings></xml><![endif]--></head><body><p class=MsoNormal>Текст<o:p></o:p></p></body></html>")
            == "Текст", Text("<html><head><title>Письмо</title><style>p.MsoNormal{margin:0cm}</style><!--[if gte mso 9]><xml><o:OfficeDocumentSettings></o:OfficeDocumentSettings></xml><![endif]--></head><body><p class=MsoNormal>Текст<o:p></o:p></p></body></html>"));
        Debug.Assert(Text("<table><tr><td>Склад</td><td>5</td></tr><tr><td>Магазин</td><td>7</td></tr></table>") == "Склад 5\nМагазин 7", "таблица");
        // html — как его показывает браузер: снимается любая разметка (объявления, условные блоки Outlook, теги HTML5); есть в нём
        // абзацы или таблица — пробелы и переводы строк исходника лишь пробел (письмо переносит строки в исходнике), ячейка Word
        // со своим абзацем — та же строка; закомментированный <style> не съедает текст до настоящего
        void Same(string raw, string want, string what) { var got = Text(raw); Debug.Assert(got == want, $"{what}: {got}"); }
        Same("<p class=MsoListParagraph><![if !supportLists]><span>1. </span><![endif]>Перезагрузить<o:p></o:p></p>", "1. Перезагрузить",
            "условные блоки Outlook");
        Same("<!DOCTYPE html><section>Срок</section><section>завтра</section>", "Срок\nзавтра", "разделы HTML5");
        Same("<dl><dt>Срок</dt><dd>завтра</dd></dl>", "Срок\nзавтра", "список определений");
        Same("<p class=MsoNormal>Добрый день! Прошу\r\nпомочь</p>", "Добрый день! Прошу помочь", "перенос строки в исходнике html");
        Same("<table>\r\n<tr>\r\n <td><p class=MsoNormal>Склад</p></td>\r\n <td><p class=MsoNormal>5</p></td>\r\n</tr>\r\n</table>", "Склад 5",
            "таблица Word");
        Same("<!-- <style> old --><p>Важный текст</p><style>p{}</style>", "Важный текст", "<style> в комментарии");
        // без абзацев (одни <br> или строчный тег) переводы строк исходника — строки: так выглядит простой текст с тегом
        Same("строка1<br>\r\nстрока2<br />\r\nстрока3", "строка1\nстрока2\nстрока3", "<br> и перевод строки за ним — одна строка");
        Same("Строка 1\r\nСтрока 2 <b>важно</b>", "Строка 1\nСтрока 2 важно", "простой текст с тегом <b>");
        // html узнаётся только по закрывающему тегу или <br>: открывающий «тег» бывает и в простом тексте — в коде маркировки
        // («<p/7z>», «<b>»), в упоминании тега — и html-разбор снял бы куски кодов и свёл строки в одну
        Same("Коды:\r\n010460<p/7z>91EE06<TTCx>92abc\r\n0104601<b>xyz", "Коды:\n010460<p/7z>91EE06<TTCx>92abc\n0104601<b>xyz", "открывающие «теги» в кодах");
        Same("В шаблоне <div class=\"x\"> не закрыт\r\nСтрока 2", "В шаблоне <div class=\"x\"> не закрыт\nСтрока 2", "упомянутый тег");
        // и закрывающий в цитате — не html: нужна пара «открывающий + такой же закрывающий» известного тега (закрывающий — в одной
        // строке), <br> или <img …>
        Same("Ошибка в шаблоне: лишний </td> в строке 5\r\nНажмите <Ctrl>+<F5>, код 0104<p/7z>99",
            "Ошибка в шаблоне: лишний </td> в строке 5\nНажмите <Ctrl>+<F5>, код 0104<p/7z>99", "цитата закрывающего тега");
        Same("Код <P>x</P\r\n>Oa<TTCx>92", "Код <P>x</P\n>Oa<TTCx>92", "закрывающий через строку");
        // а код маркировки в настоящем html — текст: «<p/7z>» не тег (за именем — «>», «/>» или атрибуты через пробел), не абзац
        Same("<b>Коды</b>\r\n010460<p/7z>91", "Коды\n010460<p/7z>91", "код в html");
        Same("<p>Код 0104<pre/7z>91 и\r\nдалее</p><pre>a</pre>", "Код 0104<pre/7z>91 и далее\na", "код с «<pre/» в html");
        // узнавание html — линейное: десятки тысяч одиночных «<b>» и чужих «</i>» в кодах маркировки — не минуты (поиск пары от
        // каждого открывающего до конца текста перебирал бы все закрывающие после него)
        var codes = string.Concat(Enumerable.Repeat("0104606056<b>72</i>99Rbk\r\n", 60_000));
        var watch = Stopwatch.StartNew();
        Debug.Assert(Text(codes) is { Length: > 0 } plain && plain.StartsWith("0104606056<b>72</i>99Rbk\n", StringComparison.Ordinal)
            && watch.Elapsed < TimeSpan.FromSeconds(5), $"60 000 «<b>» и «</i>»: {watch.Elapsed}");
        // <br> с атрибутами (Word: Shift+Enter, Outlook) — перевод строки и признак html; картинка — html, снята вместе с данными
        Same("<p class=MsoNormal>Строка1<br style='mso-special-character:line-break'>Строка2</p>", "Строка1\nСтрока2", "<br> с атрибутами");
        Same("Строка1<br clear=all>Строка2", "Строка1\nСтрока2", "<br clear=all>");
        Same("Скриншот:\r\n<img src=\"data:image/png;base64,iVBORw0KGgo=\">", "Скриншот:", "<img>");
        // без абзацев перевод строки исходника сразу за блоком — та же строка, а не лишняя пустая
        Same("Ошибка:\r\n<pre>trace</pre>\r\nСпасибо", "Ошибка:\ntrace\nСпасибо", "перевод строки за блоком");
        // блок начинается с новой строки и открывающим тегом, а пункты без закрывающих — каждый своей строкой
        Same("Добрый день!<div>Не печатает принтер</div><div>Спасибо</div>", "Добрый день!\nНе печатает принтер\nСпасибо", "текст перед блоком");
        Same("Список:<ul><li>a<li>b</ul>", "Список:\na\nb", "пункты без закрывающих тегов");
        // пустой абзац Word — пустая строка (а не строка из неразрывного пробела: Markdown не счёл бы её пустой и склеил абзацы)
        Same("<p>Текст</p><p>&nbsp;</p><p>Ещё</p>", "Текст\n\nЕщё", "пустой абзац");
        // <pre> — со своими строками
        Same("<p>Ошибка:</p><pre>System.Exception: boom\r\n   at A.B()\r\n   at C.D()</pre>", "Ошибка:\nSystem.Exception: boom\nat A.B()\nat C.D()", "<pre>");
        // прежний разбор (до 0.15.1) — чтобы узнать на доске описание, им обрезанное
        Debug.Assert(LegacyText("остаток < 5 шт, заказ > 10 шт") == "остаток  10 шт" && LegacyText("<p>a &laquo;b&raquo;</p>") == "a «b»" && LegacyText(null) is null);

        // Сырой ответ в сообщении: json — как есть; html — видимым текстом, без стилей; длинное — обрезано.
        Debug.Assert(Evidence(""" {"Message":"The request is invalid."} """) == """{"Message":"The request is invalid."}""");
        var page = Evidence("<html><head><title>401 - Unauthorized</title><style>body{color:red}</style></head><body><p>Access denied</p></body></html>");
        Debug.Assert(page == "(html-страница, показан её текст)\n401 - Unauthorized\nAccess denied");
        // страница IIS (объявление DOCTYPE, комментарий в стиле, fieldset, переводы строк исходника) и страница входа (форма) —
        // текстом, без разметки
        var iis = Evidence("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\">\r\n"
            + "<html xmlns=\"http://www.w3.org/1999/xhtml\">\r\n<head>\r\n<title>401 - Unauthorized</title>\r\n<style type=\"text/css\">\r\n<!--\r\n"
            + "body{margin:0}\r\n-->\r\n</style>\r\n</head>\r\n<body>\r\n<div id=\"header\"><h1>Server Error</h1></div>\r\n<div id=\"content\">\r\n"
            + " <div class=\"content-container\"><fieldset>\r\n  <h2>401 - Unauthorized: Access is denied.</h2>\r\n </fieldset></div>\r\n</div>\r\n</body>\r\n</html>\r\n");
        // (заголовок «401 - Unauthorized» текст страницы уже содержит — второй раз не показан)
        Debug.Assert(iis == "(html-страница, показан её текст)\nServer Error\n401 - Unauthorized: Access is denied.", iis);
        var login = Evidence("<html><head><title>Вход</title></head><body><form method=\"post\" action=\"/login\"><label>Логин</label> <input name=\"user\"> "
            + "<button>Войти</button></form></body></html>");
        Debug.Assert(login == "(html-страница, показан её текст)\nВход\nЛогин Войти", login);
        var trace = Evidence("<html><head><title>Runtime Error</title></head><body><h2>Server Error</h2><pre>System.Exception: boom\r\n   at A.B()\r\n"
            + "   at C.D()</pre></body></html>");
        Debug.Assert(trace == "(html-страница, показан её текст)\nRuntime Error\nServer Error\nSystem.Exception: boom\nat A.B()\nat C.D()", trace);
        // заголовок опускается, только если с него начинается одна из первых строк текста: «Входящие» — не «Вход», а
        // заголовок глубже показался бы за пределом в 1000 знаков
        var inbox = Evidence("<html><head><title>Вход</title></head><body><p>Входящие недоступны</p></body></html>");
        Debug.Assert(inbox == "(html-страница, показан её текст)\nВход\nВходящие недоступны", inbox);
        var deep = Evidence("<html><head><title>Доступ запрещён</title></head><body><p>a</p><p>b</p><p>c</p><p>Доступ запрещён</p></body></html>");
        Debug.Assert(deep == "(html-страница, показан её текст)\nДоступ запрещён\na\nb\nc\nДоступ запрещён", deep);
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
