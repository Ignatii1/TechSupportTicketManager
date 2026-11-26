//! Клиент Интрасервиса — перенос `HttpIntraserviceClient` (слой 1): чтение карточки, истории, пользователя, статусов,
//! списка по отбору; разбор ответов — `parse`, html → текст — `html`, тексты ошибок — `errors`, кодировка тела —
//! `charset`, адрес списка — `query`. Ошибки — строкой и теми же фразами, что у C#: первая строка по-русски и коротко,
//! дальше — что сервер ответил. Пишет клиент только в журнал (`Log`), и то если его дали.

pub mod charset;
pub mod errors;
pub mod html;
pub mod parse;
pub mod query;
mod redirect;

pub use errors::{auto_title, brief, capped, evidence, headline, http_code, is_transient, redact};
pub use redirect::redirect_path;

use crate::export;
use crate::rules::{self, ClosedNames};
use base64::Engine;
use chrono::{DateTime, FixedOffset};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

/// Дата, как её понимает разбор: момент и смещение (дата без пояса — по смещению этого компьютера, как `AssumeLocal`).
pub type Date = DateTime<FixedOffset>;

/// Заявка из `api/task/{id}` (`IntraserviceTask`). «Не прислал» — None; `executors`, `executor_group`, контакты — `Some("")`,
/// когда поле есть, но пусто (синхронизация отличит «никого» от «не знаю»). `changed_text` — `Changed` строкой, как прислал
/// сервер (уходит обратно с записью); `cut_description` — что сделал бы из описания разбор до 0.15.1.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    pub id: i32,
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub creator: Option<String>,
    pub executors: Option<String>,
    pub executor_group: Option<String>,
    pub changed: Option<Date>,
    pub creator_phone: Option<String>,
    pub creator_email: Option<String>,
    pub extra: Option<Extra>,
    pub cut_description: Option<String>,
    pub created: Option<Date>,
    pub creator_position: Option<String>,
    pub observer_ids: Option<Vec<i32>>,
    pub observers: Option<String>,
    pub changed_text: Option<String>,
    pub status_id: Option<i32>,
}

/// Что ещё известно о заявке (`IntraserviceExtra`): сервис, тип, категории и оборудование через запятую, когда решена и
/// закрыта, организация инициатора, вложения. None — поле не прислано.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    pub service: Option<String>,
    pub task_type: Option<String>,
    pub categories: Option<String>,
    pub resolved: Option<Date>,
    pub company: Option<String>,
    pub assets: Option<String>,
    pub files: Option<Vec<TaskFile>>,
    pub closed: Option<Date>,
}

/// Вложение: номер файла (`api/taskfile/{id}`) и имя; None — имён не столько, сколько номеров.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskFile {
    pub id: i32,
    pub name: Option<String>,
}

/// Запись истории (`IntraserviceEvent`): `comment` — None у простой смены статуса; `is_final` — у статуса записи признак
/// «выполнена» или «конечный» из блока Statuses ответа.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub date: Option<Date>,
    pub author: String,
    pub status: String,
    pub comment: Option<String>,
    pub is_public: Option<bool>,
    pub author_id: Option<i32>,
    pub is_final: bool,
}

/// Страница истории: записи (свежие сверху), есть ли ещё, прислал ли сервер номера страниц (`paged`: без них `has_more`
/// ничего не знает).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifetimePage {
    pub events: Vec<Event>,
    pub has_more: bool,
    pub paged: bool,
}

/// Строка списка заявок (`IntraserviceFound`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub id: i32,
    pub name: String,
    pub status: String,
    pub creator: Option<String>,
    pub created: Option<Date>,
    pub description: Option<String>,
    pub executors: Option<String>,
    pub executor_group: Option<String>,
    pub changed: Option<Date>,
    pub creator_phone: Option<String>,
    pub creator_email: Option<String>,
    pub extra: Option<Extra>,
    pub creator_id: Option<i32>,
    pub executor_ids: Option<Vec<i32>>,
    pub cut_description: Option<String>,
    pub observer_ids: Option<Vec<i32>>,
    pub observers: Option<String>,
}

/// Страница списка: строки, общее число (`Paginator.Count`, нет его — сколько пришло; 1000 — «столько или больше»,
/// `capped`) и есть ли следующая, если сервер сказал (`HasNextPage` при `count=false`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPage {
    pub found: Vec<Found>,
    pub total: i32,
    pub has_next: Option<bool>,
}

/// Текущий пользователь API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct User {
    pub id: i32,
    pub name: String,
}

/// Строка справочника (`IntraserviceRef`): сервис, тип, фильтр, сотрудник.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ref {
    pub id: i32,
    pub name: String,
    pub path: Option<String>,
    pub is_archive: bool,
    pub is_default: bool,
}

/// Страница справочника: строки, общее число, есть ли следующая.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Refs {
    pub items: Vec<Ref>,
    pub total: i32,
    pub has_more: bool,
}

/// Сервисы из блока Services списка заявок и сколько заявок на странице.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskServices {
    pub services: Vec<Ref>,
    pub rows: usize,
}

/// Сервисы для условия «сервис» окна поиска: список, ошибка (пусто — нет) и пояснение, если список не весь справочник.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Services {
    pub items: Vec<Ref>,
    pub error: String,
    pub note: String,
}

/// Статус заявки (`IntraserviceStatus`): «Заявка выполнена» (`is_fixed`), «Конечный» (`is_final`), нужен ли комментарий.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub id: i32,
    pub name: String,
    pub is_fixed: bool,
    pub is_final: bool,
    pub is_comment_required: bool,
}

