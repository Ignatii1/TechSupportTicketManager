//! Что ⟳ меняет в карточке (`MainViewModel.Apply`/`ApplyCard`) — случаи `CardCheck` (раздел 3).

use chrono::{DateTime, FixedOffset};
use ticketboard::board::{Ticket, TicketFile};
use ticketboard::intraservice::parse;
use ticketboard::sync::{apply, apply_card};

fn at(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

fn card(fields: &str) -> ticketboard::intraservice::Task {
    parse::task(&format!(r#"{{"Task":{{"Id":70114,"Name":"Нет доступа к 1С","StatusName":"В работе"{fields}}}}}"#), 70114).unwrap()
}

fn ticket(title: &str) -> Ticket {
    let mut t = Ticket::new(at("2026-10-01T09:00:00+03:00"));
    t.title = title.into();
    t.intraservice_id = Some(70114);
    t
}

#[test]
fn title_description_and_status() {
    let now = at("2026-10-10T12:00:00+03:00");
    // «Заявка #N» — название с сервера; своё — остаётся
    let mut t = ticket("Заявка #70114");
    apply(&mut t, 70114, &card(r#","Description":"Пишет «нет лицензии»""#), now);
    assert_eq!((t.title.as_str(), t.description.as_str()), ("Нет доступа к 1С", "Пишет «нет лицензии»"), "пустое описание — с сервера");
    assert_eq!((t.external_status.as_deref(), t.last_sync_at), (Some("В работе"), Some(now)));
    let mut t = ticket("своё название");
    t.description = "Своими словами: 1С на складе не открывается".into();
    apply(&mut t, 70114, &card(r#","Description":"Пишет «нет лицензии»""#), now);
    assert_eq!(t.title, "своё название");
    assert_eq!(t.description, "Своими словами: 1С на складе не открывается", "написанное на карточке — не затирается");
    // описание из одних пробелов — как пустое; без описания на сервере — ничего
    let mut t = ticket("x");
    t.description = "  \n ".into();
    apply(&mut t, 70114, &card(r#","Description":"с сервера""#), now);
    assert_eq!(t.description, "с сервера");
    let mut t = ticket("x");
    apply(&mut t, 70114, &card(""), now);
    assert_eq!(t.description, "");
    // имени нет — «Заявка #N» остаётся
    let mut t = ticket("Заявка #70114");
    let mut x = card("");
    x.name.clear();
    apply(&mut t, 70114, &x, now);
    assert_eq!(t.title, "Заявка #70114");
}

#[test]
fn a_description_cut_by_the_old_parser_is_repaired() {
    let now = at("2026-10-10T12:00:00+03:00");
    // разбор до 0.15.1 принимал «< 5 шт, заказ >» за тег: на карточке ровно его обрезок — руками не правили
    let mut t = ticket("x");
    t.description = "остаток  10 шт".into();
    apply(&mut t, 70114, &card(r#","Description":"остаток < 5 шт, заказ > 10 шт""#), now);
    assert_eq!(t.description, "остаток < 5 шт, заказ > 10 шт");
    // и html: сверяется с тем, что прежний разбор сделал из текста сервера
    let mut t = ticket("x");
    t.description = "Заказостаток  10".into();
    apply(&mut t, 70114, &card(r#","Description":"<h1>Заказ</h1><p>остаток < 5 шт, заказ > 10</p>""#), now);
    assert_eq!(t.description, "Заказ\nостаток < 5 шт, заказ > 10");
    // правленое руками (не обрезок) — не трогается
    let mut t = ticket("x");
    t.description = "остаток 10 шт, проверил".into();
    apply(&mut t, 70114, &card(r#","Description":"остаток < 5 шт, заказ > 10 шт""#), now);
    assert_eq!(t.description, "остаток 10 шт, проверил");
}

#[test]
fn people_card_fields_and_files() {
    let full = r#","Creator":"Петрова А.","CreatorPhone":"+7 900","CreatorEmail":"p@example.ru","CreatorPosition":"Бухгалтер",
        "Executors":"Я Сам","ExecutorGroup":"ТП","CreatorCompanyName":"ООО Ромашка","ServiceName":"Почта","Type":"Инцидент",
        "Categories":"Печать","Assets":"HP","FileIds":"1, 2","FileNames":"a.png||b.pdf""#;
    let mut t = ticket("x");
    apply_card(&mut t, &card(full));
    assert_eq!(
        [&t.creator, &t.creator_phone, &t.creator_email, &t.creator_position, &t.executors, &t.executor_group].map(|v| v.as_deref()),
        [Some("Петрова А."), Some("+7 900"), Some("p@example.ru"), Some("Бухгалтер"), Some("Я Сам"), Some("ТП")]
    );
    assert_eq!(
        [&t.creator_company, &t.service, &t.task_type, &t.categories, &t.assets].map(|v| v.as_deref()),
        [Some("ООО Ромашка"), Some("Почта"), Some("Инцидент"), Some("Печать"), Some("HP")]
    );
    let names = |t: &Ticket| t.files.as_ref().map(|f| f.iter().map(|f| (f.id, f.name.clone())).collect::<Vec<_>>());
    assert_eq!(names(&t), Some(vec![(1, Some("a.png".into())), (2, Some("b.pdf".into()))]));
    // поля нет в ответе — остаётся, что было; список вложений — целиком новый, когда другой
    t.files.as_mut().unwrap()[0].other.insert("Seen".into(), true.into());
    apply_card(&mut t, &card(r#","FileIds":"1, 2","FileNames":"a.png||b.pdf""#));
    assert_eq!(t.creator.as_deref(), Some("Петрова А."), "нет в ответе — осталось");
    assert_eq!(t.service.as_deref(), Some("Почта"));
    assert_eq!(t.files.as_ref().unwrap()[0].other.get("Seen"), Some(&true.into()), "тот же список — не переписан");
    apply_card(&mut t, &card(r#","FileIds":"2","FileNames":"b.pdf""#));
    assert_eq!(names(&t), Some(vec![(2, Some("b.pdf".into()))]), "вложение удалили на сервере");
    apply_card(&mut t, &card(""));
    assert_eq!(names(&t), Some(vec![(2, Some("b.pdf".into()))]), "FileIds нет — список не трогается");
    // строка списка (без полей карточки) — поля карточки и вложения не стираются
    let mut row = card(r#","Creator":"Иванов И.""#);
    row.extra = None;
    apply_card(&mut t, &row);
    assert_eq!((t.creator.as_deref(), t.service.as_deref()), (Some("Иванов И."), Some("Почта")));
    assert!(t.files.is_some());
    let _ = TicketFile { id: 0, name: None, other: Default::default() };
}
