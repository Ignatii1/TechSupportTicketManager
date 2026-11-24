//! Сочетания клавиш в полях при русской раскладке (rust-0.9.1, найдено на рабочем ПК): Slint узнаёт Ctrl+V по тексту
//! клавиши («v»), а у той же клавиши на русской раскладке текст «м» — Ctrl+V, C, X, A, Z не работали ни в одном поле
//! прототипа. У C# работают: WPF смотрит на клавишу, а не на букву. Одна проверка на процесс — платформа ставится раз.

mod common;

use common::window::{clipboard, install, set_clipboard};
use slint::platform::{Key, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, SharedString};
use ticketboard::app::App;

/// Клавиша с Ctrl (и Alt, если `alt`): нажать и отпустить, как клавиатура.
fn ctrl(window: &slint::Window, text: &str, alt: bool) {
    let text = SharedString::from(text);
    window.dispatch_event(WindowEvent::KeyPressed { text: Key::Control.into() });
    if alt {
        window.dispatch_event(WindowEvent::KeyPressed { text: Key::Alt.into() });
    }
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    if alt {
        window.dispatch_event(WindowEvent::KeyReleased { text: Key::Alt.into() });
    }
    window.dispatch_event(WindowEvent::KeyReleased { text: Key::Control.into() });
    slint::platform::update_timers_and_animations();
}

/// Щелчок левой кнопкой в точке окна.
fn click(window: &slint::Window, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    let button = slint::platform::PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// Клавиша без Ctrl — буква в поле.
fn plain(window: &slint::Window, text: &str) {
    let text = SharedString::from(text);
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn russian_layout_shortcuts_work_in_every_window() {
    let (windows, _jobs) = install();
    let dir = std::env::temp_dir().join(format!("tb-rs-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    let app = App::open(&dir, true).unwrap();
    app.window.show().unwrap();
    let board = windows.borrow()[0].clone();
    let w = board.window();

    // доска: «/» — в поле поиска; Ctrl+«м» (клавиша V) — вставить
    board.set_size(slint::PhysicalSize::new(1100, 720));
    click(w, 550.0, 400.0); // фокус — доске (щелчок по ней), потом «/»
    plain(w, "/");
    set_clipboard("принтер");
    ctrl(w, "м", false);
    assert_eq!(app.window.get_search_text(), "принтер", "Ctrl+V на русской раскладке вставляет");
    // Ctrl+«ф» (A) — выделить всё, Ctrl+«с» (C) — копировать
    set_clipboard("");
    ctrl(w, "ф", false);
    ctrl(w, "с", false);
    assert_eq!(clipboard().as_deref(), Some("принтер"), "Ctrl+A и Ctrl+C на русской раскладке");
    // Ctrl+«ч» (X) — вырезать, Ctrl+«я» (Z) — отменить
    set_clipboard("");
    ctrl(w, "ф", false);
    ctrl(w, "ч", false);
    assert_eq!((app.window.get_search_text().as_str(), clipboard().as_deref()), ("", Some("принтер")), "Ctrl+X вырезает");
    ctrl(w, "я", false);
    assert_eq!(app.window.get_search_text(), "принтер", "Ctrl+Z возвращает вырезанное");
    // Caps Lock: «М» большая — то же сочетание
    set_clipboard("сеть");
    ctrl(w, "ф", false);
    ctrl(w, "М", false);
    assert_eq!(app.window.get_search_text(), "сеть", "Ctrl+V при Caps Lock");
    // без Ctrl «м» — просто буква; Ctrl+Alt (AltGr) — не сочетание: буфер не вставлен
    plain(w, "м");
    assert_eq!(app.window.get_search_text(), "сетьм", "без Ctrl — буква");
    set_clipboard("ZZZ");
    ctrl(w, "м", true);
    assert!(!app.window.get_search_text().contains("ZZZ"), "Ctrl+Alt — не вставка: {}", app.window.get_search_text());
    // латиница — как и была
    set_clipboard("ok");
    ctrl(w, "ф", false);
    ctrl(w, "v", false);
    assert_eq!(app.window.get_search_text(), "ok", "Ctrl+V на английской раскладке");

    // вопрос или отчёт поверх доски: его текст выделяется и копируется — и по-русски
    let report: String = (1..=30).map(|i| format!("строка отчёта {i}\n")).collect();
    app.window.set_dialog_heading("Итог".into());
    app.window.set_dialog_text(report.as_str().into());
    app.window.set_dialog_open(true);
    click(w, 550.0, 350.0);
    set_clipboard("");
    ctrl(w, "ф", false);
    ctrl(w, "с", false);
    assert_eq!(clipboard().as_deref(), Some(report.as_str()), "Ctrl+A и Ctrl+C в отчёте поверх доски");
    app.window.set_dialog_open(false);

    // быстрое добавление: поле ссылки
    app.show_capture();
    let capture = windows.borrow()[1].clone();
    set_clipboard("702185");
    ctrl(capture.window(), "м", false);
    assert!(app.capture.get_text().contains("702185"), "быстрое добавление: {}", app.capture.get_text());

    // окно поиска: поле слов (фокус в нём при открытии)
    app.window.invoke_open_search();
    let search = windows.borrow()[2].clone();
    set_clipboard("сервер");
    ctrl(search.window(), "ф", false);
    ctrl(search.window(), "м", false);
    assert_eq!(app.finder.get_words(), "сервер", "окно поиска: Ctrl+V в поле слов");
    let _ = std::fs::remove_dir_all(&dir);
}
