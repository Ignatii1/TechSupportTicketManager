//! Отбор заявок для `api/task` (`TaskQuery`): всё, чем список заявок умеет фильтровать (док., стр. 14-20), по «И»; пусто —
//! условия нет. ponytail: страховки от условия, которого сервер не понял (`Outside`, `PastEnd`), — со слоем поиска и выгрузки.

use chrono::NaiveDateTime;

/// Свежие по изменению сверху — как список видит человек.
pub const DEFAULT_SORT: &str = "Changed desc";
/// Для чтения всего списка часами: по созданию и номеру — дата создания не меняется, список не «едет».
pub const STABLE_SORT: &str = "Created asc, Id asc";
/// По изменению от давних и номеру: так читают автовыгрузка и второй проход выгрузки с пределом.
pub const CHANGED_ASC_SORT: &str = "Changed asc, Id asc";
/// Поля строки для запроса с сохранённым фильтром (он сам задаёт, какие поля вернуть).
pub const ROW_FIELDS: &str = "Id,Name,Description,StatusId,Created,Changed,Creator,CreatorId,Executors,ExecutorIds,\
ExecutorGroup,ExecutorGroupId,ServiceId,TypeId,Type,Categories,ResolutionDateFact,Closed,ObserverIds,Observers";

/// Период по дате, границы включительно (`…MoreThan` — «не раньше», `…LessThan` — «не позже»); None — не задана. Даты — по
/// часам пользователя Интрасервиса, как их ждёт API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateSpan {
    pub from: Option<NaiveDateTime>,
    pub to: Option<NaiveDateTime>,
}

/// Отбор: номера в списках — через запятую (подходит хотя бы один); `filter_id` — сохранённый фильтр веб-интерфейса;
/// `include_archived` — и заявки архивных и неактуальных сервисов; `sort` — «Поле asc|desc», через запятую несколько.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskQuery {
    pub executor_ids: Vec<i32>,
    pub status_ids: Vec<i32>,
    pub search: String,
    pub creator_ids: Vec<i32>,
    pub service_ids: Vec<i32>,
    pub type_ids: Vec<i32>,
    pub created: DateSpan,
    pub changed: DateSpan,
    pub closed: DateSpan,
    pub filter_id: Option<i32>,
    pub include_archived: bool,
    pub sort: String,
    pub observer_ids: Vec<i32>,
}

impl Default for TaskQuery {
    fn default() -> Self {
        TaskQuery {
            executor_ids: Vec::new(),
            status_ids: Vec::new(),
            search: String::new(),
            creator_ids: Vec::new(),
            service_ids: Vec::new(),
            type_ids: Vec::new(),
            created: DateSpan::default(),
            changed: DateSpan::default(),
            closed: DateSpan::default(),
            filter_id: None,
            include_archived: false,
            sort: DEFAULT_SORT.into(),
            observer_ids: Vec::new(),
        }
    }
}

impl TaskQuery {
    /// Адрес страницы списка (`ToUrl`). detailed — названия сервисов в том же ответе (`include=service`); counted: false —
    /// сервер не считает общее число, а говорит, есть ли следующая страница (`count=false`; `count=all` живой сервер не
    /// принимает — HTTP 400).
    pub fn to_url(&self, page: i32, page_size: i32, detailed: bool, counted: bool) -> String {
        let mut url = String::from("api/task?");
        let ids = |url: &mut String, key: &str, ids: &[i32]| {
            if !ids.is_empty() {
                let list: Vec<String> = ids.iter().map(i32::to_string).collect();
                url.push_str(&format!("{key}={}&", list.join(",")));
            }
        };
        ids(&mut url, "ExecutorIds", &self.executor_ids);
        ids(&mut url, "StatusIds", &self.status_ids);
        ids(&mut url, "CreatorIds", &self.creator_ids);
        ids(&mut url, "ServiceIds", &self.service_ids);
        ids(&mut url, "TypeIds", &self.type_ids);
        ids(&mut url, "ObserverIds", &self.observer_ids);
        if !self.search.trim().is_empty() {
            url.push_str(&format!("search={}&", escape_data_string(self.search.trim())));
        }
        for (key, span) in [("Created", self.created), ("Changed", self.changed), ("Closed", self.closed)] {
            if let Some(from) = span.from {
                url.push_str(&format!("{key}MoreThan={}&", escape_data_string(&stamp(from))));
            }
            if let Some(to) = span.to {
                url.push_str(&format!("{key}LessThan={}&", escape_data_string(&stamp(to))));
            }
        }
        if let Some(filter) = self.filter_id {
            url.push_str(&format!("filterid={filter}&fields={ROW_FIELDS}&"));
        }
        if self.include_archived {
            url.push_str("archive=true&inactive=true&");
        }
        url.push_str(if detailed { "include=status,service&" } else { "include=status&" });
        if !counted {
            url.push_str("count=false&");
        }
        // запятая между полями сортировки — как в примере документации (стр. 15), не «%2C»
        url.push_str(&format!("sort={}&pagesize={page_size}&page={}", escape_data_string(&self.sort).replace("%2C", ","), page.max(1)));
        url
    }

    /// Условий нет вовсе: список — все заявки, которые видит учётная запись.
    pub fn is_unrestricted(&self) -> bool {
        let empty = |span: DateSpan| span.from.is_none() && span.to.is_none();
        [&self.executor_ids, &self.status_ids, &self.creator_ids, &self.service_ids, &self.type_ids, &self.observer_ids]
            .iter()
            .all(|ids| ids.is_empty())
            && self.search.trim().is_empty()
            && empty(self.created)
            && empty(self.changed)
            && empty(self.closed)
            && self.filter_id.is_none()
    }
}

/// Дата для запроса — единственный формат из примеров документации с временем («2015-11-13 10:00»).
fn stamp(d: NaiveDateTime) -> String {
    d.format("%Y-%m-%d %H:%M").to_string()
}

/// `Uri.EscapeDataString`: незарезервированные (RFC 3986) — как есть, остальное — `%XX` по байтам UTF-8, заглавными.
pub fn escape_data_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
