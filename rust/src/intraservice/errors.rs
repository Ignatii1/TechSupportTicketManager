//! Тексты ошибок клиента — как у C#: первая строка — короткая фраза с кодом (`Headline`, `Brief`), дальше — что сервер
//! ответил (`Evidence`: json как есть, страница — её видимым текстом), логин и пароль — звёздочками (`redact`).

use super::html::page_text;

/// Сколько сырого ответа показываем в самом сообщении и сколько кладём в журнал.
pub const SHOWN_CHARS: usize = 1000;
pub const LOGGED_CHARS: usize = 4000;
/// Сколько тела ответа читаем ради ошибки: ответ с кодом ошибки может прийти и с файлом в сотни мегабайт.
pub const ERROR_BODY_BYTES: usize = 64 * 1024;
/// До скольких сервер считает заявки списка (`count` по умолчанию; `count=all` живой сервер не принимает).
pub const COUNT_CEILING: i32 = 1000;

/// Тело ответа для сообщения об ошибке: json и xml — как есть; html-страница (ошибка IIS, страница прокси или входа) — её
/// видимым текстом, с пометкой; длинное — обрезано (`SHOWN_CHARS`); любой `Basic <токен>` — звёздочками.
pub fn evidence(body: &str) -> String {
    let raw = redact(body.trim(), &[]);
    let text = if raw.starts_with('<') && raw.to_ascii_lowercase().contains("<html") {
        format!("(html-страница, показан её текст)\n{}", page_text(&raw))
    } else {
        raw
    };
    head(&text, SHOWN_CHARS)
}

/// Не больше max символов, обрезанное — с «\n…» (`Head`). ponytail: символы, а не UTF-16, как у C#: разница — только у эмодзи.
pub fn head(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}\n…", &text[..i]),
        None => text.to_string(),
    }
}

/// Секреты (пароль и base64 «логин:пароль», от 4 знаков) и любой `Basic <токен>` (от 6 знаков base64 после пробелов) —
/// звёздочками: страница прокси или IIS повторяет заголовки запроса, а текст ошибки просят переслать. Строже `Redact` C#:
/// пароль прячется и сам, а «basic» — и внутри слова (лишняя звёздочка безопаснее пропущенного токена).
pub fn redact(text: &str, secrets: &[&str]) -> String {
    let mut s = text.to_string();
    for secret in secrets.iter().filter(|x| x.chars().count() >= 4) {
        s = s.replace(secret, "***");
    }
    // смещения у строки в нижнем регистре те же: меняются только ASCII-буквы
    let lower = s.to_ascii_lowercase();
    let (mut out, mut i) = (String::with_capacity(s.len()), 0);
    while let Some(pos) = lower[i..].find("basic") {
        let after = i + pos + "basic".len();
        let token_at = after + (s[after..].len() - s[after..].trim_start_matches(char::is_whitespace).len());
        let token = s[token_at..].bytes().take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')).count();
        if token_at > after && token >= 6 {
            out.push_str(&s[i..token_at]);
            out.push_str("***");
            i = token_at + token;
        } else {
            out.push_str(&s[i..after]);
            i = after;
        }
    }
    out.push_str(&s[i..]);
    out
}

/// Первая строка ошибки — короткая фраза с кодом, без ответа сервера и без «:» на конце.
pub fn headline(error: &str) -> String {
    error.split('\n').next().unwrap_or("").trim().trim_end_matches(':').to_string()
}

/// Коротко, в одну строку: фраза и первая строка подробностей («сервер недоступен — No such host is known»); пустые строки —
/// не в счёт.
pub fn brief(error: &str) -> String {
    let lines: Vec<&str> = error.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect();
    match lines.as_slice() {
        [first, second, ..] => format!("{} — {second}", first.trim_end_matches(':')),
        [only] => only.to_string(),
        [] => String::new(),
    }
}

/// Код ответа сервера из первой строки ошибки («ошибка сервера (HTTP 500):…»; в теле ниже может быть что угодно). None —
/// ошибка не из ответа: сеть, срок, разбор.
pub fn http_code(error: &str) -> Option<u16> {
    let first = error.split('\n').next().unwrap_or("");
    let b = first.as_bytes();
    let mut from = 0;
    while let Some(p) = first[from..].find("(HTTP ") {
        let at = from + p + "(HTTP ".len();
        if b.len() >= at + 4 && b[at..at + 3].iter().all(u8::is_ascii_digit) && b[at + 3] == b')' {
            return first[at..at + 3].parse().ok();
        }
        from += p + 1;
    }
    None
}

/// Сбой, который может пройти при повторе: сервер не ответил, не достался, оборвал соединение, ответил 5xx, 408 или 429.
/// Логин, доступ, «нет такой», другой отказ 4xx и непонятный ответ — нет: повтор получил бы то же.
pub fn is_transient(error: &str) -> bool {
    error.starts_with("сервер не ответил")
        || error.starts_with("сервер недоступен")
        || error.starts_with("соединение оборвалось")
        || http_code(error).is_some_and(|c| c >= 500 || c == 408 || c == 429)
}

/// Название заявки, пока настоящего нет: так его ставит доска и так разбор называет заявку без темы.
pub fn auto_title(id: i32) -> String {
    format!("Заявка #{id}")
}

/// Общее число — лишь нижняя граница: сервер досчитал до потолка и дальше не считал.
pub fn capped(total: i32) -> bool {
    total == COUNT_CEILING
}
