//! Выгрузка выбранных заявок файлами (`KnowledgeExport.ExportRowsAsync`, `Job`, `Conclude`, `Retrying`;
//! `SearchViewModel.Summary`; карточка 4.2-2): настоящий клиент против поддельного сервера на loopback (по потоку на
//! соединение — «четыре разом» видно по самому серверу), папки — временные. Сценарии — из самопроверки C#
//! (`KnowledgeExport.SelfCheck*.cs`). Вложения не качаются: лежащие в базе — ссылками, остальные — «не скачано».

mod common;

use chrono::DateTime;
use common::Reply;
use regex_lite::Regex;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use ticketboard::export::files::*;
use ticketboard::export::run::*;
use ticketboard::export::{Context, revision};
use ticketboard::intraservice::{Client, Date, Found, User};
use ticketboard::rules::ClosedNames;

fn date(text: &str) -> Date {
    DateTime::parse_from_rfc3339(text).unwrap()
}

// ---------- временные папки и файлы ----------

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("tb-rows-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
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

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref()).unwrap_or_else(|e| format!("(не прочитать {}: {e})", path.as_ref().display()))
}

fn age(path: &Path, seconds: u64) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds)).unwrap();
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

/// Имена в папке по порядку; нет папки — пусто.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).map(|d| d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

