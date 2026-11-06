//! Снимки окна без экрана (программная отрисовка): `cargo run --example shot -- <папка>` пишет board-light.png,
//! board-dark.png и drag.png — вид прототипа до запуска на Windows. Данные — обезличенный образец доски.

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};
use ticketboard::app::App;

struct Headless {
    windows: Rc<std::cell::RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    start: Instant,
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
}

/// Обезличенная доска: номера, названия и люди выдуманы.
fn sample(now: chrono::DateTime<chrono::FixedOffset>) -> String {
    let ago = |days: i64| (now - chrono::Duration::days(days)).to_rfc3339();
    let cards = [
        ("Inbox", 702180, "Не печатает принтер в бухгалтерии, 3 этаж", "High", 0, "Печать", "ООО Ромашка", 2, 1, 0),
        ("Inbox", 702191, "Новый сотрудник: учётная запись и почта", "Mid", 1, "Учётные записи", "", 0, 0, 1),
        ("Inbox", 702205, "Медленно открывается 1С", "Low", 2, "1С", "ООО Ромашка", 0, 0, 0),
        ("InProgress", 702011, "Настроить VPN на ноутбуке руководителя", "High", 3, "Удалённый доступ", "", 1, 2, 1),
        ("InProgress", 702087, "Сканер в приёмной не сохраняет в папку", "Mid", 1, "Печать", "", 0, 0, 0),
        ("InProgress", 702133, "Выдать права на папку «Договоры 2026»", "Mid", 0, "Файловые ресурсы", "ООО Лютик", 0, 0, 0),
        ("Waiting", 701954, "Не приходят письма с внешнего адреса", "High", 2, "Почта", "", 0, 3, 0),
        ("Waiting", 701990, "Заменить картридж — жду номер модели", "Low", 4, "Печать", "", 1, 0, 0),
        ("Done", 701876, "Перенести ярлыки на новый ПК", "Low", 1, "Рабочие места", "", 0, 0, 0),
        ("Done", 701902, "Сбросить пароль в домене", "Mid", 0, "Учётные записи", "", 0, 0, 0),
    ];
    let items: Vec<String> = cards
        .iter()
        .enumerate()
        .map(|(i, (status, id, title, priority, days, service, company, unread, files, notes))| {
            let completed = if *status == "Done" { format!(r#", "CompletedAt": "{}""#, ago(*days)) } else { String::new() };
            let files: Vec<String> = (0..*files).map(|f| format!(r#"{{"Id": {}, "Name": "файл{f}.png"}}"#, 900000 + f)).collect();
            let notes: Vec<String> = (0..*notes).map(|_| r#"{"Text": "позвонить"}"#.to_string()).collect();
            format!(
                r#"{{"Id": "k{i}", "Title": "{title}", "Status": "{status}", "Priority": "{priority}", "IntraserviceId": {id},
                   "StatusChangedAt": "{}"{completed}, "Service": "{service}", "CreatorCompany": "{company}", "UnreadComments": {unread},
                   "Files": [{}], "Notes": [{}], "ExternalStatus": "В работе", "Creator": "Петрова А."}}"#,
                ago(*days),
                files.join(","),
                notes.join(",")
            )
        })
        .collect();
    format!("[{}]", items.join(",\n"))
}

fn save(window: &MinimalSoftwareWindow, size: PhysicalSize, path: &Path) {
    let (w, h) = (size.width as usize, size.height as usize);
    let mut pixels = vec![PremultipliedRgbaColor::default(); w * h];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut pixels, w);
    });
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, 255]).collect();
    image::save_buffer(path, &bytes, size.width, size.height, image::ColorType::Rgba8).unwrap();
    println!("{}", path.display());
}

fn mouse(window: &slint::Window, x: f32, y: f32, press: Option<bool>) {
    let position = LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    match press {
        Some(true) => window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left }),
        Some(false) => window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left }),
        None => {}
    }
    slint::platform::update_timers_and_animations();
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let out = Path::new(&out);
    std::fs::create_dir_all(out).unwrap();
    let windows = Rc::new(std::cell::RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Headless { windows: windows.clone(), start: Instant::now() })).unwrap();

    let dir = std::env::temp_dir().join(format!("tb-rs-shot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), sample(chrono::Local::now().fixed_offset())).unwrap();
    std::fs::write(
        dir.join("settings.json"),
        r#"{"IntraserviceBaseUrl": "https://hd.example.ru", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}"#,
    )
    .unwrap();
    let app = App::open(&dir, true).unwrap();
    // как будто API настроен (снимок — без сети)
    app.use_client(Some(ticketboard::intraservice::Client::new("https://hd.example.ru", "me", "p", Duration::from_secs(1))));
    let (window, capture) = (windows.borrow()[0].clone(), windows.borrow()[1].clone());
    // снимки — вдвое крупнее (как экран 200 %): чётче на телефоне
    let scale = 2.0;
    let size = PhysicalSize::new(1100 * 2, 720 * 2);
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    window.set_size(size);
    app.window.show().unwrap();
    app.mark_shown();
    // цифры здесь — отладочной сборки под Linux: на снимке их нет, настоящие покажет окно на Windows
    app.window.set_metrics("exe … · память … · окно на экране через … — цифры покажет окно на Windows".into());
    let w = app.window.window();

    // перенос в процессе: карточка из «В работе» над «Ждёт ответа» (панель закрыта)
    mouse(w, 370.0, 110.0, Some(true));
    for (x, y) in [(380.0, 115.0), (450.0, 160.0), (560.0, 220.0), (640.0, 260.0)] {
        mouse(w, x, y, None);
    }
    save(&window, size, &out.join("drag.png"));
    mouse(w, 640.0, 260.0, Some(false));

    // выбранная карточка — панель справа; переписка — как её собирает app.rs (люди и номера выдуманы)
    mouse(w, 100.0, 110.0, Some(true));
    mouse(w, 100.0, 110.0, Some(false));
    app.window.set_conversation(
        "Свежие сверху: 2 комментария, 1 смена статуса\n\n06.11.2026 09:40 · Петрова А.\nПерезагрузили, не помогло.\n\n\
         06.11.2026 09:12 · Иванов И. И. · внутренний\nПроверить драйвер"
            .into(),
    );
    save(&window, size, &out.join("board-light.png"));
    app.window.global::<ticketboard::Palette>().set_color_scheme(slint::language::ColorScheme::Dark);
    save(&window, size, &out.join("board-dark.png"));
    app.window.global::<ticketboard::Palette>().set_color_scheme(slint::language::ColorScheme::Light);

    // быстрое добавление: номер распознан, название — из Интрасервиса
    let small = PhysicalSize::new(520 * 2, 180 * 2);
    capture.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    capture.set_size(small);
    app.show_capture();
    app.capture.set_text("https://hd.example.ru/Task/View/702185".into());
    app.capture.set_number_text("#702185".into());
    app.capture.set_preview("Не открывается 1С на складе".into());
    app.capture.set_hint("Будет создана заявка с этим названием".into());
    save(&capture, small, &out.join("capture.png"));
    std::fs::remove_dir_all(&dir).unwrap();
}
