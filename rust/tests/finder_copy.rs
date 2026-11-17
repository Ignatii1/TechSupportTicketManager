//! Окно поиска показывает текст заявки так, как его получит агент (`export::build`), и «Копировать» кладёт его в буфер
//! (`SearchViewModel.ShowPreviewAsync`/`ContextAsync`/`TextOfAsync`/`Copy`/`ApplySettings`, карточка 4.1-3). Ожидаемый
//! текст берётся вызовом `build` (тот же поддельный сервер, та же строка, ссылка и контекст), без строки `exported:` с
//! обеих сторон — у неё свой момент. Платформа Slint ставится один раз на процесс, поэтому проверка одна; каждый
//! сценарий — со своим окном и своим сервером и под `catch_unwind`: упавший не прячет следующий.

mod common;

use chrono::DateTime;
use slint::Model;
use common::Reply;
use common::window::Jobs;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
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

const CHANGED_A: &str = "2026-10-14T12:00:00+03:00";
const CHANGED_B: &str = "2026-10-15T09:30:00+03:00";
const NO_CLIENT: &str = "API не настроен: адрес, логин и пароль — в настройках TicketBoard";

/// Что делает сервер сейчас: переключатели сценария.
struct World {
    /// Карточки не отвечают, пока флаг поднят (только окну; расчёт ожидаемого текста идёт мимо).
    hold: AtomicBool,
    card_down: AtomicBool,
    life_down: AtomicBool,
    me_down: AtomicBool,
    /// Время изменения заявок («Changed») — и в списке, и в карточке.
    changed: Mutex<String>,
    /// Запросов карточки от окна пришло / отвечено.
    card_in: AtomicUsize,
    card_done: AtomicUsize,
}

impl World {
    fn new() -> World {
        World {
            hold: AtomicBool::new(false),
            card_down: AtomicBool::new(false),
            life_down: AtomicBool::new(false),
            me_down: AtomicBool::new(false),
            changed: Mutex::new(CHANGED_A.into()),
            card_in: AtomicUsize::new(0),
            card_done: AtomicUsize::new(0),
        }
    }

    fn changed(&self) -> String {
        self.changed.lock().unwrap().clone()
    }

