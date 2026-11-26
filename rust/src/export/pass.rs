//! Один заход автовыгрузки закрытых заявок (`ClosedExport.RunAsync`): страница списка после отметки, строки по порядку,
//! не больше `budget` заявок с запросом, взятые — тем же движком, что и окно поиска; отметка — за разобранными подряд.

use super::closed::{ExportFailure, ExportFailures, ExportMark, ExportScope};
use super::run::Options;
use crate::intraservice::{Client, Date};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Итог захода (`ClosedExportPass`).
#[derive(Clone, Debug)]
pub struct ClosedExportPass {
    pub next: ExportMark,
    pub not_mine: BTreeMap<i32, Date>,
    pub failures: ExportFailures,
    pub exported: usize,
    pub more: bool,
    pub error: String,
    pub given_up: Vec<ExportFailure>,
    pub waited: Duration,
}

/// Заход (`RunAsync`).
#[allow(clippy::too_many_arguments)]
pub fn run_pass(
    _client: &Client,
    _scope: &ExportScope,
    mark: &ExportMark,
    not_mine: &BTreeMap<i32, Date>,
    failures: &ExportFailures,
    _dir: &Path,
    _ticket_url: &(dyn Fn(i32) -> String + Sync),
    _budget: usize,
    _now: Date,
    _stop: &AtomicBool,
    _options: &Options,
) -> ClosedExportPass {
    // заглушка: ничего не делает, отметка стоит
    ClosedExportPass {
        next: mark.clone(),
        not_mine: not_mine.clone(),
        failures: failures.clone(),
        exported: 0,
        more: false,
        error: "заглушка: заход не сделан".into(),
        given_up: Vec::new(),
        waited: Duration::ZERO,
    }
}
