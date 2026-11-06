//! Адрес списка заявок по отбору — образцы `TaskQuery.SelfCheck` один в один.

use chrono::{NaiveDate, NaiveDateTime};
use ticketboard::intraservice::query::*;

fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
}

#[test]
fn addresses() {
    // без условий — тот же адрес, что был у списка до отбора: импорт и автообновление не меняются
    assert_eq!(TaskQuery::default().to_url(1, 200, false, true), "api/task?include=status&sort=Changed%20desc&pagesize=200&page=1");
    let mine = TaskQuery { executor_ids: vec![7], status_ids: vec![29, 30], ..TaskQuery::default() };
    assert_eq!(
        mine.to_url(2, 200, false, true),
        "api/task?ExecutorIds=7&StatusIds=29,30&include=status&sort=Changed%20desc&pagesize=200&page=2"
    );
    // пустые списки и пробельные слова — не условия; страница не меньше первой
    let empty = TaskQuery { search: "  ".into(), ..TaskQuery::default() };
    assert_eq!(empty.to_url(0, 50, false, true), "api/task?include=status&sort=Changed%20desc&pagesize=50&page=1");
    // порядок для долгого чтения — по созданию и номеру (запятая как есть, пробелы — %20); count=all живой сервер не
    // принимает — счёт либо по умолчанию, либо выключен
    let stable = TaskQuery { sort: STABLE_SORT.into(), ..TaskQuery::default() };
    assert_eq!(stable.to_url(1, 200, true, true), "api/task?include=status,service&sort=Created%20asc,%20Id%20asc&pagesize=200&page=1");
    assert_eq!(
        stable.to_url(2, 200, true, false),
        "api/task?include=status,service&count=false&sort=Created%20asc,%20Id%20asc&pagesize=200&page=2"
    );
    // всё сразу: имена параметров — по документации (стр. 18-20), даты со временем, слова — в процентах
    let full = TaskQuery {
        executor_ids: vec![7, 9],
        status_ids: vec![29],
        search: "принтер C# & .NET".into(),
        creator_ids: vec![5],
        service_ids: vec![844, 845],
        type_ids: vec![1009],
        created: DateSpan { from: Some(at(2026, 1, 1, 0, 0)), to: Some(at(2026, 1, 31, 23, 59)) },
        changed: DateSpan { from: Some(at(2026, 2, 3, 4, 5)), to: None },
        closed: DateSpan { from: None, to: Some(at(2026, 12, 31, 23, 59)) },
        filter_id: Some(45),
        include_archived: true,
        ..TaskQuery::default()
    };
    assert_eq!(
        full.to_url(3, 50, true, true),
        format!(
            "{}{}{}{}{}",
            "api/task?ExecutorIds=7,9&StatusIds=29&CreatorIds=5&ServiceIds=844,845&TypeIds=1009",
            "&search=%D0%BF%D1%80%D0%B8%D0%BD%D1%82%D0%B5%D1%80%20C%23%20%26%20.NET",
            "&CreatedMoreThan=2026-01-01%2000%3A00&CreatedLessThan=2026-01-31%2023%3A59&ChangedMoreThan=2026-02-03%2004%3A05",
            "&ClosedLessThan=2026-12-31%2023%3A59&filterid=45&fields=",
            format_args!("{ROW_FIELDS}&archive=true&inactive=true&include=status,service&sort=Changed%20desc&pagesize=50&page=3")
        )
    );
    // наблюдатели — после типов; слова обрезаются по краям
    let watched = TaskQuery { observer_ids: vec![32275, 40536], search: " vpn ".into(), ..TaskQuery::default() };
    assert_eq!(
        watched.to_url(1, 20, false, true),
        "api/task?ObserverIds=32275,40536&search=vpn&include=status&sort=Changed%20desc&pagesize=20&page=1"
    );
}

#[test]
fn unrestricted() {
    assert!(TaskQuery::default().is_unrestricted());
    assert!(TaskQuery { include_archived: true, sort: STABLE_SORT.into(), ..TaskQuery::default() }.is_unrestricted());
    assert!(TaskQuery { search: " ".into(), ..TaskQuery::default() }.is_unrestricted());
    for q in [
        TaskQuery { search: "vpn".into(), ..TaskQuery::default() },
        TaskQuery { status_ids: vec![1], ..TaskQuery::default() },
        TaskQuery { closed: DateSpan { from: None, to: Some(at(2026, 1, 1, 0, 0)) }, ..TaskQuery::default() },
        TaskQuery { filter_id: Some(3), ..TaskQuery::default() },
        TaskQuery { service_ids: vec![5], ..TaskQuery::default() },
        TaskQuery { creator_ids: vec![5], ..TaskQuery::default() },
        TaskQuery { observer_ids: vec![5], ..TaskQuery::default() },
        TaskQuery { type_ids: vec![5], ..TaskQuery::default() },
        TaskQuery { executor_ids: vec![5], ..TaskQuery::default() },
        TaskQuery { created: DateSpan { from: Some(at(2026, 1, 1, 0, 0)), to: None }, ..TaskQuery::default() },
        TaskQuery { changed: DateSpan { from: Some(at(2026, 1, 1, 0, 0)), to: None }, ..TaskQuery::default() },
    ] {
        assert!(!q.is_unrestricted(), "{q:?}");
    }
}

#[test]
fn escaping_as_uri_escape_data_string() {
    // RFC 3986: незарезервированные — как есть, остальное — %XX в UTF-8, заглавными
    assert_eq!(escape_data_string("AZaz09-_.~"), "AZaz09-_.~");
    assert_eq!(escape_data_string("a b+c/d?e=f&g#h%"), "a%20b%2Bc%2Fd%3Fe%3Df%26g%23h%25");
    assert_eq!(escape_data_string("Ё"), "%D0%81");
}
