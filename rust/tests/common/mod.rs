//! Общее для проверок: поддельный сервер Интрасервиса на loopback.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Живая карточка (2026-10-12, обезличена) — тот же образец, что в `HttpIntraserviceClient.SelfCheck`.
pub const LIVE: &str = r#"{"Priorities":null,"Rights":null,"TaskTypeSettings":null,"Services":null,"Statuses":null,"Task":{"Categories":null,"CategoryIds":null,"Changed":"2026-08-27T15:54:43.460865","Created":"2026-08-07T10:15:08.408534","Creator":"Петрова Анна","Description":"строка 1\r\nстрока 2","ExecutorGroup":null,"ExecutorGroupId":null,"ExecutorIds":"11, 12","Executors":"Иванов И. И., Сидоров С. С.","Id":700001,"Name":"Склад: удалить задания","ResolutionDateFact":"2026-08-24T17:07:00","ServiceCode":"005","ServiceId":844,"ServiceName":"Приложение на ТСД","ServicePath":"840|844|","StatusId":28,"StatusIsFinal":true,"StatusName":"Закрыта","Type":"Запрос на обслуживание","TypeId":1009},"TaskType":null,"Users":null}"#;

/// Живая история (2026-10-02, обезличена): две смены статуса и два комментария, свежие сверху.
pub const LIVE_LIFE: &str = r#"{"TaskLifetimes":[{"Comments":"Добрый день! \r\nда, свяжусь.","IsPublic":true,"Date":"2026-10-02T14:07:07.241319","EditorId":26512,"Editor":"Сидорова Мария Ивановна"},{"Comments":"<p>Не могли бы вы связаться с поставщиком?</p>","IsPublic":false,"Date":"2026-10-02T14:02:22.482802","EditorId":40800,"Editor":"Иванов Иван Иванович","StatusId":46},{"Date":"2026-10-02T14:02:02.137104","EditorId":33736,"Editor":"Кузнецова Ольга Петровна","StatusId":38},{"Date":"2026-10-02T14:00:48.041767","EditorId":40800,"Editor":"Иванов Иван Иванович","StatusId":31}],"Statuses":[{"Id":31,"Name":"Открыта"},{"Id":38,"Name":"Передана исполнителю"},{"Id":46,"Name":"Ожидание ответа пользователя"}],"Paginator":{"Count":4,"Page":1,"PageCount":1,"PageSize":50,"CountOnPage":4,"HasNextPage":null}}"#;

pub fn base64_of(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// Ответ поддельного сервера. `sent` — сколько байт тела отдать и замолчать (соединение висит открытым), None — всё тело.
pub struct Reply {
    pub code: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub delay: Duration,
    pub sent: Option<usize>,
    /// Отдав `sent` байт, закрыть соединение (обрыв), а не молчать.
    pub hang_up: bool,
    pub location: Option<String>,
}

impl Reply {
    pub fn new(code: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Reply {
        Reply {
            code,
            content_type: content_type.into(),
            body: body.into(),
            delay: Duration::ZERO,
            sent: None,
            hang_up: false,
            location: None,
        }
    }

    pub fn json(body: &str) -> Reply {
        Reply::new(200, "application/json; charset=utf-8", body)
    }
}

/// Поддельный сервер с маршрутами: `answer` получает путь запроса (с параметрами) и решает, что ответить. Заголовки
/// каждого запроса записываются по порядку.
pub fn route(answer: impl Fn(&str) -> Reply + Send + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                head.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            seen2.lock().unwrap().push(head.clone());
            let path = head.split(' ').nth(1).unwrap_or("").to_string();
            let reply = answer(&path);
            thread::sleep(reply.delay);
            let reason = match reply.code {
                200 => "OK",
                302 => "Found",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                _ => "Error",
            };
            let location = reply.location.as_deref().map(|l| format!("Location: {l}\r\n")).unwrap_or_default();
            let _ = write!(
                stream,
                "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
                reply.code,
                reply.content_type,
                reply.body.len()
            );
            match reply.sent {
                None => {
                    let _ = stream.write_all(&reply.body);
                }
                Some(n) if reply.hang_up => {
                    let _ = stream.write_all(&reply.body[..n]);
                    let _ = stream.flush();
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
                Some(n) => {
                    let _ = stream.write_all(&reply.body[..n]);
                    let _ = stream.flush();
                    // замолчало, не закрыв соединение: срок клиента истечёт раньше
                    thread::spawn(move || {
                        thread::sleep(Duration::from_secs(10));
                        drop(stream);
                    });
                }
            }
        }
    });
    (url, seen)
}