impl Status {
    /// Закрытый: по признаку или назван в `ClosedStatusNames` — одно правило у импорта, F5, поиска и автовыгрузки.
    pub fn closed_by(&self, closed: &ClosedNames) -> bool {
        self.is_fixed || self.is_final || closed.contains(&self.name)
    }
}

/// Мои открытые заявки (`FetchMyOpenAsync` импорта): строки, общее число, целый ли список, ошибка, названия закрытых
/// (из настроек и по признакам справочника) и кто я.
#[derive(Clone, Debug, Default)]
pub struct MyOpen {
    pub rows: Vec<Found>,
    pub total: i32,
    pub complete: bool,
    pub error: String,
    pub closed: ClosedNames,
    pub me: Option<User>,
}

/// Журнал ответов, которые не разобрались, и тел ошибок (`LogUnparsed` + `LogOnce`): один образец на ключ (метод и код)
/// за запуск — F5 по двумстам карточкам не даёт двухсот одинаковых строк.
pub struct Log {
    sink: Box<dyn Fn(&str) + Send + Sync>,
    seen: Mutex<HashSet<String>>,
}

impl Log {
    pub fn new(sink: impl Fn(&str) + Send + Sync + 'static) -> Arc<Log> {
        Arc::new(Log { sink: Box::new(sink), seen: Mutex::new(HashSet::new()) })
    }

    /// Строка в журнал, если по этому ключу её ещё не было; текст собирается только тогда.
    pub fn once(&self, key: &str, text: impl FnOnce() -> String) {
        let first = self.seen.lock().map(|mut seen| seen.insert(key.to_string())).unwrap_or(false);
        if first {
            (self.sink)(&text());
        }
    }
}

/// Записей истории на страницу — столько панель берёт с 0.4 (проверено на живом сервере).
pub const LIFETIME_PAGE_SIZE: i32 = 50;
/// Сколько заявок на страницу просит список «мои по исполнителю»; сервер вправе отдать меньше.
pub const EXECUTOR_PAGE_SIZE: i32 = 200;
/// Потолок страниц «моих открытых» (ponytail C#: 10 по 200 — 2000 заявок).
pub const IMPORT_PAGES: i32 = 10;
/// Тело успешного ответа — не больше (у C# предела нет; здесь — чтобы сломанный сервер не съел память).
const BODY_LIMIT: u64 = 64 * 1024 * 1024;

/// Записей справочника на страницу: API отдаёт не больше 2000 — берём с запасом (`RefPageSize`).
const REF_PAGE_SIZE: i32 = 1000;
/// Столько последних заявок просматривает `get_task_services` и столько ждёт ответ (`TaskServiceRows`).
const TASK_SERVICE_ROWS: i32 = 1000;
const TASK_SERVICES_TIMEOUT: Duration = Duration::from_secs(30);

/// Сколько ждём заголовки ответа на файл и сколько — каждый следующий его кусок (`FileHeadersTimeout`, `FileIdleTimeout`).
pub const FILE_HEADERS_TIMEOUT: Duration = Duration::from_secs(60);
pub const FILE_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Что скачали (`DownloadFileAsync`): сколько байт пришло, длина из Content-Length (нет его — None), ошибка текстом.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Downloaded {
    pub bytes: u64,
    pub length: Option<u64>,
    pub error: String,
}

pub struct Client {
    base: String,
    auth: String,
    /// Что не должно попасть в текст ошибки и журнал: пароль и base64 «логин:пароль».
    secrets: [String; 2],
    timeout: Duration,
    /// Срок заголовков и срок каждого куска файла (`FileHeadersTimeout`, `FileIdleTimeout`).
    file_timeouts: (Duration, Duration),
    agent: ureq::Agent,
    log: Option<Arc<Log>>,
}

