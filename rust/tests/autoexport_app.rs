//! Автовыгрузка между заходами (карточка 4.5-3): состояние `AutoExport`, файл отметки `autoexport-rs.json`, строка итога для
//! подсказки трея, строки `errors.log`, подсказка трея и её связка с окном — без таймера (его 15 с и период — `FIRST_PASS`,
//! `next_pass_in`; сам таймер Slint на Linux без экрана не проверить, окно вызывает заход через `App::auto_export_now`).
//! Сценарии — из `TicketBoard.SelfCheck/AutoExportCheck.cs` (1, 3, 7а, 7ж, 10, 10б–10ж, 12 в том виде, что остаётся от неё при
//! «настройки читаются при запуске», 13) и `MirrorCheck.cs` (2, 6, 9, 10, 11): тексты итога и лога — слово в слово из
//! `MainViewModel.AutoExport.cs`. Сам заход (`run_pass`) и правила (`closed`) — `autoexport_pass.rs`, `autoexport_rules.rs`.
//! Заход — против поддельного сервера на loopback (он — копия сервера `autoexport_pass.rs` + «кто я», справочник статусов, задержка
//! карточки); часы — `now`, который проверка передаёт заходу (время в итоге — по его поясу).
//!
//! Окно: одна проверка на файл с окном (платформа Slint ставится один раз на процесс), остальные окон не создают.

mod common;

use chrono::{Duration as Span, FixedOffset, TimeZone};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use ticketboard::autoexport::{
    AutoExport, FIRST_PASS, MARK_FILE, SavedMark, account_key, next_pass_in, read_mark, same_folder, tray_tip, write_mark,
};
use ticketboard::export::closed::ExportReach;
use ticketboard::export::run::{Options, Turn, export_rows};
use ticketboard::export::{AttachmentScope, Context};
use ticketboard::intraservice::{Client, Date, Found, User};
use ticketboard::rules::ClosedNames;
use ticketboard::settings::Settings;

// ---------- время и номера ----------

const ME_ID: i32 = 7;
const ME: &str = "Я Сам";
const OTHER_ID: i32 = 60;
const REQUESTER: &str = "Петрова А.";
const REQUESTER_ID: i32 = 50;
const WORKING: i32 = 32;
const FINAL: i32 = 30;

fn zone() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).unwrap()
}

