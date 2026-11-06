//! Окно без экрана: программная отрисовка (`MinimalSoftwareWindow`), события мыши — прямо в окно. Одна проверка на всё:
//! платформа Slint ставится один раз на поток.

mod common;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, PhysicalSize};
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ticketboard::app::App;

type Jobs = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
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
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Queue(self.jobs.clone())))
    }
}

/// Дождаться, пока поток пришлёт ответ в окно, и выполнить его (до 15 с).
fn pump(jobs: &Jobs) {
    let until = Instant::now() + Duration::from_secs(15);
    while jobs.lock().unwrap().is_empty() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let pending = std::mem::take(&mut *jobs.lock().unwrap());
    assert!(!pending.is_empty(), "поток так и не ответил");
    for job in pending {
        job();
    }
}

const BOARD: &str = r#"[
  {"Id": "a", "Title": "Принтер в 305 не печатает", "Status": "Inbox", "IntraserviceId": 702180, "Priority": "High",
   "StatusChangedAt": "2026-01-01T09:00:00+03:00", "Service": "Печать", "UnreadComments": 2},
  {"Id": "b", "Title": "Нет доступа к папке", "Status": "Inbox", "IntraserviceId": 702181},
  {"Id": "c", "Title": "Своя задача", "Status": "InProgress"},
  {"Id": "d", "Title": "Ждём ответа", "Status": "Waiting", "IntraserviceId": 702183},
  {"Id": "e", "Title": "Давно сделано", "Status": "Done", "CompletedAt": "2025-01-01T00:00:00+03:00"}
]"#;

fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
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

