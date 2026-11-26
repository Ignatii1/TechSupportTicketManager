//! Выгрузка скачивает вложения в базу знаний (карточка 4.4-2): строка итога о вложениях и строка хода с байтами — таблицами
//! сверки (`parity/export-files-note.tsv`, `parity/export-progress-bytes.tsv`, ответы пишет код C#), правило Git LFS в
//! `.gitattributes`, и сценарии самопроверки C# (`KnowledgeExport.SelfCheck.Files.cs`, 1–10 и блок после девятого) на
//! поддельном сервере с файлами — `FilesServer` оттуда: те же заявки и номера файлов, настоящий клиент против сокетов.
//! Не порт: наблюдатели авто-выгрузки (`Current`, `Errors`, `NotWritten` — этап 4.5), сервер с карточками в чужих
//! кодировках (`624`, `626`–`631`: чтение карточки, не вложений) и проба размера для панели (`stopAt`/`rangeTo` — панель).
//! Занятая подмена и файл, пропавший между списком и чтением, на Linux не устроить — их проверяют подмены внутри
//! `src/export/files.rs` (`MoveFile`, `FileList` у C#).

mod common;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use ticketboard::export::closed::of_ticket;
use ticketboard::export::files::{FILES_FOLDER, LFS_FOLDER, LFS_RULE, MAX_PATH_LENGTH, TICKETS_FOLDER, keep_large_in_lfs};
use ticketboard::export::run::{ExportResult, LARGE_FILE, Options, export_found, export_rows, files_note, progress_line, unchanged};
use ticketboard::export::{AttachmentScope, Context, FORMAT_VERSION, build};
use ticketboard::intraservice::query::TaskQuery;
use ticketboard::intraservice::{Client, Date, Found, User};
use ticketboard::rules::ClosedNames;

// =====================================================================================================================
// Таблицы сверки
// =====================================================================================================================

#[test]
fn the_files_note_reads_like_csharp_on_every_row() {
    // вход — «saved=;bytes=;present=;large=;skipped=;failed=;first=»: как его разбирает `ParityCheck` у C# (по «;», потом по
    // первому «=»; нет поля — 0 или пусто); ответ — строка итога или «(пусто)»
    let wrong = common::parity_mismatches("export-files-note", |c| {
        let fields: HashMap<&str, &str> =
            c[0].split(';').filter(|kv| !kv.is_empty()).map(|kv| kv.split_once('=').unwrap_or((kv, ""))).collect();
        let n = |key: &str| fields.get(key).map_or(0, |v| v.parse::<u64>().unwrap());
        let note = files_note(
            n("saved") as usize,
            n("bytes"),
            n("present") as usize,
            n("large") as usize,
            n("skipped") as usize,
            n("failed") as usize,
            fields.get("first").copied().unwrap_or(""),
        );
        if note.is_empty() { "(пусто)".to_string() } else { note }
    });
    assert!(wrong.is_empty(), "итог о вложениях расходится с C#:\n{}", wrong.join("\n"));
}

#[test]
fn the_progress_line_with_attachment_bytes_reads_like_csharp_on_every_row() {
    // легло, прошло, всего, не выгружено, байт вложений, скорость → строка хода; скорость отвечает одним числом на любой счёт
    let wrong = common::parity_mismatches("export-progress-bytes", |c| {
        let n = |i: usize| c[i].parse::<usize>().unwrap();
        let rate: f64 = c[5].parse().unwrap();
        progress_line(n(0), n(1), n(2), n(3), c[4].parse().unwrap(), |_| rate)
    });
    assert!(wrong.is_empty(), "строка хода с вложениями расходится с C#:\n{}", wrong.join("\n"));
}

// =====================================================================================================================
// Правило Git LFS в .gitattributes (`KeepLargeInLfs`)
// =====================================================================================================================

fn tmp(tag: &str) -> Tmp {
    Tmp::new(tag)
}

#[test]
fn the_lfs_rule_is_the_line_git_lfs_track_writes() {
    // `LfsRule` у C#, слово в слово: всё в files-lfs уходит через Git LFS
    assert_eq!(LFS_RULE, "files-lfs/** filter=lfs diff=lfs merge=lfs -text");
    assert_eq!((FILES_FOLDER, LFS_FOLDER), ("files", "files-lfs"));
}

#[test]
fn the_lfs_rule_is_written_into_a_new_gitattributes_once_however_many_times() {
    let dir = tmp("lfs-new");
    assert_eq!(keep_large_in_lfs(dir.path()), "", "дописано — причины нет");
    let file = dir.join(".gitattributes");
    assert_eq!(text(&file), format!("{LFS_RULE}\n"), "файла не было — создан с одной строкой");
    for _ in 0..3 {
        assert_eq!(keep_large_in_lfs(dir.path()), "", "строка уже есть — тоже пусто");
    }
    assert_eq!(text(&file), format!("{LFS_RULE}\n"), "сколько ни зови — одна строка");
}

#[test]
fn the_lfs_rule_goes_after_the_users_lines_and_a_missing_newline_is_added_first() {
    // C#: `AppendOnce` — «\n», если текст не кончался переводом строки, потом строка и «\n»; строки пользователя целы
    let dir = tmp("lfs-append");
    let file = dir.join(".gitattributes");
    fs::write(&file, "*.psd binary\n").unwrap();
    assert_eq!(keep_large_in_lfs(dir.path()), "");
    assert_eq!(text(&file), format!("*.psd binary\n{LFS_RULE}\n"));
    let other = tmp("lfs-no-newline");
    let file = other.join(".gitattributes");
    fs::write(&file, "*.psd binary").unwrap();
    assert_eq!(keep_large_in_lfs(other.path()), "");
    assert_eq!(text(&file), format!("*.psd binary\n{LFS_RULE}\n"), "конца строки не было — он добавлен перед правилом");
}

#[test]
fn a_rule_that_is_already_there_is_found_by_the_line_trimmed() {
    // C#: `l.Trim() == line` — пробелы и «\r» по краям строки не мешают
    let dir = tmp("lfs-present");
    let file = dir.join(".gitattributes");
    let before = format!("*.psd binary\r\n  {LFS_RULE}  \r\n");
    fs::write(&file, &before).unwrap();
    assert_eq!(keep_large_in_lfs(dir.path()), "");
    assert_eq!(text(&file), before, "правило есть — файл не тронут");
}

#[test]
fn a_gitattributes_that_cannot_be_written_says_why_and_names_the_file() {
    // C# 9: на месте файла папка — «не удалось дописать правило Git LFS в {путь}: {причина}»
    let dir = tmp("lfs-blocked");
    fs::create_dir_all(dir.join(".gitattributes")).unwrap();
    let why = keep_large_in_lfs(dir.path());
    let prefix = "не удалось дописать правило Git LFS в ";
    assert!(why.starts_with(prefix), "{why}");
    assert!(why[prefix.len()..].starts_with(&*dir.join(".gitattributes").to_string_lossy()), "в причине — путь файла: {why}");
    assert!(why.contains(": "), "после пути — причина ОС: {why}");
}

// =====================================================================================================================
// Папки, файлы, часы
// =====================================================================================================================

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("tb-att-{tag}-{}-{}", std::process::id(), N.fetch_add(1, SeqCst)));
        fs::create_dir_all(&path).unwrap();
        Tmp(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn join(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref()).unwrap_or_else(|e| format!("(не прочитать {}: {e})", path.as_ref().display()))
}

/// Все файлы под папкой — пути от неё, по порядку; нет папки — пусто.
fn walk(dir: &Path) -> Vec<String> {
    fn go(base: &Path, at: &Path, out: &mut Vec<String>) {
        let Ok(entries) = fs::read_dir(at) else { return };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                go(base, &path, out);
            } else {
                out.push(path.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    go(dir, dir, &mut out);
    out.sort();
    out
}

fn age(path: &Path, seconds: u64) {
    fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(SystemTime::now() - Duration::from_secs(seconds)).unwrap();
}

// =====================================================================================================================
// Поддельный сервер с файлами (`FilesServer` самопроверки C#)
// =====================================================================================================================

/// Срок куска у клиента: тишина посреди файла ловится за него; заголовки — с запасом, чтобы срок куска не путался с ними.
const IDLE: Duration = Duration::from_millis(500);
/// Порог крупного файла в проверках: 9003 — ровно 1500 байт, порог включительно (`LfsFrom = 1500`).
const LFS_FROM: u64 = 1500;

#[derive(Clone)]
struct Card {
    name: String,
    executors: &'static str,
    ids: Option<String>,
    names: String,
    changed: NaiveDateTime,
    observers: Option<&'static str>,
}

fn at(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").unwrap()
}

fn card(name: &str, executors: &'static str, ids: Option<&str>, names: &str, changed: &str) -> Card {
    Card { name: name.into(), executors, ids: ids.map(String::from), names: names.into(), changed: at(changed), observers: None }
}

fn zone() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).unwrap()
}

