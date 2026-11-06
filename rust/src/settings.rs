//! `settings.json` TicketBoard.exe: адрес, логин, зашифрованный пароль, пороги доски, клавиша, шаблон номера. Только
//! читается — прототип файл не создаёт и не правит (C# создаёт его с настройками по умолчанию, если его нет).

use crate::board::Rules;
use serde::{Deserialize, Deserializer};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub base_url: String,
    pub login: String,
    /// Пароль, зашифрованный DPAPI, base64 (`IntraservicePasswordProtected`); расшифровывает `dpapi::unprotect`.
    pub password_protected: String,
    pub hide_done_days: i64,
    pub wip_limit: i64,
    pub overdue_days: i64,
    /// Названия закрытых статусов (`ClosedStatusNames`) — для импорта, F5 и «Проверить подключение».
    pub closed_status_names: Vec<String>,
    /// Глобальная клавиша быстрого добавления (`Hotkey`).
    pub hotkey: String,
    /// Номер заявки в ссылке (`IntraserviceIdPattern`, первая группа).
    pub id_pattern: String,
}

impl Default for Settings {
    /// Как у `AppSettings`: скрывать «Готово» старше 7 дней, лимит «В работе» 5, просрочка 3 дня.
    fn default() -> Self {
        Settings {
            base_url: String::new(),
            login: String::new(),
            password_protected: String::new(),
            hide_done_days: 7,
            wip_limit: 5,
            overdue_days: 3,
            closed_status_names: ["Выполнена", "Ожидание ответа с автозакрытием", "Закрыта", "Отменена"].map(String::from).to_vec(),
            hotkey: "Ctrl+Alt+Space".into(),
            id_pattern: crate::link::DEFAULT_ID_PATTERN.into(),
        }
    }
}

impl Settings {
    pub fn rules(&self) -> Rules {
        Rules { overdue_days: self.overdue_days, hide_done_days: self.hide_done_days, wip_limit: self.wip_limit }
    }

    /// Закрытые по названию — без пустых и пробелов, без учёта регистра (`AppSettings.ClosedNames`).
    pub fn closed_names(&self) -> crate::rules::ClosedNames {
        crate::rules::ClosedNames::new(&self.closed_status_names)
    }

    /// Ссылка на заявку по номеру — из адреса Интрасервиса (`AppSettings.TicketUrl`); адреса нет — пусто.
    pub fn ticket_url(&self, id: i32) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() { String::new() } else { format!("{base}/Task/View/{id}") }
    }

    /// Есть чем ходить в Интрасервис: адрес http(s), логин и пароль — как `HttpIntraserviceClient.From`.
    pub fn has_api(&self) -> bool {
        let url = self.base_url.trim().to_ascii_lowercase();
        let host = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
        host.is_some_and(|h| !h.trim_matches('/').is_empty()) && !self.login.trim().is_empty() && !self.password_protected.is_empty()
    }
}

fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Поля файла под их именами в `settings.json`; остальные (горячая клавиша, автовыгрузка…) прототипу не нужны. null в
/// списке закрытых — «ни одного», как у C#; нет поля — по умолчанию.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct File {
    #[serde(deserialize_with = "nullable")]
    intraservice_base_url: String,
    #[serde(deserialize_with = "nullable")]
    intraservice_login: String,
    #[serde(deserialize_with = "nullable")]
    intraservice_password_protected: String,
    hide_done_older_than_days: i64,
    wip_limit: i64,
    overdue_days: i64,
    #[serde(deserialize_with = "nullable")]
    closed_status_names: Vec<String>,
    /// null и нет поля — по умолчанию (у C# null дал бы сбой, у прототипа — то, что работает)
    hotkey: Option<String>,
    intraservice_id_pattern: Option<String>,
}

impl Default for File {
    fn default() -> Self {
        let d = Settings::default();
        File {
            intraservice_base_url: d.base_url,
            intraservice_login: d.login,
            intraservice_password_protected: d.password_protected,
            hide_done_older_than_days: d.hide_done_days,
            wip_limit: d.wip_limit,
            overdue_days: d.overdue_days,
            closed_status_names: d.closed_status_names,
            hotkey: None,
            intraservice_id_pattern: None,
        }
    }
}

/// Комментарии (`//`, `/* */`) и висячие запятые — долой, как их пропускает `AppSettings` (`ReadCommentHandling.Skip`,
/// `AllowTrailingCommas`): файл правят руками. Внутри строк — ничего не трогается. Один проход: запятая вне строки
/// запоминается, и если следующим значимым знаком будет «}» или «]», её убирают.
pub fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let (mut in_string, mut comma) = (false, None::<usize>);
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                // до конца строки; сам перенос остаётся
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut star = false;
                for n in chars.by_ref() {
                    if star && n == '/' {
                        break;
                    }
                    star = n == '*';
                }
            }
            _ => {
                if !c.is_whitespace() {
                    if let (Some(at), '}' | ']') = (comma, c) {
                        out.remove(at);
                    }
                    comma = (c == ',').then_some(out.len());
                }
                in_string = c == '"';
                out.push(c);
            }
        }
    }
    out
}

pub fn parse_settings(text: &str) -> Result<Settings, String> {
    let f: File = serde_json::from_str(&strip_jsonc(text)).map_err(|e| e.to_string())?;
    Ok(Settings {
        base_url: f.intraservice_base_url,
        login: f.intraservice_login,
        password_protected: f.intraservice_password_protected,
        hide_done_days: f.hide_done_older_than_days,
        wip_limit: f.wip_limit,
        overdue_days: f.overdue_days,
        closed_status_names: f.closed_status_names,
        hotkey: f.hotkey.unwrap_or_else(|| Settings::default().hotkey),
        id_pattern: f.intraservice_id_pattern.unwrap_or_else(|| Settings::default().id_pattern),
    })
}

/// Настройки из папки; нет файла — по умолчанию; не прочитался — по умолчанию и почему (файл не трогается).
pub fn load_settings(dir: &Path) -> (Settings, String) {
    match std::fs::read_to_string(dir.join("settings.json")) {
        Ok(text) => match parse_settings(&text) {
            Ok(s) => (s, String::new()),
            Err(e) => (Settings::default(), format!("settings.json не прочитан: {e} — пока настройки по умолчанию")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), String::new()),
        Err(e) => (Settings::default(), format!("settings.json не прочитан: {e} — пока настройки по умолчанию")),
    }
}
