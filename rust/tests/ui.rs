//! Окно без экрана: программная отрисовка (`MinimalSoftwareWindow`), события мыши — прямо в окно. Одна проверка на всё:
//! платформа Slint ставится один раз на поток.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, PhysicalSize};
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};
use ticketboard::app::App;

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
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
    slint::platform::set_platform(Box::new(Headless { window: window.clone(), start: Instant::now() })).unwrap();
    window.set_size(PhysicalSize::new(1100, 720));

    let dir = std::env::temp_dir().join(format!("tb-rs-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), BOARD).unwrap();
    std::fs::write(
        dir.join("settings.json"),
        r#"{"WipLimit": 0, "IntraserviceBaseUrl": "https://hd", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}"#,
    )
    .unwrap();
    let before = snapshot(&dir);

    let app = App::open(&dir).unwrap();
    app.window.show().unwrap();
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

    // цифры внизу; прототип ничего не пишет
    assert!(app.window.get_metrics().contains("exe"), "{}", app.window.get_metrics());
    assert!(app.window.get_notice().contains("ничего не пишет"), "{}", app.window.get_notice());
    assert_eq!(snapshot(&dir), before, "папка данных — та же, байт в байт");
    std::fs::remove_dir_all(&dir).unwrap();
}