#[test]
fn the_board_window() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let jobs = Jobs::default();
    slint::platform::set_platform(Box::new(Headless { window: window.clone(), start: Instant::now(), jobs: jobs.clone() })).unwrap();
    window.set_size(PhysicalSize::new(1100, 720));

    let dir = std::env::temp_dir().join(format!("tb-rs-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), BOARD).unwrap();
    // на Windows пароль шифруется по-настоящему и читается с поддельного сервера; здесь DPAPI нет — чтение скажет об этом
    #[cfg(windows)]
    let (url, password) = (
        common::route(|path| {
            common::Reply::json(match path {
                p if p.starts_with("/api/tasklifetime?") => common::LIVE_LIFE,
                "/api/taskstatus" => r#"[{"Id":31,"Name":"Открыта"},{"Id":28,"Name":"Закрыта","IsFinal":true}]"#,
                "/api/user?getcurrentuserinfo=true" => r#"{"Id":40800,"Name":"Иванов Иван Иванович"}"#,
                p if p.starts_with("/api/task?") => {
                    r#"{"Tasks":[{"Id":702180,"Name":"a"},{"Id":702181,"Name":"b"},{"Id":702190,"Name":"c"}]}"#
                }
                _ => common::LIVE,
            })
        })
        .0,
        ticketboard::dpapi::protect("p").unwrap(),
    );
    #[cfg(not(windows))]
    let (url, password) = ("https://hd".to_string(), "AQAA".to_string());
    std::fs::write(
        dir.join("settings.json"),
        format!(
            r#"{{"WipLimit": 0, "IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "{password}"}}"#
        ),
    )
    .unwrap();
    let before = snapshot(&dir);

    let app = App::open(&dir).unwrap();
    app.window.show().unwrap();
    assert_eq!(app.window.get_detail_title(), "Выберите карточку", "сначала ничего не выбрано");
    // раскладка и первый кадр
    window.draw_if_needed(|r| {
        let mut buf = vec![slint::platform::software_renderer::PremultipliedRgbaColor::default(); 1100 * 720];
        r.render(&mut buf, 1100);
    });

    // колонки и счётчики; старая «Готово» скрыта; лимит 0 — «В работе» перегружена
    let cols = app.window.get_columns();
    let titles: Vec<String> = cols.iter().map(|c| c.title.to_string()).collect();
    assert_eq!(titles, ["Входящие", "В работе", "Ждёт ответа", "Готово"]);
    assert_eq!(counts(&app), [2, 1, 1, 0]);
    assert_eq!(keys(&app, 0), ["a", "b"]);
    assert!(cols.row_data(1).unwrap().overloaded);
    assert_eq!(cols.row_data(3).unwrap().hint, "скрыты старше 7 д");
    let a = cols.row_data(0).unwrap().cards.row_data(0).unwrap();
    assert_eq!((a.number.as_str(), a.priority.as_str(), a.priority_level, a.age_state, a.unread), ("#702180", "Высокий", 2, 2, 2));
    assert_eq!(a.context, "Печать");

    // щелчок по первой карточке «Входящих» — выбор, не перенос
    let w = app.window.window();
    mouse(w, &[(100.0, 110.0, Some(true)), (100.0, 110.0, Some(false))]);
    assert_eq!(app.window.get_selected_key(), "a");
    assert!(app.window.get_detail_title().contains("Принтер в 305"), "{}", app.window.get_detail_title());
    assert!(app.window.get_can_read(), "номер и API есть — читать можно");
    assert_eq!(counts(&app), [2, 1, 1, 0]);

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
    assert_eq!(keys(&app, 0), ["b"]);
    assert_eq!(keys(&app, 2), ["a", "d"]);
    assert_eq!(counts(&app), [1, 1, 2, 0]);

    // отпустили над той же колонкой — ничего
    mouse(w, &[(680.0, 110.0, Some(true)), (690.0, 120.0, None), (700.0, 260.0, None), (700.0, 260.0, Some(false))]);
    assert_eq!(keys(&app, 2), ["a", "d"]);

    // своя карточка без номера — читать нечего
    mouse(w, &[(370.0, 110.0, Some(true)), (370.0, 110.0, Some(false))]);
    assert_eq!(app.window.get_selected_key(), "c");
    assert!(!app.window.get_can_read());

    assert!(app.window.get_detail_text().contains("Без номера"), "{}", app.window.get_detail_text());

    // «Прочитать из Интрасервиса»: ответ приходит из потока и встаёт под выбранную карточку
    app.window.invoke_select("a".into());
    assert!(app.window.get_can_read());
    app.window.invoke_read_ticket();
    assert!(app.window.get_reading(), "пока читается — кнопка занята");
    pump(&jobs);
    let result = app.window.get_read_result().to_string();
    assert!(!app.window.get_reading());
    // карточка целиком: поля, описание без html, переписка
    #[cfg(windows)]
    assert!(
        result.contains("Склад: удалить задания")
            && result.contains("Закрыта")
            && result.contains("Описание:\nстрока 1\nстрока 2")
            && result.contains("Переписка, свежие сверху: 2 комментария")
            && result.contains("Сидорова Мария Ивановна"),
        "{result}"
    );
    #[cfg(not(windows))]
    assert!(result.starts_with("Не прочиталась") && result.contains("только в Windows"), "{result}");
    assert!(result.contains("Сеть: напрямую"), "{result}");

    // «Проверить подключение»: кто я и мои открытые — не только у выбранной карточки
    assert!(app.window.get_can_check(), "API есть — проверить можно");
    app.window.invoke_check_connection();
    assert!(app.window.get_reading(), "пока проверяется — кнопки заняты");
    pump(&jobs);
    let report = app.window.get_read_result().to_string();
    assert!(!app.window.get_reading());
    #[cfg(windows)]
    assert!(
        report.contains("Вы — Иванов Иван Иванович (№40800)")
            && report.contains("Мои открытые заявки: 3 — на доске 2, нет на доске 1: #702190."),
        "{report}"
    );
    #[cfg(not(windows))]
    assert!(report.starts_with("Подключения нет") && report.contains("только в Windows"), "{report}");
    // ответ пришёл, когда выбрана уже другая карточка, — у неё не показывается
    app.window.invoke_read_ticket();
    app.window.invoke_select("d".into());
    pump(&jobs);
    assert_eq!(app.window.get_read_result(), "", "ответ для «a» не встал под «d»");
    assert!(!app.window.get_reading());

    // наступил новый день — возраст пересчитан (окно живёт в трее сутками); тот же день — ничего не трогается
    let age = |app: &App| app.window.get_columns().row_data(2).unwrap().cards.row_data(0).unwrap().age.to_string();
    assert_eq!(age(&app), "сегодня", "«a» перенесена сегодня");
    app.tick_at(chrono::Local::now().fixed_offset());
    assert_eq!(age(&app), "сегодня");
    app.tick_at(chrono::Local::now().fixed_offset() + chrono::Duration::days(1));
    assert_eq!(age(&app), "1 д", "завтра ей день");

    // цифры внизу; шапка; прототип ничего не пишет
    assert!(app.window.get_metrics().contains("exe"), "{}", app.window.get_metrics());
    assert!(!app.window.get_metrics().contains("окно на экране"));
    app.mark_shown();
    assert!(app.window.get_metrics().contains("окно на экране через"), "{}", app.window.get_metrics());
    assert!(app.window.get_notice().contains("ничего не пишет"), "{}", app.window.get_notice());
    app.add_notice("Ctrl+Alt+Shift+Space не занять");
    assert!(app.window.get_notice().starts_with("Ctrl+Alt+Shift+Space не занять. Прототип"), "{}", app.window.get_notice());
    assert_eq!(snapshot(&dir), before, "папка данных — та же, байт в байт");
    std::fs::remove_dir_all(&dir).unwrap();
}
