//! Быстрое добавление — случаи `CardCheck` (разделы 1 и 2): что видно в окне, что спросить у сервера и какая карточка
//! получится.

use chrono::{DateTime, FixedOffset};
use ticketboard::board::{Priority, Status};
use ticketboard::capture::*;
use ticketboard::link::{DEFAULT_ID_PATTERN, LinkParser};
use ticketboard::settings::Settings;

fn at(s: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(s).unwrap()
}

fn settings() -> Settings {
    Settings { base_url: "https://hd/".into(), ..Settings::default() }
}

#[test]
fn a_card_from_what_was_typed() {
    let p = LinkParser::new(DEFAULT_ID_PATTERN);
    let now = at("2026-10-10T12:00:00+03:00");
    let add = |input: &str, priority| add_from_capture(input, priority, &p, &settings(), now);
    let c1 = add("https://hd/Task/View/70112 — не печатает на 3 этаже", Priority::High);
    assert_eq!(
        (c1.intraservice_id, c1.title.as_str(), c1.url.as_str(), c1.priority, c1.status),
        (Some(70112), "не печатает на 3 этаже", "https://hd/Task/View/70112", Priority::High, Status::Inbox),
        "ссылка с текстом: номер и ссылка — из ссылки, название — из текста"
    );
    assert_eq!((c1.created_at, c1.status_changed_at), (now, now));
    let c2 = add("70114", Priority::Mid);
    assert_eq!((c2.intraservice_id, c2.title.as_str(), c2.url.as_str()), (Some(70114), "Заявка #70114", "https://hd/Task/View/70114"));
    let c3 = add("Купить картридж", Priority::Low);
    assert_eq!((c3.intraservice_id, c3.title.as_str(), c3.url.as_str()), (None, "Купить картридж", ""), "без номера — без ссылки");
    let c4 = add("№ 70115", Priority::Mid);
    assert_eq!((c4.intraservice_id, c4.title.as_str()), (Some(70115), "Заявка #70115"), "«№ 70115» — только номер");
    let c5 = add("#70116", Priority::Mid);
    assert_eq!(c5.title, "Заявка #70116");
    let c6 = add("https://hd/Task/View/70117", Priority::Mid);
    assert_eq!((c6.title.as_str(), c6.url.as_str()), ("Заявка #70117", "https://hd/Task/View/70117"));
    let c7 = add("https://example.ru/page", Priority::Mid);
    assert_eq!((c7.intraservice_id, c7.title.as_str()), (None, "Заявка"), "ссылка без номера и без текста");
    // адреса Интрасервиса в настройках нет — голому номеру ссылку не собрать
    let bare = add_from_capture("70118", Priority::Mid, &p, &Settings::default(), now);
    assert_eq!((bare.intraservice_id, bare.url.as_str()), (Some(70118), ""));
    // текст с номером внутри — название целиком, номер — из ссылки нет, из текста нет
    let text = add("заменить картридж 12345", Priority::Mid);
    assert_eq!((text.intraservice_id, text.title.as_str()), (None, "заменить картридж 12345"));
}

#[test]
fn what_the_window_shows_and_asks() {
    let p = LinkParser::new(DEFAULT_ID_PATTERN);
    let mut c = Capture::default();
    assert_eq!((c.hint.as_str(), c.preview.as_str(), c.priority), (EMPTY_HINT, "", Priority::Mid), "с самого начала");
    // набирают номер: пока цифр меньше четырёх — номера нет; дальше каждый новый номер — свой запрос, прежний не в счёт
    assert_eq!(c.set_text("70", &p, true), None);
    assert_eq!((c.number_text(), c.hint.as_str()), (String::new(), TYPED_HINT));
    assert_eq!(c.set_text("701", &p, true), None);
    let first = c.set_text("7011", &p, true).expect("номер — спросить");
    assert_eq!(first.id, 7011);
    let last = c.set_text("70112", &p, true).expect("другой номер — спросить");
    assert_eq!((last.id, c.number_text(), c.preview.as_str()), (70112, "#70112".to_string(), LOOKING));
    assert!(!c.current(first.generation) && c.current(last.generation), "прежний запрос не в счёт");
    assert!(!c.digits_set_priority(), "номер, не ссылка: цифры — текст");
    c.answer(first.generation, Ok("чужое".into()));
    assert_eq!(c.preview, LOOKING, "ответ на прежний ввод — мимо");
    c.answer(last.generation, Ok("Принтер не печатает".into()));
    assert_eq!(c.preview, "Принтер не печатает");
    // тот же номер — второй раз не спрашиваем
    assert_eq!(c.set_text("70112 ", &p, true), None);
    assert_eq!(c.preview, "Принтер не печатает");
    // ссылка — цифры 1/2/3 меняют приоритет; нет такой заявки — одной строкой
    let link = c.set_text("https://hd/Task/View/70113", &p, true).unwrap();
    assert!(c.digits_set_priority());
    assert_eq!(c.number_text(), "#70113");
    c.answer(link.generation, Err("заявка не найдена (HTTP 404):\n{\"Message\":\"нет\"}".into()));
    assert!(c.preview.starts_with("заявка не найдена") && !c.preview.contains('\n'), "{}", c.preview);
    // сброс: пусто, «средний», подсказка
    c.priority = Priority::High;
    c.reset(&p);
    assert_eq!((c.text.as_str(), c.priority, c.number, c.preview.as_str(), c.hint.as_str()), ("", Priority::Mid, None, "", EMPTY_HINT));
    assert!(!c.digits_set_priority());
    // без API — не спрашиваем; API настроили (`forget_lookup`) — тот же номер спрашиваем заново
    let mut off = Capture::default();
    assert_eq!(off.set_text("70112", &p, false), None);
    assert_eq!(off.preview, "");
    assert_eq!(off.set_text("70112 ", &p, true), None, "тот же номер");
    off.forget_lookup();
    assert!(off.set_text("70112", &p, true).is_some(), "после смены настроек — заново");
    assert_eq!(PAUSE.as_millis(), 400);
}
