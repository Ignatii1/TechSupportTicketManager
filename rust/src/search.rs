//! Форма поиска без окна (`TicketSearch.cs`): даты полей «с» и «по» → период.

use chrono::{Days, NaiveDate};

use crate::intraservice::errors::capped;
use crate::intraservice::query::{DateSpan, TaskQuery};
use crate::intraservice::{Client, Found, Ref, SearchPage, Status};
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
    One,
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
    pub status_id: i32,
    pub service_id: i32,
    pub with_children: bool,
    pub type_id: i32,
    pub saved_filter_id: i32,
}

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
            include_archived: true,
            status_id: 0,
            service_id: 0,
            with_children: false, // ЗАГЛУШКА карточки 3.3-1: у C# true
            type_id: 0,
            saved_filter_id: 0,
        }
    }
}

impl SearchFilter {
    /// Старт окна: мои закрытые.
    pub fn usual() -> Self {
        SearchFilter { mine: true, status: SearchStatus::Closed, ..Self::default() }
    }

    /// Enter на доске: слова без пробелов по краям, остальное по умолчанию.
    pub fn quick(words: &str) -> Self {
        SearchFilter { words: words.trim().to_string(), ..Self::default() }
    }
}

/// Запрос по условиям и пояснения над списком.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub query: TaskQuery,
    pub notes: Vec<String>,
}

/// Условия формы → запрос или текст ошибки (`TicketSearch.ResolveAsync`). Дешёвые проверки — до первого обращения к
/// серверу; порядок проверок и запросов — как у C#.
pub fn resolve(client: &Client, f: &SearchFilter, statuses: &[Status], services: &[Ref], closed: &ClosedNames) -> Result<Resolved, String> {
    let _ = services; // ЗАГЛУШКА карточки 3.3-1
    let words = f.words.trim();
    // длина в единицах UTF-16, как у .NET
    if (1..MIN_WORDS).contains(&words.encode_utf16().count()) {
        return Err(format!("Слова — минимум {MIN_WORDS} символа"));
    }
    let created = parse_span(&f.created_from, &f.created_to, "Создана")?;
    let changed = parse_span(&f.changed_from, &f.changed_to, "Изменена")?;
    let closed_span = parse_span(&f.closed_from, &f.closed_to, "Закрыта")?;

    let status_ids = match f.status {
        SearchStatus::Any => Vec::new(),
        wanted => {
            if statuses.is_empty() {
                return Err("Список статусов не загрузился — «закрытые» и «открытые» определить нечем; выберите статус «Любой»".into());
            }
            let want_closed = wanted == SearchStatus::Closed;
            let ids: Vec<i32> = statuses.iter().filter(|s| s.closed_by(closed) == want_closed).map(|s| s.id).collect();
            if ids.is_empty() {
                return Err(if want_closed {
                    "Сервер не назвал ни одного закрытого статуса".into()
                } else {
                    "Все статусы считаются закрытыми — проверьте ClosedStatusNames в settings.json".into()
                });
            }
            ids
        }
    };

    let mut notes = Vec::new();
    let mut executors = Vec::new();
    let mut observers = Vec::new();
    if f.mine || f.observer_mine {
        let me = client.get_current_user()?;
        if f.mine {
            executors.push(me.id);
        }
        if f.observer_mine {
            observers.push(me.id);
        }
    }
    if !f.executor.trim().is_empty() {
        executors.extend(find_people(client, f.executor.trim(), "Исполнитель", &mut notes)?);
    }
    let mut creators = Vec::new();
    if !f.creator.trim().is_empty() {
        creators = find_people(client, f.creator.trim(), "Заявитель", &mut notes)?;
    }
    if !f.observer.trim().is_empty() {
        observers.extend(find_people(client, f.observer.trim(), "Наблюдатель", &mut notes)?);
    }

    let query = TaskQuery {
        executor_ids: distinct(executors),
        status_ids,
        search: words.to_string(),
        creator_ids: creators,
        created,
        changed,
        closed: closed_span,
        include_archived: f.include_archived,
        observer_ids: distinct(observers),
        ..TaskQuery::default()
    };
    // у исполнителя «Я» стоит по умолчанию, а условия — по «И»: вместе с наблюдателем это только мои заявки с ним — не все его
    if f.mine && f.observer_mine {
        notes.push(
            "«Я» и у исполнителя, и у наблюдателя — это заявки, где вы и то и другое; все, где вы наблюдатель, — без «Я» у исполнителя"
                .into(),
        );
    } else if f.mine && !f.observer.trim().is_empty() {
        notes.push(format!(
            "«Я» у исполнителя и наблюдатель «{}» — это только ваши заявки с этим наблюдателем; все заявки этого наблюдателя — без «Я» у исполнителя",
            f.observer.trim()
        ));
    }
    // ничего не выбрано — это не «пусто», а все заявки
    if query.is_unrestricted() {
        notes.push("Условий нет: ищу среди всех заявок, которые видит ваша учётная запись".into());
    }
    Ok(Resolved { query, notes })
}

/// Сервис и все вложенные в него (`TicketSearch.ServiceWithChildren`).
pub fn service_with_children(_services: &[Ref], _id: i32) -> Vec<i32> {
    Vec::new() // ЗАГЛУШКА карточки 3.3-1
}

/// Подпись сервиса в списке выбора (`TicketSearch.ServiceLabel`).
pub fn service_label(_s: &Ref) -> String {
    String::new() // ЗАГЛУШКА карточки 3.3-1
}

