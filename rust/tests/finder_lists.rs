//! Справочники окна «Поиск в Интрасервисе» (`SearchViewModel.OpenAsync`/`EnsureReferencesAsync`/`ReadReferencesAsync`/
//! `FillChoices`/`StartWithAsync`/`Search`, карточка 3.3-2): что читается при первом показе, какие подписи в списках, куда
//! идёт выбор, что сказано при сбое, быстрый поиск не ждёт. Платформа Slint ставится один раз на процесс, поэтому проверка
//! одна; каждый сценарий — со своим окном и своим сервером и под `catch_unwind`: упавший не прячет следующий.
//! Сервер здесь свой: на каждое соединение отдельный поток (у `common::route` соединения идут по одному, а нужно видеть,
//! что запросы справочников ушли все сразу и что быстрый поиск отвечен, пока справочники держат).

mod common;

use common::Reply;
use common::window::Jobs;
use slint::{Model, ModelRc, SharedString};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ticketboard::app::App;
use ticketboard::intraservice::Client;

// ---------- сервер ----------

/// Сервер с потоком на соединение: `answer` может ждать, остальные запросы это не задерживает. Пути — в порядке прихода.
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

/// Что делает сервер сейчас: переключатели сценария.
#[derive(Default)]
struct World {
    /// Справочники не отвечают, пока флаг поднят.
    hold: AtomicBool,
    /// Справочник отвечает, только когда придут все четыре запроса (или через 3 с — тогда ответ запомнит, сколько пришло).
    barrier: AtomicBool,
    statuses_down: AtomicBool,
    services_down: AtomicBool,
    /// Весь список сервисов — «нет доступа» (запасной список — назначенные).
    services_denied: AtomicBool,
    types_down: AtomicBool,
    filters_down: AtomicBool,
    /// Сколько запросов справочников пришло.
    arrived: AtomicUsize,
    /// Сколько запросов справочников уже пришло в момент каждого ответа.
    arrived_at_answer: Mutex<Vec<usize>>,
}

const STATUSES: &str = r#"[{"Id":31,"Name":"Открыта"},{"Id":38,"Name":"Передана исполнителю"},{"Id":28,"Name":"Закрыта","IsFinal":true}]"#;
const SERVICES_FULL: &str = r#"{"ServiceList":{"Services":[{"Id":840,"Name":"ТСД","Path":"840|","IsArchive":false},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|","IsArchive":false},{"Id":900,"Name":"Старый","Path":"900|","IsArchive":true}],"Paginator":{"Count":3,"Page":1,"PageCount":1}}}"#;
const SERVICES_OWN: &str = r#"{"ServiceList":{"Services":[{"Id":840,"Name":"ТСД","Path":"840|","IsArchive":false},{"Id":844,"Name":"Приложение на ТСД","Path":"840|844|","IsArchive":false}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}"#;
const TYPES: &str = r#"{"TaskTypeList":{"TaskTypes":[{"Id":1009,"Name":"Запрос на обслуживание"},{"Id":3,"Name":"Инцидент","IsArchive":true}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}}"#;
const FILTERS: &str = r#"[{"Id":45,"IsDefault":false,"Name":"Мои заявки"},{"Id":46,"IsDefault":true,"Name":"Основной"}]"#;
const DENIED: &str = r#"{"Message":"Нет прав на просмотр списка сервисов"}"#;

impl World {
    /// Ответ справочника: сначала сойтись с остальными (если нужно), потом ждать, пока его держат.
    fn directory(&self, down: &AtomicBool, ok: impl FnOnce() -> Reply) -> Reply {
        self.arrived.fetch_add(1, Ordering::SeqCst);
        let until = Instant::now() + Duration::from_secs(3);
        while self.barrier.load(Ordering::SeqCst) && self.arrived.load(Ordering::SeqCst) < 4 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.arrived_at_answer.lock().unwrap().push(self.arrived.load(Ordering::SeqCst));
        let until = Instant::now() + Duration::from_secs(20);
        while self.hold.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        if down.load(Ordering::SeqCst) { Reply::new(503, "text/plain", "") } else { ok() }
    }

