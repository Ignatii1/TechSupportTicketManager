//! Скачивание вложения (карточка 4.4-1): настройка «Вложения в базу знаний», правила «качать ли» и «html ли имя» —
//! таблицами сверки (ответы пишет код C#); `Client::download_file` — перенос `HttpIntraserviceClient.DownloadFileAsync` и
//! клиентской части `KnowledgeExport.SelfCheck.Files.cs` на поддельный сервер из сырых сокетов: он умеет код, заголовки,
//! тело кусками с паузами, обрыв посреди тела и молчание, пока клиент сам не уйдёт.

mod common;

use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use ticketboard::export::{AttachmentScope, downloads, is_html};
use ticketboard::intraservice::{Client, Downloaded, FILE_HEADERS_TIMEOUT, FILE_IDLE_TIMEOUT, Log};
use ticketboard::settings::{Settings, parse_settings};

// ---------------------------------------------------------------------------------------------------------------------
// Таблицы сверки и чистые правила
// ---------------------------------------------------------------------------------------------------------------------

fn yes(b: bool) -> &'static str {
    if b { "да" } else { "нет" }
}

#[test]
fn the_setting_is_read_like_net_reads_it() {
    // значение KnowledgeAttachments в settings.json → имя настройки и «качать ли» без роли и с ролью; не прочиталось — ОШИБКА
    let wrong =
        common::parity_mismatches("settings-attachments", |c| match parse_settings(&format!("{{\"KnowledgeAttachments\": {}}}", c[0])) {
            Ok(s) => {
                let k = s.knowledge_attachments;
                format!("{k}|без роли {}|с ролью {}", yes(downloads(k, "")), yes(downloads(k, "наблюдатель")))
            }
            Err(_) => "ОШИБКА".to_string(),
        });
    assert!(wrong.is_empty(), "настройка читается не как у C#:\n{}", wrong.join("\n"));
}

#[test]
fn an_html_name_is_known_like_in_the_app() {
    let wrong = common::parity_mismatches("attachment-html", |c| yes(is_html(&c[0])).to_string());
    assert!(wrong.is_empty(), "«html ли имя» не как у C#:\n{}", wrong.join("\n"));
}

#[test]
fn the_setting_defaults_to_my_tickets() {
    // C#: `KnowledgeAttachments` по умолчанию — Mine; нет поля в файле — тоже Mine
    assert_eq!(Settings::default().knowledge_attachments, AttachmentScope::Mine, "по умолчанию — моих заявок");
    assert_eq!(parse_settings("{}").unwrap().knowledge_attachments, AttachmentScope::Mine, "файл без поля — моих заявок");
    let other = parse_settings(r#"{"WipLimit": 9, "KnowledgeDir": "kb"}"#).unwrap();
    assert_eq!(other.knowledge_attachments, AttachmentScope::Mine, "поле не названо — моих заявок");
    // у контекста выгрузки (`ExportContext`) по умолчанию — не скачивать
    assert_eq!(AttachmentScope::default(), AttachmentScope::None, "значение по умолчанию у типа — не скачивать");
}

#[test]
fn the_scope_is_written_like_a_net_enum() {
    // `AttachmentScope.ToString()`: имя, а у числа без имени — само число (оно уходит в таблицу сверки и в итог)
    let shown: Vec<String> =
        [AttachmentScope::Mine, AttachmentScope::All, AttachmentScope::None, AttachmentScope::Other(3), AttachmentScope::Other(-1)]
            .iter()
            .map(|s| s.to_string())
            .collect();
    assert_eq!(shown, ["Mine", "All", "None", "3", "-1"]);
}

#[test]
fn whether_to_download_depends_on_the_role_by_length() {
    // `Attachments.Downloads`: All — всегда; Mine — если роль не пуста (`myRole.Length > 0`, пробелы — тоже роль); остальное — нет
    let cases = [
        (AttachmentScope::All, "", true),
        (AttachmentScope::All, "исполнитель", true),
        (AttachmentScope::Mine, "", false),
        (AttachmentScope::Mine, " ", true),
        (AttachmentScope::Mine, "инициатор", true),
        (AttachmentScope::None, "", false),
        (AttachmentScope::None, "исполнитель", false),
        (AttachmentScope::Other(7), "исполнитель", false),
        (AttachmentScope::Other(7), "", false),
    ];
    let wrong: Vec<String> = cases
        .iter()
        .filter(|(s, role, want)| downloads(*s, role) != *want)
        .map(|(s, role, want)| format!("{s} / «{role}»: ждали {want}"))
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_default_limits_are_a_minute_each() {
    // `FileHeadersTimeout`, `FileIdleTimeout` — по 60 с
    assert_eq!((FILE_HEADERS_TIMEOUT, FILE_IDLE_TIMEOUT), (Duration::from_secs(60), Duration::from_secs(60)));
}

// ---------------------------------------------------------------------------------------------------------------------
// Поддельный сервер
// ---------------------------------------------------------------------------------------------------------------------

/// Что сервер делает на запрос, по порядку; после последнего шага соединение закрывается.
#[derive(Clone)]
enum Step {
    /// Строка состояния и заголовки (`Connection: close` дописывается).
    Status(u16, Vec<(String, String)>),
    Send(Vec<u8>),
    Pause(u64),
    /// Молчать, не закрывая соединения, пока клиент сам его не бросит (или 30 с — чтобы поток не жил вечно).
    Hold,
}

type Script = Vec<Step>;

fn status(code: u16, headers: &[(&str, &str)]) -> Step {
    Step::Status(code, headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
}

/// Ответ целиком: заголовки, Content-Length по телу, тело.
fn reply(code: u16, headers: &[(&str, &str)], body: &[u8]) -> Script {
    let length = body.len().to_string();
    let mut all = headers.to_vec();
    all.push(("Content-Length", &length));
    vec![status(code, &all), Step::Send(body.to_vec())]
}

fn file_reply(body: &[u8]) -> Script {
    reply(200, &[("Content-Type", "application/octet-stream")], body)
}

/// Заголовки с заявленной длиной `declared`, из тела — только начало `sent`, дальше — `then` (Hold или конец: обрыв).
fn partial(code: u16, content_type: &str, declared: usize, sent: &[u8], then: Option<Step>) -> Script {
    let length = declared.to_string();
    let mut script = vec![status(code, &[("Content-Type", content_type), ("Content-Length", &length)]), Step::Send(sent.to_vec())];
    script.extend(then);
    script
}

struct Fake {
    url: String,
    /// Заголовки каждого запроса по порядку.
    seen: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    fn paths(&self) -> Vec<String> {
        self.seen.lock().unwrap().iter().map(|h| h.split(' ').nth(1).unwrap_or("").to_string()).collect()
    }
}

/// Сервер с путями `/api/taskfile/{номер}` → сценарий; чужой путь — 404 без тела. Один поток на соединение.
fn fake(routes: Vec<(i32, Script)>) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let routes = Arc::new(routes);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let (routes, seen) = (routes.clone(), seen2.clone());
            thread::spawn(move || serve_one(stream, &routes, &seen));
        }
    });
    Fake { url, seen }
}

