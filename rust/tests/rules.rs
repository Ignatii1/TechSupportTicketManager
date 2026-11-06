//! Правила автообновления — образцы `AutoSyncRules.SelfCheck` один в один (`Skip`/`ToAdd`, `Track`, `Unread`, `SeenFrom`,
//! `RequesterReply`, `ReadAllPagesAsync`).

use chrono::{DateTime, Duration, FixedOffset, TimeZone};
use std::collections::HashSet;
use ticketboard::intraservice::{Event, Found, SearchPage, User};
use ticketboard::rules::*;

fn set(ids: &[i32]) -> HashSet<i32> {
    ids.iter().copied().collect()
}

#[test]
fn what_auto_sync_adds() {
    // первый запуск: открытое, чего нет на доске, — прошлое; во «Входящие» ничего не падает
    let listed = [1, 2, 3, 4];
    let board = set(&[1]);
    let (skipped, persist) = skip(None, &listed, &board, true);
    assert!(persist && skipped == set(&[2, 3, 4]) && to_add(&listed, &board, &skipped).is_empty());
    // первый запуск на неполном списке: не добавить ничего и ничего не запомнить
    let (skipped, persist) = skip(None, &listed, &board, false);
    assert!(!persist && to_add(&listed, &board, &skipped).is_empty());
    // потом: новая на меня — добавляется; удалённая с доски (в skip) — нет; закрытая или переданная (9) — забыта
    let later = [1, 2, 3, 4, 5];
    let (skipped, persist) = skip(Some(&[2, 3, 4, 9]), &later, &board, true);
    assert!(persist && skipped == set(&[2, 3, 4]) && to_add(&later, &board, &skipped) == [5]);
    // неполный список ничего не забывает и ничего не сохраняет
    let (skipped, persist) = skip(Some(&[2, 9]), &[1], &board, false);
    assert!(!persist && skipped == set(&[2, 9]));
    // пустой сохранённый список — это «уже запускалось», а не первый запуск
    assert!(skip(Some(&[]), &listed, &board, true).0.is_empty());
    // порядок и повторы списка — как пришли
    assert_eq!(to_add(&[7, 5, 7, 1], &board, &HashSet::new()), [7, 5, 7]);
}

#[test]
fn lifecycle() {
    // первая встреча — только отсчёт, событий нет
    assert_eq!(track(None, true, true, true, None), (Some(true), Lifecycle::None));
    assert_eq!(track(None, false, true, false, Some(false)), (Some(false), Lifecycle::None));
    // в «Готово», а снова в моих открытых — вернуть в работу
    assert_eq!(track(Some(false), true, true, true, None), (Some(true), Lifecycle::Reopened));
    // перенёс в «Готово» сам, пока она ещё открыта, — не «снова открыта»
    assert_eq!(track(Some(true), true, true, true, None), (Some(true), Lifecycle::None));
    // выпала из моих открытых: перечитали — открыта, значит, передали; закрыта — своё уведомление; не перечитали — ждём
    assert_eq!(track(Some(true), false, true, false, Some(false)), (Some(false), Lifecycle::Reassigned));
    assert_eq!(track(Some(true), false, true, false, Some(true)), (Some(false), Lifecycle::None));
    assert_eq!(track(Some(true), false, true, false, None), (Some(true), Lifecycle::None));
    // неполный список ничего не решает; «Готово» выпавшую просто забывает
    assert_eq!(track(Some(true), false, false, false, Some(false)), (Some(true), Lifecycle::None));
    assert_eq!(track(Some(true), false, true, true, None), (Some(false), Lifecycle::None));
    // чужая, которую отслеживаю сам, — тишина
    assert_eq!(track(Some(false), false, true, false, Some(false)), (Some(false), Lifecycle::None));
}

fn t0() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 9, 26, 10, 0, 0).unwrap()
}

