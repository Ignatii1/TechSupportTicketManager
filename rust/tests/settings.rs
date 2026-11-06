//! `settings.json` TicketBoard.exe — как его читает `AppSettings.Load`.

use std::fs;
use ticketboard::settings::*;

#[test]
fn the_fields_the_prototype_needs() {
    let s = parse_settings(
        r#"{"Hotkey": "Ctrl+Alt+Space", "IntraserviceBaseUrl": "https://hd.example.ru/", "IntraserviceLogin": " me ",
            "IntraservicePasswordProtected": "AQAAANCMnd8=", "HideDoneOlderThanDays": 14, "WipLimit": 4, "OverdueDays": 5,
            "StatusOnMove": {"Done": {"Id": 29, "Name": "Выполнена"}}}"#,
    )
    .unwrap();
    assert_eq!(s.base_url, "https://hd.example.ru/");
    assert_eq!(s.login, " me ");
    assert_eq!(s.password_protected, "AQAAANCMnd8=");
    assert_eq!((s.hide_done_days, s.wip_limit, s.overdue_days), (14, 4, 5));
    assert_eq!(s.rules(), ticketboard::board::Rules { overdue_days: 5, hide_done_days: 14, wip_limit: 4 });
    assert!(s.has_api());
}

#[test]
fn defaults_as_in_appsettings() {
    let d = Settings::default();
    assert_eq!((d.hide_done_days, d.wip_limit, d.overdue_days), (7, 5, 3));
    assert!(!d.has_api());
    let s = parse_settings("{}").unwrap();
    assert_eq!(s, d);
    // API — адрес http(s), логин и пароль; без любого — нет
    let api = |url: &str, login: &str, pw: &str| {
        Settings { base_url: url.into(), login: login.into(), password_protected: pw.into(), ..d.clone() }.has_api()
    };
    assert!(api("http://hd", "me", "x"));
    assert!(!api("ftp://hd", "me", "x"));
    assert!(!api("https://hd", " ", "x"));
    assert!(!api("https://hd", "me", ""));
}

#[test]
fn comments_and_trailing_commas_as_appsettings_allows() {
    // тот же образец, что у AppSettings.SelfCheck: комментарий и висячая запятая — не порча
    let s = parse_settings("{\n  // мой сервер\n  \"IntraserviceBaseUrl\": \"https://hd\",\n  \"WipLimit\": 7,\n}").unwrap();
    assert_eq!((s.base_url.as_str(), s.wip_limit), ("https://hd", 7));
    // в строке «//» и «/*» — текст, не комментарий
    let s = parse_settings(r#"{"IntraserviceBaseUrl": "https://hd/a//b/*c*/", /* блок */ "IntraserviceLogin": "a,]b",}"#).unwrap();
    assert_eq!((s.base_url.as_str(), s.login.as_str()), ("https://hd/a//b/*c*/", "a,]b"));
    assert_eq!(strip_jsonc(r#"{"a": "x\"//y", "b": [1, 2,], }"#), r#"{"a": "x\"//y", "b": [1, 2] }"#);
}

#[test]
fn missing_and_corrupt_files() {
    let dir = std::env::temp_dir().join(format!("tb-rs-settings-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let _ = fs::remove_file(dir.join("settings.json"));
    assert_eq!(load_settings(&dir), (Settings::default(), String::new()), "нет файла — по умолчанию, без ошибки");
    fs::write(dir.join("settings.json"), "{\"WipLimit\": ").unwrap();
    let (s, problem) = load_settings(&dir);
    assert_eq!(s, Settings::default());
    assert!(problem.contains("settings.json"), "{problem}");
    assert_eq!(fs::read_to_string(dir.join("settings.json")).unwrap(), "{\"WipLimit\": ", "битый не трогается");
    fs::remove_dir_all(&dir).unwrap();
}
