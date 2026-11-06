//! `settings.json` TicketBoard.exe: адрес, логин, зашифрованный пароль, пороги доски. Только читается — прототип файл не
//! создаёт и не правит (C# создаёт его с настройками по умолчанию, если его нет).

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
        }
    }
}

impl Settings {
    pub fn rules(&self) -> Rules {
        Rules { overdue_days: self.overdue_days, hide_done_days: self.hide_done_days, wip_limit: self.wip_limit }
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

/// Поля файла под их именами в `settings.json`; остальные (горячая клавиша, автовыгрузка…) прототипу не нужны.
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
        }
    }
}

/// Комментарии (`//`, `/* */`) и висячие запятые — долой, как их пропускает `AppSettings` (`ReadCommentHandling.Skip`,
/// `AllowTrailingCommas`): файл правят руками. Внутри строк — ничего не трогается.
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    // проход 1: комментарии
    let mut plain = String::with_capacity(text.len());
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            plain.push(c);
            if c == '\\' && i + 1 < chars.len() {
                plain.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        } else {
            in_string = c == '"';
            plain.push(c);
        }
        i += 1;
    }
    // проход 2: запятая, за которой (через пробелы) — «}» или «]»
    let chars: Vec<char> = plain.chars().collect();
    let mut out = String::with_capacity(plain.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == ',' && chars[i + 1..].iter().find(|c| !c.is_whitespace()).is_some_and(|&n| n == '}' || n == ']') {
            // висячая — пропускаем
        } else {
            in_string = c == '"';
            out.push(c);
        }
        i += 1;
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