fn serve_one(mut stream: std::net::TcpStream, routes: &[(i32, Script)], seen: &Mutex<Vec<String>>) {
    let _ = stream.set_nodelay(true);
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    seen.lock().unwrap().push(head.clone());
    let path = head.split(' ').nth(1).unwrap_or("");
    // и под адресом сервера с папкой («…/IS/api/taskfile/…»)
    let id = path.split_once("/api/taskfile/").and_then(|(_, n)| n.parse::<i32>().ok());
    let script = routes.iter().find(|(n, _)| Some(*n) == id).map(|(_, s)| s.clone()).unwrap_or_else(|| reply(404, &[], b""));
    for step in script {
        match step {
            Step::Status(code, headers) => {
                let mut text = format!("HTTP/1.1 {code} Status\r\n");
                for (k, v) in headers {
                    text.push_str(&format!("{k}: {v}\r\n"));
                }
                text.push_str("Connection: close\r\n\r\n");
                if stream.write_all(text.as_bytes()).is_err() {
                    return;
                }
            }
            Step::Send(bytes) => {
                if stream.write_all(&bytes).and_then(|_| stream.flush()).is_err() {
                    return;
                }
            }
            Step::Pause(ms) => thread::sleep(Duration::from_millis(ms)),
            Step::Hold => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                let mut sink = [0u8; 64];
                while stream.read(&mut sink).is_ok_and(|n| n > 0) {}
                return;
            }
        }
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

// ---------------------------------------------------------------------------------------------------------------------
// Клиент и приёмник
// ---------------------------------------------------------------------------------------------------------------------

/// Заголовки — 5 с, кусок — 2 с: проверки без молчания не упираются в сроки даже на медленной машине.
fn client(url: &str) -> Client {
    limited(url, Duration::from_secs(5), Duration::from_secs(2))
}

fn limited(url: &str, headers: Duration, idle: Duration) -> Client {
    Client::new(url, "me", "p", Duration::from_secs(10)).with_file_timeouts(headers, idle)
}

/// Короткий срок куска для проверок молчания; заголовки — с запасом, чтобы срок куска не путался со сроком заголовков.
fn quiet(url: &str) -> Client {
    limited(url, Duration::from_secs(5), Duration::from_millis(300))
}

/// «Файл на диске»: записанное видно и после того, как клиент вернул управление; первая запись может быть медленной, а
/// запись — ошибочной.
#[derive(Clone)]
struct Sink {
    data: Arc<Mutex<Vec<u8>>>,
    first_write_takes: Duration,
    fail: bool,
    wrote: Arc<AtomicBool>,
}

impl Sink {
    fn new() -> Sink {
        Sink {
            data: Arc::new(Mutex::new(Vec::new())),
            first_write_takes: Duration::ZERO,
            fail: false,
            wrote: Arc::new(AtomicBool::new(false)),
        }
    }
    fn bytes(&self) -> Vec<u8> {
        self.data.lock().unwrap().clone()
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail {
            return Err(io::Error::other("диск полон"));
        }
        if !self.wrote.swap(true, Ordering::SeqCst) {
            thread::sleep(self.first_write_takes);
        }
        self.data.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Got {
    result: io::Result<Downloaded>,
    /// Что легло в файл.
    written: Vec<u8>,
    /// Каждый вызов хода: (переданное число, сколько в файле было в этот миг).
    progress: Vec<(u64, usize)>,
}

impl Got {
    fn done(&self) -> &Downloaded {
        self.result.as_ref().unwrap_or_else(|e| panic!("ждали Ok, а вышло Err: {e}"))
    }
    fn error(&self) -> String {
        self.done().error.clone()
    }
}

fn fetch_with(c: &Client, id: i32, sink: Sink, stop: &dyn Fn() -> bool, stop_at: u64, html: bool) -> Got {
    let mut to = sink.clone();
    let data = sink.data.clone();
    let mut progress = Vec::new();
    let result = c.download_file(id, &mut to, &mut |total| progress.push((total, data.lock().unwrap().len())), stop, stop_at, html);
    Got { result, written: sink.bytes(), progress }
}

/// Весь файл, не останавливаясь, html не ждём — как зовёт выгрузка за вложение с не-html именем.
fn fetch(c: &Client, id: i32) -> Got {
    fetch_with(c, id, Sink::new(), &|| false, u64::MAX, false)
}

fn fetch_html(c: &Client, id: i32) -> Got {
    fetch_with(c, id, Sink::new(), &|| false, u64::MAX, true)
}

fn log() -> (Arc<Log>, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    (Log::new(move |line| sink.lock().unwrap().push(line.to_string())), lines)
}

/// Образец логина и пароля из самопроверки C#: user / pass → base64.
const TOKEN: &str = "dXNlcjpwYXNz";

fn user_client(url: &str) -> (Client, Arc<Mutex<Vec<String>>>) {
    let (log, lines) = log();
    (
        Client::new(url, "user", "pass", Duration::from_secs(10))
            .with_file_timeouts(Duration::from_secs(5), Duration::from_secs(2))
            .with_log(log),
        lines,
    )
}

fn bytes_of(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// «<html><body>Доступ запрещён</body></html>» в windows-1251 — байтами (как `Page1251` самопроверки C#).
fn page_1251() -> Vec<u8> {
    let mut page = b"<html><body>".to_vec();
    page.extend([0xC4, 0xEE, 0xF1, 0xF2, 0xF3, 0xEF, 0x20, 0xE7, 0xE0, 0xEF, 0xF0, 0xE5, 0xF9, 0xB8, 0xED]);
    page.extend(b"</body></html>");
    page
}

// ---------------------------------------------------------------------------------------------------------------------
// Файл пришёл
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_file_arrives_byte_for_byte_with_its_length_and_progress() {
    // запрос — `GET api/taskfile/{номер}` с Basic-входом; тело кусками; длина — из Content-Length; ход — суммой после каждого
    // записанного куска
    let body = bytes_of(600);
    let script = vec![
        status(200, &[("Content-Type", "application/octet-stream"), ("Content-Length", "600")]),
        Step::Send(body[..100].to_vec()),
        Step::Pause(30),
        Step::Send(body[100..300].to_vec()),
        Step::Pause(30),
        Step::Send(body[300..].to_vec()),
    ];
    let server = fake(vec![(9001, script)]);
    let got = fetch(&client(&server.url), 9001);
    let done = got.done();
    assert_eq!((done.bytes, done.length, done.error.as_str()), (600, Some(600), ""), "{done:?}");
    assert_eq!(got.written, body, "файл побайтно");
    assert!(!got.progress.is_empty() && got.progress.windows(2).all(|w| w[0].0 < w[1].0), "ход растёт: {:?}", got.progress);
    assert_eq!(got.progress.last().map(|p| p.0), Some(600), "последний ход — весь файл");
    assert!(got.progress.iter().all(|(total, in_file)| *total as usize == *in_file), "ход — после записи куска: {:?}", got.progress);
    let head = server.seen.lock().unwrap()[0].clone();
    assert!(head.starts_with("GET /api/taskfile/9001 HTTP/1.1\r\n"), "запрос: {head}");
    let auth = format!("Basic {}", common::base64_of("me:p"));
    assert!(head.lines().any(|l| l.to_ascii_lowercase().starts_with("authorization:") && l.ends_with(&auth)), "вход Basic: {head}");
}

#[test]
fn an_empty_file_is_a_file() {
    let server = fake(vec![(9002, file_reply(b""))]);
    let got = fetch(&client(&server.url), 9002);
    assert_eq!(got.done(), &Downloaded { bytes: 0, length: Some(0), error: String::new() });
    assert!(got.written.is_empty() && got.progress.is_empty(), "ни записей, ни хода: {:?}", got.progress);
}

#[test]
fn a_file_without_content_length_has_no_length() {
    // конец тела — закрытие соединения: файл целиком, длины нет (C# 9970)
    let body = bytes_of(3000);
    let server = fake(vec![(9970, vec![status(200, &[("Content-Type", "application/octet-stream")]), Step::Send(body.clone())])]);
    let got = fetch(&client(&server.url), 9970);
    assert_eq!(got.done(), &Downloaded { bytes: 3000, length: None, error: String::new() });
    assert_eq!(got.written, body);
}

#[test]
fn a_file_of_any_other_type_than_html_is_a_file() {
    // страница — только text/html; text/plain, xhtml и ответ без типа — файлы
    let routes = vec![
        (1, reply(200, &[("Content-Type", "text/plain")], b"hello")),
        (2, reply(200, &[("Content-Type", "application/xhtml+xml")], b"<html/>")),
        (3, reply(200, &[], b"no type")),
    ];
    let server = fake(routes);
    let c = client(&server.url);
    let got: Vec<(String, Vec<u8>)> = [1, 2, 3].iter().map(|id| fetch(&c, *id)).map(|g| (g.error(), g.written)).collect();
    assert_eq!(got, [(String::new(), b"hello".to_vec()), (String::new(), b"<html/>".to_vec()), (String::new(), b"no type".to_vec())]);
}

#[test]
fn stop_at_ends_reading_with_the_first_chunk_that_reaches_it() {
    // C# 5б: stopAt: 1 — размер вложения без чтения тела: пришёл первый кусок — чтение кончается, файл не целый, длина — из заголовка
    let server = fake(vec![(9968, file_reply(&vec![0u8; 200_000]))]);
    let got = fetch_with(&client(&server.url), 9968, Sink::new(), &|| false, 1, false);
    let done = got.done();
    assert!(done.error.is_empty() && done.length == Some(200_000), "{done:?}");
    assert!(done.bytes >= 1 && done.bytes < 200_000, "прочитан только первый кусок: {}", done.bytes);
    assert_eq!(got.written.len() as u64, done.bytes, "в файле — то, что прочитано");
}

#[test]
fn stop_at_does_not_wait_for_the_rest_of_the_file() {
    // два куска по 100 байт, потом сервер молчит: stop_at 150 кончает чтение на втором куске, не дожидаясь срока куска
    let (a, b) = (bytes_of(100), vec![7u8; 100]);
    let script = vec![
        status(200, &[("Content-Type", "application/octet-stream"), ("Content-Length", "1000")]),
        Step::Send(a),
        Step::Pause(60),
        Step::Send(b),
        Step::Hold,
    ];
    let server = fake(vec![(9500, script)]);
    let got = fetch_with(&quiet(&server.url), 9500, Sink::new(), &|| false, 150, false);
    let done = got.done();
    assert_eq!((done.error.as_str(), done.length), ("", Some(1000)), "{done:?}");
    assert!(
        (150..=200).contains(&done.bytes) && got.written.len() as u64 == done.bytes,
        "прочитано {}, в файле {}",
        done.bytes,
        got.written.len()
    );
}

#[test]
fn a_body_in_slow_chunks_has_no_total_limit() {
    // срок — на каждый кусок, а не на весь файл: пять кусков с паузой 250 мс идут дольше срока куска (600 мс)
    let mut script = vec![status(200, &[("Content-Type", "application/octet-stream"), ("Content-Length", "500")])];
    for _ in 0..5 {
        script.push(Step::Send(vec![1u8; 100]));
        script.push(Step::Pause(250));
    }
    let server = fake(vec![(9600, script)]);
    let c = limited(&server.url, Duration::from_secs(5), Duration::from_millis(600));
    let got = fetch(&c, 9600);
    assert_eq!(
        got.done(),
        &Downloaded { bytes: 500, length: Some(500), error: String::new() },
        "пока куски идут в срок — файл качается сколько нужно"
    );
}

#[test]
fn a_slow_disk_is_not_the_servers_silence() {
    // C#: первая запись на диск (сетевой диск, антивирус) дольше срока куска (300 мс) — не молчание сервера
    let server = fake(vec![(9968, file_reply(&vec![0u8; 200_000]))]);
    let sink = Sink { first_write_takes: Duration::from_millis(900), ..Sink::new() };
    let got = fetch_with(&quiet(&server.url), 9968, sink, &|| false, u64::MAX, false);
    assert_eq!(got.done(), &Downloaded { bytes: 200_000, length: Some(200_000), error: String::new() });
    assert_eq!(got.written.len(), 200_000, "файл записан целиком");
}

#[test]
fn a_write_error_goes_up_as_an_error_not_a_string() {
    // ошибка записи в `to` — не сети: уходит наверх (у C# — исключением), строкой не становится
    let server = fake(vec![(9003, file_reply(&bytes_of(5000)))]);
    let sink = Sink { fail: true, ..Sink::new() };
    let got = fetch_with(&client(&server.url), 9003, sink, &|| false, u64::MAX, false);
    let err = got.result.as_ref().expect_err("ошибка записи — Err");
    assert_eq!((err.kind(), err.to_string()), (io::ErrorKind::Other, "диск полон".to_string()));
}

#[test]
fn stop_before_connecting_asks_the_server_nothing() {
    let server = fake(vec![(9004, file_reply(b"x"))]);
    let got = fetch_with(&client(&server.url), 9004, Sink::new(), &|| true, u64::MAX, false);
    assert_eq!(got.result.as_ref().expect_err("остановили — Err").kind(), io::ErrorKind::Interrupted);
    assert!(got.written.is_empty(), "ничего не записано");
    assert_eq!(server.requests(), 0, "запроса к серверу не было");
}

#[test]
fn stop_between_chunks_leaves_what_was_written_and_does_not_wait() {
    // «Остановить» смотрится перед каждым куском: после первого сервер молчит, а клиент уже не ждёт его (срок куска 2 с)
    let stopped = Arc::new(AtomicBool::new(false));
    let server = fake(vec![(9501, partial(200, "application/octet-stream", 1000, &bytes_of(100), Some(Step::Hold)))]);
    let flag = stopped.clone();
    let sink = Sink::new();
    let data = sink.data.clone();
    let mut to = sink.clone();
    let result = client(&server.url).download_file(
        9501,
        &mut to,
        &mut |_| flag.store(true, Ordering::SeqCst),
        &|| stopped.load(Ordering::SeqCst),
        u64::MAX,
        false,
    );
    assert_eq!(result.expect_err("остановили — Err").kind(), io::ErrorKind::Interrupted);
    let written = data.lock().unwrap().len();
    assert!((1..=100).contains(&written), "записан первый кусок и только он: {written}");
}

// ---------------------------------------------------------------------------------------------------------------------
// Отказы сервера
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_refusal_without_a_body_is_one_phrase_with_the_code() {
    // `RefusalAsync`: 401 / 403 / 404 («файл не найден») / остальное («ошибка сервера»); тела нет — одна фраза; повтора нет
    let cases = [
        (401, "неверный логин или пароль (HTTP 401)"),
        (403, "нет доступа (HTTP 403)"),
        (404, "файл не найден (HTTP 404)"),
        (500, "ошибка сервера (HTTP 500)"),
        (503, "ошибка сервера (HTTP 503)"),
    ];
    let server = fake(cases.iter().map(|(code, _)| (*code, reply(*code as u16, &[], b""))).collect());
    let c = client(&server.url);
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(code, want)| {
            let got = fetch(&c, *code);
            let done = got.done().clone();
            let right = done == Downloaded { bytes: 0, length: None, error: want.to_string() } && got.written.is_empty();
            (!right).then(|| format!("{code}: ждали «{want}», вышло {done:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert_eq!(server.requests(), cases.len(), "каждый — одним запросом, повтор решает вызывающий");
}

#[test]
fn a_refusal_shows_what_the_server_said() {
    // дальше — тело ответа (`Evidence`): обычный текст — как есть, пробелы по краям — долой; только пробелы — тела нет
    let routes = vec![
        (1, reply(404, &[("Content-Type", "application/json")], br#"{"Message":"not found"}"#)),
        (2, reply(403, &[("Content-Type", "text/plain; charset=utf-8")], "  Доступ закрыт \r\n".as_bytes())),
        (3, reply(500, &[("Content-Type", "text/plain")], b" \r\n\t ")),
        (4, reply(403, &[("Content-Type", "text/html; charset=utf-8")], "<html><body>Доступ запрещён</body></html>".as_bytes())),
    ];
    let server = fake(routes);
    let c = client(&server.url);
    let errors: Vec<String> = (1..=4).map(|id| fetch(&c, id).error()).collect();
    assert_eq!(
        errors,
        [
            "файл не найден (HTTP 404):\n{\"Message\":\"not found\"}",
            "нет доступа (HTTP 403):\nДоступ закрыт",
            "ошибка сервера (HTTP 500)",
            "нет доступа (HTTP 403):\n(html-страница, показан её текст)\nДоступ запрещён",
        ]
    );
}

#[test]
fn a_refusal_hides_the_login_token_and_is_logged_once() {
    // `Masked` + `LogOnce`: страница прокси повторила заголовок входа — в ошибке и в errors.log звёздочки; образец — один на код
    let page = format!("<html><body><table><tr><td>Authorization</td><td>{TOKEN}</td></tr></table></body></html>");
    let routes = vec![
        (9701, reply(403, &[("Content-Type", "text/html")], page.as_bytes())),
        (9702, reply(403, &[("Content-Type", "text/html")], page.as_bytes())),
    ];
    let server = fake(routes);
    let (c, lines) = user_client(&server.url);
    let (first, second) = (fetch(&c, 9701).error(), fetch(&c, 9702).error());
    assert!(first.starts_with("нет доступа (HTTP 403):\n") && first.contains("***") && !first.contains(TOKEN), "{first}");
    assert!(second.contains("***") && !second.contains(TOKEN), "{second}");
    let logged = lines.lock().unwrap().clone();
    // ключ образца — метод и код (`{call} HTTP {code}`); имя метода — download_file, как get_task у `get_task` (аналог nameof)
    assert_eq!(logged.len(), 1, "один образец на метод и код за запуск: {logged:?}");
    assert!(logged[0].starts_with("download_file: HTTP 403 на api/taskfile/9701 ("), "{}", logged[0]);
    assert!(logged[0].contains("***") && !logged[0].contains(TOKEN), "в журнале токена нет: {}", logged[0]);
}

#[test]
fn a_refusal_body_over_the_limit_is_shown_short() {
    // C# 629: тело отказа больше 64 КБ читается не целиком; целое и оборванное за пределом — тот же «нет доступа», а не сбой сети
    let mut page = b"<html><body>".to_vec();
    page.extend(vec![b'x'; 100_000]);
    page.extend(b"</body></html>");
    let cut = partial(403, "text/html", page.len(), &page[..70_000], None);
    let server = fake(vec![(1, reply(403, &[("Content-Type", "text/html")], &page)), (2, cut)]);
    let (c, lines) = user_client(&server.url);
    for id in [1, 2] {
        let error = fetch(&c, id).error();
        assert!(error.starts_with("нет доступа (HTTP 403):\n"), "{id}: {}", error.chars().take(120).collect::<String>());
        assert!(error.chars().count() < 1200, "{id}: в окне — не больше тысячи знаков тела, а тут {}", error.chars().count());
        assert!(!error.contains("недоступен") && !error.contains("оборвалось"), "{id}: код ответа известен, сбоя сети нет");
    }
    assert!(
        lines.lock().unwrap().iter().any(|l| l.contains("только начало")),
        "в журнале — пометка «только начало»: {:?}",
        lines.lock().unwrap()
    );
}

#[test]
fn a_refusal_body_that_breaks_off_or_goes_silent_is_still_the_refusal() {
    // тело отказа оборвалось или замолчало — тот же отказ с тем, что успело прийти (не «соединение оборвалось», не «не ответил»)
    let routes = vec![
        (1, partial(403, "text/plain", 500, "начало ответа".as_bytes(), None)),
        (2, partial(403, "text/plain", 500, "начало ответа".as_bytes(), Some(Step::Hold))),
        // C# 9965: страница в 1251 обрезана на 10 байтах («<html><bod») и сервер молчит
        (3, partial(403, "text/html; charset=windows-1251", 40, &page_1251()[..10], Some(Step::Hold))),
    ];
    let server = fake(routes);
    let c = quiet(&server.url);
    let errors: Vec<String> = (1..=3).map(|id| fetch(&c, id).error()).collect();
    for (id, error) in (1..=2).zip(&errors) {
        assert!(error.starts_with("нет доступа (HTTP 403):\n") && error.contains("начало ответа"), "{id}: {error}");
        assert!(!error.contains("не ответил") && !error.contains("оборвалось"), "{id}: {error}");
    }
    assert!(
        errors[2].starts_with("нет доступа (HTTP 403)") && !errors[2].contains("Доступ запрещён") && !errors[2].contains("не ответил"),
        "3: {}",
        errors[2]
    );
    assert_eq!(server.requests(), 3, "спрошено по разу, без повторов");
}

#[test]
fn a_refusal_in_windows_1251_is_read() {
    // C# 9961, 9963, 9966: 403 в windows-1251 читается; кодировка незнакомая (в том числе utf-7, который .NET знает, но не читает) —
    // отказ тот же, тело показано как UTF-8, клиент не падает
    let routes = vec![
        (1, reply(403, &[("Content-Type", "text/html; charset=windows-1251")], &page_1251())),
        (2, reply(403, &[("Content-Type", "text/html; charset=x-no-such")], &page_1251())),
        (3, reply(403, &[("Content-Type", "text/html; charset=utf-7")], b"<html><body>denied</body></html>")),
    ];
    let server = fake(routes);
    let c = client(&server.url);
    let errors: Vec<String> = (1..=3).map(|id| fetch(&c, id).error()).collect();
    assert_eq!(errors[0], "нет доступа (HTTP 403):\n(html-страница, показан её текст)\nДоступ запрещён");
    assert!(errors[1].starts_with("нет доступа (HTTP 403):\n"), "незнакомая кодировка: {}", errors[1]);
    assert!(errors[2].starts_with("нет доступа (HTTP 403):\n") && errors[2].contains("denied"), "utf-7: {}", errors[2]);
}

// ---------------------------------------------------------------------------------------------------------------------
// Страница вместо файла и переадресация
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_page_instead_of_a_file_is_an_error_not_a_file() {
    // html у вложения, которое не html: ошибка с текстом страницы, в файл ничего не легло; тип — без учёта регистра
    let page = "<html><body>Вход</body></html>".as_bytes();
    let routes = vec![
        (1, reply(200, &[("Content-Type", "text/html; charset=utf-8")], page)),
        (2, reply(200, &[("Content-Type", "TEXT/HTML")], page)),
    ];
    let server = fake(routes);
    let c = client(&server.url);
    for id in [1, 2] {
        let got = fetch(&c, id);
        assert_eq!(
            got.done(),
            &Downloaded {
                bytes: 0,
                length: None,
                error: "сервер вернул страницу вместо файла:\n(html-страница, показан её текст)\nВход".into()
            },
            "{id}"
        );
        assert!(got.written.is_empty() && got.progress.is_empty(), "{id}: страница в файл не легла");
    }
}

#[test]
fn an_html_attachment_is_a_file_when_html_is_expected() {
    // `htmlExpected`: у html-имени text/html — сам файл (C# 9802)
    let page = "<html><body>Отчёт</body></html>".as_bytes();
    let server = fake(vec![(9802, reply(200, &[("Content-Type", "text/html; charset=utf-8")], page))]);
    let got = fetch_html(&client(&server.url), 9802);
    assert_eq!(got.done(), &Downloaded { bytes: page.len() as u64, length: Some(page.len() as u64), error: String::new() });
    assert_eq!(got.written, page);
}

#[test]
fn a_page_repeating_the_login_is_masked_and_logged_once() {
    // C# 9801: страница-трассировка повторила токен входа — в ошибке и журнале звёздочки; образец — один на метод («страница»)
    let page = format!("<html><body><table><tr><td>Authorization</td><td>{TOKEN}</td></tr></table></body></html>");
    let ok = [("Content-Type", "text/html")];
    let server = fake(vec![(9801, reply(200, &ok, page.as_bytes())), (9802, reply(200, &ok, page.as_bytes()))]);
    let (c, lines) = user_client(&server.url);
    let (first, second) = (fetch(&c, 9801).error(), fetch(&c, 9802).error());
    assert!(first.starts_with("сервер вернул страницу вместо файла:\n") && first.contains("***") && !first.contains(TOKEN), "{first}");
    assert!(!second.contains(TOKEN), "{second}");
    let logged = lines.lock().unwrap().clone();
    assert_eq!(logged.len(), 1, "один образец страницы за запуск: {logged:?}");
    assert!(logged[0].starts_with("download_file: страница вместо файла на api/taskfile/9801 ("), "{}", logged[0]);
    assert!(!logged[0].contains(TOKEN), "в журнале токена нет: {}", logged[0]);
}

#[test]
fn a_page_that_breaks_off_or_goes_silent_is_still_a_page() {
    // C# 9813, 9962: страница оборвалась или замолчала на середине — «страница», а не сетевой сбой (его бы повторяли)
    let routes = vec![
        (1, partial(200, "text/html", 100, b"<html><bo", None)),
        (2, partial(200, "text/html; charset=windows-1251", 40, &page_1251()[..10], Some(Step::Hold))),
    ];
    let server = fake(routes);
    let c = quiet(&server.url);
    for id in [1, 2] {
        let got = fetch(&c, id);
        let (done, error) = (got.done().clone(), got.error());
        assert!(error.starts_with("сервер вернул страницу вместо файла:\n"), "{id}: {error}");
        assert!(!error.contains("не ответил") && !error.contains("оборвалось"), "{id}: {error}");
        assert_eq!((done.bytes, got.written.len()), (0, 0), "{id}");
    }
    assert_eq!(server.requests(), 2);
}

#[test]
fn a_page_in_windows_1251_is_read() {
    // C# 9960: русская страница IIS в windows-1251 вместо файла — текст прочитан
    let server = fake(vec![(9960, reply(200, &[("Content-Type", "text/html; charset=windows-1251")], &page_1251()))]);
    let error = fetch(&client(&server.url), 9960).error();
    assert_eq!(error, "сервер вернул страницу вместо файла:\n(html-страница, показан её текст)\nДоступ запрещён");
}

#[test]
fn a_relative_redirect_names_only_its_path() {
    // C# `Redirected` → `final.AbsolutePath`: путь без запроса, без якоря и без хозяина, как бы ни прислал его сервер (строка
    // идёт в примечание файла заявки — в GitHub; ReturnUrl, sid, SAMLRequest и хозяин SSO там не нужны)
    let cases = [
        (9501, "/Account/Login?ReturnUrl=%2fapi%2ftaskfile%2f9501&sid=SECRET", "/Account/Login"),
        (9502, "//sso.corp.example/adfs/ls?SAMLRequest=abc", "/adfs/ls"),
        (9503, "/login?next=https://hd/api/taskfile/5", "/login"),
        (9504, "/login#frag", "/login"),
        // относительный — от папки запроса (Uri C#): …/api/taskfile/ + login
        (9505, "login?x=1", "/api/taskfile/login"),
    ];
    let server = fake(cases.iter().map(|(id, loc, _)| (*id, reply(302, &[("Location", loc)], b""))).collect());
    let c = client(&server.url);
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(id, loc, path)| {
            let want =
                format!("сервер переадресовал запрос файла на {path} — похоже, на страницу входа: файл по логину и паролю API не отдан");
            let got = fetch(&c, *id);
            (got.done().error != want).then(|| format!("{loc}: {}", got.done().error))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_redirect_needs_a_location_and_300_is_one() {
    // HttpClient идёт за 300, 301, 302, 303, 307, 308 — и только с Location; без него ответ остаётся кодом (C# `IsSuccessStatusCode`
    // ложно → `RefusalAsync` → «ошибка сервера»)
    let server = fake(vec![(9511, reply(302, &[], b"")), (9512, reply(300, &[("Location", "/login")], b"")), (9513, reply(300, &[], b""))]);
    let c = client(&server.url);
    assert_eq!(fetch(&c, 9511).done().error, "ошибка сервера (HTTP 302)");
    assert_eq!(
        fetch(&c, 9512).done().error,
        "сервер переадресовал запрос файла на /login — похоже, на страницу входа: файл по логину и паролю API не отдан"
    );
    assert_eq!(fetch(&c, 9513).done().error, "ошибка сервера (HTTP 300)");
}

/// Тело, что приходит по байту раз в 100 мс 5 секунд подряд.
fn trickle(code: u16, content_type: &str) -> Script {
    let mut script = vec![status(code, &[("Content-Type", content_type), ("Content-Length", "50")])];
    for _ in 0..50 {
        script.push(Step::Send(b"x".to_vec()));
        script.push(Step::Pause(100));
    }
    script
}

#[test]
fn a_slow_refusal_or_page_body_is_read_within_one_chunk_limit() {
    // C# `HeadAsync(…, start.Token)`, `start.CancelAfter(idleLimit)`: тело отказа и страницы — одно окно срока куска на всё,
    // а не срок на каждый байт: сервер, что цедит тело по байту, держал бы выгрузку часами
    let server = fake(vec![(9521, trickle(404, "text/plain")), (9522, trickle(200, "text/html"))]);
    let c = quiet(&server.url);
    for (id, head) in [(9521, "файл не найден (HTTP 404):\n"), (9522, "сервер вернул страницу вместо файла:\n")]
    {
        let started = Instant::now();
        let got = fetch(&c, id);
        let took = started.elapsed();
        assert!(got.done().error.starts_with(head), "{id}: {}", got.done().error);
        assert!(took < Duration::from_secs(2), "{id}: тело читалось {took:?} — окно 0,3 с, а не по байту");
    }
}

#[test]
fn stop_while_a_refusal_body_comes_ends_the_download() {
    // C#: тело отказа читается с `ct` — «Остановить» прерывает и его
    let server = fake(vec![(9531, trickle(404, "text/plain"))]);
    let c = limited(&server.url, Duration::from_secs(5), Duration::from_secs(5));
    let started = Instant::now();
    let stop = || started.elapsed() > Duration::from_millis(300);
    let got = fetch_with(&c, 9531, Sink::new(), &stop, u64::MAX, false);
    assert_eq!(got.result.as_ref().expect_err("остановили — Err").kind(), io::ErrorKind::Interrupted);
    assert!(started.elapsed() < Duration::from_secs(2), "остановка — сразу, а не по концу тела");
}

#[test]
fn a_relative_redirect_under_a_server_folder_is_from_that_folder_and_an_empty_location_is_no_redirect() {
    // Uri C#: относительный — от папки запроса, а адрес сервера бывает с папкой («http://host/IS»); пустой Location — не
    // переадресация (за ним не пойти): код ответа
    let server = fake(vec![(9551, reply(302, &[("Location", "login")], b"")), (9552, reply(302, &[("Location", "")], b""))]);
    let c = client(&format!("{}IS", server.url));
    assert_eq!(
        fetch(&c, 9551).done().error,
        "сервер переадресовал запрос файла на /IS/api/taskfile/login — похоже, на страницу входа: файл по логину и паролю API не отдан"
    );
    assert_eq!(fetch(&c, 9552).done().error, "ошибка сервера (HTTP 302)");
}

#[test]
fn stop_is_heard_while_a_chunk_or_a_refusal_body_is_awaited() {
    // C#: отмена `ct` прерывает и ожидание куска, и чтение тела отказа — сервер молчит, срок куска 5 с, а «Остановить» — сразу
    let server = fake(vec![
        (9561, partial(200, "application/octet-stream", 1000, &bytes_of(100), Some(Step::Hold))),
        (9562, partial(404, "text/plain", 1000, b"xx", Some(Step::Hold))),
    ]);
    let c = limited(&server.url, Duration::from_secs(5), Duration::from_secs(5));
    for id in [9561, 9562] {
        let started = Instant::now();
        let stop = || started.elapsed() > Duration::from_millis(200);
        let got = fetch_with(&c, id, Sink::new(), &stop, u64::MAX, false);
        assert_eq!(got.result.as_ref().map_err(|e| e.kind()).err(), Some(io::ErrorKind::Interrupted), "{id}");
        assert!(started.elapsed() < Duration::from_secs(2), "{id}: остановка услышана через {:?}", started.elapsed());
    }
}

#[test]
fn a_refusal_body_of_exactly_the_limit_is_whole_and_one_byte_more_is_cut() {
    // C# `HeadAsync`: на пределе (64 КБ) тело цело, если сразу за ним конец; байт сверх — «только начало» в errors.log
    let exact = vec![b'a'; 64 * 1024];
    let over = vec![b'a'; 64 * 1024 + 1];
    let server = fake(vec![
        (9571, reply(403, &[("Content-Type", "text/plain")], &exact)),
        (9572, reply(404, &[("Content-Type", "text/plain")], &over)),
    ]);
    let (c, lines) = user_client(&server.url);
    let _ = (fetch(&c, 9571), fetch(&c, 9572));
    let logged = lines.lock().unwrap().clone();
    assert_eq!(logged.len(), 2, "{logged:?}");
    assert!(logged[0].starts_with("download_file: HTTP 403") && !logged[0].contains("только начало"), "{}", &logged[0][..80]);
    assert!(logged[1].starts_with("download_file: HTTP 404") && logged[1].contains("только начало"), "{}", &logged[1][..80]);
}

#[test]
fn a_redirect_is_the_login_page_error_and_is_not_followed() {
    // переадресация (страница входа) — ошибка C# с путём из Location; прототип за ней не идёт (в C# — путь после всех переходов)
    let codes = [301, 302, 303, 307, 308];
    let routes: Vec<(i32, Script)> = codes.iter().map(|c| (*c, reply(*c as u16, &[("Location", "/login")], b""))).collect();
    let server = fake(routes);
    let c = client(&server.url);
    let want = "сервер переадресовал запрос файла на /login — похоже, на страницу входа: файл по логину и паролю API не отдан";
    let wrong: Vec<String> = codes
        .iter()
        .filter_map(|code| {
            let got = fetch(&c, *code);
            (got.done() != &Downloaded { bytes: 0, length: None, error: want.into() }).then(|| format!("{code}: {:?}", got.done()))
        })
        .collect();
    assert!(wrong.is_empty(), "ждали «{want}»:\n{}", wrong.join("\n"));
    assert_eq!(
        server.paths().iter().filter(|p| !p.starts_with("/api/taskfile/")).count(),
        0,
        "за переадресацией не ходили: {:?}",
        server.paths()
    );
    assert_eq!(server.requests(), codes.len());
}

#[test]
fn an_absolute_redirect_names_only_its_path() {
    // `final.AbsolutePath`: адрес целиком — только путь, без сервера и без запроса после «?»
    let elsewhere = fake(vec![]);
    let location = format!("{}Account/Login?ReturnUrl=%2Fapi", elsewhere.url);
    let server = fake(vec![(9803, reply(302, &[("Location", &location)], b""))]);
    let error = fetch(&client(&server.url), 9803).error();
    assert_eq!(
        error,
        "сервер переадресовал запрос файла на /Account/Login — похоже, на страницу входа: файл по логину и паролю API не отдан"
    );
    assert_eq!(elsewhere.requests(), 0, "на адрес из Location запроса не было");
    assert_eq!(server.requests(), 1);
}

// ---------------------------------------------------------------------------------------------------------------------
// Сеть: нет сервера, молчание, обрыв
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn an_unreachable_server_is_a_network_error_in_words() {
    let url = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/", listener.local_addr().unwrap())
    }; // слушатель закрыт — на этом порту никого
    let got = fetch(&client(&url), 9005);
    let done = got.done();
    assert!(
        done.error.starts_with("сервер недоступен:\n") && done.error.len() > "сервер недоступен:\n".len(),
        "причина — второй строкой: {done:?}"
    );
    assert_eq!((done.bytes, done.length), (0, None));
}

#[test]
fn headers_that_do_not_come_in_time_name_the_headers_limit() {
    // C# (последняя проверка): заголовки не пришли за 200 мс — «сервер не ответил за 0,2 с»; срок куска (здесь 5 с) ни при чём
    let server = fake(vec![(9967, vec![Step::Hold])]);
    let c = limited(&server.url, Duration::from_millis(200), Duration::from_secs(5));
    let got = fetch(&c, 9967);
    assert_eq!(got.done(), &Downloaded { bytes: 0, length: None, error: "сервер не ответил за 0,2 с".into() });
    assert_eq!(server.requests(), 1, "спрошено по разу, без повторов");
}

#[test]
fn the_two_limits_do_not_borrow_from_each_other() {
    // срок заголовков не касается тела, пришедшего после ответа, а срок куска — ожидания заголовков (C#: у каждого ожидания —
    // свой источник отмены)
    let late_headers = vec![Step::Pause(500), status(200, &[("Content-Length", "10")]), Step::Send(bytes_of(10))];
    let late_body = vec![status(200, &[("Content-Length", "10")]), Step::Pause(500), Step::Send(bytes_of(10))];
    let server = fake(vec![(1, late_headers), (2, late_body)]);
    // заголовки идут 500 мс при сроке куска 200 мс: ждёт их срок заголовков (3 с)
    let slow_headers = fetch(&limited(&server.url, Duration::from_secs(3), Duration::from_millis(200)), 1);
    assert_eq!(slow_headers.done(), &Downloaded { bytes: 10, length: Some(10), error: String::new() }, "заголовки — по своему сроку");
    // тело идёт через 500 мс после заголовков при сроке заголовков 200 мс: ждёт его срок куска (3 с)
    let slow_body = fetch(&limited(&server.url, Duration::from_millis(200), Duration::from_secs(3)), 2);
    assert_eq!(slow_body.done(), &Downloaded { bytes: 10, length: Some(10), error: String::new() }, "тело — по сроку куска");
}

#[test]
fn silence_in_the_middle_of_a_file_says_where_it_stopped() {
    // C# 9402 и 605: «сервер не ответил за {срок куска} с: файл перестал приходить на {размер}» — с тем, что успело прийти
    let server = fake(vec![
        (9402, partial(200, "application/octet-stream", 1000, &bytes_of(100), Some(Step::Hold))),
        (9403, partial(200, "application/octet-stream", 1000, b"", Some(Step::Hold))),
    ]);
    let c = quiet(&server.url);
    let mid = fetch(&c, 9402);
    assert_eq!(
        mid.done(),
        &Downloaded {
            bytes: 100, length: Some(1000), error: "сервер не ответил за 0,3 с: файл перестал приходить на 100 Б".into()
        }
    );
    assert_eq!(mid.written, bytes_of(100), "что пришло — в файле");
    // заголовки были, тела нет совсем: срок куска, «на 0 Б» (C#: body != null — заголовки пришли)
    let none = fetch(&c, 9403);
    assert_eq!(
        none.done(),
        &Downloaded {
            bytes: 0, length: Some(1000), error: "сервер не ответил за 0,3 с: файл перестал приходить на 0 Б".into()
        }
    );
    assert_eq!(server.requests(), 2, "спрошено по разу, без повторов");
}

#[test]
fn silence_after_a_slow_write_with_equal_limits_is_still_the_servers_silence() {
    // сроки заголовков и куска равны, как в программе (60 и 60): чтение следующего куска начинается, пока прежний пишется на
    // диск, — его срок истекает раньше, чем ожидание куска, и тишина приходит ошибкой чтения; текст — тот же, что у C#
    let server = fake(vec![(9404, partial(200, "application/octet-stream", 1000, &bytes_of(100), Some(Step::Hold)))]);
    let equal = limited(&server.url, Duration::from_millis(300), Duration::from_millis(300));
    let sink = Sink { first_write_takes: Duration::from_millis(200), ..Sink::new() };
    let got = fetch_with(&equal, 9404, sink, &|| false, u64::MAX, false);
    assert_eq!(
        got.done(),
        &Downloaded {
            bytes: 100, length: Some(1000), error: "сервер не ответил за 0,3 с: файл перестал приходить на 100 Б".into()
        }
    );
}

#[test]
fn a_break_in_the_middle_of_a_file_says_where_it_broke() {
    // C# 9401: «соединение оборвалось на {размер}:\n{причина}»; до первого байта — «соединение оборвалось:\n{причина}» без «на»
    let server = fake(vec![
        (9401, partial(200, "application/octet-stream", 1000, &bytes_of(100), None)),
        (9404, partial(200, "application/octet-stream", 1000, b"", None)),
    ]);
    let c = client(&server.url);
    let mid = fetch(&c, 9401);
    let done = mid.done();
    assert_eq!((done.bytes, done.length), (100, Some(1000)), "{done:?}");
    let reason = done
        .error
        .strip_prefix("соединение оборвалось на 100 Б:\n")
        .unwrap_or_else(|| panic!("ждали «соединение оборвалось на 100 Б:\\n…», вышло: {}", done.error));
    assert!(!reason.trim().is_empty(), "причина — второй строкой");
    assert_eq!(mid.written, bytes_of(100));
    let start = fetch(&c, 9404);
    let error = start.error();
    let reason =
        error.strip_prefix("соединение оборвалось:\n").unwrap_or_else(|| panic!("ждали «соединение оборвалось:\\n…», вышло: {error}"));
    assert!(!reason.trim().is_empty() && start.written.is_empty(), "{error}");
}
