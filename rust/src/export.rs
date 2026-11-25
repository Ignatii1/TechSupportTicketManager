//! Текст заявки для агента и файла базы знаний (`KnowledgeExport.Format.cs`, `Secrets`, `TicketHistory`): без сети.
//! Правила C# — один в один; регулярные выражения .NET заменены маленькими разборщиками (Юникод `\s`, `\b`, `\p{L}`
//! по правилам .NET), длины и обрезка — в единицах UTF-16 там, где C# смотрит на `.Length`.

use crate::intraservice::{Client, Date, Event, Extra, Found, Task, TaskFile, User};
use crate::rules::{ClosedNames, same_name};
use std::fmt::Write;

pub mod files;
pub mod run;
pub mod table;

/// Чьи вложения выгрузка кладёт в базу (`AttachmentScope`, настройка «Вложения в базу знаний»); Other — число, которое .NET
/// читает, а имени у него нет.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AttachmentScope {
    Mine,
    All,
    #[default]
    None,
    Other(i32),
}

impl std::fmt::Display for AttachmentScope {
    // заглушка карточки 4.4-1: строка из чисел не та
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("?")
    }
}

/// Качать ли вложения заявки при такой настройке и такой моей роли (`Attachments.Downloads`). Заглушка.
pub fn downloads(_scope: AttachmentScope, _my_role: &str) -> bool {
    false
}

/// Html-вложение (`Attachments.IsHtml`). Заглушка.
pub fn is_html(_name: &str) -> bool {
    false
}

/// Версия формата файла (`FormatVersion`).
pub const FORMAT_VERSION: i32 = 8;
/// Чем заменяется пароль (`Secrets.Hidden`).
pub const HIDDEN: &str = "[скрыт]";
/// Страниц истории не больше (`MaxLifetimePages`).
pub const MAX_LIFETIME_PAGES: i32 = 100;
/// Сколько знаков закрывающего комментария в свойствах (`ClosingCommentChars`).
const CLOSING_COMMENT_CHARS: usize = 300;
/// Название файла, в котором выгрузка узнаёт своё (`SourceName`).
const SOURCE_NAME: &str = "intraservice";

/// Что выгрузка знает о человеке и сервере (`ExportContext` без вложений): кто «я» и закрытые названия статусов.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub me: Option<User>,
    pub closed: ClosedNames,
}

/// Факты истории (`TicketHistory.Facts`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub participants: Vec<String>,
    pub my_role: String,
    pub first_reaction_hours: Option<f64>,
    pub resolve_hours: Option<f64>,
    pub reopened: i32,
    pub public_comments: i32,
    pub internal_comments: i32,
    pub closing_comment: String,
}

// ---------- пароли ----------

/// Слова-ключи (`Secrets.Password`), в порядке выражения.
const KEYS: [&str; 11] = ["пароль", "пароля", "паролем", "пароли", "password", "passwd", "pwd", "pass", "пин-код", "пин", "pin"];

/// Буква, как `\p{L}` у .NET. .NET видит знак вне BMP двумя половинками пары — это не буквы; Alphabetic у Rust шире
/// буквы: цифры-буквы (Ⅻ), обведённые (Ⓐ), приклеенные метки — их вычитаем.
// ponytail: метки с Alphabetic вне U+0345 и U+0363–036F (огласовки индийских письменностей и т. п.) остаются буквами — в
// заявках на русском их нет.
fn letter(c: char) -> bool {
    u32::from(c) <= 0xFFFF
        && c.is_alphabetic()
        && !c.is_numeric()
        && !matches!(c, '\u{345}' | '\u{363}'..='\u{36F}' | '\u{24B6}'..='\u{24E9}')
}

