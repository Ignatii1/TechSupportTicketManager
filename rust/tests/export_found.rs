//! «Выгрузить найденное» (`KnowledgeExport.RunAsync`, `FoundRun`/`FoundNotes`, `TaskQuery.PastEnd`; карточка 4.3-2): настоящий
//! клиент против поддельного сервера на loopback (по потоку на соединение), папки — временные. Сценарии — из самопроверки C#
//! (`KnowledgeExport.SelfCheck.Mass.cs`, там, где зовётся `All(...)`), сервер — тот же `MassServer`: 230 заявок с номерами с
//! 10001, созданных через каждые шесть часов с 20 января 2026, изменённых часом позже. Не порт: сервер у Rust не умеет менять
//! потолок счёта (он — 1000), поэтому сценарии «упёрся в потолок» идут на 1020 заявках, где потолок обрезает список настояще.
//! Примечания и «закончено» сверяет таблица `parity/export-found-notes.tsv`; в ней и в сценариях ниже — ответы кода C#.

mod common;

use common::Reply;
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use ticketboard::export::files::TICKETS_FOLDER;
use ticketboard::export::run::*;
use ticketboard::export::{AttachmentScope, Context};
use ticketboard::intraservice::query::{DateSpan, STABLE_SORT, TaskQuery};
use ticketboard::intraservice::{Client, User, http_code, is_transient};
use ticketboard::rules::ClosedNames;

const COUNT: usize = 230;
/// Что не так с числом заявок — слово в слово, как у `InvalidLimit`.
const INVALID_LIMIT: &str = "«Не больше, заявок» — число заявок или 0, если нужны все";
/// Больше потолка счёта сервера (1000): со счётом по умолчанию список обрезается ровно на нём.
const BIG: usize = 1020;

// ---------- временные папки ----------

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("tb-found-{tag}-{}-{}", std::process::id(), N.fetch_add(1, SeqCst)));
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

/// Все файлы под папкой — пути от неё, по порядку.
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

/// Файлы заявок в tickets (в любых папках), без оглавлений.
fn on_disk(dir: &Path) -> Vec<String> {
    walk(&dir.join(TICKETS_FOLDER)).into_iter().filter(|p| p.ends_with(".md") && !p.ends_with("_index.md")).collect()
}

// ---------- заявки сервера ----------

fn created_of(i: usize) -> chrono::NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(2026, 1, 20).unwrap().and_hms_opt(8, 0, 0).unwrap() + chrono::Duration::hours(6 * i as i64)
}

fn id_of(i: usize) -> i32 {
    10001 + i as i32
}

fn iso(d: chrono::NaiveDateTime) -> String {
    format!("{}+03:00", d.format("%Y-%m-%dT%H:%M:%S"))
}

fn month(i: usize) -> String {
    created_of(i).format("%Y-%m").to_string()
}

fn named(i: usize) -> String {
    format!("{0} — Заявка {0}.md", id_of(i))
}

fn row_json(i: usize) -> String {
    row_json_created(i, created_of(i))
}

/// Строка списка с заданной датой создания (изменена часом позже неё).
fn row_json_created(i: usize, created: chrono::NaiveDateTime) -> String {
    format!(
        r#"{{"Id":{0},"Name":"Заявка {0}","StatusId":29,"Created":"{1}","Changed":"{2}","Creator":"Петрова А.","Executors":"Я Сам","Description":"<p>описание</p>"}}"#,
        id_of(i),
        iso(created),
        iso(created + chrono::Duration::hours(1))
    )
}

fn card_json(i: usize) -> String {
    format!(
        r#"{{"Task":{{"Id":{0},"Name":"Заявка {0}","StatusName":"Выполнена","ServiceName":"Сервис","Type":"Инцидент","Categories":"Кат","ExecutorGroup":"Группа","Creator":"Петрова А.","Created":"{1}","Changed":"{2}"}}}}"#,
        id_of(i),
        iso(created_of(i)),
        iso(created_of(i) + chrono::Duration::hours(1))
    )
}

fn life_json(i: usize) -> String {
    format!(
        r#"{{"TaskLifetimes":[{{"Date":"{}","Editor":"Иванов Иван","EditorId":7,"Comments":"решено {}"}}],"Paginator":{{"Count":1,"Page":1,"PageCount":1}}}}"#,
        iso(created_of(i) + chrono::Duration::hours(2)),
        id_of(i)
    )
}

// ---------- поддельный сервер (`MassServer`) ----------

/// Что делает сервер со списком — как поля `MassServer` у C#.
#[derive(Clone, Default)]
struct Cfg {
    ignore_sort: bool,
    ignore_page: bool,
    list_unauthorized: bool,
    overlap: bool,
    /// Сервер сам режет страницу до 25 и счёта не присылает.
    no_count: bool,
    /// Потолок счёта по умолчанию (0 — считает всё): со счётом за ним список пуст.
    count_cap: usize,
    reject_no_count: bool,
    /// count=false принят, но не понят: ответ как со счётом.
    ignore_no_count: bool,
    fail_from: usize,
    fail_code: u16,
    /// Счёт, который сервер называет (0 — настоящий).
    claim_count: usize,
    /// Порядок по созданию на стыке первых двух страниц не постоянный (без счёта).
    unstable_boundary: bool,
    /// То же в порядке по изменению, свежие сверху: столько заявок на стыке теряется (0 — нет).
    unstable_boundary_changed: usize,
    /// Эта заявка (номер; 0 — нет) стоит в списке дважды, подряд, и в счёт идёт дважды.
    duplicate_id: i32,
    /// После стольких запросов списка в нём появляются `new_count` новых заявок.
    new_after_lists: usize,
    new_count: usize,
    ignore_page_when_changed: bool,
    reject_no_count_when_changed: bool,
    clamp_no_has_next: bool,
    reject_created_sort: u16,
    fail_lists: u16,
    /// Первые столько запросов списка — 503.
    fail_first_lists: usize,
    list_delay: Duration,
    /// Заявки идут парами с одной датой создания (0 и 1, 2 и 3…): порядок по созданию соблюдён, но даты на стыке равны.
    tie_created: bool,
    /// Первый же запрос списка по изменению (второй проход) — отказ этим кодом (0 — нет), следующие принимаются.
    fail_changed_once: u16,
}

type Hook = Arc<dyn Fn(usize) + Send + Sync>;

struct Shared {
    count: usize,
    cfg: Mutex<Cfg>,
    log: Mutex<Vec<String>>,
    lists: Mutex<Vec<String>>,
    cards: AtomicUsize,
    lives: AtomicUsize,
    hook: Mutex<Option<Hook>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    cards_in_flight: AtomicUsize,
    /// «Четыре места»: вторая страница списка ждёт, пока в работе четыре карточки, а карточки — пока вторая страница не вернулась.
    gate: AtomicBool,
    page2_done: AtomicBool,
    changed_failed: AtomicBool,
    /// «Остановить» нажимают, когда сервер получил запрос списка с этим номером.
    stop_at: Mutex<Option<(usize, Arc<AtomicBool>)>>,
}

fn param(path: &str, name: &str) -> Option<usize> {
    let at = path.find(&format!("&{name}=")).or_else(|| path.find(&format!("?{name}=")))?;
    path[at + name.len() + 2..].split('&').next()?.parse().ok()
}

fn failure(code: u16, body: &str) -> Reply {
    Reply::new(code, "application/json; charset=utf-8", body)
}

fn wait_until(limit: Duration, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + limit;
    while !ready() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
}

