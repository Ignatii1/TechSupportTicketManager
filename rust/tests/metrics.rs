//! Цифры прототипа: размер exe, память, время от запуска процесса.

use std::time::Duration;
use ticketboard::metrics::*;

#[test]
fn where_the_start_went() {
    // этапы — отметки от создания процесса по порядку; последний («первый кадр») — от последней отметки до окна на экране
    let ms = Duration::from_millis;
    assert_eq!(startup_text(ms(4141), &[]), "окно на экране через 4141 мс");
    let marks = [("до main", ms(3800)), ("доска и окна", ms(4050)), ("окно", ms(4070)), ("трей и клавиша", ms(4100))];
    assert_eq!(
        startup_text(ms(4141), &marks),
        "окно на экране через 4141 мс (до main 3800, доска и окна 250, окно 20, трей и клавиша 30, первый кадр 41)"
    );
    // отметка позже итога (часы шагнули) — ноль, не переполнение
    assert_eq!(startup_text(ms(100), &[("до main", ms(150))]), "окно на экране через 100 мс (до main 150, первый кадр 0)");
    assert_eq!(startup_text(ms(300), &[("а", ms(200)), ("б", ms(100))]), "окно на экране через 300 мс (а 200, б 0, первый кадр 200)");
}

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