    fn answer(&self, path: &str) -> Reply {
        let no = AtomicBool::new(false);
        match path {
            "/api/user?getcurrentuserinfo=true" => Reply::json(r#"{"Id":7,"Name":"Я Сам"}"#),
            "/api/taskstatus" => self.directory(&self.statuses_down, || Reply::json(STATUSES)),
            p if p.starts_with("/api/service?") && p.contains("for=filtertasks") => self.directory(&no, || Reply::json(SERVICES_OWN)),
            p if p.starts_with("/api/service?") => self.directory(&self.services_down, || {
                if self.services_denied.load(Ordering::SeqCst) {
                    Reply::new(403, "application/json", DENIED)
                } else {
                    Reply::json(SERVICES_FULL)
                }
            }),
            p if p.starts_with("/api/tasktype?") => self.directory(&self.types_down, || Reply::json(TYPES)),
            "/api/filter?resource=task" => self.directory(&self.filters_down, || Reply::json(FILTERS)),
            p if p.starts_with("/api/task?") => Reply::json(
                r#"{"Tasks":[{"Id":701,"Name":"Заявка 701","StatusId":28}],"Statuses":[{"Id":28,"Name":"Закрыта"}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"#,
            ),
            _ => Reply::new(404, "text/plain", "мимо"),
        }
    }
}

// ---------- окно ----------

struct Env {
    app: App,
    jobs: Jobs,
    world: Arc<World>,
    seen: Arc<Mutex<Vec<String>>>,
    url: String,
}

fn env(name: &str, jobs: &Jobs, setup: impl FnOnce(&World)) -> Env {
    let world = World::default();
    setup(&world);
    let world = Arc::new(world);
    let w = world.clone();
    let (url, seen) = serve(move |p| w.answer(p));
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-lists-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, "IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();
    let app = App::open(&dir, false).unwrap();
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));
    Env { app, jobs: jobs.clone(), world, seen, url }
}

impl Env {
    fn paths(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// Сколько запросов с таким началом пути пришло.
    fn asked(&self, prefix: &str) -> usize {
        self.paths().iter().filter(|p| p.starts_with(prefix)).count()
    }

    /// Выполнять ответы потоков, пока не выполнится условие (до 8 с), иначе — упасть с названием условия.
    #[track_caller]
    fn wait(&self, what: &str, done: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            let pending = std::mem::take(&mut *self.jobs.lock().unwrap());
            for job in pending {
                job();
            }
            if done() {
                return;
            }
            assert!(Instant::now() < until, "не дождались: {what}");
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
    }

    /// Все четыре списка заполнены.
    fn all_loaded(&self) -> bool {
        let f = &self.app.finder;
        f.get_status_choices().row_count() > 3
            && f.get_service_choices().row_count() > 1
            && f.get_type_choices().row_count() > 1
            && f.get_filter_choices().row_count() > 1
    }

    /// Кнопка «Поиск»: окно показано, ждём, пока все списки заполнятся.
    #[track_caller]
    fn open_and_load(&self) {
        self.app.window.invoke_open_search();
        self.wait("заполнение всех четырёх списков", || self.all_loaded());
    }

    /// Спрятать окно и открыть снова кнопкой.
    fn reopen(&self) {
        self.app.finder.invoke_cancel();
        self.app.window.invoke_open_search();
    }

    /// Запросы страниц списка заявок (не справочник сервисов из заявок: тот — pagesize=1000).
    fn lists(&self) -> Vec<String> {
        self.paths().into_iter().filter(|p| p.starts_with("/api/task?") && p.contains("pagesize=50")).collect()
    }

    /// Поставить условия, нажать «Найти», дождаться ответа; вернуть путь запроса списка.
    #[track_caller]
    fn find(&self, set: impl FnOnce(&SearchWindowType)) -> String {
        let before = self.lists().len();
        let f = &self.app.finder;
        f.set_mine(false);
        f.set_words("".into());
        f.set_status_index(0);
        set(f);
        f.invoke_search();
        self.wait("ответ поиска", || self.lists().len() > before && !f.get_busy());
        self.lists().last().unwrap().clone()
    }
}

type Scenario<'a> = (&'a str, fn(&Jobs));
type SearchWindowType = ticketboard::SearchWindow;