impl Shared {
    fn list(&self, path: &str) -> Reply {
        let n_list = {
            let mut lists = self.lists.lock().unwrap();
            lists.push(path.to_string());
            lists.len()
        };
        if let Some((at, stop)) = &*self.stop_at.lock().unwrap()
            && n_list == *at
        {
            stop.store(true, SeqCst);
        }
        let cfg = self.cfg.lock().unwrap().clone();
        let page = if cfg.ignore_page || cfg.ignore_page_when_changed && path.contains("sort=Changed") {
            1
        } else {
            param(path, "page").unwrap_or(1)
        };
        let size = if cfg.no_count { 25 } else { param(path, "pagesize").unwrap_or(25) };
        if self.gate.load(SeqCst) && page == 2 {
            wait_until(Duration::from_millis(1500), || self.cards_in_flight.load(SeqCst) >= 4);
            self.page2_done.store(true, SeqCst);
        }
        thread::sleep(cfg.list_delay);
        if cfg.list_unauthorized {
            return failure(401, r#"{"Message":"Authorization has been denied"}"#);
        }
        if cfg.fail_lists > 0 {
            return failure(cfg.fail_lists, r#"{"Message":"bad request"}"#);
        }
        if n_list <= cfg.fail_first_lists {
            return failure(503, r#"{"Message":"busy"}"#);
        }
        if cfg.fail_changed_once > 0 && path.contains("sort=Changed") && !self.changed_failed.swap(true, SeqCst) {
            return failure(cfg.fail_changed_once, r#"{"Message":"bad request"}"#);
        }
        if cfg.reject_created_sort > 0 && path.contains("sort=Created") {
            return failure(cfg.reject_created_sort, r#"{"Message":"Invalid sort field"}"#);
        }
        let bad_count = r#"{"errors":{"count":["The value 'false' is not valid."]},"status":400}"#;
        if cfg.reject_no_count_when_changed && path.contains("count=false") && path.contains("sort=Changed") {
            return failure(400, bad_count);
        }
        if cfg.reject_no_count && path.contains("count=false") {
            return failure(400, bad_count);
        }
        if cfg.fail_from > 0 && page >= cfg.fail_from {
            return failure(cfg.fail_code, r#"{"Message":"page refused"}"#);
        }
        // по возрастанию — по созданию или по изменению: здесь одно идёт за другим
        let ascending = !cfg.ignore_sort && (path.contains("sort=Created%20asc") || path.contains("sort=Changed%20asc"));
        let counted = cfg.ignore_no_count || !path.contains("count=false");
        let all = self.count + if cfg.new_count > 0 && n_list > cfg.new_after_lists { cfg.new_count } else { 0 };
        let mut ordered: Vec<usize> = if ascending { (0..all).collect() } else { (0..all).rev().collect() };
        if let Some(at) = ordered.iter().position(|&i| cfg.duplicate_id > 0 && i as i32 == cfg.duplicate_id - 10001) {
            ordered.insert(at + 1, ordered[at]);
        }
        let listed = if counted && cfg.count_cap > 0 { ordered.len().min(cfg.count_cap) } else { ordered.len() };
        let order = &ordered[..listed];
        // без HasNextPage за концом списка сервер отдаёт снова последнюю страницу
        let page = if cfg.clamp_no_has_next && !counted { page.min(listed.div_ceil(size).max(1)) } else { page };
        // overlap: список «сдвинулся» — каждая следующая страница начинается с последней заявки предыдущей
        let skip = (page - 1) * size - usize::from(cfg.overlap && page > 1);
        let mut picked: Vec<usize> = order.iter().skip(skip).take(size).copied().collect();
        let lost = if cfg.unstable_boundary && ascending {
            1
        } else if cfg.unstable_boundary_changed > 0 && !ascending {
            cfg.unstable_boundary_changed
        } else {
            0
        };
        if lost > 0 && !counted && page == 2 {
            for i in 0..lost.min(picked.len()) {
                picked[i] = order[skip - lost + i];
            }
        }
        let rows: Vec<String> =
            picked.iter().map(|&i| if cfg.tie_created { row_json_created(i, created_of(i & !1)) } else { row_json(i) }).collect();
        let paginator = if cfg.no_count {
            String::new()
        } else if counted {
            format!(
                r#","Paginator":{{"Count":{},"Page":{page},"PageCount":{},"PageSize":{size},"CountOnPage":{}}}"#,
                if cfg.claim_count > 0 { cfg.claim_count } else { listed },
                listed.div_ceil(size),
                rows.len()
            )
        } else if cfg.clamp_no_has_next {
            format!(r#","Paginator":{{"Page":{page},"PageSize":{size},"CountOnPage":{}}}"#, rows.len())
        } else {
            format!(
                r#","Paginator":{{"Page":{page},"PageSize":{size},"CountOnPage":{},"HasNextPage":{}}}"#,
                rows.len(),
                skip + rows.len() < listed
            )
        };
        Reply::json(&format!(r#"{{"Tasks":[{}],"Statuses":[{{"Id":29,"Name":"Выполнена"}}]{paginator}}}"#, rows.join(",")))
    }

    fn reply(&self, path: &str) -> Reply {
        let extra = self.cfg.lock().unwrap().new_count;
        if path.starts_with("/api/task?") {
            return self.list(path);
        }
        if path.starts_with("/api/taskstatus") {
            return Reply::json(r#"[{"Id":29,"Name":"Выполнена","IsFixed":true}]"#);
        }
        let known = |id: usize| id >= 10001 && id - 10001 < self.count + extra;
        if let Some(rest) = path.strip_prefix("/api/task/") {
            let id: usize = rest.split('?').next().unwrap_or("").parse().unwrap_or(0);
            let seq = self.cards.fetch_add(1, SeqCst) + 1;
            let hook = self.hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(seq);
            }
            if self.gate.load(SeqCst) {
                wait_until(Duration::from_secs(3), || self.page2_done.load(SeqCst));
            }
            return if known(id) { Reply::json(&card_json(id - 10001)) } else { failure(404, "{}") };
        }
        if path.starts_with("/api/tasklifetime?") {
            let id = param(path, "taskid").unwrap_or(0);
            self.lives.fetch_add(1, SeqCst);
            return if known(id) { Reply::json(&life_json(id - 10001)) } else { failure(404, "{}") };
        }
        failure(404, "{}")
    }
}

fn serve_one(mut stream: TcpStream, shared: &Shared) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head.split(' ').nth(1).unwrap_or("").to_string();
    shared.log.lock().unwrap().push(path.clone());
    let now = shared.in_flight.fetch_add(1, SeqCst) + 1;
    shared.max_in_flight.fetch_max(now, SeqCst);
    let is_card = path.starts_with("/api/task/");
    if is_card {
        shared.cards_in_flight.fetch_add(1, SeqCst);
    }
    let reply = shared.reply(&path);
    if is_card {
        shared.cards_in_flight.fetch_sub(1, SeqCst);
    }
    shared.in_flight.fetch_sub(1, SeqCst);
    let reason = if reply.code == 200 { "OK" } else { "Error" };
    let _ = write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.code,
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(&reply.body);
}

struct Fake {
    shared: Arc<Shared>,
    url: String,
}

impl Fake {
    fn new(count: usize) -> Fake {
        let shared = Arc::new(Shared {
            count,
            cfg: Mutex::new(Cfg::default()),
            log: Mutex::new(Vec::new()),
            lists: Mutex::new(Vec::new()),
            cards: AtomicUsize::new(0),
            lives: AtomicUsize::new(0),
            hook: Mutex::new(None),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            cards_in_flight: AtomicUsize::new(0),
            gate: AtomicBool::new(false),
            page2_done: AtomicBool::new(false),
            changed_failed: AtomicBool::new(false),
            stop_at: Mutex::new(None),
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
    fn with(count: usize, set: impl FnOnce(&mut Cfg)) -> Fake {
        let fake = Fake::new(count);
        fake.set(set);
        fake
    }
    fn set(&self, f: impl FnOnce(&mut Cfg)) {
        f(&mut self.shared.cfg.lock().unwrap());
    }
    fn client(&self) -> Client {
        Client::new(&self.url, "u", "p", Duration::from_secs(10))
    }
    fn on_card(&self, f: impl Fn(usize) + Send + Sync + 'static) {
        *self.shared.hook.lock().unwrap() = Some(Arc::new(f));
    }
    /// Забыть запросы, счёт карточек и крючок — для второго захода по той же папке.
    fn reset(&self) {
        self.shared.log.lock().unwrap().clear();
        self.shared.lists.lock().unwrap().clear();
        self.shared.cards.store(0, SeqCst);
        self.shared.lives.store(0, SeqCst);
        *self.shared.hook.lock().unwrap() = None;
    }
    /// Адреса запросов списка по порядку прихода.
    fn lists(&self) -> Vec<String> {
        self.shared.lists.lock().unwrap().clone()
    }
    fn paths(&self) -> Vec<String> {
        self.shared.log.lock().unwrap().clone()
    }
    fn cards(&self) -> usize {
        self.shared.cards.load(SeqCst)
    }
    fn lives(&self) -> usize {
        self.shared.lives.load(SeqCst)
    }
}

// ---------- запуск ----------

fn context() -> Context {
    Context { me: Some(User { id: 7, name: "Я Сам".into() }), closed: ClosedNames::default(), attachments: AttachmentScope::None }
}

fn fast() -> Options {
    Options {
        retry_delays: vec![Duration::from_millis(1); 3],
        vanish_wait: Duration::ZERO,
        list_timeout: Duration::from_secs(30),
        ..Options::default()
    }
}

fn url_of(id: i32) -> String {
    format!("https://hd/Task/View/{id}")
}

/// Ход выгрузки как есть, по порядку прихода (сообщают из пула — под замком).
#[derive(Default)]
struct Said(Mutex<Vec<String>>);

impl Said {
    fn put(&self, line: String) {
        self.0.lock().unwrap().push(line);
    }
    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    /// Последняя строка хода — до сборки таблицы («Собираю таблицу заявок…» идёт за ней).
    fn last_count(&self) -> Option<String> {
        self.all().into_iter().rev().find(|l| !l.starts_with("Собираю таблицу"))
    }
}

fn export(fake: &Fake, query: &TaskQuery, limit: i32, dir: &Path, stop: &AtomicBool, options: &Options, said: &Said) -> ExportResult {
    export_found(&fake.client(), query, limit, dir, &url_of, &context(), &|s| said.put(s), stop, options)
}

/// Выгрузка «все» (limit 0) без отбора.
fn all(fake: &Fake, dir: &Path) -> ExportResult {
    export(fake, &TaskQuery::default(), 0, dir, &AtomicBool::new(false), &fast(), &Said::default())
}

fn limited(fake: &Fake, limit: i32, dir: &Path) -> ExportResult {
    export(fake, &TaskQuery::default(), limit, dir, &AtomicBool::new(false), &fast(), &Said::default())
}

/// Свой сервер, одна выгрузка (по умолчанию «все») и всё, что о ней видно.
struct Done {
    fake: Fake,
    tmp: Tmp,
    r: ExportResult,
    said: Said,
}

fn scenario(tag: &str, count: usize, limit: i32, set: impl FnOnce(&mut Cfg)) -> Done {
    let (fake, tmp, said) = (Fake::with(count, set), Tmp::new(tag), Said::default());
    let r = export(&fake, &TaskQuery::default(), limit, tmp.path(), &AtomicBool::new(false), &fast(), &said);
    Done { fake, tmp, r, said }
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

impl Done {
    fn says(&self, part: &str) -> bool {
        self.r.error.contains(part)
    }
    fn lists(&self) -> Vec<String> {
        self.fake.lists()
    }
    fn shown(&self) -> String {
        format!("{}\nсписки: {}", show(&self.r), self.lists().join(" | "))
    }
}

// ---------- примечания и «закончено»: таблица C# ----------

/// Вход строки таблицы «ключ=значение;…» — как его читает `ParityCheck.ExportFoundNotes`: нет ключа — 0, ложь, пусто;
/// «где» и «чьё» по умолчанию — слова про дату.
fn found_run_of(input: &str) -> FoundRun {
    let v: std::collections::HashMap<String, String> = input
        .split(';')
        .filter(|kv| !kv.is_empty())
        .map(|kv| match kv.split_once('=') {
            Some((k, x)) => (k.to_string(), x.to_string()),
            None => (kv.to_string(), String::new()),
        })
        .collect();
    let n = |k: &str| v.get(k).map(|x| x.parse::<i32>().unwrap()).unwrap_or(0);
    let b = |k: &str| v.get(k).is_some_and(|x| x == "1");
    let s = |k: &str, none: &str| v.get(k).cloned().unwrap_or_else(|| none.to_string());
    FoundRun {
        total: n("total"),
        seen: n("seen"),
        first_seen: n("firstSeen"),
        list_error: s("listError", ""),
        stuck: b("stuck"),
        second_pass: b("secondPass"),
        second_error: s("secondError", ""),
        second_stuck: b("secondStuck"),
        cancelled: b("cancelled"),
        twice: v.contains_key("twice").then(|| n("twice")),
        recovered: n("recovered"),
        repeated: n("repeated"),
        overflow: n("overflow"),
        second_long: n("secondLong"),
        limit: n("limit"),
        outside: n("outside"),
        outside_where: s("where", "вне выбранного периода"),
        outside_which: s("which", "условие по дате"),
        counting_worked: b("countingWorked"),
        fallback_worked: b("fallbackWorked"),
        unsorted: b("unsorted"),
        saw_has_next: b("sawHasNext"),
        reached_end: b("reachedEnd"),
        hit_limit: b("hitLimit"),
        fatal: s("fatal", ""),
    }
}

#[test]
fn the_notes_and_the_complete_flag_are_those_of_csharp_on_every_row_of_the_table() {
    let wrong = common::parity_mismatches("export-found-notes", |inputs| {
        let (notes, complete) = found_notes(&found_run_of(&inputs[0]));
        format!("{}\n|complete={}", notes.join("\n"), u8::from(complete))
    });
    assert!(wrong.is_empty(), "расходятся {} строк:\n{}", wrong.len(), wrong.join("\n"));
}

// ---------- постоянные ----------

#[test]
fn the_list_page_is_two_hundred_and_the_list_has_sixty_seconds() {
    assert_eq!(LIST_PAGE_SIZE, 200);
    let options = Options::default();
    assert_eq!(options.list_timeout, Duration::from_secs(60), "срок страницы списка");
    // прежние постоянные не тронуты
    assert_eq!(options.retry_delays, [Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(15)]);
    assert_eq!(options.vanish_wait, Duration::from_secs(3));
}

#[test]
fn a_negative_limit_is_not_a_number_of_tickets_but_zero_and_more_are() {
    assert_eq!(invalid_limit(-1), Some(INVALID_LIMIT));
    assert_eq!(invalid_limit(i32::MIN), Some(INVALID_LIMIT));
    assert_eq!(invalid_limit(0), None, "0 — без ограничения");
    assert_eq!(invalid_limit(1), None);
    assert_eq!(invalid_limit(500), None);
}

#[test]
fn a_wrong_limit_stops_at_once_with_its_own_words_and_touches_nothing() {
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("badlimit"));
    let dir = tmp.join("база");
    let r = export(&fake, &TaskQuery::default(), -1, &dir, &AtomicBool::new(false), &fast(), &Said::default());
    assert_eq!((r.complete, r.found, r.created, r.error.as_str()), (false, 0, 0, INVALID_LIMIT), "{}", show(&r));
    assert!(fake.paths().is_empty(), "неверный ввод — ни одного запроса: {:?}", fake.paths());
    assert!(!dir.exists(), "папка не создана");
}

#[test]
fn a_stop_before_the_start_writes_nothing_and_asks_nothing() {
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("prestop"));
    let dir = tmp.join("база");
    let r = export(&fake, &TaskQuery::default(), 0, &dir, &AtomicBool::new(true), &fast(), &Said::default());
    assert_eq!((r.complete, r.found, r.error.as_str()), (false, 0, "Остановлено до начала — ничего не записано."), "{}", show(&r));
    assert!(fake.paths().is_empty() && !dir.exists(), "запросы {:?}", fake.paths());
}

// ---------- страница списка у клиента ----------

fn empty_page() -> &'static str {
    r#"{"Tasks":[],"Paginator":{"Count":0,"Page":1,"PageCount":0,"PageSize":200,"CountOnPage":0}}"#
}

fn client_at(url: &str) -> Client {
    Client::new(url, "u", "p", Duration::from_secs(10))
}

#[test]
fn a_list_page_is_two_hundred_rows_with_service_names_and_a_count_only_when_asked() {
    let (url, seen) = common::serve(200, "application/json", empty_page(), Duration::ZERO);
    let client = client_at(&url);
    let first = client.get_task_list(&TaskQuery::default(), 1, true, Duration::from_secs(10));
    assert!(first.is_ok(), "первая страница: {first:?}");
    let stable = TaskQuery { sort: STABLE_SORT.into(), executor_ids: vec![7], ..TaskQuery::default() };
    let third = client.get_task_list(&stable, 3, false, Duration::from_secs(10));
    assert!(third.is_ok(), "третья страница: {third:?}");
    let paths: Vec<String> = seen.lock().unwrap().iter().map(|h| h.split(' ').nth(1).unwrap().to_string()).collect();
    assert_eq!(
        paths,
        [
            "/api/task?include=status,service&sort=Changed%20desc&pagesize=200&page=1",
            "/api/task?ExecutorIds=7&include=status,service&count=false&sort=Created%20asc,%20Id%20asc&pagesize=200&page=3"
        ]
    );
}

#[test]
fn a_list_page_brings_the_rows_the_total_and_whether_there_is_a_next_page() {
    let body = r#"{"Tasks":[{"Id":70001,"Name":"Один","StatusId":29,"Created":"2026-05-01T10:00:00+03:00","Changed":"2026-05-02T10:00:00+03:00"},{"Id":70002,"Name":"Два","StatusId":29}],"Statuses":[{"Id":29,"Name":"Выполнена"}],"Paginator":{"Count":231,"Page":1,"PageCount":2,"PageSize":200,"CountOnPage":2,"HasNextPage":true}}"#;
    let (url, _) = common::serve(200, "application/json", body, Duration::ZERO);
    let got = client_at(&url).get_task_list(&TaskQuery::default(), 1, true, Duration::from_secs(10));
    assert!(got.is_ok(), "страница: {got:?}");
    let page = got.unwrap();
    assert_eq!(page.found.iter().map(|f| f.id).collect::<Vec<_>>(), [70001, 70002]);
    assert_eq!((page.found[0].name.as_str(), page.found[0].status.as_str()), ("Один", "Выполнена"));
    assert_eq!((page.total, page.has_next), (231, Some(true)));
}

#[test]
fn a_list_page_that_does_not_answer_in_time_is_a_network_failure_that_is_repeated() {
    // сервер молчит дольше срока страницы: это сбой сети (его повторяют), а не «нет такой страницы»
    let (url, _) = common::serve(200, "application/json", empty_page(), Duration::from_secs(5));
    let late = client_at(&url).get_task_list(&TaskQuery::default(), 1, true, Duration::from_millis(100)).err().unwrap_or_default();
    assert!(late.starts_with("сервер не ответил"), "ответ клиента: «{late}»");
    assert!(is_transient(&late), "срок — сбой, который повторяют: {late}");
}

#[test]
fn a_refused_list_page_keeps_its_code_and_only_server_errors_are_transient() {
    let (url, _) = common::serve(500, "application/json", r#"{"Message":"boom"}"#, Duration::ZERO);
    let e500 = client_at(&url).get_task_list(&TaskQuery::default(), 1, true, Duration::from_secs(10)).err().unwrap_or_default();
    assert_eq!((http_code(&e500), is_transient(&e500)), (Some(500), true), "{e500}");
    let (url, _) = common::serve(400, "application/json", r#"{"Message":"bad"}"#, Duration::ZERO);
    let e400 = client_at(&url).get_task_list(&TaskQuery::default(), 2, false, Duration::from_secs(10)).err().unwrap_or_default();
    assert_eq!((http_code(&e400), is_transient(&e400)), (Some(400), false), "{e400}");
}

// ---------- 1–3: всё сразу ----------

#[test]
fn everything_goes_by_pages_in_creation_order_into_files_and_indexes() {
    let d = scenario("all", COUNT, 0, |_| {});
    let r = &d.r;
    assert_eq!((counts(r), r.complete, r.error.as_str()), ((COUNT, COUNT, 0, 0, 0), true, ""), "{}", d.shown());
    // первая страница — со счётом по умолчанию (общее число для хода), дальше — без счёта: count=false и HasNextPage
    let lists = d.lists();
    assert!(
        lists.len() == 2
            && lists[0].contains("sort=Created%20asc,%20Id%20asc&pagesize=200&page=1")
            && !lists[0].contains("count=")
            && lists[1].contains("&count=false&")
            && lists[1].ends_with("&page=2"),
        "{}",
        lists.join(" | ")
    );
    // ход: «из 230», последняя строка до таблицы — «Выгружено 230 из 230», а за ней — сборка таблицы
    let said = d.said.all();
    assert!(said.iter().any(|m| m.contains(&format!("из {COUNT}"))), "{said:?}");
    assert!(said.last().is_some_and(|m| m.starts_with("Собираю таблицу заявок: ")), "{:?}", said.last());
    assert_eq!(d.said.last_count().as_deref(), Some(format!("Выгружено {COUNT} из {COUNT}").as_str()));
    // файлы — в папках месяцев создания, временных нет
    let dir = d.tmp.path();
    assert_eq!(on_disk(dir).len(), COUNT, "{:?}", walk(dir));
    assert!(walk(dir).iter().all(|p| !p.ends_with(".tmp")), "после выгрузки временных файлов нет");
    for i in 0..COUNT {
        assert!(dir.join(TICKETS_FOLDER).join(month(i)).join(named(i)).is_file(), "нет файла {}/{}", month(i), named(i));
    }
    // оглавления: общее — по месяцам, у месяца — ссылки на каждую заявку
    let root = text(dir.join("_index.md"));
    assert!(root.contains(&format!("заявок: {COUNT}.")), "{root}");
    let months: BTreeSet<String> = (0..COUNT).map(month).collect();
    assert!(months.len() >= 3);
    for m in &months {
        let n = (0..COUNT).filter(|&i| month(i) == *m).count();
        assert!(root.contains(&format!("- [{m}](tickets/{m}/_index.md) — заявок: {n}\n")), "{m}: {root}");
        let index = text(dir.join(TICKETS_FOLDER).join(m).join("_index.md"));
        for i in (0..COUNT).filter(|&i| month(i) == *m) {
            assert!(index.contains(&format!("[[{}]]", named(i).trim_end_matches(".md"))), "{m}: нет {}", named(i));
        }
    }
    let first = root.find(&format!("[{}]", months.iter().next_back().unwrap())).unwrap();
    let last = root.find(&format!("[{}]", months.iter().next().unwrap())).unwrap();
    assert!(first < last, "свежие месяцы сверху: {root}");
    let sample = text(dir.join(TICKETS_FOLDER).join(month(4)).join(named(4)));
    assert!(
        sample.starts_with("---\nid: 10005\ntitle: \"Заявка 10005\"\n")
            && sample.contains("\nservice: \"Сервис\"\n")
            && sample.contains("решено 10005"),
        "{sample}"
    );
}

#[test]
fn a_page_is_written_before_the_cards_of_the_next_one_are_asked_and_the_next_page_is_asked_ahead() {
    let d = scenario("pages", COUNT, 0, |_| {});
    assert!(d.r.complete, "{}", show(&d.r));
    let paths = d.fake.paths();
    let ids: Vec<i32> =
        paths.iter().filter_map(|p| p.strip_prefix("/api/task/")).map(|r| r.split('?').next().unwrap().parse().unwrap()).collect();
    assert_eq!(ids.len(), COUNT);
    assert!(
        ids.iter().take(200).all(|&id| id <= id_of(199)) && ids.iter().skip(200).all(|&id| id > id_of(199)),
        "карточки второй страницы просят, когда все 200 заявок первой записаны"
    );
    // а сама вторая страница запрошена заранее, пока шла первая
    let second = paths.iter().position(|p| p.starts_with("/api/task?") && p.ends_with("&page=2")).expect("вторая страница");
    assert!(second < 100, "вторая страница списка — среди первых запросов, а она на месте {second}");
}

#[test]
fn a_stop_in_the_middle_keeps_what_is_written_and_the_next_run_goes_on_from_there() {
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("stop"));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    fake.on_card(move |n| {
        if n == 70 {
            flag.store(true, SeqCst);
        }
    });
    let stopped = export(&fake, &TaskQuery::default(), 0, tmp.path(), &stop, &fast(), &Said::default());
    assert!(!stopped.complete && stopped.failed == 0 && stopped.error.contains("Остановлено"), "{}", show(&stopped));
    assert!(stopped.created > 0 && stopped.created < COUNT, "{}", show(&stopped));
    assert_eq!(stopped.found, stopped.created, "{}", show(&stopped));
    assert_eq!(on_disk(tmp.path()).len(), stopped.created);
    assert!(text(tmp.join("_index.md")).contains(&format!("заявок: {}.", stopped.created)), "оглавление — по записанному");
    // повторный запуск продолжает: карточки читаются только у недостающих
    fake.reset();
    let resumed = all(&fake, tmp.path());
    assert_eq!((resumed.complete, resumed.failed, resumed.updated, resumed.error.as_str()), (true, 0, 0, ""), "{}", show(&resumed));
    assert_eq!(resumed.unchanged, stopped.created, "{}", show(&resumed));
    assert_eq!(resumed.created, COUNT - stopped.created, "{}", show(&resumed));
    assert_eq!(fake.cards(), resumed.created, "карточки — только у недостающих");
    assert_eq!(on_disk(tmp.path()).len(), COUNT);
}

#[test]
fn a_stop_on_the_last_page_is_still_a_stop_and_not_done() {
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("stoplast"));
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    fake.on_card(move |n| {
        if n == 215 {
            flag.store(true, SeqCst);
        }
    });
    let r = export(&fake, &TaskQuery::default(), 0, tmp.path(), &stop, &fast(), &Said::default());
    assert!(!r.complete && r.failed == 0 && r.error.contains("Остановлено"), "{}", show(&r));
    assert!(r.created > 200 && r.created < COUNT, "{}", show(&r));
}

#[test]
fn nothing_changed_reads_only_the_list_and_a_table_deleted_by_hand_is_rebuilt() {
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("same"));
    assert!(all(&fake, tmp.path()).complete);
    fake.reset();
    let same = all(&fake, tmp.path());
    assert_eq!((counts(&same), same.complete), ((COUNT, 0, 0, COUNT, 0), true), "{}", show(&same));
    assert_eq!((fake.cards(), fake.lives(), fake.lists().len()), (0, 0, 2), "только две страницы списка: {:?}", fake.paths().len());
    // таблицу удалили руками: «все» без изменений собирает её заново из файлов
    let table = tmp.join("tickets.csv");
    assert!(table.is_file(), "после выгрузки «все» таблицы нет");
    fs::remove_file(&table).unwrap();
    fake.reset();
    let again = all(&fake, tmp.path());
    assert!(again.complete && again.unchanged == COUNT, "{}", show(&again));
    let rows = text(&table).split("\r\n").filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())).count();
    assert!(table.is_file() && rows == COUNT, "таблица {} строк из {COUNT}", rows);
}

#[test]
fn a_list_page_takes_one_of_the_four_places_a_ticket_request_takes() {
    // вторая страница списка запрошена заранее; сервер держит её, пока в работе не четыре карточки, а карточки ждут её ответа:
    // если страница не занимает место из четырёх, они окажутся все вместе — пять запросов разом
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("places"));
    fake.shared.gate.store(true, SeqCst);
    let r = all(&fake, tmp.path());
    assert!(r.complete && r.created == COUNT, "{}", show(&r));
    let most = fake.shared.max_in_flight.load(SeqCst);
    assert!(most <= 4, "разом шло {most} запросов, а не больше четырёх");
}

// ---------- 10: сортировка и список, который не отдаётся ----------

#[test]
fn a_server_that_refuses_the_creation_sort_gets_the_first_page_again_by_change_date_and_it_is_said() {
    let d = scenario("fallback", COUNT, 0, |c| c.reject_created_sort = 400);
    let r = &d.r;
    assert!(r.complete && r.created == COUNT && r.failed == 0, "{}", d.shown());
    assert!(d.says("Сервер не принял сортировку по дате создания"), "{}", d.shown());
    let lists = d.lists();
    assert!(
        lists.len() == 3 && lists[0].contains("sort=Created") && lists[1].contains("sort=Changed%20desc&pagesize=200&page=1"),
        "{}",
        lists.join(" | ")
    );
}

#[test]
fn a_server_error_on_the_creation_sort_is_repeated_once_and_then_the_same_fallback_is_taken() {
    let d = scenario("fallback500", COUNT, 0, |c| c.reject_created_sort = 500);
    assert!(d.r.complete && d.r.created == COUNT && d.r.failed == 0, "{}", d.shown());
    assert!(d.says("Сервер не принял сортировку по дате создания"), "{}", d.shown());
    let lists = d.lists();
    assert!(
        lists.len() == 4 && lists.iter().filter(|t| t.contains("sort=Created")).count() == 2,
        "500: один повтор — {}",
        lists.join(" | ")
    );
}

#[test]
fn a_list_that_is_not_given_at_all_is_not_blamed_on_the_sort() {
    // 400 на любой запрос: запасной порядок не помог — о сортировке не пишем; ничего не сделано — ответ сервера как есть
    let d = scenario("listfail", COUNT, 0, |c| c.fail_lists = 400);
    assert!(!d.r.complete && d.r.found == 0 && d.r.created == 0, "{}", d.shown());
    assert!(d.says("400") && !d.says("не принял сортировку"), "{}", d.shown());
    // ничего не выгружено, список не пришёл — ответ сервера как есть, без «Список пришёл не целиком» (C# :429)
    assert_eq!(d.r.error, "ошибка сервера (HTTP 400):\n{\"Message\":\"bad request\"}");
    assert_eq!(d.lists().len(), 2, "порядок по созданию и запасной: {}", d.lists().join(" | "));
    assert_eq!(d.fake.cards(), 0);
}

#[test]
fn a_refused_login_on_the_list_is_not_a_reason_to_change_the_sort() {
    let d = scenario("nolist", COUNT, 0, |c| c.list_unauthorized = true);
    assert!(d.r.found == 0 && !d.r.complete && d.says("(HTTP 401)"), "{}", d.shown());
    assert_eq!(d.r.error, "неверный логин или пароль (HTTP 401):\n{\"Message\":\"Authorization has been denied\"}");
    assert_eq!(d.lists().len(), 1, "401 — ошибка сразу: {}", d.lists().join(" | "));
}

#[test]
fn a_list_page_that_fails_once_is_repeated_and_said_so_in_the_progress() {
    let d = scenario("listretry", COUNT, 0, |c| c.fail_first_lists = 1);
    assert!(d.r.complete && d.r.created == COUNT && d.r.error.is_empty(), "{}", d.shown());
    assert_eq!(d.lists().len(), 3, "сбойный запрос, первая страница и вторая: {}", d.lists().join(" | "));
    let said = d.said.all();
    assert!(said.iter().any(|m| m.starts_with("Список: ") && m.ends_with(" — повторяю…")), "{said:?}");
}

#[test]
fn a_list_that_keeps_failing_gives_up_after_the_retries_and_says_what_the_server_said() {
    let d = scenario("listdown", COUNT, 0, |c| c.fail_first_lists = 100);
    assert!(!d.r.complete && d.r.found == 0 && d.says("503"), "{}", d.shown());
    assert_eq!(d.r.error, "ошибка сервера (HTTP 503):\n{\"Message\":\"busy\"}");
    // C# `RunInTurnAsync` (KnowledgeExport.cs:273): отказ первой страницы с кодом от 400 (кроме 401, 403, 408, 429) при
    // «всё» — повод попробовать порядок по изменению; 503 после всех повторов — такой же: два захода по запросу и повторам
    assert_eq!(d.lists().len(), 2 * (1 + fast().retry_delays.len()), "два захода — по созданию и по изменению: {}", d.lists().join(" | "));
    assert!(d.lists().iter().any(|l| l.contains("Changed%20desc")), "второй заход — по изменению: {}", d.lists().join(" | "));
}

#[test]
fn the_list_has_its_own_time_limit_and_a_page_that_is_too_slow_is_a_failure_after_the_retries() {
    let (fake, tmp) = (Fake::with(COUNT, |c| c.list_delay = Duration::from_secs(5)), Tmp::new("slowlist"));
    let options = Options { list_timeout: Duration::from_millis(150), ..fast() };
    let r = export(&fake, &TaskQuery::default(), 0, tmp.path(), &AtomicBool::new(false), &options, &Said::default());
    assert!(!r.complete && r.found == 0 && r.error.contains("сервер не ответил"), "{}", show(&r));
    // запросы пришли все (ждём условие: сервер разбирает их по потоку на соединение)
    wait_until(Duration::from_secs(10), || fake.lists().len() >= 4);
    assert_eq!(fake.lists().len(), 4, "один запрос и три повтора");
}

// ---------- 11: порядок, страницы, счёт ----------

#[test]
fn a_server_that_ignores_the_sort_is_followed_and_the_order_is_called_out() {
    let d = scenario("unsorted", COUNT, 0, |c| c.ignore_sort = true);
    assert!(d.r.complete && d.r.created == COUNT && d.says("не по дате создания"), "{}", d.shown());
}

#[test]
fn a_server_that_does_not_understand_the_page_stops_the_run_without_a_loop() {
    let d = scenario("stuck", COUNT, 0, |c| c.ignore_page = true);
    let r = &d.r;
    assert!(!r.complete && r.found == 200 && r.created == 200 && d.says("одну и ту же страницу"), "{}", d.shown());
    assert_eq!(d.lists().len(), 2, "{}", d.lists().join(" | "));
}

#[test]
fn a_list_that_shifted_while_read_does_not_export_the_boundary_ticket_twice() {
    let d = scenario("shifted", COUNT, 0, |c| c.overlap = true);
    assert!(d.r.complete && d.r.created == COUNT && d.r.failed == 0 && d.r.error.is_empty(), "{}", d.shown());
    assert_eq!(d.fake.cards(), COUNT, "карточка каждой — один раз");
}

#[test]
fn a_server_that_cuts_the_page_to_twenty_five_and_sends_no_count_is_read_to_the_end() {
    let d = scenario("short", COUNT, 0, |c| c.no_count = true);
    assert!(d.r.complete && d.r.created == COUNT && d.r.failed == 0 && d.r.error.is_empty(), "{}", d.shown());
    assert_eq!(d.lists().len(), COUNT.div_ceil(25), "неполная страница конца не доказывает — читаем, пока не пусто");
}

#[test]
fn a_count_that_sits_on_the_ceiling_is_a_lower_bound_the_progress_has_no_total_and_all_is_read_twice() {
    // сервер досчитал только до потолка (1000) и со счётом за ним список не отдаёт: страницы после первой — count=false,
    // ход — без «из», а второй проход (по изменению) идёт всегда: пропуск при сдвиге списка счётом не заметить
    let d = scenario("overcap", BIG, 0, |c| c.count_cap = 1000);
    let r = &d.r;
    assert!(r.complete && r.created == BIG && r.failed == 0 && r.error.is_empty(), "{}", d.shown());
    let lists = d.lists();
    assert!(
        lists.len() == 12 && !lists[0].contains("count=") && lists[1].contains("&count=false&") && lists[6].contains("sort=Changed%20desc"),
        "{}",
        lists.join(" | ")
    );
    let said = d.said.all();
    assert!(said.iter().all(|m| !m.contains(" из ")), "{said:?}");
    assert!(d.said.last_count().is_some_and(|m| m.starts_with(&format!("Выгружено {BIG}"))), "{said:?}");
}

#[test]
fn a_limit_not_above_the_ceiling_is_known_even_when_the_count_is_only_a_lower_bound() {
    let d = scenario("overcap50", BIG, 50, |c| c.count_cap = 1000);
    assert!(d.r.found == 50 && d.r.created == 50 && d.r.complete, "{}", d.shown());
    assert_eq!(d.said.last_count().as_deref(), Some("Выгружено 50 из 50"));
}

#[test]
fn a_server_that_refuses_a_list_without_count_gets_the_same_page_with_it_and_it_is_said() {
    let d = scenario("nocountrefused", COUNT, 0, |c| c.reject_no_count = true);
    assert!(d.r.complete && d.r.created == COUNT && d.r.failed == 0 && d.says("без счёта"), "{}", d.shown());
    let lists = d.lists();
    assert!(lists.len() == 3 && lists[1].contains("count=false") && !lists[2].contains("count="), "{}", lists.join(" | "));
}

#[test]
fn a_refusal_on_the_second_page_that_is_not_about_the_count_is_the_servers_answer_not_the_count() {
    let d = scenario("page2refused", COUNT, 0, |c| (c.fail_from, c.fail_code) = (2, 400));
    assert!(!d.r.complete && d.r.created == 200, "{}", d.shown());
    assert!(d.says("Список пришёл не целиком") && !d.says("без счёта"), "{}", d.shown());
    assert_eq!(d.lists().len(), 3, "вторая страница без счёта и со счётом: {}", d.lists().join(" | "));
}

#[test]
fn a_list_cut_at_the_ceiling_after_a_refused_count_is_not_done_and_the_second_pass_reads_the_other_end() {
    let d = scenario("cutatcap", BIG, 0, |c| (c.reject_no_count, c.count_cap) = (true, 1000));
    assert!(!d.r.complete && d.r.created == BIG && d.r.failed == 0, "{}", d.shown());
    assert!(d.says("только первые 1000") && d.says("дочитал ещё 20"), "{}", d.shown());
}

#[test]
fn a_count_that_is_taken_but_not_understood_is_the_same_cut_and_is_not_blamed_on_the_count() {
    let d = scenario("ignorednocount", BIG, 0, |c| (c.ignore_no_count, c.count_cap) = (true, 1000));
    assert!(!d.r.complete && d.r.created == BIG, "{}", d.shown());
    assert!(d.says("только первые 1000") && !d.says("без счёта"), "{}", d.shown());
}

#[test]
fn a_count_on_the_ceiling_with_a_list_that_ended_earlier_says_it_ended_earlier_not_on_the_ceiling() {
    // C#: потолок 250 при 230 заявках; здесь потолок 1000 — сервер называет его, а список короче
    let d = scenario("shortofcap", COUNT, 0, |c| (c.ignore_no_count, c.claim_count) = (true, 1000));
    assert!(d.r.created == COUNT, "{}", d.shown());
    assert!(d.says("раньше, чем обещал сервер") && !d.says("ровно на потолке"), "{}", d.shown());
}

// ---------- 11е: стык страниц и второй проход ----------

#[test]
fn a_ticket_lost_on_the_page_boundary_is_found_by_the_second_pass_in_change_order() {
    let d = scenario("unstable", COUNT, 0, |c| c.unstable_boundary = true);
    assert!(d.r.complete && d.r.created == COUNT && d.r.failed == 0, "{}", d.shown());
    assert!(
        d.says("которых не было в первом: 1") && d.says("повторял заявки на стыке страниц: 1") && !d.says("раньше, чем обещал"),
        "{}",
        d.shown()
    );
    let lists = d.lists();
    assert!(lists.len() == 4 && lists[2].contains("sort=Changed%20desc"), "второй проход — весь список: {}", lists.join(" | "));
    // ход второго прохода по всему списку — сколько дочитано (C# :368: `дочитано {recovered}`)
    let second: Vec<String> = d.said.all().into_iter().filter(|l| l.starts_with("Сверяю")).collect();
    assert_eq!(
        second,
        ["Сверяю список вторым проходом: страница 1, дочитано 0…", "Сверяю список вторым проходом: страница 2, дочитано 1…"]
    );
}

#[test]
fn the_second_pass_also_runs_when_the_count_is_only_a_lower_bound_and_does_not_blame_the_server_for_counting_twice() {
    let d = scenario("unstablecapped", BIG, 0, |c| (c.unstable_boundary, c.count_cap) = (true, 1000));
    assert!(d.r.created == BIG && d.r.failed == 0, "{}", d.shown());
    assert!(d.says("которых не было в первом: 1") && !d.says("Сервер насчитал"), "{}", d.shown());
}

#[test]
fn a_count_that_caught_up_with_the_list_still_leaves_the_second_pass_when_a_ticket_came_twice() {
    // счёт на одну меньше (за время выгрузки появились новые), а повтор на стыке был: потерянная заявка находится
    let d = scenario("masked", COUNT, 0, |c| (c.unstable_boundary, c.claim_count) = (true, COUNT - 1));
    assert!(d.r.created == COUNT && d.r.failed == 0 && d.says("которых не было в первом: 1"), "{}", d.shown());
}

#[test]
fn a_second_pass_that_gets_no_count_refused_asks_the_same_page_with_it_and_reaches_the_end() {
    let d = scenario("secondrefused", COUNT, 0, |c| (c.unstable_boundary, c.reject_no_count_when_changed) = (true, true));
    assert!(d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(!d.says("не удался") && d.says("которых не было в первом: 1"), "{}", d.shown());
}

#[test]
fn a_server_that_repeats_the_last_page_past_the_end_without_has_next_ends_both_passes_without_a_loop() {
    let d = scenario("clamped", COUNT, 0, |c| (c.unstable_boundary, c.clamp_no_has_next) = (true, true));
    assert!(d.r.complete && d.r.created == COUNT && d.r.failed == 0, "{}", d.shown());
    // повтор хвоста за концом списка — не стык страниц: в «повторял» он не входит
    assert!(!d.says("одну и ту же страницу") && d.says("которых не было в первом: 1") && d.says("на стыке страниц: 1)"), "{}", d.shown());
}

#[test]
fn a_server_that_does_not_understand_the_page_and_cuts_it_to_twenty_five_is_stuck_not_at_the_end() {
    let d = scenario("ignoredshort", COUNT, 0, |c| (c.ignore_page, c.no_count) = (true, true));
    assert!(!d.r.complete && d.r.created == 25 && d.says("одну и ту же страницу"), "{}", d.shown());
}

#[test]
fn a_second_pass_that_gets_the_same_page_again_takes_what_is_on_it_and_does_not_go_in_circles() {
    let d = scenario("stucksecond", COUNT, 0, |c| (c.unstable_boundary, c.ignore_page_when_changed) = (true, true));
    assert!(d.r.created == COUNT && d.r.failed == 0 && !d.r.complete, "{}", d.shown());
    assert_eq!(d.lists().len(), 4, "две страницы первого прохода и две второго: вторая — та же, стоп: {}", d.lists().join(" | "));
    assert!(d.says("одну и ту же страницу"), "{}", d.shown());
}

#[test]
fn a_count_on_the_ceiling_does_not_make_a_double_count_a_conclusion() {
    // счёт — потолок («1000 или больше»), а отдано меньше: «посчитана дважды» тут не вывод — только при точном счёте
    let d = scenario("cappedclaim", COUNT, 0, |c| (c.unstable_boundary, c.claim_count) = (true, 1000));
    assert!(d.r.created == COUNT && d.says("которых не было в первом: 1") && !d.says("Сервер насчитал"), "{}", d.shown());
}

#[test]
fn a_server_that_counts_one_more_than_it_gives_is_told_so_and_not_asked_to_repeat_later() {
    let d = scenario("inflated", COUNT, 0, |c| c.claim_count = COUNT + 1);
    assert!(d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(
        d.says(&format!("Сервер насчитал {}, а разных заявок в списке {COUNT}", COUNT + 1)) && !d.says("раньше, чем обещал"),
        "{}",
        d.shown()
    );
}

#[test]
fn with_a_limit_the_same_count_one_over_is_checked_by_a_second_pass_in_change_order_from_the_old() {
    let d = scenario("inflatedlimited", COUNT, 500, |c| c.claim_count = COUNT + 1);
    assert!(d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(
        d.says(&format!("Сервер насчитал {}, а разных заявок в списке {COUNT}", COUNT + 1)) && !d.says("Повторите выгрузку позже"),
        "{}",
        d.shown()
    );
}

#[test]
fn with_a_limit_a_ticket_lost_on_the_boundary_is_found_by_the_second_pass_which_reads_by_change_from_the_old() {
    let d = scenario("lostlimited", COUNT, 500, |c| c.unstable_boundary_changed = 1);
    assert!(d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(d.says("которых не было в первом: 1") && !d.says("раньше, чем обещал"), "{}", d.shown());
    // его порядок — по изменению от давних и номеру (живой сервер его принимает), а не по созданию — тот не проверен
    let lists = d.lists();
    assert!(
        lists.iter().any(|t| t.contains("sort=Changed%20asc,%20Id%20asc")) && !lists.iter().any(|t| t.contains("sort=Created")),
        "{}",
        lists.join(" | ")
    );
    // ход второго прохода с пределом — сколько найдено (выгружается найденное в конце)
    let said = d.said.all();
    assert!(said.contains(&"Сверяю список вторым проходом: страница 2, найдено 1…".to_string()), "{said:?}");
}

#[test]
fn a_limit_exactly_the_size_of_the_list_still_gets_a_second_pass_because_the_whole_list_fits() {
    let d = scenario("exactlimit", COUNT, COUNT as i32, |c| c.unstable_boundary_changed = 1);
    assert!(d.r.created == COUNT && d.says("которых не было в первом: 1"), "{}", d.shown());
}

#[test]
fn new_tickets_over_the_limit_are_not_lost_silently_the_freshest_go_out_and_the_rest_are_counted() {
    // за время выгрузки появились три новые, а до предела одно место: выгружается самая свежая, итог говорит, сколько не уместилось
    let d = scenario("grown", COUNT, COUNT as i32, |c| (c.unstable_boundary_changed, c.new_after_lists, c.new_count) = (1, 2, 3));
    assert!(d.r.created == COUNT && d.r.complete, "{}", d.shown());
    assert!(
        d.says(&format!(
            "больше заявок, чем «Не больше, заявок» ({COUNT}): из найденных вторым проходом выгружены самые свежие, ещё 3 — за пределом"
        )),
        "{}",
        d.shown()
    );
    let freshest = COUNT + 2;
    assert!(d.tmp.join(&format!("{TICKETS_FOLDER}/{}/{}", month(freshest), named(freshest))).is_file(), "{}", d.shown());
}

#[test]
fn of_two_lost_tickets_and_one_free_place_the_fresher_one_is_taken() {
    // на стыке потеряны две, а до предела — одно место (сервер насчитал на одну меньше, предел — столько же)
    let limit = COUNT - 1;
    let d = scenario("oneslot", COUNT, limit as i32, |c| (c.unstable_boundary_changed, c.claim_count) = (2, COUNT - 1));
    assert!(d.r.created == limit && d.says("которых не было в первом: 1"), "{}", d.shown());
    assert!(d.says(&format!("больше заявок, чем «Не больше, заявок» ({limit})")), "{}", d.shown());
    // свежая из двух — как выбрал бы первый проход: предел — первые по изменению
    let at = |i: usize| d.tmp.join(&format!("{TICKETS_FOLDER}/{}/{}", month(i), named(i)));
    assert!(at(29).is_file() && !at(28).exists(), "{}", d.shown());
}

#[test]
fn a_first_pass_that_already_filled_the_limit_has_no_second_pass() {
    let d = scenario("fulllimit", COUNT, COUNT as i32, |c| c.overlap = true);
    assert!(d.r.created == COUNT, "{}", d.shown());
    assert!(!d.lists().iter().any(|t| t.contains("sort=Changed%20asc")), "{}", d.lists().join(" | "));
}

#[test]
fn a_count_on_the_ceiling_with_a_limit_above_it_has_no_second_pass_because_the_total_is_unknown() {
    let d = scenario("cappedlimit", COUNT, 2000, |c| c.claim_count = 1000);
    assert!(d.r.created == COUNT && d.lists().len() == 2, "первый проход — две страницы: {}", d.shown());
    assert!(!d.lists().iter().any(|t| t.contains("sort=Changed%20asc")), "{}", d.lists().join(" | "));
}

#[test]
fn a_ticket_that_stands_in_the_list_twice_is_named_in_the_summary() {
    let d = scenario("twice", COUNT, 500, |c| c.duplicate_id = id_of(100));
    assert!(d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(
        d.says(&format!("Сервер насчитал {}, а разных заявок в списке {COUNT}", COUNT + 1))
            && d.says(&format!("#{} пришла в списке дважды", id_of(100))),
        "{}",
        d.shown()
    );
}

#[test]
fn a_repeat_on_the_page_boundary_of_the_first_pass_is_not_a_double_count() {
    // список сдвинулся — во втором проходе, в другом порядке, заявка не повторяется, и итог её не называет
    let d = scenario("boundary", COUNT, 500, |c| (c.overlap, c.claim_count) = (true, COUNT + 1));
    assert!(
        d.says(&format!("Сервер насчитал {}, а разных заявок в списке {COUNT}", COUNT + 1)) && !d.says("пришла в списке дважды"),
        "{}",
        d.shown()
    );
}

#[test]
fn a_second_pass_that_reads_a_page_beyond_the_limit_stops_and_says_the_list_changed_a_lot() {
    // за выгрузку прибавилась тысяча: прочитав на страницу больше предела, второй проход дальше не читает
    let d = scenario("toolong", COUNT, 300, |c| (c.unstable_boundary_changed, c.new_after_lists, c.new_count) = (1, 2, 1000));
    assert!(d.r.created == 300 && !d.r.complete, "{}", d.shown());
    // (прочитаны давние, а свежие — первые по изменению — нет; «самые свежие» не обещано)
    assert!(
        d.says(&format!("больше, чем насчитал сервер ({COUNT})")) && d.says("повторите выгрузку") && !d.says("самые свежие"),
        "{}",
        d.shown()
    );
    let lists = d.lists();
    assert!(
        lists.iter().any(|t| t.contains("Changed%20asc") && t.ends_with("&page=3"))
            && !lists.iter().any(|t| t.contains("Changed%20asc") && t.ends_with("&page=4")),
        "{}",
        lists.join(" | ")
    );
}

#[test]
fn a_second_pass_that_is_past_the_limit_only_on_the_last_page_has_read_the_list_to_the_end() {
    let d = scenario("readtoend", COUNT, 250, |c| (c.unstable_boundary_changed, c.new_after_lists, c.new_count) = (1, 2, 251));
    assert!(d.r.created == 250 && d.r.complete && !d.says("не читан"), "{}", d.shown());
}

#[test]
fn a_count_above_the_limit_means_the_list_does_not_fit_and_there_is_no_second_pass() {
    let d = scenario("overlimit", COUNT, 250, |c| c.claim_count = 300);
    assert!(d.r.created == COUNT && !d.says("второй проход"), "{}", d.shown());
    assert!(d.says(&format!("раньше, чем обещал сервер ({COUNT} из 300)")), "{}", d.shown());
}

#[test]
fn a_count_far_above_what_is_given_is_not_a_double_count_and_the_run_is_not_done() {
    let d = scenario("farshort", COUNT, 0, |c| c.claim_count = COUNT + 50);
    assert!(!d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(d.says(&format!("раньше, чем обещал сервер ({COUNT} из {})", COUNT + 50)) && !d.says("посчитана дважды"), "{}", d.shown());
}

// ---------- 12, 13: период и предел ----------

#[test]
fn rows_past_the_end_of_the_period_end_the_read_on_the_first_page_when_the_server_ignored_the_date() {
    // сервер условие по дате не применил (отдал всё): чтение обрывается за концом периода, на первой же странице
    let until = chrono::NaiveDate::from_ymd_opt(2026, 1, 31).unwrap().and_hms_opt(0, 0, 0).unwrap();
    let in_period = (0..COUNT).filter(|&i| created_of(i) <= until + chrono::Duration::days(1)).count();
    assert!(in_period > 0 && in_period < 200);
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("period"));
    let query = TaskQuery { created: DateSpan { from: None, to: Some(until) }, ..TaskQuery::default() };
    let r = export(&fake, &query, 0, tmp.path(), &AtomicBool::new(false), &fast(), &Said::default());
    assert!(r.complete && r.error.is_empty() && r.found == in_period, "{}", show(&r));
    assert_eq!(fake.lists().len(), 1, "{}", fake.lists().join(" | "));
}

#[test]
fn a_limit_takes_the_freshest_by_change_date_without_the_creation_sort() {
    // «не больше 210»: двести с первой страницы и десять со второй
    let d = scenario("limit", COUNT, 210, |_| {});
    assert!(d.r.found == 210 && d.r.created == 210 && d.r.complete, "{}", d.shown());
    let lists = d.lists();
    assert!(lists.len() == 2 && lists[0].contains("sort=Changed%20desc"), "{}", lists.join(" | "));
    let names: Vec<String> = on_disk(d.tmp.path()).iter().map(|p| p.rsplit(['/', '\\']).next().unwrap().to_string()).collect();
    assert!(names.iter().any(|n| n.starts_with("10021 —")), "самая давняя из выгруженных");
    assert!(!names.iter().any(|n| n.starts_with("10020 —")), "следующая за ней — уже за пределом");
    assert!(names.iter().any(|n| n.starts_with("10230 —")), "самая свежая");
}

#[test]
fn a_limit_of_one_exports_exactly_the_freshest_ticket() {
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("limit1"));
    let r = limited(&fake, 1, tmp.path());
    assert!(r.found == 1 && r.created == 1 && r.complete, "{}", show(&r));
    let files = on_disk(tmp.path());
    assert!(files.len() == 1 && files[0].ends_with(&named(COUNT - 1)), "{files:?}");
}

// ---------- что осталось после мутаций: ход, проверка записанного, отбор по дате, границы второго прохода ----------

/// Все строки хода без «из»: общее число неизвестно.
fn said_no_total(d: &Done) -> bool {
    d.said.all().iter().all(|m| !m.contains(" из "))
}

#[test]
fn a_file_removed_right_after_its_page_was_written_is_still_found_at_the_end() {
    // C# 11ж (vanishlate): файл пропадает не сразу — проверка между страницами видит только те, которым уже vanish_wait; записанное
    // последним на странице остаётся в списке до итога. «Антивирус» убирает все файлы первой страницы, когда просят первую
    // карточку второй (201-ю): те, что легли в последние 300 мс первой, итог находит
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("vanishlate"));
    let base = tmp.path().to_path_buf();
    fake.on_card(move |seq| {
        if seq == 201 {
            for i in 0..200 {
                let _ = fs::remove_file(base.join(format!("{TICKETS_FOLDER}/{}/{}", month(i), named(i))));
            }
        }
    });
    let options = Options { vanish_wait: Duration::from_millis(300), ..fast() };
    let r = export(&fake, &TaskQuery::default(), 0, tmp.path(), &AtomicBool::new(false), &options, &Said::default());
    assert!(r.created == COUNT && !r.complete && r.error.contains("Записано, но уже нет на диске: "), "{}", show(&r));
}

#[test]
fn a_file_removed_after_its_check_time_was_already_passed_is_not_reported() {
    // файл первой страницы проверен между страницами (ему уже было vanish_wait) и лежал — потом его убрали: итог его не называет
    let (fake, tmp) = (Fake::new(COUNT), Tmp::new("vanishafter"));
    let gone = tmp.join(&format!("{TICKETS_FOLDER}/{}/{}", month(0), named(0)));
    let gone2 = gone.clone();
    fake.on_card(move |seq| {
        if seq == 201 {
            fs::remove_file(&gone2).unwrap();
        }
    });
    let options = Options { vanish_wait: Duration::from_millis(1), ..fast() };
    let r = export(&fake, &TaskQuery::default(), 0, tmp.path(), &AtomicBool::new(false), &options, &Said::default());
    assert!(!gone.exists(), "файл убран");
    assert!(r.complete && r.created == COUNT && r.error.is_empty(), "{}", show(&r));
}

#[test]
fn what_the_second_pass_finds_beyond_the_count_raises_the_total_of_the_progress() {
    // C# 11е (masked): счёт на одну меньше, чем заявок; первый проход — 229 из 229, второй находит ещё одну — «230 из 230»
    let d = scenario("maskedprogress", COUNT, 0, |c| (c.unstable_boundary, c.claim_count) = (true, COUNT - 1));
    assert!(d.r.created == COUNT && d.r.complete, "{}", d.shown());
    assert_eq!(d.said.last_count().as_deref(), Some("Выгружено 230 из 230"), "{:?}", d.said.all());
}

#[test]
fn what_the_second_pass_finds_when_the_total_is_only_a_lower_bound_does_not_make_the_total_known() {
    // C# 11е (cappedclaim): счёт — потолок, второй проход нашёл одну; хода «из» не появляется
    let d = scenario("cappedprogress", COUNT, 0, |c| (c.unstable_boundary, c.claim_count) = (true, 1000));
    assert!(d.r.created == COUNT && d.says("которых не было в первом: 1"), "{}", d.shown());
    assert!(said_no_total(&d), "{:?}", d.said.all());
    assert_eq!(d.said.last_count().as_deref(), Some("Выгружено 230"), "{:?}", d.said.all());
}

#[test]
fn a_limit_above_a_count_that_is_only_a_lower_bound_has_no_total_in_the_progress() {
    // C# 11г (cappedlimit): счёт 1000 или больше, предел 2000 — сколько выйдет, неизвестно: ход без «из»
    let d = scenario("cappedlimitprogress", COUNT, 2000, |c| c.claim_count = 1000);
    assert!(d.r.created == COUNT, "{}", d.shown());
    assert!(said_no_total(&d), "{:?}", d.said.all());
    assert_eq!(d.said.last_count().as_deref(), Some("Выгружено 230"), "{:?}", d.said.all());
}

#[test]
fn rows_before_the_start_of_the_period_are_skipped_and_counted_in_the_summary() {
    // C# 12: сервер условие по дате не применил — строки раньше «с» (за вычетом суток) пропущены: с 46-й заявки они в периоде
    let (fake, tmp, said) = (Fake::new(COUNT), Tmp::new("outsidefrom"), Said::default());
    let query = TaskQuery { created: DateSpan { from: Some(created_of(50)), to: None }, ..TaskQuery::default() };
    let r = export(&fake, &query, 0, tmp.path(), &AtomicBool::new(false), &fast(), &said);
    assert!(r.complete && r.created == COUNT - 46 && r.found == COUNT - 46, "{}", show(&r));
    assert_eq!(
        r.error,
        "Сервер вернул заявки вне выбранного периода (46) — они пропущены: условие по дате он, похоже, не применил",
        "{}",
        show(&r)
    );
    assert_eq!(fake.lists().len(), 2, "две страницы первого прохода, второго нет: {}", fake.lists().join(" | "));
}

#[test]
fn rows_beyond_the_end_of_the_period_that_only_the_second_pass_sees_are_counted_in_the_summary() {
    // период кончается на последней заявке; за время выгрузки появились десять новых — шесть из них позже конца и суток
    let (fake, tmp, said) = (
        Fake::with(COUNT, |c| (c.unstable_boundary, c.new_after_lists, c.new_count) = (true, 2, 10)),
        Tmp::new("outsidesecond"),
        Said::default(),
    );
    let query = TaskQuery { created: DateSpan { from: None, to: Some(created_of(COUNT - 1)) }, ..TaskQuery::default() };
    let r = export(&fake, &query, 0, tmp.path(), &AtomicBool::new(false), &fast(), &said);
    assert!(r.complete && r.created == COUNT + 4, "{}", show(&r));
    assert!(r.error.contains("которых не было в первом: 5"), "{}", show(&r));
    assert!(
        r.error.contains("Сервер вернул заявки вне выбранного периода (6) — они пропущены: условие по дате он, похоже, не применил"),
        "{}",
        show(&r)
    );
}

#[test]
fn a_list_of_exactly_the_ceiling_with_a_next_page_flag_is_not_called_cut() {
    // C# 11г при потолке, равном числу заявок: страницы без счёта сказали, есть ли следующая, — список не обрезан, итог чист
    let d = scenario("exactceiling", 1000, 0, |_| {});
    assert!(d.r.complete && d.r.created == 1000 && d.r.failed == 0, "{}", d.shown());
    assert!(d.r.error.is_empty(), "{}", d.shown());
}

#[test]
fn rows_created_at_the_same_moment_are_not_an_order_that_was_broken() {
    let d = scenario("ties", COUNT, 0, |c| c.tie_created = true);
    assert!(d.r.complete && d.r.created == COUNT && d.r.error.is_empty(), "{}", d.shown());
}

#[test]
fn a_list_that_is_exactly_one_page_ends_on_the_empty_second_page_and_is_not_stuck() {
    // C# (Count = 200): вторая страница пуста — это конец, а не «сервер отдаёт одну и ту же страницу»
    let d = scenario("onepage", 200, 0, |_| {});
    assert!(d.r.complete && d.r.created == 200 && d.r.error.is_empty(), "{}", d.shown());
    assert_eq!(d.lists().len(), 2, "{}", d.lists().join(" | "));
}

#[test]
fn a_full_last_page_without_a_next_page_flag_still_asks_for_one_more_page() {
    // C# (Count = 400, счёт принят, но не понят: ответы со счётом, без HasNextPage): полная страница ничего не доказывает —
    // конец подтверждает пустая третья
    let d = scenario("fulllast", 400, 0, |c| c.ignore_no_count = true);
    assert!(d.r.complete && d.r.created == 400, "{}", d.shown());
    assert_eq!(d.lists().len(), 3, "{}", d.lists().join(" | "));
}

#[test]
fn a_second_pass_whose_first_page_is_refused_with_400_is_not_asked_again_with_a_count() {
    // C# 11е: 400 на первой странице второго прохода — не из-за счёта (на первой он и так есть): итог говорит, что проход не удался
    let d = scenario("secondfirst400", COUNT, 0, |c| (c.unstable_boundary, c.fail_changed_once) = (true, 400));
    assert!(!d.r.complete && d.r.created == COUNT - 1, "{}", d.shown());
    assert!(d.says("Второй проход по списку не удался"), "{}", d.shown());
    assert_eq!(d.lists().len(), 3, "{}", d.lists().join(" | "));
}

#[test]
fn a_second_page_refused_without_a_count_is_asked_again_and_not_skipped() {
    // C# 11д для второго прохода: 400 без счёта на его второй странице — та же страница со счётом; новые на ней находятся
    let d = scenario("secondpagecount", COUNT, 0, |c| {
        (c.unstable_boundary, c.reject_no_count_when_changed, c.new_after_lists, c.new_count) = (true, true, 2, 250)
    });
    assert!(d.r.complete && d.r.created == COUNT + 250 && d.r.found == COUNT + 250, "{}", d.shown());
    assert!(d.says("которых не было в первом: 251"), "{}", d.shown());
}

#[test]
fn a_second_pass_that_has_read_exactly_a_page_more_than_the_limit_and_has_a_next_page_goes_on() {
    // C# 11ж: предел 400, прочитано ровно 600 (= предел + страница), но следующая есть — «больше» не сказано, читаем до конца
    let d = scenario("limitedge", COUNT, 400, |c| (c.unstable_boundary_changed, c.new_after_lists, c.new_count) = (1, 2, 375));
    assert!(d.r.complete && d.r.created == 400 && d.r.found == 400, "{}", d.shown());
    assert!(d.says("которых не было в первом: 171") && d.says("ещё 205 — за пределом"), "{}", d.shown());
    assert!(!d.says("Второй проход прочёл"), "{}", d.shown());
}

#[test]
fn a_stop_in_the_middle_of_the_second_pass_with_a_limit_exports_and_counts_nothing_of_what_it_found() {
    // C# (stop на четвёртом запросе списка — второй странице второго прохода): найденное в конце не выгружается и в итог не идёт
    let (fake, tmp, said) = (Fake::with(COUNT, |c| c.unstable_boundary_changed = 1), Tmp::new("stopsecond"), Said::default());
    let stop = Arc::new(AtomicBool::new(false));
    *fake.shared.stop_at.lock().unwrap() = Some((4, stop.clone()));
    let r = export(&fake, &TaskQuery::default(), 500, tmp.path(), &stop, &fast(), &said);
    assert!(!r.complete && r.created == COUNT - 1 && r.error.contains("Остановлено"), "{}", show(&r));
    assert!(!r.error.contains("которых не было в первом"), "{}", show(&r));
    assert_eq!(on_disk(tmp.path()).len(), COUNT - 1);
}

#[test]
fn a_limit_near_the_top_of_i32_does_not_overflow_in_the_second_pass() {
    // КРАСНАЯ до починки (run.rs: `limit + LIST_PAGE_SIZE` переполняется): предел 2147483647, счёт точный и на одну больше, чем
    // отдано — первый проход недобрал, второй идёт. Ответ — как на большой предел без переполнения (C#, предел 1000000)
    let d = scenario("hugelimit", COUNT, i32::MAX, |c| c.claim_count = COUNT + 1);
    assert!(d.r.complete && d.r.created == COUNT, "{}", d.shown());
    assert!(d.says(&format!("Сервер насчитал {}, а разных заявок в списке {COUNT}", COUNT + 1)), "{}", d.shown());
    assert!(!d.says("Второй проход прочёл"), "{}", d.shown());
}
