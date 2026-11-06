//! Цифры прототипа: размер exe, память, время от запуска процесса.

use ticketboard::metrics::*;

#[test]
fn numbers_are_there_and_formatted() {
    assert_eq!(megabytes(65_649_363), "65,6 МБ");
    assert_eq!(megabytes(0), "0,0 МБ");
    assert_eq!(megabytes(12_345_678_901), "12345,7 МБ");
    assert!(exe_size().is_some_and(|b| b > 0));
    let m = memory().expect("память процесса читается");
    assert!(m.working > 0 && m.private > 0, "{m:?}");
    let up = since_start().expect("время от запуска процесса читается");
    assert!(up.as_secs() < 3600, "{up:?}");
}
