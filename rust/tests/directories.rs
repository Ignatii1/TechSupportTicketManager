//! Справочники окна поиска (`HttpIntraserviceClient.GetServicesAsync`, `GetTaskTypesAsync`, `GetSavedFiltersAsync`):
//! какие запросы уходят, какой список и какое пояснение получает окно (карточка 3.3-1).

mod common;

use common::{Reply, route};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ticketboard::intraservice::{Client, Ref, Services};

const FULL: &str = "/api/service?fields=Id,Name,Path,IsArchive&archive=true&inactive=true&pagesize=1000&page=1";
const OWN: &str = "/api/service?fields=Id,Name,Path,IsArchive&archive=true&inactive=true&for=filtertasks&pagesize=1000&page=1";
const TASKS_UNCOUNTED: &str = "/api/task?archive=true&inactive=true&include=status,service&count=false&sort=Changed%20desc&pagesize=1000&page=1&fields=Id,Name,ServiceId";
const TASKS_COUNTED: &str =
    "/api/task?archive=true&inactive=true&include=status,service&sort=Changed%20desc&pagesize=1000&page=1&fields=Id,Name,ServiceId";

const DENIED: &str = r#"{"Message":"Нет прав на просмотр списка сервисов"}"#;

fn client(url: &str) -> Client {
    Client::new(url, "u", "p", Duration::from_secs(10))
}

fn paths(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock().unwrap().iter().map(|h| h.split(' ').nth(1).unwrap_or("").to_string()).collect()
}

fn denied() -> Reply {
    Reply::new(403, "application/json", DENIED)
}

fn empty(code: u16) -> Reply {
    Reply::new(code, "text/plain", "")
}

/// Страница справочника сервисов: (номер, название, путь, архивный).
fn services_page(rows: &[(i32, &str, &str, bool)], page: i32, pages: i32) -> Reply {
    let list: Vec<String> = rows
        .iter()
        .map(|(id, name, path, archive)| format!(r#"{{"Id":{id},"Name":"{name}","Path":"{path}","IsArchive":{archive}}}"#))
        .collect();
    Reply::json(&format!(
        r#"{{"ServiceList":{{"Services":[{}],"Paginator":{{"Count":{},"Page":{page},"PageCount":{pages}}}}}}}"#,
        list.join(","),
        rows.len()
    ))
}

const NO_SERVICES: &str = r#"{"ServiceList":{"Services":[],"Paginator":{"Count":0,"Page":1,"PageCount":0}}}"#;

fn tsd() -> Reply {
    services_page(&[(840, "ТСД", "840|", false), (844, "Приложение на ТСД", "840|844|", false)], 1, 1)
}

fn ids(items: &[Ref]) -> Vec<i32> {
    items.iter().map(|r| r.id).collect()
}

fn fail(error: &str) -> Services {
    Services { items: Vec::new(), error: error.into(), note: String::new() }
}

// --- сервисы: полный справочник ---

#[test]
fn the_full_reference_is_one_request_and_needs_no_note() {
    let (url, seen) = route(|_| services_page(&[(840, "ТСД", "840|", false), (9, "Почта", "9|", true)], 1, 1));
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL]);
    assert_eq!((s.error.as_str(), s.note.as_str()), ("", ""));
    assert_eq!(
        s.items,
        vec![
            Ref { id: 840, name: "ТСД".into(), path: Some("840|".into()), is_archive: false, is_default: false },
            Ref { id: 9, name: "Почта".into(), path: Some("9|".into()), is_archive: true, is_default: false },
        ]
    );
}

#[test]
fn later_pages_are_read_while_the_server_promises_them() {
    let (url, seen) = route(|path| {
        if path.ends_with("page=2") {
            services_page(&[(844, "Приложение на ТСД", "840|844|", false)], 2, 2)
        } else {
            services_page(&[(840, "ТСД", "840|", false)], 1, 2)
        }
    });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL.to_string(), FULL.replace("page=1", "page=2")]);
    assert_eq!(ids(&s.items), vec![840, 844]);
    assert_eq!((s.error.as_str(), s.note.as_str()), ("", ""));
}

#[test]
fn an_empty_page_ends_the_reading_even_if_more_is_promised() {
    let (url, seen) =
        route(|path| if path.ends_with("page=1") { services_page(&[(840, "ТСД", "840|", false)], 1, 3) } else { services_page(&[], 2, 3) });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen).len(), 2, "третью страницу не просят");
    assert_eq!((ids(&s.items), s.error.as_str(), s.note.as_str()), (vec![840], "", ""));
}

#[test]
fn the_reference_is_read_by_five_pages_at_most() {
    let (url, seen) = route(|path| {
        let page: i32 = path.rsplit("page=").next().unwrap().parse().unwrap();
        services_page(&[(100 + page, "Сервис", "", false)], page, 99)
    });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen).len(), 5);
    assert_eq!(ids(&s.items), vec![101, 102, 103, 104, 105]);
    assert_eq!((s.error.as_str(), s.note.as_str()), ("", ""));
}

