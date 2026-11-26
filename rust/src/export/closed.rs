//! Правила автовыгрузки закрытых заявок и зеркала (`ClosedExport.cs`, `AppSettings`): отметка, «моя ли», охват, пропуск
//! сбойной заявки, размер страницы, отдых зеркала.

use super::run::human_span;
use super::{AttachmentScope, Context};
use crate::intraservice::query::{CHANGED_ASC_SORT, DateSpan, TaskQuery};
use crate::intraservice::{Date, Event, Found, User, http_code, is_transient};
use crate::rules::{ClosedNames, same_name};
use chrono::{Duration as Span, Timelike};
use std::time::Duration;

/// Какие заявки автовыгрузка держит в базе знаний; Other — число, которое .NET читает, а имени у него нет.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportReach {
    #[default]
    Mine,
    Closed,
    All,
    Other(i32),
}

impl std::fmt::Display for ExportReach {
    /// Как `ToString()` у enum C#: имя, а у числа без имени — оно само.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportReach::Mine => f.write_str("Mine"),
            ExportReach::Closed => f.write_str("Closed"),
            ExportReach::All => f.write_str("All"),
            ExportReach::Other(n) => write!(f, "{n}"),
        }
    }
}

impl ExportReach {
    /// Число, как его читает .NET: 0 — Mine, 1 — Closed, 2 — All, иное — без имени.
    pub fn from_number(n: i32) -> ExportReach {
        match n {
            0 => ExportReach::Mine,
            1 => ExportReach::Closed,
            2 => ExportReach::All,
            n => ExportReach::Other(n),
        }
    }

    /// Строка настройки — как у `AttachmentScope::parse`: число или имена через запятую, сложенные как флаги .NET.
    pub fn parse(text: &str) -> Option<ExportReach> {
        let text = text.trim();
        if let Ok(n) = text.parse::<i32>() {
            return Some(Self::from_number(n));
        }
        // одна запятая в конце .NET прощает, две и в начале — нет
        let names = text.strip_suffix(',').unwrap_or(text);
        let mut bits = 0;
        for name in names.split(',') {
            bits |= match name.trim().to_ascii_lowercase().as_str() {
                "mine" => 0,
                "closed" => 1,
                "all" => 2,
                _ => return None,
            };
        }
        Some(Self::from_number(bits))
    }
}

/// Отметка автовыгрузки (`ExportMark`): закрытые, изменённые до `since`, разобраны; `seen` — номера разобранных в минуту
/// отметки (сервер сравнивает даты до минуты). `==` — `SameAs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportMark {
    pub since: Date,
    pub seen: Vec<i32>,
}

impl ExportMark {
    /// Первое включение: неделя назад, ничего не разобрано.
    pub fn start(now: Date) -> ExportMark {
        ExportMark { since: now - Span::days(7), seen: Vec::new() }
    }

    /// Строка уже разобрана: изменена раньше минуты отметки или её номер среди разобранных минуты, а изменение не позже
    /// отметки.
    pub fn covers(&self, row: &Found) -> bool {
        row.changed.is_some_and(|c| c < Self::minute(self.since) || c <= self.since && self.seen.contains(&row.id))
    }

    /// Отметка после строк, разобранных подряд по порядку списка; назад не идёт, минута та же — и прежние номера.
    pub fn after(&self, done: &[&Found]) -> ExportMark {
        let Some(last) = done.last().and_then(|r| r.changed) else { return self.clone() };
        let since = last.max(self.since);
        let minute = Self::minute(since);
        let mut seen: Vec<i32> = done.iter().filter(|r| r.changed.is_some_and(|c| c >= minute)).map(|r| r.id).collect();
        if Self::minute(self.since) == minute {
            seen.extend(&self.seen);
        }
        seen.sort_unstable();
        seen.dedup();
        ExportMark { since, seen }
    }

    /// Минута, с которой сервер вернёт заявки по этой дате: секунды и меньше — долой, пояс остаётся.
    pub fn minute(d: Date) -> Date {
        d.with_second(0).and_then(|d| d.with_nanosecond(0)).unwrap_or(d)
    }
}

/// Я в заявке (`Myself`) — по номерам, а нет их — по именам.
pub struct Myself<'a> {
    pub me: &'a User,
}

