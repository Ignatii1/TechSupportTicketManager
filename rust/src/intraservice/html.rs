//! Описание и комментарий — текстом, как `HtmlToText` C#-клиента. Html из редактора или письма (узнаётся по разметке —
//! `is_html`) — его видимый текст (`html_text`), как его показывает браузер; простой текст (так их шлёт живой сервер) — как
//! есть: «<» и «>» в нём — знаки (коды маркировки, «остаток < 5»). Сущности раскрываются в обоих (`decode_entities` —
//! правила `WebUtility.HtmlDecode`).
//!
//! Шаблоны C# здесь — сканеры: «`<` имя конец-тега», где конец — `>`, `/>` или пробел и всё до `>` (`TagEnd`); имена —
//! без учёта регистра ASCII, пробелы — все пробельные Unicode (как `\s` у .NET). Каждый сканер даёт то же самое левое
//! жадное совпадение, что шаблон, и работает за линейное время (у .NET два шаблона для этого держат `(?<!\s)`).

use std::collections::HashMap;

/// Имена, по паре которых узнаётся html (`HtmlTagNames`).
const TAG_NAMES: &[&str] = &[
    "p",
    "br",
    "div",
    "span",
    "b",
    "i",
    "u",
    "s",
    "em",
    "strong",
    "font",
    "a",
    "img",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "caption",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "code",
    "hr",
    "sub",
    "sup",
    "strike",
    "center",
    "small",
    "big",
    "nobr",
    "wbr",
    "html",
    "head",
    "body",
    "style",
    "title",
    "meta",
    "link",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "aside",
    "main",
    "address",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "legend",
    "label",
    "input",
    "button",
];
/// Абзацы, таблица, документ (`HtmlBlocks`): в таком html пробелы и переводы строк исходника — лишь пробел, кроме `<pre>`.
const BLOCK_NAMES: &[&str] =
    &["p", "div", "table", "li", "dl", "h1", "h2", "h3", "h4", "h5", "h6", "blockquote", "section", "article", "html", "body"];
/// Начало и конец блока — новая строка (`HtmlBreaks`).
const BREAK_NAMES: &[&str] = &[
    "p",
    "div",
    "li",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "tr",
    "table",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "header",
    "footer",
    "section",
    "article",
    "form",
    "fieldset",
    "legend",
    "caption",
    "address",
];
/// Не текст: стили, скрипты и заголовок документа (`HtmlNoiseStart`).
const NOISE_NAMES: &[&str] = &["style", "script", "head", "title"];

/// Описание или комментарий текстом; пусто или одни пробелы — None.
pub fn html_to_text(html: &str) -> Option<String> {
    if html.trim().is_empty() {
        return None;
    }
    Some(if is_html(html) { html_text(html) } else { tidy(html) })
}

/// Разбор описания до 0.15.1: «тегом» считалось всё от «<» до «>» — и «< 5 шт, заказ >» тоже. Только чтобы узнать на доске
/// описание, им обрезанное (C#: `MainViewModel.Apply`).
pub fn legacy_text(html: &str) -> Option<String> {
    if html.trim().is_empty() {
        return None;
    }
    let text = decode_entities(&legacy_tags(&legacy_breaks(html))).replace('\r', "");
    Some(many_newlines(&text).trim().to_string())
}

/// Html-страница ошибки (IIS, прокси, вход) для сообщения: её заголовок — причину часто называет только он — и видимый текст;
/// заголовок не повторяется, если с него начинается одна из первых трёх строк (целым словом).
pub fn page_text(html: &str) -> String {
    let bare = without_comments(html); // заголовок в комментарии — не заголовок
    let title = next_open(&bare, 0, &["title"])
        .and_then(|(_, _, start)| block_end(&bare, start, "title").map(|(end, _)| tidy(&collapse_spaces(&bare[start..end]))))
        .unwrap_or_default();
    let text = html_text(html);
    let repeated = text
        .split('\n')
        .take(3)
        .any(|line| line.strip_prefix(title.as_str()).is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric())));
    if title.is_empty() || repeated {
        text
    } else if text.is_empty() {
        title
    } else {
        format!("{title}\n{text}")
    }
}