fn comment(minutes: i64, author: &str, author_id: Option<i32>, text: Option<&str>) -> Event {
    Event {
        date: Some(t0() + Duration::minutes(minutes)),
        author: author.into(),
        status: "Открыта".into(),
        comment: text.map(String::from),
        is_public: Some(true),
        author_id,
        is_final: false,
    }
}

#[test]
fn unread_comments() {
    let me = User { id: 7, name: "Я Сам".into() };
    // первая проверка: старая переписка прочитана, отсчёт — самый новый комментарий
    let history = vec![
        comment(-30, "Иванов", Some(1), Some("текст")),
        comment(-10, "Петров", Some(2), Some("текст")),
        comment(-5, "Сидоров", Some(3), None),
    ];
    let first = unread(&history, None, Some(&me));
    assert_eq!((first.count, first.seen), (0, Some(t0() - Duration::minutes(10))));
    assert_eq!(unread(&[], None, Some(&me)).seen, None);
    // после отсчёта: чужой — новый; смена статуса без текста — нет; порядок ответа сервера не важен
    let mut later = history.clone();
    later.push(comment(5, "Иванов", Some(1), Some("текст")));
    later.push(comment(7, "Сидоров", Some(3), None));
    later.reverse();
    let u = unread(&later, Some(t0()), Some(&me));
    assert_eq!((u.count, u.seen, u.unread[0].date), (1, Some(t0()), Some(t0() + Duration::minutes(5))));
    // ровно в момент «прочитано до» — уже прочитан
    assert_eq!(unread(&[comment(0, "Иванов", Some(1), Some("текст"))], Some(t0()), Some(&me)).count, 0);
    // свой ответ: всё до него прочитано, после — снова новое; свой не считается никогда
    let mut replied = later.clone();
    replied.extend([
        comment(6, "Я Сам", Some(7), Some("текст")),
        comment(8, "Петров", Some(2), Some("текст")),
        comment(9, "Я Сам", Some(7), Some("текст")),
    ]);
    let u = unread(&replied, Some(t0()), Some(&me));
    assert_eq!((u.count, u.seen), (0, Some(t0() + Duration::minutes(9))));
    replied.extend([comment(12, "Петров", Some(2), Some("текст")), comment(11, "Иванов", Some(1), Some("текст"))]);
    let u = unread(&replied, Some(t0()), Some(&me));
    assert_eq!(u.count, 2);
    assert_eq!(u.unread.iter().map(|e| e.author.as_str()).collect::<Vec<_>>(), ["Петров", "Иванов"], "свежие сверху");
    // отсчёт от Changed с запасом: Changed в секундах, а комментарий, который его сдвинул, — с миллисекундами
    let changed = FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 9, 26, 10, 15, 0).unwrap();
    let trigger = Event {
        date: Some(changed + Duration::milliseconds(480)),
        comment: Some("тот самый".into()),
        ..comment(0, "Иванов", Some(1), None)
    };
    assert_eq!(unread(std::slice::from_ref(&trigger), Some(seen_from(changed)), Some(&me)).count, 0);
    assert_eq!(unread(std::slice::from_ref(&trigger), Some(changed), Some(&me)).count, 1, "без запаса он сошёл бы за новый");
    assert_eq!(seen_from(changed), changed + Duration::seconds(1));
    // номера автора нет — узнаём себя по имени (без учёта регистра); без даты — не в счёт
    let no_ids = vec![
        comment(5, "я сам", None, Some("текст")),
        comment(6, "Иванов", None, Some("текст")),
        Event { date: None, ..comment(0, "Петров", None, Some("без даты")) },
    ];
    let u = unread(&no_ids, Some(t0()), Some(&me));
    assert_eq!((u.count, u.seen), (1, Some(t0() + Duration::minutes(5))));
    // кто я — неизвестно: своих нет, всё чужое
    assert_eq!(unread(&no_ids, Some(t0()), None).count, 2);
    // одинаковые даты — в порядке ответа (сортировка устойчивая)
    let ties = vec![comment(5, "А", Some(1), Some("1")), comment(5, "Б", Some(2), Some("2"))];
    assert_eq!(unread(&ties, Some(t0()), Some(&me)).unread.iter().map(|e| e.author.as_str()).collect::<Vec<_>>(), ["А", "Б"]);
}

