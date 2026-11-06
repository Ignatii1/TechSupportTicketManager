//! Тексты ошибок клиента — образцы `HttpIntraserviceClient.SelfCheck` (`Evidence`, `Brief`, `HttpCode`, `IsTransient`,
//! `TryEncoding`) и разбор тела по кодировке, как `ReaderOf`.

use ticketboard::intraservice::charset::{decode, known};
use ticketboard::intraservice::errors::*;

#[test]
fn evidence_json_as_is_and_pages_as_text() {
    // сырой ответ в сообщении: json — как есть; html — видимым текстом, без стилей; длинное — обрезано
    assert_eq!(evidence(r#" {"Message":"The request is invalid."} "#), r#"{"Message":"The request is invalid."}"#);
    assert_eq!(
        evidence(
            "<html><head><title>401 - Unauthorized</title><style>body{color:red}</style></head><body><p>Access denied</p></body></html>"
        ),
        "(html-страница, показан её текст)\n401 - Unauthorized\nAccess denied"
    );
    // страница IIS (DOCTYPE, комментарий в стиле, fieldset, переводы строк исходника): заголовок текст уже содержит — не дважды
    let iis = evidence(concat!(
        "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Strict//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\">\r\n",
        "<html xmlns=\"http://www.w3.org/1999/xhtml\">\r\n<head>\r\n<title>401 - Unauthorized</title>\r\n<style type=\"text/css\">\r\n<!--\r\n",
        "body{margin:0}\r\n-->\r\n</style>\r\n</head>\r\n<body>\r\n<div id=\"header\"><h1>Server Error</h1></div>\r\n<div id=\"content\">\r\n",
        " <div class=\"content-container\"><fieldset>\r\n  <h2>401 - Unauthorized: Access is denied.</h2>\r\n </fieldset></div>\r\n</div>\r\n</body>\r\n</html>\r\n"
    ));
    assert_eq!(iis, "(html-страница, показан её текст)\nServer Error\n401 - Unauthorized: Access is denied.");
    // страница входа (форма) — текстом
    let login = evidence(concat!(
        "<html><head><title>Вход</title></head><body><form method=\"post\" action=\"/login\"><label>Логин</label> <input name=\"user\"> ",
        "<button>Войти</button></form></body></html>"
    ));
    assert_eq!(login, "(html-страница, показан её текст)\nВход\nЛогин Войти");
    let trace = evidence(concat!(
        "<html><head><title>Runtime Error</title></head><body><h2>Server Error</h2><pre>System.Exception: boom\r\n   at A.B()\r\n",
        "   at C.D()</pre></body></html>"
    ));
    assert_eq!(trace, "(html-страница, показан её текст)\nRuntime Error\nServer Error\nSystem.Exception: boom\nat A.B()\nat C.D()");
    // заголовок опускается, только если с него начинается одна из первых строк: «Входящие» — не «Вход», глубже — не в счёт
    assert_eq!(
        evidence("<html><head><title>Вход</title></head><body><p>Входящие недоступны</p></body></html>"),
        "(html-страница, показан её текст)\nВход\nВходящие недоступны"
    );
    assert_eq!(
        evidence("<html><head><title>Доступ запрещён</title></head><body><p>a</p><p>b</p><p>c</p><p>Доступ запрещён</p></body></html>"),
        "(html-страница, показан её текст)\nДоступ запрещён\na\nb\nc\nДоступ запрещён"
    );
    // заголовок в комментарии — не заголовок
    assert_eq!(
        evidence(
            "<html><!-- <title>Old portal</title> --><head><title>401 - Unauthorized</title></head><body><p>Access denied</p></body></html>"
        ),
        "(html-страница, показан её текст)\n401 - Unauthorized\nAccess denied"
    );
    // страница из одного заголовка — он и есть текст
    assert_eq!(evidence("<html><head><title>Ошибка</title></head></html>"), "(html-страница, показан её текст)\nОшибка");
    assert_eq!(evidence("Authorization: Basic aXZhbm92Om15aXZhbm92MjM="), "Authorization: Basic ***");
    assert_eq!(evidence("<Task><Id>1</Id></Task>"), "<Task><Id>1</Id></Task>", "xml — не html, как есть");
    assert_eq!(evidence(&"x".repeat(5000)).chars().count(), SHOWN_CHARS + 2);
    assert_eq!(evidence(&"x".repeat(5000)), "x".repeat(SHOWN_CHARS) + "\n…");
    assert_eq!(evidence(&"ж".repeat(SHOWN_CHARS)), "ж".repeat(SHOWN_CHARS), "ровно предел — целиком");
}

#[test]
fn short_forms() {
    assert_eq!(brief("сервер недоступен:\nNo such host is known\nещё"), "сервер недоступен — No such host is known");
    assert_eq!(brief("нет доступа (HTTP 403)"), "нет доступа (HTTP 403)");
    assert_eq!(brief(""), "");
    assert_eq!(brief(" \n  первая:: \n\n  вторая  "), "первая — вторая", "пустые строки — не в счёт, «:» на конце первой — долой");
    assert_eq!(headline("ошибка сервера (HTTP 500):\n{}"), "ошибка сервера (HTTP 500)");
    assert_eq!(headline("  сервер недоступен:: "), "сервер недоступен");
    assert_eq!(auto_title(702180), "Заявка #702180");
}

#[test]
fn transient_or_not() {
    // код — из первой строки: в теле ответа сервера на нижних строках может быть что угодно
    assert_eq!(http_code("ошибка сервера (HTTP 503):\n{\"Message\":\"(HTTP 404)\"}"), Some(503));
    assert_eq!(http_code("сервер не ответил за 10 с"), None);
    assert_eq!(http_code("x (HTTP 50):\n"), None, "три цифры");
    assert_eq!(http_code("x (HTTP 5000)"), None, "ровно три, за ними «)»");
    assert_eq!(http_code("обрезано (HTTP 50"), None, "строка кончилась раньше кода");
    assert_eq!(http_code("(HTTP abc) потом (HTTP 503)"), Some(503), "первое подходящее");
    assert!(
        is_transient("сервер не ответил за 60 с")
            && is_transient("сервер недоступен:\nNo such host")
            && is_transient("соединение оборвалось:\nreset")
    );
    assert!(
        is_transient("ошибка сервера (HTTP 500)")
            && is_transient("ошибка сервера (HTTP 503):\n<html>")
            && is_transient("ошибка сервера (HTTP 429)")
    );
    assert!(is_transient("ошибка сервера (HTTP 408)"));
    assert!(
        !is_transient("неверный логин или пароль (HTTP 401)")
            && !is_transient("нет доступа (HTTP 403)")
            && !is_transient("заявка не найдена (HTTP 404)")
    );
    assert!(!is_transient("ошибка сервера (HTTP 400):\n{}") && !is_transient("непонятный ответ сервера:\n(HTTP 500)") && !is_transient(""));
}

#[test]
fn encodings() {
    // кодировка из заголовка: в кавычках — тоже; utf-7 и незнакомое имя — нет (тогда UTF-8)
    assert!(known(Some("\"windows-1251\"")) && known(Some("UTF-8")) && known(Some("koi8-r")) && known(Some("utf-16")));
    assert!(!known(Some("utf-7")) && !known(Some("x-no-such")) && !known(None) && !known(Some("")));
    let cp1251: Vec<u8> =
        vec![0xc4, 0xee, 0xf1, 0xf2, 0xf3, 0xef, 0x20, 0xe7, 0xe0, 0xef, 0xf0, 0xe5, 0xf9, 0xb8, 0xed, 0x20, 0xb9, 0x35, 0x20, 0x96, 0x98];
    assert_eq!(decode(&cp1251, Some("windows-1251")), format!("Доступ запрещён №5 –{}", char::from_u32(0x98).unwrap()));
    assert_eq!(decode(&cp1251, Some("CP1251")), decode(&cp1251, Some("windows-1251")));
    // KOI8-R: «Привет» и рамка
    assert_eq!(decode(&[0xf0, 0xd2, 0xc9, 0xd7, 0xc5, 0xd4, 0x20, 0x80, 0xa3, 0xb3], Some("koi8-r")), "Привет ─ёЁ");
    assert_eq!(decode(&[0x41, 0xe9, 0xff], Some("iso-8859-1")), "Aéÿ");
    // BOM главнее заголовка
    assert_eq!(decode(&[0xef, 0xbb, 0xbf, 0xd0, 0xaf], Some("windows-1251")), "Я");
    assert_eq!(decode(&[0xff, 0xfe, 0x2f, 0x04, 0x41, 0x00], None), "ЯA");
    assert_eq!(decode(&[0xfe, 0xff, 0x04, 0x2f, 0x00, 0x41], None), "ЯA");
    assert_eq!(decode(&[0xff, 0xfe, 0x00, 0x00, 0x2f, 0x04, 0x00, 0x00], None), "Я", "UTF-32 LE");
    assert_eq!(decode(&[0x00, 0x00, 0xfe, 0xff, 0x00, 0x00, 0x04, 0x2f], None), "Я", "UTF-32 BE");
    assert_eq!(decode(&[0x2f, 0x04], Some("utf-16")), "Я", "utf-16 без BOM — младшим байтом вперёд");
    // без BOM — по заголовку: старшим байтом вперёд, UTF-32 обоих порядков
    assert_eq!(decode(&[0x04, 0x2f], Some("utf-16be")), "Я");
    assert_eq!(decode(&[0x2f, 0x04, 0x00, 0x00], Some("utf-32")), "Я");
    assert_eq!(decode(&[0x00, 0x00, 0x04, 0x2f], Some("UTF-32BE")), "Я");
    // оборванный хвост — знак замены, как у .NET; целый — без него; суррогат без пары и не символ — тоже знак замены
    let r = char::REPLACEMENT_CHARACTER;
    assert_eq!(decode(&[0x2f, 0x04, 0x41], Some("utf-16")), format!("Я{r}"));
    assert_eq!(decode(&[0x2f, 0x04, 0x00, 0x00, 0x41], Some("utf-32")), format!("Я{r}"));
    assert_eq!(decode(&[0x00, 0xd8, 0x41, 0x00], Some("utf-16")), format!("{r}A"));
    assert_eq!(decode(&[0x00, 0x00, 0x11, 0x00], Some("utf-32")), r.to_string());
    // нет заголовка, незнакомая или нечитаемая кодировка — UTF-8, битые байты — знаком замены
    assert_eq!(decode("Я".as_bytes(), None), "Я");
    assert_eq!(decode(&[0x41, 0xff, 0x42], Some("utf-7")), format!("A{}B", char::REPLACEMENT_CHARACTER));
}