impl Client {
    /// Адрес — как в настройках (пробелы и «/» на конце — неважно), пароль — уже расшифрованный. Соединение — напрямую,
    /// корни сертификатов — Windows (SChannel), переадресация не выполняется: она ведёт на страницу входа, и пароль туда не
    /// идёт.
    pub fn new(base_url: &str, login: &str, password: &str, timeout: Duration) -> Client {
        let tls = TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false) // тело ответа с ошибкой — тоже показываем
            .max_redirects(0)
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
            file_timeouts: (FILE_HEADERS_TIMEOUT, FILE_IDLE_TIMEOUT),
            agent,
            log: None,
        }
    }

    /// Только проверки сокращают сроки на файл.
    pub fn with_file_timeouts(mut self, headers: Duration, idle: Duration) -> Client {
        self.file_timeouts = (headers, idle);
        self
    }

    /// `GET api/taskfile/{file_id}` потоком в `to` (`DownloadFileAsync`, без Range): `progress` — сколько байт пришло к этому
    /// мигу; чтение кончается на `stop_at` байтах; `html_expected` — html у вложения-html есть файл, а не страница входа.
    /// Беда сети или сервера — `Ok` с `error` теми же фразами, что у C#; ошибка записи в `to` — `Err`, как и остановка
    /// (`Interrupted`, слушается и пока ждём кусок). Общего срока нет: файл в гигабайт по VPN качается долго, а замолчавший
    /// сервер — сбой; запись куска на диск между чтениями в срок не входит.
    /// ponytail: переадресацию не выполняем, а сообщаем её путь — за страницей входа C# идёт сам и пишет путь, куда пришёл.
    pub fn download_file(
        &self,
        file_id: i32,
        to: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64),
        stop: &dyn Fn() -> bool,
        stop_at: u64,
        html_expected: bool,
    ) -> std::io::Result<Downloaded> {
        const CALL: &str = "download_file";
        let (headers, idle) = self.file_timeouts;
        let path = format!("api/taskfile/{file_id}");
        let stopped = || std::io::Error::from(std::io::ErrorKind::Interrupted);
        if stop() {
            return Err(stopped());
        }
        let url = format!("{}/{path}", self.base);
        // сроки ureq — лишь потолки для потока ниже, позже нашего срока (срок отмеряет он); общего срока у файла нет
        let (request, ceiling) = (self.agent.get(&url).config().timeout_global(None), Some(headers + Duration::from_secs(2)));
        let request = request.timeout_resolve(ceiling).timeout_connect(ceiling).timeout_recv_response(ceiling);
        // per_read у ureq — каждое чтение сокета, и заголовков тоже: срок куска считает читающий поток ниже, а это — потолок
        let request = request.timeout_per_read(Some(headers.max(idle))).build().header("Authorization", &self.auth);
        // заголовков ждёт свой поток, а здесь — «Остановить» (у C# `ct` связан с `SendAsync`) и один срок до заголовков на всё:
        // имя, соединение, ответ (у C# — один `CancelAfter`)
        let (tx, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(request.call());
        });
        let deadline = std::time::Instant::now() + headers;
        let resp = loop {
            if stop() {
                return Err(stopped());
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match answer.recv_timeout(left.min(Duration::from_millis(100))) {
                Ok(Ok(r)) => break r,
                Ok(Err(e)) => return Ok(Downloaded { error: self.failure(e, headers), ..Downloaded::default() }),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !left.is_zero() => {}
                Err(_) => return Ok(Downloaded { error: no_answer(headers), ..Downloaded::default() }),
            }
        };
        let fail = |error: String| Ok(Downloaded { error, ..Downloaded::default() });
        let code = resp.status().as_u16();
        let header = |name: &str| resp.headers().get(name).map(|v| v.as_bytes().to_vec());
        let content_type = header("content-type").map(|v| String::from_utf8_lossy(&v).into_owned());
        let charset = charset_of(content_type.as_deref());
        // HttpClient идёт за 300, 301, 302, 303, 307, 308 — только с Location; без него это просто код ответа
        if let (true, Some(at)) = (matches!(code, 300..=303 | 307 | 308), header("location").and_then(|l| redirect_path(&url, &l))) {
            let at = self.mask(&at);
            return fail(format!(
                "сервер переадресовал запрос файла на {at} — похоже, на страницу входа: файл по логину и паролю API не отдан"
            ));
        }
        let length = header("content-length").and_then(|v| String::from_utf8_lossy(&v).trim().parse::<u64>().ok());
        let page =
            content_type.as_deref().is_some_and(|t| t.split(';').next().unwrap_or_default().trim().eq_ignore_ascii_case("text/html"));
        let refused = !(200..300).contains(&code);
        let rx = chunks(resp.into_body());
        if refused {
            let (bytes, cut) = head_within(&rx, idle, stop)?;
            return fail(self.refusal(code, "файл не найден", &path, CALL, &charset::decode(&bytes, charset.as_deref()), cut));
        }
        if page && !html_expected {
            let (bytes, cut) = head_within(&rx, idle, stop)?;
            let text = self.mask(&charset::decode(&bytes, charset.as_deref()));
            self.log_once(&format!("{CALL} html"), || {
                let part = if cut { ", только начало" } else { "" };
                format!(
                    "{CALL}: страница вместо файла на {path} ({} симв.{part}):\n{}",
                    text.chars().count(),
                    errors::head(&text, errors::LOGGED_CHARS)
                )
            });
            return fail(format!("сервер вернул страницу вместо файла:\n{}", evidence(&text)));
        }
        let mut done = Downloaded { length, ..Downloaded::default() };
        let size = |bytes: u64| export::files::size(bytes);
        loop {
            let read = match next_chunk(&rx, idle, stop)? {
                Chunk::Data(chunk) if chunk.is_empty() => return Ok(done),
                Chunk::Data(chunk) => chunk,
                Chunk::Silent => {
                    done.error = format!("{}: файл перестал приходить на {}", no_answer(idle), size(done.bytes));
                    return Ok(done);
                }
                Chunk::Broken(reason) => {
                    let at = if done.bytes > 0 { format!(" на {}", size(done.bytes)) } else { String::new() };
                    done.error = format!("соединение оборвалось{at}:\n{}", self.mask(&reason));
                    return Ok(done);
                }
            };
            to.write_all(&read)?;
            done.bytes += read.len() as u64;
            progress(done.bytes);
            if done.bytes >= stop_at {
                return Ok(done);
            }
        }
    }

    /// Ответы, которые не разобрались, и тела ошибок — в этот журнал (один образец на метод и код за запуск).
    pub fn with_log(mut self, log: Arc<Log>) -> Client {
        self.log = Some(log);
        self
    }

    /// Карточка заявки (`api/task/{id}?include=status`).
    pub fn get_task(&self, id: i32) -> Result<Task, String> {
        let json = self.get(&format!("api/task/{id}?include=status"), "заявка не найдена", "get_task")?;
        parse::task(&json, id).ok_or_else(|| self.unparsed(&json, "get_task"))
    }

    /// Страница истории (с первой), свежие сверху; первая — тем же адресом, что всегда, без `page`.
    pub fn get_lifetime_page(&self, id: i32, page: i32) -> Result<LifetimePage, String> {
        let mut path = format!("api/tasklifetime?taskid={id}&include=status&lastcommentsontop=true&pagesize={LIFETIME_PAGE_SIZE}");
        if page > 1 {
            path.push_str(&format!("&page={page}"));
        }
        let json = self.get(&path, "заявка не найдена", "get_lifetime_page")?;
        parse::lifetime(&json).ok_or_else(|| self.unparsed(&json, "get_lifetime_page"))
    }

    /// Вся история заявки (`ReadLifetimeAsync`): (записи, урезана ли). Сервер присылает номера страниц — верим им; не
    /// присылает — конец по неполной странице; не понимающий `page` отдаст ту же страницу — новых записей нет, стоп. Дошли
    /// до `max_pages`, а страницы ещё есть, — урезана: самые ранние записи не прочитаны. Без повторов; ошибка — вся ошибка.
    pub fn read_lifetime(&self, id: i32, max_pages: i32) -> Result<(Vec<Event>, bool), String> {
        let mut all: Vec<Event> = Vec::new();
        let mut keys = HashSet::new();
        for page in 1..=max_pages {
            let got = self.get_lifetime_page(id, page)?;
            let before = all.len();
            for e in &got.events {
                if keys.insert((e.date, e.author.clone(), e.status.clone(), e.comment.clone())) {
                    all.push(e.clone());
                }
            }
            let more = if got.paged { got.has_more } else { got.events.len() >= LIFETIME_PAGE_SIZE as usize };
            if all.len() == before || !more {
                break;
            }
            if page == max_pages {
                return Ok((all, true));
            }
        }
        Ok((all, false))
    }

    /// Текущий пользователь: номер — импорту («где я исполнитель»), номер и имя — автообновлению (свои комментарии не новые).
    pub fn get_current_user(&self) -> Result<User, String> {
        let json = self.get("api/user?getcurrentuserinfo=true", "не удалось определить пользователя", "get_current_user")?;
        parse::current_user(&json).ok_or_else(|| self.unparsed(&json, "get_current_user"))
    }

    /// Все статусы заявок: по признакам и названиям импорт решает, какие открытые.
    pub fn get_statuses(&self) -> Result<Vec<Status>, String> {
        let json = self.get("api/taskstatus", "по этому адресу нет API", "get_statuses")?;
        parse::statuses(&json).ok_or_else(|| self.unparsed(&json, "get_statuses"))
    }

    /// Проверка адреса и логина (`CheckAsync`): список статусов маленький; любой json — ответ API.
    pub fn check(&self) -> Result<(), String> {
        let json = self.get("api/taskstatus", "по этому адресу нет API", "check")?;
        if serde_json::from_str::<serde_json::Value>(&json).is_ok() {
            return Ok(());
        }
        self.unparsed(&json, "check");
        Err(format!("ответ не похож на API Интрасервиса:\n{}", evidence(&self.mask(&json))))
    }

    /// Страница списка по отбору (`GetTasksAsync`), свежие по изменению сверху.
    pub fn get_tasks(&self, query: &query::TaskQuery, page: i32, page_size: i32) -> Result<SearchPage, String> {
        self.tasks(query, page, page_size, "get_tasks")
    }

    /// Страница заявок, где пользователь — исполнитель, в этих статусах. Пустой список статусов — не «без фильтра» (притащил
    /// бы и закрытые): ошибка, а не запрос.
    pub fn get_executor_tasks(&self, executor_id: i32, status_ids: &[i32], page: i32) -> Result<SearchPage, String> {
        if status_ids.is_empty() {
            return Err("не задан список открытых статусов".into());
        }
        let q = query::TaskQuery { executor_ids: vec![executor_id], status_ids: status_ids.to_vec(), ..query::TaskQuery::default() };
        self.tasks(&q, page, EXECUTOR_PAGE_SIZE, "get_executor_tasks")
    }

    /// Мои открытые заявки, как их читает импорт (`FetchMyOpenAsync`): кто я, справочник статусов (каждый раз: новый открытый
    /// статус, не попавший в фильтр, выбросил бы свои заявки), открытые — не закрытые по признакам и по `closed`, страницы по
    /// 200, не больше 10. Только чтение.
    pub fn my_open(&self, closed: &ClosedNames) -> MyOpen {
        let fail = |error: String| MyOpen { error, ..MyOpen::default() };
        let me = match self.get_current_user() {
            Ok(me) => me,
            Err(e) => return fail(e),
        };
        let statuses = match self.get_statuses() {
            Ok(s) => s,
            Err(e) => return fail(e),
        };
        // пустой справочник и «все статусы закрытые» — разные беды: первая на сервере, вторую чинит сам пользователь
        if statuses.is_empty() {
            return fail("сервер не вернул ни одного статуса заявок".into());
        }
        let mut open: Vec<&Status> = Vec::new();
        for s in statuses.iter().filter(|s| !s.closed_by(closed)) {
            if !open.contains(&s) {
                open.push(s);
            }
        }
        if open.is_empty() {
            return fail("все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json".into());
        }
        let open: Vec<i32> = open.iter().map(|s| s.id).collect();
        let mut names = closed.clone();
        for s in statuses.iter().filter(|s| s.closed_by(closed)) {
            names.add(&s.name); // и названия закрытых по признаку — для F5 и отчётов
        }
        let pages = rules::read_all_pages(
            |page| self.get_executor_tasks(me.id, &open, page),
            EXECUTOR_PAGE_SIZE,
            IMPORT_PAGES,
            None,
            usize::MAX,
            None,
        );
        MyOpen { rows: pages.rows, total: pages.total, complete: pages.complete, error: pages.error, closed: names, me: Some(me) }
    }

    /// Сотрудники, чьё имя содержит text, — одна страница, на одного больше max (`FindUsersAsync`); число — всего подошло.
    pub fn find_users(&self, text: &str, max: i32) -> Result<(Vec<Ref>, i32), String> {
        let path = format!("api/user?fields=Id,Name&search={}&pagesize={}&page=1", query::escape_data_string(text.trim()), max + 1);
        let json = self.get(&path, "по этому адресу нет API", "find_users")?;
        let refs = parse::refs(&json, "Users", "UserList").ok_or_else(|| self.unparsed(&json, "find_users"))?;
        Ok((refs.items, refs.total))
    }

    /// Справочник постранично, пока Paginator обещает следующую (`GetRefsAsync`). Ошибка — что пришло до неё, то и отдаём,
    /// вместе с текстом ошибки. `paged: false` — ответ не листается (api/filter), параметры страницы не передаём.
    fn get_refs(&self, path: &str, name: &str, wrapper: &str, max_pages: i32, paged: bool, call: &str) -> (Vec<Ref>, String) {
        let mut items = Vec::new();
        for page in 1..=max_pages {
            let url = if paged {
                format!("{path}{}pagesize={REF_PAGE_SIZE}&page={page}", if path.contains('?') { '&' } else { '?' })
            } else {
                path.to_string()
            };
            let json = match self.get(&url, "по этому адресу нет API", call) {
                Ok(j) => j,
                Err(e) => return (items, e),
            };
            let Some(r) = parse::refs(&json, name, wrapper) else { return (items, self.unparsed(&json, call)) };
            let empty = r.items.is_empty();
            items.extend(r.items);
            if !paged || !r.has_more || empty {
                break;
            }
        }
        (items, String::new())
    }

    /// Сервисы для окна поиска (`GetServicesAsync`): весь справочник; отказ или пусто — назначенные на меня; нет и их —
    /// сервисы последних заявок. Сбой, который пройдёт сам, и неверный логин — не отказ: ошибка, окно спросит снова.
    pub fn get_services(&self) -> Services {
        const ALL: &str = "api/service?fields=Id,Name,Path,IsArchive&archive=true&inactive=true";
        // 500 — не сбой: так сервер отвечает и на запрет, которого не ждал, — тогда нужны запасные списки
        let retry =
            |e: &str| !e.is_empty() && (errors::http_code(e) == Some(401) || errors::is_transient(e) && errors::http_code(e) != Some(500));
        let fail = |error: String| Services { error, ..Services::default() };
        let (items, error) = self.get_refs(ALL, "Services", "ServiceList", 5, true, "get_services");
        if error.is_empty() && !items.is_empty() {
            return Services { items, ..Services::default() };
        }
        if retry(&error) {
            return fail(error);
        }
        let why = if error.is_empty() { "пустой ответ".to_string() } else { errors::headline(&error) };
        let own_path = format!("{ALL}&for=filtertasks");
        let (own, own_error) = self.get_refs(&own_path, "Services", "ServiceList", 5, true, "get_services");
        if retry(&own_error) {
            return fail(own_error);
        }
        if own_error.is_empty() && !own.is_empty() {
            let note = format!("Весь список сервисов сервер не отдал ({why}) — здесь те, на которые вы назначены.");
            return Services { items: own, error: String::new(), note };
        }
        let (seen, rows, seen_error) = self.get_task_services();
        if !seen.is_empty() {
            let note = format!(
                "Весь список сервисов сервер не отдал ({why}) — здесь сервисы последних {rows} заявок (вложенные — лишь те, что в списке)."
            );
            return Services { items: seen, error: String::new(), note };
        }
        // не вышло и из заявок — причина целиком: и почему не весь справочник, и почему не из заявок
        if !seen_error.is_empty() {
            return fail(format!("{why}; сервисы из заявок — {}", errors::headline(&seen_error)));
        }
        if let Some(first) = [error, own_error].into_iter().find(|e| !e.is_empty()) {
            return fail(first);
        }
        Services {
            note: "Сервер не вернул ни одного сервиса — ни списком, ни в заявках. Сервис можно задать сохранённым фильтром.".into(),
            ..Services::default()
        }
    }

    /// Сервисы, у которых есть заявки, — из блока Services страницы последних заявок (`GetTaskServicesAsync`): одна
    /// страница в `TASK_SERVICE_ROWS`, без счёта (не принят, 400 — со счётом), срок 30 с: сервер сортирует все видимые заявки.
    fn get_task_services(&self) -> (Vec<Ref>, usize, String) {
        let query = query::TaskQuery { include_archived: true, ..query::TaskQuery::default() };
        let page = |counted: bool| {
            let path = format!("{}&fields=Id,Name,ServiceId", query.to_url(1, TASK_SERVICE_ROWS, true, counted));
            self.get_within(&path, "по этому адресу нет API", "get_services", TASK_SERVICES_TIMEOUT)
        };
        let mut json = page(false);
        if matches!(&json, Err(e) if errors::http_code(e) == Some(400)) {
            json = page(true);
        }
        let json = match json {
            Ok(j) => j,
            Err(e) => return (Vec::new(), 0, e),
        };
        let Some(r) = parse::task_services(&json) else { return (Vec::new(), 0, self.unparsed(&json, "get_services")) };
        // по имени: окно показывает неполный список без отступов, и порядок по пути (номерам) выглядел бы случайным
        // ponytail: сравнение строк в нижнем регистре; порядок .NET по культуре (ё, знаки препинания) может отличаться
        let mut ordered: Vec<Ref> = Vec::new();
        for s in r.services {
            if !ordered.iter().any(|o| o.id == s.id) {
                ordered.push(s);
            }
        }
        ordered.sort_by_cached_key(|s| s.name.to_lowercase());
        (ordered, r.rows, String::new())
    }

    /// Типы заявок, с архивными (`GetTaskTypesAsync`).
    pub fn get_task_types(&self) -> Result<Vec<Ref>, String> {
        let (items, error) =
            self.get_refs("api/tasktype?fields=Id,Name,IsArchive&archive=true", "TaskTypes", "TaskTypeList", 5, true, "get_task_types");
        if error.is_empty() { Ok(items) } else { Err(error) }
    }

    /// Сохранённые фильтры заявок из веб-интерфейса (`GetSavedFiltersAsync`).
    pub fn get_saved_filters(&self) -> Result<Vec<Ref>, String> {
        let (items, error) = self.get_refs("api/filter?resource=task", "FilterView", "ArrayOfFilterView", 1, false, "get_saved_filters");
        if error.is_empty() { Ok(items) } else { Err(error) }
    }

    /// Страница списка окна поиска: 50 заявок, с названиями сервисов, со счётом.
    pub fn find_tasks(&self, query: &query::TaskQuery, page: i32) -> Result<SearchPage, String> {
        let json = self.get(&query.to_url(page, crate::search::PAGE_SIZE, true, true), "ничего не найдено", "find_tasks")?;
        parse::search(&json).ok_or_else(|| self.unparsed(&json, "find_tasks"))
    }

    /// Страница списка для выгрузки по отбору (`GetTasksAsync` с сроком): `EXECUTOR_PAGE_SIZE` заявок, с названиями сервисов;
    /// counted: false — без счёта (`count=false`), сервер говорит, есть ли следующая страница.
    pub fn get_task_list(&self, query: &query::TaskQuery, page: i32, counted: bool, timeout: Duration) -> Result<SearchPage, String> {
        let path = query.to_url(page, EXECUTOR_PAGE_SIZE, true, counted);
        let json = self.get_within(&path, "по этому адресу нет API", "get_task_list", timeout)?;
        parse::search(&json).ok_or_else(|| self.unparsed(&json, "get_task_list"))
    }

    /// Первая страница списка в `page_size` строк за `timeout` (`GetTasksAsync(query, 1, ct, pageSize, timeout:)`): без названий
    /// сервисов, со счётом.
    pub fn get_tasks_page(&self, query: &query::TaskQuery, page_size: i32, timeout: Duration) -> Result<SearchPage, String> {
        let json = self.get_within(&query.to_url(1, page_size, false, true), "по этому адресу нет API", "get_tasks", timeout)?;
        parse::search(&json).ok_or_else(|| self.unparsed(&json, "get_tasks"))
    }

    fn tasks(&self, query: &query::TaskQuery, page: i32, page_size: i32, call: &str) -> Result<SearchPage, String> {
        let json = self.get(&query.to_url(page, page_size, false, true), "по этому адресу нет API", call)?;
        parse::search(&json).ok_or_else(|| self.unparsed(&json, call))
    }

    /// GET {адрес}/{path} (`GetAsync`): тело успешного ответа — строкой; ошибка — фразой C#: срок, «сервер недоступен»,
    /// «соединение оборвалось», переадресация, отказ по коду (`refusal`).
    fn get(&self, path: &str, not_found: &str, call: &str) -> Result<String, String> {
        self.get_within(path, not_found, call, self.timeout)
    }

    /// То же со своим сроком на этот запрос (у `GetAsync` — параметр `timeout`).
    fn get_within(&self, path: &str, not_found: &str, call: &str, limit: Duration) -> Result<String, String> {
        let url = format!("{}/{path}", self.base);
        let request = self.agent.get(&url).config().timeout_global(Some(limit)).build();
        let mut resp = match request.header("Authorization", &self.auth).header("Accept", "application/json").call() {
            Ok(r) => r,
            Err(e) => return Err(self.failure(e, limit)),
        };
        let code = resp.status().as_u16();
        let charset = charset_of(resp.headers().get("content-type").and_then(|v| v.to_str().ok()));
        if (300..400).contains(&code) {
            let location = resp.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            return Err(format!("переадресация (HTTP {code}) на {} — похоже на страницу входа, а не на API", self.mask(&location)));
        }
        if !(200..300).contains(&code) {
            let (bytes, cut) = head_of(resp.body_mut().as_reader());
            return Err(self.refusal(code, not_found, path, call, &charset::decode(&bytes, charset.as_deref()), cut));
        }
        match resp.body_mut().with_config().limit(BODY_LIMIT).read_to_vec() {
            Ok(bytes) => Ok(charset::decode(&bytes, charset.as_deref())),
            Err(ureq::Error::BodyExceedsLimit(_)) => Err(format!("ответ сервера больше {} МБ — не читаю", BODY_LIMIT / 1024 / 1024)),
            Err(e) => Err(self.broken(e, limit)),
        }
    }

    /// Сетевая ошибка до ответа — фразой C# (`HttpRequestException`): срок, адрес не годится, иначе «сервер недоступен».
    fn failure(&self, e: ureq::Error, limit: Duration) -> String {
        match e {
            ureq::Error::Timeout(_) => no_answer(limit),
            ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => no_answer(limit),
            e @ (ureq::Error::BadUri(_) | ureq::Error::Http(_)) => {
                format!("адрес Интрасервиса в настройках не годится: {}", self.mask(&e.to_string()))
            }
            e => format!("сервер недоступен:\n{}", self.mask(&e.to_string())),
        }
    }

    /// Ошибка посреди тела ответа (`IOException` у C#): срок — «не ответил», остальное — «соединение оборвалось».
    fn broken(&self, e: ureq::Error, limit: Duration) -> String {
        match e {
            ureq::Error::Timeout(_) => no_answer(limit),
            ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => no_answer(limit),
            e => format!("соединение оборвалось:\n{}", self.mask(&e.to_string())),
        }
    }

    /// Ответ с кодом ошибки (`RefusalAsync`): первая строка — фраза по коду, дальше — что сервер ответил; тело — в журнал.
    fn refusal(&self, code: u16, not_found: &str, path: &str, call: &str, text: &str, cut: bool) -> String {
        let what = match code {
            401 => "неверный логин или пароль",
            403 => "нет доступа",
            404 => not_found,
            _ => "ошибка сервера",
        };
        let body = self.mask(text);
        if body.trim().is_empty() {
            return format!("{what} (HTTP {code})");
        }
        self.log_once(&format!("{call} HTTP {code}"), || {
            let part = if cut { ", только начало" } else { "" };
            format!("{call}: HTTP {code} на {path} ({} симв.{part}):\n{}", body.chars().count(), errors::head(&body, errors::LOGGED_CHARS))
        });
        format!("{what} (HTTP {code}):\n{}", evidence(&body))
    }

    /// Ответ пришёл, но не разобрался (`Unparsed`): его начало — в журнал, в окно — фраза и что пришло.
    fn unparsed(&self, json: &str, call: &str) -> String {
        let shown = self.mask(json);
        self.log_once(call, || {
            format!("{call}: не разобран ответ сервера ({} симв.):\n{}", json.chars().count(), errors::head(&shown, errors::LOGGED_CHARS))
        });
        format!("непонятный ответ сервера:\n{}", evidence(&shown))
    }

    fn log_once(&self, key: &str, text: impl FnOnce() -> String) {
        if let Some(log) = &self.log {
            log.once(key, text);
        }
    }

    fn mask(&self, text: &str) -> String {
        redact(text, &[&self.secrets[0], &self.secrets[1]])
    }
}