fn at(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> Date {
    zone().with_ymd_and_hms(y, m, d, h, mi, s).unwrap()
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
    /// «Кто я» отвечает этим кодом (0 — как обычно).
    user_fails: u16,
    /// Справочник статусов вместо обычного (None — обычный).
    statuses: Option<&'static str>,
    /// Любая карточка отвечает не раньше чем через столько.
    card_delay: Duration,
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
        if path.starts_with("/api/user?") {
            if sw.user_fails != 0 {
                return json(sw.user_fails, String::new());
            }
            return json(200, format!(r#"{{"Id":{ME_ID},"Name":"{ME}"}}"#));
        }
        if path.starts_with("/api/taskstatus") {
            return json(200, sw.statuses.unwrap_or(STATUSES).to_string());
        }
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
            thread::sleep(sw.card_delay);
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
            "Count": total, "Page": 1, "PageCount": total.div_ceil(size.max(1)).max(1), "HasNextPage": total > page.len(),
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
        let path = std::env::temp_dir().join(format!("tb-autoapp-{tag}-{}-{}", std::process::id(), N.fetch_add(1, SeqCst)));
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

fn fast() -> Options {
    Options { retry_delays: vec![Duration::ZERO; 3], vanish_wait: Duration::ZERO, turn: Turn::private(), ..Options::default() }
}

fn wait_until(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "не дождались: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

const HEAD: &str = "Заявки — прототип на Rust";
const MARK_KEPT: &str = "2026-11-25 10:00";
const WEEK_BACK: &str = "2026-11-19 12:00";

/// Папка данных (рядом с «exe»), поддельный сервер и настройки: автовыгрузка включена, автообновление 5 минут, охват «мои».
struct Env {
    fake: Fake,
    data: Tmp,
    settings: Settings,
}

impl Env {
    fn new(tag: &str, tickets: Vec<T>) -> Env {
        let fake = Fake::new(tickets);
        let settings = Settings {
            base_url: fake.url.clone(),
            login: "me".into(),
            auto_export_closed: true,
            auto_sync_minutes: 5,
            ..Settings::default()
        };
        Env { fake, data: Tmp::new(tag), settings }
    }

    /// Автовыгрузка доски, что пишется: журнал — в папке данных.
    fn export(&self) -> AutoExport {
        AutoExport::new(&self.settings, self.data.path(), Some(self.data.path().to_path_buf()))
    }

    fn pass(&self, ax: &mut AutoExport, at: Date) -> Option<String> {
        ax.pass(&self.fake.client(), at, &AtomicBool::new(false), &fast())
    }

    /// Заход, что обязан дойти до итога.
    fn line(&self, ax: &mut AutoExport, at: Date) -> String {
        self.pass(ax, at).expect("заход пропущен, а должен был идти")
    }

    /// Строки `errors.log`: без метки времени.
    fn log(&self) -> Vec<String> {
        let text = fs::read_to_string(self.data.join("errors.log")).unwrap_or_default();
        text.split("\n\n").filter(|b| !b.trim().is_empty()).map(|b| b.split_once("] ").map_or(b, |(_, t)| t).to_string()).collect()
    }

    fn mark(&self) -> Option<SavedMark> {
        read_mark(self.data.path())
    }

    /// Файл заявки в базе знаний (по умолчанию — `knowledge` рядом с данными).
    fn file(&self, id: i32) -> Option<PathBuf> {
        self.file_in(&self.data.join("knowledge"), id)
    }

    fn file_in(&self, root: &Path, id: i32) -> Option<PathBuf> {
        let tickets = root.join("tickets");
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

    /// С какой минуты просил список последний заход.
    fn since(&self) -> String {
        let last = self.fake.lists().pop().expect("заход не просил список");
        param(&last, "ChangedMoreThan").unwrap_or_default()
    }

    fn requests(&self) -> usize {
        self.fake.paths().len()
    }
}

fn account_of(settings: &Settings) -> String {
    account_key(settings)
}

/// Отметка на диске, как её пишет заход в папку по умолчанию.
fn saved(env: &Env, since: Date, seen: &[i32]) -> SavedMark {
    SavedMark { folder: String::new(), account: account_of(&env.settings), reach: ExportReach::Mine, since, seen: seen.to_vec() }
}

/// Итог без «за N с»: сколько длился заход — у каждой машины своё (его вес — `tray_line_says_how_long_a_slow_pass_took`).
fn plain(line: &str) -> String {
    match line.split_once(" за ") {
        Some((before, after)) => format!("{before}{}", after.split_once(" с").map_or("", |(_, rest)| rest)),
        None => line.to_string(),
    }
}

/// Сервер, которого нет: заход не доходит ни до одного запроса.
fn dead() -> Client {
    Client::new("http://127.0.0.1:1/", "me", "p", Duration::from_secs(5))
}

fn failed_line(hhmm: &str) -> String {
    format!("база знаний: не удалось ({hhmm}) — подробности в errors.log")
}

const NOT_DONE_YET: &str = "Автовыгрузка: ";

// =====================================================================================================================
// Свободные функции: период, подсказка трея, учётная запись, папка
// =====================================================================================================================

#[test]
fn the_first_pass_is_fifteen_seconds_after_start_and_then_every_period_but_not_faster_than_a_minute() {
    // C# `ApplyAutoSync` (15 с) и таймер: `Math.Max(1, AutoSyncMinutes)` минут
    assert_eq!(FIRST_PASS, Duration::from_secs(15));
    assert_eq!(next_pass_in(5), Duration::from_secs(5 * 60));
    assert_eq!(next_pass_in(1), Duration::from_secs(60));
    assert_eq!(next_pass_in(90), Duration::from_secs(90 * 60));
    assert_eq!(next_pass_in(0), Duration::from_secs(60), "ноль — не «без периода», а не чаще минуты");
    assert_eq!(next_pass_in(-3), Duration::from_secs(60), "отрицательный — как ноль");
}

#[test]
fn the_tray_tip_is_the_title_and_the_state_line_when_there_is_one() {
    assert_eq!(tray_tip(""), HEAD);
    assert_eq!(tray_tip("база знаний: 12:05, выгружено 3"), format!("{HEAD}\nбаза знаний: 12:05, выгружено 3"));
}

#[test]
fn the_tray_tip_keeps_127_utf16_units_and_cuts_longer_to_126_and_an_ellipsis() {
    // C# App.UpdateTrayIcon: `tip.Length <= 127 ? tip : tip[..126] + "…"` — .Length это единицы UTF-16, а не знаки и не байты
    let head = format!("{HEAD}\n");
    assert_eq!(head.encode_utf16().count(), 26);
    let exact = "я".repeat(101);
    let tip = tray_tip(&exact);
    assert_eq!(tip, format!("{head}{exact}"), "ровно 127 единиц — целиком");
    let longer = "я".repeat(102);
    let tip = tray_tip(&longer);
    assert_eq!(tip, format!("{head}{}…", "я".repeat(100)), "128 единиц — первые 126 и «…»");
    assert_eq!(tip.encode_utf16().count(), 127);
    // 100 «я» и один знак вне BMP — 101 знак, но 128 единиц UTF-16: счёт по знакам оставил бы строку целой
    let wide = format!("{}\u{1F600}", "я".repeat(100));
    assert_eq!(tray_tip(&wide), format!("{head}{}…", "я".repeat(100)), "считаются единицы UTF-16, не знаки");
}

#[test]
fn the_account_key_is_the_address_without_slashes_and_the_login_in_lower_case() {
    // C# `AccountKey`: `$"{url.Trim().TrimEnd('/')}|{login.Trim()}".ToLowerInvariant()`
    let key = |url: &str, login: &str| account_key(&Settings { base_url: url.into(), login: login.into(), ..Settings::default() });
    assert_eq!(key("https://Hd.Example.RU/", "Ivanov"), "https://hd.example.ru|ivanov");
    assert_eq!(
        key("  https://hd.example.ru///  ", "  Иванов И.  "),
        "https://hd.example.ru|иванов и.",
        "пробелы, все косые на конце, кириллица"
    );
    assert_eq!(key("https://hd", "me"), key("https://HD/", "ME"), "регистр и косая — та же запись");
    assert_ne!(key("https://hd", "me"), key("https://hd", "other"), "другой логин — другая");
    assert_ne!(key("https://hd", "me"), key("https://hd2", "me"), "другой адрес — другая");
    assert_eq!(key("", ""), "|");
}

#[test]
fn the_same_folder_is_compared_by_full_path_without_case_and_trailing_separator() {
    // C# `AppSettings.SameFolder`: пусто — «knowledge» рядом с данными; полный путь, без учёта регистра, без косой на конце
    let data = Path::new("/data/app");
    let same = |a: &str, b: &str| same_folder(a, b, data);
    assert!(same("", ""));
    assert!(same("", "/data/app/knowledge"), "пусто — knowledge рядом с данными");
    assert!(same("   ", "/data/app/knowledge/"), "пробелы — тоже пусто; косая на конце не в счёт");
    assert!(same("/DATA/App/Knowledge", ""), "регистр не в счёт");
    assert!(same(" /x/y ", "/x/y"), "пробелы вокруг пути срезаются");
    assert!(same("/x/y/", "/x/y"));
    assert!(same("/x/a/../y", "/x/y"), "полный путь — «..» разобран");
    assert!(!same("/data/app/other", ""));
    assert!(!same("/x/y", "/x/z"));
    assert!(!same_folder("", "/data/app/knowledge", Path::new("/moved")), "«knowledge» едет вместе с данными: у перенесённых — другая");
    assert!(same_folder("", "", Path::new("/moved")), "пусто и пусто — та же папка где бы ни лежали данные");
    assert!(same_folder("", "/moved/knowledge", Path::new("/moved")));
}

// =====================================================================================================================
// Файл отметки
// =====================================================================================================================

#[test]
fn the_mark_file_round_trips_with_rfc3339_time_and_is_written_atomically() {
    // решение карточки: autoexport-rs.json — папка как в настройках, учётная запись, охват, время (RFC 3339 со смещением), номера
    let dir = Tmp::new("mark-round");
    let mark = SavedMark {
        folder: "D:\\Знания".into(),
        account: "https://hd|me".into(),
        reach: ExportReach::All,
        since: at(2026, 11, 25, 10, 0, 7),
        seen: vec![3, 5],
    };
    assert_eq!(read_mark(dir.path()), None, "файла ещё нет — отметки нет");
    write_mark(dir.path(), &mark).unwrap();
    assert_eq!(read_mark(dir.path()), Some(mark.clone()));
    let text = fs::read_to_string(dir.join(MARK_FILE)).unwrap();
    assert!(text.contains("2026-11-25T10:00:07+03:00"), "время — RFC 3339 со смещением: {text}");
    let newer = SavedMark { reach: ExportReach::Closed, seen: vec![], since: at(2026, 11, 26, 9, 30, 0), ..mark };
    write_mark(dir.path(), &newer).unwrap();
    assert_eq!(read_mark(dir.path()), Some(newer), "вторая запись заменяет первую");
    let names: Vec<String> = fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, [MARK_FILE], "временных файлов не осталось: запись атомарная");
    assert_eq!(MARK_FILE, "autoexport-rs.json");
}

#[test]
fn a_missing_or_unreadable_mark_file_means_no_mark() {
    let dir = Tmp::new("mark-bad");
    for (what, text) in [("пустой", ""), ("не json", "это не json"), ("пустой объект", "{}"), ("массив", "[1, 2]"), ("оборван", "{\"")]
    {
        fs::write(dir.join(MARK_FILE), text).unwrap();
        assert_eq!(read_mark(dir.path()), None, "{what}: отметки нет, а не падение");
    }
    fs::remove_file(dir.join(MARK_FILE)).unwrap();
    fs::create_dir(dir.join(MARK_FILE)).unwrap();
    assert_eq!(read_mark(dir.path()), None, "на месте файла — папка");
}

// =====================================================================================================================
// Охрана: когда заход вообще идёт (C# 1, ExportCanRun)
// =====================================================================================================================

#[test]
fn can_run_wants_the_switch_a_period_a_client_and_a_board_that_is_written() {
    // C# `ExportCanRun`: включена, автообновление не на нуле, сервер задан; прототипу — ещё и доска, что пишется (журнал есть)
    let base = Settings { auto_export_closed: true, auto_sync_minutes: 5, ..Settings::default() };
    let dir = Tmp::new("can-run");
    let journal = Some(dir.path().to_path_buf());
    let can = |s: &Settings, client: bool, journal: &Option<PathBuf>| AutoExport::new(s, dir.path(), journal.clone()).can_run(client);
    assert!(can(&base, true, &journal), "всё есть — идёт");
    assert!(!can(&Settings { auto_export_closed: false, ..base.clone() }, true, &journal), "выключена");
    assert!(!can(&Settings { auto_sync_minutes: 0, ..base.clone() }, true, &journal), "автообновление 0");
    assert!(!can(&Settings { auto_sync_minutes: -1, ..base.clone() }, true, &journal), "автообновление меньше нуля");
    assert!(can(&Settings { auto_sync_minutes: 1, ..base.clone() }, true, &journal), "минута — ещё идёт");
    assert!(!can(&base, false, &journal), "сервера нет");
    assert!(!can(&base, true, &None), "доска только для чтения: ничего не пишется — и автовыгрузка тоже");
}

#[test]
fn a_switched_off_export_asks_nothing_keeps_no_file_and_says_nothing() {
    // C# 1: выключено — заход не начинается, ни одного запроса, в подсказке трея ничего
    let env = Env::new("off", vec![T::mine(1).at(t0())]);
    let mut off = Env { settings: Settings { auto_export_closed: false, ..env.settings.clone() }, ..env };
    let mut ax = off.export();
    assert_eq!(off.pass(&mut ax, now()), None);
    assert_eq!(off.requests(), 0, "ни одного запроса: {:?}", off.fake.paths());
    assert!(!off.data.join(MARK_FILE).exists() && !off.data.join("knowledge").exists(), "ни отметки, ни папки");
    assert!(off.log().is_empty());
    off.settings.auto_export_closed = true;
    off.settings.auto_sync_minutes = 0;
    let mut ax = off.export();
    assert_eq!(off.pass(&mut ax, now()), None, "автообновление 0 — тоже не идёт");
    assert_eq!(off.requests(), 0);
}

#[test]
fn a_read_only_board_runs_no_export_at_all() {
    // решение карточки: доска только для чтения (tickets.json не читается) — прототип нигде ничего не пишет, автовыгрузка тоже
    let env = Env::new("read-only", vec![T::mine(1).at(t0())]);
    let mut ax = AutoExport::new(&env.settings, env.data.path(), None);
    assert_eq!(env.pass(&mut ax, now()), None);
    assert_eq!(env.requests(), 0);
    assert!(!env.data.join(MARK_FILE).exists() && !env.data.join("knowledge").exists() && env.log().is_empty());
}

// =====================================================================================================================
// Отметка между заходами (C# 3, 10ж, 12)
// =====================================================================================================================

#[test]
fn the_first_pass_keeps_the_start_mark_before_any_request_and_a_failing_pass_does_not_move_it() {
    // C# 12: «неделя назад» — от включения, а не от первого удачного захода: отметка начала записана первым делом (до папки и до
    // любого запроса — сервера тут нет вовсе) и не ползёт, пока заходы не удаются
    let env = Env::new("first-mark", vec![]);
    let mut ax = env.export();
    let line = ax.pass(&dead(), now(), &AtomicBool::new(false), &fast());
    assert_eq!(line.as_deref(), Some(failed_line("12:00").as_str()));
    let week_ago = at(2026, 11, 19, 12, 0, 0);
    assert_eq!(env.mark(), Some(saved(&env, week_ago, &[])), "отметка начала записана, хотя заход не удался");
    let later = now() + Span::hours(3);
    let line = ax.pass(&dead(), later, &AtomicBool::new(false), &fast());
    assert_eq!(line.as_deref(), Some(failed_line("15:00").as_str()));
    assert_eq!(env.mark(), Some(saved(&env, week_ago, &[])), "и не ползёт вперёд, пока заходы не удаются");
}

#[test]
fn a_good_pass_exports_says_so_and_moves_the_mark_in_its_own_file_only() {
    // C# 3: отметка — изменение последней разобранной и номера её минуты; прототип держит её в autoexport-rs.json, а settings.json
    // не трогает (решение «одно приложение за раз»); строка итога — «база знаний: 12:00, выгружено 2»
    let env = Env::new("good-pass", vec![T::mine(1).at(t0()), T::mine(2).at(t0() + min(1))]);
    let mut ax = env.export();
    let line = env.line(&mut ax, now());
    assert_eq!(plain(&line), "база знаний: 12:00, выгружено 2", "время — по часам захода");
    assert!(env.file(1).is_some() && env.file(2).is_some(), "обе выгружены");
    assert_eq!(env.mark(), Some(saved(&env, t0() + min(1), &[2])), "отметка — на последней, с номерами её минуты");
    assert!(!env.data.join("settings.json").exists(), "settings.json не создан и не правился");
    let mut names: Vec<String> =
        fs::read_dir(env.data.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["autoexport-rs.json", "knowledge"], "в папке данных — только своя отметка и база знаний");
    assert!(env.log().is_empty(), "удачный заход в лог не пишет");
}

#[test]
fn a_second_pass_without_news_asks_only_the_list_and_the_statuses_and_says_so() {
    // C# 4: повторный заход без перемен — ни карточек, ни историй, один список; «новых нет»; «кто я» — раз за запуск (MeAsync)
    let env = Env::new("no-news", vec![T::mine(1).at(t0())]);
    let mut ax = env.export();
    env.line(&mut ax, now());
    let (lists, spent, users, statuses) =
        (env.fake.lists().len(), env.fake.spent(), env.fake.count("/api/user"), env.fake.count("/api/taskstatus"));
    assert_eq!(users, 1);
    let line = env.line(&mut ax, now() + min(5));
    assert_eq!(line, "база знаний: 12:05, новых нет");
    assert_eq!(env.fake.spent(), spent, "ни карточек, ни историй, ни файлов");
    assert_eq!(env.fake.lists().len(), lists + 1, "один список");
    assert_eq!(env.fake.count("/api/taskstatus"), statuses + 1, "справочник статусов — каждым заходом");
    assert_eq!(env.fake.count("/api/user"), 1, "«кто я» — один раз за запуск, как у автообновления");
}

#[test]
fn a_restart_goes_on_from_the_saved_mark_and_only_new_tickets_are_exported() {
    // C# 3 («новый запуск») и решение карточки: отметка в файле переживает перезапуск
    let env = Env::new("restart", vec![T::mine(1).at(t0()), T::mine(2).at(t0() + min(1))]);
    env.line(&mut env.export(), now());
    let cards = env.fake.spent();
    let mut restarted = env.export();
    let line = env.line(&mut restarted, now() + min(10));
    assert_eq!(env.since(), "2026-11-25 10:01", "список — с минуты отметки, не с недели назад");
    assert_eq!(line, "база знаний: 12:10, новых нет");
    assert_eq!(env.fake.spent(), cards, "уже выгруженные не запрашиваются");
    env.fake.put(T::mine(3).at(t0() + min(2)));
    let line = env.line(&mut restarted, now() + min(15));
    assert_eq!(plain(&line), "база знаний: 12:15, выгружено 1");
    assert_eq!(env.mark(), Some(saved(&env, t0() + min(2), &[3])));
}

/// Отметка, записанная раньше, чужая в одном из трёх: папка, учётная запись, охват; заход с ней.
fn pass_with_old_mark(tag: &str, mark: impl FnOnce(&Env) -> SavedMark, tune: impl FnOnce(&mut Settings)) -> (Env, String) {
    let mut env = Env::new(tag, vec![]);
    let old = mark(&env);
    write_mark(env.data.path(), &old).unwrap();
    tune(&mut env.settings);
    let mut ax = env.export();
    env.line(&mut ax, now());
    let since = env.since();
    (env, since)
}

fn old_mark(env: &Env) -> SavedMark {
    saved(env, at(2026, 11, 25, 10, 0, 30), &[5])
}

#[test]
fn a_mark_of_the_same_folder_account_and_reach_is_kept() {
    let (_env, since) = pass_with_old_mark("mark-kept", old_mark, |_| {});
    assert_eq!(since, MARK_KEPT, "та же папка, та же учётная запись, тот же охват — отметка годна");
}

#[test]
fn a_mark_of_another_folder_is_forgotten_and_the_new_pass_starts_a_week_back_into_the_new_one() {
    // C# 10: другая папка — отметка была про прежнюю: заново, с недели назад, и отметка теперь этой папки
    let other = std::env::temp_dir().join(format!("tb-autoapp-other-folder-{}", std::process::id()));
    let folder = other.display().to_string();
    let (env, since) = pass_with_old_mark("mark-folder", old_mark, |s| s.knowledge_dir = folder.clone());
    assert_eq!(since, WEEK_BACK);
    let kept = env.mark().unwrap();
    assert_eq!((kept.folder.as_str(), kept.since, kept.seen.clone()), (folder.as_str(), at(2026, 11, 19, 12, 0, 0), vec![]));
    let _ = fs::remove_dir_all(other);
}

#[test]
fn a_mark_of_another_account_is_forgotten() {
    let (env, since) = pass_with_old_mark("mark-account", old_mark, |s| s.login = "other".into());
    assert_eq!(since, WEEK_BACK);
    assert_eq!(env.mark().unwrap().account, account_key(&env.settings), "в файле — новая учётная запись");
}

#[test]
fn a_mark_of_a_narrower_reach_is_forgotten_and_a_wider_one_is_kept() {
    // C# `MarkServes` (MirrorCheck 5, 6, 8): шире отметки — заново с недели назад; уже — отметка та же
    let (_env, since) = pass_with_old_mark("mark-wider-reach", old_mark, |s| s.auto_export_reach = ExportReach::Closed);
    assert_eq!(since, WEEK_BACK, "отметка «моих» не годится для «всех закрытых»");
    let closed = |env: &Env| SavedMark { reach: ExportReach::Closed, ..old_mark(env) };
    let (env, since) = pass_with_old_mark("mark-narrower-reach", closed, |_| {});
    assert_eq!(since, MARK_KEPT, "отметка «всех закрытых» годится для «моих»");
    assert_eq!(env.mark().unwrap().reach, ExportReach::Closed, "ничего не сдвинулось — файл не переписан");
    let (_env, since) = pass_with_old_mark(
        "mark-all-reach",
        |env| SavedMark { reach: ExportReach::All, ..old_mark(env) },
        |s| s.auto_export_reach = ExportReach::Closed,
    );
    assert_eq!(since, MARK_KEPT, "«все заявки» годится для «всех закрытых»");
    let (_env, since) = pass_with_old_mark("mark-closed-to-all", closed, |s| s.auto_export_reach = ExportReach::All);
    assert_eq!(since, WEEK_BACK, "«все закрытые» не годится для «всех заявок»");
}

#[test]
fn the_default_folder_travels_with_the_exe_and_a_full_path_with_a_slash_is_the_same_folder() {
    // C# 10ж: папка по умолчанию — в отметке пустая строка; перенесли программу (и файл отметки) — отметка та же; полный путь
    // той же папки, с косой на конце, — та же папка
    let env = Env::new("moved", vec![T::mine(1).at(t0())]);
    env.line(&mut env.export(), now());
    assert_eq!(env.mark().unwrap().folder, "", "папка по умолчанию — пустая строка, как в настройках");
    let moved = Tmp::new("moved-exe");
    fs::copy(env.data.join(MARK_FILE), moved.join(MARK_FILE)).unwrap();
    let mut ax = AutoExport::new(&env.settings, moved.path(), Some(moved.path().to_path_buf()));
    let line = env.line(&mut ax, now() + min(5));
    assert_eq!(env.since(), MARK_KEPT, "перенесли — отметка не сброшена");
    assert_eq!(line, "база знаний: 12:05, новых нет");
    // полным путём с косой на конце — тоже
    let mut env = Env::new("full-path", vec![]);
    write_mark(env.data.path(), &old_mark(&env)).unwrap();
    env.settings.knowledge_dir = format!("{}/", env.data.join("knowledge").display());
    env.line(&mut env.export(), now());
    assert_eq!(env.since(), MARK_KEPT, "полный путь той же папки — отметка не сброшена");
}

#[test]
fn the_folder_is_kept_in_the_mark_as_typed_without_the_edge_spaces() {
    // C# `FolderSetting` = `KnowledgeDir.Trim()` и `KeepMark(…, folder, …)`
    let mut env = Env::new("folder-as-typed", vec![T::mine(1).at(t0())]);
    let typed = format!("{}/моя база", env.data.path().display());
    env.settings.knowledge_dir = format!("  {typed}  ");
    env.line(&mut env.export(), now());
    assert_eq!(env.mark().unwrap().folder, typed);
    assert!(env.file_in(Path::new(&typed), 1).is_some(), "выгружено в эту папку");
}

#[test]
fn a_mark_that_cannot_be_saved_is_logged_and_the_mark_in_memory_stays_right() {
    // решение карточки: write не вышел — «Не удалось сохранить autoexport-rs.json: …» в лог; в памяти верно — до следующего раза
    let env = Env::new("mark-unsaved", vec![T::mine(1).at(t0())]);
    fs::create_dir(env.data.join(MARK_FILE)).unwrap(); // на месте файла папка: подмена не выйдет
    let mut ax = env.export();
    let line = env.line(&mut ax, now());
    assert_eq!(plain(&line), "база знаний: 12:00, выгружено 1", "заход прошёл, сколько бы ни не писалась отметка");
    let log = env.log();
    assert!(!log.is_empty() && log.iter().all(|l| l.starts_with("Не удалось сохранить autoexport-rs.json: ")), "в логе: {log:?}");
    env.fake.put(T::mine(2).at(t0() + min(1)));
    let line = env.line(&mut ax, now() + min(5));
    assert_eq!(env.since(), MARK_KEPT, "отметка в памяти — за первой заявкой, не с недели назад");
    assert_eq!(plain(&line), "база знаний: 12:05, выгружено 1");
    assert_eq!(env.fake.cards(1), 1, "первая не запрошена снова");
}

// =====================================================================================================================
// Кто я и справочник статусов
// =====================================================================================================================

#[test]
fn who_am_i_is_asked_once_per_run_and_asked_again_after_a_failure() {
    // C# `MeAsync`: раз за запуск; не ответил — «не удалось», в лог один раз, и спрашивается снова; сервер дальше не спрошен
    let env = Env::new("who-am-i", vec![T::mine(1).at(t0())]);
    env.fake.switch(|sw| sw.user_fails = 500);
    let mut ax = env.export();
    assert_eq!(env.line(&mut ax, now()), failed_line("12:00"));
    assert_eq!(env.line(&mut ax, now() + min(5)), failed_line("12:05"));
    assert_eq!(
        (env.fake.count("/api/user"), env.fake.count("/api/taskstatus"), env.fake.lists().len()),
        (2, 0, 0),
        "статусы и список — не спрошены"
    );
    let log = env.log();
    assert_eq!(log.len(), 1, "в лог — один раз за два сбойных захода: {log:?}");
    assert!(log[0].starts_with(NOT_DONE_YET) && log[0].contains("500"), "{log:?}");
    env.fake.switch(|sw| sw.user_fails = 0);
    assert_eq!(plain(&env.line(&mut ax, now() + min(10))), "база знаний: 12:10, выгружено 1");
    env.line(&mut ax, now() + min(15));
    assert_eq!(env.fake.count("/api/user"), 3, "ответил — запомнен: дальше не спрашивается");
}

#[test]
fn an_empty_status_directory_and_a_directory_without_closed_ones_fail_with_the_words_of_csharp() {
    // C# AutoExportAsync: две разные беды — первая на сервере, вторую чинит сам пользователь
    let env = Env::new("statuses", vec![T::mine(1).at(t0())]);
    let mut ax = env.export();
    env.fake.switch(|sw| sw.statuses = Some("[]"));
    assert_eq!(env.line(&mut ax, now()), failed_line("12:00"));
    assert_eq!(env.log(), ["Автовыгрузка: сервер не вернул ни одного статуса заявок"]);
    env.fake.switch(|sw| sw.statuses = Some(r#"[{"Id":31,"Name":"Открыта"},{"Id":32,"Name":"В работе"}]"#));
    assert_eq!(env.line(&mut ax, now() + min(5)), failed_line("12:05"));
    assert_eq!(
        env.log().last().unwrap(),
        "Автовыгрузка: ни один статус не считается закрытым — проверьте ClosedStatusNames в settings.json"
    );
    assert_eq!(env.fake.lists().len(), 0, "список не спрошен ни там, ни там");
}

#[test]
fn the_reach_of_all_tickets_asks_no_status_directory() {
    // C# `reach != ExportReach.All` — «всем заявкам» справочник ни к чему, хоть он и пуст
    let mut env = Env::new("all-no-statuses", vec![T::mine(1).at(t0()).status(WORKING)]);
    env.settings.auto_export_reach = ExportReach::All;
    env.fake.switch(|sw| sw.statuses = Some("[]"));
    let line = env.line(&mut env.export(), now());
    assert_eq!(plain(&line), "база знаний: 12:00, выгружено 1");
    assert_eq!(env.fake.count("/api/taskstatus"), 0);
}

// =====================================================================================================================
// Ошибки: строка трея и лог (C# 7а, 10г)
// =====================================================================================================================

#[test]
fn a_failing_list_is_logged_once_per_distinct_text_and_again_after_a_success() {
    // C# 7а: сбой списка — отметка стоит, в лог один раз за два сбойных захода; `_autoExportError` сбрасывается удачным заходом
    let env = Env::new("list-fails", vec![T::mine(1).at(t0())]);
    let mut ax = env.export();
    env.line(&mut ax, now());
    let mark = env.mark();
    env.fake.switch(|sw| sw.fail_list = 503);
    assert_eq!(env.line(&mut ax, now() + min(5)), failed_line("12:05"));
    assert_eq!(env.line(&mut ax, now() + min(10)), failed_line("12:10"));
    assert_eq!(env.mark(), mark, "отметка стоит");
    let log = env.log();
    assert_eq!(log.len(), 1, "в лог — один раз за два сбойных захода: {log:?}");
    assert!(log[0].starts_with(NOT_DONE_YET) && log[0].contains("503"), "{log:?}");
    env.fake.switch(|sw| sw.fail_list = 500);
    env.line(&mut ax, now() + min(15));
    assert_eq!(env.log().len(), 2, "другой текст ошибки — в лог снова");
    env.fake.switch(|sw| sw.fail_list = 0);
    assert_eq!(env.line(&mut ax, now() + min(20)), "база знаний: 12:20, новых нет");
    env.fake.switch(|sw| sw.fail_list = 503);
    env.line(&mut ax, now() + min(25));
    assert_eq!(env.log().len(), 3, "тот же текст после удачного захода — снова: сбой начался заново");
}

#[test]
fn a_folder_that_cannot_be_made_fails_before_any_request_logs_once_and_heals() {
    // C# 10г: папку не создать (на её месте файл — диска нет) — заход не удался, а не упал; сервер не спрошен вовсе; в лог один
    // раз; папка снова есть — заходы проходят, с недели назад
    let mut env = Env::new("no-folder", vec![T::mine(1).at(t0())]);
    let blocker = env.data.join("blocker.txt");
    fs::write(&blocker, "").unwrap();
    env.settings.knowledge_dir = format!("{}/kb", blocker.display());
    let mut ax = env.export();
    assert_eq!(env.line(&mut ax, now()), failed_line("12:00"));
    assert_eq!(env.line(&mut ax, now() + min(5)), failed_line("12:05"));
    assert_eq!(env.requests(), 0, "сервер не спрошен: ни «кто я», ни статусов, ни списка");
    let log = env.log();
    assert_eq!(log.len(), 1, "в лог — один раз: {log:?}");
    assert!(log[0].starts_with(NOT_DONE_YET), "{log:?}");
    assert_eq!(env.mark().map(|m| m.since), Some(at(2026, 11, 19, 12, 0, 0)), "отметка начала записана, хотя заход не дошёл до сервера");
    fs::remove_file(&blocker).unwrap();
    let line = env.line(&mut ax, now() + min(10));
    assert_eq!(plain(&line), "база знаний: 12:10, выгружено 1");
    assert_eq!(env.since(), WEEK_BACK, "с недели назад: отметка за это время не уползла");
}

// =====================================================================================================================
// .gitignore папки выгрузки (C# 10, 10б, 10в, 10д, 10е)
// =====================================================================================================================

/// Заход в указанную папку (она не по умолчанию): `(env, ax)`, заход уже прошёл.
fn pass_into(tag: &str, folder: &Path) -> (Env, AutoExport, String) {
    let mut env = Env::new(tag, vec![T::mine(1).at(t0())]);
    env.settings.knowledge_dir = folder.display().to_string();
    let mut ax = env.export();
    let line = env.line(&mut ax, now());
    (env, ax, line)
}

#[test]
fn a_folder_that_is_not_a_repository_gets_no_gitignore() {
    let plain_dir = Tmp::new("git-plain");
    let (env, _ax, line) = pass_into("git-plain-env", plain_dir.path());
    assert_eq!(plain(&line), "база знаний: 12:00, выгружено 1");
    assert!(!plain_dir.join(".gitignore").exists(), ".gitignore не заведён");
    assert!(env.file_in(plain_dir.path(), 1).is_some());
}

#[test]
fn a_folder_inside_a_repository_gets_the_tmp_line_in_its_own_gitignore_once() {
    // C# 10б: корень — хранилище Obsidian; *.tmp — в .gitignore самой папки выгрузки, корень не тронут; второй раз не дописывается;
    // удалили — следующий заход пишет снова
    let vault = Tmp::new("git-vault");
    fs::create_dir_all(vault.join(".git")).unwrap();
    let inner = vault.join("Заявки");
    fs::create_dir_all(&inner).unwrap();
    let (env, mut ax, _) = pass_into("git-vault-env", &inner);
    assert_eq!(fs::read_to_string(inner.join(".gitignore")).unwrap(), "*.tmp\n");
    assert!(!vault.join(".gitignore").exists(), "корень репозитория не тронут");
    env.line(&mut ax, now() + min(5));
    assert_eq!(fs::read_to_string(inner.join(".gitignore")).unwrap(), "*.tmp\n", "второй заход строку не дописывает");
    fs::remove_file(inner.join(".gitignore")).unwrap();
    env.line(&mut ax, now() + min(10));
    assert_eq!(fs::read_to_string(inner.join(".gitignore")).unwrap(), "*.tmp\n", "удалили — заход проверяет каждый раз");
}

#[test]
fn a_worktree_where_git_is_a_file_gets_the_tmp_line_too() {
    // C# 10в: .git — файл, а не папка (рабочее дерево, подмодуль)
    let tree = Tmp::new("git-worktree");
    fs::write(tree.join(".git"), "gitdir: ../vault/.git/worktrees/kb\n").unwrap();
    pass_into("git-worktree-env", tree.path());
    assert_eq!(fs::read_to_string(tree.join(".gitignore")).unwrap(), "*.tmp\n");
}

#[test]
fn a_folder_that_does_not_exist_yet_is_made_with_its_gitignore_and_the_log_stays_empty() {
    // C# 10д: новая подпапка хранилища: создаётся, и *.tmp в её .gitignore — тем же заходом
    let vault = Tmp::new("git-fresh");
    fs::create_dir_all(vault.join(".git")).unwrap();
    let fresh = vault.join("Новая");
    let (env, _ax, line) = pass_into("git-fresh-env", &fresh);
    assert_eq!(plain(&line), "база знаний: 12:00, выгружено 1");
    assert_eq!(fs::read_to_string(fresh.join(".gitignore")).unwrap(), "*.tmp\n");
    assert!(env.log().is_empty(), "{:?}", env.log());
}

#[test]
fn a_gitignore_that_cannot_be_written_is_logged_once_per_reason_and_the_passes_go_on() {
    // C# 10е: на месте .gitignore — папка: заходы проходят; в лог по разу на причину, а не каждым заходом; починили — строка
    // дописана; сломалось так же — не снова
    let vault = Tmp::new("git-stuck");
    fs::create_dir_all(vault.join(".git")).unwrap();
    let stuck = vault.join("Закрытая");
    fs::create_dir_all(stuck.join(".gitignore")).unwrap();
    let (env, mut ax, line) = pass_into("git-stuck-env", &stuck);
    assert_eq!(plain(&line), "база знаний: 12:00, выгружено 1", ".gitignore не дописать — заход всё равно прошёл");
    assert_eq!(plain(&env.line(&mut ax, now() + min(5))), "база знаний: 12:05, новых нет");
    let ours = |env: &Env| env.log().into_iter().filter(|l| l.contains("не удалось дописать *.tmp")).collect::<Vec<_>>();
    let lines = ours(&env);
    assert_eq!(lines.len(), 1, "в лог — один раз за два захода: {lines:?}");
    assert!(
        lines[0].starts_with("Автовыгрузка: не удалось дописать *.tmp")
            && lines[0].ends_with(". Пробую каждым заходом; так и не выйдет — допишите строку *.tmp в этот .gitignore сами."),
        "{lines:?}"
    );
    fs::remove_dir(stuck.join(".gitignore")).unwrap();
    env.line(&mut ax, now() + min(10));
    assert!(fs::read_to_string(stuck.join(".gitignore")).unwrap().contains("*.tmp"), "починили — строка дописана");
    fs::remove_file(stuck.join(".gitignore")).unwrap();
    fs::create_dir(stuck.join(".gitignore")).unwrap();
    env.line(&mut ax, now() + min(15));
    assert_eq!(ours(&env).len(), 1, "сломалось так же — не снова: эта строка в логе уже есть");
}

// =====================================================================================================================
// Строка итога: сколько длился заход, пропущенные, порции (C# 7д, MirrorCheck 10)
// =====================================================================================================================

#[test]
fn the_state_line_says_how_long_a_slow_pass_took() {
    // MirrorCheck 10: заход дольше секунды — сколько длился, в подсказке трея («выгружено 1 за N с»); быстрый — без «за»
    let env = Env::new("slow", vec![T::mine(1).at(t0())]);
    env.fake.switch(|sw| sw.card_delay = Duration::from_millis(1100));
    let line = env.line(&mut env.export(), now());
    let rest = line.strip_prefix("база знаний: 12:00, выгружено 1 за ").unwrap_or_else(|| panic!("нет «за N с»: {line}"));
    let seconds: u64 = rest.strip_suffix(" с").and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("после «за» — секунды: {line}"));
    assert!(seconds >= 1, "{line}");
}

/// Заход, что встал в очередь писать за другой выгрузкой (окно «Поиск заявок»), и ждал её 1,5 с; `(итог, журнал запросов)`.
fn pass_behind_another_export() -> (Option<String>, Vec<String>, Env) {
    let env = Env::new("behind-turn", vec![T::mine(846).at(t0() + min(1))]);
    let other = Tmp::new("behind-turn-other");
    let turn = Turn::private(); // одна очередь на обоих
    let options = Options { turn: turn.clone(), ..fast() };
    env.fake.put(T::mine(9000).at(t0()).status(WORKING));
    env.fake.switch(|sw| sw.hold_card = 9000);
    let (client, row, dir, held) = (env.fake.client(), env.fake.found(9000), other.path().to_path_buf(), options.clone());
    let holder = thread::spawn(move || {
        let context =
            Context { me: Some(User { id: ME_ID, name: ME.into() }), closed: ClosedNames::default(), attachments: AttachmentScope::None };
        export_rows(&client, &[row], &dir, &|id| format!("https://hd/Task/View/{id}"), &context, &|_| {}, &AtomicBool::new(false), &held);
    });
    wait_until("карточка держащей очередь выгрузки", || env.fake.cards(9000) > 0);
    let (client, settings, data) = (env.fake.client(), env.settings.clone(), env.data.path().to_path_buf());
    let pass = thread::spawn(move || {
        let mut ax = AutoExport::new(&settings, &data, Some(data.clone()));
        ax.pass(&client, now(), &AtomicBool::new(false), &options)
    });
    wait_until("список захода", || !env.fake.lists().is_empty());
    // ждать надо столько, сколько и меряем: заход стоит в очереди дольше секунды — без вычета ожидания итог сказал бы «за 1 с»
    let since = Instant::now();
    while since.elapsed() < Duration::from_millis(1500) {
        thread::sleep(Duration::from_millis(5));
    }
    env.fake.release();
    holder.join().unwrap();
    let line = pass.join().unwrap();
    let paths = env.fake.paths();
    (line, paths, env)
}

#[test]
fn a_pass_waits_for_the_search_window_export_and_writes_after_it() {
    // C# 7ж: пока пишет другая выгрузка (окно «Поиск заявок»), автовыгрузка ждёт её, а не пишет рядом: карточка захода — после
    // того, как другая отпущена («#release» в журнале запросов раньше)
    let (line, paths, env) = pass_behind_another_export();
    assert!(line.is_some_and(|l| l.contains("выгружено 1")), "заход дождался и выгрузил");
    let released = paths.iter().position(|p| p == "#release").unwrap();
    let card = paths.iter().position(|p| p.starts_with("/api/task/846?")).expect("карточка захода");
    assert!(card > released, "карточка захода пришла, пока другая выгрузка писала: {paths:?}");
    assert!(env.file(846).is_some());
}

#[test]
fn the_wait_for_another_export_is_not_the_time_of_the_pass_in_the_state_line() {
    // MirrorCheck 10 (вторая половина) и C# 7ж: ожидание очереди — не время захода: ждала 1,5 с, а итог — без «за N с»
    let (line, _paths, _env) = pass_behind_another_export();
    assert_eq!(line.as_deref(), Some("база знаний: 12:00, выгружено 1"));
}

#[test]
fn a_pass_that_is_stopped_while_waiting_for_the_search_window_gives_up_and_says_nothing() {
    // C# 7ж (в том виде, что остаётся: заход останавливает только выход): бросил ждать очередь, ничего не записал, в лог не
    // жалуется на собственную остановку, отметка не двинулась
    let env = Env::new("stopped", vec![T::mine(848).at(t0() + min(1))]);
    let other = Tmp::new("stopped-other");
    let turn = Turn::private();
    let options = Options { turn: turn.clone(), ..fast() };
    env.fake.put(T::mine(9000).at(t0()).status(WORKING));
    env.fake.switch(|sw| sw.hold_card = 9000);
    let (client, row, dir, held) = (env.fake.client(), env.fake.found(9000), other.path().to_path_buf(), options.clone());
    let holder = thread::spawn(move || {
        let context =
            Context { me: Some(User { id: ME_ID, name: ME.into() }), closed: ClosedNames::default(), attachments: AttachmentScope::None };
        export_rows(&client, &[row], &dir, &|id| format!("https://hd/Task/View/{id}"), &context, &|_| {}, &AtomicBool::new(false), &held);
    });
    wait_until("карточка держащей очередь выгрузки", || env.fake.cards(9000) > 0);
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (client, settings, data, stop2, done2) =
        (env.fake.client(), env.settings.clone(), env.data.path().to_path_buf(), stop.clone(), done.clone());
    let pass = thread::spawn(move || {
        let mut ax = AutoExport::new(&settings, &data, Some(data.clone()));
        let line = ax.pass(&client, now(), &stop2, &options);
        done2.store(true, SeqCst);
        line
    });
    wait_until("список захода", || !env.fake.lists().is_empty());
    stop.store(true, SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done.load(SeqCst) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    let finished = done.load(SeqCst);
    env.fake.release(); // как бы ни кончилось — другую отпускаем
    holder.join().unwrap();
    let line = pass.join().unwrap();
    assert!(finished, "заход не бросил ждать очередь, пока другая выгрузка держала её");
    assert_eq!(line, None, "остановленный заход итога не пишет");
    assert!(env.log().is_empty(), "…и в лог на свою остановку не жалуется: {:?}", env.log());
    assert!(env.file(848).is_none() && env.fake.cards(848) == 0, "ничего не записано");
    assert_eq!(env.mark().map(|m| m.since), Some(at(2026, 11, 19, 12, 0, 0)), "отметка — только начальная");
}

#[test]
fn a_ticket_that_never_exports_is_skipped_after_five_passes_and_an_hour_and_the_line_and_log_say_so() {
    // C# 7д: моя, у которой история не отдаётся (404), — после пяти заходов и часа пропущена: в итоге «пропущено 1 (errors.log)»,
    // в логе — номер, сколько заходов, с какого времени, что делать и её ошибка
    let env = Env::new("given-up", vec![T::mine(838).at(t0())]);
    env.fake.edit(838, |t| t.life_fails = 404);
    let mut ax = env.export();
    for i in 0..5 {
        assert_eq!(env.line(&mut ax, now() + min(i)), failed_line(&format!("12:0{i}")), "заход {}", i + 1);
    }
    assert!(env.file(838).is_none());
    let sixth = env.line(&mut ax, now() + min(61));
    assert_eq!(sixth, "база знаний: 13:01, новых нет, пропущено 1 (errors.log)");
    let skipped: Vec<String> = env.log().into_iter().filter(|l| l.contains("#838 пропущена")).collect();
    assert_eq!(skipped.len(), 1, "{:?}", env.log());
    assert!(
        skipped[0].starts_with(
            "Автовыгрузка: заявка #838 пропущена — не далась 5 заходов подряд, с 26.11 12:00. Если она ваша — выгрузите её из окна \
             «Поиск заявок»; изменится — автовыгрузка попробует снова. Последняя ошибка: "
        ) && skipped[0].contains("404"),
        "{skipped:?}"
    );
}

// =====================================================================================================================
// Зеркало: отдых между заходами (MirrorCheck 2, 3, 9)
// =====================================================================================================================

fn mirror(tag: &str, tickets: Vec<T>) -> Env {
    let mut env = Env::new(tag, tickets);
    env.settings.auto_export_reach = ExportReach::Closed;
    env
}

#[test]
fn the_mirror_rests_59_minutes_from_the_start_of_the_pass_that_caught_up() {
    // MirrorCheck 2: сразу после догнавшего захода — отдых, ни одного запроса; ровно через 59 минут от его начала — заход, и только
    // список (и справочник); подсказка трея прежняя, пока он отдыхает
    let env = mirror("rest", vec![T::mine(1).at(t0()), T::foreign(2).at(t0() + min(1))]);
    let mut ax = env.export();
    let first = env.line(&mut ax, now());
    assert_eq!(plain(&first), "база знаний: 12:00, выгружено 2", "зеркало берёт и чужую: все закрытые");
    let (asked, spent) = (env.requests(), env.fake.spent());
    assert_eq!(env.pass(&mut ax, now() + min(1)), None, "сразу после — отдыхает");
    assert_eq!(env.pass(&mut ax, now() + min(58) + sec(59)), None, "без секунды час — ещё отдыхает");
    assert_eq!(env.requests(), asked, "отдых — ни одного запроса");
    let lists = env.fake.lists().len();
    let line = env.line(&mut ax, now() + min(59));
    assert_eq!(line, "база знаний: 12:59, новых нет", "ровно час от начала захода — заход");
    assert_eq!(env.fake.lists().len(), lists + 1);
    assert_eq!(env.fake.spent(), spent, "ни карточек, ни историй, ни файлов");
}

#[test]
fn the_mine_reach_never_rests() {
    // C# `Due`: мои — каждым заходом автообновления
    let env = Env::new("no-rest-mine", vec![T::mine(1).at(t0())]);
    let mut ax = env.export();
    env.line(&mut ax, now());
    assert_eq!(env.line(&mut ax, now() + sec(1)), "база знаний: 12:00, новых нет");
    assert_eq!(env.line(&mut ax, now() + sec(2)), "база знаний: 12:00, новых нет");
}

#[test]
fn a_mirror_pass_that_failed_does_not_rest_and_the_next_one_that_caught_up_does() {
    // MirrorCheck 9: список не отдался — «не удалось», отдыха нет: следующий заход пробует снова; отдыхает зеркало только после
    // захода, что догнал без ошибок
    let env = mirror("rest-failed", vec![T::foreign(1).at(t0())]);
    let mut ax = env.export();
    env.fake.switch(|sw| sw.fail_list = 503);
    assert_eq!(env.line(&mut ax, now()), failed_line("12:00"));
    let lists = env.fake.lists().len();
    assert_eq!(env.line(&mut ax, now() + sec(1)), failed_line("12:00"), "следующий — сразу, без отдыха");
    assert_eq!(env.fake.lists().len(), lists + 1);
    env.fake.switch(|sw| sw.fail_list = 0);
    assert!(env.line(&mut ax, now() + sec(2)).contains("выгружено 1"), "сервер ожил — заход прошёл");
    let asked = env.requests();
    assert_eq!(env.pass(&mut ax, now() + sec(3)), None, "а после него — отдых");
    assert_eq!(env.requests(), asked);
}

#[test]
fn a_mirror_cut_by_the_portion_says_catching_up_and_does_not_rest() {
    // MirrorCheck 3: порция (30 заявок с запросами) кончилась — «догоняю» и следующий заход сразу; догнал — отдых
    let tickets: Vec<T> = (1..=35).map(|i| T::foreign(i).at(t0() + sec(i as i64))).collect();
    let env = mirror("rest-portion", tickets);
    let mut ax = env.export();
    assert_eq!(plain(&env.line(&mut ax, now())), "база знаний: 12:00, выгружено 30, догоняю");
    assert_eq!(plain(&env.line(&mut ax, now() + sec(1))), "база знаний: 12:00, выгружено 5", "следующий — сразу, без отдыха");
    let asked = env.requests();
    assert_eq!(env.pass(&mut ax, now() + sec(2)), None, "догнал — отдых");
    assert_eq!(env.requests(), asked);
}

#[test]
fn a_restart_forgets_the_rest_of_the_mirror() {
    // C# `_exportRest` — в памяти: программу перезапустили — зеркало заходит сразу
    let env = mirror("rest-restart", vec![T::foreign(1).at(t0())]);
    env.line(&mut env.export(), now());
    let line = env.line(&mut env.export(), now() + min(1));
    assert_eq!(line, "база знаний: 12:01, новых нет");
}

// =====================================================================================================================
// Окно: заход по таймеру → подсказка трея (C# 13: доска не тронута)
// =====================================================================================================================

use common::window::pump_until;

/// Окно с доской в папке: `tickets.json` — `[]`, `settings.json` — автовыгрузка включена (или нет), база знаний — в `out`.
fn window_env(tag: &str, owner: bool, on: bool, fake: &Fake) -> (ticketboard::app::App, Tmp) {
    let dir = Tmp::new(tag);
    fs::write(dir.join("tickets.json"), "[]").unwrap();
    let out = serde_json::to_string(&dir.join("kb").display().to_string()).unwrap();
    fs::write(
        dir.join("settings.json"),
        format!(
            r#"{{"WipLimit": 0, "KnowledgeDir": {out}, "AutoExportClosed": {on}, "AutoSyncMinutes": 5, "IntraserviceBaseUrl": "{}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#,
            fake.url
        ),
    )
    .unwrap();
    let app = ticketboard::app::App::open(dir.path(), owner).unwrap();
    app.use_client(Some(fake.client()));
    (app, dir)
}

#[test]
fn the_tick_runs_the_export_off_the_window_thread_and_the_line_lands_in_the_tray_tip() {
    // решение карточки: заход идёт в потоке, итог возвращается в подсказку трея (`tray_tip`); тик, пока заход идёт, ничего не
    // делает; выключено и доска только для чтения — тик ничего не спрашивает; доска и settings.json не тронуты (C# 13).
    // Сам таймер (15 с, потом период) на Linux без экрана не запустить — вместо него шов `App::auto_export_now`.
    let (_windows, jobs) = common::window::install();
    let changed = chrono::Local::now().fixed_offset() - chrono::Duration::days(2);
    let fake = Fake::new(vec![T::mine(1).at(changed)]);

    // выключено и только для чтения — тик ничего не делает
    let (off, off_dir) = window_env("win-off", true, false, &fake);
    let (read_only, ro_dir) = window_env("win-readonly", false, true, &fake);
    off.auto_export_now();
    read_only.auto_export_now();

    // хозяин: заход держит карточку, пока тик не повторят
    let (app, dir) = window_env("win-owner", true, true, &fake);
    assert_eq!(app.tray_tip(), HEAD, "до захода в подсказке — только заголовок");
    let (board, settings) = (fs::read(dir.join("tickets.json")).unwrap(), fs::read(dir.join("settings.json")).unwrap());
    fake.switch(|sw| sw.hold_card = 1);
    app.auto_export_now();
    wait_until("заход дошёл до карточки", || fake.cards(1) > 0);
    app.auto_export_now(); // заход идёт — тик ничего не делает
    assert_eq!(app.tray_tip(), HEAD, "пока заход идёт — итога ещё нет");
    fake.release();
    pump_until(&jobs, || app.tray_tip().contains("база знаний"));
    let tip = app.tray_tip();
    let line = tip.strip_prefix(&format!("{HEAD}\nбаза знаний: ")).unwrap_or_else(|| panic!("подсказка: {tip}"));
    assert!(plain(line).ends_with(", выгружено 1"), "{tip}");
    assert!(dir.join("kb").exists() && dir.join(MARK_FILE).exists(), "база знаний и своя отметка");
    assert_eq!(fake.lists().len(), 1, "один заход: тик во время захода, выключенная и только читаемая доски — ничего не спрашивали");
    assert_eq!(fs::read(dir.join("tickets.json")).unwrap(), board, "доска не тронута");
    assert_eq!(fs::read(dir.join("settings.json")).unwrap(), settings, "settings.json не тронут");
    assert!(!app.dirty());
    for (what, app, dir) in [("выключена", &off, &off_dir), ("только чтение", &read_only, &ro_dir)] {
        assert_eq!(app.tray_tip(), HEAD, "{what}: подсказка прежняя");
        assert!(!dir.join(MARK_FILE).exists() && !dir.join("kb").exists(), "{what}: ничего не записано");
    }
}