/// Знак слова для `\b` у .NET (`[\p{L}\p{Mn}\p{Nd}\p{Pc}]`): цифра, «_», ZWNJ/ZWJ, метки U+0300–036F и буквы латиницы,
/// греческого и кириллицы — там Alphabetic и есть буквы. Остальное — не знак слова: граница есть и пароль скрывается —
/// лишнее «[скрыт]» лучше открытого пароля (у других письменностей Alphabetic включает и огласовки Mc, а у .NET они не
/// знаки слова: «ाpin: 1» C# скрывает).
// ponytail: буквы других письменностей (中, ا) и цифры не ASCII (١, １) у .NET — знаки слова, здесь — нет: «中Пароль: 1»
// здесь скрыт, у C# — нет. Расхождение только в сторону скрытия.
fn is_word(c: char) -> bool {
    let alphabet = matches!(c, '\u{C0}'..='\u{24F}' | '\u{370}'..='\u{3FF}' | '\u{400}'..='\u{52F}' | '\u{1E00}'..='\u{1EFF}');
    c.is_ascii_alphanumeric() || (alphabet && letter(c)) || matches!(c, '_' | '\u{200C}' | '\u{200D}' | '\u{300}'..='\u{36F}')
}

/// Пробел, кроме перевода строки (`[^\S\r\n]`).
fn is_blank(c: char) -> bool {
    c.is_whitespace() && c != '\r' && c != '\n'
}

/// Пароли скрыты: слово «пароль» (и как его пишут), до 20 знаков без знаков препинания, «:» или «=» и следующее слово —
/// на той же строке или в начале следующей. Узко нарочно: «пароль сброшен, логин: ivanov» — обычный текст.
pub fn mask(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        match password_at(&c, i) {
            Some((key_end, end)) => {
                out.extend(&c[i..key_end]);
                out.push_str(HIDDEN);
                i = end;
            }
            None => {
                out.push(c[i]);
                i += 1;
            }
        }
    }
    out
}

/// Пароль, начинающийся в `i`: (где кончается ключ с двоеточием, где кончается пароль).
fn password_at(c: &[char], i: usize) -> Option<(usize, usize)> {
    if i > 0 && is_word(c[i - 1]) {
        return None;
    }
    KEYS.iter().find_map(|key| {
        let n = key.chars().count();
        let same = c.len() >= i + n && key.chars().zip(&c[i..i + n]).all(|(k, &x)| k.to_lowercase().eq(x.to_lowercase()));
        let end = i + n;
        if !same || (end < c.len() && is_word(c[end])) {
            return None;
        }
        value_after(c, end)
    })
}

/// После ключа: до 20 единиц UTF-16 без знаков препинания, «:» или «=», пробелы, возможно перевод строки, слово.
fn value_after(c: &[char], from: usize) -> Option<(usize, usize)> {
    let (mut j, mut units) = (from, 0);
    loop {
        let ch = *c.get(j)?;
        if ch == ':' || ch == '=' {
            break;
        }
        if "\n.,;!?".contains(ch) || units + ch.len_utf16() > 20 {
            return None;
        }
        units += ch.len_utf16();
        j += 1;
    }
    j += 1;
    while c.get(j).is_some_and(|&x| is_blank(x)) {
        j += 1;
    }
    match (c.get(j), c.get(j + 1)) {
        (Some('\r'), Some('\n')) => j += 2,
        (Some('\n'), _) => j += 1,
        _ => {}
    }
    while c.get(j).is_some_and(|&x| is_blank(x)) {
        j += 1;
    }
    let mut end = j;
    while c.get(end).is_some_and(|x| !x.is_whitespace()) {
        end += 1;
    }
    (end > j).then_some((j, end))
}

// ---------- текст как разметка ----------

/// Текст заявки как есть, но чтобы Markdown не принял его за разметку (`Escape`): «#» в начале строки, «#слово», строка «---»,
/// ограда кода (и в цитате, и в пункте списка), html, команды плагинов, «%%» — `inert`.
pub fn escape(text: &str) -> String {
    let lines: Vec<String> = text
        .replace('\r', "")
        .split('\n')
        .map(|line| {
            let line = tags(line);
            let body = line.trim_start();
            let prefix = &line[..line.len() - body.len()];
            let inner = &body[containers(body)..];
            if body.starts_with('#') {
                format!("{prefix}\\{body}")
            } else if inner.starts_with("```") || inner.starts_with("~~~") {
                format!("{}\\{inner}", &line[..line.len() - inner.len()])
            } else if matches!(body.trim(), "---" | "***" | "___") {
                format!("\\{line}")
            } else {
                line
            }
        })
        .collect();
    inert(&lines.join("\n"))
}

