//! Заявка целиком для текста: вся история по страницам и сборка текста (`KnowledgeExport.ReadLifetimeAsync`,
//! `ReadAsync`/`BuildAsync`; карточка 4.1-2). Поддельный сервер — `common::route`.

mod common;

use chrono::{DateTime, Duration as Span};
use common::{Reply, route};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ticketboard::export::{Context, MAX_LIFETIME_PAGES, build, row_of};
use ticketboard::intraservice::{Client, Date, Event, User, parse};
use ticketboard::rules::ClosedNames;

const FIRST: &str = "/api/tasklifetime?taskid=900&include=status&lastcommentsontop=true&pagesize=50";

fn client(url: &str) -> Client {
    Client::new(url, "u", "p", Duration::from_secs(10))
}

fn paths(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock().unwrap().iter().map(|h| h.split(' ').nth(1).unwrap_or("").to_string()).collect()
}

fn lifetime_paths(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    paths(seen).into_iter().filter(|p| p.starts_with("/api/tasklifetime")).collect()
}

/// Номер страницы в адресе запроса: без `page` — первая.
fn page_of(path: &str) -> i32 {
    path.split("&page=").nth(1).and_then(|n| n.parse().ok()).unwrap_or(1)
}

fn moment(n: i32) -> Date {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+03:00").unwrap() + Span::seconds(n.into())
}

