//! `errors.log` — как `App.AppendLog`.

use std::fs;
use ticketboard::journal::{append, append_at};

#[test]
fn lines_and_two_writers() {
    let dir = std::env::temp_dir().join(format!("tb-rs-journal-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap().and_hms_opt(21, 9, 34).unwrap();
    append_at(&dir, "tickets.json не записан: занят", at);
    append_at(&dir, "вторая\nв две строки", at);
    assert_eq!(
        fs::read_to_string(dir.join("errors.log")).unwrap(),
        "[2026-11-06 21:09:34] tickets.json не записан: занят\n\n[2026-11-06 21:09:34] вторая\nв две строки\n\n"
    );
    // два потока — все записи целы
    let threads: Vec<_> = (0..2)
        .map(|k| {
            let dir = dir.clone();
            std::thread::spawn(move || (0..50).for_each(|i| append(&dir, &format!("поток {k}, запись {i}"))))
        })
        .collect();
    threads.into_iter().for_each(|t| t.join().unwrap());
    let text = fs::read_to_string(dir.join("errors.log")).unwrap();
    assert_eq!(text.matches("] поток ").count(), 100);
    assert!(text.ends_with("\n\n"));
    // папки нет — журнал молчит, а не падает
    append(&dir.join("нет такой"), "мимо");
    fs::remove_dir_all(&dir).unwrap();
}