/// Длина (в байтах) начала строки из маркеров цитаты и пунктов списка («> », «- », «1. », «12) ») — `Containers`.
fn containers(body: &str) -> usize {
    let c: Vec<(usize, char)> = body.char_indices().collect();
    let at = |k: usize| c.get(k).map(|&(_, ch)| ch);
    let (mut k, mut last) = (0, 0);
    loop {
        let marker = match at(k) {
            Some('>') => 1,
            Some('-' | '*' | '+') if at(k + 1).is_some_and(char::is_whitespace) => 1,
            Some(d) if d.is_ascii_digit() => {
                let digits = (k..).take_while(|&x| at(x).is_some_and(|d| d.is_ascii_digit())).count();
                let after = k + digits;
                if digits <= 9 && matches!(at(after), Some('.' | ')')) && at(after + 1).is_some_and(char::is_whitespace) {
                    digits + 1
                } else {
                    0
                }
            }
            _ => 0,
        };
        if marker == 0 {
            return last;
        }
        k += marker;
        while at(k).is_some_and(char::is_whitespace) {
            k += 1;
        }
        last = c.get(k).map_or(body.len(), |&(b, _)| b);
    }
}

/// Не команда плагинов и не разметка, что прячет текст (`Inert`): «$=», «%%» и «<», с которого начался бы html.
fn inert(s: &str) -> String {
    let s = s.replace("$=", "&#36;=").replace("%%", "%&#37;");
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &ch) in c.iter().enumerate() {
        if ch == '<' && starts_html(&c[i + 1..]) {
            out.push_str("&lt;");
        } else {
            out.push(ch);
        }
    }
    out
}

/// «<» перед `rest` начинает html или команду: перед «/», «!», «?», «%» — всегда, перед латинской буквой — если это не
/// автоссылка CommonMark (`HtmlStart`).
fn starts_html(rest: &[char]) -> bool {
    match rest.first() {
        Some('/' | '!' | '?' | '%') => true,
        Some(c) if c.is_ascii_alphabetic() => !(is_uri_autolink(rest) || is_mail_autolink(rest)),
        _ => false,
    }
}

/// «схема:что-угодно>»: схема — буква и от 1 до 31 знака [A-Za-z0-9+.-].
fn is_uri_autolink(r: &[char]) -> bool {
    let scheme = r[1..].iter().take_while(|c| c.is_ascii_alphanumeric() || "+.-".contains(**c)).count();
    if !(1..=31).contains(&scheme) || r.get(1 + scheme) != Some(&':') {
        return false;
    }
    let body = r[2 + scheme..].iter().take_while(|&&c| c > ' ' && c != '\u{7F}' && c != '<' && c != '>').count();
    r.get(2 + scheme + body) == Some(&'>')
}

