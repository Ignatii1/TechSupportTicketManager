//! `tickets.json`, который пишет прототип: те же поля и значения, что прочтёт TicketBoard.exe (`StoreCheck`), поля, которых
//! модель не знает, — на месте; хранилище — как `TicketStore` (через `.tmp`, копия за день, 30 копий).

mod common;

use chrono::NaiveDate;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use ticketboard::board::*;
use ticketboard::store::*;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tb-rs-store-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Значение без `null`: C# их не пишет (`WhenWritingNull`), а в образце они есть.
fn without_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, without_nulls(v))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(without_nulls).collect()),
        v => v,
    }
}

/// Что прототип пишет из образца `StoreCheck`: ключи — в порядке, в каком их пишет C# (`Notes` третьим, сверено с
/// `TicketStore.Save`), `null` нет, UTF-8 как есть.
const GOLDEN: &str = r#"[
  {
    "Id": "9b2f6c1e-0000-4000-8000-000000000001",
    "CreatedAt": "2026-09-01T10:00:00+03:00",
    "Notes": [
      {
        "Id": "9b2f6c1e-0000-4000-8000-000000000002",
        "CreatedAt": "2026-09-06T15:00:00+03:00",
        "Text": "позвонить"
      }
    ],
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
    "Files": [
      {
        "Id": 900101,
        "Name": "экран.png"
      },
      {
        "Id": 900102
      }
    ],
    "CommentsCheckedFor": "2026-09-05T14:00:00+03:00",
    "CommentsSeenAt": "2026-09-05T14:00:01+03:00",
    "UnreadComments": 3,
    "AssignedToMe": false
  }
]"#;

#[test]
fn the_store_check_sample_round_trips() {
    let board = Board::new(parse_tickets(common::STORE_SAMPLE).unwrap());
    let written = tickets_json(&board).unwrap();
    assert_eq!(written, GOLDEN, "золотой текст");
    let sample: Value = serde_json::from_str(common::STORE_SAMPLE).unwrap();
    let back: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(back, without_nulls(sample), "значения — те же, что прочтёт C#");
    // и читается обратно в ту же карточку
    assert_eq!(parse_tickets(&written).unwrap(), board.all().cloned().collect::<Vec<_>>());
}

