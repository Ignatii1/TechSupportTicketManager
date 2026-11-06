//! Чтение одной заявки из Интрасервиса: `GET api/task/{номер}?include=status`, Basic, ответ в JSON — как
//! `HttpIntraserviceClient.GetTaskAsync`. Ошибки — текстом и теми же фразами, что у C#-клиента: первая строка по-русски и
//! коротко, дальше — что сервер ответил.

use base64::Engine;
use serde_json::Value;
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub id: i64,
    pub name: String,
    pub status: String,
    /// `Changed` строкой, как прислал сервер.
    pub changed: Option<String>,
    pub creator: Option<String>,
    pub executors: Option<String>,
}

fn text(v: &Value, name: &str) -> Option<String> {
    v.get(name)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Карточка из ответа `{"Task": {...}, "Statuses": [...]}` (или голая); не та форма — None. Статус — свой `StatusName`,
/// иначе по `StatusId` из блока `Statuses`, иначе «статус N» (как `.Parse.cs`).
pub fn parse_task(json: &str, id: i64) -> Option<Task> {
    let root: Value = serde_json::from_str(json).ok()?;
    let task = match root.get("Task") {
        Some(t) if t.is_object() => t,
        _ if root.is_object() => &root,
        _ => return None,
    };
    let id = task.get("Id").and_then(Value::as_i64).unwrap_or(id);
    let status_id = task.get("StatusId").and_then(Value::as_i64);
    let from_block = || {
        root.get("Statuses")?.as_array()?.iter().find(|s| s.get("Id").and_then(Value::as_i64) == status_id).and_then(|s| text(s, "Name"))
    };
    let status = text(task, "StatusName").or_else(from_block).or_else(|| status_id.map(|n| format!("статус {n}"))).unwrap_or_default();
    Some(Task {
        id,
        name: text(task, "Name").unwrap_or_else(|| format!("Заявка #{id}")),
        status,
        changed: text(task, "Changed"),
        creator: text(task, "Creator"),
        executors: text(task, "Executors"),
    })
}

pub struct Client {
    base: String,
    auth: String,
    /// Что не должно попасть в текст ошибки: пароль и base64 «логин:пароль».
    secrets: [String; 2],
    timeout: Duration,
    agent: ureq::Agent,
}

/// Логин и пароль, если сервер их повторил (страница прокси или IIS с заголовками запроса), — звёздочками: сами секреты
/// и любой `Basic <токен>` (как `Redact` C#-клиента). Текст ошибки просят переслать — в нём их быть не должно.
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
        let token_at = after + (s[after..].len() - s[after..].trim_start_matches([' ', '\t']).len());
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

/// Сколько символов ответа показать в ошибке.
const EVIDENCE_CHARS: usize = 300;

fn head(s: &str, n: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

impl Client {
    /// Адрес — как в настройках (пробелы и «/» на конце — неважно), пароль — уже расшифрованный.
    pub fn new(base_url: &str, login: &str, password: &str, timeout: Duration) -> Client {
        let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false) // тело ответа с ошибкой — тоже показываем
            .max_redirects(0) // переадресация — на страницу входа: не идём (и не несём туда пароль)
            .proxy(None) // напрямую; прокси Windows — только в диагностике (system_proxy)
            .tls_config(tls)
            .build()
            .new_agent();
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{}:{password}", login.trim()));
        Client {
            base: base_url.trim().trim_end_matches('/').to_string(),
            auth: format!("Basic {token}"),
            secrets: [password.to_string(), token],
            timeout,
            agent,
        }
    }

    fn redact(&self, text: &str) -> String {
        redact(text, &[&self.secrets[0], &self.secrets[1]])
    }

    pub fn get_task(&self, id: i64) -> Result<Task, String> {
        let url = format!("{}/api/task/{id}?include=status", self.base);
        let mut resp = match self.agent.get(&url).header("Authorization", &self.auth).header("Accept", "application/json").call() {
            Ok(r) => r,
            Err(ureq::Error::Timeout(_)) => return Err(format!("сервер не ответил за {} с", self.timeout.as_secs())),
            Err(e @ (ureq::Error::BadUri(_) | ureq::Error::Http(_))) => {
                return Err(format!("адрес Интрасервиса в настройках не годится: {}", self.redact(&e.to_string())));
            }
            Err(e) => return Err(format!("сервер недоступен:\n{}", self.redact(&e.to_string()))),
        };
        let code = resp.status().as_u16();
        let location = resp.headers().get("location").and_then(|v| v.to_str().ok()).map(String::from);
        let body = match resp.body_mut().with_config().limit(16 * 1024 * 1024).read_to_string() {
            Ok(b) => b,
            Err(ureq::Error::Timeout(_)) => return Err(format!("сервер не ответил за {} с", self.timeout.as_secs())),
            Err(e) => return Err(format!("соединение оборвалось:\n{}", self.redact(&e.to_string()))),
        };
        if (300..400).contains(&code) {
            return Err(format!(
                "переадресация (HTTP {code}) на {} — похоже на страницу входа, а не на API",
                self.redact(&location.unwrap_or_default())
            ));
        }
        if !(200..300).contains(&code) {
            let what = match code {
                401 => "неверный логин или пароль",
                403 => "нет доступа",
                404 => "заявка не найдена",
                _ => "ошибка сервера",
            };
            return Err(if body.trim().is_empty() {
                format!("{what} (HTTP {code})")
            } else {
                format!("{what} (HTTP {code}):\n{}", head(&self.redact(&body), EVIDENCE_CHARS))
            });
        }
        parse_task(&body, id).ok_or_else(|| format!("ответ не разобран:\n{}", head(&self.redact(&body), EVIDENCE_CHARS)))
    }
}

/// Настройки прокси Windows (реестр текущего пользователя, как их видит .NET): включён ли, сервер, исключения, PAC.
/// Прототип ходит напрямую — по этой строке видно, нужен ли переносу свой прокси.
#[cfg(windows)]
pub fn system_proxy() -> String {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::HSTRING;
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let dword = |name: &str| -> Option<u32> {
        let mut v = 0u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: выход — u32 и его размер
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(KEY),
                &HSTRING::from(name),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut v as *mut u32).cast()),
                Some(&mut size),
            )
        };
        r.is_ok().then_some(v)
    };
    let string = |name: &str| -> Option<String> {
        let mut buf = vec![0u16; 2048];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: буфер и его размер в байтах
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(KEY),
                &HSTRING::from(name),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if r.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..len])).filter(|s| !s.trim().is_empty())
    };
    let mut parts = Vec::new();
    match (dword("ProxyEnable").unwrap_or(0), string("ProxyServer")) {
        (0, _) => parts.push("прокси Windows выключен".to_string()),
        (_, server) => parts.push(format!("прокси Windows включён: {}", server.unwrap_or_else(|| "сервер не указан".into()))),
    }
    if let Some(bypass) = string("ProxyOverride") {
        parts.push(format!("без прокси: {bypass}"));
    }
    if let Some(pac) = string("AutoConfigURL") {
        parts.push(format!("сценарий PAC: {pac}"));
    }
    parts.join("; ")
}

#[cfg(not(windows))]
pub fn system_proxy() -> String {
    "прокси Windows: здесь не Windows".into()
}