/// «почта@домен>»: до «@» — буква и знаки адреса, затем метки домена через точку (до 63 знаков, не с дефиса и не на дефис).
fn is_mail_autolink(r: &[char]) -> bool {
    let local = r[1..].iter().take_while(|c| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~.-".contains(**c)).count();
    if r.get(1 + local) != Some(&'@') {
        return false;
    }
    let mut k = 2 + local;
    loop {
        let label = r[k..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '-').count();
        let ok = label > 0 && label <= 63 && r[k].is_ascii_alphanumeric() && r[k + label - 1].is_ascii_alphanumeric();
        if !ok {
            return false;
        }
        k += label;
        match r.get(k) {
            Some('>') => return true,
            Some('.') => k += 1,
            _ => return false,
        }
    }
}

/// «#слово» — тег в Obsidian: экранируем (`Tags`, `#(?=\p{L})`).
fn tags(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &ch) in c.iter().enumerate() {
        if ch == '#' && c.get(i + 1).is_some_and(|&n| letter(n)) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Поле заявки в строке Markdown: одной строкой, без тегов и команд плагинов (`Md`).
fn md(s: &str) -> String {
    inert(&tags(&one_line(s)))
}

/// Строка YAML в двойных кавычках (`Quote`).
fn quote(s: &str) -> String {
    format!("\"{}\"", inert(&one_line(s)).replace('\\', "\\\\").replace('"', "\\\""))
}

/// Любой пробел (и перевод строки) — одним пробелом, края обрезаны (`OneLine`).
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Без знаков форматирования Юникода (Cf): имя вложения пишет заявитель, и RLO выдал бы программу за pdf (`Visible`).
fn visible(s: &str) -> String {
    const CF: [(u32, u32); 21] = [
        (0xAD, 0xAD),
        (0x600, 0x605),
        (0x61C, 0x61C),
        (0x6DD, 0x6DD),
        (0x70F, 0x70F),
        (0x890, 0x891),
        (0x8E2, 0x8E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    s.chars().filter(|&c| !CF.iter().any(|&(a, b)| (a..=b).contains(&(c as u32)))).collect()
}

// ---------- даты ----------

/// Дата для чтения программой — свойства файла: секунды без долей, пояс даты (`MachineDate.Of`).
pub fn machine_date(d: &Date) -> String {
    d.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// Точная версия заявки — момент изменения в тиках .NET (100 нс от 0001-01-01 по UTC) (`MachineDate.Revision`).
pub fn revision(changed: &Date) -> String {
    const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;
    let ticks = UNIX_EPOCH_TICKS + changed.timestamp() * 10_000_000 + i64::from(changed.timestamp_subsec_nanos()) / 100;
    ticks.to_string()
}

/// Дата для человека — в том поясе, в каком её прислал сервер.
fn show(d: &Option<Date>) -> String {
    d.map_or_else(String::new, |d| d.format("%d.%m.%Y %H:%M").to_string())
}

/// Часы между датами — с одной цифрой после запятой, половина — от нуля (`Math.Round(…, 1, AwayFromZero)`); какой-то даты
/// нет — None. Доли секунды не нужны: границы округления (0,05 ч = 180 с) — целые секунды, доля через них не перенесёт.
fn hours(from: Option<Date>, to: Option<Date>) -> Option<f64> {
    let (a, b) = (from?, to?);
    Some(((b - a).num_seconds() as f64 / 3600.0 * 10.0).round() / 10.0)
}

// ---------- факты истории ----------

/// Записи по времени; при равном — как шло на сервере (оно присылает свежие сверху — разворачиваем).
fn in_time_order(events: &[Event]) -> Vec<&Event> {
    let mut ordered: Vec<&Event> = events.iter().rev().collect();
    ordered.sort_by_key(|e| e.date);
    ordered
}

/// Я в заявке — по номерам, а нет их — по именам (`Myself`).
struct Myself<'a>(&'a User);

impl Myself<'_> {
    fn is_me(&self, name: &str) -> bool {
        !self.0.name.trim().is_empty() && same_name(name, &self.0.name)
    }

    fn named_in(&self, names: &Option<String>) -> bool {
        names.as_deref().unwrap_or("").split(',').map(str::trim).filter(|n| !n.is_empty()).any(|n| self.is_me(n))
    }

    fn executes(&self, f: &Found) -> bool {
        f.executor_ids.as_ref().map_or_else(|| self.named_in(&f.executors), |ids| ids.contains(&self.0.id))
    }

    fn initiated(&self, f: &Found) -> bool {
        f.creator_id.map_or_else(|| self.named_in(&f.creator), |id| id == self.0.id)
    }

    fn observes(&self, f: &Found) -> bool {
        f.observer_ids.as_ref().map_or_else(|| self.named_in(&f.observers), |ids| ids.contains(&self.0.id))
    }

    fn wrote(&self, e: &Event) -> bool {
        e.author_id.map_or_else(|| self.is_me(&e.author), |id| id == self.0.id)
    }
}

/// Моя роль — первая подходящая: исполнитель, инициатор, участник, наблюдатель; кто я — не знаем: «» (`RoleOf`).
pub fn role_of(f: &Found, events: &[Event], context: &Context) -> String {
    let Some(me) = &context.me else { return String::new() };
    let i = Myself(me);
    let role = if i.executes(f) {
        "исполнитель"
    } else if i.initiated(f) {
        "инициатор"
    } else if events.iter().any(|e| i.wrote(e)) {
        "участник"
    } else if i.observes(f) {
        "наблюдатель"
    } else {
        ""
    };
    role.to_string()
}

/// Начало текста — до 300 единиц UTF-16 (суррогатная пара не рвётся), обрезанное — с «…» (`Begin`).
fn begin(text: &str) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > CLOSING_COMMENT_CHARS {
            out.push('…');
            return out;
        }
        out.push(c);
    }
    out
}

fn blank(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(|t| t.trim().is_empty())
}

/// Что выгрузка говорит о заявке по её истории (`TicketHistory.Of`).
pub fn facts(f: &Found, events: &[Event], context: &Context) -> Facts {
    let ordered = in_time_order(events);
    let mut actions: Vec<(String, usize)> = Vec::new();
    let mut previous: Option<&str> = None;
    let mut closed: Option<bool> = None;
    let (mut reopened, mut last_closing) = (0, None);
    for (i, e) in ordered.iter().enumerate() {
        let status_changed = !e.status.is_empty() && previous != Some(e.status.as_str());
        if !blank(&e.comment) || status_changed {
            actions.push((e.author.trim().to_string(), i));
        }
        if e.status.is_empty() {
            continue;
        }
        previous = Some(&e.status);
        let closed_now = e.is_final || context.closed.contains(&e.status);
        if closed == Some(true) && !closed_now {
            reopened += 1;
        }
        if closed_now && closed != Some(true) {
            last_closing = Some(i);
        }
        closed = Some(closed_now);
    }
    // по числу действий, при равном — кто раньше
    let mut people: Vec<(String, usize, usize)> = Vec::new();
    for (author, at) in actions.iter().filter(|a| !a.0.is_empty()) {
        match people.iter_mut().find(|p| &p.0 == author) {
            Some(p) => p.1 += 1,
            None => people.push((author.clone(), 1, *at)),
        }
    }
    people.sort_by_key(|p| (std::cmp::Reverse(p.1), p.2));

    let by_initiator = |e: &Event| match (f.creator_id, e.author_id) {
        (Some(creator), Some(author)) => creator == author,
        _ => f.creator.as_deref().is_some_and(|name| !name.is_empty() && same_name(&e.author, name)),
    };
    // реакция — первая запись не инициатора после самой первой: та — регистрация
    let reaction = ordered.iter().skip(1).find(|e| !by_initiator(e));
    let mut closing = String::new();
    if let (Some(true), Some(last)) = (closed, last_closing) {
        let found = ordered[..=last].iter().rev().find_map(|e| match &e.comment {
            Some(text) if !text.trim().is_empty() && e.is_public != Some(false) && !by_initiator(e) => Some(text),
            _ => None,
        });
        // пароли уже скрыты в тексте, но одна строка из нескольких может сложить новое «пароль: x» — скрываем ещё раз
        if let Some(text) = found {
            closing = begin(&mask(&one_line(text)));
        }
    }
    let x = f.extra.as_ref();
    let count = |public: bool| ordered.iter().filter(|e| !blank(&e.comment) && (e.is_public != Some(false)) == public).count() as i32;
    Facts {
        participants: people.into_iter().map(|p| p.0).collect(),
        my_role: role_of(f, events, context),
        first_reaction_hours: hours(f.created, reaction.and_then(|e| e.date)),
        resolve_hours: hours(f.created, x.and_then(|x| x.resolved.or(x.closed))),
        reopened,
        public_comments: count(true),
        internal_comments: count(false),
        closing_comment: closing,
    }
}

// ---------- строка списка и карточка ----------

/// Строка списка, дополненная карточкой: сервис, тип, категории, дата решения, группа и наблюдатели — из карточки, а чего
/// она не прислала, — из строки (`WithDetails`).
pub fn with_details(f: &Found, task: &Task) -> Found {
    let (card, row) = (task.extra.as_ref(), f.extra.as_ref());
    let pick = |a: Option<&Option<String>>, b: Option<&Option<String>>| a.and_then(Clone::clone).or_else(|| b.and_then(Clone::clone));
    Found {
        executor_group: task.executor_group.clone().or_else(|| f.executor_group.clone()),
        observer_ids: task.observer_ids.clone().or_else(|| f.observer_ids.clone()),
        observers: task.observers.clone().or_else(|| f.observers.clone()),
        extra: Some(Extra {
            service: pick(card.map(|x| &x.service), row.map(|x| &x.service)),
            task_type: pick(card.map(|x| &x.task_type), row.map(|x| &x.task_type)),
            categories: pick(card.map(|x| &x.categories), row.map(|x| &x.categories)),
            resolved: card.and_then(|x| x.resolved).or_else(|| row.and_then(|x| x.resolved)),
            company: pick(card.map(|x| &x.company), row.map(|x| &x.company)),
            assets: pick(card.map(|x| &x.assets), row.map(|x| &x.assets)),
            files: card.and_then(|x| x.files.clone()).or_else(|| row.and_then(|x| x.files.clone())),
            closed: card.and_then(|x| x.closed).or_else(|| row.and_then(|x| x.closed)),
        }),
        ..f.clone()
    }
}

/// Карточка заявки строкой списка (`RowOf`): номеров инициатора и исполнителей у неё нет — роль пойдёт по именам.
pub fn row_of(task: &Task) -> Found {
    Found {
        id: task.id,
        name: task.name.clone(),
        status: task.status.clone(),
        creator: task.creator.clone(),
        created: task.created,
        description: task.description.clone(),
        executors: task.executors.clone(),
        executor_group: task.executor_group.clone(),
        changed: task.changed,
        creator_phone: task.creator_phone.clone(),
        creator_email: task.creator_email.clone(),
        extra: task.extra.clone(),
        observer_ids: task.observer_ids.clone(),
        observers: task.observers.clone(),
        ..Found::default()
    }
}

// ---------- файл заявки ----------

fn prop(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
        let _ = writeln!(out, "{key}: {}", quote(v));
    }
}