/// Все файлы под папкой — пути от неё, по порядку.
fn walk(dir: &Path) -> Vec<String> {
    fn go(base: &Path, at: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(at).unwrap() {
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

/// Файл заявки в папке месяца: «номер — название.md» или «номер.md».
fn ticket_file(dir: &Path, month: &str, id: i32) -> Option<PathBuf> {
    let folder = dir.join(TICKETS_FOLDER).join(month);
    names_in(&folder)
        .into_iter()
        .find(|n| n.starts_with(&format!("{id} — ")) && n.ends_with(".md") || *n == format!("{id}.md"))
        .map(|n| folder.join(n))
}

/// То же, но не найден — это сама неудача проверки, с перечнем того, что лежит.
fn must_ticket_file(dir: &Path, month: &str, id: i32) -> PathBuf {
    ticket_file(dir, month, id).unwrap_or_else(|| panic!("файла заявки {id} в {month} нет; в папке выгрузки: {:?}", walk(dir)))
}

fn prop(path: &Path, key: &str) -> Option<String> {
    read_props(path).and_then(|p| p.into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
}

// ---------- поддельный сервер ----------

/// Заявка на сервере: что отдают карточка и история.
#[derive(Clone)]
struct T {
    id: i32,
    name: String,
    created: Option<String>,
    changed: String,
    files: Vec<(i32, String)>,
    events: usize,
    /// История приходит с номерами страниц (по умолчанию); нет — сервер их не присылает, конец — по неполной странице.
    paged: bool,
}

impl T {
    fn new(id: i32, name: &str, created: Option<&str>) -> T {
        T {
            id,
            name: name.into(),
            created: created.map(String::from),
            changed: "2026-09-20T12:00:00+03:00".into(),
            files: Vec::new(),
            events: 3,
            paged: true,
        }
    }
    fn row(&self) -> Found {
        Found {
            id: self.id,
            name: self.name.clone(),
            status: "Выполнена".into(),
            created: self.created.as_deref().map(date),
            changed: Some(date(&self.changed)),
            ..Default::default()
        }
    }
}

type Fault = Arc<dyn Fn(&str, i32, usize) -> u16 + Send + Sync>;
type Hook = Arc<dyn Fn(&str, i32, usize) + Send + Sync>;

#[derive(Default)]
struct Inner {
    tickets: Mutex<BTreeMap<i32, T>>,
    log: Mutex<Vec<(Instant, String)>>,
    attempts: Mutex<HashMap<(String, i32), usize>>,
    card_seq: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    fault: Mutex<Option<Fault>>,
    hook: Mutex<Option<Hook>>,
}

struct Fake {
    inner: Arc<Inner>,
    url: String,
}

fn card_json(t: &T) -> String {
    let mut task = format!(
        r#""Id":{},"Name":{},"StatusName":"Выполнена","Creator":"Петрова А.","Changed":"{}""#,
        t.id,
        serde_json::to_string(&t.name).unwrap(),
        t.changed
    );
    if let Some(c) = &t.created {
        task.push_str(&format!(r#","Created":"{c}""#));
    }
    if !t.files.is_empty() {
        let ids: Vec<String> = t.files.iter().map(|(i, _)| i.to_string()).collect();
        let names: Vec<&str> = t.files.iter().map(|(_, n)| n.as_str()).collect();
        task.push_str(&format!(r#","FileIds":"{}","FileNames":{}"#, ids.join(","), serde_json::to_string(&names.join("||")).unwrap()));
    }
    format!(r#"{{"Task":{{{task}}}}}"#)
}

/// Страница истории: записи с номерами по 50 на страницу; в каждой — свой текст «запись N».
fn life_json(t: &T, page: usize) -> String {
    let (from, to) = ((page - 1) * 50, (page * 50).min(t.events));
    let base = date("2026-09-10T10:00:00+03:00");
    let events: Vec<String> = (from..to.max(from))
        .map(|n| {
            let when = base + chrono::Duration::minutes(n as i64);
            format!(r#"{{"Date":"{}","Editor":"Иванов Иван","EditorId":7,"Comments":"запись {n}"}}"#, when.to_rfc3339())
        })
        .collect();
    let pages = t.events.div_ceil(50).max(1);
    if !t.paged {
        return format!(r#"{{"TaskLifetimes":[{}]}}"#, events.join(","));
    }
    format!(
        r#"{{"TaskLifetimes":[{}],"Paginator":{{"Count":{},"Page":{page},"PageCount":{pages}}}}}"#,
        events.join(","),
        to.saturating_sub(from)
    )
}

impl Inner {
    fn attempts_of(&self, kind: &str, id: i32) -> usize {
        self.attempts.lock().unwrap().get(&(kind.to_string(), id)).copied().unwrap_or(0)
    }

    fn reply(&self, path: &str) -> Reply {
        let (kind, id, page) = if let Some(rest) = path.strip_prefix("/api/task/") {
            ("card", rest.split('?').next().unwrap_or("").parse().unwrap_or(0), 1)
        } else if path.starts_with("/api/tasklifetime?") {
            let value =
                |key: &str| path.split(&format!("{key}=")).nth(1).and_then(|v| v.split('&').next()).and_then(|v| v.parse::<i32>().ok());
            ("life", value("taskid").unwrap_or(0), value("page").unwrap_or(1) as usize)
        } else {
            ("other", 0, 1)
        };
        let attempt = {
            let mut attempts = self.attempts.lock().unwrap();
            let n = attempts.entry((if kind == "life" { format!("life{page}") } else { kind.to_string() }, id)).or_insert(0);
            *n += 1;
            *n
        };
        if kind == "card" {
            let seq = self.card_seq.fetch_add(1, Ordering::SeqCst) + 1;
            let hook = self.hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(kind, id, seq);
            }
        }
        let fault = self.fault.lock().unwrap().clone();
        if let Some(code) = fault.map(|f| f(kind, id, attempt)).filter(|c| *c != 0) {
            return Reply::new(code, "text/plain", "");
        }
        let ticket = self.tickets.lock().unwrap().get(&id).cloned();
        match (kind, ticket) {
            ("card", Some(t)) => Reply::json(&card_json(&t)),
            ("life", Some(t)) => Reply::json(&life_json(&t, page)),
            _ => Reply::new(404, "text/plain", ""),
        }
    }
}

fn serve_one(mut stream: TcpStream, inner: &Inner) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split(' ').nth(1).unwrap_or("").to_string();
    inner.log.lock().unwrap().push((Instant::now(), path.clone()));
    let now = inner.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    inner.max_in_flight.fetch_max(now, Ordering::SeqCst);
    let reply = inner.reply(&path);
    // запрос закончен тут, до отправки: следующий запрос того же работника приходит уже после ответа на этот
    inner.in_flight.fetch_sub(1, Ordering::SeqCst);
    thread::sleep(reply.delay);
    let reason = match reply.code {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.code,
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(&reply.body);
}

impl Fake {
    fn new(tickets: Vec<T>) -> Fake {
        let inner = Arc::new(Inner::default());
        *inner.tickets.lock().unwrap() = tickets.into_iter().map(|t| (t.id, t)).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let shared = inner.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let inner = shared.clone();
                thread::spawn(move || serve_one(stream, &inner));
            }
        });
        Fake { inner, url }
    }
    fn client(&self) -> Client {
        Client::new(&self.url, "u", "p", Duration::from_secs(10))
    }
    fn rows(&self) -> Vec<Found> {
        self.inner.tickets.lock().unwrap().values().map(T::row).collect()
    }
    fn row(&self, id: i32) -> Found {
        self.inner.tickets.lock().unwrap()[&id].row()
    }
    /// Изменить заявку на сервере.
    fn edit(&self, id: i32, change: impl FnOnce(&mut T)) {
        change(self.inner.tickets.lock().unwrap().get_mut(&id).unwrap());
    }
    fn set_fault(&self, f: impl Fn(&str, i32, usize) -> u16 + Send + Sync + 'static) {
        *self.inner.fault.lock().unwrap() = Some(Arc::new(f));
    }
    fn set_hook(&self, f: impl Fn(&str, i32, usize) + Send + Sync + 'static) {
        *self.inner.hook.lock().unwrap() = Some(Arc::new(f));
    }
    /// Забыть всё, что было: и запросы, и счёт попыток, и номер карточки; сбои и крючок — тоже.
    fn reset(&self) {
        self.inner.log.lock().unwrap().clear();
        self.inner.attempts.lock().unwrap().clear();
        self.inner.card_seq.store(0, Ordering::SeqCst);
        *self.inner.fault.lock().unwrap() = None;
        *self.inner.hook.lock().unwrap() = None;
    }
    fn paths(&self) -> Vec<String> {
        self.inner.log.lock().unwrap().iter().map(|(_, p)| p.clone()).collect()
    }
    fn count(&self, prefix: &str) -> usize {
        self.paths().iter().filter(|p| p.starts_with(prefix)).count()
    }
    fn cards(&self) -> usize {
        self.count("/api/task/")
    }
    fn lives(&self) -> usize {
        self.count("/api/tasklifetime")
    }
    /// Когда пришли запросы карточки, по порядку.
    fn card_times(&self) -> Vec<Instant> {
        self.inner.log.lock().unwrap().iter().filter(|(_, p)| p.starts_with("/api/task/")).map(|(t, _)| *t).collect()
    }
    fn attempts(&self, kind: &str, id: i32) -> usize {
        self.inner.attempts_of(kind, id)
    }
    /// Заявки из каждого диапазона ждут (по условию, а не «немного»), пока оно не выполнится: так волны заявок идут одна за другой
    /// в том порядке, какой нужен проверке, как бы ни легли четыре работника.
    #[allow(clippy::type_complexity)]
    fn gates(&self, list: Vec<(std::ops::Range<i32>, Box<dyn Fn(&Inner) -> bool + Send + Sync>)>) {
        let inner = self.inner.clone();
        self.set_hook(move |_, id, _| {
            let deadline = Instant::now() + Duration::from_secs(10);
            for (range, ready) in &list {
                while range.contains(&id) && !ready(&inner) && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        });
    }
}

fn month_tickets(count: usize, first: i32, month: &str) -> Vec<T> {
    (0..count)
        .map(|i| {
            T::new(first + i as i32, &format!("Заявка {}", first + i as i32), Some(&format!("{month}-{:02}T10:00:00+03:00", i % 28 + 1)))
        })
        .collect()
}

// ---------- запуск ----------

fn context() -> Context {
    Context { me: Some(User { id: 7, name: "Я Сам".into() }), closed: ClosedNames::default() }
}

fn fast() -> Options {
    Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::ZERO }
}

fn url_of(id: i32) -> String {
    format!("https://hd/Task/View/{id}")
}

fn run_with(
    fake: &Fake,
    rows: &[Found],
    dir: &Path,
    stop: &AtomicBool,
    options: &Options,
    progress: &(dyn Fn(String) + Sync),
) -> ExportResult {
    export_rows(&fake.client(), rows, dir, &url_of, &context(), progress, stop, options)
}

fn run(fake: &Fake, rows: &[Found], dir: &Path) -> ExportResult {
    run_with(fake, rows, dir, &AtomicBool::new(false), &fast(), &|_| {})
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

/// «Что успели…» — хвост итога, когда выгрузка прервалась или остановлена.
fn after_trip(head: &str, target: usize) -> Regex {
    Regex::new(&format!(
        "^{head} Что успели, сохранено \\(обработано (\\d+) из {target}\\); исправьте причину и запустите выгрузку снова — она продолжится с этого места\\.$"
    ))
    .unwrap()
}

// ---------- постоянные и итог ----------

#[test]
fn the_defaults_are_those_of_csharp() {
    // «Начинаю…» — константа карточки (проходит и с заглушкой — она нужна окну); паузы и срок — настоящая проверка
    assert_eq!(STARTING, "Начинаю…");
    let options = Options::default();
    assert_eq!(options.retry_delays, [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(15)]);
    assert_eq!(options.vanish_wait, Duration::from_secs(3));
}

fn result(found: usize, created: usize, updated: usize, unchanged: usize) -> ExportResult {
    ExportResult { found, created, updated, unchanged, complete: true, ..Default::default() }
}

#[test]
fn a_finished_export_says_done_and_counts_files() {
    assert_eq!(summary(&result(3, 1, 1, 1), "Выбрано"), "Готово. Выбрано — 3: новых файлов 1, обновлено 1, без изменений 1.");
    assert_eq!(summary(&result(3, 3, 0, 0), "По отбору"), "Готово. По отбору — 3: новых файлов 3, обновлено 0, без изменений 0.");
}

#[test]
fn an_unfinished_export_says_not_finished_and_how_many_were_handled() {
    let r = ExportResult { complete: false, ..result(3, 1, 1, 1) };
    assert_eq!(summary(&r, "Выбрано"), "Не закончено. Выбрано, обработано — 3: новых файлов 1, обновлено 1, без изменений 1.");
}

#[test]
fn nothing_found_says_so_or_gives_the_error() {
    assert_eq!(summary(&result(0, 0, 0, 0), "Выбрано"), "Под отбор не попало ни одной заявки.");
    let stopped = ExportResult {
        complete: false, error: "Остановлено до начала — ничего не записано.".into(), ..Default::default()
    };
    assert_eq!(summary(&stopped, "Выбрано"), "Остановлено до начала — ничего не записано.");
}

#[test]
fn failed_tickets_and_the_error_follow_on_their_own_lines() {
    let failed = ExportResult { failed: 2, first_error: "#7: заявка не найдена (HTTP 404)".into(), ..result(5, 2, 1, 0) };
    assert_eq!(
        summary(&failed, "Выбрано"),
        "Готово. Выбрано — 5: новых файлов 2, обновлено 1, без изменений 0.\nНе выгружены: 2 — повторная выгрузка попробует их снова. Первая ошибка — #7: заявка не найдена (HTTP 404)"
    );
    let both =
        ExportResult { error: "Остановлено — что успели, сохранено.".into(), complete: false, ..failed };
    assert_eq!(
        summary(&both, "Выбрано"),
        "Не закончено. Выбрано, обработано — 5: новых файлов 2, обновлено 1, без изменений 0.\nНе выгружены: 2 — повторная выгрузка попробует их снова. Первая ошибка — #7: заявка не найдена (HTTP 404)\nОстановлено — что успели, сохранено."
    );
    let only_error = ExportResult { error: "Вложения: уже были 2.".into(), ..result(1, 1, 0, 0) };
    assert_eq!(
        summary(&only_error, "Выбрано"),
        "Готово. Выбрано — 1: новых файлов 1, обновлено 0, без изменений 0.\nВложения: уже были 2."
    );
}

#[test]
fn numbers_in_the_summary_have_a_no_break_space_between_thousands() {
    let r = ExportResult { failed: 1000, first_error: "#1: x".into(), ..result(1_234_567, 1000, 999, 12_345) };
    assert_eq!(
        summary(&r, "Выбрано"),
        "Готово. Выбрано — 1\u{A0}234\u{A0}567: новых файлов 1\u{A0}000, обновлено 999, без изменений 12\u{A0}345.\nНе выгружены: 1\u{A0}000 — повторная выгрузка попробует их снова. Первая ошибка — #1: x"
    );
}

// ---------- файлы ----------

#[test]
fn the_first_export_writes_a_file_per_ticket_in_the_month_of_creation_and_the_indexes() {
    let tmp = Tmp::new("first");
    let dir = tmp.join("новая/база");
    let fake = Fake::new(vec![T::new(501, "Принтер не печатает", Some("2026-09-12T10:00:00+03:00")), T::new(504, "Почта не уходит", None)]);
    let r = run(&fake, &fake.rows(), &dir);
    assert_eq!((counts(&r), r.complete, r.error.as_str(), r.first_error.as_str()), ((2, 2, 0, 0, 0), true, "", ""), "{}", show(&r));

    let f501 = dir.join("tickets/2026-09/501 — Принтер не печатает.md");
    let f504 = dir.join("tickets/без даты/504 — Почта не уходит.md");
    assert!(f501.is_file() && f504.is_file(), "файлы: {:?}", walk(&dir));
    assert_eq!(prop(&f501, "id").as_deref(), Some("501"));
    assert_eq!(prop(&f501, "title").as_deref(), Some("Принтер не печатает"));
    assert_eq!(prop(&f501, "url").as_deref(), Some("https://hd/Task/View/501"), "ссылку даёт вызывающий");
    assert_eq!(prop(&f501, "source").as_deref(), Some("intraservice"));
    assert_eq!(prop(&f501, "format").as_deref(), Some("8"));
    assert_eq!(prop(&f501, "revision"), Some(revision(&date("2026-09-20T12:00:00+03:00"))));
    assert!(text(&f501).contains("\n# 501 · Принтер не печатает\n") && text(&f501).contains("запись 2"), "{}", text(&f501));

    // оглавления: месяца, «без даты» и общее — по месяцам, свежие сверху, «без даты» в конце, имя папки — в адресе по %-правилам
    assert!(text(dir.join("tickets/2026-09/_index.md")).contains("- [[501 — Принтер не печатает]] · 12.09.2026 · Выполнена\n"));
    assert!(text(dir.join("tickets/без даты/_index.md")).contains("- [[504 — Почта не уходит]] · Выполнена\n"));
    let root = text(dir.join("_index.md"));
    assert!(root.contains("заявок: 2."), "{root}");
    let (a, b) = (
        "- [2026-09](tickets/2026-09/_index.md) — заявок: 1\n",
        "- [без даты](tickets/%D0%B1%D0%B5%D0%B7%20%D0%B4%D0%B0%D1%82%D1%8B/_index.md) — заявок: 1\n",
    );
    assert!(root.find(a).is_some_and(|x| root.find(b).is_some_and(|y| x < y)), "{root}");

    // ничего лишнего: ни таблицы, ни временных, ни правил (это — следующие шаги)
    assert_eq!(names_in(&dir), ["_index.md", "tickets"]);
    assert!(walk(&dir).iter().all(|p| !p.ends_with(".tmp")), "{:?}", walk(&dir));
    // по запросу карточки и по полной истории на заявку
    assert_eq!((fake.cards(), fake.lives()), (2, 2));
}

#[test]
fn an_unchanged_ticket_is_neither_asked_nor_written_and_nothing_is_rewritten_in_the_indexes() {
    let tmp = Tmp::new("again");
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00")), T::new(504, "Почта", None)]);
    run(&fake, &fake.rows(), tmp.path());
    let files = [tmp.join("tickets/2026-09/501 — Принтер.md"), tmp.join("tickets/без даты/504 — Почта.md"), tmp.join("_index.md")];
    assert!(files.iter().all(|f| f.is_file()), "первая выгрузка записала файлы и общее оглавление: {:?}", walk(tmp.path()));
    for f in &files {
        age(f, 86_400);
    }
    let before: Vec<SystemTime> = files.iter().map(|f| modified(f)).collect();
    fake.reset();

    // ничего не записано — проверять нечего, и ждать срока проверки незачем
    let options = Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::from_secs(3) };
    let lines = Mutex::new(Vec::<String>::new());
    let progress = |line: String| lines.lock().unwrap().push(line);
    let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &progress);
    assert!(!lines.lock().unwrap().iter().any(|l| l.starts_with("Проверяю")), "{:?}", lines.lock().unwrap());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((2, 0, 0, 2, 0), true, ""), "{}", show(&r));
    assert!(fake.paths().is_empty(), "ни одного запроса: {:?}", fake.paths());
    let after: Vec<SystemTime> = files.iter().map(|f| modified(f)).collect();
    assert_eq!(before, after, "файлы заявок и общее оглавление не переписаны");
}

#[test]
fn a_changed_ticket_is_read_and_rewritten_and_the_others_are_skipped() {
    let tmp = Tmp::new("changed");
    let fake =
        Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00")), T::new(502, "Почта", Some("2026-09-13T10:00:00+03:00"))]);
    run(&fake, &fake.rows(), tmp.path());
    let (f501, f502) = (must_ticket_file(tmp.path(), "2026-09", 501), must_ticket_file(tmp.path(), "2026-09", 502));
    age(&f502, 86_400);
    let old502 = modified(&f502);
    fake.edit(501, |t| t.changed = "2026-09-21T08:30:00+03:00".into());
    fake.reset();

    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (2, 0, 1, 1, 0), "{}", show(&r));
    assert_eq!(prop(&f501, "changed").as_deref(), Some("2026-09-21T08:30:00+03:00"));
    assert_eq!(prop(&f501, "revision"), Some(revision(&date("2026-09-21T08:30:00+03:00"))));
    assert_eq!(modified(&f502), old502, "неизменная не переписана");
    assert!(
        !fake.paths().iter().any(|p| p.contains("/502") || p.contains("taskid=502")),
        "неизменную заявку не просят: {:?}",
        fake.paths()
    );
    assert_eq!((fake.cards(), fake.lives()), (1, 1));
}

#[test]
fn two_edits_in_one_second_differ_by_the_fraction() {
    // сервер присылает микросекунды, в «changed» файла их нет: вторая правка — не «без изменений»
    let tmp = Tmp::new("fraction");
    let mut t = T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"));
    t.changed = "2026-09-20T12:00:00.100000+03:00".into();
    let fake = Fake::new(vec![t]);
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 1, 0, 0, 0));
    let file = must_ticket_file(tmp.path(), "2026-09", 501);
    let first = prop(&file, "revision");

    fake.edit(501, |t| t.changed = "2026-09-20T12:00:00.900000+03:00".into());
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    assert_ne!(prop(&file, "revision"), first);
    assert_eq!(prop(&file, "changed").as_deref(), Some("2026-09-20T12:00:00+03:00"), "в файле — секунды без долей");
    // и больше не переписывается
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 0, 1, 0), "{}", show(&r));
}

