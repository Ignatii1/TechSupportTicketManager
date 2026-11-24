//! Окно поиска: несколько выбранных строк, «Копировать (N)» и «Выгрузить выбранные (N)» (`SearchViewModel.SetSelection`,
//! `Copy`, `ExportSelected`, `Summary`, `Stop`, `ApplySettings`; `KnowledgeExport.Starting`/`Conclude`; карточка 4.2-3).
//! Ожидаемые тексты — из C#: подписи, отказы и итоги. Файлы проверяются на диске (`tickets/<год-месяц>/<номер> — <название>.md`),
//! движок выгрузки здесь не вызывается. Платформа Slint ставится один раз на процесс, поэтому проверка одна; каждый сценарий —
//! со своим окном, своим сервером и под `catch_unwind`: упавший не прячет следующий.

mod common;

use chrono::DateTime;
use common::Reply;
use common::window::Jobs;
use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ticketboard::app::App;
use ticketboard::export::{Context, build};
use ticketboard::intraservice::{Client, Date, parse};
use ticketboard::rules::ClosedNames;

// ---------- сервер ----------

/// Сервер с потоком на соединение: ответ, который держат, не задерживает остальные. Пути — в порядке прихода.
fn serve(answer: impl Fn(&str) -> Reply + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (seen2, answer) = (seen.clone(), Arc::new(answer));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let (seen, answer) = (seen2.clone(), answer.clone());
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head.split(' ').nth(1).unwrap_or("").to_string();
                seen.lock().unwrap().push(path.clone());
                let reply = answer(&path);
                let reason = if reply.code == 200 { "OK" } else { "Error" };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.code,
                    reply.content_type,
                    reply.body.len()
                );
                let _ = stream.write_all(&reply.body);
            });
        }
    });
    (url, seen)
}

const CHANGED: &str = "2026-10-14T12:00:00+03:00";
const READ_ONLY: &str =
    "Доска только для чтения: выгрузка в файлы — только когда доску ведёт прототип (он ничего не пишет, пока её держит TicketBoard.exe).";
const STOPPING: &str = "Останавливаю — дописываю начатое…";

/// Что делает сервер сейчас: переключатели сценария.
struct World {
    /// Карточки не отвечают, пока флаг поднят (только окну; расчёт ожидаемого текста идёт мимо).
    hold: AtomicBool,
    me_down: AtomicBool,
    /// Сколько заявок в списке (номера с 900).
    rows: AtomicUsize,
    /// Запросов карточки от окна пришло и сколько из них уже отвечено (после удержания).
    card_in: AtomicUsize,
    card_out: AtomicUsize,
    /// Карточка с этим номером отвечает 404 (0 — таких нет).
    gone: AtomicI32,
    /// Ещё одна такая же (0 — нет) и карточка, что держится, пока `slow_open` не поднят (0 — таких нет): запоздавший ответ.
    gone2: AtomicI32,
    slow_id: AtomicI32,
    slow_open: AtomicBool,
    /// «Кто я» этого сервера: номер и имя; исполнитель каждой заявки — пользователь с номером 7.
    me_id: AtomicI32,
    me_name: &'static str,
}

impl World {
    fn list(&self) -> String {
        let n = self.rows.load(Ordering::SeqCst) as i32;
        let row = |id: i32| {
            format!(
                r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":31,"Creator":"Петров Пётр","Created":"2026-10-14T11:00:00+03:00","Changed":"{CHANGED}","ExecutorIds":"7","Description":"Описание заявки {id}."}}"#
            )
        };
        let rows: Vec<String> = (900..900 + n).map(row).collect();
        format!(
            r#"{{"Tasks":[{}],"Statuses":[{{"Id":31,"Name":"Открыта"}}],"Paginator":{{"Count":{n},"Page":1,"PageCount":1}}}}"#,
            rows.join(",")
        )
    }

    fn card(&self, id: i32) -> String {
        format!(
            r#"{{"Task":{{"Id":{id},"Name":"Заявка {id}","StatusName":"Открыта","Creator":"Петров Пётр","Created":"2026-10-14T11:00:00+03:00","Changed":"{CHANGED}","ServiceName":"Печать","Description":"Описание заявки {id}."}}}}"#
        )
    }