fn list_prop(out: &mut String, key: &str, items: &[String]) {
    if !items.is_empty() {
        let _ = writeln!(out, "{key}: [{}]", items.iter().map(|i| quote(i)).collect::<Vec<_>>().join(", "));
    }
}

/// Список через запятую (`ListProp` со строкой): пустые пропущены, края обрезаны.
fn comma_prop(out: &mut String, key: &str, list: Option<&str>) {
    let items: Vec<String> = list.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    list_prop(out, key, &items);
}

fn hours_prop(out: &mut String, key: &str, h: Option<f64>) {
    if let Some(h) = h {
        let _ = writeln!(out, "{key}: {h:.1}");
    }
}

fn date_prop(out: &mut String, key: &str, d: &Option<Date>) {
    if let Some(d) = d {
        let _ = writeln!(out, "{key}: {}", machine_date(d));
    }
}

/// Строка шапки: «Метка: значение · метка: значение» из того, что известно.
fn line(out: &mut String, parts: &[(&str, Option<&str>)]) {
    let known: Vec<String> =
        parts.iter().filter_map(|(label, v)| v.filter(|v| !v.trim().is_empty()).map(|v| format!("{label}: {}", md(v)))).collect();
    if known.is_empty() {
        return;
    }
    let joined = known.join(" · ");
    let mut chars = joined.chars();
    out.extend(chars.next().into_iter().flat_map(char::to_uppercase));
    out.push_str(chars.as_str());
    out.push_str("  \n");
}