#[test]
fn requester_and_names() {
    // ответ инициатора: его комментарий среди новых — самый свежий из его; коллега и система — не ответ
    let said = |author: &str, minutes| comment(minutes, author, None, Some("текст"));
    let fresh = vec![said("Иванов И. И.", 9), said("Петрова  Анна ", 5), said("Петрова Анна", 1)];
    assert_eq!(requester_reply(&fresh, Some("петрова анна")).and_then(|e| e.date), Some(t0() + Duration::minutes(5)));
    assert!(requester_reply(&[said("Иванов И. И.", 1), said("IntraService", 2)], Some("Петрова Анна")).is_none());
    assert!(requester_reply(&fresh, None).is_none() && requester_reply(&fresh, Some(" ")).is_none());
    assert!(same_name("  Петрова\tАнна ", "ПЕТРОВА АННА") && !same_name("Петрова Анна", "Петров Анна"));
}

#[test]
fn closed_names() {
    // список правят руками: пустые строки и пробелы — не в счёт, регистр — тоже
    let closed = ClosedNames::new(&[" Закрыта ".to_string(), "".into(), "  ".into(), "выполнена".into()]);
    assert!(closed.contains("закрыта") && closed.contains("ВЫПОЛНЕНА") && !closed.contains("Открыта") && !closed.contains(""));
    assert_eq!(closed.len(), 2);
    assert!(!closed.is_empty() && ClosedNames::new(&[" ".to_string()]).is_empty());
    // для отчёта — как написаны, без пробелов по краям, по алфавиту; повтор в другом регистре — первый
    let mut closed = closed;
    closed.add("ЗАКРЫТА");
    closed.add("Отменена ");
    assert_eq!(closed.names(), ["Закрыта", "Отменена", "выполнена"]);
}

/// Листание: сервер отдаёт страницы из `pages` (номера заявок, его Total и ошибка); просим по 4 на страницу, не больше 3.
struct Read {
    rows: Vec<i32>,
    total: i32,
    complete: bool,
    error: String,
    asked: i32,
}

fn read_with(
    stop_at: Option<&dyn Fn(&Found) -> bool>,
    max_rows: usize,
    skip: Option<&dyn Fn(&Found) -> bool>,
    pages: &[(&[i32], i32, &str)],
) -> Read {
    let mut asked = 0;
    let r = read_all_pages(
        |page| {
            asked += 1;
            let (ids, total, error) = pages.get(page as usize - 1).copied().unwrap_or((&[], 0, ""));
            if !error.is_empty() {
                return Err(error.to_string());
            }
            let found =
                ids.iter().map(|&id| Found { id, name: format!("Заявка {id}"), status: "Открыта".into(), ..Found::default() }).collect();
            Ok(SearchPage { found, total, has_next: None })
        },
        4,
        3,
        stop_at,
        max_rows,
        skip,
    );
    Read { rows: r.rows.iter().map(|f| f.id).collect(), total: r.total, complete: r.complete, error: r.error, asked }
}

fn read(pages: &[(&[i32], i32, &str)]) -> Read {
    read_with(None, usize::MAX, None, pages)
}