/// Ответ на запрос файла: заголовки с полной длиной, из тела — `cut` байт (дальше — обрыв, а с `hold` — молчание, пока клиент не
/// уйдёт или не истечёт `hold`); `sent` зовётся, как только отданное ушло.
struct FileReply {
    code: u16,
    content_type: &'static str,
    body: Vec<u8>,
    cut: Option<usize>,
    hold: Option<Duration>,
    location: Option<&'static str>,
    sent: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl FileReply {
    fn new(body: Vec<u8>) -> FileReply {
        FileReply { code: 200, content_type: "application/octet-stream", body, cut: None, hold: None, location: None, sent: None }
    }
    fn page(body: Vec<u8>, content_type: &'static str) -> FileReply {
        FileReply { content_type, ..FileReply::new(body) }
    }
}

type Hook = Arc<dyn Fn() + Send + Sync>;

struct Shared {
    cards: Mutex<BTreeMap<i32, Card>>,
    bodies: HashMap<i32, Vec<u8>>,
    /// Файлы, которых нет на сервере (404).
    missing: Mutex<HashSet<i32>>,
    /// Сколько раз ответить на файл 503 (`FailOnce`).
    fail_once: Mutex<HashMap<i32, u32>>,
    /// Какие файлы спрашивали, по порядку (`Asked`).
    asked: Mutex<Vec<i32>>,
    /// Все пути запросов по порядку.
    paths: Mutex<Vec<String>>,
    /// Было ли правило LFS в `.gitattributes`, когда спросили крупный 9003 (None — не спрашивали).
    rule_before_large: Mutex<Option<bool>>,
    attributes: Mutex<Option<PathBuf>>,
    /// «Антивирус» убирает файл, когда спросили 9907.
    vanish: Mutex<Option<Hook>>,
    /// Что сделать, когда отдано начало долгого файла 9501 (`OnSent`).
    on_sent: Mutex<Option<Hook>>,
    /// Крупный 9003 приходит частями с паузой: дочитать его до конца — дождаться молчания.
    large_stalls: AtomicBool,
    /// Номера заявок в списке «найденного» (`/api/task?`).
    listed: Mutex<Vec<i32>>,
}

struct FilesServer {
    shared: Arc<Shared>,
    url: String,
}

fn body_of(id: i32) -> Vec<u8> {
    let len = match id {
        9001 => 300,
        9002 => 500,
        9003 => 1500,
        // больше куска чтения (80 КБ): приходит несколькими
        9990 => 300_000,
        _ => 600,
    };
    (0..len).map(|i| ((i * 7 + id as usize) % 251) as u8).collect()
}

/// «<html><body>Доступ запрещён</body></html>» в windows-1251 — байтами (как `Page1251` самопроверки C#).
fn page_1251() -> Vec<u8> {
    let mut page = b"<html><body>".to_vec();
    page.extend([0xC4, 0xEE, 0xF1, 0xF2, 0xF3, 0xEF, 0x20, 0xE7, 0xE0, 0xEF, 0xF0, 0xE5, 0xF9, 0xB8, 0xED]);
    page.extend(b"</body></html>");
    page
}

impl Shared {
    fn file_reply(&self, id: i32) -> Result<FileReply, (u16, &'static str)> {
        self.asked.lock().unwrap().push(id);
        if self.missing.lock().unwrap().contains(&id) || id == 9920 {
            return Err((404, r#"{"Message":"not found"}"#));
        }
        if id == 9910 {
            return Err((401, r#"{"Message":"Authorization has been denied for this request."}"#));
        }
        {
            let mut once = self.fail_once.lock().unwrap();
            if let Some(left) = once.get_mut(&id).filter(|n| **n > 0) {
                *left -= 1;
                return Err((503, r#"{"Message":"busy"}"#));
            }
        }
        if id == 9003 {
            let attributes = self.attributes.lock().unwrap().clone();
            *self.rule_before_large.lock().unwrap() = attributes.map(|p| fs::read_to_string(p).is_ok_and(|t| t.contains(LFS_RULE)));
        }
        if id == 9907 {
            let vanish = self.vanish.lock().unwrap().clone();
            if let Some(vanish) = vanish {
                vanish(); // первый файл заявки уже лёг — «антивирус» убирает его
            }
        }
        let long = Duration::from_secs(3);
        let reply = match id {
            9003 if self.large_stalls.load(SeqCst) => FileReply { cut: Some(1600), hold: Some(long), ..FileReply::new(vec![0; 2000]) },
            9401 => FileReply { cut: Some(100), ..FileReply::new(body_of(id)) },
            9801 => FileReply::page(
                b"<html><body><table><tr><td>Authorization</td><td>dXNlcjpwYXNz</td></tr></table></body></html>".to_vec(),
                "text/html; charset=utf-8",
            ),
            9802 | 9811 => FileReply::page(body_of(id), "text/html; charset=utf-8"),
            9813 => FileReply { cut: Some(10), ..FileReply::page(body_of(id), "text/html") },
            9965 => {
                FileReply { code: 403, cut: Some(10), hold: Some(long), ..FileReply::page(page_1251(), "text/html; charset=windows-1251") }
            }
            9962 => FileReply { cut: Some(10), hold: Some(long), ..FileReply::page(page_1251(), "text/html; charset=windows-1251") },
            9803 => FileReply { code: 302, location: Some("/login"), ..FileReply::new(Vec::new()) },
            9402 => FileReply { cut: Some(100), hold: Some(long), ..FileReply::new(body_of(id)) },
            9501 => {
                FileReply { cut: Some(100), hold: Some(long), sent: self.on_sent.lock().unwrap().clone(), ..FileReply::new(body_of(id)) }
            }
            _ => FileReply::new(self.bodies.get(&id).cloned().unwrap_or_else(|| body_of(id))),
        };
        Ok(reply)
    }

    fn card_json(&self, id: i32) -> Option<String> {
        let c = self.cards.lock().unwrap().get(&id)?.clone();
        let mut task = format!(
            r#""Id":{id},"Name":{},"StatusName":"Выполнена","ServiceName":"Почта","Type":"Инцидент""#,
            serde_json::to_string(&c.name).unwrap()
        );
        if let Some(ids) = &c.ids {
            task.push_str(&format!(r#","FileIds":"{ids}","FileNames":{}"#, serde_json::to_string(&c.names).unwrap()));
        }
        if let Some(observers) = c.observers {
            task.push_str(&format!(r#","Observers":"{observers}""#));
        }
        Some(format!(r#"{{"Task":{{{task}}}}}"#))
    }

    fn row_of(&self, id: i32) -> Found {
        let c = self.cards.lock().unwrap()[&id].clone();
        Found {
            id,
            name: c.name,
            status: "Выполнена".into(),
            creator: Some("Петрова".into()),
            created: Some(zone().with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap()),
            description: Some("описание".into()),
            executors: Some(c.executors.into()),
            changed: Some(zone().from_local_datetime(&c.changed).unwrap()),
            ..Found::default()
        }
    }

    fn list_json(&self, path: &str) -> String {
        let page: usize = path.split("page=").nth(1).and_then(|v| v.split('&').next()).and_then(|v| v.parse().ok()).unwrap_or(1);
        let ids = self.listed.lock().unwrap().clone();
        let rows: Vec<String> = if page == 1 {
            ids.iter()
                .map(|&id| {
                    let r = self.row_of(id);
                    format!(
                        r#"{{"Id":{id},"Name":{},"StatusId":29,"Created":"{}","Changed":"{}","Creator":"Петрова","Executors":"{}","Description":"описание"}}"#,
                        serde_json::to_string(&r.name).unwrap(),
                        r.created.unwrap().to_rfc3339(),
                        r.changed.unwrap().to_rfc3339(),
                        r.executors.unwrap()
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        format!(
            r#"{{"Tasks":[{}],"Statuses":[{{"Id":29,"Name":"Выполнена"}}],"Paginator":{{"Count":{},"Page":{page},"PageCount":1,"PageSize":25,"CountOnPage":{},"HasNextPage":false}}}}"#,
            rows.join(","),
            ids.len(),
            rows.len()
        )
    }
}

fn respond(stream: &mut TcpStream, code: u16, content_type: &str, extra: &str, declared: usize) {
    let _ = write!(
        stream,
        "HTTP/1.1 {code} Status\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {declared}\r\nConnection: close\r\n\r\n"
    );
}

fn serve_one(mut stream: TcpStream, shared: &Shared) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split(' ').nth(1).unwrap_or("").to_string();
    shared.paths.lock().unwrap().push(path.clone());
    let number = |prefix: &str| path.strip_prefix(prefix).and_then(|n| n.split('?').next()).and_then(|n| n.parse::<i32>().ok());
    let json = |stream: &mut TcpStream, code: u16, body: &str| {
        respond(stream, code, "application/json; charset=utf-8", "", body.len());
        let _ = stream.write_all(body.as_bytes());
    };
    if let Some(id) = number("/api/taskfile/") {
        match shared.file_reply(id) {
            Err((code, body)) => json(&mut stream, code, body),
            Ok(reply) => {
                let location = reply.location.map(|l| format!("Location: {l}\r\n")).unwrap_or_default();
                respond(&mut stream, reply.code, reply.content_type, &location, reply.body.len());
                let sent = reply.cut.unwrap_or(reply.body.len()).min(reply.body.len());
                if stream.write_all(&reply.body[..sent]).and_then(|()| stream.flush()).is_err() {
                    return;
                }
                if let Some(sent) = reply.sent {
                    sent();
                }
                if let Some(hold) = reply.hold {
                    let _ = stream.set_read_timeout(Some(hold));
                    let mut sink = [0u8; 64];
                    while stream.read(&mut sink).is_ok_and(|n| n > 0) {}
                }
            }
        }
    } else if let Some(id) = number("/api/task/") {
        match shared.card_json(id) {
            Some(body) => json(&mut stream, 200, &body),
            None => json(&mut stream, 404, r#"{"Message":"not found"}"#),
        }
    } else if path.starts_with("/api/tasklifetime?") {
        json(
            &mut stream,
            200,
            r#"{"TaskLifetimes":[{"Date":"2026-09-01T10:00:00+03:00","Editor":"Петров","StatusId":29,"Comments":"готово","IsPublic":true}],"Statuses":[{"Id":29,"Name":"Выполнена","IsFixed":true}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"#,
        );
    } else if path.starts_with("/api/task?") {
        json(&mut stream, 200, &shared.list_json(&path));
    } else {
        json(&mut stream, 404, r#"{"Message":"not found"}"#);
    }
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

impl FilesServer {
    fn new() -> FilesServer {
        let cards = BTreeMap::from([
            (
                601,
                card(
                    "Скриншот ошибки",
                    "Я Сам",
                    Some("9001, 9002, 9003"),
                    "ошибка.png||Акт: сверка.pdf||большой.zip",
                    "2026-09-05T10:00:00.250000",
                ),
            ),
            (602, card("Чужая заявка", "Петров", Some("9101"), "чужое.docx", "2026-09-06T10:00:00")),
            (603, card("Битое вложение", "Я Сам", Some("9201"), "нет на сервере.txt", "2026-09-07T10:00:00")),
            (604, card("Журнал", "Я Сам", Some("9301"), "журнал.txt", "2026-09-08T10:00:00")),
            (605, card("Сбои посреди файла", "Я Сам", Some("9401, 9402"), "обрыв.bin||тишина.bin", "2026-09-09T10:00:00")),
            (606, card("Долгий файл", "Я Сам", Some("9501"), "долгий.bin", "2026-09-10T10:00:00")),
            (607, card("Длинное имя", "Я Сам", Some("9601"), &format!("{}.png", "я".repeat(120)), "2026-09-11T10:00:00")),
            (608, card("Папка не пишется", "Я Сам", Some("9701"), "папка.txt", "2026-09-12T10:00:00")),
            (609, card("Место занято", "Я Сам", Some("9702"), "блок.txt", "2026-09-13T10:00:00")),
            (
                610,
                card(
                    "Страница вместо файла",
                    "Я Сам",
                    Some("9801, 9802, 9803"),
                    "снимок.png||страница.html||вход.png",
                    "2026-09-14T10:00:00",
                ),
            ),
            (611, card("Файл не пускает", "Я Сам", Some("9910"), "закрыт.pdf", "2026-09-15T10:00:00")),
            (612, card("Призрак в папке", "Я Сам", Some("9930"), "призрак.txt", "2026-09-18T10:00:00")),
            (613, card("Без имён", "Я Сам", Some("9811, 9812"), "страница.html", "2026-09-16T10:00:00")),
            (614, card("Антивирус", "Я Сам", Some("9906, 9907"), "вирус.exe||после.txt", "2026-09-17T10:00:00")),
            (615, card("Имя со «сбоем»", "Я Сам", Some("9920"), "скан (HTTP 503).pdf", "2026-09-19T10:00:00")),
            (617, card("Обрыв страницы", "Я Сам", Some("9813"), "обрезанная.png", "2026-09-21T10:00:00")),
            (622, card("Страница молчит", "Я Сам", Some("9962"), "схема.png", "2026-09-26T10:00:00")),
            (625, card("Отказ молчит", "Я Сам", Some("9965"), "схема.png", "2026-09-29T10:00:00")),
            (
                632,
                Card {
                    observers: Some("Иванов, Я Сам"),
                    ..card("Наблюдаю", "Петров", Some("9980"), "схема наблюдателя.png", "2026-09-30T16:00:00")
                },
            ),
        ]);
        let ids = [
            9001, 9002, 9003, 9101, 9201, 9301, 9401, 9402, 9501, 9601, 9701, 9702, 9801, 9802, 9803, 9811, 9812, 9813, 9906, 9907, 9930,
            9980,
        ];
        let shared = Arc::new(Shared {
            cards: Mutex::new(cards),
            bodies: ids.iter().map(|&id| (id, body_of(id))).collect(),
            missing: Mutex::new(HashSet::from([9201])),
            fail_once: Mutex::new(HashMap::new()),
            asked: Mutex::new(Vec::new()),
            paths: Mutex::new(Vec::new()),
            rule_before_large: Mutex::new(None),
            attributes: Mutex::new(None),
            vanish: Mutex::new(None),
            on_sent: Mutex::new(None),
            large_stalls: AtomicBool::new(false),
            listed: Mutex::new(Vec::new()),
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let inner = shared.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let shared = inner.clone();
                thread::spawn(move || serve_one(stream, &shared));
            }
        });
        FilesServer { shared, url }
    }

    fn client(&self) -> Client {
        Client::new(&self.url, "user", "pass", Duration::from_secs(10)).with_file_timeouts(Duration::from_secs(5), IDLE)
    }

    fn row(&self, id: i32) -> Found {
        self.shared.row_of(id)
    }

    /// Заявка изменилась (на минуту позже): её карточку читают заново (`Touch`).
    fn touch(&self, id: i32) {
        self.shared.cards.lock().unwrap().get_mut(&id).unwrap().changed += chrono::Duration::minutes(1);
    }

    fn edit(&self, id: i32, change: impl FnOnce(&mut Card)) {
        change(self.shared.cards.lock().unwrap().get_mut(&id).unwrap());
    }

    fn asked(&self) -> Vec<i32> {
        self.shared.asked.lock().unwrap().clone()
    }

    fn asked_times(&self, id: i32) -> usize {
        self.asked().iter().filter(|&&a| a == id).count()
    }

    fn forget(&self) {
        self.shared.asked.lock().unwrap().clear();
        self.shared.paths.lock().unwrap().clear();
    }

    /// Запросы файлов (`/api/taskfile/…`) — по числу, не по номерам.
    fn file_requests(&self) -> usize {
        self.shared.paths.lock().unwrap().iter().filter(|p| p.starts_with("/api/taskfile/")).count()
    }
}

// =====================================================================================================================
// Запуск выгрузки
// =====================================================================================================================

/// Папка, сервер и всё, что нужно запускам: «моя» — «Я Сам» (номер 7), `.gitattributes` пользователя — со своей строкой.
struct Env {
    server: FilesServer,
    dir: Tmp,
}

impl Env {
    /// Папка выгрузки с `.gitattributes` пользователя («строки пользователя»).
    fn new(tag: &str) -> Env {
        let env = Env { server: FilesServer::new(), dir: tmp(tag) };
        fs::write(env.dir.join(".gitattributes"), "*.psd binary\n").unwrap();
        *env.server.shared.attributes.lock().unwrap() = Some(env.dir.join(".gitattributes"));
        env
    }

    fn run(&self, scope: AttachmentScope, ids: &[i32]) -> ExportResult {
        self.run_in(self.dir.path(), scope, ids)
    }

    fn run_in(&self, dir: &Path, scope: AttachmentScope, ids: &[i32]) -> ExportResult {
        let rows: Vec<Found> = ids.iter().map(|&id| self.server.row(id)).collect();
        export_rows(&self.server.client(), &rows, dir, &url_of, &context(scope), &|_| {}, &AtomicBool::new(false), &fast())
    }

    fn mine(&self, ids: &[i32]) -> ExportResult {
        self.run(AttachmentScope::Mine, ids)
    }

    fn plain(&self, ticket: i32, name: &str) -> PathBuf {
        self.dir.join(&format!("{FILES_FOLDER}/2026-09/{ticket}/{name}"))
    }

    fn large(&self, ticket: i32, name: &str) -> PathBuf {
        self.dir.join(&format!("{LFS_FOLDER}/2026-09/{ticket}/{name}"))
    }

    /// Файл заявки (текст) в папке месяца.
    fn md(&self, ticket: i32) -> String {
        match self.md_path(ticket) {
            Some(path) => text(path),
            None => format!("(файла заявки {ticket} нет; в папке: {:?})", walk(self.dir.path())),
        }
    }

    fn md_path(&self, ticket: i32) -> Option<PathBuf> {
        let folder = self.dir.join(&format!("{TICKETS_FOLDER}/2026-09"));
        let mut names: Vec<String> =
            fs::read_dir(folder.clone()).ok()?.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names.into_iter().find(|n| n.starts_with(&format!("{ticket} — ")) && n.ends_with(".md")).map(|n| folder.join(n))
    }

    /// Файл лежит и совпадает с тем, что сервер отдаёт под этим номером, — побайтно.
    fn same(&self, path: &Path, file: i32) -> bool {
        fs::read(path).is_ok_and(|bytes| bytes == body_of(file))
    }

    /// Временные закачки в files/ и files-lfs/.
    fn temps(&self) -> Vec<String> {
        walk(self.dir.path()).into_iter().filter(|p| p.ends_with(".tmp") && !p.starts_with(TICKETS_FOLDER)).collect()
    }

    fn tree(&self) -> Vec<String> {
        walk(self.dir.path())
    }

    /// Первый заход (C# 1): 601, 602, 603, «мои» — три файла 601, чужая 602, битая 603.
    fn first() -> (Env, ExportResult) {
        let env = Env::new("first");
        let r = env.mine(&[601, 602, 603]);
        (env, r)
    }
}

fn context(scope: AttachmentScope) -> Context {
    Context { me: Some(User { id: 7, name: "Я Сам".into() }), closed: ClosedNames::default(), attachments: scope }
}

/// Повторов без пауз; крупный — от 1500 байт; проверка «на месте ли» — сразу.
fn fast() -> Options {
    Options { retry_delays: vec![Duration::ZERO; 3], vanish_wait: Duration::ZERO, lfs_from: LFS_FROM, ..Options::default() }
}

fn url_of(id: i32) -> String {
    format!("https://hd/Task/View/{id}")
}

fn counts(r: &ExportResult) -> (usize, usize, usize, usize, usize) {
    (r.found, r.created, r.updated, r.unchanged, r.failed)
}

fn show(r: &ExportResult) -> String {
    format!(
        "найдено {}, новых {}, обновлено {}, без изменений {}, не выгружено {}, закончено {}\nпервая ошибка: «{}»\nитог: «{}»",
        r.found, r.created, r.updated, r.unchanged, r.failed, r.complete, r.first_error, r.error
    )
}

// =====================================================================================================================
// 1. Первый заход
// =====================================================================================================================

#[test]
fn the_first_export_puts_my_files_byte_for_byte_on_their_places_and_leaves_no_temp() {
    // C# 1: у моей 601 — три файла побайтно, крупный (ровно 1500 байт — порог включительно) — в files-lfs, не в files
    let (env, _) = Env::first();
    let tree = env.tree();
    assert!(env.same(&env.plain(601, "9001 — ошибка.png"), 9001), "9001 побайтно: {tree:?}");
    assert!(env.same(&env.plain(601, "9002 — Акт сверка.pdf"), 9002), "9002 побайтно (двоеточие из имени ушло): {tree:?}");
    assert!(env.same(&env.large(601, "9003 — большой.zip"), 9003), "9003 в files-lfs побайтно: {tree:?}");
    assert!(!env.plain(601, "9003 — большой.zip").exists(), "крупного нет в files: {tree:?}");
    assert!(env.temps().is_empty(), "временных не осталось: {:?}", env.temps());
}

#[test]
fn the_lfs_rule_is_in_gitattributes_before_the_large_file_is_asked_and_only_once() {
    // C# 1: правило записано раньше, чем запрошен крупный файл; строки пользователя целы, правило — одна строка в конце
    let (env, _) = Env::first();
    assert_eq!(*env.server.shared.rule_before_large.lock().unwrap(), Some(true), "правило LFS — до запроса крупного файла");
    assert_eq!(text(env.dir.join(".gitattributes")), format!("*.psd binary\n{LFS_RULE}\n"));
}

#[test]
fn a_foreign_ticket_is_not_downloaded_and_says_how_many_of_its_files_are_missing() {
    // C# 1: 602 чужая («моих заявок», роли нет) — файл не качаем; в свойствах — сколько нет в базе; у 601 — все на месте
    let (env, _) = Env::first();
    assert!(!env.server.asked().contains(&9101), "чужой файл не спрашивали: {:?}", env.server.asked());
    let md = env.md(602);
    assert!(md.contains("\n- чужое.docx · не скачано\n"), "{md}");
    assert!(md.contains("\nfiles_missing: 1\n"), "{md}");
    assert!(!env.md(601).contains("files_missing"), "у 601 все файлы скачаны — пропавших нет");
}

#[test]
fn the_ticket_file_links_the_downloaded_files_with_their_sizes() {
    // C# 1: раздел «Вложения» — ссылки с размерами (картинка — вложением «![[…]]», крупный — в files-lfs), files: 3, revision есть
    let (env, _) = Env::first();
    let md = env.md(601);
    assert!(
        md.contains(
            "\n\n## Вложения\n\n- ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б\n- [[files/2026-09/601/9002 — Акт сверка.pdf]] · 500 Б\n- [[files-lfs/2026-09/601/9003 — большой.zip]] · 1,5 КБ\n\n## Переписка"
        ),
        "{md}"
    );
    assert!(md.contains("\nfiles: 3\n"), "{md}");
    assert!(md.contains("\nrevision: "), "{md}");
    assert!(md.contains(&format!("\nformat: {FORMAT_VERSION}\n")), "{md}");
}

#[test]
fn a_file_the_server_does_not_have_is_failed_in_the_ticket_without_a_revision() {
    // C# 1: 603 — файла на сервере нет: файл заявки записан без revision (следующая выгрузка перечитает), строка «не скачалось»;
    // заявка сама посчитана записанной, выгрузка не считает её сбойной и идёт дальше
    let (env, r) = Env::first();
    let md = env.md(603);
    assert!(!md.contains("\nrevision: "), "{md}");
    assert!(md.contains("\n- нет на сервере.txt · не скачалось: файл не найден (HTTP 404)\n"), "{md}");
    assert_eq!((counts(&r), r.complete), ((3, 3, 0, 0, 0), true), "{}", show(&r));
}

#[test]
fn the_summary_names_what_was_downloaded_what_was_skipped_what_failed_and_how_big_the_base_is() {
    // C# 1: слово в слово (скачано 3 = 300 + 500 + 1500 Б; не скачано — чужая 602; не скачалось — 603) и объём вложений в базе
    let (_, r) = Env::first();
    let files_line = "Вложения: скачано 3 (2,2 КБ), крупных — в files-lfs (Git LFS): 1, не скачано (настройка «Вложения в базу знаний»): 1; \
                      не скачались 1 — повторная выгрузка попробует их снова: #603 «нет на сервере.txt»: файл не найден (HTTP 404).";
    let volume_line = "Вложения в базе: files — 800 Б, files-lfs — 1,5 КБ.";
    assert!(r.error.contains(files_line), "{}", show(&r));
    assert!(r.error.contains(volume_line), "{}", show(&r));
    let lines: Vec<&str> = r.error.lines().collect();
    let at = lines.iter().position(|l| *l == files_line).expect("строка о вложениях отдельной строкой");
    assert_eq!(lines.get(at + 1), Some(&volume_line), "объём — следом за итогом о вложениях");
}

#[test]
fn the_progress_line_counts_the_bytes_that_came() {
    // C# 1: `progress.Any(line => line.Contains(" · вложения "))`; здесь — и последняя строка хода слово в слово: пришло 300 + 500 +
    // 1500 байт. Проверку «на месте ли» ждём секунду — на ней ход говорит в последний раз, без отсечки четырёх в секунду
    let env = Env::new("progress");
    let lines = Mutex::new(Vec::<String>::new());
    let options = Options { vanish_wait: Duration::from_secs(1), ..fast() };
    let rows: Vec<Found> = [601, 602, 603].iter().map(|&id| env.server.row(id)).collect();
    let r = export_rows(
        &env.server.client(),
        &rows,
        env.dir.path(),
        &url_of,
        &context(AttachmentScope::Mine),
        &|line| lines.lock().unwrap().push(line),
        &AtomicBool::new(false),
        &options,
    );
    assert_eq!(counts(&r), (3, 3, 0, 0, 0), "{}", show(&r));
    let lines = lines.lock().unwrap();
    assert!(lines.iter().any(|l| l.contains(" · вложения ")), "{lines:?}");
    let last = lines.iter().rev().find(|l| !l.starts_with("Собираю таблицу заявок"));
    assert_eq!(last.map(String::as_str), Some("Выгружено 3 из 3 · вложения 2,2 КБ"), "{lines:?}");
}

// =====================================================================================================================
// 2. Повторные заходы
// =====================================================================================================================

/// Состояние после второго захода (C# 2): файл 9201 появился на сервере.
fn second(env: &Env) -> ExportResult {
    env.server.shared.missing.lock().unwrap().remove(&9201);
    env.server.forget();
    env.mine(&[601, 602, 603])
}

#[test]
fn a_file_that_appeared_on_the_server_rereads_only_its_ticket() {
    // C# 2: перечитана только 603 (без revision) — файл скачан, остальные не тронуты и ни одного другого запроса файла
    let (env, _) = Env::first();
    let r = second(&env);
    assert_eq!((r.unchanged, r.updated), (2, 1), "{}", show(&r));
    assert_eq!(env.server.asked(), vec![9201], "спрошен только появившийся файл");
    assert!(env.md(603).contains("\nrevision: "), "{}", env.md(603));
    assert!(env.same(&env.plain(603, "9201 — нет на сервере.txt"), 9201), "файл 603 на месте: {:?}", env.tree());
}

#[test]
fn an_export_with_nothing_new_asks_for_no_file_and_does_not_touch_gitattributes() {
    // C# 2: третий заход — всё неизменно, запросов файлов нет, правило не дописано второй раз
    let (env, _) = Env::first();
    second(&env);
    env.server.forget();
    let attributes = text(env.dir.join(".gitattributes"));
    let r = env.mine(&[601, 602, 603]);
    assert_eq!(counts(&r), (3, 0, 0, 3, 0), "{}", show(&r));
    assert_eq!(env.server.file_requests(), 0, "{:?}", env.server.asked());
    assert_eq!(text(env.dir.join(".gitattributes")), attributes);
    assert_eq!(attributes, format!("*.psd binary\n{LFS_RULE}\n"), "правило одно, сколько ни выгружай");
}

#[test]
fn a_changed_ticket_with_the_same_files_is_reread_and_downloads_nothing() {
    // C# 3: 601 изменилась, вложения те же — карточка перечитана, файлы не качаются, ссылки на месте; оборванная закачка
    // старше десяти минут убрана, свежая (может, её ещё пишут) — нет
    let (env, _) = Env::first();
    assert!(env.same(&env.plain(601, "9001 — ошибка.png"), 9001), "до правки файл скачан: {:?}", env.tree());
    env.server.touch(601);
    env.server.forget();
    let (stale, fresh) = (env.plain(601, "9001 — ошибка.png.ab12.tmp"), env.plain(601, "9001 — ошибка.png.cd34.tmp"));
    fs::write(&stale, "").unwrap();
    age(&stale, 20 * 60);
    fs::write(&fresh, "").unwrap();
    let r = env.mine(&[601]);
    assert_eq!(r.updated, 1, "{}", show(&r));
    assert_eq!(env.server.file_requests(), 0, "файлы не качались: {:?}", env.server.asked());
    assert!(env.md(601).contains("[[files-lfs/2026-09/601/9003 — большой.zip]]"), "{}", env.md(601));
    assert!(!stale.exists() && fresh.exists(), "старая закачка убрана, свежая — нет");
}

#[test]
fn a_removed_attachment_is_deleted_and_a_renamed_one_is_renamed_not_downloaded_again() {
    // C# 4: 9002 убрали с заявки — файл удалён, файл пользователя рядом цел; 9001 лежит под прежним именем — переименован
    let (env, _) = Env::first();
    assert!(env.same(&env.plain(601, "9001 — ошибка.png"), 9001), "до правки файл скачан: {:?}", env.tree());
    fs::write(env.plain(601, "мои заметки.txt"), "моё").unwrap();
    fs::rename(env.plain(601, "9001 — ошибка.png"), env.plain(601, "9001 — старое имя.png")).unwrap();
    env.server.edit(601, |c| {
        c.ids = Some("9001, 9003".into());
        c.names = "ошибка.png||большой.zip".into();
    });
    env.server.touch(601);
    env.server.forget();
    env.mine(&[601]);
    assert!(!env.plain(601, "9002 — Акт сверка.pdf").exists(), "убранный файл удалён: {:?}", env.tree());
    assert_eq!(text(env.plain(601, "мои заметки.txt")), "моё", "файл пользователя цел");
    assert!(env.plain(601, "9001 — ошибка.png").exists() && !env.plain(601, "9001 — старое имя.png").exists(), "переименован");
    assert!(env.large(601, "9003 — большой.zip").exists());
    assert_eq!(env.server.file_requests(), 0, "ничего не качали: {:?}", env.server.asked());
    assert!(!env.md(601).contains("9002"), "{}", env.md(601));
}

#[test]
fn a_card_that_sent_no_file_numbers_deletes_nothing() {
    // C# 4: карточка не прислала FileIds — ничего не удаляем
    let (env, _) = Env::first();
    assert!(env.same(&env.plain(601, "9001 — ошибка.png"), 9001), "до правки файл скачан: {:?}", env.tree());
    env.server.edit(601, |c| c.ids = None);
    env.server.touch(601);
    env.mine(&[601]);
    assert!(env.plain(601, "9001 — ошибка.png").exists() && env.large(601, "9003 — большой.zip").exists(), "{:?}", env.tree());
}

#[test]
fn an_extra_copy_of_a_downloaded_file_is_removed_and_the_current_one_stays() {
    // C# 4: лишняя копия того же вложения (руками, прежним именем) — убрана, нынешняя цела, ничего не качали
    let (env, _) = Env::first();
    let current = env.plain(601, "9001 — ошибка.png");
    assert!(env.same(&current, 9001), "до правки файл скачан: {:?}", env.tree());
    fs::copy(&current, env.plain(601, "9001 — копия.png")).unwrap();
    env.server.touch(601);
    env.server.forget();
    env.mine(&[601]);
    assert!(!env.plain(601, "9001 — копия.png").exists(), "{:?}", env.tree());
    assert!(env.same(&current, 9001), "нынешняя цела");
    assert_eq!(env.server.file_requests(), 0);
}

#[test]
fn large_starts_at_fifty_megabytes_by_default() {
    // C# `LargeFile = 50L * 1024 * 1024` — его называет SCHEMA.md; проверки ставят свой порог, а выгрузка — этот
    assert_eq!(LARGE_FILE, 52_428_800);
    assert_eq!(Options::default().lfs_from, LARGE_FILE);
}

#[test]
fn the_bytes_of_a_file_that_came_in_pieces_are_counted_once_in_the_progress() {
    // C# `Received(n - seen)`: ход складывает приращения — файл, пришедший кусками, в «· вложения N» не считается дважды
    let env = Env::new("pieces");
    env.server.shared.cards.lock().unwrap().insert(606, card("Большой отчёт", "Я Сам", Some("9990"), "отчёт.bin", "2026-09-10T10:00:00"));
    let lines = Mutex::new(Vec::new());
    let options = Options { vanish_wait: Duration::from_secs(1), lfs_from: u64::MAX, ..fast() };
    let rows = [env.server.row(606)];
    let r = export_rows(
        &env.server.client(),
        &rows,
        env.dir.path(),
        &url_of,
        &context(AttachmentScope::Mine),
        &|l| lines.lock().unwrap().push(l),
        &AtomicBool::new(false),
        &options,
    );
    assert!(env.same(&env.plain(606, "9990 — отчёт.bin"), 9990), "{}\n{:?}", show(&r), env.tree());
    let lines = lines.into_inner().unwrap();
    let last = lines.iter().rev().find(|l| l.starts_with("Выгружено")).cloned().unwrap_or_default();
    assert!(last.contains(" · вложения 293 КБ"), "300 000 байт — «293 КБ» (`Size`): {lines:?}");
}

#[test]
fn the_volume_line_names_an_absent_lfs_folder_as_empty() {
    // C# `VolumeNote`: `Directory.Exists(folder) ? … : 0` — крупных не было, папки files-lfs нет: «0 Б», строка на месте
    let env = Env::new("volume");
    let r = env.mine(&[604]);
    assert!(!env.dir.join(LFS_FOLDER).exists(), "{:?}", env.tree());
    assert!(r.error.contains("Вложения в базе: files — 600 Б, files-lfs — 0 Б."), "{}", show(&r));
}

// =====================================================================================================================
// 3. Настройка «Вложения в базу знаний»
// =====================================================================================================================

#[test]
fn switching_from_none_to_mine_downloads_what_mine_takes_by_the_role_in_the_file() {
    // C# `FilesInBase`: недостающее, что нынешняя настройка скачала бы (роль — `my_role` из файла заявки), — перечитать и
    // докачать; чужая (роли нет) так и остаётся «без изменений»
    let env = Env::new("none-then-mine");
    let first = env.run(AttachmentScope::None, &[601, 602]);
    assert_eq!(first.created, 2, "{}", show(&first));
    env.server.forget();
    let r = env.mine(&[601, 602]);
    assert_eq!((r.updated, r.unchanged), (1, 1), "601 перечитана, 602 — нет: {}", show(&r));
    let asked = env.server.asked();
    assert!([9001, 9002, 9003].iter().all(|id| asked.contains(id)) && !asked.contains(&9101), "{asked:?}");
}

static MOVES: AtomicUsize = AtomicUsize::new(0);

/// Подмена занята антивирусом дважды (код 32 — нарушение совместного доступа), потом ложится.
fn busy_twice(from: &Path, to: &Path) -> std::io::Result<()> {
    if MOVES.fetch_add(1, SeqCst) < 2 { Err(std::io::Error::from_raw_os_error(32)) } else { fs::rename(from, to) }
}

#[test]
fn the_export_waits_for_a_file_the_antivirus_holds_and_the_file_lands() {
    // C# 619 (`MoveFile`): подмену держит антивирус — выгрузка повторяет её после пауз, как `WriteAtomic`, и файл ложится
    let env = Env::new("busy-move");
    let rows = [env.server.row(604)];
    let options = Options { move_file: busy_twice, ..fast() };
    let r = export_rows(
        &env.server.client(),
        &rows,
        env.dir.path(),
        &url_of,
        &context(AttachmentScope::Mine),
        &|_| {},
        &AtomicBool::new(false),
        &options,
    );
    assert_eq!(MOVES.load(SeqCst), 3, "два раза занят, третий — лёг");
    assert!(env.same(&env.plain(604, "9301 — журнал.txt"), 9301), "{}\n{:?}", show(&r), env.tree());
    assert!(env.temps().is_empty(), "{:?}", env.temps());
}

static TEMPS: AtomicUsize = AtomicUsize::new(0);

fn disk_full(_: &Path) -> std::io::Result<fs::File> {
    TEMPS.fetch_add(1, SeqCst);
    Err(std::io::Error::other("диск полон"))
}

#[test]
fn a_temp_that_cannot_be_made_fails_the_file_as_not_written_without_retries() {
    // C# `DownloadAsync`: временный не создать — IOException мимо `Retrying` (не сеть) → «файл не записан: …»; в файле заявки —
    // только «файл не записан»
    let env = Env::new("disk-full");
    let rows = [env.server.row(604)];
    let options = Options { create_temp: disk_full, ..fast() };
    let r = export_rows(
        &env.server.client(),
        &rows,
        env.dir.path(),
        &url_of,
        &context(AttachmentScope::Mine),
        &|_| {},
        &AtomicBool::new(false),
        &options,
    );
    assert_eq!(TEMPS.load(SeqCst), 1, "не повторяется: {}", show(&r));
    assert!(env.md(604).contains("\n- журнал.txt · не скачалось: файл не записан\n"), "{}", env.md(604));
    assert!(
        r.error.contains("не скачались 1 — повторная выгрузка попробует их снова: #604 «журнал.txt»: файл не записан: диск полон."),
        "{}",
        show(&r)
    );
    assert!(env.server.asked().is_empty(), "до сервера не дошло: {:?}", env.server.asked());
}

#[test]
fn the_setting_all_downloads_a_foreign_ticket_without_an_edit_of_it() {
    // C# 5: настройку сменили на «всех» — заявка на сервере та же, но вложений в базе не хватает: перечитана и докачана
    let (env, _) = Env::first();
    env.server.forget();
    let r = env.run(AttachmentScope::All, &[602]);
    assert!(env.same(&env.plain(602, "9101 — чужое.docx"), 9101), "{}\n{:?}", show(&r), env.tree());
    assert!(env.server.asked().contains(&9101));
    assert!(!env.md(602).contains("files_missing"), "{}", env.md(602));
}

#[test]
fn the_setting_none_downloads_nothing_new_and_a_file_already_there_stays_linked() {
    // C# 5: «никаких» — новая не скачана («не скачано»), а лежащий файл — по-прежнему со ссылкой; итог без «Вложения в базе»
    let (env, _) = Env::first();
    env.server.touch(601);
    env.server.touch(604);
    env.server.forget();
    let r = env.run(AttachmentScope::None, &[601, 604]);
    assert!(!env.server.asked().contains(&9301), "{:?}", env.server.asked());
    assert!(env.md(604).contains("\n- журнал.txt · не скачано\n"), "{}", env.md(604));
    assert!(env.md(601).contains("- ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б\n"), "{}", env.md(601));
    assert!(!r.error.contains("Вложения в базе"), "{}", show(&r));
}

#[test]
fn the_default_context_downloads_nothing() {
    // `ExportContext` по умолчанию — `AttachmentScope.None`: свои заявки есть, а качать не велено
    let env = Env::new("default-context");
    let rows = [env.server.row(601)];
    let context = Context { me: Some(User { id: 7, name: "Я Сам".into() }), ..Context::default() };
    let r = export_rows(&env.server.client(), &rows, env.dir.path(), &url_of, &context, &|_| {}, &AtomicBool::new(false), &fast());
    assert_eq!(env.server.file_requests(), 0, "{}", show(&r));
    assert!(env.md(601).contains("\n- ошибка.png · не скачано\n"), "{}", env.md(601));
    assert_eq!(text(env.dir.join(".gitattributes")), "*.psd binary\n", "правило LFS — только когда качаем: не None");
}

#[test]
fn an_observer_only_in_the_card_is_a_role_for_my_tickets_and_the_file_is_downloaded() {
    // C# 0.21.0: я только наблюдатель (в карточке; строка списка наблюдателей не прислала) — «моих заявок» качает и её вложения,
    // в файле роль «наблюдатель»
    let env = Env::new("observer");
    env.mine(&[632]);
    assert!(env.same(&env.plain(632, "9980 — схема наблюдателя.png"), 9980), "{:?}", env.tree());
    assert!(env.md(632).contains("\nmy_role: \"наблюдатель\"\n"), "{}", env.md(632));
}

// =====================================================================================================================
// 4. Сбои
// =====================================================================================================================

#[test]
fn a_503_on_a_file_is_retried_and_the_file_lands() {
    // C# 6: 503 один раз — повтор и успех; файл спрошен дважды
    let env = Env::new("retry-503");
    env.server.shared.fail_once.lock().unwrap().insert(9301, 1);
    env.mine(&[604]);
    assert!(env.same(&env.plain(604, "9301 — журнал.txt"), 9301), "{:?}", env.tree());
    assert_eq!(env.server.asked_times(9301), 2, "{:?}", env.server.asked());
}

#[test]
fn a_file_deleted_from_the_disk_comes_back_though_the_ticket_did_not_change() {
    // C# 6: файл вложения удалили с диска (антивирус, руками) — заявка не менялась, но файл вернулся, заявка перечитана
    let env = Env::new("deleted");
    env.mine(&[604]);
    let file = env.plain(604, "9301 — журнал.txt");
    assert!(env.same(&file, 9301), "до удаления файл скачан: {:?}", env.tree());
    fs::remove_file(&file).unwrap();
    env.server.forget();
    let r = env.mine(&[604]);
    assert!(env.same(&file, 9301), "{:?}", env.tree());
    assert_eq!(r.updated, 1, "{}", show(&r));
    assert_eq!(env.server.asked(), vec![9301]);
}

#[test]
fn a_break_and_a_silence_in_the_middle_of_a_file_leave_no_file_no_temp_and_say_why() {
    // C# 6 (605, 611): обрыв и тишина посреди файла — ни файла, ни временного, в файле заявки — причина; 401 на файл —
    // тоже строка заявки (к файлам, может, не пускает вход API), выгрузка не прерывается и идёт дальше
    let env = Env::new("broken");
    let r = env.mine(&[605, 611]);
    assert!(!env.plain(605, "9401 — обрыв.bin").exists() && !env.plain(605, "9402 — тишина.bin").exists(), "{:?}", env.tree());
    assert!(env.temps().is_empty(), "{:?}", env.temps());
    let md = env.md(605);
    assert!(md.contains("\n- обрыв.bin · не скачалось: соединение оборвалось"), "{md}");
    assert!(md.contains("\n- тишина.bin · не скачалось: сервер не ответил за 0,5 с: файл перестал приходить на "), "{md}");
    let md611 = env.md(611);
    assert!(md611.contains("\n- закрыт.pdf · не скачалось: неверный логин или пароль (HTTP 401)\n"), "{md611}");
    assert_eq!((r.created, r.failed, r.complete), (2, 0, true), "{}", show(&r));
    assert!(r.error.contains("не скачались 3 — повторная выгрузка попробует их снова: #"), "{}", show(&r));
}

#[test]
fn a_ticket_with_a_failed_file_is_read_again_and_its_file_asked_again_by_the_next_export() {
    // C# 6 (605): не скачалось — файл заявки без revision, «повторная выгрузка попробует их снова»: следующая перечитывает заявку и
    // спрашивает файл снова (сервер всё так же обрывает — версии по-прежнему нет)
    let env = Env::new("failed-then-again");
    env.mine(&[605]);
    assert!(!env.md(605).contains("\nrevision: "), "{}", env.md(605));
    env.server.forget();
    let r = env.mine(&[605]);
    assert_eq!((r.updated, r.unchanged), (1, 0), "{}", show(&r));
    assert!(env.server.asked_times(9401) >= 1, "обрывавшийся файл спросили снова: {:?}", env.server.asked());
    assert!(!env.md(605).contains("\nrevision: "), "пока файл не скачан, версии нет");
}

#[test]
fn stop_in_the_middle_of_a_file_leaves_no_file_no_temp_and_no_ticket_file() {
    // C# 7: «Остановить» посреди файла — ни файла, ни временного, файл заявки не записан; заявка не считается
    let env = Env::new("stop");
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    *env.server.shared.on_sent.lock().unwrap() = Some(Arc::new(move || flag.store(true, SeqCst)));
    let rows = [env.server.row(606)];
    let r = export_rows(&env.server.client(), &rows, env.dir.path(), &url_of, &context(AttachmentScope::Mine), &|_| {}, &stop, &fast());
    assert!(stop.load(SeqCst), "сервер дошёл до отданного начала файла");
    assert!(!r.complete, "{}", show(&r));
    assert_eq!((r.created, r.updated), (0, 0), "{}", show(&r));
    assert!(!env.plain(606, "9501 — долгий.bin").exists(), "{:?}", env.tree());
    assert!(env.temps().is_empty(), "{:?}", env.temps());
    assert!(env.md_path(606).is_none(), "файл заявки не записан: {:?}", env.tree());
}

#[test]
fn a_long_path_shortens_the_name_to_25_characters_and_the_next_export_does_not_download_it_again() {
    // C# 8: путь у предела MAX_PATH — имя укорочено до 25 знаков (расширение цело), файл записан; следующая выгрузка (заявка
    // изменилась) его не качает. Глубина папки — от длины пути: файл в files-lfs ложится ровно на предел
    let env = Env::new("deep");
    let lfs_tail = format!("{LFS_FOLDER}/2026-09/607");
    let dir_len = env.dir.path().to_string_lossy().chars().count();
    let deep = env.dir.join(&"d".repeat(MAX_PATH_LENGTH - 1 - 25 - dir_len - 2 - lfs_tail.chars().count()));
    let r = env.run_in(&deep, AttachmentScope::Mine, &[607]);
    let found: Vec<String> = walk(&deep.join(FILES_FOLDER)).into_iter().filter(|p| p.contains("9601")).collect();
    let name = format!("9601 — {}.png", "я".repeat(14));
    assert_eq!(found.len(), 1, "{found:?}\n{}", show(&r));
    assert!(found[0].ends_with(&name), "имя укорочено до 25 знаков: {found:?}");
    assert_eq!(deep.join(&lfs_tail).join(&name).to_string_lossy().chars().count(), MAX_PATH_LENGTH, "путь к крупному — ровно предел");
    assert!(r.error.contains("Вложения: скачано 1"), "{}", show(&r));
    env.server.touch(607);
    env.server.forget();
    env.run_in(&deep, AttachmentScope::Mine, &[607]);
    assert_eq!(env.server.file_requests(), 0, "длинный путь — не качаем снова");
}

#[test]
fn without_the_lfs_rule_small_files_land_and_the_large_one_does_not_and_the_summary_says_why() {
    // C# 9: .gitattributes не записать (на его месте папка) — мелкие файлы ложатся, крупный — нет (и не качается дальше порога:
    // он приходит частями с паузой, дочитывать его до конца — дождаться молчания), итог говорит почему, файл заявки — без пути на диске
    let env = Env::new("no-rule");
    let blocked = env.dir.join("без правила");
    fs::create_dir_all(blocked.join(".gitattributes")).unwrap();
    env.server.shared.large_stalls.store(true, SeqCst);
    let r = env.run_in(&blocked, AttachmentScope::Mine, &[601]);
    let md_dir = blocked.join(format!("{TICKETS_FOLDER}/2026-09"));
    let md_name =
        fs::read_dir(&md_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).find(|n| n.starts_with("601 — "));
    let md = text(md_dir.join(md_name.expect("файл заявки 601")));
    assert!(blocked.join(format!("{FILES_FOLDER}/2026-09/601/9001 — ошибка.png")).exists(), "{:?}", walk(&blocked));
    assert!(!blocked.join(format!("{LFS_FOLDER}/2026-09/601/9003 — большой.zip")).exists(), "{:?}", walk(&blocked));
    assert!(md.contains("\n- большой.zip · не скачалось: крупный файл (от 1,5 КБ): нет правила Git LFS в .gitattributes\n"), "{md}");
    assert!(!md.contains(&*blocked.to_string_lossy()), "в файле заявки нет пути на диске: {md}");
    // C# KnowledgeExport.cs:642 и Attachments.cs:465 — целиком, кроме причины (в ней путь этой машины)
    assert!(
        r.error.contains("— крупные файлы (от 1,5 КБ) не сохраняются, пока в .gitattributes нет строки «files-lfs/** filter=lfs diff=lfs merge=lfs -text»: допишите её сами."),
        "{}",
        show(&r)
    );
    assert!(r.error.starts_with("Вложения: не удалось дописать правило Git LFS в "), "{}", show(&r));
    assert!(
        r.error.contains("повторная выгрузка попробует их снова: #601 «большой.zip»: крупный файл (от 1,5 КБ) не сохранён: не удалось дописать правило Git LFS в "),
        "{}",
        show(&r)
    );
}

#[test]
fn a_folder_that_cannot_be_made_and_a_place_that_is_taken_fail_the_file_without_a_path_in_the_ticket() {
    // C# после 9: папка заявки не создаётся (на её месте файл) и на месте файла — папка: «файл не записан», пути на диске в файле заявки нет
    let env = Env::new("unwritable");
    fs::create_dir_all(env.dir.join(&format!("{FILES_FOLDER}/2026-09"))).unwrap();
    fs::write(env.dir.join(&format!("{FILES_FOLDER}/2026-09/608")), "не папка").unwrap();
    fs::create_dir_all(env.plain(609, "9702 — блок.txt")).unwrap();
    let r = env.mine(&[608, 609]);
    let (md608, md609) = (env.md(608), env.md(609));
    assert!(md608.contains("\n- папка.txt · не скачалось: файл не записан\n"), "{md608}");
    assert!(md609.contains("\n- блок.txt · не скачалось: файл не записан\n"), "{md609}");
    let dir = env.dir.path().to_string_lossy().into_owned();
    assert!(!md608.contains(&dir) && !md609.contains(&dir), "в файле заявки нет пути на диске");
    assert_eq!(r.created, 2, "файлы заявок записаны, хотя вложения не легли: {}", show(&r));
    assert!(env.temps().is_empty(), "{:?}", env.temps());
}

// =====================================================================================================================
// 5. Страницы и переадресации вместо файлов
// =====================================================================================================================

#[test]
fn a_page_or_a_redirect_instead_of_a_file_is_a_failure_and_an_html_attachment_is_a_file() {
    // C# 610: html у картинки и переадресация (страница входа) — не файл; html-вложение — файл; логин и пароль, которые страница
    // повторила, — не в файле заявки и не в итоге
    let env = Env::new("pages");
    let r = env.mine(&[610]);
    let md = env.md(610);
    assert!(!env.plain(610, "9801 — снимок.png").exists() && !env.plain(610, "9803 — вход.png").exists(), "{:?}", env.tree());
    assert!(env.plain(610, "9802 — страница.html").exists(), "html-вложение — файл: {:?}", env.tree());
    assert!(md.contains("\n- снимок.png · не скачалось: сервер вернул страницу вместо файла\n"), "{md}");
    assert!(md.contains("\n- вход.png · не скачалось: сервер переадресовал запрос файла"), "{md}");
    assert!(r.error.contains("#610 «снимок.png»: сервер вернул страницу вместо файла"), "{}", show(&r));
    assert!(!md.contains("dXNlcjpwYXNz") && !r.error.contains("dXNlcjpwYXNz"), "токен входа не просочился");
}

#[test]
fn without_names_html_is_not_expected_and_a_page_is_not_kept_as_a_file() {
    // C# 613: имён нет (не сошлись с номерами) — html не отличить от страницы входа: не берём; остальное — под номерами
    let env = Env::new("nameless");
    env.mine(&[613]);
    assert!(!env.plain(613, "9811").exists(), "страница не осталась в базе: {:?}", env.tree());
    assert!(env.plain(613, "9812").exists(), "второй файл — под номером: {:?}", env.tree());
    assert!(env.md(613).contains("\n- файл 9811 · не скачалось: сервер вернул страницу вместо файла\n"), "{}", env.md(613));
}

#[test]
fn a_page_that_broke_off_is_still_a_page_and_is_not_retried_as_a_network_failure() {
    // C# 617: страница вместо файла оборвалась на середине — всё равно «страница», а не сетевой сбой (его бы повторяли)
    let env = Env::new("cut-page");
    let r = env.mine(&[617]);
    assert!(env.md(617).contains("\n- обрезанная.png · не скачалось: сервер вернул страницу вместо файла\n"), "{}", env.md(617));
    assert_eq!(env.server.asked_times(9813), 1, "страницу не повторяли: {}", show(&r));
}

#[test]
fn a_refusal_and_a_page_that_go_silent_are_asked_once_and_named_by_what_they_are() {
    // C# 625 и 622: тело отказа (403) и страницы, замолчавшее дольше срока куска, — «нет доступа» и «страница», а не «сервер не
    // ответил»: сетевой сбой повторяли бы минутами; спрошено по разу. По одной заявке: замолчавший ответ держит соединение
    let env = Env::new("silent");
    env.run_in(env.dir.path(), AttachmentScope::Mine, &[625]);
    let refusal = env.md(625);
    env.run_in(env.dir.path(), AttachmentScope::Mine, &[622]);
    let page = env.md(622);
    assert!(refusal.contains("\n- схема.png · не скачалось: нет доступа (HTTP 403)\n"), "{refusal}");
    assert!(page.contains("\n- схема.png · не скачалось: сервер вернул страницу вместо файла\n"), "{page}");
    assert_eq!((env.server.asked_times(9965), env.server.asked_times(9962)), (1, 1), "{:?}", env.server.asked());
}

#[test]
fn a_file_name_with_a_server_error_in_it_does_not_make_the_failure_a_network_one() {
    // C# 615: имя от заявителя с «(HTTP 503)» — не повод счесть сбой вложения сетевым и повторять: файла на сервере нет (404),
    // спрошен один раз; первая ошибка в итоге — про 404
    let env = Env::new("named-503");
    let r = env.mine(&[615]);
    assert_eq!(env.server.asked_times(9920), 1, "{:?}", env.server.asked());
    assert!(r.error.contains("#615 «скан (HTTP 503).pdf»: файл не найден (HTTP 404)."), "{}", show(&r));
}

// =====================================================================================================================
// 6. Антивирус
// =====================================================================================================================

#[test]
fn a_downloaded_file_that_is_gone_at_the_end_is_named_and_the_export_is_not_finished() {
    // C# 614: вложение записано, а потом пропало (антивирус удаляет по содержимому) — итог называет заявку, выгрузка не «закончена»
    let env = Env::new("vanish");
    let victim = env.plain(614, "9906 — вирус.exe");
    *env.server.shared.vanish.lock().unwrap() = Some(Arc::new(move || {
        let _ = fs::remove_file(&victim);
    }));
    let r = env.mine(&[614]);
    assert!(!env.plain(614, "9906 — вирус.exe").exists(), "{:?}", env.tree());
    assert!(r.error.contains("Записано, но уже нет на диске: 1 (#614)"), "{}", show(&r));
    assert!(!r.complete, "{}", show(&r));
}

// =====================================================================================================================
// 7. Просмотр и «Выгрузить найденные»
// =====================================================================================================================

#[test]
fn a_preview_names_the_files_and_asks_for_none_of_them() {
    // C# 10: просмотр (и буфер, и запрос агенту) — только имена, ни одного запроса файла, даже когда качать велено «всех»
    // (охранная проверка: просмотр не качал и до карточки — она не даст ему начать)
    let env = Env::new("preview");
    let at: Date = DateTime::parse_from_rfc3339("2026-09-20T12:00:00+03:00").unwrap();
    let md = build(&env.server.client(), &env.server.row(601), "https://hd/Task/View/601", &context(AttachmentScope::All), &at).unwrap();
    assert!(md.contains("\n## Вложения\n\n- ошибка.png\n- Акт: сверка.pdf\n- большой.zip\n"), "{md}");
    assert_eq!(env.server.file_requests(), 0, "{:?}", env.server.asked());
}

#[test]
fn export_found_downloads_attachments_like_export_rows() {
    // оба пути выгрузки идут через один движок: «Выгрузить найденные» кладёт файлы моей заявки, чужую оставляет «не скачано», итог —
    // тот же слово в слово, правило LFS — в начале
    let env = Env::new("found");
    *env.server.shared.listed.lock().unwrap() = vec![601, 602];
    let r = export_found(
        &env.server.client(),
        &TaskQuery::default(),
        0,
        env.dir.path(),
        &url_of,
        &context(AttachmentScope::Mine),
        &|_| {},
        &AtomicBool::new(false),
        &fast(),
    );
    assert!(env.same(&env.plain(601, "9001 — ошибка.png"), 9001), "{}\n{:?}", show(&r), env.tree());
    assert!(env.same(&env.large(601, "9003 — большой.zip"), 9003), "{:?}", env.tree());
    assert!(!env.server.asked().contains(&9101), "{:?}", env.server.asked());
    assert!(env.md(602).contains("\n- чужое.docx · не скачано\n"), "{}", env.md(602));
    assert!(
        r.error.contains(
            "Вложения: скачано 3 (2,2 КБ), крупных — в files-lfs (Git LFS): 1, не скачано (настройка «Вложения в базу знаний»): 1."
        ),
        "{}",
        show(&r)
    );
    assert!(r.error.contains("Вложения в базе: files — 800 Б, files-lfs — 1,5 КБ."), "{}", show(&r));
    assert_eq!(text(env.dir.join(".gitattributes")), format!("*.psd binary\n{LFS_RULE}\n"));
}

// =====================================================================================================================
// Что выгрузка выбранных строк говорит автовыгрузке о вложениях (карточка 4.5-2)
// =====================================================================================================================

#[test]
fn a_ticket_whose_attachment_did_not_come_has_its_own_error_and_is_not_current() {
    // C# `Landed`: «не скачалось вложение — файл заявки записан без него» и вложение с его ошибкой; готовая — в Current
    let (env, r) = Env::first();
    let error = r.errors.get(&603).cloned().unwrap_or_default();
    assert!(error.starts_with("не скачалось вложение — файл заявки записан без него\n"), "{:?}", r.errors);
    assert!(error.contains("«нет на сервере.txt»") && error.contains("HTTP 404"), "{error}");
    assert!(!r.current.contains(&603), "не «актуальна»: {:?}", r.current);
    assert!(r.current.contains(&601) && r.current.contains(&602), "готовые — актуальные: {:?}", r.current);
    assert!(!r.errors.contains_key(&601) && !r.errors.contains_key(&602), "{:?}", r.errors);
    assert!(r.not_written.is_empty() && r.path_blocked.is_empty(), "файл заявки записан: {r:?} {:?}", env.tree());
}

#[test]
fn a_place_that_cannot_be_written_for_an_attachment_is_the_tickets_own_error_and_its_file_is_written() {
    // C# Files.cs ~212: папка заявки не создаётся (на её месте файл) и на месте файла — папка: у каждой своя ошибка, по которой
    // автовыгрузка пропускает заявку (`of_ticket`); файл заявки записан — не «не записан»; но и не «актуальна»
    let env = Env::new("unwritable-observers");
    fs::create_dir_all(env.dir.join(&format!("{FILES_FOLDER}/2026-09"))).unwrap();
    fs::write(env.dir.join(&format!("{FILES_FOLDER}/2026-09/608")), "не папка").unwrap();
    fs::create_dir_all(env.plain(609, "9702 — блок.txt")).unwrap();
    let r = env.mine(&[608, 609]);
    for id in [608, 609] {
        let error = r.errors.get(&id).cloned().unwrap_or_else(|| panic!("#{id}: у заявки нет своей ошибки: {:?}", r.errors));
        assert!(of_ticket(&error), "#{id}: беда этой заявки — в её счёт: {:?}", r.errors);
        assert!(!r.not_written.contains(&id) && !r.current.contains(&id), "#{id}: {r:?}");
    }
}

#[test]
fn a_page_instead_of_a_file_is_in_the_error_without_the_login_the_page_repeated_and_the_ticket_is_not_current() {
    // C# Files.cs ~212 (610): логин и пароль, которые страница повторила (трассировка прокси), — не в ошибке
    let env = Env::new("pages-observers");
    let r = env.mine(&[610]);
    let error = r.errors.get(&610).cloned().unwrap_or_default();
    assert!(error.contains("страницу вместо файла") && !error.contains("dXNlcjpwYXNz"), "{error}");
    assert!(!r.current.contains(&610), "{:?}", r.current);
}

#[test]
fn an_attachment_that_vanished_makes_its_ticket_not_current_and_the_error_says_the_antivirus() {
    // C# Files.cs ~212 (614): вложение записано и пропало — заявка не «готова», в ошибке — «заявки или вложения»
    let env = Env::new("vanish-observers");
    let victim = env.plain(614, "9906 — вирус.exe");
    *env.server.shared.vanish.lock().unwrap() = Some(Arc::new(move || {
        let _ = fs::remove_file(&victim);
    }));
    let r = env.mine(&[614]);
    assert!(!r.current.contains(&614), "{:?}", r.current);
    assert!(r.errors.get(&614).is_some_and(|e| e.contains("заявки или вложения")), "{:?}", r.errors);
}

#[test]
fn a_file_is_unchanged_only_with_the_attachments_its_asker_wants() {
    // C# Files.cs 101–120 (`Unchanged`): с какими вложениями файл верен — говорит спрашивающий; свойства и вложения — по вопросу
    let (env, _) = Env::first();
    let rows: Vec<Found> = [601, 602, 603].iter().map(|&id| env.server.row(id)).collect();
    let mut current = unchanged(env.dir.path(), &rows);
    assert!(current(&rows[0], AttachmentScope::Mine) && current(&rows[0], AttachmentScope::All), "мои вложения лежат");
    assert!(current(&rows[1], AttachmentScope::Mine), "у чужой роли нет — вложения ей не нужны");
    assert!(!current(&rows[1], AttachmentScope::All), "а «всех» ей не хватает: файл 9101 не скачан");
    assert!(!current(&rows[2], AttachmentScope::Mine), "вложение моей не скачалось — файл без версии, не верен");
    fs::remove_file(env.plain(601, "9001 — ошибка.png")).unwrap();
    assert!(!current(&rows[0], AttachmentScope::Mine), "файл вложения убрали — не верен: проверяется по вопросу");
}
