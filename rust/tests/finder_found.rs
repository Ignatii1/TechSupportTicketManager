//! Окно поиска: «Не больше, заявок», «Выгрузить найденные (…)», вопрос перед большой выгрузкой и итог «По отбору»
//! (`SearchViewModel.ExportFoundLabelOf`, `LimitOf`, `ExportCount`, `ConfirmTitle`/`ConfirmText`, `ExportFound`; карточка 4.3-3).
//! Ожидаемые тексты — из C#: таблицы сверки (`export-found-label.tsv`, `export-found-confirm.tsv`, ответы пишет код C#) и
//! тексты `ExportFound`. Платформа Slint ставится один раз на процесс, поэтому оконная проверка одна; каждый сценарий — со
//! своим окном, своим сервером и под `catch_unwind`: упавший не прячет следующий. Сценарии с выгрузкой нуждаются в движке
//! `export::run::export_found` (карточка 4.3-2) — они названы «движок».

mod common;

use common::Reply;
use common::window::Jobs;
use slint::Model;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ticketboard::app::App;
use ticketboard::intraservice::Client;
use ticketboard::search::*;

// ---------- таблицы и чистые правила ----------

fn int(s: &str) -> i32 {
    s.parse().unwrap()
}

#[test]
fn the_found_label_agrees_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-found-label", |c| found_label(int(&c[0]), &c[1]));
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn the_confirm_question_agrees_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-found-confirm", |c| {
        let (found, limit) = (int(&c[0]), int(&c[1]));
        format!("{}\n---\n{}", confirm_title(found, limit), confirm_text(found, limit, &c[2]))
    });
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn the_limit_field_is_digits_only() {
    // `int.TryParse(Trim(), NumberStyles.None)`: пробелы по краям срезаны, знак и буквы — не число, не влезло в int — не число
    for (text, want) in [
        ("500", 500),
        (" 50 ", 50),
        ("0", 0),
        ("007", 7),
        ("+3", -1),
        ("-1", -1),
        ("abc", -1),
        ("", -1),
        ("   ", -1),
        ("1 000", -1),
        ("2147483647", 2147483647),
        ("2147483648", -1),
        ("99999999999", -1),
    ] {
        assert_eq!(limit_of(text), want, "поле «{text}»");
    }
}

#[test]
fn the_question_threshold_and_default_are_the_csharp_ones() {
    assert_eq!(CONFIRM_FROM, 2000, "спрашивать — от двух тысяч заявок (`ConfirmFrom`)");
    assert_eq!(DEFAULT_LIMIT, "500", "в поле сначала «500»");
}

#[test]
fn how_many_will_be_exported() {
    // найдено точно: 0 — все найденные, иначе не больше найденного
    assert_eq!(export_count(261, 0), Some(261));
    assert_eq!(export_count(261, 100), Some(100));
    assert_eq!(export_count(261, 500), Some(261));
    assert_eq!(export_count(261, 261), Some(261));
    // найдено «1 000+» (сервер досчитал до потолка): известно, только если взять задано не больше потолка
    assert_eq!(export_count(1000, 0), None);
    assert_eq!(export_count(1000, 500), Some(500));
    assert_eq!(export_count(1000, 1000), Some(1000));
    assert_eq!(export_count(1000, 5000), None);
}

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
const BAD_LIMIT: &str = "«Не больше, заявок» — число заявок или 0, если нужны все";
const CANCELLED: &str = "Выгрузка отменена.";
/// Папка в строках таблицы вопроса: в ответах C# она стоит ровно так.
const TABLE_DIR: &str = "D:\\Obsidian\\База";

/// Что делает сервер сейчас: переключатели сценария.
struct World {
    /// Карточки не отвечают, пока флаг поднят.
    hold: AtomicBool,
    me_down: AtomicBool,
    /// Сколько заявок в списке (номера с 900).
    rows: AtomicUsize,
    /// Сколько сервер называет найденным (`Paginator.Count`); 0 — столько, сколько строк.
    count: AtomicI32,
    /// Запросов карточки пришло.
    card_in: AtomicUsize,
}

