//! Запуск прототипа: окно доски, трей, глобальная клавиша (своя доска — быстрое добавление по клавише из настроек;
//! только чтение — Ctrl+Alt+Shift+Space показывает доску). Закрытие окна прячет его в трей, выход — из меню трея, как у
//! TicketBoard.exe; при выходе — запись доски.

#![cfg_attr(windows, windows_subsystem = "windows")]

use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
use std::rc::Rc;
use std::time::Duration;
use ticketboard::app::App;
use ticketboard::metrics::since_start;
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

/// Окно с ошибкой: у exe нет консоли, без него падение было бы молчаливым. В `errors.log` — только когда доска своя
/// (`run` ставит свой обработчик): в режиме чтения прототип ничего не пишет.
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
    // запуск по этапам (строка цифр): до `main` — загрузка exe и библиотек, проверка антивирусом
    let started = since_start();
    std::panic::set_hook(Box::new(|info| fail(&format!("Прототип упал:\n{info}"))));
    if let Err(e) = run(started) {
        fail(&format!("Окно не открылось: {e}"));
    }
}

fn run(started: Option<Duration>) -> Result<(), slint::PlatformError> {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| ".".into());
    // доска своя, пока жив `owner`: TicketBoard.exe в это время не запустится; держит его он — только чтение
    let owner = ticketboard::single::take();
    let app = Rc::new(App::open(&dir, owner.is_some())?);
    app.startup_stage("до main", started);
    app.startup_stage("доска и окна", since_start());
    // доска не наша (файл не прочитался) — мьютекс отпустить сразу: TicketBoard.exe должен запуститься и отложить файл
    let _owner = owner.filter(|_| app.writable()); // до конца `run`: после записи доски
    if app.writable() {
        let journal = dir.clone();
        std::panic::set_hook(Box::new(move |info| {
            ticketboard::journal::append(&journal, &format!("Прототип упал: {info}"));
            fail(&format!("Прототип упал:\n{info}"));
        }));
    }
    app.window.window().on_close_requested(|| CloseRequestResponse::HideWindow);
    app.window.show()?;
    app.startup_stage("окно", since_start());
    let shown = app.clone();
    Timer::single_shot(Duration::ZERO, move || shown.mark_shown());
    let memory = Timer::default();
    let watched = app.clone();
    memory.start(TimerMode::Repeated, Duration::from_secs(2), move || watched.update_metrics());
    let day = Timer::default();
    let aged = app.clone();
    day.start(TimerMode::Repeated, Duration::from_secs(60), move || aged.tick());

    let tray = Tray::new()?;
    let weak = app.window.as_weak();
    tray.on_show_board(move || {
        if let Some(w) = weak.upgrade() {
            toggle(&w, true);
        }
    });
    // импорт и F5 из трея: итог — поверх доски, так что сначала показать её
    let weak = app.window.as_weak();
    tray.on_import_mine(move || {
        if let Some(w) = weak.upgrade() {
            toggle(&w, true);
            w.invoke_import_mine();
        }
    });
    let weak = app.window.as_weak();
    tray.on_refresh_all(move || {
        if let Some(w) = weak.upgrade() {
            toggle(&w, true);
            w.invoke_refresh_all();
        }
    });
    tray.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    #[cfg(windows)]
    let _hotkey = match hotkey::register(&app) {
        Ok(h) => Some(h),
        Err(e) => {
            app.add_notice(&e);
            None
        }
    };

    app.startup_stage("трей и клавиша", since_start());
    slint::run_event_loop_until_quit()?;
    app.save_now(); // правки последних 600 мс
    Ok(())
}

#[cfg(windows)]
mod hotkey {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use slint::ComponentHandle;
    use std::str::FromStr;
    use ticketboard::app::App;
    use ticketboard::hotkey::{Key, parse};

    /// Код клавиши для Windows — по имени W3C (`KeyA`, `Digit1`, `F5`, `Numpad0`…).
    fn code(key: Key) -> Option<Code> {
        let name = match key {
            Key::Space => "Space".into(),
            Key::Letter(c) => format!("Key{}", c as char),
            Key::Digit(d) => format!("Digit{d}"),
            Key::F(n) => format!("F{n}"),
            Key::NumPad(d) => format!("Numpad{d}"),
            Key::Left => "ArrowLeft".into(),
            Key::Up => "ArrowUp".into(),
            Key::Right => "ArrowRight".into(),
            Key::Down => "ArrowDown".into(),
            Key::Insert => "Insert".into(),
            Key::Delete => "Delete".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::PageUp => "PageUp".into(),
            Key::PageDown => "PageDown".into(),
            Key::Enter => "Enter".into(),
            Key::Tab => "Tab".into(),
        };
        Code::from_str(&name).ok()
    }

    /// Доска своя — клавиша из настроек открывает быстрое добавление, как у TicketBoard.exe; только чтение —
    /// Ctrl+Alt+Shift+Space показывает доску (клавиша TicketBoard.exe занята им самим).
    pub fn register(app: &App) -> Result<GlobalHotKeyManager, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| format!("глобальная клавиша не работает: {e}"))?;
        let (text, key) = if app.writable() {
            let text = app.hotkey();
            let (mods, key) = parse(&text).ok_or_else(|| ticketboard::hotkey::not_understood(&text))?;
            let mut m = Modifiers::empty();
            m.set(Modifiers::CONTROL, mods.ctrl);
            m.set(Modifiers::SHIFT, mods.shift);
            m.set(Modifiers::ALT, mods.alt);
            m.set(Modifiers::META, mods.win);
            let code = code(key).ok_or_else(|| ticketboard::hotkey::not_understood(&text))?;
            (text, HotKey::new(Some(m), code))
        } else {
            ("Ctrl+Alt+Shift+Space".to_string(), HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT), Code::Space))
        };
        manager.register(key).map_err(|e| format!("Хоткей «{text}» занят другой программой: {e}"))?;
        let (id, capture, window) = (key.id(), app.writable(), app.window.as_weak());
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.id == id && e.state == HotKeyState::Pressed {
                let _ = window.upgrade_in_event_loop(move |w| if capture { w.invoke_new_ticket() } else { super::toggle(&w, false) });
            }
        }));
        Ok(manager)
    }
}
