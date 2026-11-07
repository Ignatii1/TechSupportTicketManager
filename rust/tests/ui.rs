//! Окно без экрана: программная отрисовка (`MinimalSoftwareWindow`), события мыши — прямо в окно. Одна проверка на всё:
//! платформа Slint ставится один раз на поток.

mod common;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, PhysicalSize, SharedString};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ticketboard::app::{App, PANEL_PAUSE, SAVE_DELAY};
use ticketboard::board::{Priority, Status, Ticket, parse_tickets};
use ticketboard::intraservice::Client;

type Jobs = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

/// Окна без экрана: у каждого компонента своё (доска, быстрое добавление), по порядку создания.
struct Headless {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    start: Instant,
    /// Что потоки прислали в окно (`upgrade_in_event_loop`): выполняет `pump`, как цикл событий.
    jobs: Jobs,
}

struct Queue(Jobs);

impl EventLoopProxy for Queue {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(w.clone());
        Ok(w)
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Queue(self.jobs.clone())))
    }
}

/// Выполнять, что потоки присылают в окно, как цикл событий, пока не выполнится условие (до 15 с): отвечает и тихое чтение
/// панели — ждать «первого попавшегося» ответа нельзя.
#[track_caller]
fn pump_until(jobs: &Jobs, done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        let pending = std::mem::take(&mut *jobs.lock().unwrap());
        for job in pending {
            job();
        }
        if done() {
            return;
        }
        assert!(Instant::now() < until, "поток так и не ответил");
        std::thread::sleep(Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
    }
}

/// Подождать по-настоящему и дать сработать таймерам окна.
fn wait(d: Duration) {
    std::thread::sleep(d);
    slint::platform::update_timers_and_animations();
}

const BOARD: &str = r#"[
  {"Id": "00000000-0000-4000-8000-00000000000a", "Title": "Принтер в 305 не печатает", "Status": "Inbox", "IntraserviceId": 702180,
   "Priority": "High", "StatusChangedAt": "2026-01-01T09:00:00+03:00", "Service": "Печать", "UnreadComments": 2},
  {"Id": "00000000-0000-4000-8000-00000000000b", "Title": "Нет доступа к папке", "Status": "Inbox", "IntraserviceId": 702181},
  {"Id": "00000000-0000-4000-8000-00000000000c", "Title": "Своя задача", "Status": "InProgress"},
  {"Id": "00000000-0000-4000-8000-00000000000d", "Title": "Ждём ответа", "Status": "Waiting", "IntraserviceId": 70119},
  {"Id": "00000000-0000-4000-8000-00000000000e", "Title": "Давно сделано", "Status": "Done", "CompletedAt": "2025-01-01T00:00:00+03:00"}
]"#;

fn id(c: char) -> String {
    format!("00000000-0000-4000-8000-00000000000{c}")
}

fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
        .collect()
}

fn keys(app: &App, column: usize) -> Vec<String> {
    let col = app.window.get_columns().row_data(column).expect("колонка есть");
    col.cards.iter().map(|c| c.key.to_string()).collect()
}

fn counts(app: &App) -> Vec<i32> {
    app.window.get_columns().iter().map(|c| c.count).collect()
}

fn mouse(window: &slint::Window, events: &[(f32, f32, Option<bool>)]) {
    // (x, y, нажать/отпустить/просто двинуть)
    for &(x, y, press) in events {
        let position = LogicalPosition::new(x, y);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        match press {
            Some(true) => window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left }),
            Some(false) => window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left }),
            None => {}
        }
        slint::platform::update_timers_and_animations();
    }
}

/// Клавиша (с Ctrl, если `ctrl`): нажать и отпустить.
fn key(window: &slint::Window, text: impl Into<SharedString>, ctrl: bool) {
    let text = text.into();
    if ctrl {
        window.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    }
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    if ctrl {
        window.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    }
    slint::platform::update_timers_and_animations();
}

/// Путь запроса: сервер записывает его целиком («GET /api/task/1?include=status HTTP/1.1…»).
fn path(request: &str) -> &str {
    request.split(' ').nth(1).unwrap_or("")
}

fn file_titles(dir: &Path) -> Vec<String> {
    file_tickets(dir).into_iter().map(|t| t.title).collect()
}

fn file_tickets(dir: &Path) -> Vec<Ticket> {
    parse_tickets(&std::fs::read_to_string(dir.join("tickets.json")).unwrap()).unwrap()
}

/// Окно поверх доски: (заголовок, текст, действие, отказ).
#[track_caller]
fn dialog(app: &App) -> (String, String, String, String) {
    let w = &app.window;
    assert!(w.get_dialog_open(), "окна нет");
    (w.get_dialog_heading().into(), w.get_dialog_text().into(), w.get_dialog_action().into(), w.get_dialog_decline_text().into())
}

