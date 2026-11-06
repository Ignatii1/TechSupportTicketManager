//! Клиент Интрасервиса против поддельного сервера на loopback: запрос целиком, разбор живого ответа, ошибки — фразами
//! C#-клиента (`HttpIntraserviceClient.GetAsync` / `RefusalAsync`).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use ticketboard::intraservice::*;

/// Живая карточка (2026-10-12, обезличена) — тот же образец, что в `HttpIntraserviceClient.SelfCheck`.
const LIVE: &str = r#"{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"Categories":null,"CategoryIds":null,"Changed":"2026-08-27T15:54:43.460865","Created":"2026-08-07T10:15:08.408534","Creator":"Петрова Анна","Description":"строка 1\r\nстрока 2","ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"11, 12","Executors":"Иванов И. И., Сидоров С. С.","Id":700001,"Name":"Склад: удалить задания","ResolutionDateFact":"2026-08-24T17:07:00","ServiceCode":"005","ServiceId":844,"ServiceName":"Приложение на ТСД","ServicePath":"840|844|","StatusId":28,"StatusIsFinal":true,"StatusName":"Закрыта","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}"#;

/// Поддельный сервер: на каждый запрос — ответ `answer`, заголовок запроса записывается. `delay` — молчать перед ответом.
fn serve(code: u16, content_type: &str, body: &str, delay: Duration) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (seen2, ct, body) = (seen.clone(), content_type.to_string(), body.to_string());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                head.push(byte[0]);
            }
            seen2.lock().unwrap().push(String::from_utf8_lossy(&head).into_owned());
            thread::sleep(delay);
            let reason = match code {
                200 => "OK",
                302 => "Found",
                401 => "Unauthorized",
                404 => "Not Found",
                _ => "Error",
            };
            let extra = if code == 302 { "Location: /Account/Login\r\n" } else { "" };
            let _ = write!(
                stream,
                "HTTP/1.1 {code} {reason}\r\nContent-Type: {ct}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, seen)
}

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

fn base64_of(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}
