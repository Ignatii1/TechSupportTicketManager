//! Ссылка и номер заявки в произвольном тексте — образцы `IntraserviceLinkParser.SelfCheck` один в один и шаблон из настроек.

use ticketboard::link::{DEFAULT_ID_PATTERN, LinkParser};

#[test]
fn with_the_default_pattern() {
    let p = LinkParser::new(DEFAULT_ID_PATTERN);
    // голый номер: ссылки нет, номер есть
    let l = p.parse("702180");
    assert!(!l.has_url() && l.url.is_empty() && l.id == Some(702180));
    assert_eq!(p.parse("  702180  ").id, Some(702180));
    // номер с решёткой (и со знаком номера) и с текстом
    assert_eq!(p.parse("#702180").id, Some(702180));
    assert_eq!(p.parse("#702180 не печатает").id, Some(702180));
    assert_eq!(p.parse("№ 702180").id, Some(702180));
    // ссылка
    let l = p.parse("https://help.local/Task/View/702180");
    assert!(l.has_url() && l.url == "https://help.local/Task/View/702180" && l.id == Some(702180));
    // число внутри текста номером не считается, короткое — тоже
    assert_eq!(p.parse("заменить картридж 12345").id, None);
    assert_eq!(p.parse("123").id, None);
}

#[test]
fn links_in_text() {
    let p = LinkParser::new(DEFAULT_ID_PATTERN);
    // ссылка посреди письма: знаки препинания на конце — не её часть; регистр схемы и пути — любой
    let l = p.parse("Посмотрите (HTTPS://help.local/task/view/702181), пожалуйста.");
    assert_eq!((l.url.as_str(), l.id), ("HTTPS://help.local/task/view/702181", Some(702181)));
    // в ссылке без «Task/View/» — последнее число из 4–8 цифр в ней (а у длинного — его последние 8)
    assert_eq!(p.parse("http://x.local/a/1234/b/702182?tab=2").id, None, "после номера ещё цифра — это не он");
    assert_eq!(p.parse("http://x.local/a/1234/b/702182").id, Some(702182));
    assert_eq!(p.parse("http://x.local/q?id=123456789").id, Some(23456789));
    assert_eq!(p.parse("http://x.local/q?id=12").id, None);
    // «http://» без адреса — не ссылка; дальше в тексте — ссылка
    let l = p.parse("http:// и https://help.local/Task/View/702183");
    assert_eq!((l.url.as_str(), l.id), ("https://help.local/Task/View/702183", Some(702183)));
    assert!(!p.parse("").has_url() && p.parse("").id.is_none());
    // номер по шаблону главнее последнего числа ссылки
    assert_eq!(p.parse("https://help.local/Task/View/702187?tab=1234").id, Some(702187));
    // «http» без «://» — не ссылка (httpd.conf), ищется дальше
    let l = p.parse("смотри httpd.conf, потом https://help.local/Task/View/702186");
    assert_eq!((l.url.as_str(), l.id), ("https://help.local/Task/View/702186", Some(702186)));
}

#[test]
fn a_pattern_from_the_settings() {
    // свой шаблон — номер из его первой группы; группа не число — номера нет
    let p = LinkParser::new(r"заявка\s+(\d+)");
    assert_eq!(p.parse("ЗАЯВКА 42 сломалась").id, Some(42), "без учёта регистра и для кириллицы — как IgnoreCase");
    assert_eq!(LinkParser::new(r"ЗАЯВКА\s+(\d+)").parse("заявка 43").id, Some(43));
    assert_eq!(LinkParser::new(r"\Bа(\d+)").parse("ЖА44").id, Some(44), "экранированное — как было: \\B не \\b");
    assert_eq!(LinkParser::new(r"id=(\w+)").parse("id=abc").id, None);
    assert_eq!(LinkParser::new(r"id=(\S+)").parse("id=-7").id, Some(-7), "знак — как int.TryParse");
    assert_eq!(LinkParser::new(r"id=(\S+)").parse("id=+7").id, Some(7));
    assert_eq!(LinkParser::new(r"id=(\S+)").parse("id=-+7").id, None, "знак — один");
    assert_eq!(LinkParser::new(r"id=(.+)$").parse("id= 7 ").id, Some(7), "пробелы по краям — можно");
    // кривой шаблон — ссылку всё равно берём, номер — последний в ссылке
    let broken = LinkParser::new(r"(?<=View/)(\d+");
    let l = broken.parse("https://help.local/Task/View/702184");
    assert_eq!((l.url.as_str(), l.id), ("https://help.local/Task/View/702184", Some(702184)));
    assert_eq!(broken.parse("702185").id, Some(702185), "голый номер — без шаблона");
    assert_eq!(LinkParser::new("").parse("текст 1234").id, None);
}
