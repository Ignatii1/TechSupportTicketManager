//! Клиент Интрасервиса против поддельного сервера на loopback: запрос целиком, разбор живого ответа, ошибки — фразами
//! C#-клиента (`HttpIntraserviceClient.GetAsync` / `RefusalAsync`).

mod common;

use common::{LIVE, base64_of, serve};
use std::net::TcpListener;
use std::time::Duration;
use ticketboard::intraservice::*;

fn client(url: &str) -> Client {
    Client::new(url, "me", "p@ss:ворд", Duration::from_secs(10))
}

#[test]
fn the_request_and_the_live_card() {
    let (url, seen) = serve(200, "application/json; charset=utf-8", LIVE, Duration::ZERO);
    let task = client(&url).get_task(700001).expect("живая карточка читается");
    assert_eq!(
        task,
        Task {
            id: 700001,
            name: "Склад: удалить задания".into(),
            status: "Закрыта".into(),
            changed: Some("2026-08-27T15:54:43.460865".into()),
            creator: Some("Петрова Анна".into()),
            executors: Some("Иванов И. И., Сидоров С. С.".into()),
        }
    );
    let head = seen.lock().unwrap()[0].clone();
    assert!(head.starts_with("GET /api/task/700001?include=status HTTP/1.1\r\n"), "{head}");
    // Basic: base64 от «логин:пароль» в UTF-8, как у C#
    let basic = format!("authorization: Basic {}", base64_of("me:p@ss:ворд"));
    assert!(head.to_lowercase().contains(&basic.to_lowercase()), "{head}");
    assert!(head.to_lowercase().contains("accept: application/json"), "{head}");
}

#[test]
fn the_address_with_and_without_a_slash() {
    let (url, seen) = serve(200, "application/json", LIVE, Duration::ZERO);
    let bare = url.trim_end_matches('/');
    client(&format!("  {bare}//  ")).get_task(5).unwrap();
    assert!(seen.lock().unwrap()[0].starts_with("GET /api/task/5?include=status "), "{:?}", seen.lock().unwrap());
}

