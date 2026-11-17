//! Текст заявки для агента и файла базы знаний (`KnowledgeExport.Format.cs`, `Secrets`, `TicketHistory`): без сети.
// ПУСТЫШКИ карточки 4.1-1 и 4.1-2: сигнатуры из карточки, тела заведомо неверные — их заменит исполнитель.

use crate::intraservice::{Client, Date, Event, Found, Task, User};
use crate::rules::ClosedNames;

/// Версия формата файла (`FormatVersion`).
pub const FORMAT_VERSION: i32 = 8;
/// Чем заменяется пароль (`Secrets.Hidden`).
pub const HIDDEN: &str = "[скрыт]";
/// Страниц истории не больше (`MaxLifetimePages`).
pub const MAX_LIFETIME_PAGES: i32 = 100;

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

pub fn mask(text: &str) -> String {
    text.to_string()
}

pub fn escape(text: &str) -> String {
    text.to_string()
}

pub fn one_line(s: &str) -> String {
    s.to_string()
}

pub fn machine_date(_d: &Date) -> String {
    String::new()
}

pub fn revision(_changed: &Date) -> String {
    String::new()
}

pub fn facts(_f: &Found, _events: &[Event], _context: &Context) -> Facts {
    Facts::default()
}

pub fn role_of(_f: &Found, _events: &[Event], _context: &Context) -> String {
    String::new()
}

pub fn with_details(f: &Found, _task: &Task) -> Found {
    f.clone()
}

pub fn row_of(_task: &Task) -> Found {
    Found::default()
}

pub fn format(_f: &Found, _events: &[Event], _url: &str, _exported: &Date, _truncated: bool, _context: &Context) -> String {
    String::new()
}

pub fn build(_client: &Client, _row: &Found, _url: &str, _context: &Context, _exported: &Date) -> Result<String, String> {
    Ok(String::new())
}