/// Начало тела ответа с ошибкой (`HeadAsync`): не больше `ERROR_BODY_BYTES`; cut — конца не видели: за пределом было ещё
/// (проба — ещё байт), или оборвалось, или замолчало до срока. Тогда — что успело прийти.
fn head_of(mut body: impl std::io::Read) -> (Vec<u8>, bool) {
    let mut buf = vec![0u8; errors::ERROR_BODY_BYTES];
    let mut n = 0;
    while n < buf.len() {
        match body.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => {
                buf.truncate(n);
                return (buf, true);
            }
        }
    }
    buf.truncate(n);
    let cut = n == errors::ERROR_BODY_BYTES && body.read(&mut [0u8; 1]).map_or(true, |k| k > 0);
    (buf, cut)
}

/// Срок чтения вышел: у ureq это `Error::Timeout` внутри `io::Error` (вид у него — Other, не TimedOut).
fn is_timeout(e: &std::io::Error) -> bool {
    e.get_ref().is_some_and(|i| matches!(i.downcast_ref::<ureq::Error>(), Some(ureq::Error::Timeout(_))))
}

/// Тело ответа кусками по 80 КБ из отдельного потока, через канал без запаса: ждущий кусок сам отмеряет его срок (у ureq срок
/// чтения сокета один на заголовки и тело). Пустой кусок — конец. ponytail: замолчавший сервер держит поток до потолка
/// `timeout_per_read` или до закрытия соединения.
fn chunks(body: ureq::Body) -> std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>> {
    use std::io::Read;
    let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(0);
    let mut reader = body.into_reader();
    std::thread::spawn(move || {
        let mut buffer = vec![0u8; 81920];
        loop {
            let chunk = match reader.read(&mut buffer) {
                Ok(n) => Ok(buffer[..n].to_vec()),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => Err(e),
            };
            let last = chunk.as_ref().map_or(true, |c| c.is_empty());
            if tx.send(chunk).is_err() || last {
                break;
            }
        }
    });
    rx
}