/// Выгрузить одну заявку, испортить её файл заменой, выгрузить снова — итог второго захода и файл.
fn tamper_and_run(tag: &str, edit: impl FnOnce(String) -> String) -> (ExportResult, PathBuf, Fake, Tmp) {
    let tmp = Tmp::new(tag);
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    run(&fake, &fake.rows(), tmp.path());
    let file = must_ticket_file(tmp.path(), "2026-09", 501);
    fs::write(&file, edit(text(&file))).unwrap();
    fake.reset();
    let r = run(&fake, &fake.rows(), tmp.path());
    (r, file, fake, tmp)
}

#[test]
fn a_file_of_an_older_format_is_rewritten_though_the_ticket_did_not_change() {
    let (r, file, fake, _tmp) = tamper_and_run("older", |t| t.replace("\nformat: 8\n", "\nformat: 6\n"));
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    assert_eq!(prop(&file, "format").as_deref(), Some("8"));
    assert_eq!(fake.cards(), 1);
}

#[test]
fn a_file_of_a_newer_format_is_rewritten_too_the_format_must_be_the_same() {
    let (r, file, _fake, _tmp) = tamper_and_run("newer", |t| t.replace("\nformat: 8\n", "\nformat: 9\n"));
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    assert_eq!(prop(&file, "format").as_deref(), Some("8"));
}

#[test]
fn a_file_without_a_revision_is_rewritten() {
    // так пишется файл, у которого не скачалось вложение: следующая выгрузка перечитает заявку
    let (r, file, _fake, _tmp) =
        tamper_and_run("norevision", |t| t.lines().filter(|l| !l.starts_with("revision: ")).collect::<Vec<_>>().join("\n") + "\n");
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    assert!(prop(&file, "revision").is_some());
}

#[test]
fn a_renamed_ticket_replaces_our_old_file_and_user_files_stay() {
    let tmp = Tmp::new("renamed");
    let fake = Fake::new(vec![T::new(501, "Принтер не печатает", Some("2026-09-12T10:00:00+03:00"))]);
    run(&fake, &fake.rows(), tmp.path());
    let month = tmp.join("tickets/2026-09");
    let old = month.join("501 — Принтер не печатает.md");
    assert!(old.is_file());
    // чужое с номером в имени: испорченный файл без наших свойств и заметка пользователя со своими; и заметка без номера
    put(&month.join("501 — Принтер.md"), "испорчен");
    let note = "---\ntitle: \"моё\"\n---\n\nпишу про принтер\n";
    put(&month.join("501 — мои заметки.md"), note);
    put(&tmp.join("tickets/заметки.md"), "моё");
    fake.edit(501, |t| {
        t.name = "Принтер HP не печатает".into();
        t.changed = "2026-09-21T08:30:00+03:00".into();
    });
    fake.reset();

    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    assert!(!old.exists(), "прежний наш файл убран");
    assert!(month.join("501 — Принтер HP не печатает.md").is_file());
    assert_eq!(text(month.join("501 — Принтер.md")), "испорчен");
    assert_eq!(text(month.join("501 — мои заметки.md")), note);
    assert_eq!(text(tmp.join("tickets/заметки.md")), "моё");
    let index = text(month.join("_index.md"));
    assert!(index.contains("[[501 — Принтер HP не печатает]]") && !index.contains("[[501 — Принтер не печатает]]"), "{index}");

    // и чужие файлы не заставляют заявку переписываться каждый раз
    fake.reset();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 0, 1, 0), "{}", show(&r));
    assert!(fake.paths().is_empty(), "{:?}", fake.paths());
}

#[test]
fn a_duplicate_in_the_month_of_the_ticket_is_removed_and_one_in_another_month_stays() {
    // C#: выгрузка выбранных ищет прежние файлы в папке месяца заявки; обходить ради нескольких заявок все месяцы не стоит —
    // дубль в чужом месяце уберёт только выгрузка всех (4.3)
    let tmp = Tmp::new("dups");
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    run(&fake, &fake.rows(), tmp.path());
    let file = tmp.join("tickets/2026-09/501 — Принтер.md");
    assert!(file.is_file(), "первая выгрузка: {:?}", walk(tmp.path()));
    let same_month = tmp.join("tickets/2026-09/501 — Старое название.md");
    let elsewhere = tmp.join("tickets/2026-08/501 — Принтер.md");
    fs::copy(&file, &same_month).unwrap();
    put(&elsewhere, &text(&file));
    fake.reset();

    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    assert!(file.is_file() && !same_month.exists(), "остался один файл, на своём месте");
    assert!(elsewhere.is_file(), "дубль в другом месяце не ищется");
}

#[test]
fn a_flat_file_of_ours_directly_in_tickets_is_found_moved_into_its_month_and_the_old_one_removed() {
    // C# ScanNames: файлы прямо в tickets (прежняя плоская раскладка) ищутся всегда — это наш файл заявки, он «обновлён»
    let tmp = Tmp::new("flat");
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    run(&fake, &fake.rows(), tmp.path());
    let month_file = tmp.join("tickets/2026-09/501 — Принтер.md");
    let flat = tmp.join("tickets/501 — Принтер.md");
    fs::rename(&month_file, &flat).unwrap();
    fake.reset();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "наш плоский файл найден: обновлён, а не создан: {}", show(&r));
    assert!(month_file.is_file() && !flat.exists(), "файл — в месяце, плоский убран: {:?}", walk(tmp.path()));
}

