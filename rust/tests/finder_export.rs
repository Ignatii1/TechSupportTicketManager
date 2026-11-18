//! Окно поиска: несколько выбранных строк, «Копировать (N)» и «Выгрузить выбранные (N)» (`SearchViewModel.SetSelection`,
//! `Copy`, `ExportSelected`, `Summary`, `Stop`, `ApplySettings`; `KnowledgeExport.Starting`/`Conclude`; карточка 4.2-3).
//! Ожидаемые тексты — из C#: подписи, отказы и итоги. Файлы проверяются на диске (`tickets/<год-месяц>/<номер> — <название>.md`),
//! движок выгрузки здесь не вызывается. Платформа Slint ставится один раз на процесс, поэтому проверка одна; каждый сценарий —
//! со своим окном, своим сервером и под `catch_unwind`: упавший не прячет следующий.

mod common;

use chrono::DateTime;
use common::Reply;
use common::window::Jobs;
use slint::Model;
use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
const READ_ONLY: &str = "Доска только для чтения: выгрузка в файлы — только когда доску ведёт прототип (он ничего не пишет, пока её держит TicketBoard.exe).";
const STOPPING: &str = "Останавливаю — дописываю начатое…";

/// Что делает сервер сейчас: переключатели сценария.
struct World {
    /// Карточки не отвечают, пока флаг поднят (только окну; расчёт ожидаемого текста идёт мимо).
    hold: AtomicBool,
    me_down: AtomicBool,
    /// Сколько заявок в списке (номера с 900).
    rows: AtomicUsize,
    /// Запросов карточки от окна пришло.
    card_in: AtomicUsize,
}

impl World {
    fn list(&self) -> String {
        let n = self.rows.load(Ordering::SeqCst) as i32;
        let row = |id: i32| {
            format!(
                r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":31,"Creator":"Петров Пётр","Created":"2026-10-14T11:00:00+03:00","Changed":"{CHANGED}","Description":"Описание заявки {id}."}}"#
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
                Reply::json(r#"{"Id":7,"Name":"Я Сам"}"#)
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
fn env(name: &str, jobs: &Jobs, rows: usize, owner: bool, with_out: bool) -> Env {
    let world = Arc::new(World {
        hold: AtomicBool::new(false),
        me_down: AtomicBool::new(false),
        rows: AtomicUsize::new(rows),
        card_in: AtomicUsize::new(0),
    });
    let w = world.clone();
    let (url, seen) = serve(move |p| w.answer(p, true));
    let w = world.clone();
    let (oracle_url, _) = serve(move |p| w.answer(p, false));
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("my-base");
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    let knowledge = if with_out { format!(r#""KnowledgeDir": "{}", "#, out.display()) } else { String::new() };
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
    f.invoke_copy();
    e.wait("текст в буфере", || !e.clip.borrow().is_empty());
    let put = e.clip.borrow()[0].clone();
    assert_eq!(plain(&put), want, "в буфере — два текста для агента через пустую строку");
    let summary = format!("В буфере обмена — заявок: 2, {} символов. Вставьте в чат.", n0(put.encode_utf16().count()));
    e.wait_is("итог копирования", || e.message(), &summary);
    assert_eq!(e.clip.borrow().len(), 1, "в буфер положено один раз");
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
    assert_eq!(
        e.message(),
        "В буфер — не больше 30 заявок за раз: в чат больше не поместится. Для большего — выгрузка файлами."
    );
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
    e.wait_is("итог выгрузки", || e.message(), "Готово. Выбрано — 2: новых файлов 2, обновлено 0, без изменений 0.");
    for id in [900, 901] {
        let text = std::fs::read_to_string(e.ticket_file(id)).unwrap_or_else(|er| panic!("файла заявки {id} нет: {er}"));
        assert!(text.contains(&format!("id: {id}\n")), "в свойствах файла — id: {text}");
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
    let end = e.message();
    assert!(end.starts_with("Не закончено") || end.contains("Остановлено — что успели, сохранено"), "итог остановки: {end}");
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
    e.world.hold.store(true, Ordering::SeqCst);
    let before = e.world.card_in.load(Ordering::SeqCst);
    f.invoke_export_selected();
    assert!(f.get_working(), "выгрузка идёт");
    e.wait("запрос карточки от выгрузки ушёл", || e.world.card_in.load(Ordering::SeqCst) > before);
    e.app.use_client(Some(client(&e.url, "other")));
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("выгрузка закончилась", || !f.get_working());
    assert!(!e.message().starts_with("Готово"), "чужие поздние ответы итог не переписали: {}", e.message());
    assert!(!e.message().is_empty(), "итог сказан");
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
    ];
    let mut failed = Vec::new();
    for (name, run) in scenarios {
        if catch_unwind(AssertUnwindSafe(|| run(&jobs))).is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "упали сценарии: {failed:?}");
}