/// `WebUtility.HtmlDecode`: «&» и до ближайшего «;» (раньше следующего «&») — сущность: имя из `ENTITIES` (с учётом
/// регистра), `&#десятичное;` (пробелы и знак по краям — как `uint.TryParse`), `&#xшестнадцатеричное;`; суррогаты и больше
/// U+10FFFF, незнакомое имя — как есть.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            let next = b[i..].iter().position(|&c| c == b'&').map_or(b.len(), |p| i + p);
            out.push_str(&text[i..next]);
            i = next;
            continue;
        }
        if let Some(p) = b[i + 1..].iter().position(|&c| c == b';' || c == b'&').filter(|&p| b[i + 1 + p] == b';') {
            let entity = &text[i + 1..i + 1 + p];
            let after = i + p + 2;
            if entity.len() > 1 && entity.starts_with('#') {
                if let Some(c) = numeric_entity(&entity[1..]) {
                    out.push(c);
                    i = after;
                    continue;
                }
            } else {
                match ENTITIES.binary_search_by(|(name, _)| name.as_bytes().cmp(entity.as_bytes())) {
                    Ok(k) => out.push(char::from_u32(ENTITIES[k].1).unwrap_or(char::REPLACEMENT_CHARACTER)),
                    Err(_) => {
                        out.push('&');
                        out.push_str(entity);
                        out.push(';');
                    }
                }
                i = after;
                continue;
            }
        }
        out.push('&');
        i += 1;
    }
    out
}

/// `&#…;` без «#»: `x`/`X` и шестнадцатеричные цифры, или десятичное с пробелами и знаком по краям (`NumberStyles.Integer`).
fn numeric_entity(digits: &str) -> Option<char> {
    let value = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
        if hex.is_empty() || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok()?
    } else {
        let t = digits.trim_matches(|c| matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' '));
        let (negative, t) = match t.as_bytes().first() {
            Some(b'-') => (true, &t[1..]),
            Some(b'+') => (false, &t[1..]),
            _ => (false, t),
        };
        if t.is_empty() || !t.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: u32 = t.parse().ok()?;
        if negative && v != 0 {
            return None; // «-0» — ноль, остальное отрицательное в uint не влезает
        }
        v
    };
    char::from_u32(value) // суррогаты и больше U+10FFFF — None, как у .NET
}

/// Html — по элементу известного тега (открывающий раньше такого же закрывающего; тот — в одной строке), по `<br>` или
/// `<img …>`. Одиночный «тег» — бывает и в простом тексте: код маркировки, почта в скобках, упомянутый тег.
fn is_html(text: &str) -> bool {
    let b = text.as_bytes();
    let lts: Vec<usize> = (0..b.len()).filter(|&i| b[i] == b'<').collect();
    if lts.iter().any(|&i| br_tag(text, i).is_some() || img_tag(text, i)) {
        return true;
    }
    let mut last_close: HashMap<String, usize> = HashMap::new();
    for &i in &lts {
        if let Some(name) = tight_close(text, i, TAG_NAMES) {
            last_close.insert(name.to_ascii_lowercase(), i);
        }
    }
    !last_close.is_empty()
        && lts.iter().any(|&i| {
            open_tag(text, i, TAG_NAMES).is_some_and(|(name, _)| last_close.get(&name.to_ascii_lowercase()).is_some_and(|&end| i < end))
        })
}

/// Видимый текст html: комментарии, затем стили, скрипты и заголовок документа — долой; блок — с новой строки, ячейки — через
/// пробел; разметка снята. Есть абзацы или таблица — пробелы и переводы строк исходника — пробел, кроме `<pre>`.
/// ponytail: в `<pre>` сохраняются строки, но не отступы — для трассы стека хватает.
fn html_text(html: &str) -> String {
    let text = blocks(&without_comments(html), NOISE_NAMES, false, None); // сначала комментарии: «<style>» в комментарии — не стиль
    let has_blocks = (0..text.len()).any(|i| text.as_bytes()[i] == b'<' && open_tag(&text, i, BLOCK_NAMES).is_some());
    let text = if has_blocks { blocks(&text, &["pre"], true, Some(collapse_spaces)) } else { text };
    let text = cells(&cell_starts(&text));
    let text = strip_tags(&breaks(&text)).replace('\r', "");
    tidy(&if has_blocks { line_spaces(&space_runs(&text)) } else { line_end_spaces(&text) })
}