#[test]
fn a_corrupt_file_in_the_place_of_the_ticket_is_rewritten_as_a_new_one() {
    // не наш по свойствам (испорчен или оборван) — у C# это «новый файл», а не «обновлён»: наших файлов заявки не было
    for (tag, broken) in [("corrupt", "испорчен"), ("cut", "---\nsource: intraservice\nid: 501\n")] {
        let tmp = Tmp::new(tag);
        let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
        run(&fake, &fake.rows(), tmp.path());
        let file = tmp.join("tickets/2026-09/501 — Принтер.md");
        assert!(file.is_file(), "{tag}: первая выгрузка: {:?}", walk(tmp.path()));
        fs::write(&file, broken).unwrap();
        fake.reset();
        let r = run(&fake, &fake.rows(), tmp.path());
        assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{tag}: {}", show(&r));
        assert_eq!(prop(&file, "source").as_deref(), Some("intraservice"), "{tag}");
        assert_eq!(fake.cards(), 1, "{tag}");
    }
}

#[test]
fn the_whole_history_is_read_page_by_page_into_the_file() {
    let tmp = Tmp::new("pages");
    let mut t = T::new(501, "Длинная переписка", Some("2026-09-12T10:00:00+03:00"));
    t.events = 120;
    let fake = Fake::new(vec![t]);
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{}", show(&r));
    let body = text(must_ticket_file(tmp.path(), "2026-09", 501));
    assert!(body.contains("\nзапись 0\n") && body.contains("\nзапись 60\n") && body.contains("\nзапись 119\n"), "переписка не вся");
    assert_eq!(fake.lives(), 3);
}

#[test]
fn titles_with_path_characters_stay_inside_the_month_folder() {
    let tmp = Tmp::new("paths");
    let dir = tmp.join("kb");
    let fake = Fake::new(vec![
        T::new(601, "../../escape", Some("2026-09-12T10:00:00+03:00")),
        T::new(602, "C:\\Windows\\x", Some("2026-09-12T10:00:00+03:00")),
        T::new(603, "a/b\\c", Some("2026-09-12T10:00:00+03:00")),
    ]);
    let r = run(&fake, &fake.rows(), &dir);
    assert_eq!(counts(&r), (3, 3, 0, 0, 0), "{}", show(&r));
    assert_eq!(names_in(tmp.path()), ["kb"], "рядом с папкой выгрузки ничего не появилось");
    assert_eq!(names_in(&dir), ["_index.md", "tickets"]);
    let month = names_in(&dir.join("tickets/2026-09"));
    assert_eq!(month.len(), 4, "три заявки и оглавление: {month:?}");
    assert!(month.iter().all(|n| !n.contains('/') && !n.contains('\\') && !n.contains(':')), "{month:?}");
    assert!(
        ticket_file(&dir, "2026-09", 601).is_some()
            && ticket_file(&dir, "2026-09", 602).is_some()
            && ticket_file(&dir, "2026-09", 603).is_some()
    );
}

#[test]
fn in_a_deep_folder_names_shrink_to_the_number_and_the_second_export_finds_them_unchanged() {
    // путь к tickets — 232 знака: от названий не остаётся ничего, чтобы путь не превысил 250 (MAX_PATH Windows — 260)
    let tmp = Tmp::new("deep");
    let base = tmp.path().to_string_lossy().chars().count();
    let dir = tmp.join(&format!("deep{}", "x".repeat(232 - TICKETS_FOLDER.len() - 1 - (base + 1 + 4))));
    assert_eq!(dir.join(TICKETS_FOLDER).to_string_lossy().chars().count(), 232);
    let fake = Fake::new(month_tickets(3, 10007, "2026-09"));
    let r = run(&fake, &fake.rows(), &dir);
    assert_eq!(counts(&r), (3, 3, 0, 0, 0), "{}", show(&r));
    let files = names_in(&dir.join("tickets/2026-09"));
    assert_eq!(files, ["10007.md", "10008.md", "10009.md", "_index.md"]);
    assert!(dir.join("tickets/2026-09/10009.md").to_string_lossy().chars().count() <= MAX_PATH_LENGTH);
    let r = run(&fake, &fake.rows(), &dir);
    assert_eq!(counts(&r), (3, 0, 0, 3, 0), "имена те же — без изменений: {}", show(&r));
}

// ---------- вложения ----------

fn with_files() -> Fake {
    let mut t = T::new(601, "Ошибка в программе", Some("2026-09-12T10:00:00+03:00"));
    t.files = vec![(9001, "ошибка.png".into()), (9002, "акт.pdf".into()), (9003, "большой.zip".into())];
    Fake::new(vec![t])
}

/// Базу готовит прежняя выгрузка: 9001 лежит под старым именем (переименуют), 9003 — крупный, 9002 нет; рядом — файл пользователя.
fn lay_files(dir: &Path) {
    put(&dir.join("files/2026-09/601/9001 — старое имя.png"), &"a".repeat(300));
    put(&dir.join("files-lfs/2026-09/601/9003 — большой.zip"), &"b".repeat(1536));
    put(&dir.join("files/2026-09/601/мои заметки.txt"), "моё");
}

#[test]
fn attachments_already_in_the_base_are_linked_renamed_and_never_downloaded() {
    let tmp = Tmp::new("files");
    lay_files(tmp.path());
    let fake = with_files();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{}", show(&r));

    let body = text(must_ticket_file(tmp.path(), "2026-09", 601));
    assert!(
        body.contains(
            "\n## Вложения\n\n- ![[files/2026-09/601/9001 — ошибка.png]] · 300 Б\n- акт.pdf · не скачано\n- [[files-lfs/2026-09/601/9003 — большой.zip]] · 1,5 КБ\n\n## Переписка\n"
        ),
        "{body}"
    );
    let file = must_ticket_file(tmp.path(), "2026-09", 601);
    assert_eq!((prop(&file, "files").as_deref(), prop(&file, "files_missing").as_deref()), (Some("3"), Some("1")));
    assert!(prop(&file, "revision").is_some(), "ничего не упало — версия есть");

    // лежащее под другим именем переименовано, остальное на месте
    let plain = tmp.join("files/2026-09/601");
    assert_eq!(names_in(&plain), ["9001 — ошибка.png", "мои заметки.txt"]);
    assert_eq!(text(plain.join("9001 — ошибка.png")), "a".repeat(300));
    assert_eq!(text(plain.join("мои заметки.txt")), "моё");
    assert_eq!(names_in(&tmp.join("files-lfs/2026-09/601")), ["9003 — большой.zip"]);

    // итог — о вложениях; на сервер — только карточка и история; правил LFS без нужды не пишем
    assert_eq!(r.error, "Вложения: уже были 2, не скачано (настройка «Вложения в базу знаний»): 1.");
    assert!(fake.paths().iter().all(|p| p.starts_with("/api/task/") || p.starts_with("/api/tasklifetime")), "{:?}", fake.paths());
    assert!(!tmp.join(".gitattributes").exists());
}

#[test]
fn an_unchanged_ticket_stays_unchanged_only_while_its_saved_attachments_are_in_the_base() {
    let tmp = Tmp::new("files-unchanged");
    lay_files(tmp.path());
    let fake = with_files();
    run(&fake, &fake.rows(), tmp.path());
    fake.reset();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), fake.paths().len()), ((1, 0, 0, 1, 0), 0), "всё на месте: {}", show(&r));

    // крупный файл удалили (антивирус, руками) — заявка не менялась, но в базе его не хватает: перечитана, в файле — «не скачано»
    fs::remove_file(tmp.join("files-lfs/2026-09/601/9003 — большой.zip")).unwrap();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 0, 1, 0, 0), "{}", show(&r));
    let file = must_ticket_file(tmp.path(), "2026-09", 601);
    let body = text(&file);
    assert!(body.contains("\n- большой.zip · не скачано\n"), "{body}");
    assert_eq!(prop(&file, "files_missing").as_deref(), Some("2"));
    // теперь файл записан как есть — и снова неизменен
    fake.reset();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), fake.paths().len()), ((1, 0, 0, 1, 0), 0), "{}", show(&r));
}

// ---------- оборванные записи и git ----------

#[test]
fn a_run_sweeps_stale_temps_of_ours_in_the_folders_it_writes_to_and_leaves_the_rest() {
    let tmp = Tmp::new("sweep");
    let stale = |rel: &str| {
        let path = tmp.join(rel);
        put(&path, "");
        age(&path, 3600);
        path
    };
    let gone = [
        stale("tickets/2026-09/501 — старое.md.1a2b.tmp"),
        stale("tickets/2026-09/_index.md.9a8b.tmp"),
        stale("tickets/505 — плоское.md.7c7c.tmp"),
        stale("_index.md.5e6f.tmp"),
        stale("tickets.csv.1c2d.tmp"),
        stale("SCHEMA.md.1c2d.tmp"),
        stale("CLAUDE.md.1c2d.tmp"),
    ];
    let stays = [
        stale("tickets/2026-09/501 — мои выводы.md.tmp"),
        stale("tickets/2026-09/заметка.tmp"),
        stale("tickets/2026-09/.syncthing.501 — VPN.md.tmp"),
        stale("tickets/2026-09/черновик.md.tmp"),
        stale("505 — заметка в корне.md.7d7d.tmp"), // корень — уже хранилище пользователя: убираем только оглавления
        stale("tickets/2026-05/501 — x.md.1a2b.tmp"), // месяц, куда выгрузка не пишет
    ];
    let fresh = tmp.join("tickets/2026-09/501 — пишется.md.3c4d.tmp");
    put(&fresh, "");
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{}", show(&r));
    for p in &gone {
        assert!(!p.exists(), "наш оборванный должен быть убран: {}", p.display());
    }
    for p in stays.iter().chain([&fresh]) {
        assert!(p.exists(), "чужой, свежий или в чужой папке — остаётся: {}", p.display());
    }
    // и следующей выгрузкой: оборванная запись бывает и посреди работы
    let later = stale("tickets/2026-09/501 — позже.md.2b2b.tmp");
    let later_root = stale("_index.md.6a6a.tmp");
    run(&fake, &fake.rows(), tmp.path());
    assert!(!later.exists() && !later_root.exists(), "следующая выгрузка снова убирает");
}

