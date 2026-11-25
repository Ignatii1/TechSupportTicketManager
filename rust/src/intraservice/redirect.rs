//! Куда ведёт переадресация запроса файла: путь, на который пошёл бы HttpClient C# (`Uri.AbsolutePath` после
//! `new Uri(запрос, Location)`). Прототип за переадресацией не идёт — только называет её в ошибке: HttpClient идёт туда
//! без логина (заголовок Authorization за переадресацией не отправляется), так что файла не получил бы и он.

/// Путь, куда ведёт `location` от запроса `request` (полный адрес); None — не переадресация (HttpClient бы не пошёл).
/// Правила — как ответил .NET (`rust/tests/parity/redirect-path.tsv`): края заголовка срезаны, «?…» и «#…» отброшены, «\» —
/// это «/», «.» и «..» убраны, лишнее экранировано. ponytail: байты Location не в UTF-8 экранируются как есть — .NET читает
/// их как Latin-1.
pub fn redirect_path(request: &str, location: &[u8]) -> Option<String> {
    let text = location.trim_ascii();
    if text.is_empty() {
        return None;
    }
    let end = text.iter().position(|&b| b == b'?' || b == b'#').unwrap_or(text.len());
    let reference: Vec<u8> = text[..end].iter().map(|&b| if b == b'\\' { b'/' } else { b }).collect();
    let path = match scheme_end(&reference) {
        // со схемой — только с хостом («https:/x» у .NET — исключение, «foo:bar» — не переадресация)
        Some(at) => after_host(reference[at + 1..].strip_prefix(b"//")?),
        None if reference.starts_with(b"//") => after_host(&reference[2..]),
        None if reference.starts_with(b"/") => reference,
        None => {
            // относительный — от папки запроса; пустой («?q», «#x») — сам запрос
            let own = request_path(request);
            let mut joined = if reference.is_empty() { own.to_vec() } else { own[..=own.iter().rposition(|&b| b == b'/')?].to_vec() };
            joined.extend_from_slice(&reference);
            joined
        }
    };
    Some(without_dots(&escaped(&path)))
}

/// Конец схемы («http:» → 4): буква, дальше буквы, цифры, «+», «-», «.»; иначе это относительный путь («9x:/foo»).
fn scheme_end(reference: &[u8]) -> Option<usize> {
    let at = reference.iter().position(|&b| b == b':')?;
    let (first, rest) = reference[..at].split_first()?;
    (first.is_ascii_alphabetic() && rest.iter().all(|&b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))).then_some(at)
}

/// Путь после «хост[:порт]»: «» — «/».
fn after_host(authority_and_path: &[u8]) -> Vec<u8> {
    authority_and_path.iter().position(|&b| b == b'/').map_or_else(|| b"/".to_vec(), |at| authority_and_path[at..].to_vec())
}

/// Путь запроса: после «схема://хост».
fn request_path(request: &str) -> &[u8] {
    let rest = request.split_once("://").map_or(request, |(_, rest)| rest);
    rest.find('/').map_or(b"/", |at| &rest.as_bytes()[at..])
}

/// Экранирование, как у `Uri`: «%XX» незарезервированного знака — сам знак, другого ASCII — как написан, байты UTF-8 —
/// заглавными (неверная последовательность — как написана); «%» без двух цифр — «%25»; пробел, управляющие, не ASCII и
/// `"<>\`^{|}` — «%XX».
fn escaped(path: &[u8]) -> String {
    let hex = |b: u8| (b as char).to_digit(16);
    let escape_at = |i: usize| match (path.get(i), path.get(i + 1), path.get(i + 2)) {
        (Some(b'%'), Some(&h), Some(&l)) => Some((hex(h)? * 16 + hex(l)?) as u8),
        _ => None,
    };
    let mut out = String::new();
    let mut i = 0;
    while i < path.len() {
        let b = path[i];
        match escape_at(i) {
            Some(v) if v.is_ascii_alphanumeric() || b"-._~".contains(&v) => {
                out.push(v as char);
                i += 3;
            }
            Some(v) if v.is_ascii() => {
                out.push_str(std::str::from_utf8(&path[i..i + 3]).unwrap_or_default());
                i += 3;
            }
            Some(v) => {
                // знак UTF-8 из нескольких «%XX» — все заглавными; не сложился — первый «%XX» как написан
                let len = match v {
                    0xC2..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF4 => 4,
                    _ => 0,
                };
                let bytes: Option<Vec<u8>> = (0..len).map(|k| escape_at(i + 3 * k)).collect();
                match bytes.filter(|bytes| len > 0 && std::str::from_utf8(bytes).is_ok()) {
                    Some(bytes) => {
                        bytes.iter().for_each(|b| out.push_str(&format!("%{b:02X}")));
                        i += 3 * len;
                    }
                    None => {
                        out.push_str(std::str::from_utf8(&path[i..i + 3]).unwrap_or_default());
                        i += 3;
                    }
                }
            }
            None if b <= b' ' || b >= 0x7F || b"%\"<>`^{|}".contains(&b) => {
                out.push_str(&format!("%{b:02X}"));
                i += 1;
            }
            None => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

/// «.» и «..» убраны (RFC 3986, 5.2.4): «..» уносит сегмент перед собой, на конце пути от них остаётся «/».
fn without_dots(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    let mut out: Vec<&str> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        match *segment {
            "." if last => out.push(""),
            "." => {}
            ".." => {
                out.pop();
                if last {
                    out.push("");
                }
            }
            other => out.push(other),
        }
    }
    format!("/{}", out.join("/"))
}