/// Сущности раскрыть, `\r` долой, строки из одних пробелов — пустые, три и больше пустых подряд — одна, пробелы по краям — долой.
fn tidy(text: &str) -> String {
    let text = decode_entities(text).replace('\r', "");
    many_newlines(&white_lines(&text)).trim().to_string()
}

// ---------- лексемы ----------

/// Длина пробельного символа (Unicode, как `\s` у .NET) в байтовой позиции i; не пробел или конец — 0.
fn space_at(s: &str, i: usize) -> usize {
    s[i..].chars().next().filter(|c| c.is_whitespace()).map_or(0, char::len_utf8)
}

/// Пробелы с позиции i (сколько угодно) — где они кончаются.
fn skip_spaces(s: &str, mut i: usize) -> usize {
    loop {
        match space_at(s, i) {
            0 => return i,
            w => i += w,
        }
    }
}

/// Конец тега (`TagEnd` = `(?:\s[^<>]*)?/?>`) с позиции j: позиция за «>», или None.
fn tag_end(s: &str, j: usize) -> Option<usize> {
    let b = s.as_bytes();
    match b.get(j) {
        Some(b'>') => Some(j + 1),
        Some(b'/') if b.get(j + 1) == Some(&b'>') => Some(j + 2),
        _ => {
            let w = space_at(s, j);
            if w == 0 {
                return None;
            }
            let k = b[j + w..].iter().position(|&c| c == b'<' || c == b'>')? + j + w;
            (b[k] == b'>').then_some(k + 1)
        }
    }
}

/// Имя за «<» или «</» с позиции i: буквы и цифры ASCII (шаблоны C# дальше требуют конец тега, а он не буква).
fn name_at(s: &str, i: usize) -> &str {
    let n = s.as_bytes()[i.min(s.len())..].iter().take_while(|c| c.is_ascii_alphanumeric()).count();
    &s[i..i + n]
}

fn named(name: &str, names: &[&str]) -> bool {
    !name.is_empty() && names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Открывающий тег из names в позиции i (`<(names){TagEnd}`): имя и позиция за ним.
fn open_tag<'a>(s: &'a str, i: usize, names: &[&str]) -> Option<(&'a str, usize)> {
    if s.as_bytes().get(i) != Some(&b'<') {
        return None;
    }
    let name = name_at(s, i + 1);
    if !named(name, names) {
        return None;
    }
    Some((name, tag_end(s, i + 1 + name.len())?))
}

/// Открывающий или закрывающий тег из names (`</?(names){TagEnd}`) — позиция за ним.
fn open_or_close(s: &str, i: usize, names: &[&str]) -> Option<usize> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'<') {
        return None;
    }
    let at = if b.get(i + 1) == Some(&b'/') { i + 2 } else { i + 1 };
    let name = name_at(s, at);
    if !named(name, names) {
        return None;
    }
    tag_end(s, at + name.len())
}

/// Закрывающий из names без атрибутов, в одной строке (`HtmlCloseTags` = `</(names)[ \t]*>`): имя.
fn tight_close<'a>(s: &'a str, i: usize, names: &[&str]) -> Option<&'a str> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'<') || b.get(i + 1) != Some(&b'/') {
        return None;
    }
    let name = name_at(s, i + 2);
    if !named(name, names) {
        return None;
    }
    let k = i + 2 + name.len() + b[i + 2 + name.len()..].iter().take_while(|&&c| c == b' ' || c == b'\t').count();
    (b.get(k) == Some(&b'>')).then_some(name)
}

/// Закрывающий этого имени (`</name\s*>`) в позиции i: позиция за ним.
fn close_named(s: &str, i: usize, name: &str) -> Option<usize> {
    let b = s.as_bytes();
    let n = i + 2 + name.len();
    if b.get(i) != Some(&b'<') || b.get(i + 1) != Some(&b'/') || b.len() < n || !b[i + 2..n].eq_ignore_ascii_case(name.as_bytes()) {
        return None;
    }
    let k = skip_spaces(s, n);
    (b.get(k) == Some(&b'>')).then_some(k + 1)
}