    /// Список поиска: две заявки, у обеих одно время изменения.
    fn list(&self) -> String {
        let changed = self.changed();
        let row = |id: i32| {
            format!(r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":31,"Creator":"Петров Пётр","Changed":"{changed}"}}"#)
        };
        format!(
            r#"{{"Tasks":[{},{}],"Statuses":[{{"Id":31,"Name":"Открыта"}}],"Paginator":{{"Count":2,"Page":1,"PageCount":1}}}}"#,
            row(900),
            row(901)
        )
    }

    /// Карточка с длинным описанием: текст заведомо длиннее тысячи знаков (разделитель тысяч в итоге).
    fn card(&self, id: i32) -> String {
        let description = "Длинное описание заявки, чтобы текст перевалил за тысячу знаков. ".repeat(30);
        format!(
            r#"{{"Task":{{"Id":{id},"Name":"Тема из карточки {id}","StatusName":"Открыта","Creator":"Петров Пётр","Created":"2026-10-14T11:00:00+03:00","Changed":"{}","ServiceName":"Печать","Description":"{description}"}}}}"#,
            self.changed()
        )
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
                self.card_done.fetch_add(1, Ordering::SeqCst);
            }
            return if self.card_down.load(Ordering::SeqCst) {
                Reply::new(404, "text/plain", "нет такой")
            } else {
                Reply::json(&self.card(id))
            };
        }
        if path.starts_with("/api/tasklifetime?") {
            return if self.life_down.load(Ordering::SeqCst) {
                Reply::new(500, "text/plain", "история упала")
            } else {
                Reply::json(common::LIVE_LIFE)
            };
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
    /// Второй вход в тот же мир — только для расчёта ожидаемого текста.
    oracle_url: String,
    /// Что положили в буфер обмена, по порядку.
    clip: Rc<RefCell<Vec<String>>>,
    /// Буфер занят другой программой.
    clip_busy: Rc<Cell<bool>>,
}

fn client(url: &str, login: &str) -> Client {
    Client::new(url, login, "p", Duration::from_secs(10))
}

fn env(name: &str, jobs: &Jobs) -> Env {
    let world = Arc::new(World::new());
    let w = world.clone();
    let (url, seen) = serve(move |p| w.answer(p, true));
    let w = world.clone();
    let (oracle_url, _) = serve(move |p| w.answer(p, false));
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-copy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, "IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();
    let app = App::open(&dir, false).unwrap();
    app.use_client(Some(client(&url, "me")));
    let clip = Rc::new(RefCell::new(Vec::new()));
    let clip_busy = Rc::new(Cell::new(false));
    let (c, b) = (clip.clone(), clip_busy.clone());
    app.use_clipboard(move |text| {
        if b.get() {
            return false;
        }
        c.borrow_mut().push(text.to_string());
        true
    });
    Env { app, jobs: jobs.clone(), world, seen, url, oracle_url, clip, clip_busy }
}

/// Целое с разделителем тысяч U+00A0 (`ToString("N0")` у ru-RU).
fn n0(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
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

/// Итог «Копировать»: одна заявка, N — длина текста в UTF-16.
fn summary(text: &str) -> String {
    format!("В буфере обмена — заявок: 1, {} символов. Вставьте в чат.", n0(text.encode_utf16().count()))
}

impl Env {
    fn count(&self, prefix: &str) -> usize {
        self.seen.lock().unwrap().iter().filter(|p| p.starts_with(prefix)).count()
    }

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn pump(&self) {
        let pending = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in pending {
            job();
        }
    }

    /// Выполнять ответы потоков, пока не выполнится условие (до 8 с), иначе — упасть с названием условия.
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

    /// Ждать, пока значение не станет таким; не стало — упасть, показав, что есть вместо него.
    #[track_caller]
    fn wait_is(&self, what: &str, get: impl Fn() -> String, want: &str) {
        let until = Instant::now() + Duration::from_secs(4);
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

    /// Выполнить ответы потоков, пока не выполнится хотя бы один: ответ, который не меняет окна, виден только так.
    #[track_caller]
    fn pump_one(&self, what: &str) {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            let pending = std::mem::take(&mut *self.jobs.lock().unwrap());
            let ran = !pending.is_empty();
            for job in pending {
                job();
            }
            if ran {
                return;
            }
            assert!(Instant::now() < until, "не дождались: {what}");
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
    }

    /// Быстрый поиск с доски: окно показано, список из двух заявок на месте.
    #[track_caller]
    fn find(&self) {
        let f = &self.app.finder;
        self.app.window.invoke_server_search("слово".into());
        self.wait("список поиска", || f.get_rows().row_count() == 2 && !f.get_busy());
    }

    /// Поиск ещё раз по тем же условиям.
    #[track_caller]
    fn find_again(&self) {
        let f = &self.app.finder;
        let before = self.count("/api/task?");
        f.invoke_search();
        self.wait("список поиска ещё раз", || self.count("/api/task?") > before && f.get_rows().row_count() == 2 && !f.get_busy());
    }

    /// Просмотр выбранной заявки пришёл: не занят и что-то показано.
    #[track_caller]
    fn wait_preview(&self) {
        let f = &self.app.finder;
        self.wait("просмотр", || !f.get_preview_busy() && !f.get_preview_text().is_empty());
    }

    fn preview(&self) -> String {
        self.app.finder.get_preview_text().to_string()
    }

    fn message(&self) -> String {
        self.app.finder.get_work_message().to_string()
    }

    /// Текст заявки, как его собирает `build` (тот же мир, строка из списка, ссылка и контекст — «я» и закрытые названия).
    fn agent_text(&self, id: i32) -> String {
        let c = client(&self.oracle_url, "me");
        let page = parse::search(&self.world.list()).expect("список");
        let row = page.found.into_iter().find(|r| r.id == id).expect("строка");
        let closed = ["Выполнена", "Ожидание ответа с автозакрытием", "Закрыта", "Отменена"].map(String::from);
        let context = Context { me: c.get_current_user().ok(), closed: ClosedNames::new(&closed) };
        let url = format!("{}/Task/View/{id}", self.url.trim_end_matches('/'));
        build(&c, &row, &url, &context, &exported()).expect("build собрал текст")
    }

    /// Ошибки, какими их говорит клиент: карточки и страницы истории.
    fn card_error(&self) -> String {
        client(&self.oracle_url, "me").get_task(900).map(|_| ()).expect_err("карточка должна не читаться")
    }

    fn life_error(&self) -> String {
        client(&self.oracle_url, "me").get_lifetime_page(900, 1).map(|_| ()).expect_err("история должна не читаться")
    }

    fn other_client(&self) -> Option<Client> {
        Some(client(&self.url, "other"))
    }
}

type Scenario<'a> = (&'a str, fn(&Jobs));

// ---------- сценарии ----------

/// 1. Выбрали строку: в просмотре — текст для агента; пока читается — занято.
fn preview_shows_the_agent_text(jobs: &Jobs) {
    let e = env("preview", jobs);
    let want = plain(&e.agent_text(900));
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    e.app.finder.invoke_select(0);
    assert!(e.app.finder.get_preview_busy(), "пока заявка читается — «читаю заявку…»");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait_preview();
    assert_eq!(plain(&e.preview()), want, "просмотр — текст для агента, как собирает `build`");
    assert!(!e.preview().is_empty() && !want.is_empty(), "текст не пуст");
}

/// 2. «Кто я» спрашивается один раз на клиента; сменили клиента — спросили снова.
fn who_am_i_is_asked_once_per_client(jobs: &Jobs) {
    let e = env("me-once", jobs);
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    e.app.finder.invoke_select(1);
    e.wait("просмотр второй заявки", || e.app.finder.get_preview_title().contains("901") && !e.app.finder.get_preview_busy());
    e.wait("текст второй заявки", || e.preview().contains("901"));
    assert_eq!(e.count("/api/user?getcurrentuserinfo"), 1, "два просмотра — один вопрос «кто я»");
    e.app.use_client(e.other_client());
    e.find();
    e.app.finder.invoke_select(0);
    e.wait("текст после смены клиента", || e.preview().contains("900"));
    assert_eq!(e.count("/api/user?getcurrentuserinfo"), 2, "другой клиент — спросили снова");
}

/// 2б. «Кто я» не ответил: текст всё равно пришёл, без роли, ошибки в окне нет.
fn text_comes_without_who_am_i(jobs: &Jobs) {
    let e = env("me-down", jobs);
    e.world.me_down.store(true, Ordering::SeqCst);
    let want = plain(&e.agent_text(900));
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    assert_eq!(plain(&e.preview()), want, "текст пришёл и без «кто я»");
    assert!(!e.preview().contains("my_role"), "роли нет: {}", e.preview());
    assert_eq!(e.message(), "", "ошибки «кто я» в окне нет");
    assert!(e.count("/api/user?getcurrentuserinfo") >= 1, "«кто я» спрашивали");
}

/// 3. Карточка не читается: просмотр — «Не удалось прочитать заявку:» и текст ошибки клиента.
fn unreadable_card_is_told_in_the_preview(jobs: &Jobs) {
    let e = env("card-404", jobs);
    e.world.card_down.store(true, Ordering::SeqCst);
    let error = e.card_error();
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    assert_eq!(e.preview(), format!("Не удалось прочитать заявку:\n{error}"));
}

/// 3б. Страница истории не читается: то же.
fn unreadable_history_is_told_in_the_preview(jobs: &Jobs) {
    let e = env("life-500", jobs);
    e.world.life_down.store(true, Ordering::SeqCst);
    let error = e.life_error();
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    assert_eq!(e.preview(), format!("Не удалось прочитать заявку:\n{error}"));
}

/// 4. «Копировать» доступна: нет выбранной — нет; выбрана — да; идёт копирование — нет.
fn copy_is_available_by_state(jobs: &Jobs) {
    let e = env("can-copy", jobs);
    let f = &e.app.finder;
    e.find();
    assert!(!f.get_can_copy(), "ничего не выбрано — копировать нечего");
    e.world.hold.store(true, Ordering::SeqCst);
    f.invoke_select(0);
    assert!(f.get_can_copy(), "строка выбрана — копировать можно");
    f.invoke_copy();
    assert!(!f.get_can_copy(), "идёт копирование — кнопка недоступна");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait_is("итог копирования", || e.message(), &summary(&e.clip.borrow().first().cloned().unwrap_or_default()));
    assert!(f.get_can_copy(), "копирование кончилось — кнопка снова доступна");
}

/// 5. Копирование после просмотра: в буфере ровно показанный текст, новых запросов нет, итог с разделителем тысяч.
fn copy_after_the_preview_reuses_it(jobs: &Jobs) {
    let e = env("copy-after", jobs);
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    let shown = e.preview();
    let (cards, lives) = (e.count("/api/task/"), e.count("/api/tasklifetime"));
    e.app.finder.invoke_copy();
    e.wait_is("итог копирования", || e.message(), &summary(&shown));
    assert_eq!(*e.clip.borrow(), vec![shown.clone()], "в буфере — ровно показанный текст");
    assert_eq!((e.count("/api/task/"), e.count("/api/tasklifetime")), (cards, lives), "прочитанное повторно не читается");
    assert!(shown.encode_utf16().count() >= 1000, "в проверке текст длиннее тысячи знаков: разделитель тысяч виден");
    assert!(e.message().contains('\u{a0}'), "тысячи разделены неразрывным пробелом: {}", e.message());
}

/// 6. Копирование заявки, текст которой ещё не прочитан: «Готовлю текст…», потом буфер и итог.
fn copy_of_an_unread_row_reads_it(jobs: &Jobs) {
    let e = env("copy-unread", jobs);
    let want = plain(&e.agent_text(900));
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    e.app.finder.invoke_select(0);
    e.app.finder.invoke_copy();
    assert_eq!(e.message(), "Готовлю текст: 0 из 1…", "текста ещё нет — идёт чтение");
    assert!(e.clip.borrow().is_empty(), "пока читается — буфер не тронут");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("текст в буфере", || !e.clip.borrow().is_empty());
    let put = e.clip.borrow()[0].clone();
    assert_eq!(plain(&put), want, "в буфере — текст для агента");
    e.wait_is("итог копирования", || e.message(), &summary(&put));
}

/// 6б. У строки другое «изменено» (поиск снова, сервер ответил другим временем): в буфере — текст по новому.
fn a_changed_row_is_read_again(jobs: &Jobs) {
    let e = env("copy-changed", jobs);
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    e.app.finder.invoke_copy();
    e.wait("первый текст в буфере", || e.clip.borrow().len() == 1);
    *e.world.changed.lock().unwrap() = CHANGED_B.into();
    let want = plain(&e.agent_text(900));
    e.find_again();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    e.app.finder.invoke_copy();
    e.wait("второй текст в буфере", || e.clip.borrow().len() == 2);
    let put = e.clip.borrow()[1].clone();
    assert_eq!(plain(&put), want, "строка с новым «изменено» — текст по ней");
    assert_ne!(plain(&e.clip.borrow()[0]), plain(&put), "прежний текст не взят");
}

/// 7. Не прочиталась карточка: «Ничего не скопировано.» и первая ошибка, буфер не тронут.
fn failed_copy_puts_nothing(jobs: &Jobs) {
    let e = env("copy-fails", jobs);
    e.world.card_down.store(true, Ordering::SeqCst);
    let error = e.card_error();
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    e.app.finder.invoke_copy();
    e.wait_is("итог копирования", || e.message(), &format!("Ничего не скопировано.\nНе прочитались: 1. Первая ошибка — #900: {error}"));
    assert!(e.clip.borrow().is_empty(), "буфер не тронут");
}

/// 8. Буфер занят другой программой: сказано, нажать ещё раз; нажали — положено.
fn busy_clipboard_is_told(jobs: &Jobs) {
    let e = env("clip-busy", jobs);
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    e.clip_busy.set(true);
    e.app.finder.invoke_copy();
    e.wait_is("сказано про занятый буфер", || e.message(), "Буфер обмена занят другой программой — нажмите «Копировать» ещё раз.");
    assert!(e.clip.borrow().is_empty(), "занятый буфер — ничего не положено");
    e.clip_busy.set(false);
    e.app.finder.invoke_copy();
    e.wait("текст в буфере со второго раза", || e.clip.borrow().len() == 1);
}

/// 9. Клиента нет: «Копировать» говорит его причину и ничего не спрашивает.
fn no_client_copy_says_why(jobs: &Jobs) {
    let e = env("no-client", jobs);
    e.find();
    e.app.finder.invoke_select(0);
    e.wait_preview();
    e.app.use_client(None);
    let before = e.requests();
    e.app.finder.invoke_copy();
    assert_eq!(e.message(), NO_CLIENT);
    assert_eq!(e.requests(), before, "без клиента ничего не спрашивается");
    assert!(e.clip.borrow().is_empty());
}

/// 10. Клиента сменили, пока копируется: поздний ответ ни в буфер, ни в итог; сказано «Остановлено…».
fn client_change_stops_the_copy(jobs: &Jobs) {
    let e = env("copy-stop", jobs);
    e.find();
    e.world.hold.store(true, Ordering::SeqCst);
    e.app.finder.invoke_select(0);
    e.app.finder.invoke_copy();
    e.wait("запрос карточки ушёл", || e.world.card_in.load(Ordering::SeqCst) >= 1);
    e.app.use_client(e.other_client());
    assert_eq!(e.message(), "Остановлено, в буфер ничего не положено.");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("прежние запросы отвечены", || e.world.card_done.load(Ordering::SeqCst) >= e.world.card_in.load(Ordering::SeqCst));
    e.pump_one("поздний ответ доставлен окну");
    assert!(e.clip.borrow().is_empty(), "поздний ответ буфер не тронул");
    assert_eq!(e.message(), "Остановлено, в буфер ничего не положено.", "итог поздним ответом не затёрт");
}

#[test]
fn the_search_window_copies_the_agent_text() {
    let (_windows, jobs) = common::window::install();
    let scenarios: Vec<Scenario> = vec![
        ("просмотр — текст для агента", preview_shows_the_agent_text),
        ("«кто я» — один раз на клиента", who_am_i_is_asked_once_per_client),
        ("«кто я» не ответил — текст без роли", text_comes_without_who_am_i),
        ("карточка не читается — сказано в просмотре", unreadable_card_is_told_in_the_preview),
        ("история не читается — сказано в просмотре", unreadable_history_is_told_in_the_preview),
        ("«Копировать» доступна по состоянию", copy_is_available_by_state),
        ("копирование после просмотра берёт показанное", copy_after_the_preview_reuses_it),
        ("копирование непрочитанной заявки", copy_of_an_unread_row_reads_it),
        ("другое «изменено» — текст по новому", a_changed_row_is_read_again),
        ("копирование не удалось — ничего не положено", failed_copy_puts_nothing),
        ("буфер занят", busy_clipboard_is_told),
        ("клиента нет", no_client_copy_says_why),
        ("смена клиента останавливает копирование", client_change_stops_the_copy),
    ];
    let mut failed = Vec::new();
    for (name, run) in scenarios {
        if catch_unwind(AssertUnwindSafe(|| run(&jobs))).is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "упали сценарии: {failed:?}");
}
