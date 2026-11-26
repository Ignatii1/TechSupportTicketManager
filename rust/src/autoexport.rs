//! Автовыгрузка закрытых заявок без окна: таймер, файл отметки, строка трея, лог (`MainViewModel.AutoExport.cs`, карточка 4.5-3).
//! Сам заход — `export::pass::run_pass`, правила — `export::closed`. ЗАГЛУШКИ: тела заведомо неверные, проверки красные.

use crate::export::closed::ExportReach;
use crate::export::run::Options;
use crate::intraservice::{Client, Date};
use crate::settings::Settings;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Первый заход — через столько после запуска.
pub const FIRST_PASS: Duration = Duration::from_secs(15);

/// Файл отметки прототипа — рядом с exe.
pub const MARK_FILE: &str = "autoexport-rs.json";

/// Период заходов: `max(1, минуты)` минут.
pub fn next_pass_in(_minutes: i32) -> Duration {
    Duration::ZERO
}

/// Подсказка трея: заголовок и строка автовыгрузки, не длиннее 127 знаков UTF-16.
pub fn tray_tip(_state: &str) -> String {
    String::new()
}

/// Учётная запись (`AccountKey`): адрес без «/» на конце и логин, без учёта регистра.
pub fn account_key(_settings: &Settings) -> String {
    String::new()
}

/// Та же папка выгрузки (`SameFolder`).
pub fn same_folder(_a: &str, _b: &str, _data_dir: &Path) -> bool {
    false
}

/// Отметка на диске: папка как в настройках, учётная запись, охват, время, номера минуты.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedMark {
    pub folder: String,
    pub account: String,
    pub reach: ExportReach,
    pub since: Date,
    pub seen: Vec<i32>,
}

pub fn read_mark(_data_dir: &Path) -> Option<SavedMark> {
    Some(SavedMark {
        folder: String::new(),
        account: String::new(),
        reach: ExportReach::Mine,
        since: chrono::DateTime::UNIX_EPOCH.fixed_offset(),
        seen: Vec::new(),
    })
}

pub fn write_mark(_data_dir: &Path, _mark: &SavedMark) -> io::Result<()> {
    Ok(())
}

/// Состояние автовыгрузки между заходами.
pub struct AutoExport {
    _private: (),
}

impl AutoExport {
    pub fn new(_settings: &Settings, _data_dir: &Path, _journal: Option<PathBuf>) -> AutoExport {
        AutoExport { _private: () }
    }

    pub fn can_run(&self, _has_client: bool) -> bool {
        true
    }

    /// Один заход, синхронно: строка итога или None — пропущен (нельзя, зеркало отдыхает, остановили).
    pub fn pass(&mut self, _client: &Client, _now: Date, _stop: &AtomicBool, _options: &Options) -> Option<String> {
        Some(String::new())
    }
}