/// `<br{TagEnd}` в позиции i: позиция за ним.
fn br_tag(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    (b.get(i) == Some(&b'<') && b.len() >= i + 3 && b[i + 1..i + 3].eq_ignore_ascii_case(b"br")).then(|| tag_end(s, i + 3)).flatten()
}

/// `<img\s[^<>]*>` в позиции i.
fn img_tag(s: &str, i: usize) -> bool {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'<') || b.len() < i + 4 || !b[i + 1..i + 4].eq_ignore_ascii_case(b"img") {
        return false;
    }
    let w = space_at(s, i + 4);
    w > 0 && b[i + 4 + w..].iter().find(|&&c| c == b'<' || c == b'>') == Some(&b'>')
}

/// Первый открывающий из names в позиции from или дальше: (где, имя, позиция за ним).
fn next_open<'a>(s: &'a str, from: usize, names: &[&str]) -> Option<(usize, &'a str, usize)> {
    let b = s.as_bytes();
    let mut i = from;
    while let Some(p) = b[i..].iter().position(|&c| c == b'<') {
        let at = i + p;
        if let Some((name, end)) = open_tag(s, at, names) {
            return Some((at, name, end));
        }
        i = at + 1;
    }
    None
}

/// Первый закрывающий этого имени в позиции from или дальше (`BlockEnd`): (где, позиция за ним).
fn block_end(s: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let b = s.as_bytes();
    let mut i = from;
    while let Some(p) = b[i..].windows(2).position(|w| w == b"</") {
        let at = i + p;
        if let Some(end) = close_named(s, at, name) {
            return Some((at, end));
        }
        i = at + 1;
    }
    None
}

// ---------- шаги разбора ----------

/// Текст без html-комментариев (и условных блоков Outlook). Незакрытый — дальше закрывающего нет ни у одного: остаток — текст.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    while let Some(at) = text[done..].find("<!--").map(|p| p + done) {
        let Some(end) = text[at + 4..].find("-->").map(|p| p + at + 4) else { break };
        out.push_str(&text[done..at]);
        done = end + 3;
    }
    out.push_str(&text[done..]);
    out
}

/// Блок — открывающий из names и такой же закрывающий — остаётся как есть (keep) или снимается; текст между блоками — через
/// between (нет его — как есть). Имя, чьего закрывающего дальше нет, больше не ищется.
fn blocks(text: &str, names: &[&str], keep: bool, between: Option<fn(&str) -> String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut unclosed: Vec<String> = Vec::new();
    let (mut done, mut from) = (0, 0);
    let push = |out: &mut String, part: &str| match between {
        Some(f) => out.push_str(&f(part)),
        None => out.push_str(part),
    };
    while let Some((at, name, start)) = next_open(text, from, names) {
        from = start;
        let name = name.to_ascii_lowercase();
        if unclosed.contains(&name) {
            continue;
        }
        let Some((_, end)) = block_end(text, from, &name) else {
            unclosed.push(name);
            continue;
        };
        push(&mut out, &text[done..at]);
        if keep {
            out.push_str(&text[at..end]);
        }
        done = end;
        from = end;
    }
    push(&mut out, &text[done..]);
    out
}

/// Пробелы и переводы строк — один пробел (`HtmlSpace`).
fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// Начало ячейки — вместе с абзацем в ней (у Word в каждой ячейке свой `<p>`) — долой (`HtmlCellStarts` =
/// `<t[dh]{TagEnd}(?:\s|<(?:p|div){TagEnd})*`).
fn cell_starts(text: &str) -> String {
    replace_at(text, "", |i| {
        let mut k = open_tag(text, i, &["td", "th"])?.1;
        loop {
            match space_at(text, k) {
                0 => match open_tag(text, k, &["p", "div"]) {
                    Some((_, end)) => k = end,
                    None => return Some(k),
                },
                w => k += w,
            }
        }
    })
}

