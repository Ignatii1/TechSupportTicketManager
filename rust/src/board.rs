//! Доска: карточки `tickets.json` (тот же формат, что у TicketBoard.exe: имена свойств `Ticket`, статус и приоритет
//! словами, даты — `DateTimeOffset` строкой) и правила того, что на ней видно и как она меняется — те же, что в C#
//! (`Ticket`, `MainViewModel`, `ColumnViewModel`). Модель знает все поля C#; поля, которых не знает (файл новее), хранит
//! как были и пишет обратно.

use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate, Timelike};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::fmt;
use std::path::Path;

pub type Date = DateTime<FixedOffset>;

/// Сейчас, по часам компьютера, — с точностью .NET (100 нс): что запишем, то C# и прочтёт.
pub fn now() -> Date {
    ticks(Local::now().fixed_offset())
}

/// Дата с точностью .NET: доли секунды — до 100 нс (тика), лишнее отброшено.
pub fn ticks(d: Date) -> Date {
    d.with_nanosecond(d.nanosecond() / 100 * 100).unwrap_or(d)
}

/// Колонка доски. Имена — как в `tickets.json` (enum `TicketStatus`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Inbox,
    InProgress,
    Waiting,
    Done,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::Inbox, Status::InProgress, Status::Waiting, Status::Done];
    const NAMES: [&'static str; 4] = ["Inbox", "InProgress", "Waiting", "Done"];

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    Low,
    #[default]
    Mid,
    High,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Priority::Low, Priority::Mid, Priority::High];
    const NAMES: [&'static str; 3] = ["Low", "Mid", "High"];

    /// Как `PriorityToTextConverter.Text`.
    pub fn title(self) -> &'static str {
        match self {
            Priority::Low => "Низкий",
            Priority::Mid => "Средний",
            Priority::High => "Высокий",
        }
    }
}

/// Значение enum так, как его читает `JsonStringEnumConverter`: имя без учёта регистра, число — строкой или числом
/// (номер в объявлении); null — по умолчанию. Вне списка — ошибка: C# такой файл тоже не прочтёт.
struct EnumIndex(&'static [&'static str]);

