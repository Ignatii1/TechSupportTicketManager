//! Один заход автовыгрузки (`ClosedExport.RunAsync`, карточка 4.5-2): настоящий клиент и движок выгрузки против поддельного
//! сервера на loopback — что заход спрашивает у сервера, докуда двигает отметку, кого берёт, кого запоминает «не моей», кого
//! считает сбойной и пропускает. Сценарии — из `TicketBoard.SelfCheck/AutoExportCheck.cs` (блок наблюдателей, 2, 4, 5, 5б, 6, 7а,
//! 7б, 7в, 7г, 7д, 7е, 7з, 9, 9а, 9б, 9в, 9г) и `MirrorCheck.cs` (1, 3, 4, 5, 7) на уровне `run_pass`: отметку, «не мои» и счёт
//! сбоев заход получает и отдаёт значениями (в C# их хранит окно — то, что окно сохраняет в settings.json, — карточка 4.5-3).
//! Правила без сервера (`ExportMark`, `ExportFailures`, `of_ticket`, …) — `autoexport_rules.rs`, здесь их не проверяют.
//! Не порт: выключатель и «ни одного запроса» (окно), настройки и лог (окно), подсказка трея, правка доски; 7ж — «заход ждёт
//! другую выгрузку» — на двух уровнях: порядок запросов (здесь) и ход «Жду…» движка (`export_turn.rs`).
//!
//! Поддельный сервер честен, как живой: список — по `StatusIds`, `ChangedMoreThan` (с минуты, секунды отброшены), `sort`,
//! `pagesize`; карточки, история, файлы; каждый запрос записан по порядку.

use chrono::{Duration as Span, TimeZone};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use ticketboard::export::closed::{ExportFailure, ExportFailures, ExportMark, ExportReach, ExportScope};
use ticketboard::export::files::{LFS_RULE, TICKETS_FOLDER};
use ticketboard::export::pass::{ClosedExportPass, run_pass};
use ticketboard::export::run::{Options, Turn, export_rows};
use ticketboard::export::{AttachmentScope, Context};
use ticketboard::intraservice::query::TaskQuery;
use ticketboard::intraservice::{Client, Date, Found, User};
use ticketboard::rules::ClosedNames;

// ---------- время и номера ----------

const ME_ID: i32 = 7;
const ME: &str = "Я Сам";
const OTHER_ID: i32 = 60;
const REQUESTER: &str = "Петрова А.";
const REQUESTER_ID: i32 = 50;
const WORKING: i32 = 32;
const FIXED: i32 = 29;
const FINAL: i32 = 30;
const CANCELLED: i32 = 34;

/// Даты проверок — в поясе ПК: запрос захода идёт по часам ПК (C# `mark.Since.LocalDateTime`), и так часы дат и запроса
/// совпадают, где бы ни шла проверка.
fn at(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> Date {
    chrono::Local.with_ymd_and_hms(y, m, d, h, mi, s).single().unwrap().fixed_offset()
}

/// «Сейчас» заходов: начальная отметка — неделя назад, 19.11.2026 12:00.
fn now() -> Date {
    at(2026, 11, 26, 12, 0, 0)
}

/// От него — изменения заявок (после начальной отметки), 25.11.2026 10:00:00.
fn t0() -> Date {
    at(2026, 11, 25, 10, 0, 0)
}

fn min(n: i64) -> Span {
    Span::minutes(n)
}

fn sec(n: i64) -> Span {
    Span::seconds(n)
}

// ---------- заявки на сервере ----------

#[derive(Clone)]
struct Ev {
    at: Date,
    editor: &'static str,
    editor_id: i32,
    status: Option<i32>,
    comment: Option<&'static str>,
}

#[derive(Clone)]
struct F {
    id: i32,
    name: &'static str,
    bytes: Vec<u8>,
    missing: bool,
}

#[derive(Clone)]
struct T {
    id: i32,
    status: i32,
    created: Date,
    changed: Date,
    executors: &'static str,
    executor_ids: Vec<i32>,
    creator: &'static str,
    creator_id: i32,
    observers: &'static str,
    observer_ids: Vec<i32>,
    /// Строка без номеров инициатора, исполнителей и наблюдателей — одни имена.
    no_ids: bool,
    events: Vec<Ev>,
    files: Vec<F>,
    /// Карточка отвечает этим кодом (0 — как обычно).
    card_fails: u16,
    /// История отвечает этим кодом (0 — как обычно).
    life_fails: u16,
    /// История без конца: по одной записи на страницу, страниц тысяча.
    long_history: bool,
}

impl T {
    /// Закрытая, я — исполнитель.
    fn mine(id: i32) -> T {
        T {
            id,
            status: FINAL,
            created: at(2026, 11, 10, 9, 0, 0),
            changed: t0(),
            executors: ME,
            executor_ids: vec![ME_ID],
            creator: REQUESTER,
            creator_id: REQUESTER_ID,
            observers: "",
            observer_ids: Vec::new(),
            no_ids: false,
            events: Vec::new(),
            files: Vec::new(),
            card_fails: 0,
            life_fails: 0,
            long_history: false,
        }
    }

    /// Закрытая, исполнитель — другой.
    fn foreign(id: i32) -> T {
        T { executors: "Иванов И.", executor_ids: vec![OTHER_ID], ..T::mine(id) }
    }

    fn at(mut self, changed: Date) -> T {
        self.changed = changed;
        self
    }

    fn status(mut self, status: i32) -> T {
        self.status = status;
        self
    }

    fn said(mut self, when: Date, editor: &'static str, editor_id: i32, comment: &'static str) -> T {
        self.events.push(Ev { at: when, editor, editor_id, status: None, comment: Some(comment) });
        self
    }

    fn moved(mut self, when: Date, editor: &'static str, editor_id: i32, status: i32) -> T {
        self.events.push(Ev { at: when, editor, editor_id, status: Some(status), comment: None });
        self
    }

    fn file(mut self, id: i32, name: &'static str, bytes: &[u8]) -> T {
        self.files.push(F { id, name, bytes: bytes.to_vec(), missing: false });
        self
    }

    fn name(&self) -> String {
        format!("Заявка {}", self.id)
    }

    /// Строка списка — как её разбирает клиент.
    fn found(&self) -> Found {
        Found {
            id: self.id,
            name: self.name(),
            status: status_name(self.status).into(),
            created: Some(self.created),
            changed: Some(self.changed),
            ..Found::default()
        }
    }
}

fn status_name(id: i32) -> &'static str {
    match id {
        31 => "Открыта",
        32 => "В работе",
        29 => "Выполнена",
        30 => "Закрыта",
        34 => "Отменена",
        _ => "Иной",
    }
}

const STATUSES: &str = r#"[{"Id":31,"Name":"Открыта"},{"Id":32,"Name":"В работе"},{"Id":29,"Name":"Выполнена","IsFixed":true},{"Id":30,"Name":"Закрыта","IsFinal":true},{"Id":34,"Name":"Отменена"}]"#;

/// Что сервер делает со списком не так, как надо, — как поля `BoardServer` у C#.
#[derive(Clone, Default)]
struct Switches {
    /// Список отвечает этим кодом.
    fail_list: u16,
    /// В строках списка нет `Changed`.
    no_changed: bool,
    /// Порядок «Changed asc» не соблюдён: свежие сверху.
    ignore_sort: bool,
    /// `ChangedMoreThan` не применён.
    ignore_changed: bool,
    /// Сервер сам режет страницу до стольких строк (0 — нет).
    page_cap: usize,
    /// В ответе нет Paginator.
    no_paginator: bool,
    /// Сервер читает дату на столько минут раньше (другой пояс у его пользователя).
    skew_minutes: i64,
    /// Равные изменения идут по убыванию номеров.
    ties_descending: bool,
    /// Эту строку сервер повторил подряд (0 — нет).
    repeat_row: i32,
    /// Список отвечает не раньше чем через столько.
    list_delay: Duration,
    /// Карточка этой заявки ждёт, пока `release` (0 — нет).
    hold_card: i32,
    /// В Paginator — это число вместо настоящего счёта (None — настоящий).
    force_count: Option<usize>,
    /// В Paginator — этот `HasNextPage` вместо настоящего (None — настоящий).
    force_next: Option<bool>,
}

struct State {
    tickets: BTreeMap<i32, T>,
    log: Vec<String>,
    sw: Switches,
}

struct Shared {
    state: Mutex<State>,
    released: AtomicBool,
}

struct Fake {
    shared: Arc<Shared>,
    url: String,
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn param(path: &str, key: &str) -> Option<String> {
    path.split_once('?')?.1.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}=")).map(decode))
}

