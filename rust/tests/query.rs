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

// --- `Outside` / `OutsideWords` (образцы `TaskQuery.SelfCheck`; карточка 3.2-2) ---

use chrono::{DateTime, FixedOffset, TimeZone};
use ticketboard::intraservice::Found;

/// Момент в поясе +03:00 (по умолчанию) — как дата в строке списка.
fn stamp(dt: NaiveDateTime) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(3 * 3600).unwrap().from_local_datetime(&dt).unwrap()
}

fn dated(created: Option<NaiveDateTime>, changed: Option<NaiveDateTime>) -> Found {
    Found {
        id: 1,
        name: "N".into(),
        status: "Закрыта".into(),
        created: created.map(stamp),
        changed: changed.map(stamp),
        ..Default::default()
    }
}

fn watched(ids: Option<Vec<i32>>) -> Found {
    Found { id: 1, name: "N".into(), status: "Закрыта".into(), observer_ids: ids, ..Default::default() }
}

/// Условие C#: изменена 01.06–30.06 23:59, создана не раньше 01.01.2026.
fn june() -> TaskQuery {
    TaskQuery {
        changed: DateSpan { from: Some(at(2026, 6, 1, 0, 0)), to: Some(at(2026, 6, 30, 23, 59)) },
        created: DateSpan { from: Some(at(2026, 1, 1, 0, 0)), to: None },
        ..TaskQuery::default()
    }
}

#[test]
fn a_row_inside_the_period_is_not_outside() {
    assert!(!june().outside(&dated(Some(at(2026, 3, 1, 0, 0)), Some(at(2026, 6, 15, 0, 0)))));
}

#[test]
fn a_row_changed_after_the_period_is_outside() {
    assert!(june().outside(&dated(Some(at(2026, 3, 1, 0, 0)), Some(at(2026, 8, 1, 0, 0)))));
}

#[test]
fn a_row_created_before_the_period_is_outside() {
    assert!(june().outside(&dated(Some(at(2025, 5, 1, 0, 0)), Some(at(2026, 6, 15, 0, 0)))));
}

#[test]
fn a_day_of_tolerance_around_the_edges_is_allowed() {
    // C#: на границе допуск в сутки — на часовые пояса и округление
    assert!(!june().outside(&dated(Some(at(2025, 12, 31, 20, 0)), Some(at(2026, 7, 1, 3, 0)))));
}

#[test]
fn the_tolerance_ends_exactly_one_day_from_the_edge() {
    // «с»: раньше границы минус сутки — вне; ровно на ней — нет. «по»: позже границы плюс сутки — вне; ровно на ней — нет
    let from_edge = june(); // created.from = 01.01.2026 00:00 -> 31.12.2025 00:00
    assert!(!from_edge.outside(&dated(Some(at(2025, 12, 31, 0, 0)), None)));
    assert!(from_edge.outside(&dated(Some(at(2025, 12, 30, 23, 59)), None)));
    // changed.to = 30.06 23:59 -> 01.07 23:59
    assert!(!june().outside(&dated(None, Some(at(2026, 7, 1, 23, 59)))));
    assert!(june().outside(&dated(None, Some(at(2026, 7, 2, 0, 0)))));
}

#[test]
fn a_date_is_judged_by_the_clock_of_the_row_not_by_utc() {
    // C# берёт `.DateTime` — часы пояса строки: 02.07 00:30 при +14:00 — позже 01.07 23:59, хоть в UTC это ещё 01.07
    let row = Found {
        changed: Some(FixedOffset::east_opt(14 * 3600).unwrap().with_ymd_and_hms(2026, 7, 2, 0, 30, 0).unwrap()),
        ..dated(None, None)
    };
    assert!(june().outside(&row));
}

#[test]
fn a_row_without_dates_or_a_query_without_a_period_is_not_outside() {
    assert!(!june().outside(&dated(None, None)));
    assert!(!TaskQuery::default().outside(&dated(Some(at(1999, 1, 1, 0, 0)), None)));
}

#[test]
fn a_row_without_the_wanted_observer_is_outside() {
    let q = TaskQuery { observer_ids: vec![7, 9], ..TaskQuery::default() };
    assert!(q.outside(&watched(Some(vec![3]))));
    assert!(q.outside(&watched(Some(vec![])))); // наблюдателей нет вовсе — тоже без нужного
    assert!(!q.outside(&watched(Some(vec![3, 9]))));
}

#[test]
fn a_row_that_does_not_say_its_observers_is_not_outside() {
    let q = TaskQuery { observer_ids: vec![7, 9], ..TaskQuery::default() };
    assert!(!q.outside(&watched(None))); // поля нет — не знаем, не вне
    assert!(!TaskQuery::default().outside(&watched(Some(vec![3])))); // условия нет
    assert!(!TaskQuery { observer_ids: vec![], ..TaskQuery::default() }.outside(&watched(Some(vec![3]))));
}

#[test]
fn outside_words_name_the_condition_the_server_ignored() {
    let watching = TaskQuery { observer_ids: vec![7, 9], ..TaskQuery::default() };
    assert_eq!(june().outside_words(), ("вне выбранного периода", "условие по дате"));
    assert_eq!(watching.outside_words(), ("без выбранного наблюдателя", "условие по наблюдателю"));
    let both = TaskQuery { observer_ids: vec![7], ..june() };
    assert_eq!(both.outside_words(), ("вне выбранного периода или без выбранного наблюдателя", "эти условия"));
}

#[test]
fn outside_words_with_no_period_and_no_observer_fall_back_to_the_date() {
    assert_eq!(TaskQuery::default().outside_words(), ("вне выбранного периода", "условие по дате"));
}

#[test]
fn a_closed_period_alone_does_not_count_as_a_period_for_the_words() {
    // `Outside` смотрит только создана и изменена; «закрыта» слов не меняет
    let closed_and_watching =
        TaskQuery { closed: DateSpan { from: Some(at(2026, 1, 1, 0, 0)), to: None }, observer_ids: vec![7], ..TaskQuery::default() };
    assert_eq!(closed_and_watching.outside_words(), ("без выбранного наблюдателя", "условие по наблюдателю"));
}

#[test]
fn one_period_with_an_observer_names_both_conditions() {
    // C#: период — это «создана» или «изменена»; задан хотя бы один — и в словах оба условия (мутанты 3.2-2)
    let both = ("вне выбранного периода или без выбранного наблюдателя", "эти условия");
    let span = DateSpan { from: Some(at(2026, 1, 1, 0, 0)), to: None };
    assert_eq!(TaskQuery { created: span, observer_ids: vec![7], ..TaskQuery::default() }.outside_words(), both);
    assert_eq!(TaskQuery { changed: span, observer_ids: vec![7], ..TaskQuery::default() }.outside_words(), both);
}
