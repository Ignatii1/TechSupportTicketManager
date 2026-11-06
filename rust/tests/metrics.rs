//! Цифры прототипа: размер exe, память, время от запуска процесса.

use ticketboard::metrics::*;

#[test]
fn numbers_are_there_and_formatted() {
    assert_eq!(megabytes(65_649_363), "65,6 МБ");
    assert_eq!(megabytes(0), "0,0 МБ");
    assert_eq!(megabytes(12_345_678_901), "12345,7 МБ");
    let exe = std::fs::metadata(std::env::current_exe().unwrap()).unwrap().len();
    assert_eq!(exe_size(), Some(exe));
    let m = memory().expect("память процесса читается");
    // процесс с загруженным Slint — заведомо больше мегабайта; частная — не больше рабочего набора с запасом на подкачку
    assert!(m.working > 1_000_000 && m.private > 100_000, "{m:?}");
    // время от запуска: идёт вперёд вместе с часами
    let before = since_start().expect("время от запуска процесса читается");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let after = since_start().unwrap();
    let step = after.saturating_sub(before).as_millis();
    assert!((250..1000).contains(&step), "за 300 мс прошло {step} мс ({before:?} → {after:?})");
    assert!(after.as_secs() < 600, "{after:?}");
}