/// Имя вложения для человека: как прислал сервер, одной строкой, без невидимых знаков, пароли скрыты; имени нет — «файл N»
/// (`Attachment.Title`).
fn attachment_title(file: &TaskFile) -> String {
    let name = file.name.as_deref().filter(|n| !n.is_empty()).map_or_else(|| format!("файл {}", file.id), String::from);
    mask(&one_line(&visible(&name)))
}

/// Заявка файлом Markdown: свойства, шапка, описание, вложения (только названы), переписка по времени (`Format`).
pub fn format(f: &Found, events: &[Event], url: &str, exported: &Date, truncated: bool, context: &Context) -> String {
    format_with(f, events, url, exported, truncated, context, None)
}

/// Заявка файлом с состояниями вложений (`Format(..., attachments)`); `None` — все только названы, как у `format`.
pub fn format_with(
    f: &Found,
    events: &[Event],
    url: &str,
    exported: &Date,
    truncated: bool,
    context: &Context,
    attachments: Option<&[Attachment]>,
) -> String {
    let named: Vec<Attachment>;
    let attachments: &[Attachment] = match attachments {
        Some(list) => list,
        None => {
            named = f
                .extra
                .as_ref()
                .and_then(|x| x.files.as_deref())
                .unwrap_or(&[])
                .iter()
                .map(|file| Attachment { file: file.clone(), state: AttachmentState::Listed })
                .collect();
            &named
        }
    };
    // пароли — первым делом: файл уходит в GitHub, этот же текст — в запрос агенту в облаке; название — одной строкой до
    // скрытия: склейка строк может сложить новое «пароль: x»
    let f = Found { name: mask(&one_line(&f.name)), description: f.description.as_deref().map(mask), ..f.clone() };
    let events: Vec<Event> = events.iter().map(|e| Event { comment: e.comment.as_deref().map(mask), ..e.clone() }).collect();
    let facts = facts(&f, &events, context);
    let x = f.extra.as_ref();
    let text = |pick: fn(&Extra) -> &Option<String>| x.and_then(|x| pick(x).as_deref());
    let mut sb = String::from("---\n");
    let _ = writeln!(sb, "id: {}", f.id);
    prop(&mut sb, "title", Some(&f.name));
    prop(&mut sb, "status", Some(&f.status));
    date_prop(&mut sb, "created", &f.created);
    date_prop(&mut sb, "changed", &f.changed);
    date_prop(&mut sb, "resolved", &x.and_then(|x| x.resolved));
    prop(&mut sb, "service", text(|x| &x.service));
    prop(&mut sb, "type", text(|x| &x.task_type));
    comma_prop(&mut sb, "categories", text(|x| &x.categories));
    prop(&mut sb, "creator", f.creator.as_deref());
    comma_prop(&mut sb, "executors", f.executors.as_deref());
    prop(&mut sb, "group", f.executor_group.as_deref());
    prop(&mut sb, "company", text(|x| &x.company));
    comma_prop(&mut sb, "assets", text(|x| &x.assets));
    if let Some(files) = x.and_then(|x| x.files.as_ref()) {
        let _ = writeln!(sb, "files: {}", files.len());
    }
    // сколько вложений нет в базе (не скачаны по настройке или не скачались): сменят настройку — выгрузка докачает
    let missing = attachments.iter().filter(|a| matches!(a.state, AttachmentState::Skipped | AttachmentState::Failed { .. })).count();
    if missing > 0 {
        let _ = writeln!(sb, "files_missing: {missing}");
    }
    date_prop(&mut sb, "closed", &x.and_then(|x| x.closed));
    // история урезана — её начала нет: участников, реакцию и счёт по ней не пишем
    if !truncated {
        list_prop(&mut sb, "participants", &facts.participants);
    }
    prop(&mut sb, "my_role", Some(&facts.my_role));
    if !truncated {
        hours_prop(&mut sb, "first_reaction_h", facts.first_reaction_hours);
    }
    hours_prop(&mut sb, "resolve_h", facts.resolve_hours);
    if !truncated {
        let _ = write!(
            sb,
            "reopened: {}\ncomments_public: {}\ncomments_internal: {}\n",
            facts.reopened, facts.public_comments, facts.internal_comments
        );
    }
    prop(&mut sb, "closing_comment", Some(&facts.closing_comment));
    prop(&mut sb, "url", Some(url));
    // точная версия заявки на сервере — по ней выгрузка узнаёт неизменный файл. Вложение не скачалось — версии нет:
    // следующая выгрузка перечитает заявку и попробует снова
    if let Some(changed) = f.changed.as_ref().filter(|_| !attachments.iter().any(|a| matches!(a.state, AttachmentState::Failed { .. }))) {
        let _ = writeln!(sb, "revision: {}", revision(changed));
    }
    let _ = write!(sb, "source: {SOURCE_NAME}\nformat: {FORMAT_VERSION}\n");
    date_prop(&mut sb, "exported", &Some(*exported));
    sb.push_str("---\n\n");

    let _ = write!(sb, "# {} · {}\n\n", f.id, md(&f.name));
    line(
        &mut sb,
        &[
            ("Статус", Some(&f.status)),
            ("создана", Some(&show(&f.created))),
            ("изменена", Some(&show(&f.changed))),
            ("решена", Some(&show(&x.and_then(|x| x.resolved)))),
        ],
    );
    line(&mut sb, &[("Инициатор", f.creator.as_deref()), ("исполнители", f.executors.as_deref()), ("группа", f.executor_group.as_deref())]);
    line(&mut sb, &[("Сервис", text(|x| &x.service)), ("тип", text(|x| &x.task_type)), ("категории", text(|x| &x.categories))]);
    line(&mut sb, &[("Ссылка", Some(url))]);

    sb.push_str("\n## Описание\n\n");
    match f.description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            sb.push_str(&escape(d));
            sb.push('\n');
        }
        None => sb.push_str("_Описания нет._\n"),
    }

    if !attachments.is_empty() {
        sb.push_str("\n## Вложения\n\n");
        for a in attachments {
            let _ = writeln!(sb, "- {}", attachment_line(a));
        }
    }

    sb.push_str("\n## Переписка\n");
    if truncated {
        let _ = write!(sb, "\n_Переписка длиннее: показаны последние {} записей, более ранние не выгружены._\n", events.len());
    }
    let mut written = false;
    let mut previous: Option<&str> = None;
    for e in in_time_order(&events) {
        // статус меняется на этой записи — пишем его; смена полей без комментария — шум, пропускаем
        let status = (!e.status.is_empty() && previous != Some(e.status.as_str())).then_some(e.status.as_str());
        if !e.status.is_empty() {
            previous = Some(&e.status);
        }
        if e.comment.is_none() && status.is_none() {
            continue;
        }
        let when = e.date.map_or_else(|| "без даты".to_string(), |d| show(&Some(d)));
        let author = if e.author.is_empty() { "—".to_string() } else { md(&e.author) };
        let _ = write!(sb, "\n### {when} — {author}");
        if e.is_public == Some(false) {
            sb.push_str(" (внутренний)");
        }
        if let Some(s) = status {
            let _ = write!(sb, " · статус «{}»", md(s));
        }
        sb.push('\n');
        if let Some(c) = &e.comment {
            let _ = write!(sb, "\n{}\n", escape(c.trim()));
        }
        written = true;
    }
    if !written {
        sb.push_str("\n_Переписки нет._\n");
    }
    sb
}