// --- сервисы: запасные списки ---

#[test]
fn a_refused_reference_falls_back_to_the_services_assigned_to_me() {
    let (url, seen) = route(|path| if path.contains("for=filtertasks") { tsd() } else { denied() });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL, OWN]);
    assert_eq!(ids(&s.items), vec![840, 844]);
    assert_eq!(s.error, "");
    assert_eq!(s.note, "Весь список сервисов сервер не отдал (нет доступа (HTTP 403)) — здесь те, на которые вы назначены.");
}

#[test]
fn an_empty_reference_is_not_an_answer_but_a_reason_to_fall_back() {
    let (url, seen) = route(|path| if path.contains("for=filtertasks") { tsd() } else { Reply::json(NO_SERVICES) });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL, OWN]);
    assert_eq!(ids(&s.items), vec![840, 844]);
    assert_eq!(s.note, "Весь список сервисов сервер не отдал (пустой ответ) — здесь те, на которые вы назначены.");
}

#[test]
fn an_unreadable_reference_falls_back_with_the_headline_of_the_error() {
    let (url, _) = route(|path| if path.contains("for=filtertasks") { tsd() } else { Reply::json("это не json") });
    let s = client(&url).get_services();
    assert_eq!(ids(&s.items), vec![840, 844]);
    assert_eq!(s.note, "Весь список сервисов сервер не отдал (непонятный ответ сервера) — здесь те, на которые вы назначены.");
}

#[test]
fn a_500_is_a_refusal_not_a_blip_and_gets_the_fallbacks() {
    let (url, seen) = route(|path| {
        if path.contains("for=filtertasks") {
            tsd()
        } else {
            Reply::new(500, "application/json", r#"{"Message":"An error has occurred."}"#)
        }
    });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL, OWN]);
    assert_eq!(ids(&s.items), vec![840, 844]);
    assert_eq!(s.note, "Весь список сервисов сервер не отдал (ошибка сервера (HTTP 500)) — здесь те, на которые вы назначены.");
}

#[test]
fn a_failure_on_a_later_page_is_not_a_half_list() {
    let (url, seen) = route(|path| {
        if path.contains("for=filtertasks") {
            tsd()
        } else if path.ends_with("page=2") {
            empty(500)
        } else {
            services_page(&[(1, "Только первая страница", "1|", false)], 1, 2)
        }
    });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen).len(), 3, "первая, вторая (сбой), назначенные");
    assert_eq!(ids(&s.items), vec![840, 844], "список назначенных, а не половина полного");
    assert!(s.note.contains("(ошибка сервера (HTTP 500))"), "{}", s.note);
}

// --- сервисы: сбой, который пройдёт сам, и неверный логин — без запасных списков ---

#[test]
fn a_blip_stops_at_once_with_no_fallbacks_and_no_note() {
    for code in [502, 503, 504, 408, 429] {
        let (url, seen) = route(move |_| empty(code));
        let s = client(&url).get_services();
        assert_eq!(s, fail(&format!("ошибка сервера (HTTP {code})")), "код {code}");
        assert_eq!(paths(&seen), vec![FULL], "код {code}: запасных списков не просим");
    }
}

#[test]
fn a_wrong_login_stops_at_once_with_no_fallbacks() {
    let (url, seen) = route(|_| empty(401));
    assert_eq!(client(&url).get_services(), fail("неверный логин или пароль (HTTP 401)"));
    assert_eq!(paths(&seen), vec![FULL]);
}

#[test]
fn an_unreachable_server_is_a_blip_too() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let s = client(&format!("http://127.0.0.1:{port}/")).get_services();
    assert!(s.items.is_empty() && s.note.is_empty());
    assert!(s.error.starts_with("сервер недоступен"), "{}", s.error);
    assert!(!s.error.contains("сервисы из заявок"), "запасные списки не пробовали: {}", s.error);
}

#[test]
fn a_timeout_is_a_blip_too() {
    let (url, _) = route(|_| {
        let mut r = tsd();
        r.delay = Duration::from_millis(900);
        r
    });
    let s = Client::new(&url, "u", "p", Duration::from_millis(300)).get_services();
    assert_eq!(s, fail("сервер не ответил за 0,3 с"));
}

#[test]
fn a_blip_on_the_assigned_list_stops_before_the_latest_tickets() {
    let (url, seen) = route(|path| if path.contains("for=filtertasks") { empty(503) } else { denied() });
    assert_eq!(client(&url).get_services(), fail("ошибка сервера (HTTP 503)"));
    assert_eq!(paths(&seen), vec![FULL, OWN]);
}

