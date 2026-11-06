//! Доска: карточки из `tickets.json` (тот же формат, что пишет TicketBoard.exe: имена свойств `Ticket`, статус и
//! приоритет словами, даты — `DateTimeOffset` строкой) и правила того, что на ней видно — те же, что в C# (`Ticket`,
//! `MainViewModel.Matches`, `ColumnViewModel`).

use chrono::{DateTime, Duration, FixedOffset, NaiveDate};
use serde::{Deserialize, Deserializer};
use std::path::Path;

/// Колонка доски. Имена — как в `tickets.json` (enum `TicketStatus`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Status {
    #[default]
    Inbox,
    InProgress,
    Waiting,
    Done,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::Inbox, Status::InProgress, Status::Waiting, Status::Done];

    /// Название колонки — как `TicketStatusText.Of`.
    pub fn title(self) -> &'static str {
        match self {
            Status::Inbox => "Входящие",
            Status::InProgress => "В работе",
            Status::Waiting => "Ждёт ответа",
            Status::Done => "Готово",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> Option<Status> {
        Status::ALL.get(i).copied()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub enum Priority {
    Low,
    #[default]
    Mid,
    High,
}

impl Priority {
    /// Как `PriorityToTextConverter.Text`.
    pub fn title(self) -> &'static str {
        match self {
            Priority::Low => "Низкий",
            Priority::Mid => "Средний",
            Priority::High => "Высокий",
        }
    }
}

/// Цвет подписи возраста: как `AgeState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgeState {
    Neutral,
    Warn,
    Overdue,
}

/// `null` в файле — как отсутствие поля: значение по умолчанию (C# так и читает строки и списки).
fn nullable<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Note {
    #[serde(deserialize_with = "nullable")]
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TicketFile {
    pub id: i64,
    pub name: Option<String>,
}

/// Карточка: только поля, которые прототип показывает; остальные поля файла пропускаются (прототип файл не пишет —
/// потерять их некому).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Ticket {
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    #[serde(deserialize_with = "nullable")]
    pub title: String,
    #[serde(deserialize_with = "nullable")]
    pub url: String,
    #[serde(deserialize_with = "nullable")]
    pub description: String,
    #[serde(deserialize_with = "nullable")]
    pub status: Status,
    #[serde(deserialize_with = "nullable")]
    pub priority: Priority,
    pub status_changed_at: Option<DateTime<FixedOffset>>,
    pub completed_at: Option<DateTime<FixedOffset>>,
    pub intraservice_id: Option<i64>,
    pub external_status: Option<String>,
    pub creator: Option<String>,
    pub service: Option<String>,
    pub creator_company: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub unread_comments: i32,
    #[serde(deserialize_with = "nullable")]
    pub notes: Vec<Note>,
    #[serde(deserialize_with = "nullable")]
    pub files: Vec<TicketFile>,
}

/// Пороги из настроек: просрочка, «скрыть «Готово» старше», лимит «В работе».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub overdue_days: i64,
    pub hide_done_days: i64,
    pub wip_limit: i64,
}

impl Ticket {
    /// Как `DisplayNumber`.
    pub fn number(&self) -> String {
        match self.intraservice_id {
            Some(n) => format!("#{n}"),
            None => "без номера".into(),
        }
    }

    /// Дней в колонке — по датам: сегодняшняя минус дата смены колонки в её же смещении, не меньше нуля
    /// (`DaysInStatus`).
    pub fn days_in_status(&self, today: NaiveDate) -> i64 {
        self.status_changed_at.map_or(0, |d| (today - d.date_naive()).num_days().max(0))
    }

    /// Как `AgeState`: «Готово» не подсвечивается; с просрочки — красный, за день до неё — жёлтый.
    pub fn age_state(&self, today: NaiveDate, rules: Rules) -> AgeState {
        let days = self.days_in_status(today);
        if self.status == Status::Done {
            AgeState::Neutral
        } else if days >= rules.overdue_days {
            AgeState::Overdue
        } else if days >= rules.overdue_days - 1 {
            AgeState::Warn
        } else {
            AgeState::Neutral
        }
    }

    /// Как `AgeLabel`.
    pub fn age_label(&self, today: NaiveDate) -> String {
        match self.days_in_status(today) {
            0 => "сегодня".into(),
            d => format!("{d} д"),
        }
    }