impl Myself<'_> {
    pub fn executes(&self, row: &Found) -> bool {
        self.listed(row.executor_ids.as_deref(), row.executors.as_deref())
    }

    pub fn initiated(&self, row: &Found) -> bool {
        match row.creator_id {
            Some(id) => id == self.me.id,
            None => self.named_in(row.creator.as_deref()),
        }
    }

    pub fn observes(&self, row: &Found) -> bool {
        self.listed(row.observer_ids.as_deref(), row.observers.as_deref())
    }

    pub fn wrote(&self, e: &Event) -> bool {
        match e.author_id {
            Some(id) => id == self.me.id,
            None => self.is_me(&e.author),
        }
    }

    /// Список номеров, а нет его — имена через запятую.
    fn listed(&self, ids: Option<&[i32]>, names: Option<&str>) -> bool {
        match ids {
            Some(ids) => ids.contains(&self.me.id),
            None => self.named_in(names),
        }
    }

    /// Моё имя среди перечисленных через запятую.
    fn named_in(&self, names: Option<&str>) -> bool {
        names.unwrap_or("").split(',').map(str::trim).filter(|n| !n.is_empty()).any(|n| self.is_me(n))
    }

    /// Это я по имени; моего имени сервер не прислал — ничьё имя не моё.
    fn is_me(&self, name: &str) -> bool {
        !self.me.name.trim().is_empty() && same_name(name, &self.me.name)
    }
}

/// Что выгружать автоматически (`ExportScope`).
#[derive(Clone, Debug)]
pub struct ExportScope {
    pub closed_status_ids: Vec<i32>,
    pub me: User,
    pub closed_names: ClosedNames,
    pub attachments: AttachmentScope,
    pub reach: ExportReach,
}

impl ExportScope {
    /// Для файла заявки: мои закрытые берутся своим правилом — вложения всех взятых; зеркало качает только моих.
    pub fn context(&self) -> Context {
        let attachments = match (self.reach, self.attachments) {
            (_, AttachmentScope::None) => AttachmentScope::None,
            (ExportReach::Mine, _) => AttachmentScope::All,
            _ => AttachmentScope::Mine,
        };
        Context { me: Some(self.me.clone()), closed: self.closed_names.clone(), attachments }
    }

    /// Чьи вложения должны быть в базе, чтобы файл строки считался верным (`CurrentFor`).
    pub fn current_for(&self, row: &Found) -> AttachmentScope {
        let attachments = self.context().attachments;
        if self.takes_by_row(row) || attachments == AttachmentScope::None { attachments } else { AttachmentScope::Mine }
    }

    /// Берётся по строке: у зеркала — любая; у моих — где я исполнитель, инициатор или наблюдатель.
    pub fn takes_by_row(&self, row: &Found) -> bool {
        self.reach != ExportReach::Mine || self.mine_by_row(row)
    }

    /// Закрытые (у «всех заявок» — любые), изменённые с минуты отметки, и у архивных сервисов.
    pub fn query(&self, mark: &ExportMark) -> TaskQuery {
        TaskQuery {
            status_ids: if self.reach == ExportReach::All { Vec::new() } else { self.closed_status_ids.clone() },
            // по часам ПК (`mark.Since.LocalDateTime`): отметка из даты сервера с поясом хранит его пояс
            changed: DateSpan { from: Some(mark.since.with_timezone(&chrono::Local).naive_local()), to: None },
            include_archived: true,
            sort: CHANGED_ASC_SORT.into(),
            ..TaskQuery::default()
        }
    }

    /// Исполнитель, инициатор или наблюдатель — видно по строке списка.
    pub fn mine_by_row(&self, row: &Found) -> bool {
        let i = Myself { me: &self.me };
        i.executes(row) || i.initiated(row) || i.observes(row)
    }

    /// Участник: в истории есть моё действие.
    pub fn mine_by_history(&self, events: &[Event]) -> bool {
        let i = Myself { me: &self.me };
        events.iter().any(|e| i.wrote(e))
    }
}

/// Заявка, на которой заходы спотыкаются (`ExportFailure`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportFailure {
    pub id: i32,
    pub changed: Date,
    pub times: u32,
    pub first: Date,
    pub error: String,
}

/// Сбойные заявки: номер → неудача (`ExportFailures`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportFailures {
    pub items: std::collections::BTreeMap<i32, ExportFailure>,
}

impl ExportFailures {
    /// Пропустить: на этом же изменении заходы спотыкались `GIVE_UP_TIMES` раз, и первый — не позже `GIVE_UP_AFTER` назад.
    pub fn given_up(&self, row: &Found, now: Date) -> bool {
        self.items.get(&row.id).is_some_and(|f| {
            Some(f.changed) == row.changed
                && f.times >= GIVE_UP_TIMES
                && now - f.first >= Span::from_std(GIVE_UP_AFTER).unwrap_or(Span::MAX)
        })
    }