/// Конец ячейки — пробел, вместе с концом абзаца в ней и пробелами вокруг (`HtmlCells` =
/// `(?<!\s)(?:\s|<br{TagEnd}|</(?:p|div)\s*>)*</t[dh]\s*>\s*`). Неудача от позиции — неудача до конца её серии: дальше не
/// ищем, пока серия не кончится (.NET перечитывал бы её от каждого места).
fn cells(text: &str) -> String {
    let mut failed_until = 0;
    replace_at(text, " ", |i| {
        if i < failed_until || after_space(text, i) {
            return None;
        }
        let mut k = i;
        loop {
            let w = space_at(text, k);
            if w > 0 {
                k += w;
            } else if let Some(end) = br_tag(text, k).or_else(|| close_named(text, k, "p")).or_else(|| close_named(text, k, "div")) {
                k = end;
            } else {
                break;
            }
        }
        match close_named(text, k, "td").or_else(|| close_named(text, k, "th")) {
            Some(end) => Some(skip_spaces(text, end)),
            None => {
                failed_until = k;
                None
            }
        }
    })
}

/// Новая строка: `<br>`, `<hr>`, начало и конец блока; несколько подряд — одна, и перевод строки исходника сразу за ними — та
/// же строка (`HtmlBreaks`). Блоки — только не сразу за пробелом (`(?<!\s)` у .NET): после `<br>\n` следующий блок новой
/// строки не добавляет.
fn breaks(text: &str) -> String {
    let b = text.as_bytes();
    replace_at(text, "\n", |i| {
        let mut k = match br_tag(text, i) {
            Some(end) => end,
            None if after_space(text, i) => return None,
            None => {
                let mut k = i;
                loop {
                    let at = skip_spaces(text, k);
                    match open_or_close(text, at, BREAK_NAMES).or_else(|| open_tag(text, at, &["hr"]).map(|(_, end)| end)) {
                        Some(end) => k = end,
                        None => break,
                    }
                }
                if k == i {
                    return None;
                }
                k
            }
        };
        // `(?:[ \t]*\r?\n)?`
        let mut j = k + b[k..].iter().take_while(|&&c| c == b' ' || c == b'\t').count();
        if b.get(j) == Some(&b'\r') {
            j += 1;
        }
        if b.get(j) == Some(&b'\n') {
            k = j + 1;
        }
        Some(k)
    })
}

/// Разметка, которую браузер не показывает, — долой (`HtmlTags`): тег (и Word: «o:p»), объявление, условный блок Outlook.
fn strip_tags(text: &str) -> String {
    let b = text.as_bytes();
    replace_at(text, "", |i| {
        if b.get(i) != Some(&b'<') {
            return None;
        }
        let mut j = i + 1;
        if b.get(j) == Some(&b'/') {
            j += 1;
        }
        let word = |j: usize| j + b[j..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'-').count();
        if b.get(j).is_some_and(u8::is_ascii_alphabetic) {
            j = word(j + 1);
            if b.get(j) == Some(&b':') && b.get(j + 1).is_some_and(u8::is_ascii_alphabetic) {
                j = word(j + 2);
            }
            if let Some(end) = tag_end(text, j) {
                return Some(end);
            }
        }
        // `<[!?][^<>]*>`
        if matches!(b.get(i + 1), Some(b'!' | b'?')) {
            let k = b[i + 2..].iter().position(|&c| c == b'<' || c == b'>')? + i + 2;
            return (b[k] == b'>').then_some(k + 1);
        }
        None
    })
}

/// Два и больше пробела или табуляции подряд — один пробел (`SpaceRuns`).
fn space_runs(text: &str) -> String {
    let b = text.as_bytes();
    replace_at(text, " ", |i| {
        let n = b[i..].iter().take_while(|&&c| c == b' ' || c == b'\t').count();
        (n >= 2).then_some(i + n)
    })
}

/// Пробелы вокруг перевода строки — долой (`LineSpaces` = `[ \t]*\n[ \t]*`).
fn line_spaces(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    let trimmed: Vec<&str> = lines
        .iter()
        .enumerate()
        .map(|(n, line)| {
            let line = if n > 0 { line.trim_start_matches([' ', '\t']) } else { line };
            if n < last { line.trim_end_matches([' ', '\t']) } else { line }
        })
        .collect();
    trimmed.join("\n")
}

