//! Форма поиска без окна (`TicketSearch.cs`): даты полей «с» и «по» → период.

use chrono::{Days, NaiveDate};

use crate::intraservice::query::{DateSpan, TaskQuery};
use crate::intraservice::{Client, Found, SearchPage, Status};
use crate::rules::ClosedNames;

/// Число из цифр ASCII ровно такой длины (как точные форматы .NET: ни знака, ни пробела, ни чужих цифр).
fn digits(s: &str, min: usize, max: usize) -> Option<u32> {
    if s.len() < min || s.len() > max || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Дата в одной из записей `DateFormats`: `dd.MM.yyyy`, `d.M.yyyy`, `dd.MM.yy`, `yyyy-MM-dd`. Своя проверка записи, а не
/// `parse_from_str` chrono: он мягче точного разбора .NET (необязательные нули, знаки, пробелы внутри).
fn read_date(text: &str) -> Option<NaiveDate> {
    let (d, m, y) = if let [d, m, y] = text.split('.').collect::<Vec<_>>()[..] {
        let year = if y.len() == 2 {
            // двухзначный год — только в записи `dd.MM.yy`: и день, и месяц в две цифры
            if d.len() != 2 || m.len() != 2 {
                return None;
            }
            // век — как у .NET в инвариантной культуре: 00–49 — 20xx; 50–99 там 19xx, а это раньше 2000 — отказ сразу.
            // ponytail: порог 2049 постоянный — так отвечает .NET 10 без настроек системы (таблица сверки; её же
            // сверяет CI на Windows). Возьми C# порог из настроек региона Windows — ответы разошлись бы на годах
            // около него; на рабочем ПК не проверено.
            match digits(y, 2, 2)? {
                yy @ 0..=49 => 2000 + yy,
                _ => return None,
            }
        } else {
            digits(y, 4, 4)?
        };
        (digits(d, 1, 2)?, digits(m, 1, 2)?, year)
    } else if let [y, m, d] = text.split('-').collect::<Vec<_>>()[..] {
        (digits(d, 2, 2)?, digits(m, 2, 2)?, digits(y, 4, 4)?)
    } else {
        return None;
    };
    NaiveDate::from_ymd_opt(y as i32, m, d)
}

/// `TryDate`: пусто — подходит (даты нет); непусто — только если читается и год 2000–2100.
fn try_date(text: &str) -> Option<Option<NaiveDate>> {
    let t = text.trim();
    if t.is_empty() {
        return Some(None);
    }
    let d = read_date(t)?;
    (2000..=2100).contains(&chrono::Datelike::year(&d)).then_some(Some(d))
}

/// Период из двух дат формы (`TicketSearch.ParseSpan`): с начала первого дня до начала дня после последнего; ошибка —
/// текст для окна (проверяется сначала «с», потом «по», потом порядок).
pub fn parse_span(from: &str, to: &str, label: &str) -> Result<DateSpan, String> {
    let bad = |t: &str| format!("{label}: дата «{}» не понятна — нужно дд.мм.гггг, год 2000–2100", t.trim());
    let a = try_date(from).ok_or_else(|| bad(from))?;
    let b = try_date(to).ok_or_else(|| bad(to))?;
    if let (Some(start), Some(end)) = (a, b)
        && start > end
    {
        return Err(format!("{label}: начало периода позже его конца"));
    }
    let midnight = |d: NaiveDate| d.and_hms_opt(0, 0, 0);
    Ok(DateSpan {
        from: a.and_then(midnight),
        // год ≤ 2100, день после него всегда есть
        to: b.and_then(|d| d.checked_add_days(Days::new(1))).and_then(midnight),
    })
}

/// Слова короче — слишком много совпадений (`TicketSearch.MinWordsLength`).
pub const MIN_WORDS: usize = 3;
/// Сколько сотрудников может подойти под одно имя (`TicketSearch.MaxUsersPerName`).
pub const MAX_USERS_PER_NAME: i32 = 10;
/// Заявок на страницу списка окна.
pub const PAGE_SIZE: i32 = 50;
/// Больше этого в список не набираем.
pub const MAX_SHOWN: usize = 1000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SearchStatus {
    #[default]
    Any,
    Open,
    Closed,
}

/// Условия формы поиска.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchFilter {
    pub words: String,
    pub mine: bool,
    pub executor: String,
    pub creator: String,
    pub observer_mine: bool,
    pub observer: String,
    pub status: SearchStatus,
    pub created_from: String,
    pub created_to: String,
    pub changed_from: String,
    pub changed_to: String,
    pub closed_from: String,
    pub closed_to: String,
    pub include_archived: bool,
}

// ЗАГЛУШКА карточки 3.2-2: заведомо неверна (должно быть include_archived: true)
impl Default for SearchFilter {
    fn default() -> Self {
        SearchFilter {
            words: String::new(),
            mine: false,
            executor: String::new(),
            creator: String::new(),
            observer_mine: false,
            observer: String::new(),
            status: SearchStatus::Any,
            created_from: String::new(),
            created_to: String::new(),
            changed_from: String::new(),
            changed_to: String::new(),
            closed_from: String::new(),
            closed_to: String::new(),
            include_archived: false,
        }
    }
}

impl SearchFilter {
    /// Старт окна: мои закрытые. ЗАГЛУШКА.
    pub fn usual() -> Self {
        Self::default()
    }

    /// Enter на доске: слова без пробелов по краям, остальное по умолчанию. ЗАГЛУШКА.
    pub fn quick(_words: &str) -> Self {
        Self::default()
    }
}

/// Запрос по условиям и пояснения над списком.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub query: TaskQuery,
    pub notes: Vec<String>,
}

/// Условия формы → запрос или текст ошибки (`TicketSearch.ResolveAsync`). ЗАГЛУШКА.
pub fn resolve(_client: &Client, _f: &SearchFilter, _statuses: &[Status], _closed: &ClosedNames) -> Result<Resolved, String> {
    Err(String::new())
}

/// «Сервис · тип» строки списка (`FoundTicketViewModel.Service`). ЗАГЛУШКА.
pub fn service_of(_f: &Found) -> String {
    String::new()
}

/// «Автор · исполнители · даты» строки списка (`FoundTicketViewModel.Details`). ЗАГЛУШКА.
pub fn details_of(_f: &Found) -> String {
    String::new()
}

/// Число с неразрывным пробелом между тысячами. ЗАГЛУШКА.
pub fn thousands(_n: i32) -> String {
    String::new()
}

/// Список найденного: страницы, склеенные без повторов. ЗАГЛУШКА.
#[derive(Clone, Debug, Default)]
pub struct Results {
    pub rows: Vec<Found>,
    pub total: i32,
    pub has_more: bool,
    pub pages: i32,
    pub warning: String,
}

impl Results {
    /// Добавить страницу, пришедшую по запросу. ЗАГЛУШКА.
    pub fn add_page(&mut self, _page: SearchPage, _query: &TaskQuery) {}

    /// Итог над списком. ЗАГЛУШКА.
    pub fn message(&self) -> String {
        String::new()
    }
}
