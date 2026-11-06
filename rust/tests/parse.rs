//! Разбор ответов Интрасервиса — образцы `HttpIntraserviceClient.SelfCheck` один в один (документация v5.42/v5.51 и живые
//! ответы, обезличенные), в том же порядке.

use chrono::{DateTime, FixedOffset, Local, NaiveDate, TimeZone, Utc};
use ticketboard::intraservice::parse;
use ticketboard::intraservice::*;
use ticketboard::rules::ClosedNames;

/// Местное время, как `new DateTimeOffset(new DateTime(...))` в C#: дата без пояса — по смещению этого компьютера.
fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, nanos: u32) -> DateTime<FixedOffset> {
    let naive = NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_nano_opt(h, mi, s, nanos).unwrap();
    Local.from_local_datetime(&naive).single().unwrap().fixed_offset()
}

fn task(json: &str, id: i32) -> Task {
    parse::task(json, id).unwrap_or_else(|| panic!("не разобрана: {json}"))
}

#[test]
fn cards() {
    // описание из редактора — без html, сущности раскрыты; статус — по StatusId из блока Statuses
    let t = task(
        r#"{"Task":{"Id":159,"Name":"Принтер","Description":"<p>a &laquo;b&raquo;</p><p>c<br/>d</p>","StatusId":31},"Statuses":[{"Id":31,"Name":"Открыта"}]}"#,
        1,
    );
    assert_eq!(
        t,
        Task {
            id: 159,
            name: "Принтер".into(),
            status: "Открыта".into(),
            description: Some("a «b»\nc\nd".into()),
            cut_description: Some("a «b»\nc\nd".into()),
            status_id: Some(31),
            ..Task::default()
        }
    );
    let t = task(r#"{"Id":162,"Name":"D","StatusName":"Выполнена"}"#, 1);
    assert_eq!((t.id, t.status.as_str(), t.description.as_deref()), (162, "Выполнена", None));
    let t = task(r#"{"Task":{"Name":"C","StatusId":56}}"#, 7);
    assert_eq!((t.id, t.status.as_str()), (7, "статус 56"));
    // заглушка без названия — не статус: по ней не решаем «закрыта или открыта»
    assert!(!parse::is_resolved_status(&t.status));
    assert!(!parse::is_resolved_status("статус 56") && !parse::is_resolved_status("") && !parse::is_resolved_status("  "));
    assert!(parse::is_resolved_status("Открыта") && parse::is_resolved_status("статус уточняется"));
    // как `$` у .NET: перевод строки в самом конце — всё ещё заглушка
    assert!(!parse::is_resolved_status("статус 56\n") && parse::is_resolved_status("статус 56\nещё"));
    assert!(parse::task(r#"{"Message":"The request is invalid."}"#, 1).is_none());
    // заявка без названия (из почты без темы) — «Заявка #N», как в списке; пустая обёртка — не заявка
    let t = task(r#"{"Task":{"Id":163,"StatusId":31},"Statuses":[{"Id":31,"Name":"Открыта"}]}"#, 163);
    assert_eq!((t.id, t.name.as_str(), t.status.as_str()), (163, "Заявка #163", "Открыта"));
    let t = task(r#"{"Task":{"Id":164,"Name":null}}"#, 164);
    assert_eq!((t.id, t.name.as_str()), (164, "Заявка #164"));
    assert!(parse::task(r#"{"Task":{}}"#, 1).is_none());
    // «Заявка #N» — по запрошенному номеру, а номер 0 — не заявка
    let t = task(r#"{"Task":{"Id":200,"StatusId":31}}"#, 5);
    assert_eq!((t.id, t.name.as_str()), (200, "Заявка #5"));
    assert!(parse::task(r#"{"Task":{"Id":0}}"#, 5).is_none());
    // не json и не объект — не заявка
    assert!(parse::task("<html>", 5).is_none() && parse::task("[1, 2]", 5).is_none() && parse::task("", 5).is_none());
}

#[test]
fn people_and_dates() {
    // исполнители строкой через запятую — к одному виду; поля нет — None, пусто или json null — ""
    let t = task(
        r#"{"Task":{"Id":5,"Name":"N","Creator":" Сидоров С. ","Executors":"Иванов И. И.,Петров П.;  ","ExecutorGroup":"Первая линия","Changed":"26.09.2026 10:15:00"}}"#,
        5,
    );
    assert_eq!(t.creator.as_deref(), Some("Сидоров С."));
    assert_eq!(t.executors.as_deref(), Some("Иванов И. И., Петров П."));
    assert_eq!(t.executor_group.as_deref(), Some("Первая линия"));
    assert_eq!(t.changed_text.as_deref(), Some("26.09.2026 10:15:00"));
    assert_eq!(t.changed, Some(local(2026, 9, 26, 10, 15, 0, 0)));
    let t = task(r#"{"Id":5,"Name":"N","Executors":null,"ExecutorGroup":null}"#, 5);
    assert_eq!(
        (t.creator, t.executors.as_deref(), t.executor_group.as_deref(), t.changed, t.changed_text, t.creator_phone, t.creator_email),
        (None, Some(""), Some(""), None, None, None, None)
    );
    // как связаться с подавшим — строки как есть (без пробелов по краям); json null — «пусто»
    let t = task(r#"{"Id":5,"Name":"N","CreatorPhone":" +7 (495) 123-45-67 ","CreatorEmail":null}"#, 5);
    assert_eq!((t.creator_phone.as_deref(), t.creator_email.as_deref()), (Some("+7 (495) 123-45-67"), Some("")));
    // имя поля — без учёта регистра, как у C# (`Prop`)
    let t = task(r#"{"task":{"id":9,"NAME":"регистр","statusname":"Открыта"}}"#, 1);
    assert_eq!((t.id, t.name.as_str(), t.status.as_str()), (9, "регистр", "Открыта"));
    // номер — только целое в 32 бита (`TryGetInt32`): дробное и огромное — «нет номера», и тогда решает название
    assert!(parse::task(r#"{"Task":{"Id":1.5}}"#, 5).is_none());
    assert_eq!(task(r#"{"Task":{"Id":3000000000,"Name":"x"}}"#, 5).id, 5);
}

const LIVE: &str = r#"{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"Categories":null,"CategoryIds":null,"Changed":"2026-08-27T15:54:43.460865","Created":"2026-08-07T10:15:08.408534","Creator":"Петрова Анна","Description":"строка 1\r\nстрока 2","ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"11, 12","Executors":"Иванов И. И., Сидоров С. С.","Id":700001,"Name":"Склад: удалить задания","ResolutionDateFact":"2026-08-24T17:07:00","ServiceCode":"005","ServiceId":844,"ServiceName":"Приложение на ТСД","ServicePath":"840|844|","StatusId":28,"StatusIsFinal":true,"StatusName":"Закрыта","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}"#;

const LIVE_CARD: &str = r#"{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"AssetIds":null,"Assets":null,"Categories":null,"CategoryIds":null,"Changed":"2026-10-12T09:42:10.851778","Closed":null,"Created":"2026-10-06T14:41:43.085385","Creator":"Иванов Иван Иванович","CreatorCompanyName":"Отдел поддержки","CreatorEmail":"ivanov@example.ru","CreatorId":40800,"CreatorMobilePhone":null,"CreatorPhone":null,"CreatorPosition":"Специалист технической поддержки","Description":"Просьба помочь.\r\n\r\nПримеры:\r\n0004607055280006797622IV8","EditorId":27699,"ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"38472, 40800, 17122","Executors":"Петров Пётр Петрович, Иванов Иван Иванович, Сидоров Сергей Сергеевич","Files":"ассортимент.xlsx","FileNames":"ассортимент.xlsx","FileIds":"691552","Id":712182,"Name":"Не проводятся упаковки","ObserverIds":"14361, 26181","Observers":"Орлова Ксения Александровна, Белова Анна Сергеевна","ServiceId":843,"ServiceName":"Система учёта маркировки","ServicePath":"840|843|","StatusId":46,"StatusIsFinal":false,"StatusIsFixed":false,"StatusName":"Ожидание ответа пользователя","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}"#;

#[test]
fn live_cards() {
    // карточка с живого сервера (2026-10-02): блоки рядом с Task — null, статус строкой, сервис — ServiceName, тип — Type
    let live = task(LIVE, 1);
    assert_eq!((live.id, live.status.as_str()), (700001, "Закрыта"));
    assert_eq!((live.executors.as_deref(), live.executor_group.as_deref()), (Some("Иванов И. И., Сидоров С. С."), Some("")));
    // Changed строкой — как прислал сервер, до микросекунд; без пробелов по краям, пустая — как нет
    assert_eq!(live.changed_text.as_deref(), Some("2026-08-27T15:54:43.460865"));
    let t = task(r#"{"Id":5,"Name":"N","Changed":" 2026-01-02T03:04:05.5 "}"#, 5);
    assert_eq!(t.changed_text.as_deref(), Some("2026-01-02T03:04:05.5"));
    let t = task(r#"{"Id":5,"Name":"N","Changed":" "}"#, 5);
    assert_eq!((t.changed_text, t.changed), (None, None));
    let x = live.extra.clone().expect("сервис и тип есть");
    assert_eq!(
        (x.service.as_deref(), x.task_type.as_deref(), x.categories.as_deref()),
        (Some("Приложение на ТСД"), Some("Запрос на обслуживание"), Some(""))
    );
    assert_eq!(x.resolved, Some(local(2026, 8, 24, 17, 7, 0, 0)));
    // исполнители массивом — строками или объектами с Name
    let t = task(r#"{"Id":5,"Name":"N","Executors":["Иванов",{"Id":2,"Name":"Петров"},{"Id":3},""]}"#, 5);
    assert_eq!(t.executors.as_deref(), Some("Иванов, Петров"));

    // карточка: контакты подавшего (CreatorPhone — null, почта есть), сервис и статус названиями, Closed у открытой — null
    let card = task(LIVE_CARD, 712182);
    assert_eq!((card.id, card.status.as_str()), (712182, "Ожидание ответа пользователя"));
    assert_eq!(
        (card.creator.as_deref(), card.creator_email.as_deref(), card.creator_phone.as_deref(), card.executor_group.as_deref()),
        (Some("Иванов Иван Иванович"), Some("ivanov@example.ru"), Some(""), Some(""))
    );
    assert_eq!(card.description.as_deref(), Some("Просьба помочь.\n\nПримеры:\n0004607055280006797622IV8"));
    assert_eq!(card.changed, Some(local(2026, 10, 12, 9, 42, 10, 851_778_000)));
    assert_eq!(card.created, Some(local(2026, 10, 6, 14, 41, 43, 85_385_000)));
    let x = card.extra.clone().unwrap();
    assert_eq!((x.service.as_deref(), x.task_type.as_deref()), (Some("Система учёта маркировки"), Some("Запрос на обслуживание")));
    // организация инициатора, оборудование и вложения — из карточки; даты закрытия в ней нет
    assert_eq!((x.company.as_deref(), x.assets.as_deref(), x.closed), (Some("Отдел поддержки"), Some(""), None));
    assert_eq!(x.files, Some(vec![TaskFile { id: 691552, name: Some("ассортимент.xlsx".into()) }]));
    // должность инициатора — для панели; поля нет — None
    assert_eq!((card.creator_position.as_deref(), live.creator_position.as_deref()), (Some("Специалист технической поддержки"), None));
    // наблюдатели — номерами и именами; поля нет — None
    assert_eq!(card.observer_ids, Some(vec![14361, 26181]));
    assert_eq!(card.observers.as_deref(), Some("Орлова Ксения Александровна, Белова Анна Сергеевна"));
    assert_eq!((live.observer_ids.clone(), live.observers.clone()), (None, None));
    // номер статуса — для «уже в нужном»; не прислал — None
    assert_eq!(task(r#"{"Task":{"Id":5,"Name":"N","StatusId":46}}"#, 5).status_id, Some(46));
    assert_eq!(task(r#"{"Task":{"Id":5,"Name":"N"}}"#, 5).status_id, None);
}

#[test]
fn assets_and_files() {
    // оборудование и файлы — через «||», как на карточной форме, или запятой
    let x =
        task(r#"{"Task":{"Id":5,"Name":"x","StatusId":1,"Assets":"Ноутбук HP||Принтер Canon","FileIds":"10, 11,12"}}"#, 5).extra.unwrap();
    assert_eq!(x.assets.as_deref(), Some("Ноутбук HP, Принтер Canon"));
    assert_eq!(x.files, Some(vec![TaskFile { id: 10, name: None }, TaskFile { id: 11, name: None }, TaskFile { id: 12, name: None }]));
    // вложения — номера из FileIds и имена из FileNames через «||»: запятая — часть имени; имён не столько, сколько номеров,
    // — номера без имён; FileIds пустой — вложений нет, поля нет — не прислано
    let files = |fields: &str| {
        parse::task(&format!(r#"{{"Task":{{"Id":5,"Name":"x","StatusId":1,{fields}}}}}"#), 5).unwrap().extra.and_then(|x| x.files)
    };
    let named = |pairs: &[(i32, Option<&str>)]| -> Option<Vec<TaskFile>> {
        Some(pairs.iter().map(|&(id, name)| TaskFile { id, name: name.map(String::from) }).collect())
    };
    assert_eq!(
        files(r#""FileIds":"800001, 800002","FileNames":"a.png||b, c.png""#),
        named(&[(800001, Some("a.png")), (800002, Some("b, c.png"))]),
        "имена через ||"
    );
    assert_eq!(files(r#""FileIds":"800001, 800002","FileNames":"a.png""#), named(&[(800001, None), (800002, None)]), "имён меньше");
    assert_eq!(files(r#""FileIds":"7, 7","FileNames":"a.png||a.png""#), named(&[(7, Some("a.png"))]), "номер дважды — одно вложение");
    assert_eq!(files(r#""FileIds":"","FileNames":"""#), Some(vec![]));
    assert_eq!(files(r#""FileIds":null"#), Some(vec![]));
    assert_eq!(files(r#""Assets":"""#), None, "не прислано");
    // номера массивом: числа и строки с числами; не число (знак, дробь) — пропуск
    assert_eq!(files(r#""FileIds":[13,"14",-15,1.5,"x",null]"#), named(&[(13, None), (14, None)]));
}

#[test]
fn lifetimes() {
    // пример из документации (стр. 65-66): первая запись — смена статуса без Comments; во второй — комментарий и «виден клиенту»
    let life = parse::lifetime(
        r#"{"TaskLifetimeList":{"TaskLifetimes":[
          {"Date":"12.11.2015 13:44:53","EditorId":43,"Editor":"Администратор","StatusId":29},
          {"Date":"11.11.2015 15:24:10","EditorId":43,"Editor":"Администратор","StatusId":31,"Comments":"<p>Проверьте, пожалуйста</p>","IsPublic":"True"}],
          "Statuses":[{"Id":31,"Name":"Открыта"},{"Id":29,"Name":"Выполнена"}],
          "Paginator":{"Count":2,"Page":1,"PageCount":1,"PageSize":25,"CountOnPage":2}}}"#,
    )
    .unwrap();
    assert!(!life.has_more && life.paged && life.events.len() == 2);
    let e = &life.events[0];
    assert_eq!((e.author.as_str(), e.status.as_str(), e.comment.as_deref(), e.is_public), ("Администратор", "Выполнена", None, None));
    assert_eq!(e.date, Some(local(2015, 11, 12, 13, 44, 53, 0)));
    let e = &life.events[1];
    assert_eq!(
        (e.status.as_str(), e.comment.as_deref(), e.is_public, e.author_id),
        ("Открыта", Some("Проверьте, пожалуйста"), Some(true), Some(43))
    );
    // голый массив, дата в ISO, статуса 7 в ответе нет, пустой комментарий — не комментарий
    let bare =
        parse::lifetime(r#"[{"Date":"2015-10-29T13:51:14.023","Editor":"Иванов","StatusId":7,"Comments":"","IsPublic":false}]"#).unwrap();
    assert!(!bare.has_more && !bare.paged && bare.events.len() == 1);
    let e = &bare.events[0];
    assert_eq!(
        (e.author.as_str(), e.status.as_str(), e.comment.as_deref(), e.is_public, e.author_id),
        ("Иванов", "статус 7", None, Some(false), None)
    );
    assert_eq!(e.date, Some(local(2015, 10, 29, 13, 51, 14, 23_000_000)));
    // дата WCF — миллисекунды от 1970 UTC
    let wcf = parse::lifetime(r#"[{"Date":"/Date(1447335893000)/","Editor":"Иванов","StatusId":7}]"#).unwrap();
    assert_eq!(wcf.events[0].date, Some(Utc.with_ymd_and_hms(2015, 11, 12, 13, 44, 53).unwrap().fixed_offset()));
    // пустая страница, но не последняя
    let page2 = parse::lifetime(r#"{"TaskLifetimes":[],"Paginator":{"Page":2,"PageCount":3}}"#).unwrap();
    assert!(page2.has_more && page2.events.is_empty());
    // Paginator без номеров страниц ничего не говорит о следующей
    let count_only = parse::lifetime(r#"{"TaskLifetimes":[],"Paginator":{"Count":120}}"#).unwrap();
    assert!(!count_only.has_more && !count_only.paged);
    assert!(parse::lifetime(r#"{"Message":"The request is invalid."}"#).is_none());
}

#[test]
fn live_lifetimes() {
    // обёртка TaskLifetimes с пустыми блоками и Paginator; EditorId — номер пользователя; комментарий — без \r, пробел в конце срезан
    let life = parse::lifetime(r#"{"TaskLifetimes":[{"Date":"2026-10-02T14:00:48.041767","EditorId":40800,"Editor":"Иванов Иван Иванович","Executors":"Петров Пётр Петрович, Иванов Иван Иванович","Participants":"Сидорова Мария Ивановна","StatusId":31},{"Date":"2026-10-02T14:02:02.137104","EditorId":33736,"Editor":"Кузнецова Ольга Петровна","StatusId":38},{"Comments":"Добрый день! \r\n\r\nНе могли бы вы связаться с поставщиком? \r\n\r\nСпасибо. ","IsPublic":true,"Date":"2026-10-02T14:02:22.482802","EditorId":40800,"Editor":"Иванов Иван Иванович","StatusId":46},{"Comments":"Добрый день! \r\nда, свяжусь.","IsPublic":true,"Date":"2026-10-02T14:07:07.241319","EditorId":26512,"Editor":"Сидорова Мария Ивановна"}],"Priorities":[],"Services":[],"Statuses":[],"Paginator":{"Count":4,"Page":1,"PageCount":1,"PageSize":25,"CountOnPage":4,"HasNextPage":null}}"#).unwrap();
    assert!(!life.has_more && life.paged && life.events.len() == 4);
    assert_eq!((life.events[0].author_id, life.events[0].comment.as_deref()), (Some(40800), None));
    let e = &life.events[2];
    assert_eq!(
        (e.author_id, e.is_public, e.comment.as_deref()),
        (Some(40800), Some(true), Some("Добрый день! \n\nНе могли бы вы связаться с поставщиком? \n\nСпасибо."))
    );
    assert_eq!((life.events[3].author_id, life.events[3].author.as_str()), (Some(26512), "Сидорова Мария Ивановна"));

    // история 715007 (обезличена): у записи — признак «выполнена» или «конечный» её статуса из блока Statuses
    let life = parse::lifetime(r#"{"TaskLifetimes":[{"Date":"2026-10-15T09:14:00.1","EditorId":300,"Editor":"Сидоров Сидор","StatusId":29},{"Comments":"Логин: ТСД_Склад_3\r\nПароль: 11111111","IsPublic":true,"Date":"2026-10-14T15:29:33.359278","EditorId":200,"Editor":"Иванов Иван"},{"Comments":"Игнат, чиркани логин и пароль для 2х ТСД","IsPublic":true,"Date":"2026-10-14T15:22:50.260533","EditorId":300,"Editor":"Сидоров Сидор"},{"Comments":"@Сидоров Сидор, привет! Давай попробуем настроить как в прошлый раз.","IsPublic":true,"Date":"2026-10-14T11:06:07.476226","EditorId":200,"Editor":"Иванов Иван","StatusId":46},{"Date":"2026-10-14T11:02:49.322765","EditorId":600,"Editor":"Диспетчерова Дарья","StatusId":38},{"Date":"2026-10-14T11:02:00.555984","EditorId":500,"Editor":"Диспетчер Денис","Executors":"Иванов Иван, Сидоров Сидор","ExecutorsGroup":"ТП склад","Participants":"Иванов Иван","StatusId":31}],"Priorities":[],"Services":[],"Statuses":[{"Id":31,"Name":"Открыта","IsFinal":false,"IsFixed":false},{"Id":38,"Name":"Передана исполнителю","IsFinal":false,"IsFixed":false},{"Id":46,"Name":"Ожидание ответа пользователя","IsCommentRequired":true,"IsFinal":false,"IsFixed":false},{"Id":29,"Name":"Выполнена","IsFinal":false,"IsFixed":true}],"Paginator":{"Count":6,"Page":1,"PageCount":1,"PageSize":50,"CountOnPage":6,"HasNextPage":null}}"#).unwrap();
    assert_eq!(life.events.len(), 6);
    assert_eq!((life.events[0].status.as_str(), life.events[0].is_final), ("Выполнена", true));
    let e = &life.events[3];
    assert_eq!(
        (e.status.as_str(), e.is_final, e.comment.as_deref()),
        ("Ожидание ответа пользователя", false, Some("@Сидоров Сидор, привет! Давай попробуем настроить как в прошлый раз."))
    );
    assert_eq!(life.events.iter().filter(|e| e.is_final).count(), 1);
}

#[test]
fn lists() {
    // форма из документации (стр. 16-17): строка без Name — «Заявка #N»; общее число — из Paginator.Count
    let found = parse::search(
        r#"{"Tasks":[{"Id":159,"Name":"Принтер","StatusId":31,"Created":"26.11.2015 16:18:06","Creator":"Администратор"},
          {"Id":160,"StatusId":31},
          {"Id":161,"Name":"Сканер","StatusName":"В работе"}],
          "Statuses":[{"Id":31,"Name":"Открыта"}],
          "Paginator":{"Count":137,"Page":1,"PageCount":7,"PageSize":20,"CountOnPage":3}}"#,
    )
    .unwrap();
    assert_eq!((found.total, found.found.len(), found.has_next), (137, 3, None));
    assert_eq!((found.found[1].id, found.found[1].name.as_str()), (160, "Заявка #160"));
    let f = &found.found[0];
    assert_eq!((f.id, f.name.as_str(), f.status.as_str(), f.creator.as_deref()), (159, "Принтер", "Открыта", Some("Администратор")));
    assert_eq!(f.created, Some(local(2015, 11, 26, 16, 18, 6, 0)));
    let f = &found.found[2];
    assert_eq!((f.id, f.status.as_str(), f.creator.as_deref(), f.executors.as_deref(), f.changed), (161, "В работе", None, None, None));
    // строка списка несёт исполнителей и Changed — импорт и автообновление берут их отсюда
    let f = &parse::search(r#"{"Tasks":[{"Id":7,"Name":"C","Executors":"Иванов","ExecutorGroup":"ИТ","Changed":"2026-09-26T10:15:00","CreatorEmail":"a@b.ru"}]}"#)
        .unwrap()
        .found[0];
    assert_eq!(
        (f.executors.as_deref(), f.executor_group.as_deref(), f.changed.is_some(), f.creator_email.as_deref(), f.creator_phone.as_deref()),
        (Some("Иванов"), Some("ИТ"), true, Some("a@b.ru"), None)
    );
    // для выгрузки: сервис, тип, категории (строкой или массивом) и фактическая дата решения; нет полей — None
    let extra = parse::search(r#"{"Tasks":[{"Id":7,"Name":"C","ServiceName":"Принтеры","Type":"Инцидент","Categories":["Печать","HP"],"ResolutionDateFact":"20.09.2026 16:40:00"},{"Id":8,"Name":"D"}]}"#).unwrap();
    let x = extra.found[0].extra.clone().unwrap();
    assert_eq!(
        (x.service.as_deref(), x.task_type.as_deref(), x.categories.as_deref()),
        (Some("Принтеры"), Some("Инцидент"), Some("Печать, HP"))
    );
    assert_eq!(x.resolved, Some(local(2026, 9, 20, 16, 40, 0, 0)));
    assert_eq!(extra.found[1].extra, None);
    // include=service: у строки только ServiceId — название из блока Services; ServiceName в строке главнее
    let with = parse::search(r#"{"Tasks":[{"Id":7,"Name":"C","ServiceId":844,"Type":"Инцидент"},{"Id":8,"Name":"D","ServiceId":99},{"Id":9,"Name":"E","ServiceId":844,"ServiceName":"Свой"}],"Services":[{"Id":844,"Name":"Приложение на ТСД"}],"Paginator":{"Count":3,"Page":1,"PageCount":1}}"#).unwrap();
    let x = with.found[0].extra.clone().unwrap();
    assert_eq!((x.service.as_deref(), x.task_type.as_deref()), (Some("Приложение на ТСД"), Some("Инцидент")));
    assert_eq!(with.found[1].extra, None, "сервиса 99 в блоке нет");
    assert_eq!(with.found[2].extra.clone().unwrap().service.as_deref(), Some("Свой"));
    // номера инициатора и исполнителей: строкой через «, » или массивом; пустая строка — исполнителей нет; полей нет — None
    let ids = parse::search(r#"{"Tasks":[{"Id":7,"Name":"C","CreatorId":5,"ExecutorIds":"11, 12"},{"Id":8,"Name":"D","ExecutorIds":[13,14]},{"Id":9,"Name":"E","ExecutorIds":""},{"Id":10,"Name":"F"}]}"#).unwrap();
    let pairs: Vec<(Option<i32>, Option<Vec<i32>>)> = ids.found.iter().map(|f| (f.creator_id, f.executor_ids.clone())).collect();
    assert_eq!(pairs, [(Some(5), Some(vec![11, 12])), (None, Some(vec![13, 14])), (None, Some(vec![])), (None, None)]);
    // обёртка TaskList, Paginator'а нет: общее число — сколько пришло, статуса нет вовсе — пустая строка
    let wrapped = parse::search(r#"{"TaskList":{"Tasks":[{"Id":7,"Name":"C"}]}}"#).unwrap();
    assert_eq!((wrapped.total, wrapped.found[0].status.as_str(), wrapped.has_next), (1, "", None));
    // счёт не заказан (count=false): вместо Count — HasNextPage, в Paginator или рядом со списком
    let no_count =
        parse::search(r#"{"Tasks":[{"Id":7,"Name":"C"}],"Paginator":{"Page":3,"PageSize":200,"CountOnPage":1,"HasNextPage":false}}"#)
            .unwrap();
    assert_eq!((no_count.has_next, no_count.total), (Some(false), 1));
    assert_eq!(parse::search(r#"{"TaskList":{"Tasks":[{"Id":7,"Name":"C"}],"HasNextPage":"True"}}"#).unwrap().has_next, Some(true));
    assert!(parse::search(r#"{"Message":"The request is invalid."}"#).is_none());
    // описание в списке — тот же html из редактора, что и в карточке
    let f = &parse::search(r#"{"Tasks":[{"Id":7,"Name":"C","Description":"<p>a &laquo;b&raquo;</p><p>c<br/>d</p>"}]}"#).unwrap().found[0];
    assert_eq!(f.description.as_deref(), Some("a «b»\nc\nd"));
    // строка без номера — пропуск, а не весь ответ; голый массив — тоже список
    let bare = parse::search(r#"[{"Name":"без номера"},{"Id":3,"Name":"есть"}, 5]"#).unwrap();
    assert_eq!((bare.found.len(), bare.found[0].id, bare.total), (1, 3, 1));
}

#[test]
fn live_list() {
    // живой (2026-10-12): даты без пояса, с микросекундами; номера; Closed, ServiceId, Type; счёт упирается в тысячу
    let list = parse::search(r#"{"Tasks":[{"Assets":null,"Categories":null,"Changed":"2026-10-12T11:13:02.945214","Closed":"2026-10-12T11:13:02.894457","Created":"2026-10-12T07:53:54.984246","Creator":"Петрова Анна Сергеевна","EditorId":41404,"CreatorIP":"10.0.0.1","CreatorId":41404,"Data":"<field id=\"1564\"></field><field id=\"3061\">SHIPMENT</field>","Deadline":"2026-10-12T11:15:00","Description":"прошу помочь\r\nс реализацией","Evaluation":"Отлично","EvaluationId":4,"ExecutorGroup":null,"ExecutorGroupId":null,"Files":"800001| Screenshot 2026-10-12 8 52 05.png,800002| реализация.png","Hours":null,"Id":700101,"IsMassIncident":false,"Name":"не проводится реализация","ParentId":null,"Price":null,"PriorityId":12,"ReactionDate":"2026-10-12T09:15:00","ReactionDateFact":"2026-10-12T07:55:00","ReactionOverdue":false,"ResolutionDateFact":"2026-10-12T09:15:00","ResolutionOverdue":false,"ServiceId":69,"ServiceStage":null,"ServiceStageId":null,"StatusId":28,"TaskRepeatRuleId":null,"Type":"Запрос на обслуживание","TypeId":1009,"ExecutorIds":"40620, 18621, 40536","Executors":"Куратор 1С, Куратор ФТО, Сидорова Мария Ивановна","ObserverIds":"32275, 40536","Observers":"Кузнецова Ольга Петровна, Сидорова Мария Ивановна","CoordinatorIds":"","Coordinators":"","FileIds":"800001, 800002"},{"Assets":null,"Categories":null,"Changed":"2026-10-12T11:10:32.789819","Closed":null,"Created":"2026-10-12T10:31:08.270431","Creator":"Смирнова Елена Викторовна","EditorId":32275,"CreatorId":28471,"Description":"Добрый день! Просьба подцепить спеццены","ExecutorGroup":null,"Files":null,"Id":700102,"Name":"Не прикрепляются спец.цены","ServiceId":69,"StatusId":38,"Type":"Запрос на обслуживание","TypeId":1009,"ExecutorIds":"32275","Executors":"Кузнецова Ольга Петровна","ObserverIds":"30415","Observers":"Орлова Ксения Александровна","CoordinatorIds":"","Coordinators":"","FileIds":""}],"Priorities":[],"Services":[],"Statuses":[],"Paginator":{"Count":1000,"Page":1,"PageCount":500,"PageSize":2,"CountOnPage":2,"HasNextPage":null}}"#).unwrap();
    assert_eq!((list.total, list.has_next, list.found.len()), (1000, None, 2));
    assert!(capped(list.total) && !capped(999));
    let f = &list.found[0];
    assert_eq!((f.id, f.creator_id, f.executor_ids.clone()), (700101, Some(41404), Some(vec![40620, 18621, 40536])));
    assert_eq!(f.executors.as_deref(), Some("Куратор 1С, Куратор ФТО, Сидорова Мария Ивановна"));
    assert_eq!(f.description.as_deref(), Some("прошу помочь\nс реализацией"));
    let x = f.extra.clone().unwrap();
    assert_eq!((x.task_type.as_deref(), x.categories.as_deref()), (Some("Запрос на обслуживание"), Some("")));
    assert_eq!(f.changed, Some(local(2026, 10, 12, 11, 13, 2, 945_214_000)));
    assert_eq!(x.closed, Some(local(2026, 10, 12, 11, 13, 2, 894_457_000)));
    assert_eq!((list.found[1].id, list.found[1].creator_id, list.found[1].executor_ids.clone()), (700102, Some(28471), Some(vec![32275])));
    // наблюдатели — номерами и именами
    assert_eq!(
        (f.observer_ids.clone(), f.observers.as_deref()),
        (Some(vec![32275, 40536]), Some("Кузнецова Ольга Петровна, Сидорова Мария Ивановна"))
    );
    let g = &list.found[1];
    assert_eq!((g.observer_ids.clone(), g.observers.as_deref()), (Some(vec![30415]), Some("Орлова Ксения Александровна")));
}

#[test]
fn rights() {
    // права (include=usertaskrights): блок UserTaskRights или Rights, рядом с Task или в нём; ToStatuses — строкой номеров,
    // массивом номеров или объектов с Id; нет блока или списка — None (решит сервер), пустой список — переходов нет
    assert_eq!(parse::rights(r#"{"Task":{"Id":5},"UserTaskRights":{"ToStatuses":"29, 46"}}"#), Some(vec![29, 46]));
    assert_eq!(parse::rights(r#"{"Task":{"Id":5},"Rights":{"ToStatuses":[29,46]}}"#), Some(vec![29, 46]));
    assert_eq!(
        parse::rights(r#"{"Task":{"Id":5,"UserTaskRights":{"ToStatuses":[{"Id":29,"Name":"Выполнена"},{"Id":46}]}}}"#),
        Some(vec![29, 46])
    );
    assert_eq!(parse::rights(r#"{"Task":{"Id":5},"Rights":{"ToStatuses":""}}"#), Some(vec![]));
    assert_eq!(parse::rights(r#"{"Task":{"Id":5},"Rights":null}"#), None);
    // список null — «не прислал», а не «переходов нет»; блок дальше ещё ищется
    assert_eq!(parse::rights(r#"{"Task":{"Id":5},"UserTaskRights":{"ToStatuses":null}}"#), None);
    assert_eq!(parse::rights(r#"{"Task":{"Id":5,"Rights":{"ToStatuses":[46]}},"UserTaskRights":{"ToStatuses":null}}"#), Some(vec![46]));
    assert_eq!(parse::rights(r#"{"Task":{"Id":5},"UserTaskRights":{"CanEdit":true}}"#), None);
    assert_eq!(parse::rights(r#"{"Task":{"Id":5}}"#), None);
    // в массиве: число со знаком — номер (`TryGetInt32`), строка — только цифры; список не массив и не строка — не знаем
    assert_eq!(parse::rights(r#"{"Rights":{"ToStatuses":[-3,"4","+5"," 6",true]}}"#), Some(vec![-3, 4]));
    assert_eq!(parse::rights(r#"{"Rights":{"ToStatuses":29}}"#), None);
}

#[test]
fn references() {
    // сервисы в обёртке с Paginator, фильтры голым массивом, сотрудники без обёртки; без номера и названия — пропуск
    let services = parse::refs(r#"{"ServiceList":{"Services":[{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|","IsArchive":false},{"Id":840,"Name":"ТСД","Path":"840|","IsArchive":"True"},{"Id":9,"Name":""},{"Name":"без номера"}],"Paginator":{"Count":3,"Page":1,"PageCount":2}}}"#, "Services", "ServiceList").unwrap();
    assert_eq!((services.total, services.has_more, services.items.len()), (3, true, 2));
    assert_eq!(
        services.items[0],
        Ref {
            id: 844, name: "Приложение на ТСД".into(), path: Some("840|844|".into()), is_archive: false, is_default: false
        }
    );
    assert_eq!((services.items[1].id, services.items[1].is_archive), (840, true));
    let filters = parse::refs(
        r#"[{"Id":106,"IsCommon":false,"IsDefault":true,"Name":"1. Инциденты"},{"Id":107,"IsDefault":false,"Name":"2. Проблемы"}]"#,
        "FilterView",
        "ArrayOfFilterView",
    )
    .unwrap();
    assert_eq!((filters.total, filters.has_more, filters.items[0].is_default, filters.items[1].is_default), (2, false, true, false));
    let users =
        parse::refs(r#"{"Users":[{"Id":45,"Name":"Иванов И."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"#, "Users", "UserList")
            .unwrap();
    assert_eq!((users.total, users.has_more, users.items[0].name.as_str()), (1, false, "Иванов И."));
    assert!(parse::refs(r#"{"Message":"The request is invalid."}"#, "Users", "UserList").is_none());
    // сервисы из списка заявок (include=service): только блок Services; строки — заявки, их не путать с сервисами
    let ts = parse::task_services(r#"{"TaskList":{"Tasks":[{"Id":7,"Name":"C","ServiceId":844},{"Id":8,"Name":"D","ServiceId":850}],"Services":[{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"},{"Id":850,"Name":"Принтеры"},{"Id":9}]}}"#).unwrap();
    assert_eq!((ts.rows, ts.services.len()), (2, 2));
    assert_eq!(ts.services[0], Ref { id: 844, name: "Приложение на ТСД".into(), path: Some("840|844|".into()), ..Ref::default() });
    let none = parse::task_services(r#"{"Tasks":[{"Id":7,"Name":"C"}]}"#).unwrap();
    assert_eq!((none.rows, none.services.len()), (1, 0), "блока нет — сервисов нет");
    assert!(
        parse::task_services(r#"[{"Id":7,"Name":"Заявка, а не сервис"}]"#).is_none()
            && parse::task_services(r#"{"Message":"x"}"#).is_none()
    );
}

#[test]
fn current_user() {
    // пример из документации (стр. 57): корень там назван <CurrenUserInfo> — понимаем оба написания и голый объект
    assert_eq!(
        parse::current_user(
            r#"{"CompanyId":30,"DefaultTaskFilterId":106,"Email":"test@test.ru","Id":1,"IsArchive":false,"Language":"ru",
             "Login":"admin","Name":"Администратор","RoleId":37,"RoleType":1,"UtcOffset":"+03:00"}"#
        ),
        Some(User { id: 1, name: "Администратор".into() })
    );
    assert_eq!(
        parse::current_user(r#"{"CurrenUserInfo":{"Id":1,"Login":"admin","Name":"Администратор","RoleType":1}}"#).map(|u| u.id),
        Some(1)
    );
    assert_eq!(parse::current_user(r#"{"CurrentUserInfo":{"Id":44,"Login":"test1"}}"#), Some(User { id: 44, name: String::new() }));
    assert_eq!(parse::current_user(r#"{"Message":"The request is invalid."}"#), None);
    assert_eq!(parse::current_user(r#"{"Id":0,"Name":"никто"}"#), None, "нулевой номер — не пользователь");
    // живой — голый объект
    assert_eq!(
        parse::current_user(
            r#"{"Id":40800,"UtcOffset":"+03:00","Language":"Ru","Email":"ivanov@example.ru","IsArchive":false,"Login":"ivanovii","MobilePhone":null,"Name":"Иванов Иван Иванович","Phone":null,"Position":"Специалист технической поддержки","RoleId":55,"RoleType":1,"Comments":"ООО Пример / ИТ / ","CompanyId":179,"DefaultTaskFilterId":null,"Phones":[]}"#
        ),
        Some(User { id: 40800, name: "Иванов Иван Иванович".into() })
    );
}

#[test]
fn statuses() {
    // пример из документации (стр. 39): голый массив; признаки и булевыми, и строкой; строка без номера пропадает
    let s = parse::statuses(
        r#"[{"Id":31,"Name":"Открыта","IsCommentRequired":false,"IsFinal":false,"IsFixed":false,"IsInitial":true},
         {"Id":29,"Name":"Выполнена","IsFixed":"True","IsFinal":"False"},
         {"Id":30,"Name":"Закрыта","IsFinal":true},
         {"Name":"Без номера","IsFixed":true}]"#,
    )
    .unwrap();
    assert_eq!(s.len(), 3);
    assert_eq!(s[0], Status { id: 31, name: "Открыта".into(), ..Status::default() });
    assert_eq!(s[1], Status { id: 29, name: "Выполнена".into(), is_fixed: true, ..Status::default() });
    assert_eq!((s[2].id, s[2].name.as_str(), s[2].is_fixed, s[2].is_final), (30, "Закрыта", false, true));
    // обёртки: из xml-документации в лоб и «как блок Statuses»
    assert_eq!(parse::statuses(r#"{"ArrayOfTaskStatusView":{"TaskStatusView":[{"Id":7,"Name":"Открыта"}]}}"#).map(|s| s.len()), Some(1));
    assert_eq!(parse::statuses(r#"{"Statuses":[{"Id":7,"Name":"Открыта"}]}"#).map(|s| s.len()), Some(1));
    assert_eq!(parse::statuses(r#"{"Message":"The request is invalid."}"#), None);
    // «True» строкой — без учёта регистра и пробелов по краям, как `bool.TryParse`
    let s = parse::statuses(r#"[{"Id":1,"Name":"a","IsFinal":" true "},{"Id":2,"Name":"b","IsFinal":"да"}]"#).unwrap();
    assert_eq!((s[0].is_final, s[1].is_final), (true, false));

    // живой справочник: в какой статус нужен комментарий; закрытые по правилу программы — по признакам и по названию
    let live = parse::statuses(r#"[{"Id":31,"Name":"Открыта","Description":"Заявка для регистрации","Image16Url":"https://hd/img/statuses/actual16.png","IsCommentRequired":false,"IsFinal":false,"IsFixed":false,"IsInitial":true,"IsConcurrence":false,"IsExternal":false,"IsDeadlineAccountingPaused":false,"Changed":"2016-02-08T17:04:58.76"},{"Id":29,"Name":"Выполнена","IsFinal":false,"IsFixed":true},{"Id":28,"Name":"Закрыта","IsFinal":true,"IsFixed":true},{"Id":30,"Name":"Отменена","IsFinal":true,"IsFixed":false},{"Id":46,"Name":"Ожидание ответа пользователя","IsCommentRequired":true,"IsFinal":false,"IsFixed":false},{"Id":55,"Name":"Отклонена","IsFinal":true,"IsFixed":false},{"Id":88,"Name":"Утверждена ТК","IsFinal":true,"IsFixed":true},{"Id":120,"Name":"Ожидание ответа с автозакрытием","IsFinal":false,"IsFixed":false}]"#).unwrap();
    assert_eq!(live.iter().filter(|s| s.is_comment_required).map(|s| s.id).collect::<Vec<_>>(), [46]);
    let closed = ClosedNames::new(&["Выполнена", "Ожидание ответа с автозакрытием", "Закрыта", "Отменена"].map(String::from));
    let mut ids: Vec<i32> = live.iter().filter(|s| s.closed_by(&closed)).map(|s| s.id).collect();
    ids.sort();
    assert_eq!(ids, [28, 29, 30, 55, 88, 120], "закрытые — 28, 29, 30, 55, 88 по признакам и 120 по названию");
}

#[test]
fn dates() {
    // три вида: документации (сначала: иначе 12.11 прочиталось бы как 11 декабря), ISO и WCF; непонятная — None
    assert_eq!(parse::date("12.11.2015 13:44"), Some(local(2015, 11, 12, 13, 44, 0, 0)));
    assert_eq!(parse::date("12.11.2015"), Some(local(2015, 11, 12, 0, 0, 0, 0)));
    assert_eq!(parse::date(" 2026-10-12T11:13:02 "), Some(local(2026, 10, 12, 11, 13, 2, 0)));
    assert_eq!(parse::date("2026-10-12 11:13"), Some(local(2026, 10, 12, 11, 13, 0, 0)));
    assert_eq!(parse::date("2026-10-12"), Some(local(2026, 10, 12, 0, 0, 0, 0)));
    // пояс — свой; доли — до десятимиллионных (тиков), дальше — округление до тика
    let utc = |h, mi, s, nanos| Utc.with_ymd_and_hms(2026, 10, 12, h, mi, s).unwrap().fixed_offset() + chrono::Duration::nanoseconds(nanos);
    assert_eq!(parse::date("2026-10-12T11:13:02Z"), Some(utc(11, 13, 2, 0)));
    assert_eq!(parse::date("2026-10-12T14:13:02+03:00"), Some(utc(11, 13, 2, 0)));
    assert_eq!(parse::date("2026-10-12T14:13:02.12345678+03:00"), Some(utc(11, 13, 2, 123_456_800)));
    assert_eq!(parse::date("/Date(-1000)/"), Some(Utc.with_ymd_and_hms(1969, 12, 31, 23, 59, 59).unwrap().fixed_offset()));
    assert_eq!(parse::date("/Date(1447335893000+0300)/"), Some(Utc.with_ymd_and_hms(2015, 11, 12, 13, 44, 53).unwrap().fixed_offset()));
    // пояс: минус, без двоеточия, одни часы, крайние значения; минуты пояса считаются
    for (text, h, mi) in [
        ("2026-10-12T08:13:02-03:00", 11, 13),
        ("2026-10-12T14:13:02+0300", 11, 13),
        ("2026-10-12T14:13:02+03", 11, 13),
        ("2026-10-12T16:43:02+05:30", 11, 13),
        ("2026-10-13T01:13:02+14:00", 11, 13),
        ("2026-10-12T15:12:02+03:59", 11, 13),
    ] {
        assert_eq!(parse::date(text), Some(utc(h, mi, 2, 0)), "{text}");
    }
    // доли: ровно семь знаков; восьмой — округление до тика: больше половины — вверх, меньше — вниз, ровно половина — к чётному
    for (fraction, nanos) in [
        ("1234567", 123_456_700),
        ("12345671", 123_456_700),
        ("123456851", 123_456_900),
        ("12345685", 123_456_800),
        ("12345675", 123_456_800),
        ("123456750000", 123_456_800),
    ] {
        assert_eq!(parse::date(&format!("2026-10-12T11:13:02.{fraction}Z")), Some(utc(11, 13, 2, nanos)), "{fraction}");
    }
    // Paginator массивом — как в одном примере документации
    let p = parse::search(r#"{"Tasks":[{"Id":1,"Name":"a"}],"Paginator":[{"Count":5,"Page":1,"PageCount":2}]}"#).unwrap();
    assert_eq!(p.total, 5);
    // пустой ServiceName — не название: берётся из блока Services
    let p = parse::search(r#"{"Tasks":[{"Id":7,"Name":"C","ServiceName":"","ServiceId":844}],"Services":[{"Id":844,"Name":"Принтеры"}]}"#)
        .unwrap();
    assert_eq!(p.found[0].extra.clone().and_then(|x| x.service).as_deref(), Some("Принтеры"));
    // (WCF за пределами DateTimeOffset — сразу None, как в C#: другие виды не пробуются)
    for bad in [
        "",
        "вчера",
        "31.02.2026",
        "12.11.2015 25:00",
        "/Date(abc)/",
        "/Date(99999999999999999)/",
        // WCF: за числом — только ±чччч
        "/Date(1447335893000+03)/",
        "/Date(1447335893000x0300)/",
        "/Date(1000x)/",
        // формат документации — ровно: точки на местах, длина, пробел и двоеточие у времени, год от первого
        "12.11-2015",
        "12.11.20151",
        "12.11.2015T13:44",
        "01.01.0000",
        // ISO — так же
        "2026",
        "2026-10/12",
        "2026-10-12T11",
        "2026-10-12T11-13",
        "0000-01-01",
        "2026-10-12T14:13:02+15:00",
        "2026-10-12T14:13:02+03:60",
        "2026-10-12T14:13:02+03x00",
        "2026-10-12T14:13:02 MSK",
    ] {
        assert_eq!(parse::date(bad), None, "{bad}");
    }
}
