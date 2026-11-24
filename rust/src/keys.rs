//! Сочетания клавиш в полях при русской раскладке. Slint узнаёт Ctrl+V, C, X, A, Z, Y по тексту клавиши («v»), а у той же
//! клавиши на русской раскладке текст «м» — и сочетания не работали ни в одном поле (у C# работают: WPF смотрит на
//! клавишу, а не на букву). Окно перехватывает Ctrl+русская буква до поля и подаёт ту же клавишу латиницей.

use slint::platform::WindowEvent;

/// Та же клавиша на английской раскладке (ЙЦУКЕН над QWERTY) — только сочетания полей ввода; иначе None. Большая буква
/// (Caps Lock) — то же.
// ponytail: только русская раскладка — у других свои буквы на этих клавишах; понадобятся — добавить сюда
pub fn latin(text: &str) -> Option<&'static str> {
    Some(match text {
        "м" | "М" => "v",
        "с" | "С" => "c",
        "ч" | "Ч" => "x",
        "ф" | "Ф" => "a",
        "я" | "Я" => "z",
        "н" | "Н" => "y",
        _ => return None,
    })
}

/// Ctrl+русская буква сочетания — подать окну ту же клавишу латиницей (Ctrl ещё нажат: Slint узнает сочетание и сделает
/// его в поле с фокусом); true — подали, исходную клавишу окно глотает.
pub fn press_latin(window: &slint::Window, text: &str) -> bool {
    let Some(latin) = latin(text) else { return false };
    window.dispatch_event(WindowEvent::KeyPressed { text: latin.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: latin.into() });
    true
}

#[cfg(test)]
mod tests {
    use super::latin;

    #[test]
    fn russian_letters_of_the_edit_keys_become_latin() {
        let pairs = [("м", "v"), ("с", "c"), ("ч", "x"), ("ф", "a"), ("я", "z"), ("н", "y")];
        for (ru, en) in pairs {
            assert_eq!(latin(ru), Some(en), "{ru}");
            assert_eq!(latin(&ru.to_uppercase()), Some(en), "{ru} при Caps Lock");
        }
        for other in ["v", "a", "к", "ы", "", "мм", "1"] {
            assert_eq!(latin(other), None, "{other:?}");
        }
    }
}