fn card_reply(name: &str, extra: &str) -> common::Reply {
    common::Reply::json(&format!(
        r#"{{"Task":{{"Id":1,"Name":"{name}","StatusName":"В работе","Description":"Не печатает с утра","Creator":"Петрова А."{extra}}}}}"#
    ))
}

#[test]
fn the_board_window() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let jobs = Jobs::default();
    slint::platform::set_platform(Box::new(Headless { windows: windows.clone(), start: Instant::now(), jobs: jobs.clone() })).unwrap();

    let dir = std::env::temp_dir().join(format!("tb-rs-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), BOARD).unwrap();
    // поддельный сервер — и на Linux: клиент подставляется (`use_client`), DPAPI здесь не нужен. Статус 702190 (её добавит
    // импорт) и справочник меняют шаги F5
    let status_c = Arc::new(Mutex::new(String::from("Решена")));
    let statuses_down = Arc::new(AtomicBool::new(false));
    let (server_status, server_down) = (status_c.clone(), statuses_down.clone());
    let (url, requests) = common::route(move |path| match path {
        p if p.starts_with("/api/task/70112?") => card_reply("Принтер не печатает", r#","ServiceName":"Печать""#),
        p if p.starts_with("/api/task/70119?") => common::Reply::new(404, "application/json", r#"{"Message":"нет такой"}"#),
        p if p.starts_with("/api/task/702199?") => {
            std::thread::sleep(Duration::from_millis(300)); // медленная: пока отвечает, сервер сменят
            card_reply("Медленная", "")
        }
        p if p.starts_with("/api/task/702190?") => common::Reply::json(&format!(
            r#"{{"Task":{{"Id":702190,"Name":"Склад: c","StatusName":"{}","Creator":"Петрова А."}}}}"#,
            server_status.lock().unwrap()
        )),
        p if p.starts_with("/api/task/") => card_reply("Склад: удалить задания", ""),
        p if p.starts_with("/api/tasklifetime?") => common::Reply::json(common::LIVE_LIFE),
        "/api/taskstatus" if server_down.load(Ordering::SeqCst) => common::Reply::new(500, "text/plain", "справочник упал"),
        "/api/taskstatus" => common::Reply::json(
            r#"[{"Id":31,"Name":"Открыта"},{"Id":28,"Name":"Закрыта","IsFinal":true},{"Id":29,"Name":"Решена","IsFinal":true}]"#,
        ),
        "/api/user?getcurrentuserinfo=true" => common::Reply::json(r#"{"Id":40800,"Name":"Иванов Иван Иванович"}"#),
        p if p.starts_with("/api/task?") => {
            common::Reply::json(r#"{"Tasks":[{"Id":702180,"Name":"a"},{"Id":702181,"Name":"b"},{"Id":702190,"Name":"c"}]}"#)
        }
        _ => common::Reply::new(404, "text/plain", "мимо"),
    });
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, "IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();

    let app = App::open(&dir, true).unwrap();
    let (window, capture) = (windows.borrow()[0].clone(), windows.borrow()[1].clone());
    assert_eq!(app.hotkey(), "Ctrl+Alt+Space", "клавиша — из настроек, по умолчанию как у C#");
    assert_eq!(app.capture.get_hint(), ticketboard::capture::EMPTY_HINT, "окно добавления готово с самого начала");
    assert!(!dir.join("errors.log").exists(), "всё прочиталось — журнала нет");
    // пароль здесь не расшифровать — ⟳ нельзя; подставили клиента — панель сразу знает, без нового выбора
    app.window.invoke_select(id('a').into());
    assert!(!app.window.get_can_sync());
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(5))));
    assert!(app.window.get_can_sync(), "use_client — панель заново");
    app.window.set_panel_open(false);
    window.set_size(PhysicalSize::new(1100, 720));
    app.window.show().unwrap();
    assert!(app.writable(), "доска своя — пишет");
    assert!(app.window.get_notice().contains("TicketBoard.exe не запустится"), "{}", app.window.get_notice());
    window.draw_if_needed(|r| {
        let mut buf = vec![slint::platform::software_renderer::PremultipliedRgbaColor::default(); 1100 * 720];
        r.render(&mut buf, 1100);
    });

    // колонки и счётчики; старая «Готово» скрыта; лимит 0 — «В работе» перегружена
    let cols = app.window.get_columns();
    let titles: Vec<String> = cols.iter().map(|c| c.title.to_string()).collect();
    assert_eq!(titles, ["Входящие", "В работе", "Ждёт ответа", "Готово"]);
    assert_eq!(counts(&app), [2, 1, 1, 0]);
    assert_eq!(keys(&app, 0), [id('a'), id('b')]);
    assert!(cols.row_data(1).unwrap().overloaded);
    let a = cols.row_data(0).unwrap().cards.row_data(0).unwrap();
    assert_eq!((a.number.as_str(), a.priority.as_str(), a.priority_level, a.age_state, a.unread), ("#702180", "Высокий", 2, 2, 2));

    // щелчок по карточке — выбор и панель
    let w = app.window.window();
    mouse(w, &[(100.0, 110.0, Some(true)), (100.0, 110.0, Some(false))]);
    assert_eq!(app.window.get_selected_key(), id('a').as_str());
    assert!(app.window.get_panel_open(), "выбор открывает панель");
    let panel = app.window.get_panel();
    assert_eq!((panel.number.as_str(), app.window.get_panel_status(), panel.priority_level, panel.has_number), ("#702180", 0, 2, true));
    assert_eq!(app.window.get_title_text(), "Принтер в 305 не печатает");
    assert!(app.window.get_can_sync(), "номер и API — ⟳ можно");

    // правка названия — на карточке сразу, в файле — через паузу, одной записью
    let before = std::fs::read(dir.join("tickets.json")).unwrap();
    app.window.invoke_edit_title("Принтер в 305".into());
    app.window.invoke_edit_title("Принтер в 305 — замятие".into());
    assert_eq!(cols.row_data(0).unwrap().cards.row_data(0).unwrap().title, "Принтер в 305 — замятие");
    assert!(app.dirty());
    assert_eq!(std::fs::read(dir.join("tickets.json")).unwrap(), before, "до паузы файл не тронут");
    wait(SAVE_DELAY + Duration::from_millis(150));
    assert!(!app.dirty(), "записано");
    assert_eq!(file_titles(&dir)[0], "Принтер в 305 — замятие");
    assert!(dir.join("backups").read_dir().unwrap().count() == 1, "копия прежнего файла за день");

    // клавиши: 1/2/3 — приоритет, → и ← — колонка; Esc — панель закрыть, Enter — открыть
    key(w, "1", false);
    assert_eq!(app.window.get_panel().priority_level, 0);
    key(w, Key::RightArrow, false);
    assert_eq!(keys(&app, 1)[0], id('a'), "→ — в «В работе», наверх");
    key(w, Key::LeftArrow, false);
    key(w, Key::LeftArrow, false);
    assert_eq!(keys(&app, 0)[0], id('a'), "← у края — ничего");
    key(w, Key::Escape, false);
    assert!(!app.window.get_panel_open());
    key(w, Key::Return, false);
    assert!(app.window.get_panel_open());
    key(w, Key::Escape, false);

    // перетаскивание: из «Входящих» в «Ждёт ответа» — наверх колонки
    mouse(
        w,
        &[
            (100.0, 110.0, Some(true)),
            (110.0, 115.0, None),
            (200.0, 200.0, None),
            (500.0, 250.0, None),
            (680.0, 300.0, None),
            (680.0, 300.0, Some(false)),
        ],
    );
    assert_eq!(keys(&app, 2), [id('a'), id('d')]);
    assert_eq!(counts(&app), [1, 1, 2, 0]);
    app.window.set_panel_open(false);

    // заметки и колонка из панели
    app.window.invoke_select(id('b').into());
    app.window.invoke_add_note("  позвонить  ".into());
    app.window.invoke_add_note("   ".into());
    let notes = app.window.get_notes();
    assert_eq!(notes.row_count(), 1, "пустая не добавилась");
    assert_eq!(notes.row_data(0).unwrap().text, "позвонить");
    app.window.invoke_delete_note(notes.row_data(0).unwrap().id);
    assert_eq!(app.window.get_notes().row_count(), 0);
    app.window.invoke_move_to(3);
    assert_eq!(keys(&app, 3), [id('b')], "колонка в панели — перенос");
    assert_eq!(app.window.get_panel_status(), 3, "и список колонок в панели — за ней");
    app.window.invoke_select(id('d').into());
    assert_eq!(app.window.get_panel_status(), 2, "другая карточка — её колонка");
    app.window.invoke_select(id('b').into());

    // удаление: вопрос; «Отмена» (Esc) — карточка цела; «Удалить» — нет её
    app.window.invoke_select(id('c').into());
    assert!(!app.window.get_can_sync(), "без номера — ⟳ нельзя");
    app.window.invoke_delete_card();
    assert!(app.window.get_dialog_open());
    assert_eq!(
        (app.window.get_dialog_heading().as_str(), app.window.get_dialog_text().as_str()),
        ("Удалить заявку с доски?", "«Своя задача»")
    );
    key(w, Key::Escape, false);
    assert!(!app.window.get_dialog_open() && app.window.get_columns().row_data(1).unwrap().count == 1, "отменили — цела");
    app.window.invoke_delete_card();
    app.window.invoke_dialog_accept();
    assert_eq!(counts(&app)[1], 0, "удалена");
    assert_eq!(app.window.get_selected_key(), "");
    app.save_now();
    assert!(!file_titles(&dir).contains(&"Своя задача".to_string()));

    // ⟳: «обновляю…», ответ из потока — поля с сервера, переписка; своё название не тронуто
    app.window.invoke_select(id('a').into());
    app.window.invoke_sync();
    assert!(app.window.get_syncing() && app.window.get_sync_message() == "обновляю…");
    pump_until(&jobs, || !app.window.get_syncing());
    assert!(!app.window.get_syncing());
    assert_eq!(app.window.get_sync_message(), "", "{}", app.window.get_sync_message());
    let panel = app.window.get_panel();
    assert_eq!((panel.external_status.as_str(), panel.creator.as_str()), ("В работе", "Петрова А."));
    assert!(panel.synced.starts_with("обновлено"));
    assert!(app.window.get_conversation().contains("Сидорова Мария Ивановна"), "{}", app.window.get_conversation());
    assert_eq!(app.window.get_title_text(), "Принтер в 305 — замятие", "своё название остаётся");
    // нет такой на сервере — сказано под ⟳
    app.window.invoke_select(id('d').into());
    app.window.invoke_sync();
    pump_until(&jobs, || !app.window.get_syncing());
    assert!(app.window.get_sync_message().starts_with("заявка не найдена (HTTP 404)"), "{}", app.window.get_sync_message());

    // быстрое добавление: N — окно; название ищется после паузы и только по последнему номеру
    key(w, "n", false);
    assert!(capture.is_visible(), "N — окно добавления");
    let typed = Instant::now();
    app.capture.invoke_edited("7011".into());
    app.capture.invoke_edited("70112".into());
    assert_eq!(app.capture.get_number_text(), "#70112");
    assert_eq!(app.capture.get_preview(), "Ищу в Интрасервисе…");
    pump_until(&jobs, || app.capture.get_preview() != "Ищу в Интрасервисе…");
    assert_eq!(app.capture.get_preview(), "Принтер не печатает");
    let asked = requests.lock().unwrap().clone();
    assert!(typed.elapsed() >= Duration::from_millis(400), "спрошено после паузы");
    assert_eq!(asked.iter().filter(|r| path(r).starts_with("/api/task/7011?")).count(), 0, "прежний номер не спрашивали: {asked:?}");
    assert_eq!(asked.iter().filter(|r| path(r).starts_with("/api/task/70112?")).count(), 1, "последний — спросили: {asked:?}");
    // ссылка — 1/2/3 меняют приоритет, а не текст
    app.capture.set_text(format!("{url}/Task/View/70112").into());
    app.capture.invoke_edited(format!("{url}/Task/View/70112").into());
    assert!(app.capture.get_digits_set_priority());
    key(app.capture.window(), "3", false);
    assert_eq!(app.capture.get_priority_level(), 2);
    assert_eq!(app.capture.get_text().as_str(), format!("{url}/Task/View/70112"), "цифра не напечаталась");
    // Enter — карточка сверху во «Входящих», выбрана, и сразу ⟳: название с сервера
    app.capture.invoke_accepted();
    assert!(!capture.is_visible(), "после Enter окно спрятано");
    let fresh = keys(&app, 0)[0].clone();
    assert_eq!(app.window.get_selected_key().as_str(), fresh);
    assert!(app.window.get_panel_open());
    assert_eq!(app.window.get_title_text(), "Заявка #70112");
    pump_until(&jobs, || !app.window.get_syncing());
    assert_eq!(app.window.get_title_text(), "Принтер не печатает", "«Заявка #N» — с сервера");
    assert_eq!(app.window.get_panel().priority_level, 2);
    app.save_now();
    let saved = parse_tickets(&std::fs::read_to_string(dir.join("tickets.json")).unwrap()).unwrap();
    assert_eq!((saved[0].intraservice_id, saved[0].priority, saved[0].status), (Some(70112), Priority::High, Status::Inbox));
    // Esc — спрятать; окно снова — пустое
    app.show_capture();
    assert_eq!(app.capture.get_text(), "");
    key(app.capture.window(), Key::Escape, false);
    assert!(!capture.is_visible(), "Esc — спрятано");
    // ушли в другое окно — спрятано, как OnDeactivated
    app.show_capture();
    assert!(capture.is_visible());
    app.capture.window().dispatch_event(WindowEvent::WindowActiveChanged(false));
    slint::platform::update_timers_and_animations();
    assert!(!capture.is_visible(), "фокус ушёл — спрятано");

    // пустое поле — ничего; текст без номера — карточка без запросов и без сообщений
    let inbox = counts(&app)[0];
    app.show_capture();
    app.capture.invoke_accepted();
    assert_eq!(counts(&app)[0], inbox, "пустое поле — ничего");
    app.show_capture();
    app.capture.set_text("Купить картридж".into());
    app.capture.invoke_edited("Купить картридж".into());
    app.capture.invoke_accepted();
    assert_eq!(counts(&app)[0], inbox + 1);
    assert_eq!((app.window.get_title_text().as_str(), app.window.get_sync_message().as_str()), ("Купить картридж", ""));

    // запись не удалась (на месте файла — папка) — сказано в шапке и в журнале, правка ждёт; удалась — шапка прежняя
    std::fs::remove_file(dir.join("tickets.json")).unwrap();
    std::fs::create_dir(dir.join("tickets.json")).unwrap();
    app.window.invoke_edit_title("Купить картридж HP".into());
    app.save_now();
    assert!(app.window.get_notice().starts_with("tickets.json не записан"), "{}", app.window.get_notice());
    assert!(app.dirty(), "правка не потеряна");
    assert!(std::fs::read_to_string(dir.join("errors.log")).unwrap().contains("tickets.json не записан"));
    std::fs::remove_dir(dir.join("tickets.json")).unwrap();
    app.window.invoke_edit_title("Купить картридж HP 85A".into());
    app.save_now();
    assert!(!app.window.get_notice().contains("не записан") && !app.dirty(), "{}", app.window.get_notice());
    assert!(file_titles(&dir).contains(&"Купить картридж HP 85A".to_string()));

    // «Проверить подключение» — отчёт поверх доски
    app.window.invoke_check_connection();
    assert!(app.window.get_reading());
    pump_until(&jobs, || !app.window.get_reading());
    assert!(!app.window.get_reading() && app.window.get_dialog_open());
    let report = app.window.get_dialog_text().to_string();
    assert!(report.contains("Вы — Иванов Иван Иванович (№40800)") && report.contains("нет на доске"), "{report}");
    assert!(report.contains("Сеть: напрямую"), "{report}");
    app.window.invoke_dialog_close();

    // импорт моих: новые — в конец «Входящих», что уже на доске (и в «Готово») — не тронуто; пока идёт — второй не идёт
    let asked = |prefix: &str| requests.lock().unwrap().iter().filter(|r| path(r).starts_with(prefix)).count();
    let lists = asked("/api/task?");
    app.window.invoke_import_mine();
    assert!(app.window.get_importing(), "идёт — кнопка недоступна, в шапке «импорт…»");
    app.window.invoke_import_mine();
    pump_until(&jobs, || !app.window.get_importing());
    assert_eq!(asked("/api/task?"), lists + 1, "второй раз не пошло");
    assert_eq!(dialog(&app), ("Импорт моих заявок".into(), "Добавлено: 1, уже было: 2".into(), String::new(), String::new()));
    app.window.invoke_dialog_close();
    let inbox = keys(&app, 0);
    let c = inbox.last().unwrap().clone();
    assert_eq!((inbox.len(), counts(&app)), (3, vec![3, 0, 2, 1]), "{inbox:?}");
    app.window.invoke_import_mine();
    pump_until(&jobs, || !app.window.get_importing());
    assert_eq!(dialog(&app).1, "Добавлено: 0, уже было: 3", "повтор — ничего нового");
    app.window.invoke_dialog_close();
    app.save_now();
    let saved = file_tickets(&dir);
    let new = saved.iter().find(|t| t.intraservice_id == Some(702190)).expect("в файле");
    assert_eq!(
        (new.title.as_str(), new.url.as_str(), new.status, new.priority),
        ("c", format!("{}/Task/View/702190", url.trim_end_matches('/')).as_str(), Status::Inbox, Priority::Mid)
    );
    assert_eq!(new.assigned_to_me, Some(true));

    // F5: с номером и не из «Готово» — по четыре разом, справочник — параллельно, у открытой (её переписка в панели) —
    // переписка заново; закрытая по признаку справочника — вопрос со списком, итог — одной строкой
    app.window.invoke_select(fresh.clone().into());
    assert!(!app.window.get_conversation().is_empty(), "переписка 70112 — с её ⟳");
    let (directory, done_card, lives) = (asked("/api/taskstatus"), asked("/api/task/702181?"), asked("/api/tasklifetime?taskid=70112&"));
    app.window.invoke_refresh_all();
    assert!(app.window.get_refreshing(), "идёт — кнопка недоступна, в шапке «обновляю статусы…»");
    app.window.invoke_refresh_all();
    pump_until(&jobs, || !app.window.get_refreshing());
    assert_eq!(asked("/api/taskstatus"), directory + 1, "второй раз не пошло");
    assert_eq!(asked("/api/task/702181?"), done_card, "«Готово» не перечитывается");
    assert_eq!(asked("/api/tasklifetime?taskid=70112&"), lives + 1, "переписка открытой — заново");
    assert!(app.dirty(), "статусы — в файл, через паузу");
    let (heading, text, action, decline) = dialog(&app);
    assert_eq!((heading.as_str(), action.as_str(), decline.as_str()), ("Перенести закрытые в «Готово»?", "Перенести", "Оставить"));
    assert!(text.starts_with("Обновлено: 3, не удалось: 1. Первая ошибка — #70119: заявка не найдена (HTTP 404) — "), "{text}");
    assert!(text.ends_with("\n\nЗакрыты в Интрасервисе (1):\n#702190  c") && text.lines().count() == 4, "{text}");
    let panel = app.window.get_panel();
    assert_eq!((panel.external_status.as_str(), panel.creator.as_str()), ("В работе", "Петрова А."), "статусы с сервера");
    // Esc — окно закрыто, ничего не решено: следующее F5 (клавиша) спросит снова
    key(w, Key::Escape, false);
    assert!(!app.window.get_dialog_open() && keys(&app, 0).last() == Some(&c));
    key(w, Key::F5, false);
    assert!(app.window.get_refreshing(), "F5 — обновление");
    pump_until(&jobs, || !app.window.get_refreshing());
    assert_eq!(dialog(&app).0, "Перенести закрытые в «Готово»?", "Esc не решал — спрошено снова");
    // «Оставить» — карточка помнит статус; F5 снова — итог без вопроса, ошибка целиком
    app.window.invoke_dialog_decline();
    assert!(!app.window.get_dialog_open() && keys(&app, 0).last() == Some(&c));
    app.window.invoke_refresh_all();
    pump_until(&jobs, || !app.window.get_refreshing());
    let (heading, text, action, _) = dialog(&app);
    assert_eq!((heading.as_str(), action.as_str()), ("Обновление статусов", ""), "{text}");
    assert_eq!(text, "Обновлено: 3, не удалось: 1. Первая ошибка — #70119: заявка не найдена (HTTP 404):\n{\"Message\":\"нет такой\"}");
    app.window.invoke_dialog_close();
    app.save_now();
    let kept = file_tickets(&dir).into_iter().find(|t| t.intraservice_id == Some(702190)).unwrap();
    assert_eq!((kept.kept_open_status.as_deref(), kept.external_status.as_deref()), (Some("Решена"), Some("Решена")));
    // закрыли иначе (по списку настроек), справочник не пришёл — вопрос снова, в итоге сказано; «Перенести» — наверх «Готово»
    *status_c.lock().unwrap() = "Выполнена".into();
    statuses_down.store(true, Ordering::SeqCst);
    app.window.invoke_refresh_all();
    pump_until(&jobs, || !app.window.get_refreshing());
    let (heading, text, _, _) = dialog(&app);
    assert_eq!(heading, "Перенести закрытые в «Готово»?", "{text}");
    assert!(text.contains("\n\nСправочник статусов не получен — закрытые определены только по списку:\n"), "{text}");
    assert!(text.ends_with("Закрыты в Интрасервисе (1):\n#702190  c"), "{text}");
    app.window.invoke_dialog_accept();
    assert!(!app.window.get_dialog_open());
    assert_eq!(keys(&app, 3).first(), Some(&c), "наверх «Готово»");
    statuses_down.store(false, Ordering::SeqCst);
    app.save_now();
    let moved = file_tickets(&dir).into_iter().find(|t| t.intraservice_id == Some(702190)).unwrap();
    assert!(moved.status == Status::Done && moved.completed_at.is_some());

    // без API: F5 и импорт — так и сказано
    app.use_client(None);
    app.window.invoke_refresh_all();
    assert_eq!(dialog(&app).0, "Обновление статусов");
    assert_eq!(dialog(&app).1, "API не настроен: адрес, логин и пароль — в настройках TicketBoard");
    app.window.invoke_dialog_close();
    app.window.invoke_import_mine();
    assert_eq!(dialog(&app).0, "Импорт моих заявок");
    assert!(!app.window.get_importing() && !app.window.get_refreshing());
    app.window.invoke_dialog_close();
    // сервер сменили: свежесть была у прежнего — открытая карточка перечитана с нового, тихо
    let reads = |n: i32| asked(&format!("/api/task/{n}?"));
    let r70112 = reads(70112);
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(5))));
    pump_until(&jobs, || reads(70112) == r70112 + 1);

    // тихое чтение: через паузу после выбора — люди и поля с сервера, статус не тронут, в панели ни слова
    let b_reads = reads(702181);
    let picked = Instant::now();
    app.window.invoke_select(id('b').into());
    assert!(!app.fresh(&id('b')) && app.window.get_panel().creator.is_empty(), "до паузы не читается");
    pump_until(&jobs, || app.window.get_panel().creator == "Петрова А.");
    assert!(picked.elapsed() >= PANEL_PAUSE && reads(702181) == b_reads + 1);
    assert_eq!((app.window.get_panel().external_status.as_str(), app.window.get_sync_message().as_str()), ("", ""));
    assert!(app.dirty(), "поля карточки — в файл");
    // снова открыли до двух минут — второго запроса нет
    app.window.invoke_select(id('d').into());
    app.window.invoke_select(id('b').into());
    wait(PANEL_PAUSE + Duration::from_millis(150));
    wait(Duration::from_millis(100));
    assert_eq!(reads(702181), b_reads + 1, "свежая — не читается");
    // быстро перебрали — прочитана только последняя; не прочиталась (404) — откроют снова, спросим снова
    let (a_reads, c_reads, d_reads) = (reads(702180), reads(702190), reads(70119));
    app.window.invoke_select(id('a').into());
    app.window.invoke_select(c.as_str().into());
    app.window.invoke_select(id('d').into());
    pump_until(&jobs, || reads(70119) == d_reads + 1 && !app.fresh(&id('d')));
    assert_eq!((reads(702180), reads(702190)), (a_reads, c_reads), "прежние выборы не в счёт");
    app.window.invoke_select(id('d').into());
    pump_until(&jobs, || reads(70119) == d_reads + 2 && !app.fresh(&id('d')));
    // выбрали — и тут же добавили карточку окном (выбрана новая, без номера): прежняя не читается
    app.window.invoke_select(c.as_str().into());
    app.show_capture();
    app.capture.set_text("Позвонить в бухгалтерию".into());
    app.capture.invoke_edited("Позвонить в бухгалтерию".into());
    app.capture.invoke_accepted();
    wait(PANEL_PAUSE + Duration::from_millis(150));
    wait(Duration::from_millis(100));
    assert_eq!(reads(702190), c_reads, "выбрана другая — не читается");
    // ⟳ — свежая с начала запроса: открытие не читает второй раз; ⟳ не удался — свежей не стала
    app.window.invoke_select(id('a').into());
    app.window.invoke_sync();
    assert!(app.fresh(&id('a')));
    pump_until(&jobs, || !app.window.get_syncing());
    wait(PANEL_PAUSE + Duration::from_millis(150));
    wait(Duration::from_millis(100));
    assert_eq!(reads(702180), a_reads + 1, "одно чтение — у ⟳");
    app.window.invoke_select(id('d').into());
    app.window.invoke_sync();
    assert!(app.fresh(&id('d')));
    pump_until(&jobs, || !app.window.get_syncing());
    assert!(!app.fresh(&id('d')), "не прочиталась — не свежая");
    app.save_now();
    let b = file_tickets(&dir).into_iter().find(|t| t.intraservice_id == Some(702181)).unwrap();
    assert_eq!((b.creator.as_deref(), b.external_status.as_deref()), (Some("Петрова А."), None));

    // наступил новый день — возраст пересчитан; tick — по настоящим часам
    app.window.invoke_select(fresh.clone().into());
    let age = |app: &App| app.window.get_columns().row_data(0).unwrap().cards.row_data(0).unwrap().age.to_string();
    assert_eq!(age(&app), "сегодня");
    app.tick_at(chrono::Local::now().fixed_offset() + chrono::Duration::days(1));
    assert_eq!(age(&app), "1 д", "завтра ей день");
    app.tick();
    assert_eq!(age(&app), "сегодня", "tick — по часам");

    // цифры внизу
    assert!(app.window.get_metrics().contains("exe"), "{}", app.window.get_metrics());
    app.mark_shown();
    assert!(app.window.get_metrics().contains("окно на экране через"), "{}", app.window.get_metrics());
    app.add_notice("Ctrl+Alt+Space не занять");
    assert!(app.window.get_notice().starts_with("Ctrl+Alt+Space не занять. Доска"), "{}", app.window.get_notice());

    // доска чужая (TicketBoard.exe открыт): правки не принимаются, папка — байт в байт
    let other = dir.join("чужая");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("tickets.json"), BOARD).unwrap();
    let before = snapshot(&other);
    let ro = App::open(&other, false).unwrap();
    assert!(!ro.writable() && !ro.window.get_writable());
    assert!(ro.window.get_notice().contains("TicketBoard.exe открыт — доска только для чтения"), "{}", ro.window.get_notice());
    ro.window.invoke_select(id('a').into());
    ro.window.invoke_edit_title("чужое".into());
    ro.window.invoke_set_priority(0);
    ro.window.invoke_move_to(3);
    ro.window.invoke_add_note("нет".into());
    ro.window.invoke_delete_card();
    ro.window.invoke_new_ticket();
    assert!(!ro.window.get_dialog_open(), "удалять нечего");
    // ⟳ в режиме чтения: без API — так и сказано; с API — читается, но доска не меняется
    ro.window.invoke_sync();
    assert_eq!(ro.window.get_sync_message(), "API не настроен: адрес, логин и пароль — в настройках TicketBoard");
    // импорт и F5 — только у своей доски: ни окна, ни запросов; тихое чтение — тоже без запросов
    ro.window.invoke_import_mine();
    ro.window.invoke_refresh_all();
    key(ro.window.window(), Key::F5, false);
    assert!(!ro.window.get_dialog_open() && !ro.window.get_importing() && !ro.window.get_refreshing());
    // считаются только запросы, которые могла бы сделать она: тихое чтение своей доски может прийти в любой миг
    let ro_asks = || asked("/api/task?") + asked("/api/taskstatus") + asked("/api/user?") + asked("/api/task/702181?");
    let asked_before = ro_asks();
    ro.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(5))));
    ro.window.invoke_import_mine();
    ro.window.invoke_refresh_all();
    ro.window.invoke_select(id('b').into());
    wait(PANEL_PAUSE + Duration::from_millis(150));
    wait(Duration::from_millis(100));
    assert!(!ro.window.get_dialog_open(), "{}", ro.window.get_dialog_text());
    assert_eq!(ro_asks(), asked_before, "чужая доска ничего не спрашивает: {:?}", requests.lock().unwrap());
    ro.window.invoke_select(id('a').into());
    ro.window.invoke_sync();
    pump_until(&jobs, || !ro.window.get_syncing());
    assert_eq!(ro.window.get_panel().external_status, "", "чужая доска не меняется");
    assert_eq!(ro.window.get_title_text(), "Принтер в 305 не печатает");
    assert_eq!(ro.window.get_columns().row_data(0).unwrap().count, 2);
    wait(SAVE_DELAY + Duration::from_millis(150));
    ro.save_now();
    assert_eq!(snapshot(&other), before, "папка чужой доски — байт в байт");

    // настройки не прочитались — доска своя: причина — в шапке и в журнале
    let bad_settings = dir.join("настройки");
    std::fs::create_dir_all(&bad_settings).unwrap();
    std::fs::write(bad_settings.join("settings.json"), "{ \"WipLimit\": ").unwrap();
    let s = App::open(&bad_settings, true).unwrap();
    assert!(s.writable() && s.window.get_notice().starts_with("settings.json не прочитан"), "{}", s.window.get_notice());
    assert!(std::fs::read_to_string(bad_settings.join("errors.log")).unwrap().contains("settings.json не прочитан"));
    // доска пустая — F5 нечего перечитывать: так и сказано, запросов нет
    s.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(5))));
    let asked_before = asked("/api/taskstatus");
    s.window.invoke_refresh_all();
    assert_eq!(dialog(&s).1, "на доске нет заявок с номером — обновлять нечего");
    assert!(!s.window.get_refreshing() && asked("/api/taskstatus") == asked_before);

    // сменили сервер, пока шло тихое чтение, — ответ прежнего не про эту доску
    let switched = dir.join("смена");
    std::fs::create_dir_all(&switched).unwrap();
    let slow = r#"[{"Id": "00000000-0000-4000-8000-00000000000f", "Title": "Медленная", "Status": "Inbox", "IntraserviceId": 702199}]"#;
    std::fs::write(switched.join("tickets.json"), slow).unwrap();
    let q = App::open(&switched, true).unwrap();
    q.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(5))));
    q.window.invoke_select(id('f').into());
    pump_until(&jobs, || asked("/api/task/702199?") == 1);
    q.use_client(None);
    let until = Instant::now() + Duration::from_millis(900);
    pump_until(&jobs, || Instant::now() >= until);
    assert_eq!(q.window.get_panel().creator, "", "ответ прежнего сервера не применён");
    q.save_now();
    assert!(file_tickets(&switched)[0].creator.is_none());

    // файл не прочитался — только чтение: причина — в шапке, в папке ничего не тронуто (и журнала нет)
    let broken = dir.join("битая");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("tickets.json"), "[{\"Id\": ").unwrap();
    let before = snapshot(&broken);
    let bad = App::open(&broken, true).unwrap();
    assert!(!bad.writable());
    assert!(bad.window.get_notice().contains("TicketBoard.exe отложит его в сторону сам"), "{}", bad.window.get_notice());
    bad.window.invoke_select(id('a').into());
    bad.window.invoke_edit_title("x".into());
    bad.save_now();
    assert_eq!(snapshot(&broken), before, "битая доска — байт в байт, без errors.log");
    std::fs::remove_dir_all(&dir).unwrap();
}
