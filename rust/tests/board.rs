//! Доска: `tickets.json` в формате TicketBoard.exe и правила того, что видно на доске (как в C#: `Ticket`, `Matches`,
//! `ColumnViewModel`).

use chrono::{DateTime, FixedOffset, NaiveDate};
use std::fs;
use ticketboard::board::*;

/// Тот же образец, что у `StoreCheck` (TicketBoard.SelfCheck/StoreCheck.cs): файл в нынешних именах, все поля заполнены.
const SAMPLE: &str = r#"
[
  {
    "Id": "9b2f6c1e-0000-4000-8000-000000000001",
    "CreatedAt": "2026-09-01T10:00:00+03:00",
    "Title": "Принтер не печатает",
    "Url": "https://hd/Task/View/702180",
    "Description": "с утра",
    "Priority": "High",
    "Status": "Waiting",
    "StatusChangedAt": "2026-09-02T11:00:00+03:00",
    "CompletedAt": "2026-09-03T12:00:00+03:00",
    "IntraserviceId": 702180,
    "ExternalStatus": "Закрыта",
    "LastSyncAt": "2026-09-04T13:00:00+03:00",
    "KeptOpenStatus": "Закрыта",
    "Creator": "Петрова А.",
    "CreatorPhone": "+7 900 000-00-00",
    "CreatorEmail": "petrova@example.ru",
    "Executors": "Я Сам, Сидоров С.",
    "ExecutorGroup": "Вторая линия",
    "CreatorPosition": "Бухгалтер",
    "CreatorCompany": "ООО Ромашка",
    "Service": "Почта",
    "TaskType": "Инцидент",
    "Categories": "Печать, Картриджи",
    "Assets": "Принтер HP",
    "Files": [ { "Id": 900101, "Name": "экран.png" }, { "Id": 900102, "Name": null } ],
    "CommentsCheckedFor": "2026-09-05T14:00:00+03:00",
    "CommentsSeenAt": "2026-09-05T14:00:01+03:00",
    "UnreadComments": 3,
    "AssignedToMe": false,
    "Notes": [ { "Id": "9b2f6c1e-0000-4000-8000-000000000002", "CreatedAt": "2026-09-06T15:00:00+03:00", "Text": "позвонить" } ]
  }
]
"#;

