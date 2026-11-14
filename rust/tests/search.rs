//! Форма поиска без окна (`TicketSearch.cs`): даты «с» и «по» → период (карточка 3.2-1).

mod common;

use chrono::{NaiveDate, NaiveDateTime, TimeZone};
use common::{Reply, route};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ticketboard::intraservice::query::{DateSpan, TaskQuery, escape_data_string};
use ticketboard::intraservice::{Client, Found, Ref, SearchPage, Status, parse};
use ticketboard::rules::ClosedNames;
use ticketboard::search::*;

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

/// Поле «по» тоже судят: начало хорошее, конец не читается — ошибка о конце. (Раньше ли это проверки порядка, по
/// ответу не увидеть: у нечитаемого конца нет даты, сравнивать нечего.)
#[test]
fn a_bad_to_date_is_an_error_after_a_good_from() {
    let e = parse_span("05.03.2026", "abc", "Создана").unwrap_err();
    assert!(e.contains("«abc»"), "{e}");
}

// --- карточка 3.2-2: условия → запрос (`TicketSearch.ResolveAsync`, блок «живое» его SelfCheck) ---

const NBSP: &str = "\u{A0}";

fn users_page(users: &[(i32, &str)], count: usize) -> String {
    let list: Vec<String> = users.iter().map(|(id, name)| format!(r#"{{"Id":{id},"Name":"{name}"}}"#)).collect();
    format!(r#"{{"Users":[{}],"Paginator":{{"Count":{count},"Page":1,"PageCount":1}}}}"#, list.join(","))
}

/// Сотрудники поддельного сервера (`FakeUsers`): по «максимов» один, по «иванов» два, по «много» одиннадцать, по «сам» — я,
/// по «оба» — два из двадцати пяти (Count больше предела), по другому — никого.
fn fake_users(path: &str) -> String {
    let search = path.split("search=").nth(1).and_then(|r| r.split('&').next()).unwrap_or("");
    let is = |word: &str| search == escape_data_string(word);
    if is("максимов") {
        users_page(&[(38472, "Максимов М. С.")], 1)
    } else if is("иванов") {
        users_page(&[(5, "Иванов А."), (6, "Иванов Б.")], 2)
    } else if is("много") {
        let many: Vec<(i32, String)> = (100..111).map(|i| (i, format!("Сотрудник {i}"))).collect();
        let refs: Vec<(i32, &str)> = many.iter().map(|(i, n)| (*i, n.as_str())).collect();
        users_page(&refs, 11)
    } else if is("сам") {
        users_page(&[(7, "Я Сам")], 1)
    } else if is("оба") {
        users_page(&[(8, "Оба А."), (9, "Оба Б.")], 25)
    } else if is("десять") {
        let ten: Vec<(i32, String)> = (200..210).map(|i| (i, format!("Сотрудник {i}"))).collect();
        let refs: Vec<(i32, &str)> = ten.iter().map(|(i, n)| (*i, n.as_str())).collect();
        users_page(&refs, 10)
    } else if is("двое") {
        users_page(&[(11, "Двое А."), (12, "Двое Б.")], 10)
    } else {
        users_page(&[], 0)
    }
}

const EMPTY_TASKS: &str = r#"{"Tasks":[],"Paginator":{"Count":0,"Page":1,"PageCount":1}}"#;

fn world() -> (Client, Arc<Mutex<Vec<String>>>) {
    let (url, seen) = route(|path| {
        if path.starts_with("/api/user?getcurrentuserinfo=true") {
            Reply::json(r#"{"Id":7,"Name":"Я Сам"}"#)
        } else if path.starts_with("/api/user?") {
            Reply::json(&fake_users(path))
        } else {
            Reply::json(EMPTY_TASKS)
        }
    });
    (Client::new(&url, "u", "p", Duration::from_secs(10)), seen)
}

fn paths(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock().unwrap().iter().map(|h| h.split(' ').nth(1).unwrap_or("").to_string()).collect()
}

fn status(id: i32, name: &str, is_fixed: bool) -> Status {
    Status { id, name: name.into(), is_fixed, ..Default::default() }
}

/// Открыта 31; Выполнена 29 (признак сервера); Закрыта 30 (только по названию из настроек).
fn statuses() -> Vec<Status> {
    vec![status(31, "Открыта", false), status(29, "Выполнена", true), status(30, "Закрыта", false)]
}

fn closed_names() -> ClosedNames {
    ClosedNames::new(&["Закрыта".to_string()])
}

fn run(f: &SearchFilter) -> (Result<Resolved, String>, Vec<String>) {
    let (client, seen) = world();
    let r = resolve(&client, f, &statuses(), &closed_names());
    (r, paths(&seen))
}

fn ok(f: &SearchFilter) -> Resolved {
    run(f).0.unwrap_or_else(|e| panic!("ждали запрос, пришла ошибка: {e}"))
}

fn err(f: &SearchFilter) -> String {
    run(f).0.expect_err("ждали ошибку")
}

fn filter(edit: impl FnOnce(&mut SearchFilter)) -> SearchFilter {
    let mut f = SearchFilter::default();
    edit(&mut f);
    f
}

#[test]
fn the_default_filter_is_empty_but_for_archived_services() {
    let f = SearchFilter::default();
    assert_eq!((f.words.as_str(), f.mine, f.executor.as_str(), f.creator.as_str()), ("", false, "", ""));
    assert_eq!((f.observer_mine, f.observer.as_str(), f.status), (false, "", SearchStatus::Any));
    assert!([&f.created_from, &f.created_to, &f.changed_from, &f.changed_to, &f.closed_from, &f.closed_to].iter().all(|s| s.is_empty()));
    assert!(f.include_archived, "и архивные сервисы — по умолчанию, как у C#");
}

#[test]
fn the_usual_filter_is_my_closed_tickets() {
    let f = SearchFilter::usual();
    assert_eq!(f, SearchFilter { mine: true, status: SearchStatus::Closed, ..SearchFilter::default() });
    assert!(f.include_archived);
}

#[test]
fn a_quick_filter_is_just_trimmed_words() {
    assert_eq!(SearchFilter::quick("  принтер vpn \t"), SearchFilter { words: "принтер vpn".into(), ..SearchFilter::default() });
    assert!(!SearchFilter::quick("принтер").mine, "прочие условия — по умолчанию, без «Я» и статуса");
    assert_eq!(SearchFilter::quick("   "), SearchFilter::default());
}

#[test]
fn all_conditions_at_once_become_one_query() {
    let f = filter(|f| {
        f.words = " принтер ".into();
        f.mine = true;
        f.executor = "максимов".into();
        f.creator = "иванов".into();
        f.status = SearchStatus::Closed;
        f.created_from = "01.01.2026".into();
        f.changed_to = "31.12.2026".into();
    });
    let q = ok(&f).query;
    assert_eq!(q.executor_ids, vec![7, 38472], "я и Максимов; я не задваиваюсь");
    assert_eq!(q.creator_ids, vec![5, 6]);
    assert_eq!(q.status_ids, vec![29, 30], "закрытые — по признаку сервера и по названию из настроек");
    assert_eq!(q.search, "принтер");
    assert_eq!(q.created, DateSpan { from: Some(midnight(2026, 1, 1)), to: None });
    assert_eq!(q.changed, DateSpan { from: None, to: Some(midnight(2027, 1, 1)) });
    assert_eq!(q.closed, DateSpan::default());
    assert!(q.observer_ids.is_empty() && q.service_ids.is_empty() && q.type_ids.is_empty() && q.filter_id.is_none());
    assert!(q.include_archived);
}

#[test]
fn the_notes_say_who_was_found_by_name() {
    let f = filter(|f| {
        f.mine = true;
        f.executor = "максимов".into();
        f.creator = "иванов".into();
    });
    assert_eq!(ok(&f).notes, vec!["Исполнитель: Максимов М. С.", "Заявитель «иванов»: подошли 2 — Иванов А., Иванов Б."]);
}

#[test]
fn the_query_goes_to_the_server_as_the_window_sends_it() {
    let f = filter(|f| {
        f.words = " принтер ".into();
        f.mine = true;
        f.executor = "максимов".into();
        f.creator = "иванов".into();
        f.status = SearchStatus::Closed;
        f.created_from = "01.01.2026".into();
        f.changed_to = "31.12.2026".into();
    });
    let (client, seen) = world();
    let q = resolve(&client, &f, &statuses(), &closed_names()).unwrap().query;
    client.find_tasks(&q, 1).unwrap();
    let url = paths(&seen).pop().unwrap();
    assert!(url.contains("ExecutorIds=7,38472&StatusIds=29,30&CreatorIds=5,6&search="), "{url}");
    assert!(url.contains("&CreatedMoreThan=2026-01-01%2000%3A00&ChangedLessThan=2027-01-01%2000%3A00&"), "{url}");
    assert!(url.contains("&archive=true&inactive=true&include=status,service&sort="), "{url}");
    assert!(!url.contains("count="), "count=all живой сервер не принимает: {url}");
    assert!(url.ends_with("&pagesize=50&page=1"), "{url}");
}

#[test]
fn the_archived_switch_goes_into_the_query() {
    let q = ok(&filter(|f| {
        f.words = "принтер".into();
        f.include_archived = false;
    }))
    .query;
    assert!(!q.include_archived);
}

#[test]
fn an_observer_is_me_and_or_a_part_of_a_name() {
    let r = ok(&filter(|f| {
        f.observer_mine = true;
        f.observer = "максимов".into();
    }));
    assert!(r.query.executor_ids.is_empty(), "исполнителя не задавали");
    assert_eq!(r.query.observer_ids, vec![7, 38472]);
    assert_eq!(r.notes, vec!["Наблюдатель: Максимов М. С."]);
    assert_eq!(ok(&filter(|f| f.observer_mine = true)).query.observer_ids, vec![7]);
}

#[test]
fn me_is_in_both_lists_when_both_i_are_set() {
    let q = ok(&filter(|f| {
        f.mine = true;
        f.observer_mine = true;
    }))
    .query;
    assert_eq!((q.executor_ids, q.observer_ids), (vec![7], vec![7]));
}

#[test]
fn a_person_found_by_name_who_is_me_is_not_counted_twice() {
    let q = ok(&filter(|f| {
        f.mine = true;
        f.executor = "сам".into();
        f.observer_mine = true;
        f.observer = "сам".into();
    }))
    .query;
    assert_eq!((q.executor_ids, q.observer_ids), (vec![7], vec![7]));
}

#[test]
fn names_are_trimmed_before_the_directory_and_in_the_texts() {
    let (r, asked) = run(&filter(|f| f.executor = "  максимов  ".into()));
    assert_eq!(r.unwrap().notes, vec!["Исполнитель: Максимов М. С."]);
    assert!(asked[0].contains(&format!("search={}&", escape_data_string("максимов"))), "{asked:?}");
    assert_eq!(
        err(&filter(|f| f.creator = "  никого ".into())),
        "Заявитель «никого»: такого сотрудника не нашёл — достаточно части фамилии, но верной"
    );
}

#[test]
fn a_blank_name_is_no_condition_and_no_request() {
    let (r, asked) = run(&filter(|f| {
        f.executor = "   ".into();
        f.creator = "\t".into();
        f.observer = " ".into();
        f.words = "принтер".into();
    }));
    let r = r.unwrap();
    assert!(r.query.executor_ids.is_empty() && r.query.creator_ids.is_empty() && r.query.observer_ids.is_empty());
    assert!(asked.is_empty(), "справочник не спрашивали: {asked:?}");
}

#[test]
fn nobody_by_that_name_is_an_error_with_a_hint() {
    assert_eq!(
        err(&filter(|f| f.creator = "никого".into())),
        "Заявитель «никого»: такого сотрудника не нашёл — достаточно части фамилии, но верной"
    );
    assert_eq!(
        err(&filter(|f| f.executor = "никого".into())),
        "Исполнитель «никого»: такого сотрудника не нашёл — достаточно части фамилии, но верной"
    );
    assert_eq!(
        err(&filter(|f| f.observer = "никого".into())),
        "Наблюдатель «никого»: такого сотрудника не нашёл — достаточно части фамилии, но верной"
    );
}

#[test]
fn more_than_ten_by_that_name_is_an_error() {
    assert_eq!(err(&filter(|f| f.executor = "много".into())), "Исполнитель «много»: подходит больше 10 сотрудников — уточните");
}

#[test]
fn exactly_ten_by_that_name_is_still_a_search() {
    // предел — «больше 10»: десять подошедших (и строк, и по счёту сервера) — ещё поиск по всем десяти (мутанты 3.2-2)
    let ten = ok(&filter(|f| f.executor = "десять".into()));
    assert_eq!(ten.query.executor_ids, (200..210).collect::<Vec<_>>());
    let counted = ok(&filter(|f| f.creator = "двое".into()));
    assert_eq!(counted.query.creator_ids, vec![11, 12], "сервер насчитал ровно 10 — ещё не «слишком много»");
}

#[test]
fn a_total_above_ten_is_too_many_even_if_few_rows_came() {
    // сервер отдал две строки, а подходит двадцать пять
    assert_eq!(err(&filter(|f| f.creator = "оба".into())), "Заявитель «оба»: подходит больше 10 сотрудников — уточните");
}

#[test]
fn a_server_failure_while_looking_for_a_person_is_an_error_with_its_text() {
    let (url, seen) =
        route(|path| if path.starts_with("/api/user?") { Reply::new(500, "text/plain", "") } else { Reply::json(EMPTY_TASKS) });
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    let e = resolve(&client, &filter(|f| f.executor = "максимов".into()), &statuses(), &closed_names()).unwrap_err();
    assert_eq!(e, "Исполнитель «максимов»: не удалось найти сотрудника\nошибка сервера (HTTP 500)");
    assert_eq!(paths(&seen).len(), 1);
}

#[test]
fn me_is_asked_from_the_server_and_a_failure_stops_before_any_name() {
    let (url, seen) = route(|path| {
        if path.starts_with("/api/user?getcurrentuserinfo") { Reply::new(500, "text/plain", "") } else { Reply::json(&fake_users(path)) }
    });
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    let f = filter(|f| {
        f.mine = true;
        f.executor = "максимов".into();
    });
    let e = resolve(&client, &f, &statuses(), &closed_names()).unwrap_err();
    assert_eq!(e, "ошибка сервера (HTTP 500)");
    assert_eq!(paths(&seen).len(), 1, "справочник сотрудников не спрашивали: {:?}", paths(&seen));
}

#[test]
fn the_people_are_looked_up_executor_then_creator_then_observer() {
    let e = err(&filter(|f| {
        f.executor = "никого".into();
        f.creator = "никого".into();
        f.observer = "никого".into();
    }));
    assert!(e.starts_with("Исполнитель «никого»"), "{e}");
    let e = err(&filter(|f| {
        f.creator = "никого".into();
        f.observer = "никого".into();
    }));
    assert!(e.starts_with("Заявитель «никого»"), "{e}");
    // исполнитель найден, заявитель — нет: ошибка о заявителе, не о наблюдателе
    let e = err(&filter(|f| {
        f.executor = "максимов".into();
        f.creator = "никого".into();
        f.observer = "никого".into();
    }));
    assert!(e.starts_with("Заявитель «никого»"), "{e}");
}

#[test]
fn me_and_an_observer_note_both_i_are_set() {
    let both = ok(&filter(|f| {
        f.mine = true;
        f.observer_mine = true;
    }))
    .notes;
    assert_eq!(
        both,
        vec!["«Я» и у исполнителя, и у наблюдателя — это заявки, где вы и то и другое; все, где вы наблюдатель, — без «Я» у исполнителя"]
    );
}

#[test]
fn me_as_executor_with_a_named_observer_is_noted() {
    let notes = ok(&filter(|f| {
        f.mine = true;
        f.observer = " максимов ".into();
    }))
    .notes;
    assert_eq!(
        notes,
        vec![
            "Наблюдатель: Максимов М. С.",
            "«Я» у исполнителя и наблюдатель «максимов» — это только ваши заявки с этим наблюдателем; все заявки этого наблюдателя — без «Я» у исполнителя"
        ]
    );
}

#[test]
fn with_all_three_set_only_the_first_warning_is_given() {
    let notes = ok(&filter(|f| {
        f.mine = true;
        f.observer_mine = true;
        f.observer = "максимов".into();
    }))
    .notes;
    assert_eq!(notes.iter().filter(|n| n.starts_with("«Я»")).count(), 1, "{notes:?}");
    assert!(notes[1].starts_with("«Я» и у исполнителя, и у наблюдателя"), "{notes:?}");
}

#[test]
fn single_i_and_a_named_observer_without_i_give_no_warning() {
    assert!(ok(&filter(|f| f.observer_mine = true)).notes.is_empty());
    assert!(ok(&filter(|f| f.mine = true)).notes.is_empty());
    assert_eq!(ok(&filter(|f| f.observer = "максимов".into())).notes, vec!["Наблюдатель: Максимов М. С."]);
}

#[test]
fn no_conditions_is_all_tickets_and_says_so_without_a_request() {
    let (r, asked) = run(&SearchFilter::default());
    let r = r.unwrap();
    assert!(r.query.is_unrestricted());
    assert_eq!(r.notes, vec!["Условий нет: ищу среди всех заявок, которые видит ваша учётная запись"]);
    assert!(asked.is_empty(), "«я» и сотрудников не искали: {asked:?}");
}

#[test]
fn any_condition_removes_the_no_conditions_note() {
    assert!(ok(&filter(|f| f.words = "принтер".into())).notes.is_empty());
    assert!(ok(&filter(|f| f.status = SearchStatus::Closed)).notes.is_empty());
    assert!(ok(&filter(|f| f.closed_to = "01.02.2026".into())).notes.is_empty());
}

#[test]
fn the_archived_switch_alone_is_not_a_condition() {
    let r = ok(&filter(|f| f.include_archived = false));
    assert_eq!(r.notes.len(), 1);
}

#[test]
fn words_shorter_than_three_are_an_error() {
    assert_eq!(err(&filter(|f| f.words = "аб".into())), "Слова — минимум 3 символа");
    assert_eq!(err(&filter(|f| f.words = " аб ".into())), "Слова — минимум 3 символа");
    assert_eq!(err(&filter(|f| f.words = "а".into())), "Слова — минимум 3 символа");
}

#[test]
fn three_characters_are_enough_and_blank_words_are_no_words() {
    assert_eq!(ok(&filter(|f| f.words = "абв".into())).query.search, "абв");
    assert_eq!(ok(&filter(|f| f.words = "  абв  ".into())).query.search, "абв");
    assert_eq!(ok(&filter(|f| f.words = "   ".into())).query.search, "");
}

#[test]
fn the_length_of_words_is_counted_as_dotnet_counts_it() {
    // `string.Length` — единицы UTF-16: смайлик — две, с буквой — три, этого хватает (ревью «проверки» 3.2)
    assert_eq!(ok(&filter(|f| f.words = "\u{1F600}a".into())).query.search, "\u{1F600}a");
    assert!(err(&filter(|f| f.words = "\u{1F600}".into())).contains("минимум 3"));
}

#[test]
fn each_period_is_judged_with_its_own_label() {
    assert_eq!(err(&filter(|f| f.created_from = "вчера".into())), "Создана: дата «вчера» не понятна — нужно дд.мм.гггг, год 2000–2100");
    assert_eq!(err(&filter(|f| f.changed_from = "вчера".into())), "Изменена: дата «вчера» не понятна — нужно дд.мм.гггг, год 2000–2100");
    assert_eq!(err(&filter(|f| f.closed_to = "завтра".into())), "Закрыта: дата «завтра» не понятна — нужно дд.мм.гггг, год 2000–2100");
    assert_eq!(
        err(&filter(|f| {
            f.changed_from = "01.03.2026".into();
            f.changed_to = "01.02.2026".into();
        })),
        "Изменена: начало периода позже его конца"
    );
}

#[test]
fn the_periods_come_in_the_order_created_changed_closed() {
    let bad = |f: &mut SearchFilter| {
        f.created_from = "а".into();
        f.changed_from = "б".into();
        f.closed_from = "в".into();
    };
    assert!(err(&filter(bad)).starts_with("Создана:"));
    assert!(
        err(&filter(|f| {
            f.changed_from = "б".into();
            f.closed_from = "в".into();
        }))
        .starts_with("Изменена:")
    );
    assert!(err(&filter(|f| f.closed_from = "в".into())).starts_with("Закрыта:"));
}

#[test]
fn the_closed_period_goes_into_the_query() {
    let q = ok(&filter(|f| {
        f.closed_from = "01.02.2026".into();
        f.closed_to = "28.02.2026".into();
    }))
    .query;
    assert_eq!(q.closed, DateSpan { from: Some(midnight(2026, 2, 1)), to: Some(midnight(2026, 3, 1)) });
}

#[test]
fn words_are_judged_before_dates() {
    let e = err(&filter(|f| {
        f.words = "аб".into();
        f.created_from = "вчера".into();
    }));
    assert_eq!(e, "Слова — минимум 3 символа");
}

#[test]
fn dates_are_judged_before_the_status_list() {
    let (client, seen) = world();
    let f = filter(|f| {
        f.created_from = "вчера".into();
        f.status = SearchStatus::Open;
    });
    let e = resolve(&client, &f, &[], &closed_names()).unwrap_err();
    assert!(e.starts_with("Создана:"), "{e}");
    assert!(paths(&seen).is_empty());
}

#[test]
fn the_status_list_is_judged_before_me_and_the_names() {
    let (client, seen) = world();
    let f = filter(|f| {
        f.status = SearchStatus::Open;
        f.mine = true;
        f.executor = "никого".into();
    });
    let e = resolve(&client, &f, &[], &closed_names()).unwrap_err();
    assert_eq!(e, "Список статусов не загрузился — «закрытые» и «открытые» определить нечем; выберите статус «Любой»");
    assert!(paths(&seen).is_empty(), "до ошибки сервер не спрашивали: {:?}", paths(&seen));
}

#[test]
fn early_errors_make_no_request_at_all() {
    let short_words = filter(|f| {
        f.words = "аб".into();
        f.mine = true;
    });
    let bad_date = filter(|f| {
        f.closed_to = "вчера".into();
        f.mine = true;
        f.executor = "максимов".into();
    });
    for (bad, expected) in
        [(short_words, "Слова — минимум 3 символа"), (bad_date, "Закрыта: дата «вчера» не понятна — нужно дд.мм.гггг, год 2000–2100")]
    {
        let (r, asked) = run(&bad);
        assert_eq!(r.unwrap_err(), expected);
        assert!(asked.is_empty(), "{asked:?}");
    }
}

#[test]
fn closed_and_open_statuses_are_told_by_the_server_flags_and_the_settings_names() {
    assert_eq!(ok(&filter(|f| f.status = SearchStatus::Closed)).query.status_ids, vec![29, 30]);
    assert_eq!(ok(&filter(|f| f.status = SearchStatus::Open)).query.status_ids, vec![31]);
    assert!(ok(&filter(|f| f.status = SearchStatus::Any)).query.status_ids.is_empty());
}

#[test]
fn any_status_needs_no_status_list() {
    let (client, _) = world();
    let f = filter(|f| f.words = "принтер".into());
    assert!(resolve(&client, &f, &[], &closed_names()).is_ok());
}

#[test]
fn an_empty_status_list_is_an_error_for_open_and_closed() {
    let (client, _) = world();
    for s in [SearchStatus::Open, SearchStatus::Closed] {
        let e = resolve(&client, &filter(|f| f.status = s), &[], &closed_names()).unwrap_err();
        assert_eq!(e, "Список статусов не загрузился — «закрытые» и «открытые» определить нечем; выберите статус «Любой»");
    }
}

#[test]
fn no_closed_status_at_all_is_an_error_for_closed() {
    let (client, _) = world();
    let only_open = [status(31, "Открыта", false), status(32, "В работе", false)];
    let e = resolve(&client, &filter(|f| f.status = SearchStatus::Closed), &only_open, &ClosedNames::default()).unwrap_err();
    assert_eq!(e, "Сервер не назвал ни одного закрытого статуса");
}

#[test]
fn all_statuses_closed_is_an_error_for_open() {
    let (client, _) = world();
    let all_closed = [status(29, "Выполнена", true), status(30, "Закрыта", false)];
    let e = resolve(&client, &filter(|f| f.status = SearchStatus::Open), &all_closed, &closed_names()).unwrap_err();
    assert_eq!(e, "Все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json");
}

// --- карточка 3.2-2: сотрудники и страницы списка по адресу ---

fn user_names(users: &[Ref]) -> Vec<(i32, &str)> {
    users.iter().map(|u| (u.id, u.name.as_str())).collect()
}

#[test]
fn find_users_asks_one_page_of_max_plus_one() {
    let (client, seen) = world();
    let (users, total) = client.find_users("  ив а ", 10).unwrap();
    assert_eq!(paths(&seen), vec!["/api/user?fields=Id,Name&search=%D0%B8%D0%B2%20%D0%B0&pagesize=11&page=1"]);
    assert_eq!((users.len(), total), (0, 0));
    client.find_users("ив", 3).unwrap();
    assert_eq!(paths(&seen)[1], "/api/user?fields=Id,Name&search=%D0%B8%D0%B2&pagesize=4&page=1");
}

#[test]
fn find_users_returns_the_rows_and_the_servers_total_and_reads_no_second_page() {
    // сервер обещает ещё страницы (PageCount 3), а всего подошло 25
    let (url, seen) = route(|_| {
        Reply::json(r#"{"Users":[{"Id":8,"Name":"Оба А."},{"Id":9,"Name":"Оба Б."}],"Paginator":{"Count":25,"Page":1,"PageCount":3}}"#)
    });
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    let (users, total) = client.find_users("оба", 10).unwrap();
    assert_eq!(user_names(&users), vec![(8, "Оба А."), (9, "Оба Б.")]);
    assert_eq!(total, 25);
    assert_eq!(paths(&seen).len(), 1, "вторую страницу не читаем");
}

#[test]
fn find_users_without_a_paginator_counts_what_came() {
    let (url, _) = route(|_| Reply::json(r#"{"Users":[{"Id":8,"Name":"Оба А."}]}"#));
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    let (users, total) = client.find_users("оба", 10).unwrap();
    assert_eq!((users.len(), total), (1, 1));
}

#[test]
fn find_users_turns_a_server_failure_into_an_error() {
    let (url, _) = route(|_| Reply::new(404, "text/plain", ""));
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    assert_eq!(client.find_users("оба", 10).unwrap_err(), "по этому адресу нет API (HTTP 404)");
    let (url, _) = route(|_| Reply::json("это не json"));
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    assert!(client.find_users("оба", 10).unwrap_err().starts_with("непонятный ответ сервера:"));
}

#[test]
fn find_tasks_asks_a_detailed_counted_page_of_fifty() {
    let (client, seen) = world();
    client.find_tasks(&TaskQuery::default(), 2).unwrap();
    assert_eq!(paths(&seen), vec!["/api/task?include=status,service&sort=Changed%20desc&pagesize=50&page=2"]);
}

#[test]
fn find_tasks_reads_the_page_and_its_total() {
    let (url, _) = route(|_| Reply::json(r#"{"Tasks":[{"Id":5,"Name":"Принтер"}],"Paginator":{"Count":1234,"Page":1,"PageCount":25}}"#));
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    let page = client.find_tasks(&TaskQuery::default(), 1).unwrap();
    assert_eq!((page.found.len(), page.found[0].id, page.total), (1, 5, 1234));
}

#[test]
fn find_tasks_has_its_own_not_found_text() {
    let (url, _) = route(|_| Reply::new(404, "text/plain", ""));
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    assert_eq!(client.find_tasks(&TaskQuery::default(), 1).unwrap_err(), "ничего не найдено (HTTP 404)");
}

#[test]
fn find_tasks_reports_an_unreadable_answer() {
    let (url, _) = route(|_| Reply::json("{\"что-то\":1}"));
    let client = Client::new(&url, "u", "p", Duration::from_secs(10));
    assert!(client.find_tasks(&TaskQuery::default(), 1).unwrap_err().starts_with("непонятный ответ сервера:"));
}

// --- карточка 3.2-2: подписи строки (таблица сверки) ---

#[test]
fn row_labels_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("search-rows", |inputs| match parse::search(&inputs[0]) {
        Some(page) => page.found.iter().map(|f| format!("{} | {}", service_of(f), details_of(f))).collect::<Vec<_>>().join(" ‖ "),
        None => "НЕ РАЗОБРАН".to_string(),
    });
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

// --- карточка 3.2-2: число с тысячами ---

#[test]
fn thousands_are_grouped_with_a_no_break_space() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(999), "999");
    assert_eq!(thousands(1000), format!("1{NBSP}000"));
    assert_eq!(thousands(12345), format!("12{NBSP}345"));
    assert_eq!(thousands(1234567), format!("1{NBSP}234{NBSP}567"));
    assert_eq!(thousands(-1500), format!("-1{NBSP}500"));
}

// --- карточка 3.2-2: список найденного (`SearchViewModel.LoadPageAsync`) ---

fn row(id: i32) -> Found {
    Found { id, name: format!("Заявка {id}"), status: "Закрыта".into(), ..Default::default() }
}

fn page(ids: std::ops::Range<i32>, total: i32) -> SearchPage {
    SearchPage { found: ids.map(row).collect(), total, has_next: None }
}

fn ids(r: &Results) -> Vec<i32> {
    r.rows.iter().map(|f| f.id).collect()
}

fn plain() -> TaskQuery {
    TaskQuery::default()
}

#[test]
fn a_page_seam_repeat_is_shown_once() {
    let mut r = Results::default();
    r.add_page(page(1..4, 10), &plain());
    // список сдвинулся: третья пришла и второй страницей, под другим названием
    r.add_page(SearchPage { found: vec![Found { name: "другое".into(), ..row(3) }, row(4)], total: 10, has_next: None }, &plain());
    assert_eq!(ids(&r), vec![1, 2, 3, 4]);
    assert_eq!(r.rows[2].name, "Заявка 3", "остаётся первая");
    assert_eq!(r.pages, 2);
}

#[test]
fn the_total_is_never_below_what_is_shown() {
    let mut r = Results::default();
    r.add_page(page(1..4, 2), &plain());
    assert_eq!((r.total, r.rows.len()), (3, 3));
    let mut r = Results::default();
    r.add_page(page(1..4, 3), &plain());
    assert_eq!(r.total, 3);
    r.add_page(page(4..6, 5), &plain());
    assert_eq!(r.total, 5);
}

#[test]
fn the_total_follows_the_latest_page() {
    // как у C#: Total = max(ответ страницы, показано) — предыдущий итог не помнится
    let mut r = Results::default();
    r.add_page(page(1..51, 200), &plain());
    r.add_page(page(51..101, 120), &plain());
    assert_eq!(r.total, 120);
}

#[test]
fn more_is_offered_while_a_non_empty_page_leaves_some_unshown() {
    let mut r = Results::default();
    r.add_page(page(1..51, 120), &plain());
    assert!(r.has_more);
    r.add_page(page(51..101, 120), &plain());
    assert!(r.has_more);
    r.add_page(page(101..121, 120), &plain());
    assert!(!r.has_more, "показаны все");
}

#[test]
fn an_empty_page_ends_the_list() {
    let mut r = Results::default();
    r.add_page(page(1..51, 500), &plain());
    r.add_page(page(51..51, 500), &plain());
    assert!(!r.has_more);
    assert_eq!(r.rows.len(), 50);
}

#[test]
fn more_stops_at_a_thousand_shown() {
    let mut r = Results::default();
    r.add_page(page(1..1000, 5000), &plain());
    assert!(r.has_more, "999 показано — ещё можно");
    let mut r = Results::default();
    r.add_page(page(1..1001, 5000), &plain());
    assert!(!r.has_more, "1000 показано — хватит");
    assert_eq!(r.rows.len(), 1000);
}

#[test]
fn a_page_of_only_repeats_adds_nothing_but_still_counts_as_loaded() {
    let mut r = Results::default();
    r.add_page(page(1..4, 10), &plain());
    r.add_page(page(1..4, 10), &plain());
    assert_eq!((r.rows.len(), r.pages), (3, 2));
    assert!(r.has_more, "страница не пуста, показано меньше итога");
}

#[test]
fn nothing_found_says_so() {
    let mut r = Results::default();
    assert_eq!(r.message(), "Ничего не найдено");
    r.add_page(page(0..0, 0), &plain());
    assert_eq!((r.message().as_str(), r.has_more, r.pages), ("Ничего не найдено", false, 1));
}

#[test]
fn everything_found_is_just_the_count() {
    let mut r = Results::default();
    r.add_page(page(1..4, 3), &plain());
    assert_eq!(r.message(), "Найдено: 3");
    let mut r = Results::default();
    r.add_page(page(1..121, 120), &plain());
    assert_eq!(r.message(), "Найдено: 120");
}

#[test]
fn a_part_found_says_how_many_are_shown() {
    let mut r = Results::default();
    r.add_page(page(1..51, 120), &plain());
    assert_eq!(r.message(), "Найдено: 120 · показано 50");
}

#[test]
fn the_server_ceiling_is_shown_with_a_plus() {
    let mut r = Results::default();
    r.add_page(page(1..51, 1000), &plain());
    assert_eq!(r.message(), format!("Найдено: 1{NBSP}000+ · показано 50"));
}

#[test]
fn at_a_thousand_shown_the_message_asks_to_narrow_the_conditions() {
    let mut r = Results::default();
    r.add_page(page(1..1001, 5000), &plain());
    assert_eq!(r.message(), format!("Найдено: 5{NBSP}000 · показано 1{NBSP}000 — уточните условия"));
    let mut r = Results::default();
    r.add_page(page(1..1001, 1000), &plain());
    assert_eq!(r.message(), format!("Найдено: 1{NBSP}000+ · показано 1{NBSP}000 — уточните условия"));
    let mut r = Results::default();
    r.add_page(page(1..1000, 5000), &plain());
    assert_eq!(r.message(), format!("Найдено: 5{NBSP}000 · показано 999"), "на 999 просьбы ещё нет");
}

// предупреждение «сервер условие не применил»

fn dated_row(id: i32, changed_day: u32) -> Found {
    let at = chrono::FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 6, changed_day, 12, 0, 0).unwrap();
    Found { changed: Some(at), ..row(id) }
}

/// Изменена с 10 по 20 июня 2026: 15-е — внутри, 25-е — вне (больше чем на сутки за краем).
fn june_query() -> TaskQuery {
    TaskQuery { changed: DateSpan { from: Some(midnight(2026, 6, 10)), to: Some(midnight(2026, 6, 21)) }, ..TaskQuery::default() }
}

fn watching_query() -> TaskQuery {
    TaskQuery { observer_ids: vec![7], ..TaskQuery::default() }
}

#[test]
fn no_warning_while_every_row_fits_the_conditions() {
    let mut r = Results::default();
    r.add_page(SearchPage { found: vec![dated_row(1, 15), dated_row(2, 16)], total: 2, has_next: None }, &june_query());
    assert_eq!(r.warning, "");
}

#[test]
fn rows_outside_the_period_raise_a_warning_with_the_counts() {
    let mut r = Results::default();
    r.add_page(SearchPage { found: vec![dated_row(1, 25), dated_row(2, 15)], total: 2, has_next: None }, &june_query());
    assert_eq!(
        r.warning,
        "Сервер вернул заявки вне выбранного периода (1 из 2 загруженных): условие по дате он, похоже, не применил — в списке лишнее"
    );
}

#[test]
fn the_warning_words_name_the_observer_condition() {
    let mut r = Results::default();
    let wrong = Found { observer_ids: Some(vec![3]), ..row(1) };
    r.add_page(SearchPage { found: vec![wrong], total: 1, has_next: None }, &watching_query());
    assert_eq!(
        r.warning,
        "Сервер вернул заявки без выбранного наблюдателя (1 из 1 загруженных): условие по наблюдателю он, похоже, не применил — в списке лишнее"
    );
}

#[test]
fn the_warning_counts_over_all_loaded_pages() {
    let mut r = Results::default();
    r.add_page(SearchPage { found: vec![dated_row(1, 25), dated_row(2, 15)], total: 6, has_next: None }, &june_query());
    r.add_page(SearchPage { found: vec![dated_row(2, 15), dated_row(3, 16), dated_row(4, 17)], total: 6, has_next: None }, &june_query());
    assert_eq!(
        r.warning,
        "Сервер вернул заявки вне выбранного периода (1 из 5 загруженных): условие по дате он, похоже, не применил — в списке лишнее"
    );
}

#[test]
fn an_outside_row_repeated_on_the_next_page_is_counted_again() {
    // как у C#: считаются все пришедшие строки, а не показанные
    let mut r = Results::default();
    r.add_page(SearchPage { found: vec![dated_row(1, 25)], total: 2, has_next: None }, &june_query());
    r.add_page(SearchPage { found: vec![dated_row(1, 25)], total: 2, has_next: None }, &june_query());
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        r.warning,
        "Сервер вернул заявки вне выбранного периода (2 из 2 загруженных): условие по дате он, похоже, не применил — в списке лишнее"
    );
}
