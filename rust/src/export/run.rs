//! Выгрузка выбранных заявок файлами (`KnowledgeExport.ExportRowsAsync`, `Job`, `Conclude`; `SearchViewModel.Summary`).
//! Заглушка карточки 4.2-2: тела нарочно неверны, настоящие приходят с кодом.

use crate::export::Context;
use crate::intraservice::{Client, Found};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const STARTING: &str = "Начинаю…";

/// Паузы перед повторами сбойного запроса и сколько ждать, прежде чем проверять, что записанные файлы на месте.
#[derive(Clone, Debug)]
pub struct Options {
    pub retry_delays: Vec<Duration>,
    pub vanish_wait: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options { retry_delays: Vec::new(), vanish_wait: Duration::ZERO }
    }
}

/// Итог выгрузки (`ExportResult`).
#[derive(Clone, Debug, Default)]
pub struct ExportResult {
    pub found: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub first_error: String,
    pub error: String,
    pub complete: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn export_rows(
    _client: &Client,
    _rows: &[Found],
    _dir: &Path,
    _ticket_url: &(dyn Fn(i32) -> String + Sync),
    _context: &Context,
    _progress: &(dyn Fn(String) + Sync),
    _stop: &AtomicBool,
    _options: &Options,
) -> ExportResult {
    ExportResult::default()
}

pub fn summary(_r: &ExportResult, _what: &str) -> String {
    String::new()
}