#[test]
fn an_export_into_a_git_repository_puts_tmp_into_the_gitignore_before_the_first_file() {
    let tmp = Tmp::new("git");
    fs::create_dir_all(tmp.join("repo/.git")).unwrap();
    let kb = tmp.join("repo/kb");
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    let seen = Arc::new(Mutex::new(String::new()));
    let (seen2, ignore) = (seen.clone(), kb.join(".gitignore"));
    fake.set_hook(move |_, _, _| *seen2.lock().unwrap() = text(&ignore));
    let r = run(&fake, &fake.rows(), &kb);
    assert_eq!((counts(&r), r.error.as_str()), ((1, 1, 0, 0, 0), ""), "{}", show(&r));
    assert_eq!(text(kb.join(".gitignore")), "*.tmp\n");
    assert_eq!(*seen.lock().unwrap(), "*.tmp\n", "к первой заявке строка уже записана");
}

#[test]
fn a_gitignore_that_cannot_be_written_does_not_stop_the_export_and_is_said_in_the_result() {
    let tmp = Tmp::new("git-stuck");
    fs::create_dir_all(tmp.join("repo/.git")).unwrap();
    let kb = tmp.join("repo/kb");
    fs::create_dir_all(kb.join(".gitignore")).unwrap();
    let fake = Fake::new(vec![T::new(501, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    let r = run(&fake, &fake.rows(), &kb);
    assert_eq!(r.created, 1, "{}", show(&r));
    assert!(r.error.contains("Папка — в git-репозитории, а не удалось дописать *.tmp в "), "{}", r.error);
    assert!(
        r.error.contains(": допишите строку *.tmp в этот .gitignore сами, иначе коммит может поймать недописанный файл."),
        "{}",
        r.error
    );
    assert!(!r.error.contains(".:"), "{}", r.error);
}

// ---------- сбои и повторы ----------

#[test]
fn a_ticket_that_cannot_be_read_is_counted_with_its_first_error_and_the_others_are_written() {
    let tmp = Tmp::new("unreadable");
    let fake = Fake::new(month_tickets(3, 901, "2026-09"));
    fake.set_fault(|kind, id, _| if kind == "card" && id == 902 { 404 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(
        (counts(&r), r.complete, r.first_error.as_str(), r.error.as_str()),
        ((3, 2, 0, 0, 1), true, "#902: заявка не найдена (HTTP 404)", ""),
        "{}",
        show(&r)
    );
    assert!(ticket_file(tmp.path(), "2026-09", 902).is_none() && ticket_file(tmp.path(), "2026-09", 901).is_some());
    assert_eq!(fake.lives(), 2, "историю заявки, чья карточка не прочиталась, не просят");
    assert_eq!(
        summary(&r, "Выбрано"),
        "Готово. Выбрано — 3: новых файлов 2, обновлено 0, без изменений 0.\nНе выгружены: 1 — повторная выгрузка попробует их снова. Первая ошибка — #902: заявка не найдена (HTTP 404)"
    );
}

#[test]
fn a_network_failure_is_retried_after_the_pauses_of_the_options() {
    let tmp = Tmp::new("retry-pause");
    let fake = Fake::new(vec![T::new(901, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    fake.set_fault(|kind, _, attempt| if kind == "card" && attempt <= 2 { 503 } else { 0 });
    let options = Options { retry_delays: vec![Duration::from_millis(250), Duration::from_secs(1)], vanish_wait: Duration::ZERO };
    let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &|_| {});
    assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{}", show(&r));
    assert_eq!((fake.cards(), fake.lives()), (3, 1));
    let times = fake.card_times();
    assert!(times[1] - times[0] >= Duration::from_millis(250), "первая пауза — первая из options: {:?}", times[1] - times[0]);
    assert!(times[1] - times[0] < Duration::from_secs(1), "первая пауза не самая длинная: {:?}", times[1] - times[0]);
    assert!(times[2] - times[1] >= Duration::from_secs(1), "вторая пауза — вторая из options: {:?}", times[2] - times[1]);
}

#[test]
fn a_history_page_that_fails_once_is_retried_too() {
    let tmp = Tmp::new("retry-life");
    let fake = Fake::new(vec![T::new(901, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    fake.set_fault(|kind, _, attempt| if kind == "life" && attempt == 1 { 500 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{}", show(&r));
    assert_eq!((fake.cards(), fake.lives()), (1, 2));
}

#[test]
fn error_500_is_retried_once_only() {
    let tmp = Tmp::new("retry-500");
    let fake = Fake::new(vec![T::new(901, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    fake.set_fault(|kind, _, _| if kind == "card" { 500 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.first_error.as_str()), ((1, 0, 0, 0, 1), "#901: ошибка сервера (HTTP 500)"), "{}", show(&r));
    assert_eq!(fake.cards(), 2, "один повтор");
}

#[test]
fn other_server_errors_use_every_pause_and_then_the_ticket_fails() {
    let tmp = Tmp::new("retry-503");
    let fake = Fake::new(vec![T::new(901, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
    fake.set_fault(|kind, _, _| if kind == "card" { 503 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.first_error.as_str()), ((1, 0, 0, 0, 1), "#901: ошибка сервера (HTTP 503)"), "{}", show(&r));
    assert_eq!(fake.cards(), 4, "запрос и три повтора");
    assert!(walk(tmp.path()).is_empty(), "файлов нет: {:?}", walk(tmp.path()));
}

#[test]
fn not_found_and_no_access_are_not_retried() {
    let tmp = Tmp::new("retry-never");
    let fake =
        Fake::new(vec![T::new(901, "Принтер", Some("2026-09-12T10:00:00+03:00")), T::new(902, "Почта", Some("2026-09-12T10:00:00+03:00"))]);
    fake.set_fault(|kind, id, _| match (kind, id) {
        ("card", 901) => 404,
        ("card", 902) => 403,
        _ => 0,
    });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (2, 0, 0, 0, 2), "{}", show(&r));
    assert_eq!((fake.attempts("card", 901), fake.attempts("card", 902)), (1, 1));
    assert!(r.first_error == "#901: заявка не найдена (HTTP 404)" || r.first_error == "#902: нет доступа (HTTP 403)", "{}", r.first_error);
}

#[test]
fn the_number_of_retries_follows_the_options() {
    for (delays, errors, cards) in [(1usize, 503u16, 2usize), (0, 503, 1), (2, 503, 3), (1, 500, 2), (0, 500, 1)] {
        let tmp = Tmp::new("retry-count");
        let fake = Fake::new(vec![T::new(901, "Принтер", Some("2026-09-12T10:00:00+03:00"))]);
        fake.set_fault(move |kind, _, _| if kind == "card" { errors } else { 0 });
        let options = Options { retry_delays: vec![Duration::from_millis(1); delays], vanish_wait: Duration::ZERO };
        let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &|_| {});
        assert_eq!((r.failed, fake.cards()), (1, cards), "пауз {delays}, ответ {errors}: {}", show(&r));
    }
}

#[test]
fn a_history_page_that_keeps_failing_leaves_no_file_for_the_ticket() {
    // половина заявки хуже, чем никакой: файл потом сочли бы неизменным
    let tmp = Tmp::new("half");
    let mut t = T::new(901, "Длинная переписка", Some("2026-09-12T10:00:00+03:00"));
    t.events = 120;
    let fake = Fake::new(vec![t]);
    fake.set_fault({
        let seen = Arc::new(AtomicUsize::new(0));
        move |kind, _, _| if kind == "life" && seen.fetch_add(1, Ordering::SeqCst) >= 1 { 503 } else { 0 }
    });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.first_error.as_str()), ((1, 0, 0, 0, 1), "#901: ошибка сервера (HTTP 503)"), "{}", show(&r));
    assert!(ticket_file(tmp.path(), "2026-09", 901).is_none());
    assert_eq!(fake.lives(), 5, "первая страница и вторая с тремя повторами");
}

// ---------- четыре разом ----------

#[test]
fn four_tickets_are_read_at_once_and_not_more() {
    let tmp = Tmp::new("parallel");
    let fake = Fake::new(month_tickets(12, 1001, "2026-09"));
    let arrived = Arc::new((Mutex::new(0usize), Condvar::new()));
    let timed_out = Arc::new(AtomicBool::new(false));
    let (a2, t2) = (arrived.clone(), timed_out.clone());
    // первые четыре карточки ждут, пока не придут все четыре (иначе вперёд не пройти — «разом» не получилось); остальные
    // отвечают не сразу, чтобы лишние работники успели бы наложиться
    fake.set_hook(move |_, _, seq| {
        if seq <= 4 {
            let (m, cv) = &*a2;
            let mut n = m.lock().unwrap();
            *n += 1;
            cv.notify_all();
            let (_n, res) = cv.wait_timeout_while(n, Duration::from_secs(10), |n| *n < 4).unwrap();
            if res.timed_out() {
                t2.store(true, Ordering::SeqCst);
            }
        } else {
            thread::sleep(Duration::from_millis(30));
        }
    });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert!(!timed_out.load(Ordering::SeqCst), "четыре заявки одновременно так и не набрались");
    assert_eq!(counts(&r), (12, 12, 0, 0, 0), "{}", show(&r));
    assert!(
        fake.inner.max_in_flight.load(Ordering::SeqCst) <= 4,
        "одновременных запросов: {}",
        fake.inner.max_in_flight.load(Ordering::SeqCst)
    );
}

// ---------- выключатель ----------

fn all_cards(code: u16) -> impl Fn(&str, i32, usize) -> u16 + Send + Sync + 'static {
    move |kind, _, _| if kind == "card" { code } else { 0 }
}

/// Номер заявки из «последняя — #N» и сколько обработано, из итога прерванной выгрузки; обе — в пределах выгрузки.
fn tripped(r: &ExportResult, head: &str, target: usize, first: i32) -> i32 {
    let caps = after_trip(head, target).captures(&r.error).unwrap_or_else(|| panic!("итог не такой, как у C#:\n{}", show(r)));
    assert_eq!(caps[caps.len() - 1].parse::<usize>().unwrap(), r.found, "«обработано» — столько, сколько в итоге: {}", show(r));
    let id: i32 = caps[1].parse().unwrap();
    assert!((first..first + target as i32).contains(&id), "последняя — #{id}");
    id
}

#[test]
fn a_refused_login_stops_the_export_at_once() {
    let tmp = Tmp::new("trip-401");
    let fake = Fake::new(month_tickets(40, 1001, "2026-09"));
    fake.set_fault(all_cards(401));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = regex_lite::escape("Выгрузка прервана: сервер не принимает логин и пароль (HTTP 401) — проверьте их в настройках.");
    assert!(after_trip(&head, 40).is_match(&r.error), "{}", show(&r));
    assert!(!r.complete && r.created == 0 && (1..=4).contains(&r.failed), "{}", show(&r));
    assert_eq!(r.found, r.failed, "{}", show(&r));
    assert!(fake.cards() <= 4, "401 не повторяют; после остановки заявки не берут: {}", fake.cards());
    assert!(r.first_error.ends_with(": неверный логин или пароль (HTTP 401)") && r.first_error.starts_with('#'), "{}", r.first_error);
}

#[test]
fn thirty_network_failures_in_a_row_stop_the_export() {
    let tmp = Tmp::new("trip-transient");
    let fake = Fake::new(month_tickets(60, 1001, "2026-09"));
    fake.set_fault(all_cards(503));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = r"Выгрузка прервана: 30 заявок подряд не прочитались из-за сети или сервера \(последняя — #(\d+): ошибка сервера \(HTTP 503\)\)\. Проверьте связь; если компьютер засыпал — не давайте ему\.";
    tripped(&r, head, 60, 1001);
    assert!(!r.complete && r.created == 0 && (30..38).contains(&r.failed), "{}", show(&r));
    assert!(fake.cards() <= (30 + 8) * 4, "повторы — на каждую заявку, но не бесконечно: {}", fake.cards());
    assert!(names_in(tmp.path()).iter().all(|n| n != "_index.md"), "ничего не записано — оглавлений нет");
}

#[test]
fn exactly_thirty_network_failures_before_any_success_stop_the_export() {
    let tmp = Tmp::new("trip-transient-exact");
    // только сбойные: после них нет ни одной заявки, чья удача могла бы сбросить счёт подряд
    let fake = Fake::new(month_tickets(30, 1001, "2026-09"));
    fake.set_fault(all_cards(503));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = r"Выгрузка прервана: 30 заявок подряд не прочитались из-за сети или сервера \(последняя — #(\d+): ошибка сервера \(HTTP 503\)\)\. Проверьте связь; если компьютер засыпал — не давайте ему\.";
    tripped(&r, head, 30, 1001);
    assert!(!r.complete && r.failed == 30, "тридцатая — последняя капля: {}", show(&r));
}

/// Три волны: сбои, потом удачи, потом снова сбои — всего их порог, но подряд — вдвое меньше: удача между ними сбрасывает счёт.
fn waves(fake: &Fake, dir: &Path, failing: i32, answered: fn(&Inner, i32) -> bool, good_month: &str) {
    let (first, good, second) = (1001..1001 + failing, 2001..2003, 3001..3001 + failing);
    let first2 = first.clone();
    let base = dir.to_path_buf();
    let month = good_month.to_string();
    fake.gates(vec![
        (good.clone(), Box::new(move |inner: &Inner| first2.clone().all(|id| answered(inner, id)))),
        // третья волна ждёт, пока четыре её запроса не придут: работник просит следующую карточку, только когда прежняя заявка
        // сосчитана до конца, — значит, и обе удачные заявки уже сбросили счёт подряд (а не только легли файлом)
        (
            second.clone(),
            Box::new(move |inner: &Inner| {
                (2001..2003).all(|id| ticket_file(&base, &month, id).is_some())
                    && second.clone().filter(|id| inner.attempts_of("card", *id) >= 1).count() >= 4
            }),
        ),
    ]);
}

#[test]
fn network_failures_are_counted_in_a_row_a_success_between_starts_the_count_anew() {
    let tmp = Tmp::new("waves-transient");
    let mut tickets = month_tickets(15, 1001, "2026-09");
    tickets.extend(month_tickets(2, 2001, "2026-09"));
    tickets.extend(month_tickets(15, 3001, "2026-09"));
    let fake = Fake::new(tickets);
    fake.set_fault(|kind, id, _| if kind == "card" && !(2000..=3000).contains(&id) { 503 } else { 0 });
    waves(&fake, tmp.path(), 15, |inner, id| inner.attempts_of("card", id) >= 4, "2026-09");
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((32, 2, 0, 0, 30), true, ""), "{}", show(&r));
}

#[test]
fn server_errors_are_counted_in_a_row_a_success_between_starts_the_count_anew() {
    let tmp = Tmp::new("waves-500");
    let mut tickets = month_tickets(50, 1001, "2026-09");
    tickets.extend(month_tickets(2, 2001, "2026-09"));
    tickets.extend(month_tickets(50, 3001, "2026-09"));
    let fake = Fake::new(tickets);
    fake.set_fault(|kind, id, _| if kind == "card" && !(2000..=3000).contains(&id) { 500 } else { 0 });
    waves(&fake, tmp.path(), 50, |inner, id| inner.attempts_of("card", id) >= 2, "2026-09");
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((102, 2, 0, 0, 100), true, ""), "{}", show(&r));
}

#[test]
fn unwritable_files_are_counted_in_a_row_a_written_one_between_starts_the_count_anew() {
    let tmp = Tmp::new("waves-write");
    put(&tmp.join("tickets/2026-08"), "я файл, а не папка");
    let mut tickets = month_tickets(5, 1001, "2026-08");
    tickets.extend(month_tickets(2, 2001, "2026-09"));
    tickets.extend(month_tickets(5, 3001, "2026-08"));
    let fake = Fake::new(tickets);
    waves(&fake, tmp.path(), 5, |inner, id| inner.attempts_of("life1", id) >= 1, "2026-09");
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((12, 2, 0, 0, 10), true, ""), "{}", show(&r));
}

#[test]
fn twenty_nine_network_failures_do_not() {
    let tmp = Tmp::new("no-trip-transient");
    let fake = Fake::new(month_tickets(60, 1001, "2026-09"));
    fake.set_fault(|kind, id, _| if kind == "card" && id < 1001 + 29 { 503 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((60, 31, 0, 0, 29), true, ""), "{}", show(&r));
}

#[test]
fn a_hundred_server_errors_in_a_row_stop_the_export() {
    let tmp = Tmp::new("trip-500");
    let fake = Fake::new(month_tickets(130, 1001, "2026-09"));
    fake.set_fault(all_cards(500));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = r"Выгрузка прервана: 100 заявок подряд сервер ответил ошибкой 500 \(последняя — #(\d+): ошибка сервера \(HTTP 500\)\)\. Похоже, он не справляется или сломан: подождите и запустите выгрузку снова\.";
    tripped(&r, head, 130, 1001);
    assert!(!r.complete && r.created == 0 && (100..108).contains(&r.failed), "{}", show(&r));
}

#[test]
fn exactly_a_hundred_server_errors_before_any_success_stop_the_export() {
    let tmp = Tmp::new("trip-500-exact");
    let fake = Fake::new(month_tickets(100, 1001, "2026-09"));
    fake.set_fault(all_cards(500));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = r"Выгрузка прервана: 100 заявок подряд сервер ответил ошибкой 500 \(последняя — #(\d+): ошибка сервера \(HTTP 500\)\)\. Похоже, он не справляется или сломан: подождите и запустите выгрузку снова\.";
    tripped(&r, head, 100, 1001);
    assert!(!r.complete && r.failed == 100, "{}", show(&r));
}

#[test]
fn a_damaged_run_of_sixty_tickets_with_error_500_does_not_stop_the_export() {
    // иначе дальше битого места не пройти ни при каком повторе; каждой — один повтор, не три
    let tmp = Tmp::new("damaged");
    let fake = Fake::new(month_tickets(130, 1001, "2026-09"));
    fake.set_fault(|kind, id, _| if kind == "card" && (1041..=1100).contains(&id) { 500 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((130, 70, 0, 0, 60), true, ""), "{}", show(&r));
    assert!(r.first_error.ends_with(": ошибка сервера (HTTP 500)"), "{}", r.first_error);
    assert_eq!((1041..=1100).map(|id| fake.attempts("card", id)).sum::<usize>(), 120);
}

#[test]
fn twenty_five_refusals_without_a_single_success_stop_the_export() {
    let tmp = Tmp::new("trip-404");
    let fake = Fake::new(month_tickets(60, 1001, "2026-09"));
    fake.set_fault(all_cards(404));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = r"Выгрузка прервана: ни одна из 25 заявок не прочиталась \(последняя — #(\d+): заявка не найдена \(HTTP 404\)\)\.";
    tripped(&r, head, 60, 1001);
    assert!(!r.complete && r.created == 0 && (25..33).contains(&r.failed), "{}", show(&r));
    assert!(!r.error.contains("Первая ошибка"), "{}", r.error);
}

#[test]
fn exactly_twenty_five_refusals_before_any_success_stop_the_export() {
    let tmp = Tmp::new("trip-404-exact");
    let fake = Fake::new(month_tickets(25, 1001, "2026-09"));
    fake.set_fault(all_cards(404));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head = r"Выгрузка прервана: ни одна из 25 заявок не прочиталась \(последняя — #(\d+): заявка не найдена \(HTTP 404\)\)\.";
    tripped(&r, head, 25, 1001);
    assert!(!r.complete && r.failed == 25 && r.created == 0, "{}", show(&r));
}

#[test]
fn twenty_four_refusals_do_not_stop_it() {
    let tmp = Tmp::new("no-trip-404");
    let fake = Fake::new(month_tickets(60, 1001, "2026-09"));
    fake.set_fault(|kind, id, _| if kind == "card" && id < 1001 + 24 { 404 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((60, 36, 0, 0, 24), true, ""), "{}", show(&r));
}

#[test]
fn refusals_after_a_success_do_not_stop_it_however_many_there_are() {
    // «ни одной удачной»: три заявки уже выгружены и не менялись, сорок следующих сервер не знает
    let tmp = Tmp::new("rescued");
    let mut tickets = month_tickets(3, 1001, "2026-09");
    tickets.extend(month_tickets(40, 2001, "2026-09"));
    let fake = Fake::new(tickets);
    let good: Vec<Found> = (1001..1004).map(|id| fake.row(id)).collect();
    run(&fake, &good, tmp.path());
    fake.reset();
    fake.set_fault(|kind, id, _| if kind == "card" && id >= 2001 { 404 } else { 0 });
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((43, 0, 0, 3, 40), true, ""), "{}", show(&r));
}

#[test]
fn ten_files_in_a_row_that_cannot_be_written_stop_the_export() {
    let tmp = Tmp::new("trip-write");
    put(&tmp.join("tickets/2026-09"), "я файл, а не папка");
    let fake = Fake::new(month_tickets(30, 1001, "2026-09"));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head =
        r"Выгрузка прервана: файлы не записываются \(10 подряд\) — .+\. Проверьте место на диске, доступ к папке и длину пути к ней\.";
    let caps = after_trip(head, 30).captures(&r.error).unwrap_or_else(|| panic!("итог не такой, как у C#:\n{}", show(&r)));
    assert_eq!(caps[1].parse::<usize>().unwrap(), r.found, "{}", show(&r));
    assert!(!r.complete && r.created == 0 && (10..18).contains(&r.failed), "{}", show(&r));
    let first = Regex::new(r"^#\d+: файл не записан: .+$").unwrap();
    assert!(first.is_match(&r.first_error), "{}", r.first_error);
}

#[test]
fn exactly_ten_files_that_cannot_be_written_before_any_success_stop_the_export() {
    let tmp = Tmp::new("trip-write-exact");
    put(&tmp.join("tickets/2026-08"), "я файл, а не папка");
    let fake = Fake::new(month_tickets(10, 1001, "2026-08"));
    let r = run(&fake, &fake.rows(), tmp.path());
    let head =
        r"Выгрузка прервана: файлы не записываются \(10 подряд\) — .+\. Проверьте место на диске, доступ к папке и длину пути к ней\.";
    assert!(after_trip(head, 10).is_match(&r.error), "{}", show(&r));
    assert!(!r.complete && r.failed == 10, "{}", show(&r));
}

#[test]
fn nine_files_that_cannot_be_written_do_not() {
    let tmp = Tmp::new("no-trip-write");
    put(&tmp.join("tickets/2026-08"), "я файл, а не папка");
    let mut tickets = month_tickets(9, 1001, "2026-08");
    tickets.extend(month_tickets(30, 2001, "2026-09"));
    let fake = Fake::new(tickets);
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((39, 30, 0, 0, 9), true, ""), "{}", show(&r));
    assert!(r.first_error.starts_with('#') && r.first_error.contains(": файл не записан: "), "{}", r.first_error);
    assert!(text(tmp.join("_index.md")).contains("заявок: 30."), "оглавление — по месяцу, куда записано");
}

#[test]
fn a_write_failure_names_the_path_it_could_not_write_to() {
    // C#: ex.Message называет путь («Could not find a part of the path '…'»); голый текст ОС («Not a directory») — нет
    let tmp = Tmp::new("write-path-folder");
    // по частям: в тексте ошибки Windows путь с обратными чертами, «tickets/2026-09» в нём не найти
    let month = tmp.join("tickets").join("2026-09");
    put(&month, "я файл, а не папка");
    let fake = Fake::new(month_tickets(1, 1001, "2026-09"));
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(r.failed, 1, "{}", show(&r));
    assert!(
        r.first_error.starts_with("#1001: файл не записан: ") && r.first_error.contains(&month.display().to_string()),
        "первая ошибка называет путь {}: {}",
        month.display(),
        r.first_error
    );
}

#[test]
fn a_write_failure_over_a_directory_in_place_of_the_file_names_the_file() {
    let tmp = Tmp::new("write-path-file");
    let fake = Fake::new(month_tickets(1, 1001, "2026-09"));
    let file = tmp.join("tickets/2026-09/1001 — Заявка 1001.md");
    fs::create_dir_all(&file).unwrap();
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(r.failed, 1, "{}", show(&r));
    assert!(r.first_error.contains("1001 — Заявка 1001.md"), "первая ошибка называет файл: {}", r.first_error);
}

// ---------- «Остановить» ----------

#[test]
fn a_stop_before_the_start_writes_nothing_and_touches_nothing() {
    let tmp = Tmp::new("stop-before");
    let leftover = tmp.join("_index.md.4c4c.tmp");
    put(&leftover, "");
    age(&leftover, 3600);
    let fake = Fake::new(month_tickets(3, 1001, "2026-09"));
    let stop = AtomicBool::new(true);
    let r = run_with(&fake, &fake.rows(), tmp.path(), &stop, &fast(), &|_| {});
    assert_eq!((r.found, r.complete, r.error.as_str()), (0, false, "Остановлено до начала — ничего не записано."), "{}", show(&r));
    assert!(fake.paths().is_empty() && leftover.exists() && !tmp.join("tickets").exists(), "ничего не тронуто: {:?}", walk(tmp.path()));
    assert_eq!(summary(&r, "Выбрано"), "Остановлено до начала — ничего не записано.");
}

#[test]
fn a_stop_in_the_middle_keeps_what_was_written_with_its_indexes_and_the_next_run_continues() {
    let tmp = Tmp::new("stop");
    let fake = Fake::new(month_tickets(40, 1001, "2026-09"));
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    fake.set_hook(move |_, _, seq| {
        if seq == 30 {
            s2.store(true, Ordering::SeqCst);
        }
    });
    // остановили — ждать срока проверки записанного не нужно: итог сразу
    let options = Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::from_secs(3) };
    let lines = Mutex::new(Vec::<String>::new());
    let progress = |line: String| lines.lock().unwrap().push(line);
    let r = run_with(&fake, &fake.rows(), tmp.path(), &stop, &options, &progress);
    assert!(!lines.lock().unwrap().iter().any(|l| l.starts_with("Проверяю")), "{:?}", lines.lock().unwrap());
    let tail = format!(
        "Остановлено — что успели, сохранено (обработано {} из 40). Запустите выгрузку снова: готовое пропустится, она продолжится с этого места.",
        r.found
    );
    assert_eq!((r.complete, r.error.as_str()), (false, tail.as_str()), "{}", show(&r));
    assert!(r.created >= 20 && r.created < 40 && r.failed == 0, "{}", show(&r));
    assert_eq!(r.found, r.created + r.updated + r.unchanged + r.failed, "{}", show(&r));
    assert!(fake.cards() < 40, "после остановки заявки не берут: {}", fake.cards());
    let written = names_in(&tmp.join("tickets/2026-09")).iter().filter(|n| *n != "_index.md").count();
    assert_eq!(written, r.created, "файлов — столько, сколько создано");
    assert!(
        text(tmp.join("_index.md")).contains(&format!("заявок: {}.", r.created)),
        "оглавления — по записанному: {}",
        text(tmp.join("_index.md"))
    );
    assert!(summary(&r, "Выбрано").starts_with(&format!("Не закончено. Выбрано, обработано — {}: ", r.found)));

    // повторный запуск: готовое пропускается, читается только недостающее
    let done = r.created;
    fake.reset();
    let again = run(&fake, &fake.rows(), tmp.path());
    assert_eq!((counts(&again), again.complete, again.error.as_str()), ((40, 40 - done, 0, done, 0), true, ""), "{}", show(&again));
    assert_eq!(fake.cards(), 40 - done);
}

// ---------- записанное и пропавшее ----------

fn files_of_month(dir: &Path) -> Vec<(i32, PathBuf)> {
    let folder = dir.join("tickets/2026-09");
    names_in(&folder)
        .into_iter()
        .filter(|n| n.ends_with(".md") && n != "_index.md") // временные «….md.1a2b.tmp» рядом — не файлы заявок
        .map(|n| (n.split(' ').next().unwrap().trim_end_matches(".md").parse().unwrap(), folder.join(n)))
        .collect()
}

const VANISHED_TAIL: &str = "Похоже, их удаляет антивирус — посмотрите его журнал или карантин («Безопасность Windows» → «Журнал защиты»), добавьте папку выгрузки в исключения и выгрузите снова.";

/// Выгрузить 40 заявок; на 30-й карточке «антивирус» удаляет столько записанных файлов. Номера удалённых — по порядку.
fn run_with_vanishing(tag: &str, how_many: usize) -> (ExportResult, Vec<i32>, Tmp) {
    let tmp = Tmp::new(tag);
    let fake = Fake::new(month_tickets(40, 1001, "2026-09"));
    let removed = Arc::new(Mutex::new(Vec::new()));
    let (removed2, base) = (removed.clone(), tmp.path().to_path_buf());
    fake.set_hook(move |_, _, seq| {
        if seq == 30 {
            for (id, path) in files_of_month(&base).into_iter().take(how_many) {
                fs::remove_file(path).unwrap();
                removed2.lock().unwrap().push(id);
            }
        }
    });
    let r = run(&fake, &fake.rows(), tmp.path());
    let mut ids = removed.lock().unwrap().clone();
    ids.sort();
    (r, ids, tmp)
}

#[test]
fn a_written_file_that_is_gone_at_the_end_is_named_and_the_export_is_not_finished() {
    let (r, ids, tmp) = run_with_vanishing("vanish", 3);
    assert_eq!(ids.len(), 3);
    let named = ids.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ");
    assert_eq!(r.error, format!("Записано, но уже нет на диске: 3 ({named}). {VANISHED_TAIL}"), "{}", show(&r));
    assert!(!r.complete && r.created == 40 && r.failed == 0, "считается записанным, а итог — «не закончено»: {}", show(&r));
    assert!(text(tmp.join("_index.md")).contains("заявок: 37."), "оглавления — по тому, что на диске");
}

#[test]
fn more_than_ten_vanished_files_are_named_ten_and_counted() {
    let (r, ids, _tmp) = run_with_vanishing("vanish-many", 12);
    assert_eq!(ids.len(), 12);
    let named = ids.iter().take(10).map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ");
    assert_eq!(r.error, format!("Записано, но уже нет на диске: 12 ({named} и ещё 2). {VANISHED_TAIL}"), "{}", show(&r));
}

#[test]
fn a_file_that_goes_while_the_end_waits_is_found_after_the_wait() {
    // антивирус удаляет не в тот же миг: итог ждёт vanish_wait — и только потом смотрит; ждём по условию — по ходу, который
    // выгрузка сама присылает перед ожиданием
    let tmp = Tmp::new("vanish-late");
    let fake = Fake::new(month_tickets(6, 1001, "2026-09"));
    let options = Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::from_secs(1) };
    let lines = Mutex::new(Vec::<String>::new());
    let removed = Mutex::new(Vec::<i32>::new());
    let base = tmp.path().to_path_buf();
    let progress = |line: String| {
        if line == "Проверяю, что файлы на месте…"
            && let Some((id, path)) = files_of_month(&base).into_iter().next()
        {
            fs::remove_file(path).unwrap();
            removed.lock().unwrap().push(id);
        }
        lines.lock().unwrap().push(line);
    };
    let started = Instant::now();
    let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &progress);
    assert!(started.elapsed() >= Duration::from_secs(1), "выгрузка не ждала vanish_wait: {:?}", started.elapsed());
    let id = removed.lock().unwrap().first().copied();
    assert!(id.is_some(), "перед ожиданием ход «Проверяю, что файлы на месте…» не пришёл: {:?}", lines.lock().unwrap());
    let id = id.unwrap();
    assert_eq!(r.error, format!("Записано, но уже нет на диске: 1 (#{id}). {VANISHED_TAIL}"), "{}", show(&r));
    assert!(!r.complete && r.created == 6, "{}", show(&r));
    let lines = lines.lock().unwrap();
    assert_eq!(lines.iter().filter(|l| *l == "Проверяю, что файлы на месте…").count(), 1, "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("Выгружено 6 из 6"), "после ожидания — снова ход: {lines:?}");
}

// ---------- ход ----------

#[test]
fn the_progress_line_counts_written_and_not_written_tickets() {
    let tmp = Tmp::new("progress");
    let fake = Fake::new(month_tickets(3, 1001, "2026-09"));
    fake.set_fault(|kind, id, _| if kind == "card" && id == 1002 { 404 } else { 0 });
    let options = Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::from_secs(1) };
    let lines = Mutex::new(Vec::<String>::new());
    let progress = |line: String| lines.lock().unwrap().push(line);
    let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &progress);
    assert_eq!(counts(&r), (3, 2, 0, 0, 1), "{}", show(&r));
    let lines = lines.lock().unwrap();
    assert_eq!(lines.last().map(String::as_str), Some("Выгружено 2 из 3 · не выгружено 1"), "{lines:?}");
    let at = lines.iter().position(|l| l == "Проверяю, что файлы на месте…");
    assert!(at.is_some_and(|i| i + 1 < lines.len()), "перед ожиданием — «Проверяю…»: {lines:?}");
    assert!(lines.iter().all(|l| l.starts_with("Выгружено ") || l == "Проверяю, что файлы на месте…"), "{lines:?}");
}

#[test]
fn an_empty_list_is_finished_at_once_and_the_summary_says_nothing_was_found() {
    let tmp = Tmp::new("empty");
    let fake = Fake::new(Vec::new());
    let r = run(&fake, &[], tmp.path());
    assert_eq!((counts(&r), r.complete, r.error.as_str()), ((0, 0, 0, 0, 0), true, ""), "{}", show(&r));
    assert!(fake.paths().is_empty());
    assert_eq!(summary(&r, "Выбрано"), "Под отбор не попало ни одной заявки.");
}

#[test]
fn the_progress_goes_at_most_four_times_a_second_and_still_goes() {
    // шестьдесят заявок, каждая читается заметное время: без отсечки строк было бы шестьдесят; предел — по прошедшему времени
    let tmp = Tmp::new("throttle");
    let fake = Fake::new(month_tickets(60, 1001, "2026-09"));
    fake.set_hook(|_, _, _| thread::sleep(Duration::from_millis(40)));
    let options = Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::from_millis(300) };
    let lines = Mutex::new(Vec::<String>::new());
    let progress = |line: String| lines.lock().unwrap().push(line);
    let started = Instant::now();
    let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &progress);
    let elapsed_ms = started.elapsed().as_millis() as usize;
    assert_eq!(counts(&r), (60, 60, 0, 0, 0), "{}", show(&r));
    let lines = lines.lock().unwrap();
    // «Проверяю…» и итоговый ход после ожидания идут без отсечки — их двое
    let forced = 2;
    assert!(lines.len() <= elapsed_ms / 250 + forced + 2, "строк хода {} за {elapsed_ms} мс: {lines:?}", lines.len());
    let before_check = lines.iter().position(|l| l == "Проверяю, что файлы на месте…").expect("«Проверяю…» пришла");
    assert!(before_check >= 1, "за время выгрузки (больше полусекунды) ход приходил: {lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("Выгружено 60 из 60"), "{lines:?}");
}

#[test]
fn the_progress_line_counts_created_updated_and_unchanged_together() {
    let tmp = Tmp::new("progress-sum");
    let fake = Fake::new(month_tickets(3, 1001, "2026-09"));
    let first: Vec<Found> = (1001..1003).map(|id| fake.row(id)).collect();
    run(&fake, &first, tmp.path());
    fake.edit(1002, |t| t.changed = "2026-09-21T12:00:00+03:00".into());
    let options = Options { retry_delays: vec![Duration::from_millis(1); 3], vanish_wait: Duration::from_millis(200) };
    let lines = Mutex::new(Vec::<String>::new());
    let progress = |line: String| lines.lock().unwrap().push(line);
    let r = run_with(&fake, &fake.rows(), tmp.path(), &AtomicBool::new(false), &options, &progress);
    assert_eq!(counts(&r), (3, 1, 1, 1, 0), "по одной заявке каждого рода: {}", show(&r));
    let lines = lines.lock().unwrap();
    assert_eq!(lines.last().map(String::as_str), Some("Выгружено 3 из 3"), "{lines:?}");
}

// ---------- история без номеров страниц ----------

fn history_requests(events: usize) -> (usize, String) {
    let tmp = Tmp::new("unpaged");
    let mut t = T::new(1001, "Заявка 1001", Some("2026-09-05T10:00:00+03:00"));
    (t.events, t.paged) = (events, false);
    let fake = Fake::new(vec![t]);
    let r = run(&fake, &fake.rows(), tmp.path());
    assert_eq!(counts(&r), (1, 1, 0, 0, 0), "{}", show(&r));
    (fake.lives(), text(must_ticket_file(tmp.path(), "2026-09", 1001)))
}

#[test]
fn a_history_without_page_numbers_ends_on_a_short_page() {
    let (requests, file) = history_requests(3);
    assert_eq!(requests, 1, "три записи из пятидесяти — страница неполная, больше не просим");
    assert!(file.contains("запись 0") && file.contains("запись 2"), "{file}");
}

#[test]
fn a_history_without_page_numbers_asks_for_the_next_page_after_a_full_one() {
    let (requests, file) = history_requests(50);
    assert_eq!(requests, 2, "полная страница — просим следующую; она пуста — конец");
    assert!(file.contains("запись 49"), "{file}");
    let (requests, file) = history_requests(120);
    assert_eq!(requests, 3, "50 + 50 + 20");
    assert!(file.contains("запись 119"), "{file}");
}