/// Запись с номером n: у каждой своё время и текст, так что ни одна не повторяет другую.
fn event_json(n: i32) -> String {
    format!(r#"{{"Date":"{}","Editor":"Иванов Иван","EditorId":7,"Comments":"запись {n}"}}"#, moment(n).to_rfc3339())
}

/// Страница истории: записи с номерами `from..to` (в этом порядке), `paged` — (страница, всего страниц) или без Paginator.
fn page(from: i32, to: i32, paged: Option<(i32, i32)>) -> Reply {
    let events: Vec<String> = (from..to).map(event_json).collect();
    let paginator = paged.map_or(String::new(), |(p, of)| format!(r#","Paginator":{{"Count":{},"Page":{p},"PageCount":{of}}}"#, to - from));
    Reply::json(&format!(r#"{{"TaskLifetimes":[{}]{paginator}}}"#, events.join(",")))
}

fn comments(events: &[Event]) -> Vec<String> {
    events.iter().filter_map(|e| e.comment.clone()).collect()
}

fn texts(numbers: impl IntoIterator<Item = i32>) -> Vec<String> {
    numbers.into_iter().map(|n| format!("запись {n}")).collect()
}

// ---------- read_lifetime ----------

#[test]
fn one_page_when_the_server_says_there_are_no_more() {
    let (url, seen) = route(|_| page(0, 3, Some((1, 1))));
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((comments(&events), truncated), (texts(0..3), false));
    assert_eq!(paths(&seen), [FIRST]);
}

#[test]
fn pages_are_read_while_the_server_says_there_are_more_and_kept_in_the_order_of_the_server() {
    let (url, seen) = route(|path| match page_of(path) {
        1 => page(0, 2, Some((1, 3))),
        2 => page(2, 4, Some((2, 3))),
        _ => page(4, 6, Some((3, 3))),
    });
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((comments(&events), truncated), (texts(0..6), false));
    // первая страница — без page, дальше — по номеру
    assert_eq!(paths(&seen), [FIRST.to_string(), format!("{FIRST}&page=2"), format!("{FIRST}&page=3")]);
}

#[test]
fn a_full_page_of_fifty_with_no_more_pages_is_not_followed_by_a_request() {
    // иначе на заявке ровно в 50 записей просили бы несуществующую страницу
    let (url, seen) = route(|_| page(0, 50, Some((1, 1))));
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((events.len(), truncated), (50, false));
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn without_page_numbers_a_full_page_asks_for_the_next_and_a_short_one_is_the_end() {
    let (url, seen) = route(|path| if page_of(path) == 1 { page(0, 50, None) } else { page(50, 60, None) });
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((comments(&events), truncated), (texts(0..60), false));
    assert_eq!(paths(&seen), [FIRST.to_string(), format!("{FIRST}&page=2")]);
}

#[test]
fn without_page_numbers_a_page_shorter_than_fifty_is_the_end() {
    let (url, seen) = route(|_| page(0, 49, None));
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((events.len(), truncated), (49, false));
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn an_empty_history_is_no_events_and_one_request() {
    let (url, seen) = route(|_| page(0, 0, None));
    assert_eq!(client(&url).read_lifetime(900, 100), Ok((Vec::new(), false)));
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn a_server_that_ignores_the_page_gives_the_same_page_again_and_that_is_the_end() {
    let (url, seen) = route(|_| page(0, 50, None));
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((comments(&events), truncated), (texts(0..50), false));
    assert_eq!(paths(&seen).len(), 2);
}

#[test]
fn a_page_with_nothing_new_ends_the_reading_even_when_the_server_promises_more() {
    let (url, seen) = route(|path| if page_of(path) <= 2 { page(0, 2, Some((page_of(path), 5))) } else { page(100, 102, Some((3, 5))) });
    let (events, truncated) = client(&url).read_lifetime(900, 100).unwrap();
    assert_eq!((comments(&events), truncated), (texts(0..2), false));
    assert_eq!(paths(&seen).len(), 2);
}

#[test]
fn a_record_seen_before_is_skipped_by_date_author_status_and_comment() {
    let record = |date: &str, author: &str, status: &str, comment: &str, editor: i32| {
        format!(r#"{{"Date":"{date}","Editor":"{author}","EditorId":{editor},"StatusName":"{status}","Comments":"{comment}"}}"#)
    };
    let first = [
        record("2026-10-02T10:00:00+03:00", "Иванов", "Открыта", "один", 7),
        record("2026-10-02T09:00:00+03:00", "Иванов", "Открыта", "два", 7),
    ];
    let second = [
        record("2026-10-02T10:00:00+03:00", "Иванов", "Открыта", "один", 99), // тот же ключ, другой номер автора — повтор
        record("2026-10-02T10:00:00+03:00", "Иванов", "В работе", "один", 7), // другой статус — новая
        record("2026-10-02T10:00:00+03:00", "Иванов", "Открыта", "иной", 7),  // другой текст — новая
        record("2026-10-02T10:00:00+03:00", "Петров", "Открыта", "один", 7),  // другой автор — новая
        record("2026-10-02T10:00:01+03:00", "Иванов", "Открыта", "один", 7),  // другое время — новая
    ];
    let (url, _) = route(move |path| {
        let (rows, n) = if page_of(path) == 1 { (&first[..], 1) } else { (&second[..], 2) };
        Reply::json(&format!(r#"{{"TaskLifetimes":[{}],"Paginator":{{"Page":{n},"PageCount":2}}}}"#, rows.join(",")))
    });
    let (events, _) = client(&url).read_lifetime(900, 100).unwrap();
    let got: Vec<(String, String, String)> =
        events.iter().map(|e| (e.author.clone(), e.status.clone(), e.comment.clone().unwrap_or_default())).collect();
    let row = |a: &str, s: &str, c: &str| (a.to_string(), s.to_string(), c.to_string());
    assert_eq!(
        got,
        [
            row("Иванов", "Открыта", "один"),
            row("Иванов", "Открыта", "два"),
            row("Иванов", "В работе", "один"),
            row("Иванов", "Открыта", "иной"),
            row("Петров", "Открыта", "один"),
            row("Иванов", "Открыта", "один"),
        ]
    );
}

#[test]
fn the_page_limit_with_more_pages_left_is_a_truncated_history() {
    let (url, seen) = route(|path| page(page_of(path) * 10, page_of(path) * 10 + 2, Some((page_of(path), 5))));
    let (events, truncated) = client(&url).read_lifetime(900, 2).unwrap();
    assert_eq!((comments(&events), truncated), (texts([10, 11, 20, 21]), true));
    assert_eq!(paths(&seen).len(), 2);
}

#[test]
fn the_page_limit_reached_with_nothing_left_is_not_truncated() {
    let (url, seen) = route(|path| page(page_of(path) * 10, page_of(path) * 10 + 2, Some((page_of(path), 3))));
    let (events, truncated) = client(&url).read_lifetime(900, 3).unwrap();
    assert_eq!((events.len(), truncated), (6, false));
    assert_eq!(paths(&seen).len(), 3);
}

#[test]
fn the_limit_of_one_page_is_truncated_only_when_the_server_has_a_second() {
    let (url, _) = route(|_| page(0, 2, Some((1, 2))));
    assert_eq!(client(&url).read_lifetime(900, 1).map(|(e, t)| (e.len(), t)), Ok((2, true)));
    let (url, _) = route(|_| page(0, 2, Some((1, 1))));
    assert_eq!(client(&url).read_lifetime(900, 1).map(|(e, t)| (e.len(), t)), Ok((2, false)));
    // ни одной страницы не позволено — ничего не читается и не спрашивается
    let (url, seen) = route(|_| page(0, 2, Some((1, 1))));
    assert_eq!(client(&url).read_lifetime(900, 0), Ok((Vec::new(), false)));
    assert!(paths(&seen).is_empty());
}

#[test]
fn without_page_numbers_the_limit_is_truncated_on_a_full_page_and_not_on_a_short_one() {
    let (url, seen) = route(|path| {
        let n = page_of(path);
        page((n - 1) * 50, n * 50, None)
    });
    let (events, truncated) = client(&url).read_lifetime(900, 2).unwrap();
    assert_eq!((events.len(), truncated), (100, true));
    assert_eq!(paths(&seen).len(), 2);
    // вторая страница короткая — это конец, а не урезание
    let (url, _) = route(|path| if page_of(path) == 1 { page(0, 50, None) } else { page(50, 55, None) });
    assert_eq!(client(&url).read_lifetime(900, 2).map(|(e, t)| (e.len(), t)), Ok((55, false)));
}

#[test]
fn an_error_on_the_first_page_is_the_error_of_that_request() {
    let (url, seen) = route(|_| Reply::new(404, "text/plain", ""));
    assert_eq!(client(&url).read_lifetime(900, 100), Err("заявка не найдена (HTTP 404)".to_string()));
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn an_error_on_a_later_page_loses_the_whole_reading_and_is_not_retried() {
    let (url, seen) = route(|path| if page_of(path) == 1 { page(0, 2, Some((1, 3))) } else { Reply::new(404, "text/plain", "") });
    assert_eq!(client(&url).read_lifetime(900, 100), Err("заявка не найдена (HTTP 404)".to_string()));
    assert_eq!(paths(&seen).len(), 2);
    // сбой сервера — тоже ошибка с первого раза: повторов здесь нет
    let (url, seen) = route(|path| if page_of(path) == 1 { page(0, 2, Some((1, 3))) } else { Reply::new(500, "text/plain", "сбой") });
    let result = client(&url).read_lifetime(900, 100);
    assert!(result.as_ref().is_err_and(|e| e.contains("500")), "{result:?}");
    assert_eq!(paths(&seen).len(), 2);
}

#[test]
fn a_page_that_is_not_a_history_is_an_error() {
    let (url, _) = route(|path| if page_of(path) == 1 { page(0, 2, Some((1, 2))) } else { Reply::json(r#"{"Message":"не то"}"#) });
    assert!(client(&url).read_lifetime(900, 100).is_err());
}

// ---------- build ----------

const CARD: &str = r#"{"Task":{"Id":900,"Name":"Тема из карточки","StatusName":"Открыта","Creator":"Петров Пётр","Created":"2026-10-14T11:00:00+03:00","Changed":"2026-10-14T12:00:00+03:00","ServiceName":"Печать","Description":"Описание"}}"#;

fn context() -> Context {
    Context { me: Some(User { id: 7, name: "Иванов Иван".into() }), closed: ClosedNames::default() }
}

fn exported() -> Date {
    DateTime::parse_from_rfc3339("2026-11-17T10:00:00+03:00").unwrap()
}

fn card_row() -> ticketboard::intraservice::Found {
    row_of(&parse::task(CARD, 900).unwrap())
}

const URL: &str = "https://is.example/Task/View/900";

#[test]
fn the_text_of_a_ticket_agrees_with_csharp_on_every_table_row_read_through_the_client() {
    // каждая строка таблицы export-text, где история не урезана: сервер отдаёт её карточку и страницу, текст — как у C#
    let current = Arc::new(Mutex::new((String::new(), String::new())));
    let served = current.clone();
    let (base, _) = route(move |path| {
        let (card, life) = served.lock().unwrap().clone();
        if path.starts_with("/api/tasklifetime") { Reply::json(&life) } else { Reply::json(&card) }
    });
    let c = client(&base);
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (line, cells) in common::parity("export-text") {
        let [list, card, life, me, closed, truncated, expected] = &cells[..] else {
            panic!("export-text.tsv, строка {line}: ячеек не 7")
        };
        if truncated == "да" {
            continue;
        }
        *current.lock().unwrap() = (card.clone(), life.clone());
        let task = parse::task(card, 0).expect("карточка");
        let row = if list == "-" { row_of(&task) } else { parse::search(list).unwrap().found.remove(0) };
        let me = (me != "-").then(|| {
            let (id, name) = me.split_once('|').unwrap();
            User { id: id.parse().unwrap(), name: name.into() }
        });
        let names: Vec<String> = closed.split('|').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
        let context = Context { me, closed: ClosedNames::new(&names) };
        let url = format!("https://is.example/Task/View/{}", task.id);
        let got = build(&c, &row, &url, &context, &exported());
        checked += 1;
        if got.as_ref() != Ok(expected) {
            wrong
                .push(format!("строка {line}: {}", got.map_or_else(|e| format!("ошибка: {e}"), |t| format!("текст не совпал с C#:\n{t}"))));
        }
    }
    assert!(checked > 100, "строк в таблице меньше, чем ждали: {checked}");
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn the_text_carries_the_moment_it_was_given() {
    let (url, _) = route(|path| if path.starts_with("/api/tasklifetime") { page(0, 1, Some((1, 1))) } else { Reply::json(CARD) });
    let moment = DateTime::parse_from_rfc3339("2030-01-02T03:04:05+05:00").unwrap();
    let text = build(&client(&url), &card_row(), URL, &context(), &moment).unwrap();
    assert!(text.contains("\nexported: 2030-01-02T03:04:05+05:00\n"), "{text}");
    assert!(!text.contains("2026-11-17"), "{text}");
    assert!(text.contains(&format!("\nurl: \"{URL}\"\n")) && text.contains("# 900 · Тема из карточки"), "{text}");
    assert!(text.contains("запись 0"), "{text}");
}

#[test]
fn a_card_error_is_the_answer_and_the_history_is_not_asked() {
    let (url, seen) = route(|_| Reply::new(404, "text/plain", ""));
    assert_eq!(build(&client(&url), &card_row(), URL, &context(), &exported()), Err("заявка не найдена (HTTP 404)".to_string()));
    assert_eq!(paths(&seen), ["/api/task/900?include=status"]);
}

#[test]
fn a_card_server_failure_is_not_retried() {
    let (url, seen) = route(|_| Reply::new(500, "text/plain", "сбой"));
    let result = build(&client(&url), &card_row(), URL, &context(), &exported());
    assert!(result.as_ref().is_err_and(|e| e.contains("500")), "{result:?}");
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn a_history_error_is_the_answer_not_a_text_without_history() {
    let (url, seen) =
        route(|path| if path.starts_with("/api/tasklifetime") { Reply::new(404, "text/plain", "") } else { Reply::json(CARD) });
    assert_eq!(build(&client(&url), &card_row(), URL, &context(), &exported()), Err("заявка не найдена (HTTP 404)".to_string()));
    assert_eq!(paths(&seen), ["/api/task/900?include=status", FIRST]);
    // ошибка на второй странице — тоже ошибка
    let (url, _) = route(|path| match path {
        p if p.starts_with("/api/tasklifetime") && page_of(p) == 1 => page(0, 2, Some((1, 2))),
        p if p.starts_with("/api/tasklifetime") => Reply::new(404, "text/plain", ""),
        _ => Reply::json(CARD),
    });
    assert_eq!(build(&client(&url), &card_row(), URL, &context(), &exported()), Err("заявка не найдена (HTTP 404)".to_string()));
}

#[test]
fn a_history_of_exactly_the_limit_pages_is_whole_and_one_page_more_is_cut_with_a_note() {
    let serve = |pages: i32| {
        route(move |path| {
            if path.starts_with("/api/tasklifetime") {
                page(page_of(path), page_of(path) + 1, Some((page_of(path), pages)))
            } else {
                Reply::json(CARD)
            }
        })
    };
    // ровно 100 страниц — целая: участники и счёт на месте
    let (url, seen) = serve(MAX_LIFETIME_PAGES);
    let whole = build(&client(&url), &card_row(), URL, &context(), &exported()).unwrap();
    assert_eq!(lifetime_paths(&seen).len(), MAX_LIFETIME_PAGES as usize);
    assert!(whole.contains("\nparticipants: [") && whole.contains("\ncomments_public: 100\n"), "{whole}");
    assert!(!whole.contains("Переписка длиннее"), "{whole}");
    // 101 страница — читается 100, в тексте пометка вместо участников и счёта
    let (url, seen) = serve(MAX_LIFETIME_PAGES + 1);
    let cut = build(&client(&url), &card_row(), URL, &context(), &exported()).unwrap();
    assert_eq!(lifetime_paths(&seen).len(), MAX_LIFETIME_PAGES as usize);
    assert!(cut.contains("_Переписка длиннее: показаны последние 100 записей, более ранние не выгружены._"), "{cut}");
    assert!(!cut.contains("participants:") && !cut.contains("comments_public:"), "{cut}");
}