/// Что пришло за срок куска.
enum Chunk {
    Data(Vec<u8>),
    Silent,
    Broken(String),
}

/// Следующий кусок за `idle`; «Остановить» слушается раз в 100 мс (у C# отмена мгновенна). Поток чтения пропал (паника) —
/// обрыв: недочитанный файл не должен сойти за целый.
fn next_chunk(rx: &std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>, idle: Duration, stop: &dyn Fn() -> bool) -> std::io::Result<Chunk> {
    use std::sync::mpsc::RecvTimeoutError;
    let until = std::time::Instant::now() + idle;
    loop {
        if stop() {
            return Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        }
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Ok(Chunk::Silent);
        }
        return Ok(match rx.recv_timeout(left.min(Duration::from_millis(100))) {
            Ok(Ok(chunk)) => Chunk::Data(chunk),
            Ok(Err(e)) if is_timeout(&e) => Chunk::Silent,
            Ok(Err(e)) => Chunk::Broken(e.to_string()),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => Chunk::Broken("чтение ответа прервалось".into()),
        });
    }
}

/// Начало тела отказа или страницы (`HeadAsync` с `start.CancelAfter(idleLimit)`): не больше `ERROR_BODY_BYTES` за одно окно
/// `idle` на всё тело; оборвалось, замолчало или окно кончилось — cut и что успело прийти; остановили — `Interrupted`.
fn head_within(
    rx: &std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    idle: Duration,
    stop: &dyn Fn() -> bool,
) -> std::io::Result<(Vec<u8>, bool)> {
    let until = std::time::Instant::now() + idle;
    let mut body = Vec::new();
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        match next_chunk(rx, left, stop)? {
            Chunk::Data(chunk) if chunk.is_empty() => return Ok((body, false)),
            Chunk::Data(chunk) => {
                body.extend_from_slice(&chunk);
                // на пределе целым тело назовёт только конец сразу за ним: прочитано больше — cut
                if body.len() > errors::ERROR_BODY_BYTES {
                    body.truncate(errors::ERROR_BODY_BYTES);
                    return Ok((body, true));
                }
            }
            Chunk::Silent | Chunk::Broken(_) => return Ok((body, true)),
        }
    }
}