/// Подпись типа в списке выбора (`SearchViewModel.FillChoices`).
pub fn type_label(_t: &Ref) -> String {
    String::new() // ЗАГЛУШКА карточки 3.3-1
}

/// Подпись сохранённого фильтра в списке выбора (`SearchViewModel.FillChoices`).
pub fn filter_label(_f: &Ref) -> String {
    String::new() // ЗАГЛУШКА карточки 3.3-1
}

/// Пояснение над списком о вложенных сервисах (`SearchViewModel.Search`).
pub fn children_note(_f: &SearchFilter, _services_loaded: bool, _partial: bool) -> Option<String> {
    None // ЗАГЛУШКА карточки 3.3-1
}

/// Без повторов, в порядке появления.
fn distinct(ids: Vec<i32>) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::with_capacity(ids.len());
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Номера сотрудников, чьё имя содержит text. Никого или слишком много — ошибка: молча искать «по всем Ивановым» или «ни по
/// кому» значило бы показать не то, что просили (`FindPeopleAsync`).
fn find_people(client: &Client, text: &str, label: &str, notes: &mut Vec<String>) -> Result<Vec<i32>, String> {
    let (users, total) =
        client.find_users(text, MAX_USERS_PER_NAME).map_err(|e| format!("{label} «{text}»: не удалось найти сотрудника\n{e}"))?;
    if users.is_empty() {
        return Err(format!("{label} «{text}»: такого сотрудника не нашёл — достаточно части фамилии, но верной"));
    }
    if users.len() > MAX_USERS_PER_NAME as usize || total > MAX_USERS_PER_NAME {
        return Err(format!("{label} «{text}»: подходит больше {MAX_USERS_PER_NAME} сотрудников — уточните"));
    }
    notes.push(if users.len() == 1 {
        format!("{label}: {}", users[0].name)
    } else {
        let names: Vec<&str> = users.iter().map(|u| u.name.as_str()).collect();
        format!("{label} «{text}»: подошли {} — {}", users.len(), names.join(", "))
    });
    Ok(users.iter().map(|u| u.id).collect())
}

/// «Сервис · тип» строки списка (`FoundTicketViewModel.Service`).
pub fn service_of(f: &Found) -> String {
    let (service, kind) = f.extra.as_ref().map_or((None, None), |e| (e.service.as_deref(), e.task_type.as_deref()));
    [service, kind].into_iter().flatten().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" · ")
}

/// «Автор · исполнители · даты» строки списка (`FoundTicketViewModel.Details`). Даты — по часам, что пришли с сервером.
pub fn details_of(f: &Found) -> String {
    let mut parts = Vec::new();
    if let Some(creator) = f.creator.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("автор: {creator}"));
    }
    if let Some(executors) = f.executors.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("исполнители: {executors}"));
    }
    if let Some(created) = f.created {
        parts.push(format!("создана {}", created.format("%d.%m.%Y")));
    }
    if let Some(changed) = f.changed {
        parts.push(format!("изменена {}", changed.format("%d.%m.%Y")));
    }
    parts.join(" · ")
}

/// Число с неразрывным пробелом между тысячами.
pub fn thousands(n: i32) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{A0}');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Общее число найденного: «1 000+», когда сервер досчитал до потолка и дальше не считал.
fn found_num(total: i32) -> String {
    if capped(total) { format!("{}+", thousands(total)) } else { thousands(total) }
}

/// Список найденного: страницы, склеенные без повторов (`SearchViewModel.LoadPageAsync`).
#[derive(Clone, Debug, Default)]
pub struct Results {
    pub rows: Vec<Found>,
    pub total: i32,
    pub has_more: bool,
    pub pages: i32,
    pub warning: String,
    loaded: usize,
    outside: usize,
}

impl Results {
    /// Добавить страницу, пришедшую по запросу.
    pub fn add_page(&mut self, page: SearchPage, query: &TaskQuery) {
        self.pages += 1;
        // список мог сдвинуться между страницами — заявка на стыке пришла бы дважды
        let mut shown: std::collections::HashSet<i32> = self.rows.iter().map(|f| f.id).collect();
        let arrived = page.found.len();
        let outside = page.found.iter().filter(|f| query.outside(f)).count();
        self.rows.extend(page.found.into_iter().filter(|f| shown.insert(f.id)));

        self.total = page.total.max(self.rows.len() as i32);
        self.has_more = arrived > 0 && (self.rows.len() as i32) < self.total && self.rows.len() < MAX_SHOWN;
        self.loaded += arrived;
        self.outside += outside;
        if self.outside > 0 {
            let (place, which) = query.outside_words();
            self.warning = format!(
                "Сервер вернул заявки {place} ({} из {} загруженных): {which} он, похоже, не применил — в списке лишнее",
                self.outside, self.loaded
            );
        }
    }

    /// Итог над списком.
    pub fn message(&self) -> String {
        let shown = self.rows.len();
        if shown == 0 {
            "Ничего не найдено".into()
        } else if self.total > shown as i32 || capped(self.total) {
            let narrow = if shown >= MAX_SHOWN { " — уточните условия" } else { "" };
            format!("Найдено: {} · показано {}{narrow}", found_num(self.total), thousands(shown as i32))
        } else {
            format!("Найдено: {}", found_num(self.total))
        }
    }
}
