//! Форма поиска без окна (`TicketSearch.cs`): даты «с» и «по» → период (карточка 3.2-1).

mod common;

use chrono::{NaiveDate, NaiveDateTime};
use ticketboard::intraservice::query::DateSpan;
use ticketboard::search::parse_span;

fn midnight(y: i32, m: u32, d: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(0, 0, 0).unwrap()
}

/// Ответ в виде строки таблицы: `OK <с> <по>` или `ERR <текст>`.
fn answer(inputs: &[String]) -> String {
    let show = |d: Option<NaiveDateTime>| d.map_or("-".to_string(), |d| d.format("%Y-%m-%dT%H:%M:%S").to_string());
    match parse_span(&inputs[0], &inputs[1], &inputs[2]) {
        Ok(s) => format!("OK {} {}", show(s.from), show(s.to)),
        Err(e) => format!("ERR {e}"),
    }
}

#[test]
fn dates_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("search-dates", answer);
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn a_valid_period_runs_from_the_first_day_to_the_day_after_the_last() {
    let span = parse_span("01.02.2026", "28.2.2026", "Создана").unwrap();
    assert_eq!(span, DateSpan { from: Some(midnight(2026, 2, 1)), to: Some(midnight(2026, 3, 1)) });
}

#[test]
fn empty_fields_leave_the_period_open() {
    assert_eq!(parse_span("", "  ", "Создана"), Ok(DateSpan { from: None, to: None }));
    let only_to = parse_span("", "31.12.2100", "Создана").unwrap();
    assert_eq!((only_to.from, only_to.to), (None, Some(midnight(2101, 1, 1))));
}

#[test]
fn a_single_day_is_a_period_of_one_day() {
    let span = parse_span("05.03.2026", "2026-03-05", "Изменена").unwrap();
    assert_eq!((span.from, span.to), (Some(midnight(2026, 3, 5)), Some(midnight(2026, 3, 6))));
}

#[test]
fn an_unreadable_date_is_an_error_with_the_exact_text() {
    assert_eq!(
        parse_span("\u{A0}1.2.26 ", "", "Закрыта"),
        Err("Закрыта: дата «1.2.26» не понятна — нужно дд.мм.гггг, год 2000–2100".to_string())
    );
}

#[test]
fn the_from_field_is_judged_before_the_to_field() {
    let e = parse_span("abc", "xyz", "Создана").unwrap_err();
    assert!(e.contains("«abc»") && !e.contains("xyz"), "{e}");
}

#[test]
fn a_start_after_the_end_is_an_error() {
    assert_eq!(parse_span("01.03.2026", "01.02.2026", "Изменена"), Err("Изменена: начало периода позже его конца".to_string()));
}

#[test]
fn a_bad_to_date_wins_over_the_order_of_the_period() {
    let e = parse_span("05.03.2026", "abc", "Создана").unwrap_err();
    assert!(e.contains("«abc»"), "{e}");
}