#[test]
fn parse_shapes() {
    // название по статусу из блока Statuses, когда своего StatusName нет; номера нет — из запроса; без названия — «Заявка #N»
    let t = parse_task(r#"{"Task":{"StatusId":31},"Statuses":[{"Id":31,"Name":"Открыта"}]}"#, 163).unwrap();
    assert_eq!((t.id, t.name.as_str(), t.status.as_str()), (163, "Заявка #163", "Открыта"));
    let t = parse_task(r#"{"Task":{"Name":"C","StatusId":56}}"#, 7).unwrap();
    assert_eq!(t.status, "статус 56");
    assert!(parse_task("[1, 2]", 7).is_none());
    assert!(parse_task("<html>", 7).is_none());
}

#[test]
fn bare_cards_and_a_big_body() {
    // карточка без обёртки Task — как она есть; Task не объект — тоже берётся корень
    let t = parse_task(r#"{"Id": 5, "Name": "голая", "StatusName": "Открыта"}"#, 1).unwrap();
    assert_eq!((t.id, t.name.as_str(), t.status.as_str()), (5, "голая", "Открыта"));
    let t = parse_task(r#"{"Task": null, "Id": 6, "Name": "корень"}"#, 1).unwrap();
    assert_eq!(t.name, "корень");
    // описание на 2 МБ (вставленный журнал) — читается целиком
    let big = format!(r#"{{"Task":{{"Id":7,"Name":"большая","StatusName":"Открыта","Description":"{}"}}}}"#, "x".repeat(2_000_000));
    let (url, _) = serve(200, "application/json", &big, Duration::ZERO);
    assert_eq!(client(&url).get_task(7).map(|t| t.name), Ok("большая".to_string()));
}

#[test]
fn credentials_echoed_by_the_server_are_masked() {
    // страница прокси или IIS повторяет заголовки запроса — в окне (а его просят переслать) логина и пароля быть не должно
    let token = base64_of("me:p@ss:ворд");
    let body = format!("Trace: Authorization: Basic {token}\nauthorization: basic   {token}\nпароль p@ss:ворд");
    let (url, _) = serve(500, "text/plain", &body, Duration::ZERO);
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.starts_with("ошибка сервера (HTTP 500)"), "{err}");
    assert!(!err.contains(&token) && !err.contains("p@ss:ворд"), "{err}");
    assert!(err.contains("Basic ***") && err.contains("basic   ***"), "{err}");
    // ответ, который не разобран, — тоже
    let (url, _) = serve(200, "text/html", &format!("<pre>Basic {token}</pre>"), Duration::ZERO);
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.starts_with("ответ не разобран") && !err.contains(&token), "{err}");
}

#[test]
fn a_broken_address_is_named() {
    let err = client("https://hd example ru").get_task(5).unwrap_err();
    assert!(err.starts_with("адрес Интрасервиса в настройках не годится"), "{err}");
}

#[test]
fn refusals_in_csharp_words() {
    for (code, first) in [
        (401, "неверный логин или пароль (HTTP 401)"),
        (403, "нет доступа (HTTP 403)"),
        (404, "заявка не найдена (HTTP 404)"),
        (500, "ошибка сервера (HTTP 500)"),
    ] {
        let (url, _) = serve(code, "application/json", r#"{"Message":"подробности"}"#, Duration::ZERO);
        let err = client(&url).get_task(5).unwrap_err();
        assert!(err.starts_with(first), "{code}: {err}");
        assert!(err.contains("подробности"), "тело ответа — на виду: {err}");
    }
    let (url, _) = serve(404, "application/json", "", Duration::ZERO);
    assert_eq!(client(&url).get_task(5).unwrap_err(), "заявка не найдена (HTTP 404)");
}

#[test]
fn a_redirect_is_not_followed() {
    // переадресация — обычно на страницу входа: не идём за ней (и не несём туда логин и пароль)
    let (url, seen) = serve(302, "text/html", "<html>вход</html>", Duration::ZERO);
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.contains("переадресация") && err.contains("/Account/Login"), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 1, "за переадресацией — ни шагу");
}

#[test]
fn a_page_instead_of_json() {
    let (url, _) = serve(200, "text/html", "<html><body>Вход в систему</body></html>", Duration::ZERO);
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.starts_with("ответ не разобран") && err.contains("Вход в систему"), "{err}");
}

#[test]
fn no_server_and_a_silent_one() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(); // порт закрыт: слушатель ушёл
    let err = client(&format!("http://127.0.0.1:{port}")).get_task(5).unwrap_err();
    assert!(err.starts_with("сервер недоступен"), "{err}");
    let (url, _) = serve(200, "application/json", LIVE, Duration::from_secs(3));
    let err = Client::new(&url, "me", "p", Duration::from_secs(1)).get_task(5).unwrap_err();
    assert_eq!(err, "сервер не ответил за 1 с");
}

#[test]
fn system_proxy_is_described() {
    let text = system_proxy();
    assert!(!text.is_empty());
    #[cfg(not(windows))]
    assert!(text.contains("Windows"), "{text}");
}

#[test]
fn redact_edges() {
    // токен — после «basic» и пробелов (табуляции), от 6 знаков base64 (+ / = тоже); регистр — любой; всё вокруг — как было
    assert_eq!(redact("Authorization: Basic abcdef", &[]), "Authorization: Basic ***");
    assert_eq!(redact("x BASIC\tAb+/=12, дальше", &[]), "x BASIC\t***, дальше");
    assert_eq!(redact("Basic abcde", &[]), "Basic abcde", "5 знаков — не токен");
    assert_eq!(redact("Basicabcdefgh", &[]), "Basicabcdefgh", "без пробела — слово, не токен");
    assert_eq!(redact("basic basic Zm9vYmFy end", &[]), "basic basic *** end", "«basic» без токена пропускается, ищется дальше");
    assert_eq!(redact("до Basic Zm9vYmFy и Basic YmFyYmF6 после", &[]), "до Basic *** и Basic *** после");
    assert_eq!(redact("пароль 12345 и abc", &["12345", "abc"]), "пароль *** и abc", "секрет короче 4 знаков не прячем: он в любом тексте");
    assert_eq!(redact("Ёж basic Zm9vYmFy", &[]), "Ёж basic ***", "кириллица перед — смещения те же");
}