/// Пробелы в конце строки — долой (`LineEndSpaces` = `[ \t]+\n`).
fn line_end_spaces(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let last = lines.len() - 1;
    lines
        .iter()
        .enumerate()
        .map(|(n, line)| if n < last { line.trim_end_matches([' ', '\t']) } else { line })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Строка из одних пробелов (и неразрывных: пустой абзац Word — «&nbsp;») — пустая (`WhiteLines`): иначе Markdown склеил бы
/// абзацы.
fn white_lines(text: &str) -> String {
    text.split('\n').map(|line| if line.chars().all(char::is_whitespace) { "" } else { line }).collect::<Vec<_>>().join("\n")
}

/// Три и больше переводов строки среди пробелов — два (`ManyNewlines` = `\n\s*\n\s*\n+`): от первого перевода серии пробелов
/// до последнего.
fn many_newlines(text: &str) -> String {
    let b = text.as_bytes();
    let mut skip_to = 0;
    replace_at(text, "\n\n", |i| {
        if i < skip_to || b[i] != b'\n' {
            return None;
        }
        let (mut k, mut count, mut last) = (i, 0, i);
        while k < b.len() {
            let w = space_at(text, k);
            if w == 0 {
                break;
            }
            if b[k] == b'\n' {
                count += 1;
                last = k;
            }
            k += w;
        }
        skip_to = k;
        (count >= 3).then_some(last + 1)
    })
}

/// `<br\s*/?>|</p>|</div>|</li>` — перевод строки (`LegacyBreaks`).
fn legacy_breaks(text: &str) -> String {
    let b = text.as_bytes();
    replace_at(text, "\n", |i| {
        if b.get(i) == Some(&b'<') && b.len() >= i + 3 && b[i + 1..i + 3].eq_ignore_ascii_case(b"br") {
            let mut k = skip_spaces(text, i + 3);
            if b.get(k) == Some(&b'/') {
                k += 1;
            }
            if b.get(k) == Some(&b'>') {
                return Some(k + 1);
            }
        }
        ["</p>", "</div>", "</li>"]
            .iter()
            .find(|tag| b.len() >= i + tag.len() && b[i..i + tag.len()].eq_ignore_ascii_case(tag.as_bytes()))
            .map(|tag| i + tag.len())
    })
}

/// Всё от «<» до «>» — долой (`LegacyTags` = `<[^>]*>`).
fn legacy_tags(text: &str) -> String {
    let b = text.as_bytes();
    let mut no_more = false;
    replace_at(text, "", |i| {
        if no_more || b[i] != b'<' {
            return None;
        }
        match b[i + 1..].iter().position(|&c| c == b'>') {
            Some(p) => Some(i + p + 2),
            None => {
                no_more = true; // «>» дальше нет ни для одного «<»
                None
            }
        }
    })
}

/// Перед позицией i — пробельный символ (`(?<!\s)` не пускает совпадение отсюда).
fn after_space(s: &str, i: usize) -> bool {
    s[..i].chars().next_back().is_some_and(char::is_whitespace)
}

/// Замена совпадений слева направо, как `Regex.Replace`: `at(i)` — конец совпадения, начатого в позиции i (символа), или None.
/// Совпадения не бывают пустыми.
fn replace_at(text: &str, with: &str, mut at: impl FnMut(usize) -> Option<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        match at(i) {
            Some(end) => {
                debug_assert!(end > i);
                out.push_str(with);
                i = end;
            }
            None => {
                let w = text[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&text[i..i + w]);
                i += w;
            }
        }
    }
    out
}