#[test]
fn unknown_fields_nulls_enums_and_dates() {
    let json = r#"[{"Id": "9b2f6c1e-0000-4000-8000-0000000000aa", "Title": null, "Status": "done", "Priority": 2,
        "Files": [], "FutureField": {"x": [1, 2]}, "UnreadComments": null,
        "StatusChangedAt": "2026-10-05T14:03:12.1234567+03:00", "CreatedAt": "2026-10-05T14:03:12.5+00:00",
        "Notes": [{"Text": "x", "Pinned": true}]}]"#;
    let t = parse_tickets(json).unwrap().remove(0);
    assert_eq!((t.title.as_str(), t.status, t.priority, t.unread_comments), ("", Status::Done, Priority::High, 0));
    assert_eq!(t.files, Some(vec![]), "[] — прочитан, вложений нет");
    let out: Value = serde_json::from_str(&tickets_json(&Board::new(vec![t.clone()])).unwrap()).unwrap();
    let o = &out[0];
    assert_eq!(o["FutureField"], serde_json::json!({"x": [1, 2]}), "чужое поле — на месте");
    assert_eq!((o["Status"].as_str(), o["Priority"].as_str()), (Some("Done"), Some("High")), "пишутся именами");
    assert_eq!(o["Files"], serde_json::json!([]));
    assert_eq!(o["Title"], "", "строки — всегда");
    assert_eq!(o["UnreadComments"], 0);
    assert_eq!(o["StatusChangedAt"], "2026-10-05T14:03:12.1234567+03:00", "доли — до 7 знаков, как у .NET");
    assert_eq!(o["CreatedAt"], "2026-10-05T14:03:12.5+00:00", "без хвостовых нулей, пояс — +00:00");
    assert_eq!(o["Notes"][0]["Pinned"], true, "чужое поле заметки — на месте");
    assert!(is_guid(o["Notes"][0]["Id"].as_str().unwrap()), "заметке без номера — новый Guid");
    assert!(o.get("CompletedAt").is_none() && o.get("IntraserviceId").is_none(), "null не пишется");
    // Files: null — «не читали», не пишется
    let t = parse_tickets(r#"[{"Id": "9b2f6c1e-0000-4000-8000-0000000000ab", "Files": null}]"#).unwrap().remove(0);
    assert_eq!(t.files, None);
    assert!(!tickets_json(&Board::new(vec![t])).unwrap().contains("Files"));
    // статус числом и строкой с числом — как JsonStringEnumConverter; вне списка — файл не читается (как у C#)
    let status = |s: &str| parse_tickets(&format!(r#"[{{"Status": {s}}}]"#)).map(|t| t[0].status);
    assert_eq!(status("3"), Ok(Status::Done));
    assert_eq!(status(r#""2""#), Ok(Status::Waiting));
    assert_eq!(status(r#""INPROGRESS""#), Ok(Status::InProgress));
    assert_eq!(status("null"), Ok(Status::Inbox));
    assert!(status(r#""Archived""#).is_err() && status("7").is_err() && status("-1").is_err());
    assert!(status("4").is_err() && status(r#""4""#).is_err(), "колонок четыре: 4 — уже нет");
    let err = status("true").expect_err("не строка и не число");
    assert!(err.contains("одно из") && err.contains("Inbox"), "{err}");
    let priority = |s: &str| parse_tickets(&format!(r#"[{{"Priority": {s}}}]"#)).map(|t| t[0].priority);
    assert_eq!((priority("null"), priority("0")), (Ok(Priority::Mid), Ok(Priority::Low)), "null — по умолчанию «средний»");
    // дата без пояса — местное время, как DateTimeOffset у .NET
    let t = parse_tickets(r#"[{"StatusChangedAt": "2026-09-01T10:00:00"}]"#).unwrap().remove(0);
    let local =
        chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap().and_hms_opt(10, 0, 0).unwrap().and_local_timezone(chrono::Local).unwrap();
    assert_eq!(t.status_changed_at, local.fixed_offset());
    assert!(parse_tickets(r#"[{"StatusChangedAt": "вчера"}]"#).is_err(), "не дата — файл не читается");
}

#[test]
fn ids_missing_wrong_and_repeated() {
    let g = "9b2f6c1e-0000-4000-8000-000000000001";
    let tickets = parse_tickets(&format!(r#"[{{"Id": "{g}"}}, {{"Id": "{g}"}}, {{"Id": "a"}}, {{}}, {{"Id": null}}]"#)).unwrap();
    assert_eq!(tickets[0].id, g, "свой — как был");
    assert!(tickets[1..].iter().all(|t| is_guid(&t.id) && t.id != g), "повтор, не Guid, нет, null — новый");
    let ids: std::collections::HashSet<&str> = tickets.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids.len(), 5, "у каждой свой");
    assert!(is_guid(&new_guid()) && new_guid() != new_guid());
    let v = new_guid();
    assert_eq!(&v[14..15], "4", "версия 4");
    assert!(matches!(v.as_bytes()[19], b'8' | b'9' | b'a' | b'b'), "вариант RFC 4122: {v}");
    assert!(!is_guid("9b2f6c1e-0000-4000-8000-00000000000g") && !is_guid("9b2f6c1e00004000800000000000000a"));
}

#[test]
fn the_store_writes_through_tmp_and_keeps_daily_copies() {
    let dir = temp("write");
    let store = Store::new(&dir);
    assert_eq!(store.load().map(|t| t.len()), Ok(0), "файла нет — пустая доска");
    let today = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    assert_eq!(store.backup(today), Ok(()), "файла нет — копировать нечего");
    assert!(!store.backup_dir().exists());
    let mut board = Board::new(parse_tickets(common::STORE_SAMPLE).unwrap());
    store.write(&board).unwrap();
    assert!(store.file().exists() && !dir.join("tickets.json.tmp").exists(), "через .tmp, его не осталось");
    assert_eq!(store.load().unwrap().len(), 1);
    let first = fs::read(store.file()).unwrap();

    // копия за день — прежнего файла, одна
    store.backup(today).unwrap();
    let copy = store.backup_dir().join("tickets-2026-11-06.json");
    assert_eq!(fs::read(&copy).unwrap(), first);
    board.insert(Ticket::new(now()));
    store.write(&board).unwrap();
    store.backup(today).unwrap();
    assert_eq!(fs::read(&copy).unwrap(), first, "второй раз за день — не переписывается");

    // старше тридцати — долой (по имени), чужое не трогается
    for d in 1..=31 {
        fs::write(store.backup_dir().join(format!("tickets-2026-09-{d:02}.json")), "[]").unwrap();
    }
    fs::write(store.backup_dir().join("заметки.txt"), "своё").unwrap();
    fs::write(store.backup_dir().join("a.json"), "своё").unwrap();
    store.backup(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()).unwrap();
    let mut names: Vec<String> = fs::read_dir(store.backup_dir()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    assert_eq!(names.len(), 32, "30 копий и два чужих файла: {names:?}");
    assert!(names.contains(&"заметки.txt".to_string()) && names.contains(&"a.json".to_string()), "не tickets-*.json — не трогается");
    // было 31 + за 06.11, стала и за 07.11: 33 — три самых старых по имени долой
    let has = |n: &str| names.contains(&format!("tickets-2026-{n}.json"));
    assert!(!has("09-01") && !has("09-02") && !has("09-03"), "{names:?}");
    assert!(has("09-04") && has("11-06") && has("11-07"), "{names:?}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unreadable_file_is_left_alone_and_a_failed_write_says_why() {
    let dir = temp("broken");
    let store = Store::new(&dir);
    fs::write(store.file(), "[{\"Id\": \"a\", ").unwrap();
    let err = store.load().expect_err("битый файл — ошибка");
    assert!(err.starts_with("tickets.json не прочитан"), "{err}");
    assert_eq!(fs::read(store.file()).unwrap(), b"[{\"Id\": \"a\", ", "файл не тронут");
    assert!(fs::read_dir(&dir).unwrap().count() == 1, "ничего рядом не появилось");

    // на месте файла — папка: записать нельзя, ошибка словами
    let dir = temp("blocked");
    let store = Store::new(&dir);
    fs::create_dir(store.file()).unwrap();
    let err = store.write(&Board::default()).expect_err("не записать поверх папки");
    assert!(err.starts_with("tickets.json не записан"), "{err}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_saved_by_notepad_still_loads() {
    // как File.ReadAllText: BOM UTF-8 и UTF-16 снимается, битый байт — U+FFFD, а не «файл не читается»
    let dir = temp("bom");
    let store = Store::new(&dir);
    let json = r#"[{"Title": "Принтер"}]"#;
    fs::write(store.file(), [b"\xEF\xBB\xBF".as_slice(), json.as_bytes()].concat()).unwrap();
    assert_eq!(store.load().map(|t| t[0].title.clone()), Ok("Принтер".into()), "BOM UTF-8");
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain(json.encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
    fs::write(store.file(), utf16).unwrap();
    assert_eq!(store.load().map(|t| t[0].title.clone()), Ok("Принтер".into()), "UTF-16 LE");
    fs::write(store.file(), b"[{\"Title\": \"a\xFFb\"}]").unwrap();
    assert_eq!(store.load().map(|t| t[0].title.clone()), Ok("a\u{FFFD}b".into()), "битый байт");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn repeated_note_ids_get_their_own() {
    // скопированная руками заметка: удаление одной не уносит другую
    let n = "9b2f6c1e-0000-4000-8000-0000000000aa";
    let json = format!(
        r#"[{{"Notes": [{{"Id": "{n}", "Text": "a"}}, {{"Id": "{n}", "Text": "b"}}]}}, {{"Notes": [{{"Id": "{n}", "Text": "c"}}]}}]"#
    );
    let mut tickets = parse_tickets(&json).unwrap();
    assert_eq!(tickets[0].notes[0].id, n);
    assert!(tickets[0].notes[1].id != n && is_guid(&tickets[0].notes[1].id), "повтор в карточке — новый");
    assert_eq!(tickets[1].notes[0].id, n, "в другой карточке — свой счёт");
    assert!(tickets[0].delete_note(n));
    assert_eq!(tickets[0].notes.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["b"]);
}