impl Shared {
    fn reply(&self, path: &str) -> (u16, String, Vec<u8>) {
        let json = |code: u16, body: String| (code, "application/json; charset=utf-8".to_string(), body.into_bytes());
        let (sw, delay) = {
            let st = self.state.lock().unwrap();
            (st.sw.clone(), st.sw.list_delay)
        };
        if let Some(rest) = path.strip_prefix("/api/taskfile/") {
            let id: i32 = rest.split('?').next().unwrap_or("").parse().unwrap_or(0);
            let st = self.state.lock().unwrap();
            let file = st.tickets.values().flat_map(|t| t.files.iter()).find(|f| f.id == id);
            return match file {
                Some(f) if !f.missing => (200, "application/octet-stream".into(), f.bytes.clone()),
                _ => json(404, r#"{"Message":"not found"}"#.into()),
            };
        }
        if path.starts_with("/api/tasklifetime?") {
            let id: i32 = param(path, "taskid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let page: usize = param(path, "page").and_then(|v| v.parse().ok()).unwrap_or(1);
            let st = self.state.lock().unwrap();
            let Some(t) = st.tickets.get(&id) else { return json(404, r#"{"Message":"not found"}"#.into()) };
            if t.life_fails != 0 {
                return json(t.life_fails, String::new());
            }
            return json(200, life_json(t, page));
        }
        if let Some(rest) = path.strip_prefix("/api/task/") {
            let id: i32 = rest.split('?').next().unwrap_or("").parse().unwrap_or(0);
            if sw.hold_card == id {
                let deadline = Instant::now() + Duration::from_secs(20);
                while !self.released.load(SeqCst) && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            let st = self.state.lock().unwrap();
            let Some(t) = st.tickets.get(&id) else { return json(404, r#"{"Message":"not found"}"#.into()) };
            if t.card_fails != 0 {
                return json(t.card_fails, String::new());
            }
            return json(200, card_json(t));
        }
        if path.starts_with("/api/task?") {
            thread::sleep(delay);
            if sw.fail_list != 0 {
                return json(sw.fail_list, String::new());
            }
            let st = self.state.lock().unwrap();
            return json(200, list_json(&st, path, &sw));
        }
        json(404, r#"{"Message":"not found"}"#.into())
    }
}

fn card_json(t: &T) -> String {
    let mut task = serde_json::json!({
        "Id": t.id, "Name": t.name(), "StatusName": status_name(t.status), "StatusId": t.status, "Creator": t.creator,
        "Created": t.created.to_rfc3339(), "Changed": t.changed.to_rfc3339(), "Executors": t.executors,
    });
    if !t.files.is_empty() {
        let ids: Vec<String> = t.files.iter().map(|f| f.id.to_string()).collect();
        let names: Vec<&str> = t.files.iter().map(|f| f.name).collect();
        task["FileIds"] = ids.join(",").into();
        task["FileNames"] = names.join("||").into();
    }
    serde_json::json!({ "Task": task }).to_string()
}

fn life_json(t: &T, page: usize) -> String {
    let statuses: serde_json::Value = serde_json::from_str(STATUSES).unwrap();
    if t.long_history {
        let when = t0() - min(page as i64);
        let event =
            serde_json::json!({ "Date": when.to_rfc3339(), "Editor": "Петров П.", "EditorId": 61, "Comments": format!("запись {page}") });
        return serde_json::json!({ "TaskLifetimes": [event], "Statuses": statuses,
            "Paginator": { "Count": 1, "Page": page, "PageCount": 1000 } })
        .to_string();
    }
    let mut all = t.events.clone();
    all.sort_by_key(|e| std::cmp::Reverse(e.at)); // свежие сверху
    let pages = all.len().div_ceil(50).max(1);
    let events: Vec<serde_json::Value> = all
        .iter()
        .skip((page - 1) * 50)
        .take(50)
        .map(|e| {
            let mut v = serde_json::json!({ "Date": e.at.to_rfc3339(), "Editor": e.editor, "EditorId": e.editor_id });
            if let Some(s) = e.status {
                v["StatusId"] = s.into();
            }
            if let Some(c) = e.comment {
                v["Comments"] = c.into();
            }
            v
        })
        .collect();
    serde_json::json!({ "TaskLifetimes": events, "Statuses": statuses,
        "Paginator": { "Count": events.len(), "Page": page, "PageCount": pages } })
    .to_string()
}

fn list_json(st: &State, path: &str, sw: &Switches) -> String {
    let statuses: Option<Vec<i32>> = param(path, "StatusIds").map(|v| v.split(',').filter_map(|s| s.parse().ok()).collect());
    let since = param(path, "ChangedMoreThan")
        .and_then(|v| chrono::NaiveDateTime::parse_from_str(&v, "%Y-%m-%d %H:%M").ok())
        .map(|d| d - Span::minutes(sw.skew_minutes));
    let mut rows: Vec<&T> = st
        .tickets
        .values()
        .filter(|t| statuses.as_ref().is_none_or(|s| s.contains(&t.status)))
        .filter(|t| sw.ignore_changed || since.is_none_or(|s| t.changed.naive_local() >= s))
        .collect();
    rows.sort_by_key(|t| (t.changed, if sw.ties_descending { -t.id } else { t.id }));
    if sw.ignore_sort {
        rows.reverse();
    }
    let total = rows.len();
    let size: usize = param(path, "pagesize").and_then(|v| v.parse().ok()).unwrap_or(25);
    let cap = if sw.page_cap > 0 { size.min(sw.page_cap) } else { size };
    let mut page: Vec<&T> = rows.into_iter().take(cap).collect();
    if let Some(i) = page.iter().position(|t| t.id == sw.repeat_row) {
        page.insert(i, page[i]);
    }
    let tasks: Vec<serde_json::Value> = page
        .iter()
        .map(|t| {
            let mut v = serde_json::json!({
                "Id": t.id, "Name": t.name(), "StatusId": t.status, "Created": t.created.to_rfc3339(), "Creator": t.creator,
                "Executors": t.executors, "Description": "",
            });
            if !sw.no_changed {
                v["Changed"] = t.changed.to_rfc3339().into();
            }
            if !t.observers.is_empty() {
                v["Observers"] = t.observers.into();
            }
            if !t.no_ids {
                v["CreatorId"] = t.creator_id.into();
                v["ExecutorIds"] = t.executor_ids.clone().into();
                if !t.observer_ids.is_empty() {
                    v["ObserverIds"] = t.observer_ids.clone().into();
                }
            }
            v
        })
        .collect();
    let mut body = serde_json::json!({ "Tasks": tasks, "Statuses": serde_json::from_str::<serde_json::Value>(STATUSES).unwrap() });
    if !sw.no_paginator {
        body["Paginator"] = serde_json::json!({
            "Count": sw.force_count.unwrap_or(total), "Page": 1, "PageCount": total.div_ceil(size.max(1)).max(1),
            "HasNextPage": sw.force_next.unwrap_or(total > page.len()),
        });
    }
    body.to_string()
}

fn serve_one(mut stream: TcpStream, shared: &Shared) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split(' ').nth(1).unwrap_or("").to_string();
    shared.state.lock().unwrap().log.push(path.clone());
    let (code, content_type, body) = shared.reply(&path);
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

impl Fake {
    fn new(tickets: Vec<T>) -> Fake {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                tickets: tickets.into_iter().map(|t| (t.id, t)).collect(),
                log: Vec::new(),
                sw: Switches::default(),
            }),
            released: AtomicBool::new(false),
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
        Fake { shared, url }
    }

    fn client(&self) -> Client {
        Client::new(&self.url, "u", "p", Duration::from_secs(10))
    }

    fn edit(&self, id: i32, change: impl FnOnce(&mut T)) {
        change(self.shared.state.lock().unwrap().tickets.get_mut(&id).unwrap());
    }

    fn put(&self, t: T) {
        self.shared.state.lock().unwrap().tickets.insert(t.id, t);
    }

    fn switch(&self, change: impl FnOnce(&mut Switches)) {
        change(&mut self.shared.state.lock().unwrap().sw);
    }

    fn changed(&self, id: i32) -> Date {
        self.shared.state.lock().unwrap().tickets[&id].changed
    }

    fn found(&self, id: i32) -> Found {
        self.shared.state.lock().unwrap().tickets[&id].found()
    }

    fn paths(&self) -> Vec<String> {
        self.shared.state.lock().unwrap().log.clone()
    }

    fn count(&self, prefix: &str) -> usize {
        self.paths().iter().filter(|p| p.starts_with(prefix)).count()
    }

    fn lists(&self) -> Vec<String> {
        self.paths().into_iter().filter(|p| p.starts_with("/api/task?")).collect()
    }

    fn cards(&self, id: i32) -> usize {
        self.count(&format!("/api/task/{id}?"))
    }

    fn lives(&self, id: i32) -> usize {
        self.count(&format!("/api/tasklifetime?taskid={id}&"))
    }

    fn file_requests(&self, id: i32) -> usize {
        self.count(&format!("/api/taskfile/{id}"))
    }

    /// Всего запросов карточек, историй и файлов — то, что тратит заход.
    fn spent(&self) -> (usize, usize, usize) {
        (self.count("/api/task/"), self.count("/api/tasklifetime"), self.count("/api/taskfile/"))
    }

    /// Отпустить удержанную карточку; в журнале — метка, раньше которой её никто не получит.
    fn release(&self) {
        self.shared.state.lock().unwrap().log.push("#release".into());
        self.shared.released.store(true, SeqCst);
    }
}

// ---------- запуск ----------

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("tb-pass-{tag}-{}-{}", std::process::id(), N.fetch_add(1, SeqCst)));
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

fn scope(reach: ExportReach, attachments: AttachmentScope) -> ExportScope {
    ExportScope {
        closed_status_ids: vec![FINAL, FIXED, CANCELLED],
        me: User { id: ME_ID, name: ME.into() },
        closed_names: ClosedNames::new(&["Отменена".to_string()]),
        attachments,
        reach,
    }
}

fn fast() -> Options {
    Options { retry_delays: vec![Duration::ZERO; 3], vanish_wait: Duration::ZERO, turn: Turn::private(), ..Options::default() }
}

fn url_of(id: i32) -> String {
    format!("https://hd/Task/View/{id}")
}

/// Заходы подряд, как их ведёт окно: отметка, «не мои» и счёт сбоев каждого — у следующего.
struct Session {
    fake: Fake,
    dir: Tmp,
    scope: ExportScope,
    mark: ExportMark,
    not_mine: BTreeMap<i32, Date>,
    failures: ExportFailures,
    now: Date,
}

impl Session {
    fn new(tag: &str, tickets: Vec<T>) -> Session {
        Session {
            fake: Fake::new(tickets),
            dir: Tmp::new(tag),
            scope: scope(ExportReach::Mine, AttachmentScope::Mine),
            mark: ExportMark::start(now()),
            not_mine: BTreeMap::new(),
            failures: ExportFailures::default(),
            now: now(),
        }
    }

    fn run(&self, budget: usize, stop: &AtomicBool, options: &Options) -> ClosedExportPass {
        run_pass(
            &self.fake.client(),
            &self.scope,
            &self.mark,
            &self.not_mine,
            &self.failures,
            self.dir.path(),
            &url_of,
            budget,
            self.now,
            stop,
            options,
        )
    }

    /// Заход и принятие его итога: отметка, «не мои» и счёт — как есть, и с ошибкой (отметка встала перед сбойной).
    fn pass(&mut self, budget: usize) -> ClosedExportPass {
        let p = self.run(budget, &AtomicBool::new(false), &fast());
        self.mark = p.next.clone();
        self.not_mine = p.not_mine.clone();
        self.failures = p.failures.clone();
        p
    }

    /// Файл заявки в любой папке месяца.
    fn file(&self, id: i32) -> Option<PathBuf> {
        let tickets = self.dir.join(TICKETS_FOLDER);
        for month in fs::read_dir(&tickets).ok()?.flatten() {
            for entry in fs::read_dir(month.path()).ok()?.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.ends_with(".md") && (name.starts_with(&format!("{id} — ")) || name == format!("{id}.md")) {
                    return Some(entry.path());
                }
            }
        }
        None
    }

    fn text(&self, id: i32) -> String {
        self.file(id).and_then(|p| fs::read_to_string(p).ok()).unwrap_or_else(|| "(нет файла)".into())
    }

    fn written(&self, ids: &[i32]) -> Vec<i32> {
        ids.iter().copied().filter(|&id| self.file(id).is_some()).collect()
    }

    /// Мой заход, который прошёл как надо: без ошибки.
    fn ok(&mut self, budget: usize) -> ClosedExportPass {
        let p = self.pass(budget);
        assert_eq!(p.error, "", "{}", show(&p));
        p
    }
}

fn show(p: &ClosedExportPass) -> String {
    format!(
        "отметка {} {:?}, выгружено {}, ещё есть {}, пропущено {}, не мои {:?}\nошибка: «{}»",
        p.next.since,
        p.next.seen,
        p.exported,
        p.more,
        p.given_up.len(),
        p.not_mine.keys().collect::<Vec<_>>(),
        p.error
    )
}

fn wait_until(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "не дождались: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn failure_of(p: &ClosedExportPass, id: i32) -> &ExportFailure {
    p.failures.items.get(&id).unwrap_or_else(|| panic!("заявки {id} нет в счёте сбоев: {:?}", p.failures.items.keys().collect::<Vec<_>>()))
}

// =====================================================================================================================
// Страница списка (`Client::get_tasks_page`)
// =====================================================================================================================

#[test]
fn a_list_page_is_page_one_of_the_asked_size_without_service_names_and_with_a_count() {
    // C# `GetTasksAsync(query, 1, ct, pageSize, timeout:)`: без `include=service` (не «детально»), со счётом (нет `count=false`)
    let fake = Fake::new(vec![T::mine(801).at(t0()), T::mine(802).at(t0() + min(1))]);
    let query = scope(ExportReach::Mine, AttachmentScope::None).query(&ExportMark::start(now()));
    let page = fake.client().get_tasks_page(&query, 505, Duration::from_secs(10)).expect("страница списка");
    assert_eq!(page.found.iter().map(|f| f.id).collect::<Vec<_>>(), [801, 802], "строки — по порядку изменения");
    assert_eq!(page.total, 2);
    let lists = fake.lists();
    assert_eq!(lists.len(), 1, "{lists:?}");
    assert!(lists[0].contains("pagesize=505&page=1"), "{}", lists[0]);
    assert!(lists[0].contains("include=status&") && !lists[0].contains("service"), "без названий сервисов: {}", lists[0]);
    assert!(!lists[0].contains("count=false"), "со счётом: {}", lists[0]);
}

#[test]
fn a_list_page_has_its_own_time_limit() {
    // C# 15 (`KnowledgeExport.SelfCheck.Mass.cs`): срок страницы — свой, а не срок клиента; не ответила — «сервер не ответил»
    let fake = Fake::new(vec![T::mine(801)]);
    fake.switch(|s| s.list_delay = Duration::from_secs(3));
    let query = TaskQuery::default();
    let error = fake.client().get_tasks_page(&query, 500, Duration::from_millis(150)).expect_err("срок вышел");
    assert!(error.starts_with("сервер не ответил за 0,15 с"), "{error}");
}

#[test]
fn the_pass_waits_for_the_list_only_as_long_as_the_option_says() {
    // `options.list_timeout` — срок страницы захода (C# `KnowledgeExport.ListTimeout`): сервер молчит дольше — заход не удался,
    // отметка стоит
    let s = Session::new("list-timeout", vec![T::mine(801)]);
    s.fake.switch(|sw| sw.list_delay = Duration::from_secs(3));
    let options = Options { list_timeout: Duration::from_millis(150), ..fast() };
    let p = s.run(30, &AtomicBool::new(false), &options);
    assert!(p.error.starts_with("сервер не ответил"), "{}", show(&p));
    assert_eq!((p.next, p.exported, p.more), (s.mark.clone(), 0, false));
    assert_eq!(s.fake.cards(801), 0);
}

// =====================================================================================================================
// 2. Первый заход: кто чьё
// =====================================================================================================================

/// Закрытые разных видов (C# 2, блок наблюдателей) и один заход с неделей назад.
fn first_session(tag: &str) -> (Session, ClosedExportPass) {
    let tickets = vec![
        T::mine(801).at(t0() + min(1)),
        T { creator: ME, creator_id: ME_ID, ..T::foreign(802) }.at(t0() + min(2)),
        // только мой комментарий в истории
        T::foreign(803).at(t0() + min(3)).said(t0() + min(1), ME, ME_ID, "Посмотрел логи").moved(
            t0() + min(2),
            "Иванов И.",
            OTHER_ID,
            FINAL,
        ),
        // только моя смена статуса
        T::foreign(804).at(t0() + min(4)).moved(t0() + min(1), ME, ME_ID, FIXED).moved(t0() + min(2), "Иванов И.", OTHER_ID, FINAL),
        // моё имя — только в тексте чужого комментария
        T::foreign(805).at(t0() + min(5)).said(t0() + min(1), REQUESTER, REQUESTER_ID, "Я Сам, посмотрите, пожалуйста").moved(
            t0() + min(2),
            "Иванов И.",
            OTHER_ID,
            FINAL,
        ),
        T::mine(806).at(t0() + min(6)).status(WORKING),
        // давняя закрытая: раньше начальной отметки
        T::mine(807).at(now() - Span::days(8)),
        // закрыта по названию статуса, а не по признаку
        T::mine(808).at(t0() + min(7)).status(CANCELLED),
        // без номеров: исполнитель по имени
        T { no_ids: true, ..T::mine(809) }.at(t0() + min(8)),
        // без номеров: инициатор по имени
        T { no_ids: true, creator: ME, ..T::foreign(810) }.at(t0() + min(9)),
        // наблюдатель — по номеру и без номеров по имени; где наблюдает другой — не моя
        T { observers: "Иванов И., Я Сам", observer_ids: vec![OTHER_ID, ME_ID], ..T::foreign(811) }.at(t0() + min(10)),
        T { no_ids: true, observers: ME, ..T::foreign(812) }.at(t0() + min(11) + sec(30)),
        T { observers: "Иванов И.", observer_ids: vec![OTHER_ID], ..T::foreign(813) }.at(t0() + min(11) + sec(50)),
    ];
    let mut s = Session::new(tag, tickets);
    let p = s.pass(30);
    (s, p)
}

#[test]
fn the_first_pass_asks_one_page_of_closed_tickets_from_the_week_ago_minute_in_changed_order() {
    // C# 2: «одна страница: 500 строк и номера минуты отметки»; `ChangedMoreThan` — с минуты (секунды отброшены), закрытые статусы
    let (s, p) = first_session("first-query");
    assert_eq!(p.error, "", "{}", show(&p));
    let lists = s.fake.lists();
    assert_eq!(lists.len(), 1, "один список за заход: {lists:?}");
    let list = &lists[0];
    assert!(list.contains("StatusIds=30,29,34&"), "закрытые статусы: {list}");
    assert!(list.contains("ChangedMoreThan=2026-11-19%2012%3A00&"), "с минуты начальной отметки: {list}");
    assert!(list.contains("sort=Changed%20asc,%20Id%20asc&pagesize=500&page=1"), "по изменению, страница 500: {list}");
    assert!(list.contains("archive=true&inactive=true&"), "и архивные сервисы: {list}");
}

#[test]
fn the_first_pass_takes_executor_creator_participant_and_observer_and_leaves_the_rest() {
    // C# 2 и блок наблюдателей (0.21.0): 801 исполнитель, 802 инициатор, 803/804 участник по истории, 808 закрыта по названию,
    // 809/810/812 — по именам без номеров, 811/812 наблюдатель; не выгружены: 805 (имя только в тексте), 806 открытая, 807 давняя, 813
    let (s, p) = first_session("first-who");
    for (id, why) in [
        (801, "исполнитель"),
        (802, "инициатор"),
        (803, "только мой комментарий"),
        (804, "только моя смена статуса"),
        (808, "закрыта по названию статуса"),
        (809, "исполнитель по имени"),
        (810, "инициатор по имени"),
        (811, "наблюдатель по номеру"),
        (812, "наблюдатель по имени"),
    ] {
        assert!(s.file(id).is_some(), "должна быть выгружена: {why} (#{id})\n{}", show(&p));
    }
    for (id, why) in [
        (805, "моё имя — только в тексте чужого комментария"),
        (806, "открытая"),
        (807, "закрыта раньше недели"),
        (813, "наблюдает другой"),
    ] {
        assert!(s.file(id).is_none(), "не должна быть выгружена: {why} (#{id})");
    }
    assert_eq!(p.exported, 9, "{}", show(&p));
    assert!(!p.more && p.given_up.is_empty() && p.failures.items.is_empty(), "{}", show(&p));
}

#[test]
fn a_ticket_that_is_mine_by_the_row_costs_one_history_read_and_by_history_two_and_a_foreign_one_one() {
    // C# 2: чужая — история один раз, карточка ни разу; моя по строке — история один раз (для файла); по истории — дважды
    // (разбор и файл)
    let (s, _) = first_session("first-requests");
    let f = &s.fake;
    assert_eq!((f.lives(805), f.cards(805)), (1, 0), "чужая: история один раз, карточки нет");
    assert_eq!((f.lives(813), f.cards(813)), (1, 0), "наблюдает другой: тоже");
    for id in [801, 802, 808, 809, 810, 811, 812] {
        assert_eq!((f.lives(id), f.cards(id)), (1, 1), "моя по строке #{id}: история для файла, карточка");
    }
    for id in [803, 804] {
        assert_eq!((f.lives(id), f.cards(id)), (2, 1), "моя по истории #{id}: история для разбора и для файла, карточка");
    }
    assert_eq!((f.lives(806) + f.lives(807), f.cards(806) + f.cards(807)), (0, 0), "открытая и давняя в список не попали");
}

#[test]
fn the_files_of_my_tickets_say_my_role() {
    // C# 2: «в файле — моя роль: исполнитель, участник, наблюдатель» — роль считает движок по контексту захода
    let (s, _) = first_session("first-roles");
    assert!(s.text(801).contains("\nmy_role: \"исполнитель\"\n"), "{}", s.text(801));
    assert!(s.text(802).contains("\nmy_role: \"инициатор\"\n"), "{}", s.text(802));
    assert!(s.text(803).contains("\nmy_role: \"участник\"\n"), "{}", s.text(803));
    assert!(s.text(811).contains("\nmy_role: \"наблюдатель\"\n"), "{}", s.text(811));
    assert!(s.text(812).contains("\nmy_role: \"наблюдатель\"\n"), "{}", s.text(812));
}

#[test]
fn the_mark_goes_to_the_last_row_with_the_numbers_of_its_minute_and_a_foreign_one_is_remembered() {
    // C# 3 (+ `Keep`): отметка — изменение последней разобранной и номера её минуты; «не моя» помнится, если не раньше минуты
    // отметки (805 — раньше, её забыли)
    let (s, p) = first_session("first-mark");
    let last = s.fake.changed(813);
    assert_eq!(p.next, ExportMark { since: last, seen: vec![812, 813] }, "{}", show(&p));
    assert_eq!(p.not_mine, BTreeMap::from([(813, last)]), "{}", show(&p));
}

// =====================================================================================================================
// 4–5. Повторный заход и перемены
// =====================================================================================================================

#[test]
fn a_repeated_pass_without_changes_asks_only_the_list() {
    // C# 4: ни карточек, ни историй, один список; отметка та же
    let (mut s, first) = first_session("repeat");
    let spent = s.fake.spent();
    let p = s.ok(30);
    assert_eq!(s.fake.spent(), spent, "ни карточек, ни историй, ни файлов");
    assert_eq!(s.fake.lists().len(), 2, "за заход — один список");
    assert_eq!((p.exported, p.more, p.next.clone()), (0, false, first.next), "{}", show(&p));
}

#[test]
fn a_new_closed_ticket_and_an_old_one_that_got_a_comment_are_exported_by_the_next_pass() {
    // C# 5: новая закрытая выгружена, дописанная старая закрытая — файл обновлён; в итоге — оба
    let (mut s, _) = first_session("changes");
    s.fake.put(T::mine(814).at(t0() + min(20)));
    s.fake.edit(801, |t| {
        t.events.push(Ev {
            at: t0() + min(21),
            editor: REQUESTER,
            editor_id: REQUESTER_ID,
            status: None,
            comment: Some("Спасибо, всё работает"),
        });
        t.changed = t0() + min(21);
    });
    let p = s.ok(30);
    assert_eq!(p.exported, 2, "{}", show(&p));
    assert!(s.file(814).is_some(), "новая закрытая выгружена");
    assert!(s.text(801).contains("Спасибо, всё работает"), "файл дописанной обновлён: {}", s.text(801));
    assert_eq!((s.fake.cards(801), s.fake.lives(801)), (2, 2), "перечитана целиком — заново");
    assert_eq!(p.next.since, t0() + min(21), "{}", show(&p));
}

// =====================================================================================================================
// 5б. Вложения
// =====================================================================================================================

fn with_files() -> Vec<T> {
    vec![T::mine(815).at(t0()).file(9801, "экран.png", &[1, 2, 3]).file(9802, "лог.txt", &[4, 5])]
}

#[test]
fn a_ticket_whose_attachment_is_missing_on_the_server_holds_the_mark_and_the_next_pass_downloads_only_that_file() {
    // C# 5б: вложения моей закрытой — в files/, правило LFS — в .gitattributes; второго файла на сервере нет — заявка не готова:
    // отметка за неё не уходит, ошибка называет заявку и вложение; когда файл появился, докачивается только он
    let mut s = Session::new("attachments", with_files());
    s.fake.edit(815, |t| t.files[1].missing = true);
    let p = s.pass(30);
    assert!(p.error.contains("#815") && p.error.contains("HTTP 404") && p.error.contains("«лог.txt»"), "{}", show(&p));
    assert_eq!(p.next, ExportMark::start(now()), "отметка не ушла за заявку: {}", show(&p));
    assert!(s.file(815).is_some(), "файл заявки записан, хоть вложения нет");
    let shot = s.dir.join("files/2026-11/815/9801 — экран.png");
    assert_eq!(fs::read(&shot).unwrap_or_default(), [1, 2, 3], "скриншот лёг побайтно: {:?}", shot);
    assert!(fs::read_to_string(s.dir.join(".gitattributes")).unwrap_or_default().contains(LFS_RULE), "правило LFS — в .gitattributes");
    s.fake.edit(815, |t| t.files[1].missing = false);
    let p = s.ok(30);
    assert!(s.dir.join("files/2026-11/815/9802 — лог.txt").exists(), "файл докачан");
    assert_eq!(s.fake.file_requests(9801), 1, "скачанный — не заново");
    assert_eq!(p.next.since, s.fake.changed(815), "отметка — за заявкой: {}", show(&p));
}

// =====================================================================================================================
// 6. Порции
// =====================================================================================================================

fn portions() -> Vec<T> {
    vec![
        T::mine(820).at(t0()),
        T::foreign(821).at(t0() + sec(10)),
        T::mine(822).at(t0() + sec(20)),
        T::foreign(823).at(t0() + sec(20)),
        T::mine(824).at(t0() + sec(20)),
        T::foreign(825).at(t0() + sec(40)),
        T::mine(826).at(t0() + sec(50)),
    ]
}

#[test]
fn the_budget_counts_tickets_that_need_a_request_and_the_border_may_fall_between_equal_changes() {
    // C# 6: семь заявок при порции 3, четыре — в одну минуту, граница порции — между равными (822, 823, 824 — одна секунда);
    // «более ранние» номера минуты в отметке не дают перечитать 820–822 в следующем заходе
    let mut s = Session::new("portions", portions());
    let one = s.ok(3);
    assert_eq!((one.exported, one.more), (2, true), "порция 1: 820, 822 выгружены, 821 прочитана: {}", show(&one));
    assert!(s.written(&[820, 822]) == [820, 822] && s.file(824).is_none() && s.fake.lives(823) == 0, "порция 1 кончилась на 823");
    assert_eq!(one.next, ExportMark { since: t0() + sec(20), seen: vec![820, 821, 822] }, "{}", show(&one));
    let two = s.ok(3);
    assert_eq!((two.exported, two.more), (1, true), "порция 2: 823 и 825 прочитаны, 824 выгружена: {}", show(&two));
    let three = s.ok(3);
    assert_eq!((three.exported, three.more), (1, false), "порция 3: 826, дальше ничего: {}", show(&three));
    let four = s.ok(3);
    assert_eq!((four.exported, four.more), (0, false), "после порций — тишина: {}", show(&four));
    assert_eq!(s.written(&[820, 822, 824, 826]), [820, 822, 824, 826], "все мои выгружены, ни одна не потеряна");
    for id in [821, 823, 825] {
        assert_eq!((s.fake.lives(id), s.fake.cards(id)), (1, 0), "чужая #{id}: история один раз");
    }
    for id in [820, 822, 824, 826] {
        assert_eq!((s.fake.lives(id), s.fake.cards(id)), (1, 1), "моя #{id}: прочитана один раз");
    }
    assert_eq!(four.next.since, t0() + sec(50), "отметка — на последней");
}

// =====================================================================================================================
// 7а–7в. Сбои списка, карточки, истории
// =====================================================================================================================

#[test]
fn a_failed_list_stops_the_pass_with_the_error_and_nothing_moves() {
    // C# 7а: сбой списка — отметка стоит; один запрос (без повтора); «не мои» и счёт — как были
    let mut s = Session::new("list-fails", vec![T::mine(801).at(t0())]);
    s.mark = ExportMark { since: t0() - min(5), seen: vec![1, 2] };
    s.not_mine = BTreeMap::from([(900, t0())]);
    s.failures = ExportFailures {
        items: BTreeMap::from([(901, ExportFailure { id: 901, changed: t0(), times: 2, first: now(), error: "прежняя".into() })]),
    };
    s.fake.switch(|sw| sw.fail_list = 503);
    let (mark, not_mine, failures) = (s.mark.clone(), s.not_mine.clone(), s.failures.clone());
    let p = s.pass(30);
    assert!(p.error.contains("HTTP 503"), "{}", show(&p));
    assert_eq!((p.next, p.not_mine, p.failures), (mark, not_mine, failures), "ничего не сдвинулось");
    assert_eq!((p.exported, p.more, p.given_up.len()), (0, false, 0));
    assert_eq!(s.fake.lists().len(), 1, "список спрошен один раз");
    assert_eq!(s.fake.spent(), (0, 0, 0));
}

fn after_mark() -> ExportMark {
    ExportMark { since: t0(), seen: vec![826] }
}

#[test]
fn a_card_that_fails_holds_the_mark_before_it_and_later_ones_are_still_exported() {
    // C# 7б: сбой карточки моей заявки (500) — отметка перед ней, в ошибке — её собственная (не соседней: у 833 история 404),
    // моя после неё всё равно выгружена, обе сбойные — в счёте; «не моя» после сбойной — запомнена
    let mut s = Session::new(
        "card-fails",
        vec![
            T::mine(826).at(t0()),
            T { card_fails: 500, ..T::mine(830) }.at(t0()),
            T::foreign(831).at(t0()),
            T::mine(832).at(t0() + min(1)),
            T { life_fails: 404, ..T::mine(833) }.at(t0() + min(2)),
        ],
    );
    s.mark = after_mark();
    let p = s.pass(30);
    assert!(p.error.contains("#830") && p.error.contains("HTTP 500") && !p.error.contains("HTTP 404"), "{}", show(&p));
    assert_eq!(p.next, after_mark(), "отметка перед сбойной: {}", show(&p));
    assert!(s.file(832).is_some() && s.file(830).is_none(), "моя после неё выгружена");
    assert_eq!(failure_of(&p, 830).times, 1);
    assert_eq!(failure_of(&p, 833).times, 1);
    assert!(failure_of(&p, 830).error.contains("HTTP 500") && failure_of(&p, 833).error.contains("HTTP 404"), "у каждой — своя ошибка");
    assert_eq!(failure_of(&p, 830).changed, t0(), "счёт ведётся по изменению заявки");
    assert_eq!(p.failures.items.len(), 2, "чужая и выгруженная в счёт не идут");
    assert_eq!(p.not_mine, BTreeMap::from([(831, t0())]), "{}", show(&p));
}

#[test]
fn a_retry_after_a_card_failure_rereads_neither_the_foreign_one_nor_the_finished_one_and_a_repair_moves_the_mark() {
    // C# 7б: повтор — «не моя» после сбойной не перечитана (её запомнили), готовая — без запросов; починили — заход прошёл,
    // отметка — за последней, счёт чист
    let mut s = Session::new(
        "card-retry",
        vec![
            T::mine(826).at(t0()),
            T { card_fails: 500, ..T::mine(830) }.at(t0()),
            T::foreign(831).at(t0()),
            T::mine(832).at(t0() + min(1)),
            T { life_fails: 404, ..T::mine(833) }.at(t0() + min(2)),
        ],
    );
    s.mark = after_mark();
    s.pass(30);
    let again = s.pass(30);
    assert!(again.error.contains("#830"), "{}", show(&again));
    assert_eq!((s.fake.lives(831), s.fake.cards(832)), (1, 1), "«не моя» не перечитана, готовая — без запросов");
    assert_eq!((failure_of(&again, 830).times, failure_of(&again, 833).times), (2, 2));
    s.fake.edit(830, |t| t.card_fails = 0);
    s.fake.edit(833, |t| t.life_fails = 0);
    let fixed = s.ok(30);
    assert_eq!(fixed.exported, 2, "{}", show(&fixed));
    assert!(s.file(830).is_some() && s.file(833).is_some());
    assert_eq!(fixed.next.since, s.fake.changed(833), "отметка — за последней: {}", show(&fixed));
    assert!(fixed.failures.items.is_empty(), "прошедшие — из счёта вон");
}

fn broken_history() -> Vec<T> {
    vec![
        T { life_fails: 500, ..T::foreign(834) }.at(t0() + min(1)),
        T::mine(835).at(t0() + min(2)),
        T { life_fails: 503, ..T::foreign(836) }.at(t0() + min(3)),
        T::foreign(837).at(t0() + min(4)),
    ]
}

#[test]
fn a_foreign_history_that_fails_by_the_ticket_is_counted_and_passed_but_a_network_failure_stops_the_pass() {
    // C# 7в: история «не моей» не читается из-за самой заявки (500) — заход идёт дальше (моя после неё выгружена); сбой сети
    // (503) — стоп, дальше не идём; история — без повтора
    let mut s = Session::new("history-fails", broken_history());
    let p = s.pass(30);
    assert!(p.error.starts_with("#834: история не читается") && p.error.contains("HTTP 500"), "{}", show(&p));
    assert_eq!(p.next, ExportMark::start(now()), "отметка перед ней: {}", show(&p));
    assert!(s.file(835).is_some(), "моя после неё выгружена тем же заходом");
    assert_eq!((s.fake.lives(836), s.fake.lives(837)), (1, 0), "после сбоя сети дальше не идём; история без повтора");
    assert_eq!(p.failures.items.keys().collect::<Vec<_>>(), [&834], "в счёт — только сама заявка, сеть не в счёт");
}

#[test]
fn a_broken_ticket_is_given_up_after_five_passes_and_an_hour_and_a_network_failure_never_is() {
    // C# 7г: сломанная заявка — пять заходов подряд и не раньше часа (здесь — сдвигом `now`), и отметка идёт дальше без неё;
    // сбой сети в счёт не идёт: на нём отметка стоит, сколько бы ни длился
    let mut s = Session::new("give-up", broken_history());
    let first = s.now;
    for k in 1..=5 {
        let p = s.pass(30);
        assert!(p.error.contains("#834"), "заход {k}: {}", show(&p));
        assert_eq!(p.next, ExportMark::start(now()), "заход {k}: отметка стоит");
        assert_eq!(failure_of(&p, 834).times, k, "заход {k}");
    }
    s.now = first + Span::minutes(59);
    let early = s.pass(30);
    assert_eq!(s.fake.lives(834), 6, "без часа — снова спрашиваем: {}", show(&early));
    assert!(early.given_up.is_empty() && early.error.contains("#834"), "{}", show(&early));
    s.now = first + Span::minutes(60);
    let late = s.pass(30);
    assert_eq!(s.fake.lives(834), 6, "пропущенную больше не спрашиваем");
    assert_eq!(late.given_up.iter().map(|f| f.id).collect::<Vec<_>>(), [834], "{}", show(&late));
    assert!(late.given_up[0].error.contains("HTTP 500") && late.given_up[0].times >= 5, "{:?}", late.given_up[0]);
    assert_eq!(late.next.since, s.fake.changed(835), "отметка за моей после неё, перед сбоем сети: {}", show(&late));
    assert!(late.error.contains("#836") && late.error.contains("HTTP 503"), "сбой сети — ошибка захода: {}", show(&late));
    assert!(!late.failures.items.contains_key(&836) && !late.failures.items.contains_key(&834), "{:?}", late.failures.items);
    // сеть не пропускается, сколько бы заходов ни длилась
    s.now = first + Span::days(3);
    for _ in 0..6 {
        let p = s.pass(30);
        assert!(p.given_up.is_empty() && p.error.contains("#836"), "{}", show(&p));
        assert_eq!(p.next.since, s.fake.changed(835));
    }
    assert_eq!(s.fake.lives(837), 0, "за сетевым сбоем не читаем");
    s.fake.edit(836, |t| t.life_fails = 0);
    let fixed = s.ok(30);
    assert_eq!(fixed.next.since, s.fake.changed(837), "сеть починилась — отметка за последней: {}", show(&fixed));
}

// =====================================================================================================================
// 7д–7е. Моя, которая не выгружается; сломанная за моей
// =====================================================================================================================

#[test]
fn my_ticket_that_never_exports_is_given_up_with_its_error_and_tried_again_when_it_changes() {
    // C# 7д: история моей — 404: после пяти заходов (и часа) пропущена — заход прошёл, отметка на ней; изменилась — снова
    let mut s = Session::new("mine-given-up", vec![T { life_fails: 404, ..T::mine(838) }.at(t0())]);
    let first = s.now;
    for k in 1..=5 {
        let p = s.pass(30);
        assert!(p.error.contains("#838") && p.error.contains("HTTP 404"), "заход {k}: {}", show(&p));
        assert_eq!(failure_of(&p, 838).times, k);
    }
    s.now = first + Span::minutes(60);
    let p = s.ok(30);
    assert_eq!(p.given_up.iter().map(|f| f.id).collect::<Vec<_>>(), [838], "{}", show(&p));
    assert!(p.given_up[0].error.contains("HTTP 404"), "{:?}", p.given_up[0]);
    assert_eq!((p.exported, s.file(838).is_none(), p.next.since), (0, true, t0()), "файла нет, отметка — на ней");
    assert!(!p.failures.items.contains_key(&838), "отметка её прошла — из счёта вон");
    // изменилась и починена — пробуем снова
    s.fake.edit(838, |t| {
        t.life_fails = 0;
        t.changed = t0() + min(1);
        t.events
            .push(Ev { at: t0() + min(1), editor: REQUESTER, editor_id: REQUESTER_ID, status: None, comment: Some("Ну что там?") });
    });
    let again = s.ok(30);
    assert_eq!(again.exported, 1, "{}", show(&again));
    assert!(s.file(838).is_some());
}

#[test]
fn a_broken_ticket_behind_my_failing_one_is_given_up_but_not_reread_and_is_named_when_the_mark_passes_it() {
    // C# 7е: сломанная (500) — за моей, которая не выгружается из-за сети (503): сломанную пора пропустить, но отметка до неё не
    // дошла — счёт не теряется (не перечитываем); когда сеть починилась, она в итоге захода — один раз, отметка за ней
    let mut s = Session::new(
        "behind-network",
        vec![T { life_fails: 503, ..T::mine(842) }.at(t0()), T { life_fails: 500, ..T::foreign(843) }.at(t0() + min(1))],
    );
    let first = s.now;
    for k in 1..=5 {
        let p = s.pass(30);
        assert!(p.error.contains("#842") && p.error.contains("HTTP 503"), "заход {k}: {}", show(&p));
        assert_eq!(failure_of(&p, 843).times, k, "сломанная считается, 842 — нет");
        assert!(!p.failures.items.contains_key(&842));
    }
    s.now = first + Span::minutes(61);
    for _ in 0..2 {
        let p = s.pass(30);
        assert!(p.given_up.is_empty() && p.next == ExportMark::start(now()), "отметка до неё не дошла: {}", show(&p));
        assert_eq!(failure_of(&p, 843).times, 5, "счёт цел");
    }
    assert_eq!(s.fake.lives(843), 5, "пропущена, но не перечитана");
    s.fake.edit(842, |t| t.life_fails = 0);
    let fixed = s.ok(30);
    assert!(s.file(842).is_some(), "моя выгружена");
    assert_eq!(fixed.given_up.iter().map(|f| f.id).collect::<Vec<_>>(), [843], "{}", show(&fixed));
    assert_eq!(fixed.next.since, s.fake.changed(843), "отметка за ней");
    assert!(fixed.failures.items.is_empty());
}

// =====================================================================================================================
// 7з. Файл не записать
// =====================================================================================================================

#[test]
fn when_the_folder_cannot_be_written_nothing_is_counted_against_the_ticket() {
    // C# 7з: беда папки (месяц занят файлом — не пишется ни временный): заход не удался, сколько ни длится, но в счёт заявки не
    // идёт и не пропускается; место появилось — выгружена
    let mut s = Session::new("folder-trouble", vec![T::mine(849).at(t0())]);
    fs::create_dir_all(s.dir.join(TICKETS_FOLDER)).unwrap();
    fs::write(s.dir.join("tickets/2026-11"), "я файл, а не папка").unwrap();
    let first = s.now;
    for k in 1..=6 {
        s.now = first + Span::minutes(61 * k);
        let p = s.pass(30);
        assert!(p.error.contains("#849"), "заход {k}: {}", show(&p));
        assert!(p.failures.items.is_empty() && p.given_up.is_empty(), "заход {k}: беда папки не в счёт заявки");
        assert_eq!(p.next, ExportMark::start(now()), "заход {k}: отметка стоит");
    }
    fs::remove_file(s.dir.join("tickets/2026-11")).unwrap();
    let fixed = s.ok(30);
    assert_eq!((fixed.exported, s.file(849).is_some()), (1, true), "{}", show(&fixed));
}

#[test]
fn when_a_folder_takes_the_place_of_the_file_it_is_the_tickets_trouble_and_it_is_given_up() {
    // C# 7з: временный записан, а на месте самого файла — папка: беда этой заявки — пять заходов, и пропуск (час — сдвигом now);
    // в итоге — «файл не записан»
    let mut s = Session::new("path-blocked", vec![T::mine(847).at(t0())]);
    fs::create_dir_all(s.dir.join("tickets/2026-11/847 — Заявка 847.md")).unwrap();
    let first = s.now;
    for k in 1..=5 {
        let p = s.pass(30);
        assert!(p.error.contains("#847") && p.failures.items.contains_key(&847), "заход {k}: {}", show(&p));
        assert_eq!((failure_of(&p, 847).times, p.next == ExportMark::start(now())), (k, true), "заход {k}");
    }
    s.now = first + Span::minutes(60);
    let p = s.ok(30);
    assert_eq!(p.given_up.iter().map(|f| f.id).collect::<Vec<_>>(), [847], "{}", show(&p));
    assert!(p.given_up[0].error.contains("файл не записан"), "{:?}", p.given_up[0]);
    assert_eq!((p.exported, p.next.since), (0, t0()), "отметка — за ней");
}

// =====================================================================================================================
// 9. Список, которому нельзя верить
// =====================================================================================================================

#[test]
fn rows_without_a_change_date_or_out_of_order_stop_the_pass_before_any_history_is_read() {
    // C# 9: строки без Changed и не по возрастанию — дальше не идём: отметка стоит, историй не читаем; исправился — заход прошёл
    let mut s = Session::new("disorder", vec![T::foreign(850).at(t0() + min(1)), T::mine(851).at(t0() + min(2))]);
    s.mark = ExportMark { since: t0(), seen: vec![5] };
    s.not_mine = BTreeMap::from([(900, t0())]);
    let mark = s.mark.clone();
    s.fake.switch(|sw| sw.no_changed = true);
    let p = s.pass(30);
    assert!(p.error.contains("#850") && p.error.contains("нет даты изменения"), "{}", show(&p));
    assert_eq!((p.next, p.not_mine.keys().copied().collect::<Vec<_>>()), (mark.clone(), vec![900]), "отметка и память стоят");
    assert_eq!(s.fake.spent(), (0, 0, 0), "историй и карточек не читаем");
    s.fake.switch(|sw| (sw.no_changed, sw.ignore_sort) = (false, true));
    let p = s.pass(30);
    assert!(p.error.contains("не по возрастанию") && p.error.contains("#851") && p.error.contains("#850"), "{}", show(&p));
    assert_eq!(p.next, mark, "отметка стоит");
    assert_eq!(s.fake.spent(), (0, 0, 0));
    s.fake.switch(|sw| sw.ignore_sort = false);
    let fixed = s.ok(30);
    assert_eq!((fixed.exported, fixed.next.since), (1, t0() + min(2)), "сервер исправился: {}", show(&fixed));
}

#[test]
fn a_page_of_only_older_tickets_from_a_server_that_ignored_the_date_is_an_error_and_the_mark_stays() {
    // C# 9а: сервер не применил ChangedMoreThan, и вся страница — старые заявки (страница сервера — три строки, всего больше):
    // заход не удался, отметка стоит, в ошибке — почему; сервер исправился — заход прошёл
    let old = |id: i32, days: i64| T::mine(id).at(t0() - Span::days(days));
    let mut s = Session::new("ignored-date", vec![old(860, 12), old(861, 11), old(862, 10), old(863, 9), T::mine(864).at(t0() + min(1))]);
    s.mark = ExportMark { since: t0(), seen: Vec::new() };
    let mark = s.mark.clone();
    s.fake.switch(|sw| (sw.ignore_changed, sw.page_cap) = (true, 3));
    let p = s.pass(30);
    assert!(p.error.contains("ChangedMoreThan"), "{}", show(&p));
    assert_eq!((p.next, p.exported, p.more), (mark.clone(), 0, false));
    assert_eq!(s.fake.spent(), (0, 0, 0), "старые строки не разбираем");
    s.fake.switch(|sw| (sw.ignore_changed, sw.page_cap) = (false, 0));
    let fixed = s.ok(30);
    assert_eq!((fixed.exported, fixed.next.since), (1, t0() + min(1)), "{}", show(&fixed));
}

#[test]
fn without_a_paginator_a_short_page_far_before_the_mark_is_the_same_error_but_a_short_page_near_it_is_not() {
    // C# 9а: без Paginator и короче запрошенной — полной не назвать, но первая строка раньше отметки дальше, чем бывает сдвиг
    // поясов: сервер не применил условие; в пределах сдвига — «новых нет», не ошибка
    let mut s = Session::new("no-paginator", vec![T::mine(860).at(t0() - Span::days(10)), T::mine(861).at(t0() - Span::days(9))]);
    s.mark = ExportMark { since: t0(), seen: Vec::new() };
    s.fake.switch(|sw| (sw.ignore_changed, sw.page_cap, sw.no_paginator) = (true, 3, true));
    let p = s.pass(30);
    assert!(p.error.contains("ChangedMoreThan"), "{}", show(&p));
    assert_eq!((p.next, p.exported), (s.mark.clone(), 0));
    // рядом с отметкой: сутки назад — это поясной сдвиг, их пропускает Covers
    let mut near = Session::new("no-paginator-near", vec![T::mine(860).at(t0() - Span::days(1))]);
    near.mark = ExportMark { since: t0(), seen: Vec::new() };
    near.fake.switch(|sw| (sw.ignore_changed, sw.no_paginator) = (true, true));
    let p = near.ok(30);
    assert_eq!((p.exported, p.more, p.next.clone()), (0, false, near.mark.clone()), "{}", show(&p));
    assert_eq!(near.fake.spent(), (0, 0, 0));
}

#[test]
fn a_server_that_reads_the_date_minutes_early_gives_covered_rows_that_cost_nothing_and_the_mark_never_goes_back() {
    // C# 9а (сдвиг поясов): заявки чуть раньше отметки пропускаются (уже разобраны), новая — выгружается тем же заходом,
    // отметка назад не идёт; заявку отметки открыли снова — короткая страница ранних — «новых нет», не ошибка
    let since = t0() + sec(30);
    let mut s = Session::new("skew", vec![T::mine(870).at(t0() - min(2)), T::mine(871).at(t0() - min(1)), T::mine(873).at(t0() + min(1))]);
    s.mark = ExportMark { since, seen: Vec::new() };
    s.fake.switch(|sw| sw.skew_minutes = 3);
    let p = s.ok(1);
    assert_eq!((p.exported, p.more), (1, false), "ранние не стоят места в порции: {}", show(&p));
    assert!(s.file(873).is_some() && s.file(870).is_none() && s.file(871).is_none());
    assert_eq!(s.fake.spent(), (1, 1, 0), "один запрос карточки и одна история — у 873");
    assert!(p.next.since >= since && p.next.since == t0() + min(1), "{}", show(&p));
    s.fake.edit(873, |t| t.status = WORKING);
    let quiet = s.ok(30);
    assert_eq!((quiet.exported, quiet.more, quiet.next.clone()), (0, false, p.next.clone()), "{}", show(&quiet));
}

#[test]
fn a_history_that_is_cut_does_not_make_a_ticket_not_mine_forever() {
    // C# 9г: история длиннее, чем читаем (обрезана): моё действие могло остаться в непрочитанной части — заявку выгружаем, а не
    // записываем в «не мои» (лишний файл лучше потерянной заявки)
    let mut s = Session::new("cut-history", vec![T { long_history: true, executors: "Петров П.", ..T::foreign(857) }.at(t0())]);
    let p = s.ok(30);
    assert!(s.file(857).is_some(), "{}", show(&p));
    assert_eq!((p.exported, p.not_mine.len()), (1, 0), "{}", show(&p));
}

#[test]
fn equal_changes_in_descending_number_order_and_a_repeated_row_are_not_a_disorder_and_are_exported_once() {
    // C# 9б: равные изменения, номера по убыванию (вторую часть sort сервер не соблюл) — не беспорядок; строку сервер повторил —
    // выгружена один раз; отметка — на их минуте с обоими номерами
    let mut s = Session::new("ties", vec![T::mine(855).at(t0() + sec(5)), T::mine(856).at(t0() + sec(5))]);
    s.fake.switch(|sw| (sw.ties_descending, sw.repeat_row) = (true, 856));
    let p = s.ok(30);
    assert_eq!(p.exported, 2, "{}", show(&p));
    assert!(s.file(855).is_some() && s.file(856).is_some());
    assert_eq!(p.next, ExportMark { since: t0() + sec(5), seen: vec![855, 856] }, "{}", show(&p));
    assert_eq!((s.fake.cards(856), s.fake.lives(856)), (1, 1), "повторённая — один раз");
}

#[test]
fn more_changes_in_one_minute_than_the_server_gives_at_once_read_what_is_given_and_then_say_so() {
    // C# 9в: в одну минуту изменилось больше заявок, чем сервер отдаёт за раз (страница 5, заявок 8), и все отданные разобраны —
    // ошибка, а не тишина; размер страницы — 500 и номера минуты отметки; сервер отдаёт больше — заход проходит
    let bulk = t0() + sec(5);
    let mut s = Session::new("bulk", (890..898).map(|id| T::foreign(id).at(bulk)).collect());
    s.fake.switch(|sw| sw.page_cap = 5);
    let one = s.ok(30);
    assert_eq!((one.exported, one.more, one.next.seen.len()), (0, true, 5), "разобрано, что отдано; ещё есть: {}", show(&one));
    let mark = s.mark.clone();
    let two = s.pass(30);
    assert!(two.error.contains("за раз") && two.error.contains("(5)"), "{}", show(&two));
    assert_eq!((two.next, two.more), (mark, false), "отметка стоит");
    assert!(s.fake.lists()[1].contains("pagesize=505&"), "500 строк и номера минуты отметки: {}", s.fake.lists()[1]);
    s.fake.switch(|sw| sw.page_cap = 0);
    let three = s.ok(30);
    assert_eq!(three.next.seen, (890..898).collect::<Vec<_>>(), "{}", show(&three));
    assert!(!three.more);
}

// =====================================================================================================================
// Зеркало (MirrorCheck)
// =====================================================================================================================

fn mirror_tickets() -> Vec<T> {
    vec![
        T::mine(901).at(t0() + min(1)).file(9901, "экран.png", &[1, 2]),
        T::foreign(902).at(t0() + min(2)).file(9902, "отчёт.pdf", &[3]),
        T::foreign(903)
            .at(t0() + min(3))
            .said(t0() + min(1), ME, ME_ID, "Посмотрел логи")
            .moved(t0() + min(2), "Иванов И.", OTHER_ID, FINAL)
            .file(9903, "лог.txt", &[4]),
        T::foreign(904).at(t0() + min(4)).status(WORKING),
        T::mine(905).at(t0() + min(5)).status(WORKING),
        T::foreign(906).at(now() - Span::days(8)),
    ]
}

fn mirror(tag: &str, tickets: Vec<T>, reach: ExportReach) -> Session {
    let mut s = Session::new(tag, tickets);
    s.scope = scope(reach, AttachmentScope::All);
    s
}

#[test]
fn the_mirror_of_all_closed_takes_foreign_tickets_without_a_history_read_to_decide_and_downloads_only_mine() {
    // MirrorCheck 1: «все закрытые» — и чужие, историю читаем только для файла; вложения — только моей и той, где я писал;
    // у чужой в файле «не скачано» и роли нет; один список со статусами, страница 500
    let mut s = mirror("mirror-closed", mirror_tickets(), ExportReach::Closed);
    let p = s.ok(30);
    assert_eq!(p.exported, 3, "{}", show(&p));
    assert_eq!(s.written(&[901, 902, 903, 904, 905, 906]), [901, 902, 903], "выгружены закрытые; открытые и давняя — нет");
    let lists = s.fake.lists();
    assert!(lists.len() == 1 && lists[0].contains("StatusIds=") && lists[0].contains("&pagesize=500&"), "{lists:?}");
    for id in [901, 902, 903] {
        assert_eq!((s.fake.cards(id), s.fake.lives(id)), (1, 1), "по карточке и истории на заявку — #{id}");
    }
    assert_eq!((s.fake.file_requests(9901), s.fake.file_requests(9903), s.fake.file_requests(9902)), (1, 1, 0), "вложения — только моих");
    assert!(s.text(902).contains("отчёт.pdf · не скачано") && !s.text(902).contains("\nmy_role:"), "{}", s.text(902));
    assert!(s.text(901).contains("\nmy_role: \"исполнитель\"\n") && s.text(903).contains("\nmy_role: \"участник\"\n"));
}

#[test]
fn the_mirror_budget_counts_every_row_and_goes_three_three_one() {
    // MirrorCheck 3: порция 3 из семи новых — «ещё есть» и следующий заход сразу; порции 3, 3, 1 по порядку изменения
    let tickets = (0..7)
        .map(|i| {
            let id = 910 + i;
            (if i % 2 == 0 { T::mine(id) } else { T::foreign(id) }).at(t0() + min(i64::from(i)))
        })
        .collect();
    let mut s = mirror("mirror-portions", tickets, ExportReach::Closed);
    let mut cards = Vec::new();
    let mut more = Vec::new();
    for _ in 0..3 {
        let before = s.fake.spent().0;
        let p = s.ok(3);
        cards.push(s.fake.spent().0 - before);
        more.push(p.more);
    }
    assert_eq!((cards, more), (vec![3, 3, 1], vec![true, true, false]));
    assert_eq!(s.written(&(910..917).collect::<Vec<_>>()), (910..917).collect::<Vec<_>>());
    assert!((910..917).all(|id| s.fake.cards(id) == 1), "ни одна не прочитана дважды");
}

#[test]
fn files_that_are_already_current_cost_nothing_and_only_the_changed_ticket_is_reread() {
    // MirrorCheck 4: отметку забыли (как после полной выгрузки из «Поиска заявок»): заход с недели назад проходит десять
    // выгруженных без карточек и историй и с порцией 3 не «догоняет»; изменённая — перечитана
    let tickets = (0..10)
        .map(|i| {
            let id = 910 + i;
            (if i % 2 == 0 { T::mine(id) } else { T::foreign(id) }).at(t0() + min(i64::from(i)))
        })
        .collect();
    let mut s = mirror("mirror-current", tickets, ExportReach::Closed);
    s.ok(30);
    s.fake.edit(912, |t| {
        t.events.push(Ev {
            at: t0() + min(20),
            editor: REQUESTER,
            editor_id: REQUESTER_ID,
            status: None,
            comment: Some("А отчёт за сентябрь?"),
        });
        t.changed = t0() + min(20);
    });
    s.mark = ExportMark::start(now());
    s.not_mine.clear();
    let before = s.fake.spent();
    let p = s.ok(3);
    assert_eq!((p.exported, p.more), (1, false), "{}", show(&p));
    let after = s.fake.spent();
    assert_eq!((after.0 - before.0, after.1 - before.1), (1, 1), "только изменённая перечитана, остальные — без запросов");
    assert_eq!((s.fake.cards(912), s.fake.cards(911)), (2, 1));
    assert!(s.text(912).contains("отчёт за сентябрь"), "{}", s.text(912));
    assert_eq!(p.next.since, t0() + min(20), "отметка — на последней");
}

#[test]
fn the_mirror_of_all_tickets_lists_without_statuses_and_files_that_are_current_do_not_spend_the_budget() {
    // MirrorCheck 5: «все заявки» — список без статусов; открытые выгружены, давняя — нет; уже верные (и чужая с «не скачано» по
    // правилу зеркала) порцию не тратят: порция 2 ровно на две открытые — не «ещё есть»
    let mut s = mirror("mirror-all", mirror_tickets(), ExportReach::Closed);
    s.ok(30);
    s.scope.reach = ExportReach::All;
    s.mark = ExportMark::start(now());
    let before = s.fake.spent();
    let p = s.ok(2);
    let last = s.fake.lists().pop().unwrap();
    assert!(!last.contains("StatusIds"), "список без статусов: {last}");
    assert_eq!((p.exported, p.more), (2, false), "{}", show(&p));
    assert!(s.file(904).is_some() && s.file(905).is_some() && s.file(906).is_none(), "открытые выгружены, давняя нет");
    assert_eq!(s.fake.spent().0 - before.0, 2, "уже верные — без запросов");
}

#[test]
fn a_pass_for_my_tickets_after_the_mirror_asks_nothing_for_files_that_are_right() {
    // MirrorCheck 7: «мои» после зеркала и сброса отметки: чужие с верным файлом — без чтения истории, карточек нет; мои и в счёт
    // не шли — не «ещё есть»
    let mut s = mirror("mirror-then-mine", mirror_tickets(), ExportReach::Closed);
    s.ok(30);
    s.scope.reach = ExportReach::Mine;
    s.mark = ExportMark::start(now());
    let before = s.fake.spent();
    let p = s.ok(30);
    assert_eq!(s.fake.spent(), before, "ни карточек, ни историй, ни файлов");
    assert_eq!((p.exported, p.more), (0, false), "{}", show(&p));
    assert!(s.fake.lists().pop().unwrap().contains("&pagesize=500&"), "страница и у моих — 500 строк");
}

// =====================================================================================================================
// 7ж. Заход и другая выгрузка: одна пишущая за раз
// =====================================================================================================================

/// Выгрузка, которая держит очередь писать: её карточка (заявка 9000) ждёт `release`. Возвращает, когда очередь уже у неё.
fn hold_the_turn(fake: &Fake, dir: &Path, turn: &Turn) -> thread::JoinHandle<()> {
    fake.put(T::mine(9000).at(t0()).status(WORKING)); // открытая: в список захода не попадает
    fake.switch(|sw| sw.hold_card = 9000);
    let (client, row, dir) = (fake.client(), fake.found(9000), dir.to_path_buf());
    let options = Options { turn: turn.clone(), ..fast() };
    let holder = thread::spawn(move || {
        let context =
            Context { me: Some(User { id: ME_ID, name: ME.into() }), closed: ClosedNames::default(), attachments: AttachmentScope::None };
        export_rows(&client, &[row], &dir, &url_of, &context, &|_| {}, &AtomicBool::new(false), &options);
    });
    wait_until("карточка держащей очередь выгрузки", || fake.cards(9000) > 0);
    holder
}

#[test]
fn a_pass_waits_for_another_writing_export_and_exports_after_it_is_done() {
    // C# 7ж: пока пишет другая выгрузка (окно «Поиск заявок»), автовыгрузка ждёт её, а не пишет рядом: первая карточка захода
    // приходит только после того, как другая отпущена (метка «#release» в журнале запросов раньше)
    let s = Session::new("pass-waits", vec![T::mine(846).at(t0() + min(1))]);
    let other = Tmp::new("pass-waits-other");
    let turn = Turn::private(); // одна очередь на обоих
    let options = Options { turn: turn.clone(), ..fast() };
    let holder = hold_the_turn(&s.fake, other.path(), &turn);
    let (client, scope, mark, dir) = (s.fake.client(), s.scope.clone(), s.mark.clone(), s.dir.path().to_path_buf());
    let pass = thread::spawn(move || {
        run_pass(
            &client,
            &scope,
            &mark,
            &BTreeMap::new(),
            &ExportFailures::default(),
            &dir,
            &url_of,
            30,
            now(),
            &AtomicBool::new(false),
            &options,
        )
    });
    wait_until("список захода", || !s.fake.lists().is_empty());
    // ждать, что чего-то нет, можно только до границы: заход, что не встал в очередь, спросил бы карточку за миллисекунды после
    // списка; медленная машина может лишь не поймать нарушение — верный заход от неё не упадёт
    let window = Instant::now() + Duration::from_millis(300);
    while Instant::now() < window && s.fake.cards(846) == 0 {
        thread::sleep(Duration::from_millis(2));
    }
    let early = s.fake.cards(846) > 0;
    s.fake.release();
    holder.join().unwrap();
    let p = pass.join().unwrap();
    assert!(!early, "карточка захода пришла, пока другая выгрузка держала очередь писать");
    let paths = s.fake.paths();
    let released = paths.iter().position(|p| p == "#release").unwrap();
    let card = paths.iter().position(|p| p.starts_with("/api/task/846?")).expect("карточка захода");
    assert!(card > released, "карточка захода пришла, пока другая выгрузка писала: {paths:?}");
    assert_eq!((p.error.as_str(), p.exported), ("", 1), "{}", show(&p));
    assert!(s.file(846).is_some());
}

#[test]
fn a_pass_that_is_stopped_while_another_export_writes_gives_up_the_wait_and_writes_nothing() {
    // C# 7ж: выключили выгрузку, пока заход ждал очереди, — не ждёт дальше и ничего не пишет (заход кончается, пока другая выгрузка
    // ещё держит очередь); остановка — не беда заявки: в счёт сбоев не идёт, отметка стоит
    let s = Session::new("pass-stopped", vec![T::mine(848).at(t0() + min(1))]);
    let other = Tmp::new("pass-stopped-other");
    let turn = Turn::private(); // одна очередь на обоих
    let options = Options { turn: turn.clone(), ..fast() };
    let holder = hold_the_turn(&s.fake, other.path(), &turn);
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (client, scope, dir) = (s.fake.client(), s.scope.clone(), s.dir.path().to_path_buf());
    let (stop2, done2) = (stop.clone(), done.clone());
    let pass = thread::spawn(move || {
        let p = run_pass(
            &client,
            &scope,
            &ExportMark::start(now()),
            &BTreeMap::new(),
            &ExportFailures::default(),
            &dir,
            &url_of,
            30,
            now(),
            &stop2,
            &options,
        );
        done2.store(true, SeqCst);
        p
    });
    wait_until("список захода", || !s.fake.lists().is_empty());
    stop.store(true, SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done.load(SeqCst) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    let finished = done.load(SeqCst);
    s.fake.release(); // как бы ни кончилось — другую отпускаем
    holder.join().unwrap();
    let p = pass.join().unwrap();
    assert!(finished, "заход не бросил ждать очередь, пока другая выгрузка держала её");
    assert!(p.error.contains("ничего не записано"), "{}", show(&p));
    assert_eq!((p.exported, p.next.clone(), p.failures.items.len()), (0, ExportMark::start(now()), 0), "{}", show(&p));
    assert!(s.file(848).is_none() && s.fake.cards(848) == 0, "ничего не записано и карточка не спрошена");
}

#[test]
fn a_foreign_ticket_remembered_with_the_same_change_costs_nothing_and_one_that_changed_is_read_again() {
    // `not_mine` — вход и выход захода: известная «не моя» с тем же изменением не стоит ни запроса, ни места в порции; изменилась —
    // читается заново
    let mut s = Session::new("not-mine-kept", vec![T::foreign(831).at(t0()), T::mine(832).at(t0() + sec(10))]);
    s.mark = ExportMark { since: t0(), seen: Vec::new() };
    s.not_mine = BTreeMap::from([(831, t0())]);
    let p = s.ok(1);
    assert_eq!(
        (s.fake.lives(831), p.exported, p.more),
        (0, 1, false),
        "известная «не моя» — без запроса и без места в порции: {}",
        show(&p)
    );
    assert_eq!(p.not_mine, BTreeMap::from([(831, t0())]));
    // изменилась — «не моя» с прежней датой уже не знакома
    let mut s = Session::new("not-mine-changed", vec![T::foreign(831).at(t0() + sec(10))]);
    s.mark = ExportMark { since: t0(), seen: Vec::new() };
    s.not_mine = BTreeMap::from([(831, t0())]);
    let p = s.ok(30);
    assert_eq!(s.fake.lives(831), 1, "изменилась — история читается заново: {}", show(&p));
    assert_eq!(p.not_mine, BTreeMap::from([(831, t0() + sec(10))]));
}

// =====================================================================================================================
// Раунд 4.5, карточка 4.5-4: что видно из итога захода (P5, M1)
// =====================================================================================================================

#[test]
fn a_tickets_folder_that_cannot_be_made_leaves_the_state_of_the_pass_as_it_was() {
    // C# `KnowledgeExport.cs` 175 бросает внутри `InTurnAsync`, окно (`AutoExport.cs` 115–118) — `AutoExportFailed`: ни отметка, ни
    // «не мои», ни счёт сбоев не тронуты. «Не моя» #5 стоит перед моей #7 и разобрана (история прочитана) — но её файл не нужен, а
    // отметку за ней двигать нельзя: записать #7 не вышло (на месте `tickets/` — файл), и первая же запись заход закончила
    let mut s = Session::new("tickets-is-a-file", vec![T::foreign(5).at(t0()), T::mine(7).at(t0() + min(1))]);
    fs::write(s.dir.join(TICKETS_FOLDER), "я файл, а не папка").unwrap();
    s.not_mine = BTreeMap::from([(900, t0())]);
    let before = (s.mark.clone(), s.not_mine.clone());
    let p = s.pass(30);
    assert!(!p.error.is_empty(), "заход должен сказать, что не удалось: {}", show(&p));
    assert_eq!(p.next, before.0, "отметка не сдвинута за строкой #5: {}", show(&p));
    assert_eq!(p.not_mine, before.1, "память «не мои» — как была, #5 в неё не вошла: {}", show(&p));
    assert!(p.failures.items.is_empty() && p.given_up.is_empty(), "счёт сбоев — как был: {:?}", p.failures.items);
    assert_eq!(p.exported, 0, "{}", show(&p));
}

/// Заход по странице, где все строки раньше минуты отметки, но в пределах двух суток сдвига поясов (отметка — `t0`, строки —
/// сутки назад), и одно условие «вне границы» из четырёх; ответ — ошибка захода.
fn page_before_the_mark(tag: &str, count: usize, seen: usize, set: impl FnOnce(&mut Switches)) -> ClosedExportPass {
    let tickets: Vec<T> = (0..count).map(|i| T::mine(1000 + i as i32).at(t0() - Span::days(1) + sec(i as i64 % 50))).collect();
    let mut s = Session::new(tag, tickets);
    s.mark = ExportMark { since: t0(), seen: (1..=seen as i32).collect() };
    s.fake.switch(|sw| {
        sw.ignore_changed = true;
        set(sw);
    });
    s.pass(30)
}

fn says_the_server_ignored_the_date(p: &ClosedExportPass, what: &str) {
    assert!(p.error.contains("ChangedMoreThan") && p.error.contains("отметка стоит"), "{what}: {}", show(p));
    assert_eq!((p.exported, p.more), (0, false), "{what}: {}", show(p));
}

#[test]
fn a_page_of_exactly_the_asked_size_before_the_mark_is_a_server_that_ignored_the_date() {
    // C# `ClosedExport.cs` 222: `list.Found.Count >= pageSize` — страница полная (500 строк, без Paginator, счёт = строкам): даже
    // в пределах сдвига поясов это не сдвиг, а сервер без условия по дате
    says_the_server_ignored_the_date(&page_before_the_mark("beyond-full", 500, 0, |sw| sw.no_paginator = true), "полная страница");
}

#[test]
fn a_short_page_with_a_next_page_before_the_mark_is_a_server_that_ignored_the_date() {
    // C# 223: `list.HasNext == true` — короткая страница, счёт = строкам, но сервер говорит «есть следующая»
    let p = page_before_the_mark("beyond-next", 3, 0, |sw| sw.force_next = Some(true));
    says_the_server_ignored_the_date(&p, "есть следующая страница");
}

#[test]
fn a_short_page_with_a_total_above_its_rows_before_the_mark_is_a_server_that_ignored_the_date() {
    // C# 223: `list.Total > list.Found.Count` — короткая страница без «есть следующая», но счёт больше строк
    let p = page_before_the_mark("beyond-total", 3, 0, |sw| (sw.force_count, sw.force_next) = (Some(8), Some(false)));
    says_the_server_ignored_the_date(&p, "счёт больше строк");
}

#[test]
fn a_total_on_the_ceiling_before_the_mark_is_a_server_that_ignored_the_date() {
    // C# 224: `HttpIntraserviceClient.Capped(list.Total)` — счёт на потолке (1000) — нижняя граница: страница из 1000 строк короче
    // запрошенной (500 + 600 номеров минуты = 1100), следующей нет, счёт равен строкам — из четырёх условий верно одно, потолок
    let p = page_before_the_mark("beyond-capped", 1000, 600, |sw| sw.force_next = Some(false));
    says_the_server_ignored_the_date(&p, "счёт на потолке");
}

#[test]
fn a_short_page_before_the_mark_with_nothing_beyond_is_only_a_time_zone_shift() {
    // контроль к четырём: короткая страница, счёт = строкам, следующей нет, не потолок — сдвиг поясов, «новых нет», не ошибка
    let p = page_before_the_mark("beyond-none", 3, 0, |_| {});
    assert_eq!((p.error.as_str(), p.exported, p.more), ("", 0, false), "{}", show(&p));
}
