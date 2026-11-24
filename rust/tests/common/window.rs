//! Окно без экрана для проверок окон (`tests/ui.rs`, `tests/finder.rs`): программная отрисовка
//! (`MinimalSoftwareWindow`), ответы потоков — очередь, которую `pump_until` выполняет, как цикл событий. Платформа Slint
//! ставится один раз на процесс — поэтому в файле проверок окна одна проверка на всё.

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Jobs = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;
pub type Windows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

/// Окна без экрана: у каждого компонента своё (доска, быстрое добавление), по порядку создания.
pub struct Headless {
    pub windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    pub start: Instant,
    /// Что потоки прислали в окно (`upgrade_in_event_loop`): выполняет `pump`, как цикл событий.
    pub jobs: Jobs,
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

/// Буфер обмена окон (вставка и копирование в полях Slint); у настоящего окна — буфер Windows.
static CLIPBOARD: Mutex<Option<String>> = Mutex::new(None);

pub fn set_clipboard(text: &str) {
    *CLIPBOARD.lock().unwrap() = Some(text.to_string());
}

pub fn clipboard() -> Option<String> {
    CLIPBOARD.lock().unwrap().clone()
}

impl Platform for Headless {
    fn set_clipboard_text(&self, text: &str, which: slint::platform::Clipboard) {
        if which == slint::platform::Clipboard::DefaultClipboard {
            set_clipboard(text);
        }
    }
    fn clipboard_text(&self, which: slint::platform::Clipboard) -> Option<String> {
        (which == slint::platform::Clipboard::DefaultClipboard).then(clipboard).flatten()
    }
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
pub fn pump_until(jobs: &Jobs, done: impl Fn() -> bool) {
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
pub fn wait(d: Duration) {
    std::thread::sleep(d);
    slint::platform::update_timers_and_animations();
}

/// Поставить платформу без экрана; окна — по порядку создания, очередь ответов потоков.
pub fn install() -> (Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>, Jobs) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let jobs = Jobs::default();
    slint::platform::set_platform(Box::new(Headless { windows: windows.clone(), start: Instant::now(), jobs: jobs.clone() })).unwrap();
    (windows, jobs)
}