/// Именованные сущности, как их знает `WebUtility.HtmlDecode`: 252 из HTML 4 и `&apos;`; по возрастанию имени (байты) —
/// для двоичного поиска. Значения — числами: среди них невидимые (`&zwnj;`, `&lrm;`…).
pub(super) const ENTITIES: [(&str, u32); 253] = [
    ("AElig", 0xC6),
    ("Aacute", 0xC1),
    ("Acirc", 0xC2),
    ("Agrave", 0xC0),
    ("Alpha", 0x391),
    ("Aring", 0xC5),
    ("Atilde", 0xC3),
    ("Auml", 0xC4),
    ("Beta", 0x392),
    ("Ccedil", 0xC7),
    ("Chi", 0x3A7),
    ("Dagger", 0x2021),
    ("Delta", 0x394),
    ("ETH", 0xD0),
    ("Eacute", 0xC9),
    ("Ecirc", 0xCA),
    ("Egrave", 0xC8),
    ("Epsilon", 0x395),
    ("Eta", 0x397),
    ("Euml", 0xCB),
    ("Gamma", 0x393),
    ("Iacute", 0xCD),
    ("Icirc", 0xCE),
    ("Igrave", 0xCC),
    ("Iota", 0x399),
    ("Iuml", 0xCF),
    ("Kappa", 0x39A),
    ("Lambda", 0x39B),
    ("Mu", 0x39C),
    ("Ntilde", 0xD1),
    ("Nu", 0x39D),
    ("OElig", 0x152),
    ("Oacute", 0xD3),
    ("Ocirc", 0xD4),
    ("Ograve", 0xD2),
    ("Omega", 0x3A9),
    ("Omicron", 0x39F),
    ("Oslash", 0xD8),
    ("Otilde", 0xD5),
    ("Ouml", 0xD6),
    ("Phi", 0x3A6),
    ("Pi", 0x3A0),
    ("Prime", 0x2033),
    ("Psi", 0x3A8),
    ("Rho", 0x3A1),
    ("Scaron", 0x160),
    ("Sigma", 0x3A3),
    ("THORN", 0xDE),
    ("Tau", 0x3A4),
    ("Theta", 0x398),
    ("Uacute", 0xDA),
    ("Ucirc", 0xDB),
    ("Ugrave", 0xD9),
    ("Upsilon", 0x3A5),
    ("Uuml", 0xDC),
    ("Xi", 0x39E),
    ("Yacute", 0xDD),
    ("Yuml", 0x178),
    ("Zeta", 0x396),
    ("aacute", 0xE1),
    ("acirc", 0xE2),
    ("acute", 0xB4),
    ("aelig", 0xE6),
    ("agrave", 0xE0),
    ("alefsym", 0x2135),
    ("alpha", 0x3B1),
    ("amp", 0x26),
    ("and", 0x2227),
    ("ang", 0x2220),
    ("apos", 0x27),
    ("aring", 0xE5),
    ("asymp", 0x2248),
    ("atilde", 0xE3),
    ("auml", 0xE4),
    ("bdquo", 0x201E),
    ("beta", 0x3B2),
    ("brvbar", 0xA6),
    ("bull", 0x2022),
    ("cap", 0x2229),
    ("ccedil", 0xE7),
    ("cedil", 0xB8),
    ("cent", 0xA2),
    ("chi", 0x3C7),
    ("circ", 0x2C6),
    ("clubs", 0x2663),
    ("cong", 0x2245),
    ("copy", 0xA9),
    ("crarr", 0x21B5),
    ("cup", 0x222A),
    ("curren", 0xA4),
    ("dArr", 0x21D3),
    ("dagger", 0x2020),
    ("darr", 0x2193),
    ("deg", 0xB0),
    ("delta", 0x3B4),
    ("diams", 0x2666),
    ("divide", 0xF7),
    ("eacute", 0xE9),
    ("ecirc", 0xEA),
    ("egrave", 0xE8),
    ("empty", 0x2205),
    ("emsp", 0x2003),
    ("ensp", 0x2002),
    ("epsilon", 0x3B5),
    ("equiv", 0x2261),
    ("eta", 0x3B7),
    ("eth", 0xF0),
    ("euml", 0xEB),
    ("euro", 0x20AC),
    ("exist", 0x2203),
    ("fnof", 0x192),
    ("forall", 0x2200),
    ("frac12", 0xBD),
    ("frac14", 0xBC),
    ("frac34", 0xBE),
    ("frasl", 0x2044),
    ("gamma", 0x3B3),
    ("ge", 0x2265),
    ("gt", 0x3E),
    ("hArr", 0x21D4),
    ("harr", 0x2194),
    ("hearts", 0x2665),
    ("hellip", 0x2026),
    ("iacute", 0xED),
    ("icirc", 0xEE),
    ("iexcl", 0xA1),
    ("igrave", 0xEC),
    ("image", 0x2111),
    ("infin", 0x221E),
    ("int", 0x222B),
    ("iota", 0x3B9),
    ("iquest", 0xBF),
    ("isin", 0x2208),
    ("iuml", 0xEF),
    ("kappa", 0x3BA),
    ("lArr", 0x21D0),
    ("lambda", 0x3BB),
    ("lang", 0x2329),
    ("laquo", 0xAB),
    ("larr", 0x2190),
    ("lceil", 0x2308),
    ("ldquo", 0x201C),
    ("le", 0x2264),
    ("lfloor", 0x230A),
    ("lowast", 0x2217),
    ("loz", 0x25CA),
    ("lrm", 0x200E),
    ("lsaquo", 0x2039),
    ("lsquo", 0x2018),
    ("lt", 0x3C),
    ("macr", 0xAF),
    ("mdash", 0x2014),
    ("micro", 0xB5),
    ("middot", 0xB7),
    ("minus", 0x2212),
    ("mu", 0x3BC),
    ("nabla", 0x2207),
    ("nbsp", 0xA0),
    ("ndash", 0x2013),
    ("ne", 0x2260),
    ("ni", 0x220B),
    ("not", 0xAC),
    ("notin", 0x2209),
    ("nsub", 0x2284),
    ("ntilde", 0xF1),
    ("nu", 0x3BD),
    ("oacute", 0xF3),
    ("ocirc", 0xF4),
    ("oelig", 0x153),
    ("ograve", 0xF2),
    ("oline", 0x203E),
    ("omega", 0x3C9),
    ("omicron", 0x3BF),
    ("oplus", 0x2295),
    ("or", 0x2228),
    ("ordf", 0xAA),
    ("ordm", 0xBA),
    ("oslash", 0xF8),
    ("otilde", 0xF5),
    ("otimes", 0x2297),
    ("ouml", 0xF6),
    ("para", 0xB6),
    ("part", 0x2202),
    ("permil", 0x2030),
    ("perp", 0x22A5),
    ("phi", 0x3C6),
    ("pi", 0x3C0),
    ("piv", 0x3D6),
    ("plusmn", 0xB1),
    ("pound", 0xA3),
    ("prime", 0x2032),
    ("prod", 0x220F),
    ("prop", 0x221D),
    ("psi", 0x3C8),
    ("quot", 0x22),
    ("rArr", 0x21D2),
    ("radic", 0x221A),
    ("rang", 0x232A),
    ("raquo", 0xBB),
    ("rarr", 0x2192),
    ("rceil", 0x2309),
    ("rdquo", 0x201D),
    ("real", 0x211C),
    ("reg", 0xAE),
    ("rfloor", 0x230B),
    ("rho", 0x3C1),
    ("rlm", 0x200F),
    ("rsaquo", 0x203A),
    ("rsquo", 0x2019),
    ("sbquo", 0x201A),
    ("scaron", 0x161),
    ("sdot", 0x22C5),
    ("sect", 0xA7),
    ("shy", 0xAD),
    ("sigma", 0x3C3),
    ("sigmaf", 0x3C2),
    ("sim", 0x223C),
    ("spades", 0x2660),
    ("sub", 0x2282),
    ("sube", 0x2286),
    ("sum", 0x2211),
    ("sup", 0x2283),
    ("sup1", 0xB9),
    ("sup2", 0xB2),
    ("sup3", 0xB3),
    ("supe", 0x2287),
    ("szlig", 0xDF),
    ("tau", 0x3C4),
    ("there4", 0x2234),
    ("theta", 0x3B8),
    ("thetasym", 0x3D1),
    ("thinsp", 0x2009),
    ("thorn", 0xFE),
    ("tilde", 0x2DC),
    ("times", 0xD7),
    ("trade", 0x2122),
    ("uArr", 0x21D1),
    ("uacute", 0xFA),
    ("uarr", 0x2191),
    ("ucirc", 0xFB),
    ("ugrave", 0xF9),
    ("uml", 0xA8),
    ("upsih", 0x3D2),
    ("upsilon", 0x3C5),
    ("uuml", 0xFC),
    ("weierp", 0x2118),
    ("xi", 0x3BE),
    ("yacute", 0xFD),
    ("yen", 0xA5),
    ("yuml", 0xFF),
    ("zeta", 0x3B6),
    ("zwj", 0x200D),
    ("zwnj", 0x200C),
];