/// Кодировка из `Content-Type`: значение `charset=` (без учёта регистра имени), None — не указана.
fn charset_of(content_type: Option<&str>) -> Option<String> {
    content_type?.split(';').skip(1).find_map(|p| {
        let (name, value) = p.split_once('=')?;
        name.trim().eq_ignore_ascii_case("charset").then(|| value.trim().to_string())
    })
}

/// Срок запроса истёк: фраза называет срок этого запроса (у страницы заявок для сервисов — свой, 30 с).
fn no_answer(limit: Duration) -> String {
    format!("сервер не ответил за {} с", seconds(limit))
}

/// Срок словами, как `{0:0.##}` C# на русской Windows: «10», «1,5», «0,25».
fn seconds(d: Duration) -> String {
    let hundredths = (d.as_millis() + 5) / 10;
    let (whole, frac) = (hundredths / 100, hundredths % 100);
    match frac {
        0 => whole.to_string(),
        f if f % 10 == 0 => format!("{whole},{}", f / 10),
        f => format!("{whole},{f:02}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Тело ответа по кускам: Interrupted — повторить, ошибка — обрыв.
    struct Chunks(Vec<std::io::Result<Vec<u8>>>);

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            match self.0.remove(0) {
                Ok(bytes) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Err(e) => Err(e),
            }
        }
    }

    #[test]
    fn error_bodies_are_read_through_interruptions() {
        let interrupted = || Err(std::io::Error::from(std::io::ErrorKind::Interrupted));
        let broken = || Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        // Interrupted — не обрыв: читаем дальше, тело целое
        assert_eq!(head_of(Chunks(vec![Ok(b"ab".to_vec()), interrupted(), Ok(b"cd".to_vec())])), (b"abcd".to_vec(), false));
        // оборвалось — что пришло, и «урезано»
        assert_eq!(head_of(Chunks(vec![Ok(b"ab".to_vec()), broken()])), (b"ab".to_vec(), true));
        // ровно предел и конец; предел и ещё байт; предел и обрыв на пробе
        let full = vec![b'x'; errors::ERROR_BODY_BYTES];
        assert!(!head_of(Chunks(vec![Ok(full.clone())])).1, "ровно предел — не урезано");
        assert_eq!(head_of(Chunks(vec![Ok(full.clone()), Ok(b"y".to_vec())])), (full.clone(), true));
        assert_eq!(head_of(Chunks(vec![Ok(full.clone()), broken()])), (full, true));
    }

    #[test]
    fn timeouts_in_words() {
        // как {0:0.##} на русской Windows
        let s = |ms| seconds(Duration::from_millis(ms));
        assert_eq!(
            (s(10_000), s(1500), s(1250), s(250), s(60_000)),
            ("10".into(), "1,5".into(), "1,25".into(), "0,25".into(), "60".into())
        );
    }

    /// Срок, о котором сказала сама система (`Io` с TimedOut, а не таймер ureq), — тоже «не ответил»: и до ответа, и посреди
    /// тела. Поддельному серверу его не устроить — ureq 3.5 сам переводит такие ошибки сокета в `Error::Timeout`.
    #[test]
    fn an_os_timeout_is_a_timeout() {
        let client = Client::new("https://sd.example.ru/", "u", "p", Duration::from_millis(1500));
        let os = || ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut));
        let reset = || ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(
            (client.failure(os(), client.timeout), client.broken(os(), client.timeout)),
            ("сервер не ответил за 1,5 с".into(), "сервер не ответил за 1,5 с".into())
        );
        let (before, during) = (client.failure(reset(), client.timeout), client.broken(reset(), client.timeout));
        assert!(before.starts_with("сервер недоступен:\n") && during.starts_with("соединение оборвалось:\n"), "{before} | {during}");
    }

    /// Срок запроса — его собственный, а не общий срок клиента (страница заявок для сервисов ждёт 30 с, у `GetAsync` C# это
    /// параметр `timeout`): фраза о сроке называет тот, что был у запроса. 30 с ждать долго — срок взят короткий, смысл тот же.
    #[test]
    fn the_timeout_text_names_the_limit_of_the_request() {
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = Client::new(&format!("http://{}", silent.local_addr().unwrap()), "u", "p", Duration::from_secs(10));
        // сервер принимает соединение и молчит; общий срок клиента — 10 с, у запроса — 0,3 с
        let said = client.get_within("api/x", "нет", "check", Duration::from_millis(300));
        assert_eq!(said, Err("сервер не ответил за 0,3 с".to_string()), "в фразе — срок запроса, а не клиента");
        // страница заявок для сервисов — 30 с
        let os = ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert_eq!(client.failure(os, TASK_SERVICES_TIMEOUT), "сервер не ответил за 30 с");
    }

    #[test]
    fn the_charset_of_a_content_type() {
        assert_eq!(charset_of(Some("text/html; Charset=\"windows-1251\"")), Some("\"windows-1251\"".into()));
        assert_eq!(charset_of(Some("application/json; boundary=x; charset=utf-8")), Some("utf-8".into()));
        assert_eq!((charset_of(Some("application/json")), charset_of(None)), (None, None));
    }
}
