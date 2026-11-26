//! Одна пишущая выгрузка за раз (`KnowledgeExport.Writer`, `InTurnAsync`, карточка 4.5-2): окно «Поиск заявок» и автовыгрузка
//! пишут в одну папку — вторая ждёт первую и так и говорит («Жду…», потом «Начинаю…»), «Остановить» слышит и в очереди, и до неё.
//! Сценарии — `KnowledgeExport.SelfCheck.Mass.cs` («очередь пишущих», ~636–691). Очередь одна на процесс, поэтому проверки этого
//! файла идут по одной (`SERIAL`): иначе «не ждала» зависело бы от соседней проверки. Держит очередь выгрузка, чья карточка
//! ждёт сигнала сервера; «ждёт» — по ходу, который выгрузка присылает сама, до того как встала.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use ticketboard::export::run::{ExportResult, Options, WAITING, export_found, export_rows};
use ticketboard::export::{AttachmentScope, Context};
use ticketboard::intraservice::query::TaskQuery;
use ticketboard::intraservice::{Client, Date, Found, User};
use ticketboard::rules::ClosedNames;

const STOPPED_BEFORE_START: &str = "Остановлено до начала — ничего не записано.";
const STOPPED_WAITING: &str = "Остановлено, пока ждали другую выгрузку, — ничего не записано.";
const STARTING: &str = "Начинаю…";

static SERIAL: Mutex<()> = Mutex::new(());

fn date(text: &str) -> Date {
    chrono::DateTime::parse_from_rfc3339(text).unwrap()
}

// ---------- поддельный сервер: карточки, истории и список из одной заявки ----------

#[derive(Default)]
struct Shared {
    log: Mutex<Vec<String>>,
    /// Карточка этой заявки ждёт `released` (0 — нет).
    hold: Mutex<i32>,
    released: AtomicBool,
    /// Карточки отвечают этим кодом (0 — нет).
    fail: Mutex<u16>,
}