    /// Строка в подробностях — как `AgeText`: «с сегодня» / «2 дня» / «5 дней — просрочена».
    pub fn age_text(&self, today: NaiveDate, rules: Rules) -> String {
        match self.days_in_status(today) {
            0 => "с сегодня".into(),
            d => {
                let overdue = if self.age_state(today, rules) == AgeState::Overdue { " — просрочена" } else { "" };
                format!("{d} {}{overdue}", plural(d, "день", "дня", "дней"))
            }
        }
    }

    /// Скрыта ли с доски: «Готово», выполненная раньше, чем `hide_done_days` дней назад (`Matches`, «скрыть старые»
    /// включено, как по умолчанию у доски).
    pub fn hidden(&self, now: DateTime<FixedOffset>, rules: Rules) -> bool {
        self.status == Status::Done && self.completed_at.is_some_and(|done| done < now - Duration::days(rules.hide_done_days))
    }

    /// Строка под названием: сервис · организация инициатора; пустое — пропускается.
    pub fn context(&self) -> String {
        [&self.service, &self.creator_company]
            .into_iter()
            .flatten()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Как `Ticket.MoveTo`: дата смены колонки — сейчас, «выполнена» — только в «Готово».
    pub fn move_to(&mut self, status: Status, now: DateTime<FixedOffset>) {
        if self.status == status {
            return;
        }
        self.status = status;
        self.status_changed_at = Some(now);
        self.completed_at = (status == Status::Done).then_some(now);
    }
}

/// Карточки из текста `tickets.json`. Ключ карточки — её `Id`; без `Id` или с повтором (правка руками) — свой,
/// `строка-N`: по ключу карточку выбирают и переносят, и он у каждой один.
pub fn parse_tickets(json: &str) -> Result<Vec<Ticket>, String> {
    let mut tickets: Vec<Ticket> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    for (i, t) in tickets.iter_mut().enumerate() {
        if t.id.trim().is_empty() || !seen.insert(t.id.clone()) {
            t.id = format!("строка-{}", i + 1);
            seen.insert(t.id.clone());
        }
    }
    Ok(tickets)
}

/// Карточки из файла; нет файла — пустая доска (как `TicketStore.Load`); не прочитался — ошибка текстом.
pub fn load_tickets(path: &Path) -> Result<Vec<Ticket>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_tickets(&text).map_err(|e| format!("tickets.json не прочитан: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("tickets.json не прочитан: {e}")),
    }
}

/// Карточки по колонкам — в порядке файла; перенесённая встаёт наверх, как `MoveTicket`.
pub struct Board {
    pub tickets: Vec<Ticket>,
    columns: [Vec<usize>; 4],
}

impl Board {
    pub fn new(tickets: Vec<Ticket>) -> Board {
        let mut columns: [Vec<usize>; 4] = Default::default();
        for (i, t) in tickets.iter().enumerate() {
            columns[t.status.index()].push(i);
        }
        Board { tickets, columns }
    }

    /// Видимые карточки колонки, сверху вниз.
    pub fn visible(&self, status: Status, now: DateTime<FixedOffset>, rules: Rules) -> Vec<&Ticket> {
        self.columns[status.index()].iter().map(|&i| &self.tickets[i]).filter(|t| !t.hidden(now, rules)).collect()
    }

    pub fn find(&self, key: &str) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.id == key)
    }

    /// Перенести карточку в колонку (наверх). false — нет такой или она уже там.
    pub fn move_card(&mut self, key: &str, to: Status, now: DateTime<FixedOffset>) -> bool {
        let Some(i) = self.tickets.iter().position(|t| t.id == key) else { return false };
        let from = self.tickets[i].status;
        if from == to {
            return false;
        }
        self.columns[from.index()].retain(|&j| j != i);
        self.columns[to.index()].insert(0, i);
        self.tickets[i].move_to(to, now);
        true
    }

    /// Перегруз — только у «В работе», видимых больше лимита (`ColumnViewModel.IsOverloaded`).
    pub fn overloaded(&self, status: Status, now: DateTime<FixedOffset>, rules: Rules) -> bool {
        status == Status::InProgress && self.visible(status, now, rules).len() as i64 > rules.wip_limit
    }

    /// Подпись справа в заголовке колонки — как у доски.
    pub fn hint(status: Status, rules: Rules) -> String {
        match status {
            Status::InProgress => format!("лимит {}", rules.wip_limit),
            Status::Done => format!("скрыты старше {} д", rules.hide_done_days),
            _ => String::new(),
        }
    }
}

/// Форма слова при числе — как `Ticket.Plural`: 1 день, 2 дня, 5 дней, 11 дней, 21 день.
pub fn plural(n: i64, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=19) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    }
}
