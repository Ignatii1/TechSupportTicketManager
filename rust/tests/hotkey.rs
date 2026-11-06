//! Клавиша из настроек — как `HotkeyService.TryParse`.

use ticketboard::hotkey::*;

fn mods(ctrl: bool, shift: bool, alt: bool, win: bool) -> Mods {
    Mods { ctrl, shift, alt, win }
}

#[test]
fn modifiers_and_one_key() {
    assert_eq!(parse("Ctrl+Alt+Space"), Some((mods(true, false, true, false), Key::Space)), "по умолчанию у C#");
    assert_eq!(parse(" control + shift + f5 "), Some((mods(true, true, false, false), Key::F(5))), "регистр и пробелы");
    assert_eq!(parse("Win+D1"), Some((mods(false, false, false, true), Key::Digit(1))));
    assert_eq!((parse("Ctrl+D0").map(|k| k.1), parse("Ctrl+D7").map(|k| k.1)), (Some(Key::Digit(0)), Some(Key::Digit(7))));
    assert_eq!(parse("Windows+Alt+NumPad0"), Some((mods(false, false, true, true), Key::NumPad(0))));
    assert_eq!(parse("Ctrl+Shift+PageUp").map(|k| k.1), Some(Key::PageUp));
    assert_eq!(parse("Ctrl+Prior").map(|k| k.1), Some(Key::PageUp), "второе имя WPF");
    assert_eq!(parse("Ctrl+Next").map(|k| k.1), Some(Key::PageDown));
    assert_eq!(parse("Alt+Return").map(|k| k.1), Some(Key::Enter));
    assert_eq!(parse("Ctrl+q").map(|k| k.1), Some(Key::Letter(b'Q')));
    assert_eq!(parse("Ctrl+A+B").map(|k| k.1), Some(Key::Letter(b'B')), "несколько — последняя");
    assert_eq!(parse("Ctrl+F24").map(|k| k.1), Some(Key::F(24)));
    let named = ["Left", "Up", "Right", "Down", "Insert", "Delete", "Home", "End", "PageDown", "Enter", "Tab"];
    let keys =
        [Key::Left, Key::Up, Key::Right, Key::Down, Key::Insert, Key::Delete, Key::Home, Key::End, Key::PageDown, Key::Enter, Key::Tab];
    for (n, k) in named.iter().zip(keys) {
        assert_eq!(parse(&format!("Ctrl+{n}")).map(|k| k.1), Some(k), "{n}");
    }
}

#[test]
fn not_understood() {
    for bad in [
        "",
        "Ctrl+Shift",
        "Ctrl+OemQuestion",
        "Ctrl+F25",
        "Ctrl+F0",
        "Ctrl+F01",
        "Ctrl+D10",
        "Ctrl+NumPad10",
        "Ctrl+Пробел",
        "Ctrl+65",
        "Ctrl+AB",
    ] {
        assert_eq!(parse(bad), None, "«{bad}»");
    }
    assert_eq!(ticketboard::hotkey::not_understood("Ctrl+?"), "Не понял хоткей «Ctrl+?». Пример: Ctrl+Shift+Space");
}
