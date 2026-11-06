//! Клиент Интрасервиса против поддельного сервера на loopback: запросы целиком, разбор живых ответов, ошибки — фразами
//! C#-клиента (`HttpIntraserviceClient.GetAsync` / `RefusalAsync` / `Unparsed`), журнал — один образец на метод.

mod common;

use common::{LIVE, LIVE_LIFE, Reply, base64_of, route, serve};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ticketboard::intraservice::*;
use ticketboard::rules::ClosedNames;

fn client(url: &str) -> Client {
    Client::new(url, "me", "p@ss:ворд", Duration::from_secs(10))
}

/// Клиент с журналом в память: что ушло бы в errors.log.
fn logged(url: &str) -> (Client, Arc<Mutex<Vec<String>>>) {
    logged_within(url, Duration::from_secs(10))
}

fn logged_within(url: &str, timeout: Duration) -> (Client, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let log = Log::new(move |line| sink.lock().unwrap().push(line.to_string()));
    (Client::new(url, "me", "p@ss:ворд", timeout).with_log(log), lines)
}

fn paths(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock().unwrap().iter().map(|h| h.split(' ').nth(1).unwrap_or("").to_string()).collect()
}

#[test]
fn the_request_and_the_live_card() {
    let (url, seen) = serve(200, "application/json; charset=utf-8", LIVE, Duration::ZERO);
    let task = client(&url).get_task(700001).expect("живая карточка читается");
    assert_eq!((task.id, task.name.as_str(), task.status.as_str()), (700001, "Склад: удалить задания", "Закрыта"));
    assert_eq!(task.changed_text.as_deref(), Some("2026-08-27T15:54:43.460865"));
    assert_eq!((task.creator.as_deref(), task.executors.as_deref()), (Some("Петрова Анна"), Some("Иванов И. И., Сидоров С. С.")));
    assert_eq!(task.description.as_deref(), Some("строка 1\nстрока 2"));
    assert_eq!(task.extra.and_then(|x| x.service).as_deref(), Some("Приложение на ТСД"));
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
fn a_big_description() {
    // описание на 2 МБ (вставленный журнал) — читается целиком
    let big = format!(r#"{{"Task":{{"Id":7,"Name":"большая","StatusName":"Открыта","Description":"{}"}}}}"#, "x".repeat(2_000_000));
    let (url, _) = serve(200, "application/json", &big, Duration::ZERO);
    let t = client(&url).get_task(7).unwrap();
    assert_eq!((t.name.as_str(), t.description.map(|d| d.len())), ("большая", Some(2_000_000)));
}

#[test]
fn lifetime_pages() {
    let (url, seen) = route(|path| {
        if path.ends_with("&page=2") {
            Reply::json(
                r#"{"TaskLifetimes":[{"Date":"2026-10-01T10:00:00","Editor":"Иванов","Comments":"раньше"}],"Paginator":{"Page":2,"PageCount":2}}"#,
            )
        } else {
            Reply::json(LIVE_LIFE)
        }
    });
    let c = client(&url);
    let first = c.get_lifetime_page(712182, 1).unwrap();
    assert!(first.paged && !first.has_more && first.events.len() == 4);
    let comments: Vec<(&str, Option<bool>, &str)> =
        first.events.iter().filter_map(|e| Some((e.author.as_str(), e.is_public, e.comment.as_deref()?))).collect();
    assert_eq!(
        comments,
        [
            ("Сидорова Мария Ивановна", Some(true), "Добрый день! \nда, свяжусь."),
            ("Иванов Иван Иванович", Some(false), "Не могли бы вы связаться с поставщиком?")
        ]
    );
    assert_eq!(first.events[1].status, "Ожидание ответа пользователя");
    let second = c.get_lifetime_page(712182, 2).unwrap();
    assert_eq!(second.events[0].comment.as_deref(), Some("раньше"));
    // первая — тем же адресом, что всегда, без page; страница — 50 записей, свежие сверху
    assert_eq!(
        paths(&seen),
        [
            "/api/tasklifetime?taskid=712182&include=status&lastcommentsontop=true&pagesize=50",
            "/api/tasklifetime?taskid=712182&include=status&lastcommentsontop=true&pagesize=50&page=2"
        ]
    );
}

#[test]
fn user_statuses_and_check() {
    let (url, seen) = route(|path| match path {
        "/api/user?getcurrentuserinfo=true" => Reply::json(r#"{"Id":40800,"Login":"ivanovii","Name":"Иванов Иван Иванович"}"#),
        "/api/taskstatus" => Reply::json(r#"[{"Id":31,"Name":"Открыта"},{"Id":28,"Name":"Закрыта","IsFinal":true}]"#),
        _ => Reply::new(404, "text/plain", ""),
    });
    let c = client(&url);
    assert_eq!(c.get_current_user(), Ok(User { id: 40800, name: "Иванов Иван Иванович".into() }));
    let statuses = c.get_statuses().unwrap();
    assert_eq!((statuses.len(), statuses[1].is_final), (2, true));
    assert_eq!(c.check(), Ok(()));
    assert_eq!(paths(&seen), ["/api/user?getcurrentuserinfo=true", "/api/taskstatus", "/api/taskstatus"]);
    // 404 — у каждого метода своя фраза
    let (url, _) = serve(404, "text/plain", "", Duration::ZERO);
    let c = client(&url);
    assert_eq!(c.get_current_user(), Err("не удалось определить пользователя (HTTP 404)".into()));
    assert_eq!(c.get_statuses(), Err("по этому адресу нет API (HTTP 404)".into()));
    assert_eq!(c.check(), Err("по этому адресу нет API (HTTP 404)".into()));
    assert_eq!(c.get_lifetime_page(5, 1).map(|p| p.events.len()), Err("заявка не найдена (HTTP 404)".into()));
    // «Проверить»: неверный логин; страница вместо API — её текстом
    let (url, _) = serve(401, "text/plain", "", Duration::ZERO);
    assert_eq!(client(&url).check(), Err("неверный логин или пароль (HTTP 401)".into()));
    let (url, _) = serve(200, "text/html", "<html><body><p>Вход в систему</p></body></html>", Duration::ZERO);
    assert_eq!(client(&url).check(), Err("ответ не похож на API Интрасервиса:\n(html-страница, показан её текст)\nВход в систему".into()));
}

#[test]
fn any_list_by_a_query() {
    let (url, seen) = route(|_| Reply::json(r#"{"Tasks":[{"Id":7,"Name":"vpn не работает"}],"Paginator":{"Count":1}}"#));
    let q = query::TaskQuery { search: "vpn".into(), ..query::TaskQuery::default() };
    let page = client(&url).get_tasks(&q, 1, 20).unwrap();
    assert_eq!((page.total, page.found[0].name.as_str()), (1, "vpn не работает"));
    assert_eq!(paths(&seen), ["/api/task?search=vpn&include=status&sort=Changed%20desc&pagesize=20&page=1"]);
}

#[test]
fn a_success_body_that_stalls() {
    // ответ начался и замолчал до срока — «не ответил», как у C# (срок на весь запрос, с чтением тела)
    let (url, _) = route(|_| Reply { sent: Some(10), ..Reply::json(LIVE) });
    let err = Client::new(&url, "me", "p", Duration::from_secs(1)).get_task(5).unwrap_err();
    assert_eq!(err, "сервер не ответил за 1 с");
}

#[test]
fn executor_tasks() {
    let (url, seen) = route(|_| {
        Reply::json(
            r#"{"Tasks":[{"Id":7,"Name":"C","StatusId":31}],"Statuses":[{"Id":31,"Name":"Открыта"}],"Paginator":{"Count":1,"Page":2,"PageCount":2}}"#,
        )
    });
    let c = client(&url);
    let page = c.get_executor_tasks(40800, &[31, 38], 2).unwrap();
    assert_eq!((page.total, page.found[0].status.as_str()), (1, "Открыта"));
    assert_eq!(paths(&seen), ["/api/task?ExecutorIds=40800&StatusIds=31,38&include=status&sort=Changed%20desc&pagesize=200&page=2"]);
    // пустой список статусов — не «без фильтра» (притащил бы и закрытые): ошибка, и запроса нет
    assert_eq!(c.get_executor_tasks(40800, &[], 1).map(|p| p.total), Err("не задан список открытых статусов".into()));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

const STATUSES: &str = r#"[{"Id":31,"Name":"Открыта"},{"Id":38,"Name":"Передана исполнителю"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":120,"Name":"Ожидание ответа с автозакрытием"},{"Id":28,"Name":"Закрыта","IsFinal":true}]"#;

fn closed() -> ClosedNames {
    ClosedNames::new(&["Ожидание ответа с автозакрытием".to_string()])
}

#[test]
fn my_open_tickets() {
    // кто я, справочник, открытые — без закрытых по признакам и по названию; две страницы (сервер отдаёт меньше, чем просили)
    let (url, seen) = route(|path| match path {
        "/api/user?getcurrentuserinfo=true" => Reply::json(r#"{"Id":40800,"Name":"Иванов Иван Иванович"}"#),
        "/api/taskstatus" => Reply::json(STATUSES),
        p if p.ends_with("&page=1") => {
            Reply::json(r#"{"Tasks":[{"Id":1,"Name":"a"},{"Id":2,"Name":"b"}],"Paginator":{"Count":3,"Page":1,"PageCount":2}}"#)
        }
        _ => Reply::json(r#"{"Tasks":[{"Id":3,"Name":"c"}],"Paginator":{"Count":3,"Page":2,"PageCount":2}}"#),
    });
    let mine = client(&url).my_open(&closed());
    assert_eq!(mine.error, "");
    assert_eq!((mine.rows.iter().map(|f| f.id).collect::<Vec<_>>(), mine.total, mine.complete), (vec![1, 2, 3], 3, true));
    assert_eq!(mine.me, Some(User { id: 40800, name: "Иванов Иван Иванович".into() }));
    // названия закрытых по признаку — тоже в списке закрытых (для F5 и отчётов)
    assert!(
        mine.closed.contains("закрыта") && mine.closed.contains("Выполнена") && mine.closed.contains("ожидание ответа с автозакрытием")
    );
    assert!(!mine.closed.contains("Открыта"));
    assert_eq!(
        paths(&seen)[2..],
        [
            "/api/task?ExecutorIds=40800&StatusIds=31,38&include=status&sort=Changed%20desc&pagesize=200&page=1",
            "/api/task?ExecutorIds=40800&StatusIds=31,38&include=status&sort=Changed%20desc&pagesize=200&page=2"
        ]
    );
}

#[test]
fn my_open_tickets_failing() {
    let user = r#"{"Id":40800,"Name":"Я"}"#;
    let with = |statuses: &'static str| {
        route(move |path| match path {
            "/api/user?getcurrentuserinfo=true" => Reply::json(user),
            "/api/taskstatus" => Reply::json(statuses),
            _ => Reply::new(500, "application/json", r#"{"Message":"сбой"}"#),
        })
        .0
    };
    // пустой справочник и «все статусы закрытые» — разные беды
    assert_eq!(client(&with("[]")).my_open(&closed()).error, "сервер не вернул ни одного статуса заявок");
    assert_eq!(
        client(&with(r#"[{"Id":28,"Name":"Закрыта","IsFinal":true}]"#)).my_open(&closed()).error,
        "все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json"
    );
    // список не прочитался — ошибка его страницы, строк нет, список не целый
    let broken = client(&with(STATUSES)).my_open(&closed());
    assert!(broken.error.starts_with("ошибка сервера (HTTP 500):\n") && broken.rows.is_empty() && !broken.complete, "{}", broken.error);
    // кто я — не узнали: дальше не идём
    let (url, seen) = serve(401, "text/plain", "", Duration::ZERO);
    let nobody = client(&url).my_open(&closed());
    assert_eq!((nobody.error.as_str(), nobody.me, seen.lock().unwrap().len()), ("неверный логин или пароль (HTTP 401)", None, 1));
}

#[test]
fn unparsed_answers_are_logged_once_per_method() {
    let (url, _) = serve(200, "text/html", "<html><body>Вход в систему</body></html>", Duration::ZERO);
    let (c, log) = logged(&url);
    let err = c.get_task(5).unwrap_err();
    assert_eq!(err, "непонятный ответ сервера:\n(html-страница, показан её текст)\nВход в систему");
    c.get_task(6).unwrap_err();
    c.get_statuses().unwrap_err();
    let lines = log.lock().unwrap().clone();
    assert_eq!(lines.len(), 2, "один образец на метод за запуск: {lines:?}");
    assert_eq!(lines[0], "get_task: не разобран ответ сервера (40 симв.):\n<html><body>Вход в систему</body></html>");
    assert!(lines[1].starts_with("get_statuses: не разобран ответ сервера"), "{}", lines[1]);
    // «Проверить» — свой образец и своя фраза
    c.check().unwrap_err();
    assert!(log.lock().unwrap()[2].starts_with("check: не разобран ответ сервера"));
}

#[test]
fn refusals_in_csharp_words() {
    for (code, first) in [
        (401, "неверный логин или пароль (HTTP 401)"),
        (403, "нет доступа (HTTP 403)"),
        (404, "заявка не найдена (HTTP 404)"),
        (500, "ошибка сервера (HTTP 500)"),
        (418, "ошибка сервера (HTTP 418)"),
    ] {
        let (url, _) = serve(code, "application/json", r#"{"Message":"подробности"}"#, Duration::ZERO);
        let err = client(&url).get_task(5).unwrap_err();
        assert_eq!(err, format!("{first}:\n{{\"Message\":\"подробности\"}}"), "{code}");
    }
    let (url, _) = serve(404, "application/json", " \r\n ", Duration::ZERO);
    assert_eq!(client(&url).get_task(5).unwrap_err(), "заявка не найдена (HTTP 404)", "пустое тело — одна фраза");
}

#[test]
fn error_bodies_are_read_up_to_64_kb() {
    // тело ошибки — не больше 64 КБ; «только начало» в журнале — когда пришло больше; ровно столько — целиком
    for (size, cut) in [(64 * 1024, false), (64 * 1024 + 1, true)] {
        let (url, _) = serve(500, "text/plain", &"x".repeat(size), Duration::ZERO);
        let (c, log) = logged(&url);
        let err = c.get_task(5).unwrap_err();
        assert_eq!(err, format!("ошибка сервера (HTTP 500):\n{}\n…", "x".repeat(1000)));
        let line = log.lock().unwrap()[0].clone();
        let want =
            format!("get_task: HTTP 500 на api/task/5?include=status ({} симв.{}):\n", 64 * 1024, if cut { ", только начало" } else { "" });
        assert!(line.starts_with(&want), "{size}: {}", &line[..120]);
        assert_eq!(line.chars().count(), want.chars().count() + 4000 + 2, "в журнал — 4000 знаков");
    }
    // замолчало посреди тела — что успело прийти, и «только начало»
    let (url, _) = route(|_| Reply { sent: Some(1000), ..Reply::new(500, "text/plain", "y".repeat(100_000)) });
    let (c, log) = logged_within(&url, Duration::from_secs(1));
    let err = c.get_task(5).unwrap_err();
    assert_eq!(err, format!("ошибка сервера (HTTP 500):\n{}", "y".repeat(1000)));
    assert!(log.lock().unwrap()[0].starts_with("get_task: HTTP 500 на api/task/5?include=status (1000 симв., только начало):\n"));
}

#[test]
fn a_russian_error_page_in_1251() {
    let page = "<html><head><title>Доступ запрещён</title></head><body><p>Нет прав на этот ресурс</p></body></html>";
    let bytes: Vec<u8> = page
        .chars()
        .map(|c| match c as u32 {
            0x410..=0x44f => (c as u32 - 0x410 + 0xc0) as u8,
            0x451 => 0xb8,
            n => n as u8,
        })
        .collect();
    let (url, _) = route(move |_| Reply::new(403, "text/html; charset=windows-1251", bytes.clone()));
    assert_eq!(
        client(&url).get_task(5).unwrap_err(),
        "нет доступа (HTTP 403):\n(html-страница, показан её текст)\nДоступ запрещён\nНет прав на этот ресурс"
    );
}

#[test]
fn credentials_echoed_by_the_server_are_masked() {
    // страница прокси или IIS повторяет заголовки запроса — в окне (а его просят переслать) логина и пароля быть не должно
    let token = base64_of("me:p@ss:ворд");
    let body = format!("Trace: Authorization: Basic {token}\nauthorization: basic   {token}\nсам токен: {token}\nпароль p@ss:ворд");
    let (url, _) = serve(500, "text/plain", &body, Duration::ZERO);
    let (c, log) = logged(&url);
    let err = c.get_task(5).unwrap_err();
    assert!(err.starts_with("ошибка сервера (HTTP 500)"), "{err}");
    for text in [err.as_str(), log.lock().unwrap()[0].as_str()] {
        assert!(!text.contains(&token) && !text.contains("p@ss:ворд"), "{text}");
        assert!(text.contains("Basic ***") && text.contains("basic   ***") && text.contains("сам токен: ***"), "{text}");
    }
    // ответ, который не разобран, — тоже
    let (url, _) = serve(200, "text/html", &format!("<pre>Basic {token}</pre>"), Duration::ZERO);
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.starts_with("непонятный ответ сервера") && !err.contains(&token), "{err}");
}

#[test]
fn a_broken_address_is_named() {
    let err = client("https://hd example ru").get_task(5).unwrap_err();
    assert!(err.starts_with("адрес Интрасервиса в настройках не годится"), "{err}");
}

#[test]
fn a_redirect_is_not_followed() {
    // переадресация — обычно на страницу входа: не идём за ней (и не несём туда логин и пароль)
    let (url, seen) = serve(302, "text/html", "<html>вход</html>", Duration::ZERO);
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.contains("переадресация (HTTP 302)") && err.contains("/Account/Login"), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 1, "за переадресацией — ни шагу");
}

#[test]
fn no_server_and_a_silent_one() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(); // порт закрыт: слушатель ушёл
    let err = client(&format!("http://127.0.0.1:{port}")).get_task(5).unwrap_err();
    assert!(err.starts_with("сервер недоступен:\n"), "{err}");
    assert!(is_transient(&err));
    let (url, _) = serve(200, "application/json", LIVE, Duration::from_secs(3));
    let err = Client::new(&url, "me", "p", Duration::from_secs(1)).get_task(5).unwrap_err();
    assert_eq!(err, "сервер не ответил за 1 с");
    // срок с долями — как у C# на русской Windows: запятая
    let err = Client::new(&url, "me", "p", Duration::from_millis(1500)).get_task(5).unwrap_err();
    assert_eq!(err, "сервер не ответил за 1,5 с");
}

#[test]
fn a_broken_connection_is_named_by_when() {
    // ответ начался и оборвался — «соединение оборвалось» (у C# — IOException при чтении тела)
    let (url, _) = route(|_| Reply {
        sent: Some(100),
        hang_up: true,
        ..Reply::json(&format!("{{\"Task\":{{\"Id\":5,\"Name\":\"{}\"}}}}", "x".repeat(5000)))
    });
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.starts_with("соединение оборвалось:\n") && is_transient(&err), "{err}");
    // оборвалось до ответа — «сервер недоступен» (у C# — HttpRequestException)
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            drop(stream);
        }
    });
    let err = client(&url).get_task(5).unwrap_err();
    assert!(err.starts_with("сервер недоступен:\n"), "{err}");
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
    // токен — после «basic» и пробелов (любых, и перевода строки), от 6 знаков base64 (+ / = тоже); регистр — любой
    assert_eq!(redact("Authorization: Basic abcdef", &[]), "Authorization: Basic ***");
    assert_eq!(redact("x BASIC\tAb+/=12, дальше", &[]), "x BASIC\t***, дальше");
    assert_eq!(redact("Basic\r\n  Zm9vYmFy", &[]), "Basic\r\n  ***", "перевод строки — тоже пробел, как \\s у .NET");
    assert_eq!(redact("Basic abcde", &[]), "Basic abcde", "5 знаков — не токен");
    assert_eq!(redact("Basicabcdefgh", &[]), "Basicabcdefgh", "без пробела — слово, не токен");
    assert_eq!(redact("basic basic Zm9vYmFy end", &[]), "basic basic *** end", "«basic» без токена пропускается, ищется дальше");
    assert_eq!(redact("до Basic Zm9vYmFy и Basic YmFyYmF6 после", &[]), "до Basic *** и Basic *** после");
    assert_eq!(redact("пароль 12345 и abc", &["12345", "abc"]), "пароль *** и abc", "секрет короче 4 знаков не прячем: он в любом тексте");
    assert_eq!(redact("Ёж basic Zm9vYmFy", &[]), "Ёж basic ***", "кириллица перед — смещения те же");
}
