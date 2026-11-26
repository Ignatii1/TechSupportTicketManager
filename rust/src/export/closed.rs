//! Правила автовыгрузки закрытых заявок и зеркала (`ClosedExport.cs`, `AppSettings`): отметка, «моя ли», охват, пропуск
//! сбойной заявки, размер страницы, отдых зеркала. Заглушки карточки 4.5-1 — тело пишет исполнитель.

use super::{AttachmentScope, Context};
use crate::intraservice::query::TaskQuery;
use crate::intraservice::{Date, Event, Found, User};
use crate::rules::ClosedNames;
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
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl ExportReach {
    pub fn from_number(_n: i32) -> ExportReach {
        todo!()
    }

    pub fn parse(_text: &str) -> Option<ExportReach> {
        todo!()
    }
}

/// Отметка автовыгрузки (`ExportMark`); `==` — `SameAs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportMark {
    pub since: Date,
    pub seen: Vec<i32>,
}

impl ExportMark {
    pub fn start(_now: Date) -> ExportMark {
        todo!()
    }

    pub fn covers(&self, _row: &Found) -> bool {
        todo!()
    }

    pub fn after(&self, _done: &[&Found]) -> ExportMark {
        todo!()
    }

    pub fn minute(_d: Date) -> Date {
        todo!()
    }
}

/// Я в заявке (`Myself`).
pub struct Myself<'a> {
    pub me: &'a User,
}

impl Myself<'_> {
    pub fn executes(&self, _row: &Found) -> bool {
        todo!()
    }

    pub fn initiated(&self, _row: &Found) -> bool {
        todo!()
    }

    pub fn observes(&self, _row: &Found) -> bool {
        todo!()
    }

    pub fn wrote(&self, _e: &Event) -> bool {
        todo!()
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
    pub fn context(&self) -> Context {
        todo!()
    }

    pub fn current_for(&self, _row: &Found) -> AttachmentScope {
        todo!()
    }

    pub fn takes_by_row(&self, _row: &Found) -> bool {
        todo!()
    }

    pub fn query(&self, _mark: &ExportMark) -> TaskQuery {
        todo!()
    }

    pub fn mine_by_row(&self, _row: &Found) -> bool {
        todo!()
    }

    pub fn mine_by_history(&self, _events: &[Event]) -> bool {
        todo!()
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
    pub fn given_up(&self, _row: &Found, _now: Date) -> bool {
        todo!()
    }

    pub fn after(&self, _failed: &[(&Found, String)], _passed: &[i32], _now: Date) -> ExportFailures {
        todo!()
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

pub fn page_size(_mark: &ExportMark) -> usize {
    todo!()
}

pub fn mark_serves(_mark_reach: Option<ExportReach>, _reach: ExportReach) -> bool {
    todo!()
}

pub fn due(_reach: ExportReach, _rest: Option<(ExportReach, Date)>, _now: Date) -> bool {
    todo!()
}

pub fn took(_t: Duration) -> String {
    todo!()
}

pub fn of_ticket(_error: &str) -> bool {
    todo!()
}

pub fn before_mark(_rows: &[Found], _mark: &ExportMark, _full: bool) -> String {
    todo!()
}

pub fn disorder(_rows: &[Found]) -> String {
    todo!()
}
