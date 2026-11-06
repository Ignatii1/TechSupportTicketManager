//! Общее для проверок: поддельный сервер Интрасервиса на loopback.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Живая карточка (2026-10-12, обезличена) — тот же образец, что в `HttpIntraserviceClient.SelfCheck`.
pub const LIVE: &str = r#"{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"Categories":null,"CategoryIds":null,"Changed":"2026-08-27T15:54:43.460865","Created":"2026-08-07T10:15:08.408534","Creator":"Петрова Анна","Description":"строка 1\r\nстрока 2","ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"11, 12","Executors":"Иванов И. И., Сидоров С. С.","Id":700001,"Name":"Склад: удалить задания","ResolutionDateFact":"2026-08-24T17:07:00","ServiceCode":"005","ServiceId":844,"ServiceName":"Приложение на ТСД","ServicePath":"840|844|","StatusId":28,"StatusIsFinal":true,"StatusName":"Закрыта","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}"#;

pub fn base64_of(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// Поддельный сервер: на каждый запрос — ответ `answer`, заголовок запроса записывается. `delay` — молчать перед ответом.
pub fn serve(code: u16, content_type: &str, body: &str, delay: Duration) -> (String, Arc<Mutex<Vec<String>>>) {
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