impl World {
    fn list(&self) -> String {
        let n = self.rows.load(Ordering::SeqCst) as i32;
        let count = match self.count.load(Ordering::SeqCst) {
            0 => n,
            c => c,
        };
        let row = |id: i32| {
            format!(
                r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":31,"Creator":"Петров Пётр","Created":"2026-10-14T11:00:00+03:00","Changed":"{CHANGED}","ExecutorIds":"7","Description":"Описание заявки {id}."}}"#
            )
        };
        let rows: Vec<String> = (900..900 + n).map(row).collect();
        format!(
            r#"{{"Tasks":[{}],"Statuses":[{{"Id":31,"Name":"Открыта"}}],"Paginator":{{"Count":{count},"Page":1,"PageCount":1}}}}"#,
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

    fn answer(&self, path: &str) -> Reply {
        if path == "/api/user?getcurrentuserinfo=true" {
            return if self.me_down.load(Ordering::SeqCst) {
                Reply::new(503, "text/plain", "")
            } else {
                Reply::json(r#"{"Id":7,"Name":"Я Сам"}"#)
            };
        }
        if let Some(rest) = path.strip_prefix("/api/task/") {
            let id: i32 = rest.split('?').next().unwrap_or("").parse().unwrap_or(0);
            self.card_in.fetch_add(1, Ordering::SeqCst);
            let until = Instant::now() + Duration::from_secs(20);
            while self.hold.load(Ordering::SeqCst) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
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
    dir: PathBuf,
    /// Папка выгрузки из settings.json «KnowledgeDir».
    out: PathBuf,
}

/// `rows` — заявок в списке, `count` — сколько сервер называет найденным (0 — столько же), `owner` — доска своя.
fn env(name: &str, jobs: &Jobs, rows: usize, count: i32, owner: bool) -> Env {
    let world = Arc::new(World {
        hold: AtomicBool::new(false),
        me_down: AtomicBool::new(false),
        rows: AtomicUsize::new(rows),
        count: AtomicI32::new(count),
        card_in: AtomicUsize::new(0),
    });
    let w = world.clone();
    let (url, seen) = serve(move |p| w.answer(p));
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-found-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("my-base");
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    // путь — строкой JSON: обратные черты Windows без экранирования ломают весь файл
    let knowledge = format!(r#""KnowledgeDir": {}, "#, serde_json::to_string(&out.display().to_string()).unwrap());
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, {knowledge}"IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();
    let app = App::open(&dir, owner).unwrap();
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));
    Env { app, jobs: jobs.clone(), world, seen, dir, out }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
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

    /// Быстрый поиск с доски: список из всех строк мира на месте.
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

    /// Поле «Не больше, заявок» набрано: текст в поле и сообщение окну.
    fn type_limit(&self, text: &str) {
        self.app.finder.set_limit(text.into());
        self.app.finder.invoke_limit_edited(text.into());
    }

    fn ticket_file(&self, id: i32) -> PathBuf {
        self.out.join("tickets").join("2026-10").join(format!("{id} — Заявка {id}.md"))
    }

    /// Вопрос по таблице C# для найденного и «не больше»: заголовок и текст, папка таблицы заменена папкой окна.
    fn asked_by_csharp(&self, found: i32, limit: i32) -> (String, String) {
        let rows = common::parity("export-found-confirm");
        let (_, cells) = rows
            .iter()
            .find(|(_, c)| c[0] == found.to_string() && c[1] == limit.to_string() && c[2] == TABLE_DIR)
            .unwrap_or_else(|| panic!("в таблице нет строки ({found}, {limit})"));
        let (title, text) = cells[3].split_once("\n---\n").expect("в ответе — заголовок и текст");
        (title.to_string(), text.replace(TABLE_DIR, &self.out.display().to_string()))
    }

    /// Выгрузку остановили и дождались конца (сервер держал карточки).
    #[track_caller]
    fn stop_and_settle(&self) {
        self.app.finder.invoke_stop();
        self.world.hold.store(false, Ordering::SeqCst);
        self.wait("выгрузка закончилась", || !self.app.finder.get_working());
    }
}

type Scenario<'a> = (&'a str, fn(&Jobs));

// ---------- сценарии ----------

/// 1. Подпись и доступность кнопки: до поиска — без числа и выключена; после — по найденному и полю; условия поменяли — выключена.
fn label_follows_the_total_and_the_field(jobs: &Jobs) {
    let e = env("label", jobs, 3, 261, true);
    let f = &e.app.finder;
    assert_eq!(f.get_limit().as_str(), "500", "в поле сначала «500»");
    assert_eq!(f.get_found_label().as_str(), "Выгрузить найденные", "до поиска — без числа");
    assert!(!f.get_can_export_found(), "до поиска выгружать нечего");
    e.find();
    assert_eq!(f.get_found_label().as_str(), "Выгрузить найденные (261)", "найдено 261, взять до 500 — все 261");
    assert!(f.get_can_export_found(), "после поиска — можно");
    e.type_limit("100");
    assert_eq!(f.get_found_label().as_str(), "Выгрузить найденные (100 из 261)");
    assert!(f.get_can_export_found(), "правка поля кнопку не гасит: условия те же");
    assert!(!f.get_stale(), "и список не устарел: поле «не больше» — не условие поиска");
    e.type_limit("0");
    assert_eq!(f.get_found_label().as_str(), "Выгрузить найденные (261)", "0 — все");
    e.type_limit("abc");
    assert_eq!(f.get_found_label().as_str(), "Выгрузить найденные (261)", "не число — подпись как для «всех»");
    assert!(f.get_can_export_found(), "кнопка нажимаема: отказ с причиной скажет сама выгрузка");
    e.type_limit("500");
    f.set_words("совсем другое".into());
    f.invoke_edited();
    assert!(f.get_stale(), "условия изменены");
    assert!(!f.get_can_export_found(), "список по прежним условиям — выгружать найденное нельзя, пока не «Найти»");
}

/// 2. Идущая выгрузка: кнопка выключена; после «Остановить» — снова включена.
fn found_button_is_off_while_working(jobs: &Jobs) {
    let e = env("working", jobs, 3, 0, true);
    let f = &e.app.finder;
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_export_found();
    assert!(f.get_working(), "выгрузка идёт — занят");
    assert!(!f.get_can_export_found(), "пока идёт — «Выгрузить найденные» недоступна");
    assert_eq!(e.message(), "Начинаю…");
    e.stop_and_settle();
    assert!(f.get_can_export_found(), "остановили — снова доступна");
}

/// 3. В поле не число: текст `InvalidLimit`, к серверу ничего, папки нет.
fn a_bad_limit_is_refused(jobs: &Jobs) {
    let e = env("bad-limit", jobs, 3, 0, true);
    let f = &e.app.finder;
    e.find();
    let (cards, lists) = (e.count("/api/task/"), e.count("/api/task?"));
    for bad in ["abc", "-1", "+3", "", "99999999999", "1 000"] {
        f.set_work_message("".into());
        e.type_limit(bad);
        f.invoke_export_found();
        e.pump();
        assert_eq!(e.message(), BAD_LIMIT, "поле «{bad}»");
        assert!(!f.get_working() && !f.get_asking(), "поле «{bad}»: отказ — работа не началась, вопроса нет");
    }
    assert_eq!((e.count("/api/task/"), e.count("/api/task?")), (cards, lists), "сервер ни о чём не спрашивали");
    assert!(!e.out.exists(), "папка выгрузки не создана");
}

/// 4. Две тысячи и больше: вопрос с заголовком и текстом C#; «Отмена» — «Выгрузка отменена.», ничего не начато.
fn a_big_export_asks_and_cancel_starts_nothing(jobs: &Jobs) {
    let e = env("ask-no", jobs, 3, 2500, true);
    let f = &e.app.finder;
    e.find();
    assert_eq!(f.get_found_label().as_str(), "Выгрузить найденные (500 из 2\u{a0}500)", "найдено 2 500, поле «500»");
    e.type_limit("0");
    let (title, text) = e.asked_by_csharp(2500, 0);
    let (cards, lists) = (e.count("/api/task/"), e.count("/api/task?"));
    f.invoke_export_found();
    assert!(f.get_asking(), "2 500 заявок — сначала вопрос");
    assert_eq!(f.get_ask_title().as_str(), title, "заголовок вопроса — как у C#");
    assert_eq!(f.get_ask_text().as_str(), text, "текст вопроса — как у C#, с папкой выгрузки");
    assert!(!f.get_working(), "пока человек не ответил — ничего не идёт");
    f.invoke_ask_no();
    assert!(!f.get_asking(), "«Отмена» — вопрос закрыт");
    assert_eq!(e.message(), CANCELLED);
    e.pump();
    assert!(!f.get_working(), "отменили — работа не началась");
    assert_eq!((e.count("/api/task/"), e.count("/api/task?")), (cards, lists), "после отмены к серверу ничего не ушло");
    assert!(!e.out.exists(), "и папка не создана");
}

/// 5. Порог вопроса — по тому, сколько выгрузится, а не по найденному: 1999 — без вопроса, 2000 — с вопросом.
fn the_question_depends_on_how_many_will_be_exported(jobs: &Jobs) {
    let e = env("threshold", jobs, 3, 2500, true);
    let f = &e.app.finder;
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    e.type_limit("1999");
    f.invoke_export_found();
    assert!(!f.get_asking(), "1999 из 2 500 — до порога, без вопроса");
    assert!(f.get_working(), "выгрузка пошла сразу");
    e.stop_and_settle();
    e.type_limit("2000");
    let (title, _) = e.asked_by_csharp(2500, 2000);
    f.invoke_export_found();
    assert!(f.get_asking() && !f.get_working(), "2000 — ровно порог: вопрос");
    assert_eq!(f.get_ask_title().as_str(), title);
    f.invoke_ask_no();
    assert_eq!(e.message(), CANCELLED);
}

/// 5б. Найдено мало, а «не больше» огромно — выгрузится найденное, вопроса нет.
fn a_huge_limit_over_a_small_result_does_not_ask(jobs: &Jobs) {
    let e = env("threshold-small", jobs, 3, 0, true);
    let f = &e.app.finder;
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    e.type_limit("5000");
    f.invoke_export_found();
    assert!(!f.get_asking() && f.get_working(), "найдено 3 — вопроса нет, хоть «не больше» — 5 000");
    e.stop_and_settle();
}

/// 6. (движок) «Выгрузить» в вопросе: выгрузка идёт — список спрашивается, файлы ложатся.
fn engine_yes_starts_the_export(jobs: &Jobs) {
    let e = env("ask-yes", jobs, 3, 2500, true);
    let f = &e.app.finder;
    e.find();
    e.type_limit("0");
    let lists = e.count("/api/task?");
    f.invoke_export_found();
    assert!(f.get_asking(), "вопрос задан");
    f.invoke_ask_yes();
    assert!(!f.get_asking(), "«Выгрузить» — вопрос закрыт");
    assert!(f.get_working(), "выгрузка началась");
    e.wait("выгрузка кончилась", || !f.get_working());
    assert!(e.count("/api/task?") > lists, "выгрузка спросила список у сервера");
    assert!(e.ticket_file(900).is_file(), "файл заявки на месте: {:?}", files_under(&e.dir));
    assert!(e.message().starts_with("Готово. По отбору — "), "итог выгрузки: {}", e.message());
}

/// 7. (движок) Мало заявок — вопроса нет, выгрузка идёт; итог — «Готово. По отбору — 261», без пояснения про «не больше».
fn engine_small_export_runs_without_a_question(jobs: &Jobs) {
    let e = env("small", jobs, 261, 0, true);
    let f = &e.app.finder;
    e.find();
    f.invoke_export_found();
    assert!(!f.get_asking(), "261 заявка — без вопроса");
    assert!(f.get_working(), "выгрузка пошла сразу");
    e.wait("выгрузка кончилась", || !f.get_working());
    let m = e.message();
    assert!(m.starts_with("Готово. По отбору — 261: новых файлов 261"), "{m}");
    assert!(!m.contains("Всего найдено"), "взяты все — пояснения нет: {m}");
    assert!(e.ticket_file(900).is_file() && e.ticket_file(1160).is_file(), "первый и последний файлы на месте");
}

/// 8. (движок) «Не больше» меньше найденного: итог кончается пояснением, как у C#.
fn engine_limited_export_explains_the_limit(jobs: &Jobs) {
    let e = env("limited", jobs, 261, 0, true);
    let f = &e.app.finder;
    e.find();
    e.type_limit("100");
    f.invoke_export_found();
    e.wait("выгрузка кончилась", || !f.get_working());
    let m = e.message();
    assert!(m.starts_with("Готово. По отбору — 100"), "{m}");
    assert!(m.ends_with("\nВсего найдено 261, взяты первые 100: так задано в «Не больше, заявок» (0 — выгрузить все)."), "{m}");
}

/// 9. Доска только для чтения: отказ с текстом, как у «Выгрузить выбранные»; папки нет.
fn read_only_board_refuses_found(jobs: &Jobs) {
    let e = env("ro", jobs, 3, 0, false);
    let f = &e.app.finder;
    e.find();
    f.invoke_export_found();
    e.pump();
    assert_eq!(e.message(), READ_ONLY);
    assert!(!f.get_working() && !f.get_asking(), "отказ — работа не началась");
    assert!(!e.out.exists(), "папка выгрузки не создана");
}

/// 10. «Кто я» не ответил: «Выгрузка не начата: …», файлов нет.
fn found_not_started_without_who_am_i(jobs: &Jobs) {
    let e = env("me-down", jobs, 3, 0, true);
    let f = &e.app.finder;
    e.world.me_down.store(true, Ordering::SeqCst);
    e.find();
    f.invoke_export_found();
    e.wait("итог: выгрузка не начата", || e.message().starts_with("Выгрузка не начата: ") && !f.get_working());
    assert!(e.message().len() > "Выгрузка не начата: ".len(), "причина названа: {}", e.message());
    assert!(files_under(&e.out).is_empty(), "ни одного файла: {:?}", files_under(&e.out));
}

/// 11. (движок) «Остановить» во время выгрузки найденных: остановлено, кнопка возвращается.
fn engine_stop_ends_the_found_export(jobs: &Jobs) {
    let e = env("stop", jobs, 3, 0, true);
    let f = &e.app.finder;
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    let before = e.world.card_in.load(Ordering::SeqCst);
    f.invoke_export_found();
    e.wait("запрос карточки от выгрузки ушёл", || e.world.card_in.load(Ordering::SeqCst) > before);
    f.invoke_stop();
    assert_eq!(e.message(), STOPPING, "«Остановить» — сразу сказано");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("выгрузка закончилась", || !f.get_working());
    assert!(e.message().starts_with("Остановлено"), "итог остановки: {}", e.message());
    assert!(f.get_can_export_found(), "после остановки кнопка снова доступна");
}

#[test]
fn the_search_window_exports_what_was_found() {
    let (_windows, jobs) = common::window::install();
    let scenarios: Vec<Scenario> = vec![
        ("подпись и доступность кнопки", label_follows_the_total_and_the_field),
        ("кнопка выключена, пока идёт выгрузка", found_button_is_off_while_working),
        ("в поле не число — отказ", a_bad_limit_is_refused),
        ("большая выгрузка: вопрос и «Отмена»", a_big_export_asks_and_cancel_starts_nothing),
        ("порог вопроса — по числу выгружаемых", the_question_depends_on_how_many_will_be_exported),
        ("огромное «не больше» над малым списком", a_huge_limit_over_a_small_result_does_not_ask),
        ("движок: «Выгрузить» в вопросе", engine_yes_starts_the_export),
        ("движок: мало заявок — без вопроса, итог", engine_small_export_runs_without_a_question),
        ("движок: «не больше» — пояснение в итоге", engine_limited_export_explains_the_limit),
        ("доска только для чтения — отказ", read_only_board_refuses_found),
        ("«кто я» не ответил — выгрузка не начата", found_not_started_without_who_am_i),
        ("движок: «Остановить»", engine_stop_ends_the_found_export),
    ];
    let mut failed = Vec::new();
    for (name, run) in scenarios {
        if catch_unwind(AssertUnwindSafe(|| run(&jobs))).is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "упали сценарии: {failed:?}");
}