#[test]
fn a_wrong_login_on_the_assigned_list_stops_before_the_latest_tickets() {
    let (url, seen) = route(|path| if path.contains("for=filtertasks") { empty(401) } else { denied() });
    assert_eq!(client(&url).get_services(), fail("неверный логин или пароль (HTTP 401)"));
    assert_eq!(paths(&seen), vec![FULL, OWN]);
}

// --- сервисы: из последних заявок ---

fn latest_tickets() -> Reply {
    Reply::json(
        r#"{"Tasks":[{"Id":701,"Name":"П","ServiceId":844},{"Id":702,"Name":"В","ServiceId":850},{"Id":703,"Name":"Т","ServiceId":1}],
"Services":[{"Id":850,"Name":"Принтеры"},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|"},{"Id":2,"Name":"Бета"},
{"Id":844,"Name":"Дубль"},{"Id":1,"Name":"альфа"}],"Paginator":{"Page":1,"HasNextPage":false}}"#,
    )
}

fn only_tickets(tasks: Reply) -> impl Fn(&str) -> Reply + Send + 'static {
    let tasks = Arc::new(Mutex::new(Some(tasks)));
    move |path| {
        if path.starts_with("/api/task?") { tasks.lock().unwrap().take().unwrap_or_else(|| empty(500)) } else { denied() }
    }
}

#[test]
fn without_any_reference_the_services_come_from_the_latest_tickets() {
    let (url, seen) = route(only_tickets(latest_tickets()));
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL, OWN, TASKS_UNCOUNTED]);
    assert_eq!(s.error, "");
    assert_eq!(
        s.note,
        "Весь список сервисов сервер не отдал (нет доступа (HTTP 403)) — здесь сервисы последних 3 заявок (вложенные — лишь те, что в списке)."
    );
    // по названию без учёта регистра («альфа» перед «Бета»), без повторов номера (остаётся первая запись)
    assert_eq!(ids(&s.items), vec![1, 2, 844, 850]);
    assert_eq!(s.items[2].name, "Приложение на ТСД");
}

#[test]
fn the_latest_tickets_are_asked_again_with_a_count_when_count_false_is_refused() {
    let (url, seen) = route(|path| {
        if path.contains("count=false") {
            Reply::new(400, "application/json", r#"{"errors":{"count":["The value 'false' is not valid."]},"status":400}"#)
        } else if path.starts_with("/api/task?") {
            latest_tickets()
        } else {
            denied()
        }
    });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen), vec![FULL, OWN, TASKS_UNCOUNTED, TASKS_COUNTED]);
    assert_eq!(ids(&s.items), vec![1, 2, 844, 850]);
    assert_eq!(s.error, "");
}

#[test]
fn only_a_400_gets_the_counted_retry_and_the_error_tells_both_stories() {
    let (url, seen) = route(only_tickets(empty(500)));
    let s = client(&url).get_services();
    assert_eq!(paths(&seen).len(), 3, "после 500 второй попытки со счётом нет");
    assert_eq!(s, fail("нет доступа (HTTP 403); сервисы из заявок — ошибка сервера (HTTP 500)"));
}

#[test]
fn the_latest_tickets_page_may_take_thirty_seconds_not_the_usual_few() {
    // общий срок клиента — 0,5 с; страница заявок отвечает через 1,2 с и всё же читается: у неё свой срок, 30 с
    let (url, _) = route(|path| {
        if path.starts_with("/api/task?") {
            let mut r = latest_tickets();
            r.delay = Duration::from_millis(1200);
            r
        } else {
            denied()
        }
    });
    let s = Client::new(&url, "u", "p", Duration::from_millis(500)).get_services();
    assert_eq!((ids(&s.items), s.error.as_str()), (vec![1, 2, 844, 850], ""));
}