fn strings(m: ModelRc<SharedString>) -> Vec<String> {
    m.iter().map(|s| s.to_string()).collect()
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const DEFAULT_STATUSES: [&str; 3] = ["Любой", "Закрытые", "Открытые"];
const PARTIAL_NOTE: &str = "Список сервисов неполный: вложенные сервисы, которых в нём нет, в поиск не вошли.";
const BLIP: &str = "ошибка сервера (HTTP 503)";

// ---------- сценарии ----------

/// Первый показ читает все четыре справочника, один раз и все сразу.
fn reads_all_at_once(jobs: &Jobs) {
    let e = env("once", jobs, |w| w.barrier.store(true, Ordering::SeqCst));
    e.open_and_load();
    assert_eq!(
        *e.world.arrived_at_answer.lock().unwrap(),
        vec![4, 4, 4, 4],
        "каждый справочник ответили, когда пришли все четыре запроса: читаются сразу, а не по очереди"
    );
    for prefix in ["/api/taskstatus", "/api/service?", "/api/tasktype?", "/api/filter?"] {
        assert_eq!(e.asked(prefix), 1, "{prefix} читается один раз");
    }
}

/// Подписи списков и выбор по умолчанию (`FillChoices`).
fn labels(jobs: &Jobs) {
    let e = env("labels", jobs, |_| {});
    e.open_and_load();
    let f = &e.app.finder;
    assert_eq!(
        strings(f.get_status_choices()),
        v(&["Любой", "Закрытые", "Открытые", "Открыта", "Передана исполнителю", "Закрыта"]),
        "статусы: «Любой», «Закрытые», «Открытые», дальше каждый статус сервера по порядку справочника"
    );
    assert_eq!(
        strings(f.get_service_choices()),
        v(&["— любой —", "ТСД", "   Приложение на ТСД", "(архив) Старый"]),
        "сервисы: вложенный сдвинут, архивный с пометкой"
    );
    assert_eq!(strings(f.get_type_choices()), v(&["— любой —", "Запрос на обслуживание", "(архив) Инцидент"]));
    assert_eq!(strings(f.get_filter_choices()), v(&["— нет —", "Мои заявки", "Основной (по умолчанию)"]));
    assert_eq!((f.get_service_note().as_str(), f.get_reference_notes().as_str()), ("", ""), "всё прочиталось — подписей нет");
    assert_eq!(f.get_status_index(), 1, "окно начинается с «моих закрытых»: «Закрытые» — строка 1");
    assert_eq!((f.get_service_index(), f.get_type_index(), f.get_filter_index()), (0, 0, 0), "остальные — «любой»/«нет»");
    assert!(f.get_with_children(), "«С вложенными сервисами» включено");
}

/// Список прочитан — следующий показ его не спрашивает.
fn reopen_does_not_ask_again(jobs: &Jobs) {
    let e = env("reopen", jobs, |_| {});
    e.open_and_load();
    e.reopen();
    // запрос чтения, если бы он ушёл при открытии, пришёл бы раньше запросов двух поисков, идущих после него
    e.find(|f| f.set_status_index(1));
    e.find(|f| f.set_status_index(2));
    for prefix in ["/api/taskstatus", "/api/service?", "/api/tasktype?", "/api/filter?"] {
        assert_eq!(e.asked(prefix), 1, "{prefix}: прочитанное второй раз не читается");
    }
}

/// Не загрузились типы и фильтры: сказано; следующий показ спрашивает только их, пока они не прочитаются.
fn only_failed_lists_are_asked_again(jobs: &Jobs) {
    let e = env("failed", jobs, |w| {
        w.types_down.store(true, Ordering::SeqCst);
        w.filters_down.store(true, Ordering::SeqCst);
    });
    let f = &e.app.finder;
    e.app.window.invoke_open_search();
    e.wait("сообщение о несчитанных списках", || !f.get_reference_notes().is_empty());
    assert_eq!(f.get_reference_notes(), format!("Не загрузились: типы ({BLIP}), сохранённые фильтры ({BLIP}). Поиск работает и без них."));
    e.wait("статусы и сервисы заполнены", || {
        f.get_status_choices().row_count() > 3 && f.get_service_choices().row_count() > 1
    });
    assert_eq!(strings(f.get_type_choices()), v(&["— любой —"]), "типы не прочитались — остаётся «любой»");
    assert_eq!(strings(f.get_filter_choices()), v(&["— нет —"]), "фильтры не прочитались — остаётся «нет»");
    assert_eq!((e.asked("/api/taskstatus"), e.asked("/api/service?"), e.asked("/api/tasktype?"), e.asked("/api/filter?")), (1, 1, 1, 1));

    // второй показ: типы ожили, фильтры — нет
    e.world.types_down.store(false, Ordering::SeqCst);
    e.reopen();
    let note = format!("Не загрузились: сохранённые фильтры ({BLIP}). Поиск работает и без них.");
    e.wait("типы прочитаны, сказано только про фильтры", || {
        f.get_type_choices().row_count() == 3 && f.get_reference_notes() == note
    });
    assert_eq!((e.asked("/api/taskstatus"), e.asked("/api/service?")), (1, 1), "прочитанные статусы и сервисы не спрашивались");
    assert_eq!((e.asked("/api/tasktype?"), e.asked("/api/filter?")), (2, 2), "спрошены только типы и фильтры");

    // третий: ожили и фильтры — сказать больше нечего
    e.world.filters_down.store(false, Ordering::SeqCst);
    e.reopen();
    e.wait("фильтры прочитаны, подписи нет", || {
        f.get_filter_choices().row_count() == 3 && f.get_reference_notes().is_empty()
    });
    assert_eq!((e.asked("/api/tasktype?"), e.asked("/api/filter?")), (2, 3), "типы прочитаны — их не спрашивали, фильтры — в третий раз");
    assert_eq!((e.asked("/api/taskstatus"), e.asked("/api/service?")), (1, 1));
}

/// Не загрузились статусы: сказано словами сервера; в списке остаются три строки.
fn failed_statuses_are_named(jobs: &Jobs) {
    let e = env("statuses", jobs, |w| w.statuses_down.store(true, Ordering::SeqCst));
    let f = &e.app.finder;
    e.app.window.invoke_open_search();
    e.wait("сообщение о статусах", || !f.get_reference_notes().is_empty());
    assert_eq!(f.get_reference_notes(), format!("Не загрузились: статусы ({BLIP}). Поиск работает и без них."));
    e.wait("остальные списки заполнены", || {
        f.get_type_choices().row_count() == 3 && f.get_service_choices().row_count() > 1
    });
    assert_eq!(strings(f.get_status_choices()), v(&DEFAULT_STATUSES), "статусов сервера нет — только три строки");
}

/// Сервисы не загрузились: подпись под полем сервиса, а не над условиями; следующий показ читает их снова.
fn failed_services_are_told_under_the_field(jobs: &Jobs) {
    let e = env("services", jobs, |w| w.services_down.store(true, Ordering::SeqCst));
    let f = &e.app.finder;
    e.app.window.invoke_open_search();
    let note = format!("Список сервисов не загрузился: {BLIP}. Сервис можно задать сохранённым фильтром.");
    e.wait("подпись под сервисами", || f.get_service_note() == note && f.get_type_choices().row_count() == 3);
    assert_eq!(f.get_reference_notes(), "", "о сервисах — под их полем, не среди «Не загрузились»");
    assert_eq!(strings(f.get_service_choices()), v(&["— любой —"]));
    assert_eq!(e.asked("/api/service?"), 1, "после 503 запасных списков не просят");

    e.world.services_down.store(false, Ordering::SeqCst);
    e.reopen();
    e.wait("сервисы прочитаны, подписи нет", || {
        f.get_service_choices().row_count() == 4 && f.get_service_note().is_empty()
    });
    assert_eq!(e.asked("/api/service?"), 2, "следующий показ спросил сервисы снова");
    assert_eq!((e.asked("/api/taskstatus"), e.asked("/api/tasktype?"), e.asked("/api/filter?")), (1, 1, 1), "прочитанное не спрашивалось");
}

/// Сервер отдал не весь список сервисов: под полем — какой это список, подписи плоские.
fn partial_services_say_what_list_it_is(jobs: &Jobs) {
    let e = env("partial", jobs, |w| w.services_denied.store(true, Ordering::SeqCst));
    e.open_and_load();
    let f = &e.app.finder;
    assert_eq!(f.get_service_note(), "Весь список сервисов сервер не отдал (нет доступа (HTTP 403)) — здесь те, на которые вы назначены.");
    assert_eq!(
        strings(f.get_service_choices()),
        v(&["— любой —", "ТСД", "Приложение на ТСД"]),
        "неполный список — без сдвигов: родителя в нём может не быть"
    );
    assert_eq!(f.get_reference_notes(), "", "список получен (пусть и неполный) — среди несчитанных его нет");
}

/// Выбран сервис «с вложенными», а список неполный — над списком заявок сказано, что вложенные не вошли.
fn nested_note_over_the_list_when_partial(jobs: &Jobs) {
    let e = env("nested", jobs, |w| w.services_denied.store(true, Ordering::SeqCst));
    e.open_and_load();
    let f = &e.app.finder;
    e.find(|f| f.set_service_index(1));
    assert!(f.get_notes().contains(PARTIAL_NOTE), "сервис с вложенными из неполного списка: {}", f.get_notes());
    e.find(|f| {
        f.set_service_index(1);
        f.set_with_children(false);
    });
    assert!(!f.get_notes().contains("Список сервисов"), "без вложенных сказать нечего: {}", f.get_notes());
    e.find(|f| f.set_service_index(0));
    assert!(!f.get_notes().contains("Список сервисов"), "сервис не выбран: {}", f.get_notes());
}

/// Весь список сервисов получен — вложенные найдены, замечания нет.
fn no_nested_note_when_the_list_is_whole(jobs: &Jobs) {
    let e = env("whole", jobs, |_| {});
    e.open_and_load();
    e.find(|f| f.set_service_index(1));
    assert!(!e.app.finder.get_notes().contains("Список сервисов"), "{}", e.app.finder.get_notes());
}

/// Статус из списка — в запрос: закрытые, открытые, один статус.
fn status_choice_reaches_the_request(jobs: &Jobs) {
    let e = env("status", jobs, |_| {});
    e.open_and_load();
    for (index, expected) in [
        (1, Some("StatusIds=28&")),
        (2, Some("StatusIds=31,38&")),
        (3, Some("StatusIds=31&")),
        (4, Some("StatusIds=38&")),
        (5, Some("StatusIds=28&")),
        (0, None),
    ] {
        let sent = e.find(|f| f.set_status_index(index));
        match expected {
            Some(part) => assert!(sent.contains(part), "строка {index}: в запросе нет «{part}»: {sent}"),
            None => assert!(!sent.contains("StatusIds"), "«Любой» — без статуса: {sent}"),
        }
    }
}

/// Сервис из списка — в запрос: сам, с вложенными, без вложенных.
fn service_choice_reaches_the_request(jobs: &Jobs) {
    let e = env("service", jobs, |_| {});
    e.open_and_load();
    let sent = e.find(|f| f.set_service_index(1));
    assert!(sent.contains("ServiceIds=840,844&"), "с вложенными: {sent}");
    let sent = e.find(|f| {
        f.set_service_index(1);
        f.set_with_children(false);
    });
    assert!(sent.contains("ServiceIds=840&"), "без вложенных: {sent}");
    let sent = e.find(|f| f.set_service_index(2));
    assert!(sent.contains("ServiceIds=844&"), "вложенный сам: {sent}");
    let sent = e.find(|f| f.set_service_index(0));
    assert!(!sent.contains("ServiceIds"), "«— любой —» — без сервиса: {sent}");
}

/// Тип из списка — в запрос.
fn type_choice_reaches_the_request(jobs: &Jobs) {
    let e = env("type", jobs, |_| {});
    e.open_and_load();
    let sent = e.find(|f| f.set_type_index(2));
    assert!(sent.contains("TypeIds=3&"), "{sent}");
    let sent = e.find(|f| f.set_type_index(1));
    assert!(sent.contains("TypeIds=1009&"), "{sent}");
    let sent = e.find(|f| f.set_type_index(0));
    assert!(!sent.contains("TypeIds"), "{sent}");
}

/// Сохранённый фильтр из списка — в запрос (`filterid` и поля).
fn filter_choice_reaches_the_request(jobs: &Jobs) {
    let e = env("filter", jobs, |_| {});
    e.open_and_load();
    let sent = e.find(|f| f.set_filter_index(2));
    assert!(sent.contains("filterid=46&fields="), "{sent}");
    let sent = e.find(|f| f.set_filter_index(0));
    assert!(!sent.contains("filterid"), "{sent}");
}

/// Быстрый поиск с доски не ждёт справочников: список ответили, пока их держат; а чтение справочников уже идёт.
fn quick_search_does_not_wait(jobs: &Jobs) {
    let e = env("quick", jobs, |w| w.hold.store(true, Ordering::SeqCst));
    let f = &e.app.finder;
    e.app.window.invoke_server_search("принтер".into());
    e.wait("строки быстрого поиска при удерживаемых справочниках", || {
        f.get_rows().row_count() == 1 && e.world.arrived.load(Ordering::SeqCst) >= 4
    });
    assert!(e.world.hold.load(Ordering::SeqCst) && !e.all_loaded(), "справочники всё ещё держат — а список уже на экране");
    assert_eq!(f.get_message(), "Найдено: 1");
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("справочники пришли в фоне", || e.all_loaded());
    assert_eq!(f.get_rows().row_count(), 1, "список быстрого поиска на месте");
}

/// Выбор, сделанный до прихода справочников, переживает их заполнение.
fn choice_survives_the_loading(jobs: &Jobs) {
    let e = env("survive", jobs, |w| w.hold.store(true, Ordering::SeqCst));
    let f = &e.app.finder;
    e.app.window.invoke_open_search();
    e.wait("запросы справочников ушли", || e.world.arrived.load(Ordering::SeqCst) >= 4);
    f.set_status_index(2);
    e.world.hold.store(false, Ordering::SeqCst);
    e.wait("списки заполнены", || e.all_loaded());
    assert_eq!(f.get_status_index(), 2, "«Открытые» остались выбранными");
    assert_eq!(strings(f.get_status_choices())[2], "Открытые");
}

/// «Сбросить»: каждый список — «любой»/«нет», «С вложенными» включено, сами списки на месте.
fn clear_resets_every_list(jobs: &Jobs) {
    let e = env("clear", jobs, |_| {});
    e.open_and_load();
    let f = &e.app.finder;
    f.set_status_index(4);
    f.set_service_index(2);
    f.set_with_children(false);
    f.set_type_index(1);
    f.set_filter_index(1);
    f.invoke_clear();
    assert_eq!(f.get_status_index(), 0, "статус — «Любой»");
    assert_eq!((f.get_service_index(), f.get_type_index(), f.get_filter_index()), (0, 0, 0), "сервис, тип, фильтр — «любой»/«нет»");
    assert!(f.get_with_children(), "«С вложенными сервисами» снова включено");
    assert_eq!(f.get_service_choices().row_count(), 4, "прочитанные списки «Сбросить» не опустошает");
}

/// Другой клиент: списки и подписи забыты, читаются заново при следующем показе.
fn client_change_forgets_the_lists(jobs: &Jobs) {
    let e = env("client", jobs, |w| w.types_down.store(true, Ordering::SeqCst));
    let f = &e.app.finder;
    e.app.window.invoke_open_search();
    e.wait("списки и сообщение о типах", || e.all_loaded_but_types() && !f.get_reference_notes().is_empty());
    e.app.finder.invoke_cancel();
    e.app.use_client(Some(Client::new(&e.url, "other", "p", Duration::from_secs(10))));
    assert_eq!(strings(f.get_status_choices()), v(&DEFAULT_STATUSES), "списки прежнего сервера забыты");
    assert_eq!(strings(f.get_service_choices()), v(&["— любой —"]));
    assert_eq!(strings(f.get_type_choices()), v(&["— любой —"]));
    assert_eq!(strings(f.get_filter_choices()), v(&["— нет —"]));
    assert_eq!((f.get_service_note().as_str(), f.get_reference_notes().as_str()), ("", ""), "подписи прежнего клиента убраны");
    e.world.types_down.store(false, Ordering::SeqCst);
    e.app.window.invoke_open_search();
    e.wait("списки прочитаны заново", || e.all_loaded() && f.get_reference_notes().is_empty());
    assert_eq!(
        (e.asked("/api/taskstatus"), e.asked("/api/service?"), e.asked("/api/tasktype?"), e.asked("/api/filter?")),
        (2, 2, 2, 2),
        "новый клиент прочитал всё заново"
    );
}

impl Env {
    /// Статусы, сервисы и фильтры заполнены (типы — нет).
    fn all_loaded_but_types(&self) -> bool {
        let f = &self.app.finder;
        f.get_status_choices().row_count() > 3 && f.get_service_choices().row_count() > 1 && f.get_filter_choices().row_count() > 1
    }
}

#[test]
fn the_search_window_lists() {
    let (_windows, jobs) = common::window::install();
    let scenarios: Vec<Scenario> = vec![
        ("все четыре справочника сразу и один раз", reads_all_at_once),
        ("подписи списков и выбор по умолчанию", labels),
        ("прочитанное повторно не читается", reopen_does_not_ask_again),
        ("повторно спрашиваются только несчитанные", only_failed_lists_are_asked_again),
        ("несчитанные статусы названы", failed_statuses_are_named),
        ("несчитанные сервисы — под полем сервиса", failed_services_are_told_under_the_field),
        ("неполный список сервисов — какой он", partial_services_say_what_list_it_is),
        ("вложенные не вошли — над списком", nested_note_over_the_list_when_partial),
        ("полный список сервисов — без замечания", no_nested_note_when_the_list_is_whole),
        ("статус уходит в запрос", status_choice_reaches_the_request),
        ("сервис уходит в запрос", service_choice_reaches_the_request),
        ("тип уходит в запрос", type_choice_reaches_the_request),
        ("фильтр уходит в запрос", filter_choice_reaches_the_request),
        ("быстрый поиск не ждёт справочников", quick_search_does_not_wait),
        ("выбор переживает заполнение", choice_survives_the_loading),
        ("«Сбросить» возвращает списки", clear_resets_every_list),
        ("смена клиента забывает списки", client_change_forgets_the_lists),
    ];
    let mut failed = Vec::new();
    for (name, run) in scenarios {
        if catch_unwind(AssertUnwindSafe(|| run(&jobs))).is_err() {
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "упали сценарии: {failed:?}");
}