fn at(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

fn rules() -> Rules {
    Rules { overdue_days: 3, hide_done_days: 7, wip_limit: 5 }
}

fn card(json: &str) -> Ticket {
    parse_tickets(&format!("[{json}]")).unwrap().remove(0)
}

#[test]
fn the_store_check_sample_loads_whole() {
    let tickets = parse_tickets(SAMPLE).expect("образец читается");
    assert_eq!(tickets.len(), 1);
    let t = &tickets[0];
    assert_eq!(t.id, "9b2f6c1e-0000-4000-8000-000000000001");
    assert_eq!(t.title, "Принтер не печатает");
    assert_eq!(t.url, "https://hd/Task/View/702180");
    assert_eq!(t.description, "с утра");
    assert_eq!(t.priority, Priority::High);
    assert_eq!(t.status, Status::Waiting);
    assert_eq!(t.status_changed_at, Some(at("2026-09-02T11:00:00+03:00")));
    assert_eq!(t.completed_at, Some(at("2026-09-03T12:00:00+03:00")));
    assert_eq!(t.intraservice_id, Some(702180));
    assert_eq!(t.external_status.as_deref(), Some("Закрыта"));
    assert_eq!(t.creator.as_deref(), Some("Петрова А."));
    assert_eq!(t.service.as_deref(), Some("Почта"));
    assert_eq!(t.creator_company.as_deref(), Some("ООО Ромашка"));
    assert_eq!(t.unread_comments, 3);
    assert_eq!(t.files.len(), 2);
    assert_eq!((t.files[0].id, t.files[0].name.as_deref()), (900101, Some("экран.png")));
    assert_eq!((t.files[1].id, t.files[1].name.as_deref()), (900102, None));
    assert_eq!(t.notes.len(), 1);
    assert_eq!(t.notes[0].text, "позвонить");
}

#[test]
fn unknown_fields_nulls_and_missing_fields() {
    // поле из будущей версии — мимо; null и отсутствие — значения по умолчанию, как у C# (Mid, Inbox)
    let t = card(r#"{"Id": "a", "Title": null, "FutureField": {"x": 1}, "Notes": null, "Files": null, "UnreadComments": null}"#);
    assert_eq!(t.title, "");
    assert_eq!(t.priority, Priority::Mid);
    assert_eq!(t.status, Status::Inbox);
    assert!(t.notes.is_empty() && t.files.is_empty());
    assert_eq!(t.unread_comments, 0);
    assert_eq!(t.intraservice_id, None);
    assert_eq!(t.number(), "без номера");
    // дата с долями секунды, как пишет .NET (7 знаков)
    let t = card(r#"{"Id": "b", "StatusChangedAt": "2026-10-05T14:03:12.1234567+03:00", "IntraserviceId": 9}"#);
    assert_eq!(t.status_changed_at.map(|d| d.date_naive()), NaiveDate::from_ymd_opt(2026, 10, 5));
    assert_eq!(t.number(), "#9");
}

#[test]
fn missing_and_corrupt_files() {
    let dir = std::env::temp_dir().join(format!("tb-rs-board-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tickets.json");
    let _ = fs::remove_file(&path);
    assert_eq!(load_tickets(&path).map(|t| t.len()), Ok(0), "файла нет — пустая доска");
    fs::write(&path, "[{\"Id\": \"a\", ").unwrap();
    let err = load_tickets(&path).expect_err("битый файл — ошибка, не паника");
    assert!(err.contains("tickets.json"), "{err}");
    fs::write(&path, "[{\"Id\": \"a\", \"Status\": \"Archived\"}]").unwrap();
    assert!(load_tickets(&path).is_err(), "колонка, которой нет, — ошибка, как у C#");
    fs::write(&path, SAMPLE).unwrap();
    assert_eq!(load_tickets(&path).map(|t| t.len()), Ok(1));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn column_titles_and_order() {
    let titles: Vec<_> = Status::ALL.iter().map(|s| s.title()).collect();
    assert_eq!(titles, ["Входящие", "В работе", "Ждёт ответа", "Готово"]);
    for (i, s) in Status::ALL.iter().enumerate() {
        assert_eq!(s.index(), i);
        assert_eq!(Status::from_index(i), Some(*s));
    }
    assert_eq!(Status::from_index(4), None);
    let p: Vec<_> = [Priority::Low, Priority::Mid, Priority::High].iter().map(|p| p.title()).collect();
    assert_eq!(p, ["Низкий", "Средний", "Высокий"]);
}

#[test]
fn age_rules_as_in_csharp() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
    let t = |status: &str, changed: &str| card(&format!(r#"{{"Id": "a", "Status": "{status}", "StatusChangedAt": "{changed}"}}"#));
    // дни — по датам (дата записи — по её смещению), не по часам
    assert_eq!(t("InProgress", "2026-10-10T00:01:00+03:00").days_in_status(today), 0);
    assert_eq!(t("InProgress", "2026-10-09T23:59:00+03:00").days_in_status(today), 1);
    assert_eq!(t("InProgress", "2026-10-11T09:00:00+03:00").days_in_status(today), 0, "в будущем — 0, не минус");
    assert_eq!(card(r#"{"Id": "a"}"#).days_in_status(today), 0);
    // подпись
    assert_eq!(t("InProgress", "2026-10-10T09:00:00+03:00").age_label(today), "сегодня");
    assert_eq!(t("InProgress", "2026-10-06T09:00:00+03:00").age_label(today), "4 д");
    // состояние: просрочка 3 дня — внимание с 2, просрочена с 3; «Готово» — всегда нейтрально
    let r = rules();
    assert_eq!(t("InProgress", "2026-10-09T09:00:00+03:00").age_state(today, r), AgeState::Neutral);
    assert_eq!(t("InProgress", "2026-10-08T09:00:00+03:00").age_state(today, r), AgeState::Warn);
    assert_eq!(t("Waiting", "2026-10-07T09:00:00+03:00").age_state(today, r), AgeState::Overdue);
    assert_eq!(t("Done", "2026-09-01T09:00:00+03:00").age_state(today, r), AgeState::Neutral);
}

#[test]
fn old_done_cards_are_hidden() {
    let now = at("2026-10-10T12:00:00+03:00");
    let r = rules();
    let done = |completed: &str| card(&format!(r#"{{"Id": "a", "Status": "Done", "CompletedAt": "{completed}"}}"#));
    assert!(done("2026-10-03T11:59:00+03:00").hidden(now, r), "старше 7 дней — скрыта");
    assert!(!done("2026-10-03T12:01:00+03:00").hidden(now, r), "моложе — видна");
    assert!(!card(r#"{"Id": "a", "Status": "Done"}"#).hidden(now, r), "без даты — видна");
    assert!(
        !card(r#"{"Id": "a", "Status": "InProgress", "CompletedAt": "2026-01-01T00:00:00+03:00"}"#).hidden(now, r),
        "не в «Готово» — видна"
    );
}

#[test]
fn context_line_number_and_move() {
    let t = card(r#"{"Id": "a", "Service": "Почта", "CreatorCompany": "ООО Ромашка"}"#);
    assert_eq!(t.context(), "Почта · ООО Ромашка");
    assert_eq!(card(r#"{"Id": "a", "CreatorCompany": "ООО Ромашка"}"#).context(), "ООО Ромашка");
    assert_eq!(card(r#"{"Id": "a", "Service": " "}"#).context(), "");
    // перенос — как Ticket.MoveTo: дата смены колонки — сейчас, «выполнена» — только в «Готово»
    let now = at("2026-10-10T12:00:00+03:00");
    let mut t = card(r#"{"Id": "a", "Status": "InProgress", "StatusChangedAt": "2026-01-01T00:00:00+03:00"}"#);
    t.move_to(Status::Done, now);
    assert_eq!((t.status, t.status_changed_at, t.completed_at), (Status::Done, Some(now), Some(now)));
    t.move_to(Status::Inbox, now);
    assert_eq!((t.status, t.completed_at), (Status::Inbox, None));
    let before = t.status_changed_at;
    t.move_to(Status::Inbox, at("2026-10-11T12:00:00+03:00"));
    assert_eq!(t.status_changed_at, before, "в ту же колонку — ничего");
}

#[test]
fn board_columns_moves_and_limits() {
    let now = at("2026-10-10T12:00:00+03:00");
    let r = Rules { wip_limit: 2, ..rules() };
    let tickets = parse_tickets(
        r#"[{"Id": "1", "Status": "Inbox"}, {"Id": "2", "Status": "InProgress"}, {"Id": "3", "Status": "Inbox"},
            {"Id": "4", "Status": "InProgress"}, {"Id": "5", "Status": "Done", "CompletedAt": "2026-01-01T00:00:00+03:00"},
            {"Id": "6", "Status": "Done", "CompletedAt": "2026-10-09T00:00:00+03:00"}, {"Id": "", "Title": "без ключа"}]"#,
    )
    .unwrap();
    let mut board = Board::new(tickets);
    let ids = |b: &Board, s: Status| {
        b.visible(s, now, r).iter().map(|t| if t.title.is_empty() { t.id.clone() } else { t.title.clone() }).collect::<Vec<_>>()
    };
    assert_eq!(ids(&board, Status::Inbox), ["1", "3", "без ключа"], "порядок файла");
    assert_eq!(ids(&board, Status::Done), ["6"], "старая «Готово» скрыта");
    assert!(!board.overloaded(Status::InProgress, now, r), "2 при лимите 2 — не перегруз");
    // перенос — наверх колонки, как MoveTicket
    assert!(board.move_card("3", Status::InProgress, now));
    assert_eq!(ids(&board, Status::InProgress), ["3", "2", "4"]);
    assert_eq!(ids(&board, Status::Inbox), ["1", "без ключа"]);
    assert!(board.overloaded(Status::InProgress, now, r), "3 при лимите 2 — перегруз");
    assert!(!board.overloaded(Status::Inbox, now, r), "лимит — только у «В работе»");
    assert!(!board.move_card("3", Status::InProgress, now), "в ту же колонку — не перенос");
    assert!(!board.move_card("нет такой", Status::Done, now));
    assert_eq!(board.find("2").map(|t| t.status), Some(Status::InProgress));
    // карточка без Id получает свой ключ — переносится
    let key = board.visible(Status::Inbox, now, r)[1].id.clone();
    assert!(!key.is_empty());
    assert!(board.move_card(&key, Status::Waiting, now));
    // подписи колонок — как в C#
    assert_eq!(Board::hint(Status::InProgress, r), "лимит 2");
    assert_eq!(Board::hint(Status::Done, r), "скрыты старше 7 д");
    assert_eq!(Board::hint(Status::Inbox, r), "");
}

#[test]
fn age_text_as_in_the_panel() {
    // как Ticket.AgeText: «с сегодня» / «2 дня» / «5 дней — просрочена»; «Готово» не просрочивается
    let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
    let r = rules();
    let t = |status: &str, days: i64| {
        let changed = today - chrono::Days::new(days as u64);
        card(&format!(r#"{{"Id": "a", "Status": "{status}", "StatusChangedAt": "{changed}T10:00:00+03:00"}}"#)).age_text(today, r)
    };
    assert_eq!(t("Inbox", 0), "с сегодня");
    assert_eq!(t("Inbox", 1), "1 день");
    assert_eq!(t("Inbox", 2), "2 дня");
    assert_eq!(t("InProgress", 5), "5 дней — просрочена");
    assert_eq!(t("InProgress", 11), "11 дней — просрочена");
    assert_eq!(t("InProgress", 21), "21 день — просрочена");
    assert_eq!(t("Waiting", 22), "22 дня — просрочена");
    assert_eq!(t("Done", 5), "5 дней");
    assert_eq!(plural(14, "день", "дня", "дней"), "дней");
    assert_eq!(plural(104, "день", "дня", "дней"), "дня");
}

#[test]
fn hidden_boundary_and_read_errors() {
    // ровно N дней назад — ещё видна: скрыты только раньше (как `done < now.AddDays(-N)`)
    let now = at("2026-10-10T12:00:00+03:00");
    let t = card(r#"{"Id": "a", "Status": "Done", "CompletedAt": "2026-10-03T12:00:00+03:00"}"#);
    assert!(!t.hidden(now, rules()));
    // файл есть, но не читается (на его месте папка) — ошибка, а не «пустая доска»
    let dir = std::env::temp_dir().join(format!("tb-rs-board-dir-{}", std::process::id()));
    fs::create_dir_all(dir.join("tickets.json")).unwrap();
    let err = load_tickets(&dir.join("tickets.json")).expect_err("не читается — ошибка");
    assert!(err.contains("tickets.json"), "{err}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn duplicate_ids_get_their_own_keys() {
    // два одинаковых Id (правка руками, слияние) — у каждой карточки свой ключ: перенос одной не трогает другую
    let now = at("2026-10-10T12:00:00+03:00");
    let mut board =
        Board::new(parse_tickets(r#"[{"Id": "x", "Title": "первая"}, {"Id": "x", "Title": "вторая"}, {"Title": "третья"}]"#).unwrap());
    let keys: Vec<String> = board.tickets.iter().map(|t| t.id.clone()).collect();
    assert_eq!(keys.len(), 3);
    assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2], "{keys:?}");
    assert_eq!(keys[0], "x", "первая сохраняет свой Id");
    assert!(board.move_card(&keys[1], Status::Done, now));
    let titles = |s| board.visible(s, now, rules()).iter().map(|t| t.title.clone()).collect::<Vec<_>>();
    assert_eq!(titles(Status::Inbox), ["первая", "третья"]);
    assert_eq!(titles(Status::Done), ["вторая"]);
}