#[test]
fn nothing_anywhere_and_no_errors_is_an_answer_with_a_hint() {
    let (url, seen) = route(|path| {
        if path.starts_with("/api/task?") {
            Reply::json(r#"{"Tasks":[],"Paginator":{"Page":1,"HasNextPage":false}}"#)
        } else {
            Reply::json(NO_SERVICES)
        }
    });
    let s = client(&url).get_services();
    assert_eq!(paths(&seen).len(), 3);
    assert_eq!(
        s,
        Services {
            items: Vec::new(),
            error: String::new(),
            note: "Сервер не вернул ни одного сервиса — ни списком, ни в заявках. Сервис можно задать сохранённым фильтром.".into()
        }
    );
}

#[test]
fn when_no_list_helps_the_error_of_the_full_reference_comes_first() {
    let (url, _) = route(|path| {
        if path.starts_with("/api/service?") && !path.contains("for=filtertasks") {
            empty(404)
        } else if path.starts_with("/api/task?") {
            Reply::json(r#"{"Tasks":[],"Paginator":{"Page":1,"HasNextPage":false}}"#)
        } else {
            Reply::json(NO_SERVICES)
        }
    });
    assert_eq!(client(&url).get_services(), fail("по этому адресу нет API (HTTP 404)"));
}

#[test]
fn when_no_list_helps_the_error_of_the_assigned_list_comes_if_the_full_one_had_none() {
    let (url, _) = route(|path| {
        if path.contains("for=filtertasks") {
            empty(403)
        } else if path.starts_with("/api/task?") {
            Reply::json(r#"{"Tasks":[],"Paginator":{"Page":1,"HasNextPage":false}}"#)
        } else {
            Reply::json(NO_SERVICES)
        }
    });
    assert_eq!(client(&url).get_services(), fail("нет доступа (HTTP 403)"));
}

// --- типы заявок ---

#[test]
fn task_types_are_one_counted_page_with_archived_ones() {
    let (url, seen) = route(|_| {
        Reply::json(
            r#"{"TaskTypeList":{"TaskTypes":[{"Id":1009,"Name":"Запрос на обслуживание"},{"Id":3,"Name":"Инцидент","IsArchive":true}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}"#,
        )
    });
    let types = client(&url).get_task_types().unwrap();
    assert_eq!(paths(&seen), vec!["/api/tasktype?fields=Id,Name,IsArchive&archive=true&pagesize=1000&page=1"]);
    assert_eq!(
        types,
        vec![
            Ref { id: 1009, name: "Запрос на обслуживание".into(), path: None, is_archive: false, is_default: false },
            Ref { id: 3, name: "Инцидент".into(), path: None, is_archive: true, is_default: false },
        ]
    );
}

#[test]
fn task_types_follow_the_pages() {
    let (url, seen) = route(|path| {
        let (page, id) = if path.ends_with("page=2") { (2, 3) } else { (1, 1009) };
        Reply::json(&format!(
            r#"{{"TaskTypeList":{{"TaskTypes":[{{"Id":{id},"Name":"Тип {id}"}}],"Paginator":{{"Count":2,"Page":{page},"PageCount":2}}}}}}"#
        ))
    });
    let types = client(&url).get_task_types().unwrap();
    assert_eq!(ids(&types), vec![1009, 3]);
    assert_eq!(paths(&seen).len(), 2);
}

#[test]
fn task_types_fail_with_the_text_of_the_server_error() {
    let (url, _) = route(|_| empty(404));
    assert_eq!(client(&url).get_task_types(), Err("по этому адресу нет API (HTTP 404)".to_string()));
    let (url, _) = route(|_| Reply::json("это не json"));
    assert!(client(&url).get_task_types().unwrap_err().starts_with("непонятный ответ сервера:"));
}

#[test]
fn a_failure_on_a_later_page_of_task_types_is_an_error() {
    let (url, _) = route(|path| {
        if path.ends_with("page=2") {
            empty(500)
        } else {
            Reply::json(r#"{"TaskTypeList":{"TaskTypes":[{"Id":1,"Name":"Тип"}],"Paginator":{"Count":2,"Page":1,"PageCount":2}}}"#)
        }
    });
    assert_eq!(client(&url).get_task_types(), Err("ошибка сервера (HTTP 500)".to_string()));
}

// --- сохранённые фильтры ---

#[test]
fn saved_filters_are_one_request_without_paging_parameters() {
    let (url, seen) =
        route(|_| Reply::json(r#"[{"Id":45,"IsDefault":false,"Name":"Мои заявки"},{"Id":46,"IsDefault":true,"Name":"Основной"}]"#));
    let filters = client(&url).get_saved_filters().unwrap();
    assert_eq!(paths(&seen), vec!["/api/filter?resource=task"]);
    assert_eq!(
        filters,
        vec![
            Ref { id: 45, name: "Мои заявки".into(), path: None, is_archive: false, is_default: false },
            Ref { id: 46, name: "Основной".into(), path: None, is_archive: false, is_default: true },
        ]
    );
}

#[test]
fn saved_filters_are_not_followed_to_a_second_page_even_if_one_is_promised() {
    let (url, seen) = route(|_| {
        Reply::json(
            r#"{"ArrayOfFilterView":{"FilterView":[{"Id":45,"Name":"Мои заявки"}],"Paginator":{"Count":2,"Page":1,"PageCount":3}}}"#,
        )
    });
    let filters = client(&url).get_saved_filters().unwrap();
    assert_eq!(ids(&filters), vec![45]);
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn saved_filters_fail_with_the_text_of_the_server_error() {
    let (url, _) = route(|_| empty(404));
    assert_eq!(client(&url).get_saved_filters(), Err("по этому адресу нет API (HTTP 404)".to_string()));
    let (url, _) = route(|_| Reply::json("это не json"));
    assert!(client(&url).get_saved_filters().unwrap_err().starts_with("непонятный ответ сервера:"));
}
