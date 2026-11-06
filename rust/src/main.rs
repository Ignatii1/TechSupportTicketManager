//! Запуск прототипа: окно доски, трей, глобальная клавиша Ctrl+Alt+Shift+Space. Закрытие окна прячет его в трей, выход —
//! из меню трея, как у TicketBoard.exe.

#![cfg_attr(windows, windows_subsystem = "windows")]

use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
use std::rc::Rc;
use std::time::Duration;
use ticketboard::app::App;
use ticketboard::{MainWindow, Tray};

/// Показать окно доски (из трея) или переключить: видно — спрятать, нет — показать (глобальная клавиша).
fn toggle(w: &MainWindow, show_only: bool) {
    let window = w.window();
    if !show_only && window.is_visible() && !window.is_minimized() {
        let _ = window.hide();
    } else {
        let _ = w.show();
        window.set_minimized(false);
    }
}

/// Окно с ошибкой: у exe нет консоли, без него падение было бы молчаливым. Прототип ничего не пишет — и журнала тоже.
#[cfg(windows)]
fn fail(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MessageBoxW};
    use windows::core::HSTRING;
    // SAFETY: строки живут до конца вызова
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Заявки — прототип на Rust"), MB_ICONERROR) };
}

#[cfg(not(windows))]
fn fail(text: &str) {
    eprintln!("{text}");
}

fn main() {
    std::panic::set_hook(Box::new(|info| fail(&format!("Прототип упал:\n{info}"))));
    if let Err(e) = run() {
        fail(&format!("Окно не открылось: {e}"));
    }
}

fn run() -> Result<(), slint::PlatformError> {
    // данные — рядом с exe, как у TicketBoard.exe (App.DataDir = папка exe)
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| ".".into());
    let app = Rc::new(App::open(&dir)?);
    app.window.window().on_close_requested(|| CloseRequestResponse::HideWindow);
    app.window.show()?;
    // окно на экране — на следующем шаге цикла событий после show
    let shown = app.clone();
    Timer::single_shot(Duration::ZERO, move || shown.mark_shown());
    // память — раз в 2 с (и в трее: её и сравнивать)
    let memory = Timer::default();
    let watched = app.clone();
    memory.start(TimerMode::Repeated, Duration::from_secs(2), move || watched.update_metrics());

    let tray = Tray::new()?;
    let weak = app.window.as_weak();
    tray.on_show_board(move || {
        if let Some(w) = weak.upgrade() {
            toggle(&w, true);
        }
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    #[cfg(windows)]
    let _hotkey = match hotkey::register(app.window.as_weak()) {
        Ok(h) => Some(h),
        Err(e) => {
            app.add_notice(&e);
            None
        }
    };

    slint::run_event_loop_until_quit()
}

#[cfg(windows)]
mod hotkey {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use ticketboard::MainWindow;

    /// Своя клавиша: Ctrl+Alt+Space держит работающий TicketBoard.exe.
    pub fn register(window: slint::Weak<MainWindow>) -> Result<GlobalHotKeyManager, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| format!("глобальная клавиша не работает: {e}"))?;
        let key = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT), Code::Space);
        manager.register(key).map_err(|e| format!("Ctrl+Alt+Shift+Space не занять: {e}"))?;
        let id = key.id();
        // обработчик — в потоке окна (сообщение WM_HOTKEY), но вызывается из библиотеки: в окно — через цикл событий
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.id == id && e.state == HotKeyState::Pressed {
                let _ = window.upgrade_in_event_loop(|w| super::toggle(&w, false));
            }
        }));
        Ok(manager)
    }
}