#[test]
fn pages() {
    // одна неполная страница — весь список, второй запрос не нужен; пусто — тоже целый список
    let one = read(&[(&[1, 2, 3], 3, "")]);
    assert!(one.complete && one.rows.len() == 3 && one.asked == 1);
    let none = read(&[(&[], 0, "")]);
    assert!(none.complete && none.rows.is_empty());
    // без Paginator Total равен пришедшему: полная страница — ещё не конец
    let no_paginator = read(&[(&[1, 2, 3, 4], 4, ""), (&[5, 6], 2, "")]);
    assert!(no_paginator.complete && no_paginator.rows.len() == 6 && no_paginator.asked == 2);
    // сервер отдаёт меньше, чем просили (по 2 вместо 4), но Total знает — листаем, пока не заберём всё
    let capped = read(&[(&[1, 2], 5, ""), (&[3, 4], 5, ""), (&[5], 5, "")]);
    assert!(capped.complete && capped.rows.len() == 5 && capped.asked == 3 && capped.total == 5);
    // полная последняя страница — конец узнаём по пустой следующей
    let exact = read(&[(&[1, 2, 3, 4], 4, "")]);
    assert!(exact.complete && exact.asked == 2);
    // список сдвинулся, пока листали: номер 4 пришёл дважды (второй раз пропущен), одна заявка проскочила — не целый
    let shifted = read(&[(&[1, 2, 3, 4], 8, ""), (&[4, 5, 6], 7, ""), (&[], 7, "")]);
    assert!(!shifted.complete && shifted.rows == [1, 2, 3, 4, 5, 6] && shifted.error.is_empty());
    // ошибка на второй странице: первая пригодится, но список не целый
    let broken = read(&[(&[1, 2, 3, 4], 6, ""), (&[], 0, "HTTP 500")]);
    assert!(!broken.complete && broken.rows.len() == 4 && broken.error == "HTTP 500");
    // упёрлись в потолок страниц (3 полных, а заявок больше) — не целый
    let ceiling = read(&[(&[1, 2, 3, 4], 20, ""), (&[5, 6, 7, 8], 20, ""), (&[9, 10, 11, 12], 20, "")]);
    assert!(!ceiling.complete && ceiling.rows.len() == 12 && ceiling.asked == 3);
}

#[test]
fn pages_for_the_export() {
    // дальше строки старше периода список не нужен — следующую страницу не просим; потолок строк — так же
    let stop = |f: &Found| f.id == 3;
    let cut = read_with(Some(&stop), usize::MAX, None, &[(&[1, 2, 3, 4], 20, ""), (&[5, 6, 7, 8], 20, "")]);
    assert!(!cut.complete && cut.rows == [1, 2] && cut.asked == 1);
    let capped3 = read_with(None, 3, None, &[(&[1, 2, 3, 4], 20, ""), (&[5, 6, 7, 8], 20, "")]);
    assert!(!capped3.complete && capped3.rows.len() == 3 && capped3.asked == 1);
    // потолок набран ровно на конце страницы — следующую не просим; повтор номера в потолок не засчитывается
    let capped4 = read_with(None, 4, None, &[(&[1, 2, 3, 4], 20, ""), (&[5, 6, 7, 8], 20, "")]);
    assert!(!capped4.complete && capped4.rows.len() == 4 && capped4.asked == 1);
    // не из выборки (чётные): не в список и не в счёт потолка — набрали три нужных, лишней страницы нет
    let even = |f: &Found| f.id % 2 == 0;
    let skipping = read_with(None, 3, Some(&even), &[(&[1, 2, 3, 4], 20, ""), (&[5, 6, 7, 8], 20, ""), (&[9, 10, 11, 12], 20, "")]);
    assert!(skipping.rows == [1, 3, 5] && skipping.asked == 2);
    // старая строка останавливает чтение раньше, чем пропуск её проглотил бы: дальше только старее
    let six = |f: &Found| f.id == 6;
    let stops_first = read_with(Some(&six), usize::MAX, Some(&even), &[(&[1, 2, 3, 4], 20, ""), (&[5, 6, 7, 8], 20, "")]);
    assert!(stops_first.rows == [1, 3, 5] && stops_first.asked == 2);
    let dup_capped = read_with(None, 5, None, &[(&[1, 2, 3, 4], 20, ""), (&[4, 5, 6, 7], 20, "")]);
    assert!(dup_capped.rows == [1, 2, 3, 4, 5] && dup_capped.asked == 2);
}
