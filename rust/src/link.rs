//! Ссылка и номер заявки в произвольном тексте (`IntraserviceLinkParser`): ссылка — первая `http(s)://…`; номер — по шаблону
//! из настроек (`IntraserviceIdPattern`, первая группа), иначе последнее число из 4–8 цифр в ссылке, иначе весь текст —
//! номер из 4–8 цифр (любое число в обычном тексте номером не считается).
//!
//! Шаблон — синтаксис `regex-lite` (маленький движок; полный `regex` — +1,4 МБ к exe): без просмотра вперёд/назад и `\p{…}`,
//! `\s`, `\d`, `\w` — только ASCII. Без учёта регистра, как `IgnoreCase` у C#: ASCII — флагом `(?i)`, остальные буквы
//! (кириллица) — строчными и в тексте, и в шаблоне (кроме экранированных). Непонятный шаблон — как кривой в C#: ссылка
//! берётся, номер — последний в ссылке.

/// Шаблон по умолчанию — как у `AppSettings`.
pub const DEFAULT_ID_PATTERN: &str = r"(?:Task/View/|[#№]\s?)(\d{4,8})";

/// Что нашлось: ссылка (пусто — нет) и номер.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub id: Option<i32>,
}

impl Link {
    /// Нашлась ли ссылка: номер без ссылки — обычное дело («#702180», «702180»).
    pub fn has_url(&self) -> bool {
        !self.url.is_empty()
    }
}

#[derive(Clone)]
pub struct LinkParser {
    pattern: Option<regex_lite::Regex>,
}

impl LinkParser {
    pub fn new(pattern: &str) -> LinkParser {
        LinkParser { pattern: regex_lite::Regex::new(&format!("(?i){}", fold_pattern(pattern))).ok() }
    }

    pub fn parse(&self, input: &str) -> Link {
        let url = find_url(input).map(|u| u.trim_end_matches(['.', ',', ')', ';', '>']).to_string()).unwrap_or_default();
        let mut id = self.pattern.as_ref().and_then(|re| re.captures(&fold(input))?.get(1).and_then(|g| integer(g.as_str())));
        if id.is_none() && !url.is_empty() {
            id = last_number(&url);
        }
        // в поле только номер: весь текст целиком, иначе любое число в тексте («заменить картридж 12345») стало бы номером
        let bare = input.trim();
        if id.is_none() && (4..=8).contains(&bare.len()) && bare.bytes().all(|b| b.is_ascii_digit()) {
            id = bare.parse().ok();
        }
        Link { url, id }
    }
}

/// Первая `https?://\S+` (схема — без учёта регистра).
fn find_url(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase(); // смещения те же: меняются только ASCII-буквы
    let mut from = 0;
    while let Some(p) = lower[from..].find("http") {
        let at = from + p;
        let scheme = if lower[at + 4..].starts_with("s://") {
            8
        } else if lower[at + 4..].starts_with("://") {
            7
        } else {
            0
        };
        if scheme > 0 {
            let len: usize = s[at + scheme..].chars().take_while(|c| !c.is_whitespace()).map(char::len_utf8).sum();
            if len > 0 {
                return Some(&s[at..at + scheme + len]);
            }
        }
        from = at + 1;
    }
    None
}

/// Номер из ссылки: `(\d{4,8})(?!.*\d)` — серия цифр, за которой цифр больше нет, от 4 знаков (у длинной — последние 8).
fn last_number(url: &str) -> Option<i32> {
    let b = url.as_bytes();
    let end = b.iter().rposition(u8::is_ascii_digit)? + 1;
    let start = end - b[..end].iter().rev().take_while(|c| c.is_ascii_digit()).count();
    (end - start >= 4).then(|| url[start.max(end - 8)..end].parse().ok()).flatten()
}

/// `int.TryParse` (`NumberStyles.Integer`): пробелы по краям, знак, цифры ASCII, в 32 бита.
fn integer(s: &str) -> Option<i32> {
    let t = s.trim_matches(|c| matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' '));
    let (negative, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: i64 = digits.parse().ok()?;
    i32::try_from(if negative { -v } else { v }).ok()
}

/// Не-ASCII буквы — строчными (простое соответствие, один символ на символ): так кириллица сравнивается без учёта регистра.
fn fold(s: &str) -> String {
    s.chars().map(lower).collect()
}

/// То же в шаблоне, кроме экранированного (`\B` — не `\b`).
fn fold_pattern(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut chars = p.chars();
    while let Some(c) = chars.next() {
        out.push(lower(c));
        if c == '\\'
            && let Some(next) = chars.next()
        {
            out.push(next);
        }
    }
    out
}

fn lower(c: char) -> char {
    if c.is_ascii() {
        return c;
    }
    let mut low = c.to_lowercase();
    match (low.next(), low.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}
