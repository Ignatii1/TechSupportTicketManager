//! Глобальная клавиша из `settings.json` (`Hotkey`, «Ctrl+Alt+Space») — разбор как у `HotkeyService.TryParse`:
//! модификаторы Ctrl/Control, Shift, Alt, Win/Windows и одна клавиша по имени WPF без учёта регистра (несколько — берётся
//! последняя). Здесь — без Windows: что за клавиша; зарегистрировать её — `main.rs`. ponytail: имён WPF много, здесь —
//! те, что ставят на глобальную клавишу; прочие (`Oem…`, номер вместо имени) — «не понял».

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Space,
    /// A–Z
    Letter(u8),
    /// D0–D9
    Digit(u8),
    /// F1–F24
    F(u8),
    /// NumPad0–NumPad9
    NumPad(u8),
    Left,
    Up,
    Right,
    Down,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Tab,
}

/// Клавиша по имени WPF (`Key`), без учёта регистра.
fn key(name: &str) -> Option<Key> {
    let n = name.to_ascii_lowercase();
    // «F1», «NumPad0»: цифры без ведущего нуля, не больше max
    let number = |prefix: &str, max: u8| {
        let d = n.strip_prefix(prefix)?;
        let plain = !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) && (d == "0" || !d.starts_with('0'));
        d.parse::<u8>().ok().filter(|&v| plain && v <= max)
    };
    Some(match n.as_str() {
        "space" => Key::Space,
        "left" => Key::Left,
        "up" => Key::Up,
        "right" => Key::Right,
        "down" => Key::Down,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "prior" => Key::PageUp,
        "pagedown" | "next" => Key::PageDown,
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        _ if n.len() == 1 && n.as_bytes()[0].is_ascii_lowercase() => Key::Letter(n.as_bytes()[0].to_ascii_uppercase()),
        _ if n.len() == 2 && n.starts_with('d') && n.as_bytes()[1].is_ascii_digit() => Key::Digit(n.as_bytes()[1] - b'0'),
        _ if n.starts_with("numpad") && n.len() == 7 => Key::NumPad(number("numpad", 9)?),
        _ if n.starts_with('f') && (2..=3).contains(&n.len()) => Key::F(number("f", 24).filter(|&f| f >= 1)?),
        _ => return None,
    })
}

/// «Ctrl+Shift+Space» → модификаторы и клавиша; без клавиши или с незнакомым именем — None.
pub fn parse(text: &str) -> Option<(Mods, Key)> {
    let mut mods = Mods::default();
    let mut found = None;
    for raw in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match raw.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            "win" | "windows" => mods.win = true,
            _ => found = Some(key(raw)?),
        }
    }
    found.map(|k| (mods, k))
}

/// Почему клавиша не годится — как у C#.
pub fn not_understood(text: &str) -> String {
    format!("Не понял хоткей «{text}». Пример: Ctrl+Shift+Space")
}