/// Строка вложения: лежит в базе — ссылкой Obsidian с путём от корня базы (картинка — встроенной, `![[…]]`) и размером; нет —
/// имя и почему нет (у ошибки — первая строка, без тела ответа сервера); только известно — имя (`AttachmentLine`).
fn attachment_line(a: &Attachment) -> String {
    let title = || md(&attachment_title(&a.file));
    match &a.state {
        AttachmentState::Saved { path, size } => {
            format!("{}[[{path}]] · {}", if files::is_image(path) { "!" } else { "" }, files::size(*size))
        }
        AttachmentState::Skipped => format!("{} · не скачано", title()),
        AttachmentState::Failed { note } => format!("{} · не скачалось: {}", title(), md(note)),
        AttachmentState::Listed => title(),
    }
}

/// Текст заявки целиком (`BuildAsync`): карточка, вся история, формат. Не прочиталось одно из двух — ошибка: половина
/// заявки хуже, чем никакой. Без повторов.
pub fn build(client: &Client, row: &Found, url: &str, context: &Context, exported: &Date) -> Result<String, String> {
    let task = client.get_task(row.id)?;
    let (events, truncated) = client.read_lifetime(row.id, MAX_LIFETIME_PAGES)?;
    Ok(format(&with_details(row, &task), &events, url, exported, truncated, context))
}

/// Что с вложением после выгрузки (`AttachmentState`): только известно, лежит в базе (путь от корня через «/» и размер),
/// не скачано по настройке, не скачалось (что написать в файле заявки).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentState {
    Listed,
    Saved { path: String, size: u64 },
    Skipped,
    Failed { note: String },
}

/// Вложение в файле заявки (`Attachment`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub file: TaskFile,
    pub state: AttachmentState,
}