    fn life(&self) -> &'static str {
        r#"{"TaskLifetimes":[{"Date":"2026-10-02T14:00:48","EditorId":40800,"Editor":"Иванов Иван Иванович","StatusId":31}],"Statuses":[{"Id":31,"Name":"Открыта"}],"Paginator":{"Count":1,"Page":1,"PageCount":1,"PageSize":50,"CountOnPage":1,"HasNextPage":null}}"#
    }

    /// `counted` — запрос от окна: его считают и его держат; расчёт ожидаемого текста идёт мимо.
    fn answer(&self, path: &str, counted: bool) -> Reply {
        if path == "/api/user?getcurrentuserinfo=true" {
            return if self.me_down.load(Ordering::SeqCst) {
                Reply::new(503, "text/plain", "")
            } else {
                Reply::json(&format!(r#"{{"Id":{},"Name":"{}"}}"#, self.me_id.load(Ordering::SeqCst), self.me_name))
            };
        }
        if let Some(rest) = path.strip_prefix("/api/task/") {
            let id: i32 = rest.split('?').next().unwrap_or("").parse().unwrap_or(0);
            if counted {
                self.card_in.fetch_add(1, Ordering::SeqCst);
                let until = Instant::now() + Duration::from_secs(20);
                while self.hold.load(Ordering::SeqCst) && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                while id == self.slow_id.load(Ordering::SeqCst) && !self.slow_open.load(Ordering::SeqCst) && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                self.card_out.fetch_add(1, Ordering::SeqCst);
            }
            if id == self.gone.load(Ordering::SeqCst) || id == self.gone2.load(Ordering::SeqCst) {
                return Reply::new(404, "text/plain", "");
            }
            return Reply::json(&self.card(id));
        }
        if path.starts_with("/api/tasklifetime?") {
            return Reply::json(self.life());
        }
        if path.starts_with("/api/task?") {
            return Reply::json(&self.list());
        }
        if path == "/api/taskstatus" {
            return Reply::json("[]");
        }
        Reply::new(404, "text/plain", "мимо")
    }
}

// ---------- окно ----------

struct Env {
    app: App,
    jobs: Jobs,
    world: Arc<World>,
    seen: Arc<Mutex<Vec<String>>>,
    url: String,
    oracle_url: String,
    /// Папка доски (`App.DataDir`).
    dir: PathBuf,
    /// Папка выгрузки из settings.json «KnowledgeDir» (в настройках она есть не всегда).
    out: PathBuf,
    clip: Rc<RefCell<Vec<String>>>,
}

fn client(url: &str, login: &str) -> Client {
    Client::new(url, login, "p", Duration::from_secs(10))
}

/// `owner` — доска своя (пишется); `with_out` — в settings.json задана папка выгрузки.
fn world(rows: usize, me_id: i32, me_name: &'static str) -> Arc<World> {
    Arc::new(World {
        hold: AtomicBool::new(false),
        me_down: AtomicBool::new(false),
        rows: AtomicUsize::new(rows),
        card_in: AtomicUsize::new(0),
        card_out: AtomicUsize::new(0),
        gone: AtomicI32::new(0),
        gone2: AtomicI32::new(0),
        slow_id: AtomicI32::new(0),
        slow_open: AtomicBool::new(false),
        me_id: AtomicI32::new(me_id),
        me_name,
    })
}

fn env(name: &str, jobs: &Jobs, rows: usize, owner: bool, with_out: bool) -> Env {
    let world = world(rows, 7, "Я Сам");
    let w = world.clone();
    let (url, seen) = serve(move |p| w.answer(p, true));
    let w = world.clone();
    let (oracle_url, _) = serve(move |p| w.answer(p, false));
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("my-base");
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    // путь — строкой JSON: обратные черты Windows без экранирования ломают весь файл, и настройки — по умолчанию
    let knowledge = if with_out {
        format!(r#""KnowledgeDir": {}, "#, serde_json::to_string(&out.display().to_string()).unwrap())
    } else {
        String::new()
    };
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, {knowledge}"IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();
    let app = App::open(&dir, owner).unwrap();
    app.use_client(Some(client(&url, "me")));
    let clip = Rc::new(RefCell::new(Vec::new()));
    let c = clip.clone();
    app.use_clipboard(move |text| {
        c.borrow_mut().push(text.to_string());
        true
    });
    Env { app, jobs: jobs.clone(), world, seen, url, oracle_url, dir, out, clip }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Целое с разделителем тысяч U+00A0 (`ToString("N0")` у ru-RU).
fn n0(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(c);
    }
    out
}

/// Текст без строки `exported:` — у неё свой момент у каждой стороны.
fn plain(text: &str) -> String {
    text.split('\n').filter(|l| !l.starts_with("exported: ")).collect::<Vec<_>>().join("\n")
}

fn exported() -> Date {
    DateTime::parse_from_rfc3339("2026-11-17T10:00:00+03:00").unwrap()
}

/// Все файлы под папкой (пусто, если папки нет).
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else { return found };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

impl Env {
    fn count(&self, prefix: &str) -> usize {
        self.seen.lock().unwrap().iter().filter(|p| p.starts_with(prefix)).count()
    }

    fn pump(&self) {
        let pending = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in pending {
            job();
        }
    }

    #[track_caller]
    fn wait(&self, what: &str, done: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            self.pump();
            if done() {
                return;
            }
            assert!(Instant::now() < until, "не дождались: {what}");
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
    }

    #[track_caller]
    fn wait_is(&self, what: &str, get: impl Fn() -> String, want: &str) {
        let until = Instant::now() + Duration::from_secs(6);
        loop {
            self.pump();
            if get() == want || Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
        assert_eq!(get(), want, "{what}");
    }

    /// Быстрый поиск с доски: список из всех заявок мира на месте.
    #[track_caller]
    fn find(&self) {
        let f = &self.app.finder;
        let n = self.world.rows.load(Ordering::SeqCst);
        self.app.window.invoke_server_search("слово".into());
        self.wait("список поиска", || f.get_rows().row_count() == n && !f.get_busy());
        self.wait("справочники окна", || !f.get_reference_notes().is_empty());
    }

    /// То же на другом сервере (после смены клиента): список из всех его заявок на месте.
    #[track_caller]
    fn find_on(&self, other: &World) {
        let f = &self.app.finder;
        let n = other.rows.load(Ordering::SeqCst);
        self.app.window.invoke_server_search("слово".into());
        self.wait("список поиска на новом сервере", || f.get_rows().row_count() == n && !f.get_busy());
    }

    /// Просмотры, что запустили щелчки (`n` штук), дочитаны: их запросы карточки сосчитаны, окно не ждёт чтения. Дальше
    /// счёт запросов окна — только от того, что запустили после.
    #[track_caller]
    fn previews_done(&self, n: usize) {
        self.wait("просмотры дочитаны", || {
            self.world.card_in.load(Ordering::SeqCst) >= n && !self.app.finder.get_preview_busy()
        });
    }

    /// Номера из путей запросов с этим началом (`/api/task/` — карточки, `/api/tasklifetime` — истории).
    fn ids(&self, prefix: &str) -> Vec<i32> {
        let seen = self.seen.lock().unwrap();
        seen.iter()
            .filter(|p| p.starts_with(prefix))
            .filter_map(|p| {
                let rest = p.strip_prefix(prefix)?;
                let digits = rest.trim_start_matches("?taskid=").trim_start_matches('/');
                digits.split(['?', '&']).next()?.parse().ok()
            })
            .collect()
    }

    /// Файл заявки в другой папке выгрузки.
    fn ticket_file_in(&self, base: &Path, id: i32) -> PathBuf {
        base.join("tickets").join("2026-10").join(format!("{id} — Заявка {id}.md"))
    }

    fn message(&self) -> String {
        self.app.finder.get_work_message().to_string()
    }

    fn picked(&self) -> Vec<bool> {
        self.app.finder.get_rows().iter().map(|r| r.picked).collect()
    }

    fn pick(&self, i: i32, ctrl: bool, shift: bool) {
        self.app.finder.invoke_pick(i, ctrl, shift);
    }

    /// Просмотр назвал заявку с этим номером (заголовок ставится сразу, до чтения).
    fn title_has(&self, id: i32) -> bool {
        self.app.finder.get_preview_title().contains(&id.to_string())
    }

    fn agent_text(&self, id: i32) -> String {
        let c = client(&self.oracle_url, "me");
        let page = parse::search(&self.world.list()).expect("список");
        let row = page.found.into_iter().find(|r| r.id == id).expect("строка");
        let closed = ["Выполнена", "Ожидание ответа с автозакрытием", "Закрыта", "Отменена"].map(String::from);
        let context = Context { me: c.get_current_user().ok(), closed: ClosedNames::new(&closed) };
        let url = format!("{}/Task/View/{id}", self.url.trim_end_matches('/'));
        build(&c, &row, &url, &context, &exported()).expect("build собрал текст")
    }

    /// Файл заявки: месяц создания — по дате из строки списка.
    fn ticket_file(&self, id: i32) -> PathBuf {
        self.out.join("tickets").join("2026-10").join(format!("{id} — Заявка {id}.md"))
    }
}

type Scenario<'a> = (&'a str, fn(&Jobs));

// ---------- сценарии ----------

/// 1. Выбор строк: один щелчок, Ctrl добавляет и снимает, Shift — подряд; просмотр и подписи следуют за выбором.
fn picking_rows(jobs: &Jobs) {
    let e = env("pick", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    assert_eq!(e.picked(), vec![false; 3], "до щелчков не выбрано ничего");
    assert_eq!((f.get_copy_label().as_str(), f.get_export_label().as_str()), ("Копировать", "Выгрузить выбранные"), "подписи без выбора");
    assert!(!f.get_can_export(), "ничего не выбрано — выгружать нечего");

    e.pick(0, false, false);
    assert_eq!(e.picked(), vec![true, false, false], "щелчок — выбрана одна строка");
    assert!(e.title_has(900), "просмотр — выбранная: {}", f.get_preview_title());
    assert_eq!((f.get_copy_label().as_str(), f.get_export_label().as_str()), ("Копировать", "Выгрузить выбранные (1)"));
    assert!(f.get_can_export() && f.get_can_copy(), "выбрана — можно копировать и выгружать");

    e.pick(2, true, false);
    assert_eq!(e.picked(), vec![true, false, true], "Ctrl — строка добавлена");
    assert!(e.title_has(902), "просмотр — только что добавленная: {}", f.get_preview_title());
    assert_eq!((f.get_copy_label().as_str(), f.get_export_label().as_str()), ("Копировать (2)", "Выгрузить выбранные (2)"));

    e.pick(2, true, false);
    assert_eq!(e.picked(), vec![true, false, false], "Ctrl по выбранной — снята");
    assert!(e.title_has(900), "снята — просмотр последней из оставшихся: {}", f.get_preview_title());
    assert_eq!(f.get_copy_label().as_str(), "Копировать");

    e.pick(0, true, false);
    assert_eq!(e.picked(), vec![false; 3], "снята последняя — выбора нет");
    assert_eq!(f.get_preview_title().as_str(), "", "выбора нет — просмотр пуст");
    assert!(!f.get_can_copy() && !f.get_can_export(), "выбора нет — кнопки недоступны");
    assert_eq!(f.get_export_label().as_str(), "Выгрузить выбранные");

    e.pick(0, false, false);
    e.pick(2, false, true);
    assert_eq!(e.picked(), vec![true; 3], "Shift — подряд от прежней вниз");
    assert!(e.title_has(902), "просмотр при Shift — строка, по которой щёлкнули: {}", f.get_preview_title());
    assert_eq!(f.get_export_label().as_str(), "Выгрузить выбранные (3)");

    e.pick(1, false, false);
    assert_eq!(e.picked(), vec![false, true, false], "обычный щелчок оставляет одну");
    e.pick(2, false, false);
    e.pick(0, false, true);
    assert_eq!(e.picked(), vec![true; 3], "Shift — подряд от прежней вверх");
    assert!(e.title_has(900), "просмотр при Shift вверх: {}", f.get_preview_title());
}

/// 2. «Копировать» с двумя выбранными: тексты по порядку строк через пустую строку, итог с числом заявок.
fn copy_of_two_rows(jobs: &Jobs) {
    let e = env("copy2", jobs, 3, true, true);
    let f = &e.app.finder;
    let want = format!("{}\n\n{}", plain(&e.agent_text(900)), plain(&e.agent_text(901)));
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    // просмотр второй строки дочитан: её текст «Копировать» берёт как есть, первая читается — так всегда, а не как повезёт
    e.previews_done(2);
    f.invoke_copy();
    e.wait("текст в буфере", || !e.clip.borrow().is_empty());
    let put = e.clip.borrow()[0].clone();
    assert_eq!(plain(&put), want, "в буфере — два текста для агента через пустую строку");
    let summary = format!("В буфере обмена — заявок: 2, {} символов. Вставьте в чат.", n0(put.encode_utf16().count()));
    e.wait_is("итог копирования", || e.message(), &summary);
    assert_eq!(e.clip.borrow().len(), 1, "в буфер положено один раз");
    // два просмотра и одно чтение первой строки: вторая в просмотре, не менялась — читается один раз, не два
    assert_eq!(e.count("/api/task/"), 3, "карточки: {:?}", e.ids("/api/task/"));
}

/// 2б. Больше тридцати: отказ с текстом C#, работа не начата, в буфере пусто.
fn copy_refuses_more_than_thirty(jobs: &Jobs) {
    let e = env("copy31", jobs, 31, true, true);
    e.find();
    e.pick(0, false, false);
    e.pick(30, false, true);
    assert_eq!(e.picked(), vec![true; 31], "выбрано 31");
    assert_eq!(e.app.finder.get_copy_label().as_str(), "Копировать (31)");
    e.app.finder.invoke_copy();
    assert_eq!(e.message(), "В буфер — не больше 30 заявок за раз: в чат больше не поместится. Для большего — выгрузка файлами.");
    assert!(!e.app.finder.get_working(), "отказ — работа не началась");
    e.pump();
    assert!(e.clip.borrow().is_empty(), "в буфер ничего не положено");
    // просмотры двух щелчков — единственное, что спрашивало сервер; три десятка карточек за копирование — нет
    assert!(e.count("/api/task/") <= 2, "отказ ничего не спрашивает: {}", e.count("/api/task/"));
}

/// 3. Выгрузка выбранных: файлы по месяцам, оглавление, итог; повтор — «без изменений» и без чтения.
fn export_selected_writes_files(jobs: &Jobs) {
    let e = env("export", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    assert_eq!(f.get_folder().as_str(), e.out.display().to_string(), "папка — KnowledgeDir из настроек");
    e.pick(0, false, false);
    e.pick(1, true, false);
    f.invoke_export_selected();
    // окно показывает ход: после записи файлов три секунды идёт проверка, что они на месте (`SettleAsync`)
    e.wait_is("строка хода: проверка записанного", || e.message(), "Проверяю, что файлы на месте…");
    e.wait_is("итог выгрузки", || e.message(), "Готово. Выбрано — 2: новых файлов 2, обновлено 0, без изменений 0.");
    for id in [900, 901] {
        let text = std::fs::read_to_string(e.ticket_file(id)).unwrap_or_else(|er| panic!("файла заявки {id} нет: {er}"));
        assert!(text.contains(&format!("id: {id}\n")), "в свойствах файла — id: {text}");
        assert!(text.contains("\nmy_role: \"исполнитель\"\n"), "кто я — пользователь 7, исполнитель заявки: {text}");
    }
    assert!(!e.ticket_file(902).exists(), "невыбранная не выгружена");
    assert!(e.out.join("tickets").join("2026-10").join("_index.md").exists(), "оглавление месяца записано");
    assert!(!f.get_working(), "выгрузка кончилась — не занят");
    assert!(f.get_can_export(), "и выбранные остались — можно снова");

    let (cards, lives) = (e.count("/api/task/"), e.count("/api/tasklifetime"));
    f.invoke_export_selected();
    e.wait_is("итог повторной выгрузки", || e.message(), "Готово. Выбрано — 2: новых файлов 0, обновлено 0, без изменений 2.");
    assert_eq!((e.count("/api/task/"), e.count("/api/tasklifetime")), (cards, lives), "не менявшиеся заявки не читаются заново");
}

/// 3б. Папка не задана в настройках — «knowledge» рядом с данными.
fn empty_knowledge_dir_is_next_to_the_data(jobs: &Jobs) {
    let e = env("folder", jobs, 3, true, false);
    e.find();
    assert_eq!(e.app.finder.get_folder().as_str(), e.dir.join("knowledge").display().to_string());
}

/// 4. Идущая выгрузка: занято, «Начинаю…»; «Остановить» сразу говорит, а потом итог — остановлено, начатое сохранено.
fn export_working_and_stop(jobs: &Jobs) {
    let e = env("stop", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.previews_done(2); // запросы просмотров — до «держать»: за запрос выгрузки их не принять
    e.world.hold.store(true, Ordering::SeqCst);
    let before = e.world.card_in.load(Ordering::SeqCst);
    f.invoke_export_selected();
    assert!(f.get_working(), "выгрузка идёт — занят");
    assert!(!f.get_can_export() && !f.get_can_copy(), "пока идёт — выгрузка и копирование недоступны");
    assert_eq!(e.message(), "Начинаю…");
    e.wait("запрос карточки от выгрузки ушёл", || e.world.card_in.load(Ordering::SeqCst) > before);
    f.invoke_stop();
    assert_eq!(e.message(), STOPPING, "«Остановить» — сразу сказано");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("выгрузка закончилась", || !f.get_working());
    assert_eq!(
        e.message(),
        "Остановлено — что успели, сохранено (обработано 0 из 2). Запустите выгрузку снова: готовое пропустится, она продолжится с этого места.",
        "итог остановки до первой записи (`Conclude`)"
    );
    assert!(f.get_can_export() && f.get_can_copy(), "после остановки кнопки снова доступны");
}

/// 5. «Кто я» не ответил: выгрузка не начата, файлов нет.
fn export_not_started_without_who_am_i(jobs: &Jobs) {
    let e = env("me-down", jobs, 3, true, true);
    let f = &e.app.finder;
    e.world.me_down.store(true, Ordering::SeqCst);
    e.find();
    e.pick(0, false, false);
    f.invoke_export_selected();
    e.wait("итог: выгрузка не начата", || e.message().starts_with("Выгрузка не начата: ") && !f.get_working());
    assert!(e.message().len() > "Выгрузка не начата: ".len(), "причина названа: {}", e.message());
    assert!(files_under(&e.out).is_empty(), "ни одного файла: {:?}", files_under(&e.out));
}

/// 6. Доска только для чтения: отказ с текстом, папка не создана.
fn read_only_board_refuses_export(jobs: &Jobs) {
    let e = env("ro", jobs, 3, false, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    f.invoke_export_selected();
    e.pump();
    assert_eq!(e.message(), READ_ONLY);
    assert!(!f.get_working(), "отказ — работа не началась");
    assert!(!e.out.exists(), "папка выгрузки не создана");
}

/// 7. Другой клиент во время выгрузки: она останавливается, «Готово» поздние ответы не пишут.
fn client_change_stops_the_export(jobs: &Jobs) {
    let e = env("client-change", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.previews_done(2);
    e.world.hold.store(true, Ordering::SeqCst);
    let before = e.world.card_in.load(Ordering::SeqCst);
    f.invoke_export_selected();
    assert!(f.get_working(), "выгрузка идёт");
    e.wait("запрос карточки от выгрузки ушёл", || e.world.card_in.load(Ordering::SeqCst) > before);
    e.app.use_client(Some(client(&e.url, "other")));
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("выгрузка закончилась", || !f.get_working());
    assert_eq!(
        e.message(),
        "Остановлено — что успели, сохранено (обработано 0 из 2). Запустите выгрузку снова: готовое пропустится, она продолжится с этого места.",
        "итог остановки: чужие поздние ответы его не переписали"
    );
    assert!(!e.ticket_file(900).exists() && !e.ticket_file(901).exists(), "остановили до записи — файлов нет");
}

/// 8. Ctrl+Shift вместе — диапазон от прежней строки ДОБАВЛЯЕТСЯ к выбранным (WPF Extended); один Shift заменяет.
fn ctrl_shift_adds_the_range(jobs: &Jobs) {
    let e = env("ctrl-shift", jobs, 5, true, true);
    e.find();
    e.pick(0, false, false);
    e.pick(2, true, false);
    assert_eq!(e.picked(), vec![true, false, true, false, false]);
    e.pick(4, true, true);
    assert_eq!(e.picked(), vec![true, false, true, true, true], "Ctrl+Shift: диапазон 2..4 добавлен, нулевая осталась");
    assert!(e.title_has(904), "просмотр — строка, по которой щёлкнули: {}", e.app.finder.get_preview_title());
    assert_eq!(e.app.finder.get_export_label().as_str(), "Выгрузить выбранные (4)");
}

/// 9. Ctrl снимает из трёх выбранных одну за другой: просмотр — последняя из оставшихся.
fn ctrl_removal_from_three_rows(jobs: &Jobs) {
    let e = env("ctrl-remove", jobs, 3, true, true);
    e.find();
    e.pick(0, false, false);
    e.pick(2, true, false);
    e.pick(1, true, false);
    assert_eq!(e.picked(), vec![true; 3]);
    assert!(e.title_has(901), "просмотр — добавленная последней: {}", e.app.finder.get_preview_title());
    e.pick(1, true, false);
    assert_eq!(e.picked(), vec![true, false, true], "средняя снята");
    assert!(e.title_has(902), "просмотр — последняя из оставшихся (выбранная раньше второй): {}", e.app.finder.get_preview_title());
}

/// 10. «Копировать» нескольких: одна карточка ответила 404 — текст другой лёг в буфер, итог называет не прочитавшуюся; ход виден.
fn copy_with_one_unreadable(jobs: &Jobs) {
    let e = env("copy-404", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.previews_done(2);
    e.world.gone.store(900, Ordering::SeqCst);
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_copy();
    assert_eq!(e.message(), "Готовлю текст: 0 из 2…", "работа началась");
    // текст второй строки уже в просмотре — он готов сразу, первая ещё читается
    e.wait_is("ход копирования", || e.message(), "Готовлю текст: 1 из 2…");
    assert!(e.clip.borrow().is_empty(), "пока не всё готово, в буфере пусто");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    let put = e.clip.borrow()[0].clone();
    assert_eq!(plain(&put), plain(&e.agent_text(901)), "в буфере — текст прочитавшейся заявки");
    assert_eq!(
        e.message(),
        format!(
            "В буфере обмена — заявок: 1, {} символов. Вставьте в чат.\nНе прочитались: 1. Первая ошибка — #900: заявка не найдена (HTTP 404)",
            n0(put.encode_utf16().count())
        )
    );
}

/// Сколько запросов карточки и истории пришло после остановки сверх тех, что уже шли (`card_ids`): нарушение видно сразу;
/// нарушения нет — ждём до конца срока (это единственная пауза: отсутствие запроса иначе не доказать).
fn extra_requests_after(e: &Env, in_flight: &[i32]) -> Vec<String> {
    let until = Instant::now() + Duration::from_millis(1500);
    loop {
        e.pump();
        let mut extra: Vec<String> = Vec::new();
        for (prefix, ids) in [("карточка", e.ids("/api/task/")), ("история", e.ids("/api/tasklifetime"))] {
            extra.extend(ids.into_iter().filter(|id| !in_flight.contains(id)).map(|id| format!("{prefix} #{id}")));
        }
        if !extra.is_empty() || Instant::now() >= until {
            return extra;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Копирование десяти заявок застряло на первых четырёх; `end` его прерывает. Потом ответы отданы: новых запросов быть не
/// должно, в буфере пусто, сообщение осталось «Остановлено…».
/// `right_after` — сообщение сразу после `end`: «Остановить» дочитывает начатое, смена клиента бросает его сразу.
fn interrupted_copy(jobs: &Jobs, name: &str, right_after: &str, end: impl Fn(&Env)) {
    let e = env(name, jobs, 10, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(9, false, true);
    assert_eq!(e.picked(), vec![true; 10]);
    e.previews_done(2);
    let base = e.world.card_in.load(Ordering::SeqCst);
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_copy();
    // у C# по четыре запроса разом: четыре карточки в пути; одна — уже начало (четвёртая — отдельная проверка), больше
    // секунды четырёх не ждём: нужна только граница «что уже шло» в миг остановки
    e.wait("карточка в пути", || e.world.card_in.load(Ordering::SeqCst) > base);
    let until = Instant::now() + Duration::from_secs(1);
    e.wait("четыре карточки в пути или секунда", || {
        e.world.card_in.load(Ordering::SeqCst) >= base + 4 || Instant::now() >= until
    });
    end(&e);
    assert_eq!(e.message(), right_after);
    let in_flight = e.ids("/api/task/");
    e.world.hold.store(false, Ordering::SeqCst);
    let started = e.world.card_in.load(Ordering::SeqCst);
    e.wait("начатые запросы отвечены", || e.world.card_out.load(Ordering::SeqCst) >= started);
    e.wait("окно не занято", || !e.app.finder.get_working());
    let extra = extra_requests_after(&e, &in_flight);
    assert!(extra.is_empty(), "после остановки к серверу пришло ещё: {extra:?}");
    assert!(e.clip.borrow().is_empty(), "поздние ответы в буфер ничего не положили");
    assert_eq!(e.message(), "Остановлено, в буфер ничего не положено.", "и сообщения не переписали");
}

/// 11. «Остановить» во время копирования: новых запросов к серверу нет.
fn stop_during_copy(jobs: &Jobs) {
    interrupted_copy(jobs, "copy-stop", "Останавливаю — дочитываю начатое…", |e| {
        e.app.finder.invoke_stop()
    });
}

/// 12. Другой клиент во время копирования: то же.
fn client_change_during_copy(jobs: &Jobs) {
    interrupted_copy(jobs, "copy-client", "Остановлено, в буфер ничего не положено.", |e| {
        e.app.use_client(Some(client(&e.url, "other")))
    });
}

/// 12б. Копирование читает по четыре заявки разом, как C# (`MaxParallel`): четыре запроса карточки приходят вместе.
fn copy_reads_four_at_once(jobs: &Jobs) {
    let e = env("copy-four", jobs, 10, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(9, false, true);
    e.previews_done(2);
    let base = e.world.card_in.load(Ordering::SeqCst);
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_copy();
    e.wait("четыре карточки в пути разом", || e.world.card_in.load(Ordering::SeqCst) >= base + 4);
    assert_eq!(e.world.card_in.load(Ordering::SeqCst), base + 4, "и не больше четырёх");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    assert_eq!(e.clip.borrow().len(), 1);
}

/// 13. Папка поправлена руками: выгрузка — туда, правка переживает «спрятать и показать снова».
fn edited_folder_is_used(jobs: &Jobs) {
    let e = env("folder-edit", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    let other = e.dir.join("other-base");
    f.set_folder(other.display().to_string().into());
    f.hide().unwrap();
    e.app.window.invoke_open_search();
    assert_eq!(f.get_folder().as_str(), other.display().to_string(), "правка папки пережила «спрятать и показать»");
    // быстрый поиск спрятан — окно открылось с условиями последнего поиска человека и пустым списком: ищем заново
    e.find();
    e.pick(0, false, false);
    f.invoke_export_selected();
    e.wait_is("итог выгрузки", || e.message(), "Готово. Выбрано — 1: новых файлов 1, обновлено 0, без изменений 0.");
    assert!(e.ticket_file_in(&other, 900).is_file(), "файл — в поправленной папке: {:?}", files_under(&e.dir));
    assert!(!e.out.exists(), "папка из настроек не тронута");
}

/// 14. Папка — не полный путь: отказ текстом C#, ничего не начато.
fn relative_folder_is_refused(jobs: &Jobs) {
    let e = env("folder-relative", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    f.set_folder("база/заявки".into());
    e.pick(0, false, false);
    let cards = e.count("/api/task/");
    f.invoke_export_selected();
    e.pump();
    assert_eq!(e.message(), "Папка — полный путь, например D:\\Obsidian\\База\\Заявки");
    assert!(!f.get_working(), "отказ — работа не началась");
    assert_eq!(e.count("/api/task/"), cards, "ничего не спрашивали");
}

/// 15. Папка — на самом деле файл: «Не удалось записать в папку {dir}:» (`Failure`), файлов заявок нет.
fn folder_that_is_a_file(jobs: &Jobs) {
    let e = env("folder-file", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    let file = e.dir.join("я-файл");
    std::fs::write(&file, "не папка").unwrap();
    f.set_folder(file.display().to_string().into());
    e.pick(0, false, false);
    f.invoke_export_selected();
    e.wait("итог", || !f.get_working());
    let prefix = format!("Не удалось записать в папку {}:\n", file.display());
    assert!(e.message().starts_with(&prefix) && e.message().len() > prefix.len(), "итог: {}", e.message());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "не папка", "файл на месте папки не тронут");
}

// 16. Смена клиента во время выгрузки: поздний ответ старого сервера не возвращает прежнего «кто я» — следующая выгрузка
// на новом клиенте пишет роль для ЕГО пользователя (исполнитель заявок — пользователь 7 первого сервера, у второго — 8).
fn the_old_user_does_not_come_back_after_a_client_change(jobs: &Jobs) {
    let e = env("me-after-change", jobs, 3, true, true);
    let f = &e.app.finder;
    let second = world(3, 8, "Другой Человек");
    let w = second.clone();
    let (url2, _) = serve(move |p| w.answer(p, true));
    e.find();
    e.pick(0, false, false);
    e.previews_done(1);
    e.world.hold.store(true, Ordering::SeqCst);
    let before = e.world.card_in.load(Ordering::SeqCst);
    f.invoke_export_selected();
    e.wait("запрос карточки от выгрузки ушёл", || e.world.card_in.load(Ordering::SeqCst) > before);
    e.app.use_client(Some(client(&url2, "other")));
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("старая выгрузка закончилась", || !f.get_working());
    // окно очищено сменой клиента: ищем заново, уже на втором сервере
    e.find();
    e.pick(0, false, false);
    f.invoke_export_selected();
    e.wait_is("итог выгрузки на новом сервере", || e.message(), "Готово. Выбрано — 1: новых файлов 1, обновлено 0, без изменений 0.");
    let text = std::fs::read_to_string(e.ticket_file(900)).unwrap();
    assert!(!text.contains("\nmy_role: \"исполнитель\"\n"), "роль — прежнего пользователя (он исполнитель), а не нового:\n{text}");
}

/// 17. Доска только для чтения: «Открыть» не создаёт папки выгрузки.
fn open_folder_on_a_read_only_board_creates_nothing(jobs: &Jobs) {
    let e = env("open-ro", jobs, 3, false, true);
    e.find();
    assert_eq!(e.app.finder.get_folder().as_str(), e.out.display().to_string());
    e.app.finder.invoke_open_folder();
    e.pump();
    assert!(!e.out.exists(), "прототип в этом режиме ничего не пишет — и папок не создаёт");
}

/// 18. Своя доска: «Открыть» создаёт папку, если её нет (`OpenFolder`).
fn open_folder_creates_it_for_the_owner(jobs: &Jobs) {
    let e = env("open-owner", jobs, 3, true, true);
    e.find();
    e.app.finder.invoke_open_folder();
    e.pump();
    assert!(e.out.is_dir(), "папки не было — создана");
}

// ---------- rust-0.9.1: находки проверки на рабочем ПК ----------

/// Часть пути кончается пробелом («…\\папка \\внутри»): Windows обрезает пробел только у последней части пути — создаст
/// «папка», а путь через «папка » не найдёт («The system cannot find the path specified»). Окно говорит, что поправить,
/// и ничего не начинает и не создаёт.
fn folder_part_ending_in_a_space_is_refused(jobs: &Jobs) {
    let e = env("folder-space", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    let bad = e.dir.join("папка ").join("внутри");
    f.set_folder(bad.display().to_string().into());
    e.pick(0, false, false);
    let cards = e.count("/api/task/");
    f.invoke_export_selected();
    e.pump();
    assert_eq!(e.message(), "Папка «папка » в пути кончается пробелом или точкой — Windows её не найдёт. Уберите их из поля.");
    assert!(!f.get_working() && e.count("/api/task/") == cards, "отказ — ничего не спрашивали");
    assert!(!e.dir.join("папка ").exists(), "ничего не создано");
    // «Открыть» — то же: не создаёт, говорит
    f.invoke_open_folder();
    e.pump();
    assert_eq!(e.message(), "Папка «папка » в пути кончается пробелом или точкой — Windows её не найдёт. Уберите их из поля.");
    assert!(!e.dir.join("папка ").exists(), "«Открыть» ничего не создало");
}

// ---------- итоговая карточка 4.2: просмотр, «Остановить» при копировании, порядок ошибок, защита от двойного запуска ----------

const COPY_STOPPING: &str = "Останавливаю — дочитываю начатое…";
const COPY_STOPPED: &str = "Остановлено, в буфер ничего не положено.";

impl Env {
    /// Ответы потоков по одному, а после каждого — сообщение окна: промежуточное не пропадёт в пачке, которую `pump` берёт разом.
    #[track_caller]
    fn watch(&self, what: &str, done: impl Fn() -> bool) -> Vec<String> {
        let until = Instant::now() + Duration::from_secs(8);
        let mut seen = Vec::new();
        loop {
            let pending = std::mem::take(&mut *self.jobs.lock().unwrap());
            for job in pending {
                job();
                seen.push(self.message());
            }
            if done() {
                return seen;
            }
            assert!(Instant::now() < until, "не дождались: {what}");
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
    }

    /// Ответов потоков в очереди окна набралось не меньше `n` (их не выполняя) или вышел срок: потоки, что не пришлют ответ
    /// вовсе, не повисят проверку, а пришедшие до срока проверку не обманут.
    fn queued(&self, n: usize, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        while self.jobs.lock().unwrap().len() < n {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

/// 19. Просмотр строки не прочитался (сервер ответил ошибкой): щелчок по ней же читает её снова — и текст появляется.
fn failed_preview_is_read_again_on_the_same_row(jobs: &Jobs) {
    let e = env("preview-retry", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.world.gone.store(900, Ordering::SeqCst);
    e.pick(0, false, false);
    e.wait("просмотр не прочитался", || {
        !f.get_preview_busy() && f.get_preview_text().starts_with("Не удалось прочитать заявку")
    });
    assert_eq!(e.count("/api/task/900"), 1);
    e.world.gone.store(0, Ordering::SeqCst); // сервер поправился
    e.pick(0, false, false);
    assert!(f.get_preview_busy(), "щелчок по строке с неудавшимся просмотром читает её заново");
    e.wait("просмотр прочитан", || !f.get_preview_busy());
    assert_eq!(e.count("/api/task/900"), 2, "новый запрос про эту заявку дошёл до сервера");
    assert_eq!(plain(f.get_preview_text().as_str()), plain(&e.agent_text(900)), "текст появился");
}

/// 20. Просмотр строки уже показан: щелчок по ней же запроса не делает (только щелчок по другой читает).
fn shown_preview_is_not_read_again(jobs: &Jobs) {
    let e = env("preview-same", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.wait("просмотр прочитан", || !f.get_preview_busy() && !f.get_preview_text().is_empty());
    assert_eq!(e.count("/api/task/900"), 1);
    e.pick(0, false, false);
    assert!(!f.get_preview_busy(), "та же строка, текст на месте: читать заново не нужно");
    // метка: запрос по другой строке дошёл бы после повторного запроса про эту
    e.pick(1, false, false);
    e.wait("просмотр второй строки", || !f.get_preview_busy() && e.count("/api/task/901") == 1);
    assert_eq!(e.count("/api/task/900"), 1, "повторного запроса про 900 не было");
}

/// Десять заявок: выбраны все, просмотр — последняя (её текст готов); четыре чтения — в пути, все держатся.
fn held_copy(jobs: &Jobs, name: &str) -> Env {
    let e = env(name, jobs, 10, true, true);
    e.find();
    e.pick(0, false, false);
    e.pick(9, false, true);
    e.previews_done(2);
    let base = e.world.card_in.load(Ordering::SeqCst);
    e.world.hold.store(true, Ordering::SeqCst);
    e.app.finder.invoke_copy();
    e.wait("четыре карточки в пути", || e.world.card_in.load(Ordering::SeqCst) >= base + 4);
    e
}

/// 21. «Остановить» при копировании: окно остаётся занятым, пока начатые чтения не вернулись; хода больше нет; потом — итог, буфер пуст.
fn stop_during_copy_waits_for_the_started_reads(jobs: &Jobs) {
    let e = held_copy(jobs, "copy-stop-wait");
    let f = &e.app.finder;
    f.invoke_stop();
    assert_eq!(e.message(), COPY_STOPPING, "«Остановить» — сразу сказано, что дочитываем начатое");
    assert!(f.get_working() && !f.get_can_copy() && !f.get_can_export(), "начатые чтения ещё идут — окно занято");
    e.world.hold.store(false, Ordering::SeqCst);
    let seen = e.watch("копирование кончилось", || !f.get_working());
    assert!(seen.iter().all(|m| !m.starts_with("Готовлю текст")), "после «Остановить» хода нет: {seen:?}");
    assert_eq!(e.message(), COPY_STOPPED);
    assert!(f.get_can_copy() && f.get_can_export(), "прочитано — кнопки снова доступны");
    assert!(e.clip.borrow().is_empty(), "в буфер обмена не обращались");
}

/// 22. Две заявки не прочитались: первая ошибка — той, что кончила первой (у C# `errors.Add` по завершении), а не по порядку строк.
fn first_copy_error_is_the_first_to_finish(jobs: &Jobs) {
    let e = env("copy-err-order", jobs, 3, true, true);
    let f = &e.app.finder;
    e.world.gone.store(900, Ordering::SeqCst);
    e.world.gone2.store(901, Ordering::SeqCst);
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.wait("просмотры отвечены", || e.world.card_out.load(Ordering::SeqCst) >= 2 && !f.get_preview_busy());
    e.world.slow_id.store(900, Ordering::SeqCst); // первая строка отвечает поздно
    let base = e.world.card_in.load(Ordering::SeqCst);
    f.invoke_copy();
    e.wait("оба чтения начались", || e.world.card_in.load(Ordering::SeqCst) >= base + 2);
    e.wait_is("вторая строка уже не прочиталась", || e.message(), "Готовлю текст: 1 из 2…");
    e.world.slow_open.store(true, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    assert_eq!(e.message(), "Ничего не скопировано.\nНе прочитались: 2. Первая ошибка — #901: заявка не найдена (HTTP 404)");
    assert!(e.clip.borrow().is_empty());
}

/// 23. Тексты в буфере — в порядке строк, как бы ни кончили чтения (первая строка отвечает позже второй).
fn copied_texts_stay_in_row_order(jobs: &Jobs) {
    let e = env("copy-order", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.wait("просмотры отвечены", || {
        e.world.card_out.load(Ordering::SeqCst) >= 2 && !f.get_preview_busy() && !f.get_preview_text().is_empty()
    });
    e.world.slow_id.store(900, Ordering::SeqCst);
    f.invoke_copy();
    e.wait_is("вторая строка (она в просмотре) готова", || e.message(), "Готовлю текст: 1 из 2…");
    e.world.slow_open.store(true, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    let put = e.clip.borrow().clone();
    assert_eq!(put.len(), 1);
    assert_eq!(
        plain(&put[0]),
        plain(&format!("{}\n\n{}", e.agent_text(900), e.agent_text(901))),
        "первая строка — первой, хоть и кончила позже"
    );
}

/// 24. Копирование идёт: «Копировать» ещё раз (обработчик, хоть кнопка и выключена) ничего не начинает заново.
fn copy_is_refused_while_copying(jobs: &Jobs) {
    let e = held_copy(jobs, "copy-twice");
    let f = &e.app.finder;
    e.wait_is("готова строка из просмотра", || e.message(), "Готовлю текст: 1 из 10…");
    f.invoke_copy();
    assert_eq!(e.message(), "Готовлю текст: 1 из 10…", "второй запуск не обнулил ход");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    assert_eq!(e.clip.borrow().len(), 1, "в буфер — один раз");
}

/// 25. Выгрузка идёт: «Копировать» (обработчик напрямую) не начинается, и в буфер ничего не кладётся.
fn copy_is_refused_while_exporting(jobs: &Jobs) {
    let e = env("copy-in-export", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.previews_done(2);
    let base = e.world.card_in.load(Ordering::SeqCst);
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_export_selected();
    e.wait("карточка от выгрузки ушла", || e.world.card_in.load(Ordering::SeqCst) > base);
    f.invoke_copy();
    assert_eq!(e.message(), "Начинаю…", "копирование при выгрузке не начато");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("выгрузка кончилась", || !f.get_working());
    assert!(e.message().starts_with("Готово. Выбрано — 2"), "{}", e.message());
    assert!(e.clip.borrow().is_empty(), "копирование не шло — в буфере пусто");
}

/// 26. Копирование идёт: «Выгрузить выбранные» (обработчик напрямую) не начинается.
fn export_is_refused_while_copying(jobs: &Jobs) {
    let e = held_copy(jobs, "export-in-copy");
    let f = &e.app.finder;
    e.wait_is("готова строка из просмотра", || e.message(), "Готовлю текст: 1 из 10…");
    f.invoke_export_selected();
    assert_eq!(e.message(), "Готовлю текст: 1 из 10…", "выгрузка при копировании не начата");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    assert!(!e.out.exists(), "ничего не выгружалось");
}

/// 27. Ничего не выбрано: ни «Копировать», ни «Выгрузить выбранные» (обработчики напрямую) ничего не начинают и не говорят.
fn nothing_picked_starts_nothing(jobs: &Jobs) {
    let e = env("nothing-picked", jobs, 3, true, true);
    let f = &e.app.finder;
    e.find();
    assert_eq!(e.picked(), vec![false; 3]);
    f.invoke_copy();
    assert_eq!(e.message(), "", "копировать нечего — тишина");
    f.invoke_export_selected();
    assert_eq!(e.message(), "", "выгружать нечего — тишина");
    assert!(!f.get_working());
    e.pump();
    assert!(e.clip.borrow().is_empty() && !e.out.exists());
    assert_eq!(e.count("/api/task/"), 0, "ни одного запроса карточки");
}

/// 28. Ровно тридцать заявок — ещё можно копировать (отказ — от тридцати одной).
fn exactly_thirty_rows_are_copied(jobs: &Jobs) {
    let e = env("copy30", jobs, 30, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(29, false, true);
    assert_eq!(e.picked(), vec![true; 30]);
    e.previews_done(2);
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_copy();
    assert_eq!(e.message(), "Готовлю текст: 0 из 30…", "тридцать — не отказ");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("копирование кончилось", || !f.get_working());
    assert!(e.message().starts_with("В буфере обмена — заявок: 30, "), "{}", e.message());
}

/// 29. Копирование прежнего клиента ещё не вернулось, а на новом уже идёт своё: поздний ход прежнего чужое не переписывает.
fn stale_copy_progress_does_not_overwrite_the_new_copy(jobs: &Jobs) {
    let e = env("copy-stale-progress", jobs, 10, true, true);
    let f = &e.app.finder;
    let second = world(10, 8, "Другой Человек");
    let w = second.clone();
    let (url2, _) = serve(move |p| w.answer(p, true));
    e.find();
    e.pick(0, false, false);
    e.pick(3, false, true);
    e.previews_done(2);
    let base = e.world.card_in.load(Ordering::SeqCst);
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_copy(); // четыре строки: последняя из просмотра готова, три читаются и держатся
    e.wait("три карточки в пути", || e.world.card_in.load(Ordering::SeqCst) >= base + 3);
    e.wait_is("готова строка из просмотра", || e.message(), "Готовлю текст: 1 из 4…");
    e.app.use_client(Some(client(&url2, "other")));
    e.find_on(&second);
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.wait("просмотры нового сервера", || second.card_out.load(Ordering::SeqCst) >= 2 && !f.get_preview_busy());
    second.hold.store(true, Ordering::SeqCst);
    f.invoke_copy(); // две строки: вторая из просмотра готова, первая держится
    e.wait_is("ход нового копирования", || e.message(), "Готовлю текст: 1 из 2…");
    assert!(e.queued(0, Duration::ZERO));
    e.world.hold.store(false, Ordering::SeqCst); // прежние чтения возвращаются и шлют свой ход и итог
    e.queued(4, Duration::from_secs(3));
    e.pump();
    assert_eq!(e.message(), "Готовлю текст: 1 из 2…", "ход прежнего копирования новое не переписал");
    second.hold.store(false, Ordering::SeqCst);
    e.wait("новое копирование кончилось", || !f.get_working());
    assert_eq!(e.clip.borrow().len(), 1, "в буфер — только текст нового");
}

/// 30. «Остановить» выгрузку: ход, отправленный до остановки, но принятый после неё, окно не показывает — итог скажет, что успели.
fn export_progress_after_stop_is_not_shown(jobs: &Jobs) {
    let e = env("export-progress-stop", jobs, 2, true, true);
    let f = &e.app.finder;
    e.find();
    e.pick(0, false, false);
    e.pick(1, true, false);
    e.previews_done(2);
    e.world.slow_id.store(900, Ordering::SeqCst); // эта держится до конца
    e.world.hold.store(true, Ordering::SeqCst); // эта — пока не пройдёт четверть секунды (ход чаще не идёт)
    let (before, started) = (e.world.card_in.load(Ordering::SeqCst), Instant::now());
    f.invoke_export_selected();
    e.wait("обе карточки от выгрузки ушли", || e.world.card_in.load(Ordering::SeqCst) >= before + 2);
    e.wait("четверть секунды выгрузки прошла", || started.elapsed() >= Duration::from_millis(400));
    e.world.hold.store(false, Ordering::SeqCst); // 901 кончилась — ход «Выгружено 1 из 2» отправлен потоком
    assert!(e.queued(1, Duration::from_secs(5)), "ход выгрузки не пришёл");
    f.invoke_stop(); // до того, как окно принято его читать
    assert_eq!(e.message(), STOPPING);
    e.pump();
    assert_eq!(e.message(), STOPPING, "ход после «Остановить» не показан");
    e.world.slow_open.store(true, Ordering::SeqCst);
    e.wait("выгрузка кончилась", || !f.get_working());
    assert!(e.message().starts_with("Не закончено. Выбрано, обработано — "), "{}", e.message());
}

#[test]
fn the_search_window_picks_copies_and_exports_rows() {
    let (_windows, jobs) = common::window::install();
    let scenarios: Vec<Scenario> = vec![
        ("выбор строк: щелчок, Ctrl, Shift", picking_rows),
        ("«Копировать (2)» — два текста", copy_of_two_rows),
        ("«Копировать» — больше тридцати отказ", copy_refuses_more_than_thirty),
        ("выгрузка выбранных — файлы и повтор", export_selected_writes_files),
        ("пустая папка — knowledge рядом с данными", empty_knowledge_dir_is_next_to_the_data),
        ("выгрузка идёт и останавливается", export_working_and_stop),
        ("«кто я» не ответил — выгрузка не начата", export_not_started_without_who_am_i),
        ("доска только для чтения — отказ", read_only_board_refuses_export),
        ("смена клиента останавливает выгрузку", client_change_stops_the_export),
        ("Ctrl+Shift добавляет диапазон", ctrl_shift_adds_the_range),
        ("Ctrl снимает из трёх выбранных", ctrl_removal_from_three_rows),
        ("копирование: одна заявка не прочиталась", copy_with_one_unreadable),
        ("«Остановить» во время копирования", stop_during_copy),
        ("смена клиента во время копирования", client_change_during_copy),
        ("копирование — по четыре разом", copy_reads_four_at_once),
        ("папка, поправленная руками", edited_folder_is_used),
        ("папка — не полный путь", relative_folder_is_refused),
        ("папка — файл", folder_that_is_a_file),
        ("после смены клиента «кто я» — нового", the_old_user_does_not_come_back_after_a_client_change),
        ("«Открыть» на доске только для чтения", open_folder_on_a_read_only_board_creates_nothing),
        ("«Открыть» у хозяина создаёт папку", open_folder_creates_it_for_the_owner),
        ("просмотр, что не прочитался, читается снова", failed_preview_is_read_again_on_the_same_row),
        ("показанный просмотр не читается снова", shown_preview_is_not_read_again),
        ("«Остановить» при копировании дожидается начатого", stop_during_copy_waits_for_the_started_reads),
        ("первая ошибка копирования — кто кончил первым", first_copy_error_is_the_first_to_finish),
        ("тексты в буфере — в порядке строк", copied_texts_stay_in_row_order),
        ("копирование при копировании", copy_is_refused_while_copying),
        ("копирование при выгрузке", copy_is_refused_while_exporting),
        ("выгрузка при копировании", export_is_refused_while_copying),
        ("ничего не выбрано", nothing_picked_starts_nothing),
        ("ровно тридцать заявок копируются", exactly_thirty_rows_are_copied),
        ("поздний ход прежнего копирования", stale_copy_progress_does_not_overwrite_the_new_copy),
        ("ход выгрузки после «Остановить»", export_progress_after_stop_is_not_shown),
        ("часть пути кончается пробелом", folder_part_ending_in_a_space_is_refused),
    ];
    let mut failed = Vec::new();
    for (name, run) in scenarios {
        if catch_unwind(AssertUnwindSafe(|| run(&jobs))).is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "упали сценарии: {failed:?}");
}
