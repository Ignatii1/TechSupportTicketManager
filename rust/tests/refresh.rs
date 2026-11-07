//! Импорт моих и F5 — правила без окна (`MainViewModel.Intraservice.cs`; случаи `BoardCheck`, разделы 1–2).

use chrono::{DateTime, FixedOffset};
use ticketboard::board::*;
use ticketboard::intraservice::{Found, MyOpen, Status as ServerStatus};
use ticketboard::refresh::*;
use ticketboard::rules::ClosedNames;
use ticketboard::settings::Settings;

fn at(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

fn settings() -> Settings {
    Settings { base_url: "https://hd".into(), ..Settings::default() }
}

fn row(id: i32, name: &str) -> Found {
    Found { id, name: name.into(), status: "Открыта".into(), ..Default::default() }
}

fn mine(rows: Vec<Found>) -> MyOpen {
    MyOpen { total: rows.len() as i32, rows, complete: true, ..Default::default() }
}

fn card(id: i32, title: &str, status: Status) -> Ticket {
    let mut t = Ticket::new(at("2026-10-01T09:00:00+03:00"));
    t.intraservice_id = Some(id);
    t.title = title.into();
    t.status = status;
    t
}

fn numbers(board: &Board, s: Status) -> Vec<i32> {
    board.all().filter(|t| t.status == s).filter_map(|t| t.intraservice_id).collect()
}

#[test]
fn a_card_from_a_list_row() {
    let now = at("2026-10-10T12:00:00+03:00");
    let changed = at("2026-10-09T15:00:00+03:00");
    let f = Found {
        creator: Some("Петрова А.".into()),
        creator_phone: Some("+7 900".into()),
        creator_email: Some("p@example.ru".into()),
        executors: Some("Я Сам".into()),
        executor_group: Some("ТП".into()),
        description: Some("с утра".into()),
        changed: Some(changed),
        ..row(101, "Принтер не печатает")
    };
    let t = new_card(&f, &settings(), now);
    assert_eq!(
        (t.title.as_str(), t.intraservice_id, t.url.as_str(), t.description.as_str(), t.external_status.as_deref()),
        ("Принтер не печатает", Some(101), "https://hd/Task/View/101", "с утра", Some("Открыта"))
    );
    assert_eq!((t.priority, t.status, t.last_sync_at, t.assigned_to_me), (Priority::Mid, Status::Inbox, Some(now), Some(true)));
    assert_eq!(
        [&t.creator, &t.creator_phone, &t.creator_email, &t.executors, &t.executor_group].map(|v| v.as_deref()),
        [Some("Петрова А."), Some("+7 900"), Some("p@example.ru"), Some("Я Сам"), Some("ТП")]
    );
    // переписка до появления на доске — прочитана: отсчёт с Changed, «прочитано до» — на секунду позже
    assert_eq!((t.comments_checked_for, t.comments_seen_at), (Some(changed), Some(at("2026-10-09T15:00:01+03:00"))));
    let bare = new_card(&row(5, "Голая"), &Settings::default(), now);
    assert_eq!((bare.url.as_str(), bare.description.as_str(), bare.comments_seen_at), ("", "", None), "адреса нет — ссылки нет");
}

#[test]
fn import_adds_only_new_ones_to_the_end_of_inbox() {
    let now = at("2026-10-10T12:00:00+03:00");
    let mut moved = card(102, "Своё название", Status::InProgress);
    moved.description = "своими словами".into();
    let mut board = Board::new(vec![card(1, "уже во «Входящих»", Status::Inbox), moved]);
    let r = import(&mut board, &mine(vec![row(101, "a"), row(102, "Нет доступа к 1С"), row(101, "a"), row(199, "c")]), &settings(), now);
    assert_eq!(r, Imported { added: vec![101, 199], had: 2, text: "Добавлено: 2, уже было: 2".into() });
    assert_eq!(numbers(&board, Status::Inbox), [1, 101, 199], "в конец «Входящих», в порядке списка");
    let t = board.all().find(|t| t.intraservice_id == Some(102)).unwrap();
    assert_eq!((t.status, t.title.as_str(), t.description.as_str()), (Status::InProgress, "Своё название", "своими словами"), "не тронута");
    // ещё раз — всё уже было
    let again = import(&mut board, &mine(vec![row(101, "a"), row(102, "b"), row(199, "c")]), &settings(), now);
    assert_eq!((again.added.len(), again.text.as_str()), (0, "Добавлено: 0, уже было: 3"));
    assert_eq!(board.len(), 4);
}

#[test]
fn import_says_what_it_did_not_get() {
    let now = at("2026-10-10T12:00:00+03:00");
    let mut board = Board::default();
    let broken = MyOpen { error: "ошибка сервера (HTTP 500)".into(), ..mine(vec![row(1, "a")]) };
    assert_eq!(
        import(&mut board, &broken, &settings(), now).text,
        "Добавлено: 1, уже было: 0\nЗагружены не все страницы: ошибка сервера (HTTP 500)"
    );
    let capped = MyOpen { total: 2500, complete: false, ..mine(vec![row(2, "b")]) };
    assert_eq!(
        import(&mut board, &capped, &settings(), now).text,
        "Добавлено: 1, уже было: 0\nВзяты первые 1 из 2500 — запустите импорт ещё раз"
    );
    let none = import(&mut board, &MyOpen { error: "неверный логин или пароль (HTTP 401)".into(), ..MyOpen::default() }, &settings(), now);
    assert_eq!((none.added.len(), none.text.as_str()), (0, "неверный логин или пароль (HTTP 401)"));
    let empty = import(&mut board, &mine(vec![]), &settings(), now);
    assert_eq!(empty.text, "открытых заявок, где вы исполнитель, не нашлось");
    assert_eq!(board.len(), 2, "пустой ответ доску не трогает");
}

#[test]
fn what_f5_rereads_and_what_is_closed() {
    let mut own = Ticket::new(at("2026-10-01T09:00:00+03:00"));
    own.title = "без номера".into();
    let board = Board::new(vec![
        card(101, "a", Status::Inbox),
        own,
        card(102, "b", Status::InProgress),
        card(103, "c", Status::Done),
        card(104, "d", Status::Waiting),
    ]);
    let numbers: Vec<i32> = to_refresh(&board).into_iter().map(|(_, n)| n).collect();
    assert_eq!(numbers, [101, 102, 104], "с номером и не из «Готово»");
    // закрытые: из настроек и по признакам справочника
    let statuses = Ok(vec![
        ServerStatus { id: 31, name: "Открыта".into(), ..Default::default() },
        ServerStatus { id: 28, name: "Решена".into(), is_fixed: true, ..Default::default() },
        ServerStatus { id: 29, name: "Архив".into(), is_final: true, ..Default::default() },
    ]);
    let s = Settings { closed_status_names: vec!["Отменена".into()], ..Settings::default() };
    let closed = closed_names(&s, &statuses);
    assert!(closed.contains("решена") && closed.contains("Архив") && closed.contains("Отменена") && !closed.contains("Открыта"));
    let only_list = closed_names(&s, &Err("нет справочника".into()));
    assert!(only_list.contains("Отменена") && !only_list.contains("Решена") && only_list.len() == 1);
}

#[test]
fn the_summary_as_in_csharp() {
    let failed = vec![(199, "заявка не найдена (HTTP 404):\n{\"Message\":\"нет\"}".to_string()), (200, "x".into())];
    assert_eq!(summary(2, &[], None, |e| e.to_string()), "Обновлено: 2");
    assert_eq!(
        summary(2, &failed, None, ticketboard::intraservice::brief),
        "Обновлено: 2, не удалось: 2. Первая ошибка — #199: заявка не найдена (HTTP 404) — {\"Message\":\"нет\"}",
        "в вопросе — одной строкой (Brief)"
    );
    assert_eq!(
        summary(0, &[], Some("сервер недоступен:\nнет сети"), |e| e.to_string()),
        "Обновлено: 0\n\nСправочник статусов не получен — закрытые определены только по списку:\nсервер недоступен:\nнет сети"
    );
}

#[test]
fn closed_ones_the_question_keep_and_move() {
    let now = at("2026-10-10T12:00:00+03:00");
    let closed = ClosedNames::new(&["Закрыта".to_string(), "Выполнена".to_string()]);
    let with = |id: i32, status: Status, external: &str, kept: Option<&str>| {
        let mut t = card(id, &format!("заявка {id}"), status);
        t.external_status = Some(external.into());
        t.kept_open_status = kept.map(String::from);
        t
    };
    let mut board = Board::new(vec![
        with(101, Status::Inbox, "Закрыта", None),
        with(102, Status::InProgress, "В работе", None),
        with(103, Status::Waiting, "Закрыта", Some("Закрыта")),
        with(104, Status::Waiting, "Выполнена", Some("Закрыта")),
        with(105, Status::Done, "Закрыта", None),
    ]);
    let ids: std::collections::HashMap<i32, String> = board.all().map(|t| (t.intraservice_id.unwrap(), t.id.clone())).collect();
    let key = |n: i32| ids[&n].clone();
    let fresh: Vec<String> = [101, 102, 103, 104, 105].map(key).to_vec();
    // ответы приходят в каком придётся порядке — список в вопросе всё равно в порядке доски
    let mut gone: Vec<String> = fresh.iter().rev().cloned().collect();
    gone.push("удалена, пока шли запросы".into());
    let to_move = closed_to_move(&board, &gone, &closed);
    assert_eq!(to_move, [key(101), key(104)], "закрыта, не в «Готово», не оставлена с этим статусом; удалённой нет");
    // вопрос: итог, потом список
    assert_eq!(
        move_question(&board, &to_move, "Обновлено: 5\n\n"),
        "Обновлено: 5\n\nЗакрыты в Интрасервисе (2):\n#101  заявка 101\n#104  заявка 104"
    );
    let many: Vec<String> = (0..12).map(|_| key(101)).collect();
    assert!(move_question(&board, &many, "").ends_with("#101  заявка 101\n…и ещё 2"), "до десяти и сколько ещё");
    let ten = move_question(&board, &many[..10], "");
    assert!(ten.ends_with("#101  заявка 101") && ten.lines().count() == 11, "ровно десять — без «…и ещё»: {ten}");
    // «Оставить» — помнит статус; F5 снова — без вопроса; ещё раз «Оставить» — ничего не меняет
    assert!(keep_open(&mut board, &to_move));
    assert!(closed_to_move(&board, &fresh, &closed).is_empty());
    assert!(!keep_open(&mut board, &to_move));
    // закрыли иначе — спрашивает снова; «Перенести» — наверх «Готово», выполнена — сейчас
    board.find_mut(&key(101)).unwrap().external_status = Some("Выполнена".into());
    let again = closed_to_move(&board, &fresh, &closed);
    assert_eq!(again, [key(101)]);
    let mut with_gone = again.clone();
    with_gone.push("удалена, пока висел вопрос".into());
    assert_eq!(move_closed(&mut board, &with_gone, now), 1);
    let t = board.find(&key(101)).unwrap();
    assert_eq!((t.status, t.completed_at), (Status::Done, Some(now)));
    assert_eq!(numbers(&board, Status::Done), [101, 105], "наверх «Готово»");
}