struct Fake {
    shared: Arc<Shared>,
    url: String,
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
    let (code, body) = if let Some(rest) = path.strip_prefix("/api/task/") {
        let id: i32 = rest.split('?').next().unwrap_or("").parse().unwrap_or(0);
        if *shared.hold.lock().unwrap() == id {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !shared.released.load(SeqCst) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
        let fail = *shared.fail.lock().unwrap();
        if fail != 0 {
            (fail, String::new())
        } else {
            (
                200,
                format!(
                    r#"{{"Task":{{"Id":{id},"Name":"Заявка {id}","StatusName":"Выполнена","Creator":"Петрова А.","Created":"2026-09-10T10:00:00+03:00","Changed":"2026-09-20T12:00:00+03:00"}}}}"#
                ),
            )
        }
    } else if path.starts_with("/api/tasklifetime?") {
        (
            200,
            r#"{"TaskLifetimes":[{"Date":"2026-09-11T10:00:00+03:00","Editor":"Иванов И.","EditorId":60,"Comments":"готово"}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"#.into(),
        )
    } else if path.starts_with("/api/task?") {
        // список — одна заявка №2 на первой странице, дальше пусто
        let first = !path.contains("page=2") && !path.contains("page=3");
        let rows = if first {
            r#"{"Id":2,"Name":"Заявка 2","StatusId":29,"Created":"2026-09-10T10:00:00+03:00","Changed":"2026-09-20T12:00:00+03:00","Creator":"Петрова А."}"#
        } else {
            ""
        };
        (
            200,
            format!(
                r#"{{"Tasks":[{rows}],"Statuses":[{{"Id":29,"Name":"Выполнена"}}],"Paginator":{{"Count":{},"Page":1,"PageCount":1,"HasNextPage":false}}}}"#,
                usize::from(first)
            ),
        )
    } else {
        (404, String::new())
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {code} X\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

impl Fake {
    fn new() -> Fake {
        let shared = Arc::new(Shared::default());
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

    fn cards(&self, id: i32) -> usize {
        self.shared.log.lock().unwrap().iter().filter(|p| p.starts_with(&format!("/api/task/{id}?"))).count()
    }

    fn hold(&self, id: i32) {
        *self.shared.hold.lock().unwrap() = id;
        self.shared.released.store(false, SeqCst);
    }

    fn release(&self) {
        self.shared.released.store(true, SeqCst);
    }
}

fn row(id: i32) -> Found {
    Found {
        id,
        name: format!("Заявка {id}"),
        status: "Выполнена".into(),
        created: Some(date("2026-09-10T10:00:00+03:00")),
        changed: Some(date("2026-09-20T12:00:00+03:00")),
        ..Found::default()
    }
}

// ---------- запуск ----------

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("tb-turn-{tag}-{}-{}", std::process::id(), N.fetch_add(1, SeqCst)));
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

fn context() -> Context {
    Context { me: Some(User { id: 7, name: "Я Сам".into() }), closed: ClosedNames::default(), attachments: AttachmentScope::None }
}

fn fast() -> Options {
    Options { retry_delays: vec![Duration::ZERO; 3], vanish_wait: Duration::ZERO, ..Options::default() }
}

fn url_of(id: i32) -> String {
    format!("https://hd/Task/View/{id}")
}

type Lines = Arc<Mutex<Vec<String>>>;

fn lines_of(lines: &Lines) -> Vec<String> {
    lines.lock().unwrap().clone()
}

/// Выгрузка в своей нити: что она сказала по ходу, остановка и «закончила ли».
struct Running {
    handle: thread::JoinHandle<ExportResult>,
    lines: Lines,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl Running {
    fn rows(fake: &Fake, ids: &[i32], dir: &Path, stop: bool) -> Running {
        let rows: Vec<Found> = ids.iter().map(|&id| row(id)).collect();
        let (client, dir) = (fake.client(), dir.to_path_buf());
        Running::start(stop, move |lines, stop| {
            export_rows(&client, &rows, &dir, &url_of, &context(), &|l| lines.lock().unwrap().push(l), &stop, &fast())
        })
    }

    fn found(fake: &Fake, dir: &Path) -> Running {
        let (client, dir) = (fake.client(), dir.to_path_buf());
        Running::start(false, move |lines, stop| {
            export_found(&client, &TaskQuery::default(), 1, &dir, &url_of, &context(), &|l| lines.lock().unwrap().push(l), &stop, &fast())
        })
    }

    fn start(stopped: bool, run: impl FnOnce(Lines, Arc<AtomicBool>) -> ExportResult + Send + 'static) -> Running {
        let lines: Lines = Arc::default();
        let stop = Arc::new(AtomicBool::new(stopped));
        let done = Arc::new(AtomicBool::new(false));
        let (l, s, d) = (lines.clone(), stop.clone(), done.clone());
        let handle = thread::spawn(move || {
            let r = run(l, s);
            d.store(true, SeqCst);
            r
        });
        Running { handle, lines, stop, done }
    }

    fn says(&self, n: usize) {
        wait_until("ход выгрузки", || lines_of(&self.lines).len() >= n);
    }

    /// Кончила ли за срок; нет — это сама неудача проверки.
    fn finished_within(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while !self.done.load(SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        self.done.load(SeqCst)
    }
}

fn wait_until(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "не дождались: {what}");
        thread::sleep(Duration::from_millis(1));
    }
}

/// Выгрузка, что держит очередь: её карточка (заявка 1) ждёт `release`. Возвращается, когда очередь уже у неё.
fn hold_the_turn(fake: &Fake, dir: &Path) -> Running {
    fake.hold(1);
    let holder = Running::rows(fake, &[1], dir, false);
    wait_until("карточка держащей очередь выгрузки", || fake.cards(1) > 0);
    holder
}

/// Отпустить держащую и дождаться её итога.
fn let_go(fake: &Fake, holder: Running) -> ExportResult {
    fake.release();
    holder.handle.join().unwrap()
}

// ---------- очередь ----------

#[test]
fn an_export_that_comes_while_another_writes_waits_and_says_so_then_says_it_started() {
    // C# «очередь пишущих»: пока пишет другая выгрузка, эта ждёт и так и говорит, а не молчит на «Начинаю…»; папку не трогает;
    // дождалась — сразу «Начинаю…» и пишет; ждала — в итоге сколько
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::new();
    let (first_dir, dir) = (Tmp::new("turn-first"), Tmp::new("turn-second"));
    let _ = fs::remove_dir_all(dir.path());
    let holder = hold_the_turn(&fake, first_dir.path());
    let second = Running::rows(&fake, &[2], dir.path(), false);
    second.says(1);
    assert_eq!(lines_of(&second.lines), [WAITING], "пока пишет другая — только «Жду…»");
    assert!(!second.done.load(SeqCst) && !dir.path().exists(), "и папку не трогает");
    assert_eq!(fake.cards(2), 0, "и карточку не спрашивает");
    let first = let_go(&fake, holder);
    let r = second.handle.join().unwrap();
    let said = lines_of(&second.lines);
    assert_eq!(said.get(..2), Some(&[WAITING.to_string(), STARTING.to_string()][..]), "дождалась — «Начинаю…»: {said:?}");
    assert_eq!((r.created, r.error.as_str()), (1, ""), "{r:?}");
    assert!(r.waited > Duration::ZERO, "ждала — в итоге сколько: {:?}", r.waited);
    assert_eq!((first.created, first.waited), (1, Duration::ZERO), "держащая не ждала");
}

#[test]
fn a_find_export_waits_for_the_turn_in_the_same_way() {
    // C# «и выгрузка по условию ждёт так же»: «Жду…», папка не тронута; потом «Начинаю…», запись, `waited`
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::new();
    let (first_dir, dir) = (Tmp::new("turn-first"), Tmp::new("turn-found"));
    let _ = fs::remove_dir_all(dir.path());
    let holder = hold_the_turn(&fake, first_dir.path());
    let second = Running::found(&fake, dir.path());
    second.says(1);
    assert_eq!(lines_of(&second.lines), [WAITING]);
    assert!(!second.done.load(SeqCst) && !dir.path().exists(), "папку не трогает");
    let_go(&fake, holder);
    let r = second.handle.join().unwrap();
    let said = lines_of(&second.lines);
    assert_eq!(said.get(..2), Some(&[WAITING.to_string(), STARTING.to_string()][..]), "{said:?}");
    assert_eq!((r.created, r.error.as_str()), (1, ""), "{r:?}");
    assert!(r.waited > Duration::ZERO, "{:?}", r.waited);
}

#[test]
fn an_export_that_is_free_to_write_does_not_say_waiting_and_did_not_wait() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::new();
    let dir = Tmp::new("turn-free");
    let one = Running::rows(&fake, &[2], dir.path(), false);
    let r = one.handle.join().unwrap();
    assert_eq!((r.created, r.waited), (1, Duration::ZERO), "{r:?}");
    assert!(!lines_of(&one.lines).iter().any(|l| l == WAITING), "{:?}", lines_of(&one.lines));
}

#[test]
fn a_stop_while_waiting_gives_up_the_wait_and_writes_nothing() {
    // C# «остановили в очереди — когда «Жду…» уже сказано»: итог «пока ждали», ничего не создано, «Начинаю…» не сказано,
    // папка не тронута — и всё это, пока другая выгрузка ещё держит очередь
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::new();
    let (first_dir, dir) = (Tmp::new("turn-first"), Tmp::new("turn-stopped"));
    let _ = fs::remove_dir_all(dir.path());
    let holder = hold_the_turn(&fake, first_dir.path());
    let second = Running::rows(&fake, &[2], dir.path(), false);
    second.says(1);
    second.stop.store(true, SeqCst);
    let finished = second.finished_within(Duration::from_secs(5));
    let_go(&fake, holder); // как бы ни кончилось — держащую отпускаем
    assert!(finished, "остановленная выгрузка не бросила ждать, пока другая держала очередь");
    let r = second.handle.join().unwrap();
    assert_eq!((r.complete, r.created, r.error.as_str()), (false, 0, STOPPED_WAITING), "{r:?}");
    assert_eq!(lines_of(&second.lines), [WAITING], "«Начинаю…» не сказано");
    assert!(!dir.path().exists() && fake.cards(2) == 0, "ничего не записано и не спрошено");
}

#[test]
fn a_stop_before_the_start_does_not_join_the_queue_at_all() {
    // C# «остановили до начала, а очередь занята: не встаём в неё — ни «Жду…», ни «пока ждали»»
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::new();
    let (first_dir, dir) = (Tmp::new("turn-first"), Tmp::new("turn-before"));
    let holder = hold_the_turn(&fake, first_dir.path());
    let second = Running::rows(&fake, &[2], dir.path(), true);
    let finished = second.finished_within(Duration::from_secs(5));
    let_go(&fake, holder);
    assert!(finished, "остановленная до начала выгрузка встала в очередь");
    let r = second.handle.join().unwrap();
    assert_eq!((r.complete, r.created, r.error.as_str()), (false, 0, STOPPED_BEFORE_START), "{r:?}");
    assert!(lines_of(&second.lines).is_empty(), "ни «Жду…», ни «Начинаю…»: {:?}", lines_of(&second.lines));
    assert!(!dir.join("tickets").exists() && fake.cards(2) == 0, "ничего не записано и не спрошено");
}

#[test]
fn the_turn_is_given_back_whatever_the_export_ended_with() {
    // C# «очередь отдаётся, чем бы всё ни кончилось»: остановили до начала, остановили в очереди, оборвалась (401), закончилась —
    // следующая выгрузка встаёт сразу, не ждёт никого
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let fake = Fake::new();
    let dir = Tmp::new("turn-given-back");
    let next = |tag: &str| {
        let own = Tmp::new(tag);
        let run = Running::rows(&fake, &[2], own.path(), false);
        let finished = run.finished_within(Duration::from_secs(5));
        run.stop.store(true, SeqCst); // зависшая — не вечно
        let r = run.handle.join().unwrap();
        assert!(finished, "после «{tag}» очередь не отдана: следующая выгрузка ждёт");
        assert_eq!((r.created, r.waited, r.error.as_str()), (1, Duration::ZERO, ""), "после «{tag}»: {r:?}");
    };
    // остановили до начала
    let before = Running::rows(&fake, &[2], dir.path(), true);
    before.handle.join().unwrap();
    next("остановили до начала");
    // остановили в очереди
    let holder = hold_the_turn(&fake, dir.path());
    let waiting = Running::rows(&fake, &[3], dir.path(), false);
    waiting.says(1);
    waiting.stop.store(true, SeqCst);
    waiting.handle.join().unwrap();
    let_go(&fake, holder);
    next("остановили в очереди");
    // оборвалась: сервер не принимает логин
    *fake.shared.fail.lock().unwrap() = 401;
    let broken = Running::rows(&fake, &[4], dir.path(), false);
    let r = broken.handle.join().unwrap();
    assert!(r.error.contains("HTTP 401"), "{r:?}");
    *fake.shared.fail.lock().unwrap() = 0;
    next("оборвалась");
}