/// Поддельный сервер: на каждый запрос — один и тот же ответ. `delay` — молчать перед ответом.
pub fn serve(code: u16, content_type: &str, body: &str, delay: Duration) -> (String, Arc<Mutex<Vec<String>>>) {
    let (ct, body) = (content_type.to_string(), body.to_string());
    route(move |_| {
        let mut r = Reply::new(code, &ct, body.clone());
        r.delay = delay;
        if code == 302 {
            r.location = Some("/Account/Login".into());
        }
        r
    })
}

/// Образец `StoreCheck` (TicketBoard.SelfCheck/StoreCheck.cs): `tickets.json` в нынешних именах, все поля заполнены.
pub const STORE_SAMPLE: &str = r#"
[
  {
    "Id": "9b2f6c1e-0000-4000-8000-000000000001",
    "CreatedAt": "2026-09-01T10:00:00+03:00",
    "Title": "Принтер не печатает",
    "Url": "https://hd/Task/View/702180",
    "Description": "с утра",
    "Priority": "High",
    "Status": "Waiting",
    "StatusChangedAt": "2026-09-02T11:00:00+03:00",
    "CompletedAt": "2026-09-03T12:00:00+03:00",
    "IntraserviceId": 702180,
    "ExternalStatus": "Закрыта",
    "LastSyncAt": "2026-09-04T13:00:00+03:00",
    "KeptOpenStatus": "Закрыта",
    "Creator": "Петрова А.",
    "CreatorPhone": "+7 900 000-00-00",
    "CreatorEmail": "petrova@example.ru",
    "Executors": "Я Сам, Сидоров С.",
    "ExecutorGroup": "Вторая линия",
    "CreatorPosition": "Бухгалтер",
    "CreatorCompany": "ООО Ромашка",
    "Service": "Почта",
    "TaskType": "Инцидент",
    "Categories": "Печать, Картриджи",
    "Assets": "Принтер HP",
    "Files": [ { "Id": 900101, "Name": "экран.png" }, { "Id": 900102, "Name": null } ],
    "CommentsCheckedFor": "2026-09-05T14:00:00+03:00",
    "CommentsSeenAt": "2026-09-05T14:00:01+03:00",
    "UnreadComments": 3,
    "AssignedToMe": false,
    "Notes": [ { "Id": "9b2f6c1e-0000-4000-8000-000000000002", "CreatedAt": "2026-09-06T15:00:00+03:00", "Text": "позвонить" } ]
  }
]
"#;

/// Таблица сверки `tests/parity/<name>.tsv`: входы и ответ кода C# на них (последняя ячейка). Ответы пишет и сверяет с
/// самим кодом C# `TicketBoard.SelfCheck` (область `parity`), так что образец не расходится с приложением. Возвращает
/// строки таблицы — номер строки файла и ячейки уже без экранирования.
pub fn parity(name: &str) -> Vec<(usize, Vec<String>)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity").join(format!("{name}.tsv"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rows: Vec<_> = text
        .split('\n')
        .enumerate()
        .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
        .map(|(i, line)| (i + 1, line.split('\t').map(|cell| parity_cell(cell, name, i + 1)).collect()))
        .collect();
    assert!(!rows.is_empty(), "{name}.tsv: строк нет");
    rows
}

/// Ячейка таблицы сверки без экранирования: обратная черта и за ней `\\`, `t`, `n`, `r` или `u` с четырьмя цифрами.
fn parity_cell(cell: &str, name: &str, line: usize) -> String {
    let bad = || -> ! { panic!("{name}.tsv, строка {line}: ячейка не читается: «{cell}»") };
    let mut units: Vec<u16> = Vec::new();
    let mut chars = cell.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            units.extend(c.encode_utf16(&mut [0; 2]).iter());
            continue;
        }
        match chars.next() {
            Some('\\') => units.push('\\' as u16),
            Some('t') => units.push('\t' as u16),
            Some('n') => units.push('\n' as u16),
            Some('r') => units.push('\r' as u16),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u16::from_str_radix(&hex, 16) {
                    Ok(unit) if hex.len() == 4 => units.push(unit),
                    _ => bad(),
                }
            }
            _ => bad(),
        }
    }
    String::from_utf16(&units).unwrap_or_else(|_| bad())
}

/// Строки таблицы сверки, на которых ответ `answer(входы)` не совпал с ответом кода C#, — все сразу, чтобы первая не
/// прятала остальные. Пусто — перенос отвечает как C# на каждой строке.
pub fn parity_mismatches(name: &str, answer: impl Fn(&[String]) -> String) -> Vec<String> {
    parity(name)
        .into_iter()
        .filter_map(|(line, cells)| {
            let (expected, inputs) = cells.split_last().expect("в строке есть ячейки");
            let got = answer(inputs);
            (got != *expected).then(|| format!("{name}.tsv, строка {line}: входы {inputs:?}\n   здесь: {got}\n   C#:    {expected}"))
        })
        .collect()
}