    /// После захода: прошедшие — из счёта вон; споткнувшиеся — ещё раз (изменилась — счёт заново); прочие — как были.
    pub fn after(&self, failed: &[(&Found, String)], passed: &[i32], now: Date) -> ExportFailures {
        let mut items = self.items.clone();
        for id in passed {
            items.remove(id);
        }
        for (row, error) in failed {
            let Some(changed) = row.changed else { continue };
            let next = match self.items.get(&row.id) {
                Some(f) if f.changed == changed => ExportFailure { times: f.times + 1, error: error.clone(), ..f.clone() },
                _ => ExportFailure { id: row.id, changed, times: 1, first: now, error: error.clone() },
            };
            items.insert(row.id, next);
        }
        ExportFailures { items }
    }
}

/// Заходов подряд до пропуска (`GiveUpTimes`).
pub const GIVE_UP_TIMES: u32 = 5;
/// Не раньше, чем через столько от первого сбоя (`GiveUpAfter`).
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(3600);
/// Заявок с запросом за заход (`Budget`).
pub const BUDGET: usize = 30;
/// Больше строк на страницу сервер не даёт (`MaxPage`).
pub const MAX_PAGE: usize = 2000;
/// Строк в странице без номеров минуты отметки (`PageRows`).
pub const PAGE_ROWS: usize = 500;
/// Без минуты час (`MirrorEvery`).
pub const MIRROR_EVERY: Duration = Duration::from_secs(59 * 60);
/// Часы двух поясов расходятся не больше чем на 26 часов — с запасом двое суток (`MaxSkew`).
const MAX_SKEW: Span = Span::days(2);

/// Строк в странице: `PAGE_ROWS` и номера минуты отметки (они придут снова), не больше, чем сервер отдаёт за раз.
pub fn page_size(mark: &ExportMark) -> usize {
    (PAGE_ROWS + mark.seen.len()).min(MAX_PAGE)
}

/// Отметка годится для охвата: её охват не уже; нет охвата у отметки — «мои».
pub fn mark_serves(mark_reach: Option<ExportReach>, reach: ExportReach) -> bool {
    match mark_reach.unwrap_or(ExportReach::Mine) {
        ExportReach::All => true,
        ExportReach::Closed => matches!(reach, ExportReach::Closed | ExportReach::Mine),
        _ => reach == ExportReach::Mine,
    }
}

/// Пора заходить: мои — каждым заходом; зеркало — если не отдыхает (охват сменили — отдых не в счёт).
pub fn due(reach: ExportReach, rest: Option<(ExportReach, Date)>, now: Date) -> bool {
    reach == ExportReach::Mine || rest.is_none_or(|(r, at)| r != reach || now - at >= Span::from_std(MIRROR_EVERY).unwrap_or(Span::MAX))
}

/// Сколько длился заход — для подсказки трея: короче секунды — ничего, короче минуты — секунды, дальше — `human_span`.
pub fn took(t: Duration) -> String {
    match t.as_secs() {
        _ if t < Duration::from_secs(1) => String::new(),
        s if s < 60 => format!(" за {s} с"),
        _ => format!(" за {}", human_span(t)),
    }
}

/// Сбой из-за самой заявки — в счёт её пропуска: 500 и отказ 4xx (кроме 401, 408, 429), непонятный ответ. Сеть, сроки,
/// 502–504 и неверный логин — не её вина.
pub fn of_ticket(error: &str) -> bool {
    http_code(error) == Some(500) || !is_transient(error) && http_code(error) != Some(401)
}

/// Вся страница раньше минуты отметки, и она полная или её первая строка раньше отметки дальше сдвига поясов: сервер не
/// применил условие по дате. Пусто — в порядке.
pub fn before_mark(rows: &[Found], mark: &ExportMark, full: bool) -> String {
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else { return String::new() };
    let Some(last_at) = last.changed.filter(|&c| c < ExportMark::minute(mark.since)) else { return String::new() };
    if !(full || first.changed.is_some_and(|c| c < mark.since - MAX_SKEW)) {
        return String::new();
    }
    format!(
        "сервер вернул только заявки, изменённые раньше отметки ({}; последняя — #{}, {}): похоже, не применил условие по дате (ChangedMoreThan); отметка стоит",
        mark.since.format("%d.%m.%Y %H:%M"),
        last.id,
        last_at.format("%d.%m.%Y %H:%M")
    )
}

/// Что не так со страницей списка: строка без даты изменения или не по возрастанию. Равные — в любом порядке номеров.
pub fn disorder(rows: &[Found]) -> String {
    let mut before: Option<(&Found, Date)> = None;
    for row in rows {
        let Some(changed) = row.changed else {
            return format!("в строке заявки #{} нет даты изменения (Changed) — не понять, докуда разобрано", row.id);
        };
        if let Some((b, at)) = before
            && changed < at
        {
            return format!(
                "сервер отдал список не по возрастанию изменения (#{}, затем #{}) — отметка перескочила бы заявки",
                b.id, row.id
            );
        }
        before = Some((row, changed));
    }
    String::new()
}