impl<'de> Visitor<'de> for EnumIndex {
    type Value = Option<usize>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "одно из {:?} или его номер", self.0)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<usize>, E> {
        let t = v.trim();
        let i = if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
            t.parse::<usize>().ok()
        } else {
            self.0.iter().position(|n| n.eq_ignore_ascii_case(v))
        };
        i.filter(|&i| i < self.0.len()).map(Some).ok_or_else(|| E::custom(format!("«{v}» — не {:?}", self.0)))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<usize>, E> {
        usize::try_from(v).ok().filter(|&i| i < self.0.len()).map(Some).ok_or_else(|| E::custom(format!("{v} — не {:?}", self.0)))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<usize>, E> {
        u64::try_from(v).map_err(|_| E::custom(format!("{v} — не {:?}", self.0))).and_then(|v| self.visit_u64(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Option<usize>, E> {
        Ok(None)
    }
}

impl<'de> Deserialize<'de> for Status {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Status, D::Error> {
        Ok(d.deserialize_any(EnumIndex(&Status::NAMES))?.and_then(Status::from_index).unwrap_or_default())
    }
}

impl Serialize for Status {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(Status::NAMES[self.index()])
    }
}

impl<'de> Deserialize<'de> for Priority {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Priority, D::Error> {
        Ok(d.deserialize_any(EnumIndex(&Priority::NAMES))?.and_then(|i| Priority::ALL.get(i).copied()).unwrap_or_default())
    }
}

impl Serialize for Priority {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(Priority::NAMES[*self as usize])
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

/// Даты — как `System.Text.Json` у `DateTimeOffset`: читается ISO с поясом и без (без — местное время, как .NET);
/// пишется `гггг-мм-ддTчч:мм:сс`, доли секунды — до 7 знаков без хвостовых нулей, пояс `+03:00`.
pub mod date {
    use super::Date;
    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn text(d: &Date) -> String {
        let ticks = d.timestamp_subsec_nanos().min(999_999_999) / 100;
        let fraction = if ticks == 0 { String::new() } else { format!(".{:07}", ticks).trim_end_matches('0').to_string() };
        format!("{}{fraction}{}", d.format("%Y-%m-%dT%H:%M:%S"), d.format("%:z"))
    }

    pub fn parse(s: &str) -> Option<Date> {
        crate::intraservice::parse::date(s)
    }

    pub fn serialize<S: Serializer>(d: &Date, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&text(d))
    }

    /// null — «сейчас», как значение по умолчанию у C# (`= DateTimeOffset.Now`).
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Date, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(super::now()),
            Some(s) => parse(&s).ok_or_else(|| de::Error::custom(format!("«{s}» — не дата"))),
        }
    }

    pub mod option {
        use super::super::Date;
        use serde::{Deserialize, Deserializer, Serializer, de};

        pub fn serialize<S: Serializer>(d: &Option<Date>, s: S) -> Result<S::Ok, S::Error> {
            match d {
                Some(d) => s.serialize_str(&super::text(d)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Date>, D::Error> {
            match Option::<String>::deserialize(d)? {
                None => Ok(None),
                Some(s) => super::parse(&s).map(Some).ok_or_else(|| de::Error::custom(format!("«{s}» — не дата"))),
            }
        }
    }
}

/// Новый `Guid` в виде C# (`D`: 8-4-4-4-12, строчные). Случайность — из `RandomState` (ключи от системы, свои у каждого
/// потока) и счётчика: номер карточки не секрет, ему нужна только уникальность.
pub fn new_guid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let state = std::collections::hash_map::RandomState::new();
    let part = |salt: u64| {
        let mut h = state.build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_u64(salt);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.finish()
    };
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&part(1).to_le_bytes());
    b[8..].copy_from_slice(&part(2).to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40; // версия 4
    b[8] = (b[8] & 0x3f) | 0x80; // вариант RFC 4122
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// `Guid` вида `D`, который прочтёт `System.Text.Json` (8-4-4-4-12, шестнадцатеричные цифры).
pub fn is_guid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

/// Номер карточки или заметки: не `Guid` (правили руками, null, нет поля) — новый, иначе C# файл не прочтёт.
fn guid<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.and_then(|v| v.as_str().filter(|s| is_guid(s)).map(String::from)).unwrap_or_else(new_guid))
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Note {
    #[serde(default = "new_guid", deserialize_with = "guid")]
    pub id: String,
    #[serde(default = "now", with = "date")]
    pub created_at: Date,
    #[serde(default, deserialize_with = "nullable")]
    pub text: String,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TicketFile {
    #[serde(default, deserialize_with = "nullable")]
    pub id: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// Карточка — все поля C# `Ticket` в том порядке, в каком их пишет `System.Text.Json` (сверено с `TicketStore.Save`);
/// `null` не пишется
/// (`WhenWritingNull`), строки, приоритет, статус, даты в колонке, счётчик и заметки — всегда. `other` — поля, которых
/// модель не знает: читаются и пишутся как были.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Ticket {
    #[serde(default = "new_guid", deserialize_with = "guid")]
    pub id: String,
    #[serde(default = "now", with = "date")]
    pub created_at: Date,
    /// Третьей, как у C#: `Notes` — свойство, написанное руками, а они идут раньше сгенерированных `[ObservableProperty]`.
    #[serde(default, deserialize_with = "nullable")]
    pub notes: Vec<Note>,
    #[serde(default, deserialize_with = "nullable")]
    pub title: String,
    #[serde(default, deserialize_with = "nullable")]
    pub url: String,
    #[serde(default, deserialize_with = "nullable")]
    pub description: String,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub status: Status,
    #[serde(default = "now", with = "date")]
    pub status_changed_at: Date,
    #[serde(default, with = "date::option", skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intraservice_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_status: Option<String>,
    #[serde(default, with = "date::option", skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept_open_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executors: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    /// None — список вложений ещё не читали; `[]` — прочитан, вложений нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<TicketFile>>,
    #[serde(default, with = "date::option", skip_serializing_if = "Option::is_none")]
    pub comments_checked_for: Option<Date>,
    #[serde(default, with = "date::option", skip_serializing_if = "Option::is_none")]
    pub comments_seen_at: Option<Date>,
    #[serde(default, deserialize_with = "nullable")]
    pub unread_comments: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to_me: Option<bool>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// Пороги из настроек: просрочка, «скрыть «Готово» старше», лимит «В работе».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub overdue_days: i64,
    pub hide_done_days: i64,
    pub wip_limit: i64,
}

impl Ticket {
    /// Новая карточка, как `new Ticket()` у C#: свой `Guid`, добавлена и в колонке — сейчас, «Входящие», средний.
    pub fn new(now: Date) -> Ticket {
        Ticket {
            id: new_guid(),
            created_at: now,
            title: String::new(),
            url: String::new(),
            description: String::new(),
            priority: Priority::Mid,
            status: Status::Inbox,
            status_changed_at: now,
            completed_at: None,
            intraservice_id: None,
            external_status: None,
            last_sync_at: None,
            kept_open_status: None,
            creator: None,
            creator_phone: None,
            creator_email: None,
            executors: None,
            executor_group: None,
            creator_position: None,
            creator_company: None,
            service: None,
            task_type: None,
            categories: None,
            assets: None,
            files: None,
            comments_checked_for: None,
            comments_seen_at: None,
            unread_comments: 0,
            assigned_to_me: None,
            notes: Vec::new(),
            other: Map::new(),
        }
    }

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
        (today - self.status_changed_at.date_naive()).num_days().max(0)
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
    pub fn hidden(&self, now: Date, rules: Rules) -> bool {
        self.status == Status::Done && self.completed_at.is_some_and(|done| done < now - Duration::days(rules.hide_done_days))
    }

    /// Строка под названием: сервис · организация инициатора; пустое — пропускается.
    pub fn context(&self) -> String {
        joined(&[&self.service, &self.creator_company])
    }

    /// Должность · организация инициатора (`CreatorRole`).
    pub fn creator_role(&self) -> String {
        joined(&[&self.creator_position, &self.creator_company])
    }

    /// Телефон · почта инициатора (`CreatorContacts`).
    pub fn creator_contacts(&self) -> String {
        joined(&[&self.creator_phone, &self.creator_email])
    }

    /// Сервис · тип (`ServiceText`).
    pub fn service_text(&self) -> String {
        joined(&[&self.service, &self.task_type])
    }

    /// Как `Ticket.MoveTo`: дата смены колонки — сейчас, «выполнена» — только в «Готово».
    pub fn move_to(&mut self, status: Status, now: Date) {
        if self.status == status {
            return;
        }
        self.status = status;
        self.status_changed_at = now;
        self.completed_at = (status == Status::Done).then_some(now);
    }

    /// Заметка сверху, как `AddNote`: пустая (одни пробелы) — не добавляется.
    pub fn add_note(&mut self, text: &str, now: Date) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        self.notes.insert(0, Note { id: new_guid(), created_at: now, text: text.into(), other: Map::new() });
        true
    }

    /// Удалить заметку (`DeleteNote`, без вопроса: откат — ежедневная копия файла).
    pub fn delete_note(&mut self, id: &str) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }
}

fn joined(parts: &[&Option<String>]) -> String {
    parts.iter().filter_map(|p| p.as_deref()).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// Вопрос перед удалением карточки (`DeleteSelected`): с номером — что в Интрасервисе она останется.
pub fn delete_question(t: &Ticket) -> String {
    match t.intraservice_id {
        None => format!("«{}»", t.title),
        Some(_) => format!("{}  «{}»\n\nВ Интрасервисе она останется.", t.number(), t.title),
    }
}

/// Карточки из текста `tickets.json`. Номер (`Id`) у каждой свой, у заметки карточки — тоже: повтор (правка руками)
/// получает новый — по номеру карточку выбирают и переносят, заметку удаляют.
pub fn parse_tickets(json: &str) -> Result<Vec<Ticket>, String> {
    let mut tickets: Vec<Ticket> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    for t in tickets.iter_mut() {
        unique(&mut t.id, &mut seen);
        let mut notes = std::collections::HashSet::new();
        for n in t.notes.iter_mut() {
            unique(&mut n.id, &mut notes);
        }
    }
    Ok(tickets)
}

/// Номер уже встречался — новый.
fn unique(id: &mut String, seen: &mut std::collections::HashSet<String>) {
    if !seen.insert(id.to_ascii_lowercase()) {
        *id = new_guid();
        seen.insert(id.clone());
    }
}

/// Карточки из файла; нет файла — пустая доска (как `TicketStore.Load`); не прочитался — ошибка текстом. Текст — как
/// `File.ReadAllText`: BOM (UTF-8, UTF-16, UTF-32) понимается и снимается (файл правят Блокнотом), битые байты — U+FFFD.
pub fn load_tickets(path: &Path) -> Result<Vec<Ticket>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            parse_tickets(&crate::intraservice::charset::decode(&bytes, None)).map_err(|e| format!("tickets.json не прочитан: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("tickets.json не прочитан: {e}")),
    }
}

/// Текст `tickets.json`: карточки по колонкам сверху вниз (`AllTickets`), отступ — два пробела, как у `WriteIndented`.
/// Не вышло — ошибка, а не пустой список: пустой затёр бы доску.
pub fn tickets_json(board: &Board) -> Result<String, String> {
    serde_json::to_string_pretty(&board.all().collect::<Vec<_>>()).map_err(|e| format!("tickets.json не собран: {e}"))
}

/// Карточки по колонкам, сверху вниз — в порядке файла; перенесённая и новая встают наверх, как `MoveTicket` и `Insert`.
#[derive(Clone, Debug, Default)]
pub struct Board {
    columns: [Vec<Ticket>; 4],
}

impl Board {
    pub fn new(tickets: Vec<Ticket>) -> Board {
        let mut board = Board::default();
        for t in tickets {
            board.columns[t.status.index()].push(t);
        }
        board
    }

    /// Все карточки в порядке записи: колонки слева направо, в колонке — сверху вниз.
    pub fn all(&self) -> impl Iterator<Item = &Ticket> {
        self.columns.iter().flatten()
    }

    pub fn len(&self) -> usize {
        self.columns.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Видимые карточки колонки, сверху вниз.
    pub fn visible(&self, status: Status, now: Date, rules: Rules) -> Vec<&Ticket> {
        self.columns[status.index()].iter().filter(|t| !t.hidden(now, rules)).collect()
    }

    pub fn find(&self, key: &str) -> Option<&Ticket> {
        self.all().find(|t| t.id == key)
    }

    pub fn find_mut(&mut self, key: &str) -> Option<&mut Ticket> {
        self.columns.iter_mut().flatten().find(|t| t.id == key)
    }

    /// Перенести карточку в колонку (наверх). false — нет такой или она уже там.
    pub fn move_card(&mut self, key: &str, to: Status, now: Date) -> bool {
        let Some(from) = self.find(key).map(|t| t.status) else { return false };
        if from == to {
            return false;
        }
        let i = self.columns[from.index()].iter().position(|t| t.id == key).expect("карточка в своей колонке");
        let mut t = self.columns[from.index()].remove(i);
        t.move_to(to, now);
        self.columns[to.index()].insert(0, t);
        true
    }

    /// ←/→ (`MoveSelected`): на delta колонок, у края — дальше некуда.
    pub fn shift(&mut self, key: &str, delta: i32, now: Date) -> bool {
        let Some(from) = self.find(key).map(|t| t.status.index() as i32) else { return false };
        let to = (from + delta).clamp(0, Status::ALL.len() as i32 - 1);
        to != from && self.move_card(key, Status::ALL[to as usize], now)
    }

    /// Новая карточка — во «Входящие» сверху (`Insert`).
    pub fn insert(&mut self, mut t: Ticket) {
        t.status = Status::Inbox;
        self.columns[Status::Inbox.index()].insert(0, t);
    }

    /// Новая карточка — в конец «Входящих» (импорт: `Items.Add`, пачкой, в порядке списка).
    pub fn push_inbox(&mut self, mut t: Ticket) {
        t.status = Status::Inbox;
        self.columns[Status::Inbox.index()].push(t);
    }

    /// Убрать с доски (`DeleteSelected`).
    pub fn remove(&mut self, key: &str) -> Option<Ticket> {
        let column = self.columns.iter_mut().find(|c| c.iter().any(|t| t.id == key))?;
        let i = column.iter().position(|t| t.id == key)?;
        Some(column.remove(i))
    }

    /// Номера Интрасервиса на доске.
    pub fn numbers(&self) -> std::collections::HashSet<i32> {
        self.all().filter_map(|t| t.intraservice_id).collect()
    }

    /// Перегруз — только у «В работе», видимых больше лимита (`ColumnViewModel.IsOverloaded`).
    pub fn overloaded(&self, status: Status, now: Date, rules: Rules) -> bool {
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
