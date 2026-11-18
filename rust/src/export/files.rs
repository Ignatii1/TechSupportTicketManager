//! Файлы базы знаний на диске (`KnowledgeExport.Files.cs`, `.Attachments.cs`): где лежит заявка, как называется, какой файл
//! наш, запись через временный файл, оглавления. Заглушка карточки 4.2-1: тела нарочно неверны, настоящие приходят с кодом.

use crate::intraservice::{Date, Found};
use std::path::Path;

pub const TICKETS_FOLDER: &str = "tickets";
pub const INDEX_FILE: &str = "_index.md";
pub const NO_DATE_FOLDER: &str = "без даты";
pub const FILES_FOLDER: &str = "files";
pub const LFS_FOLDER: &str = "files-lfs";
pub const MAX_PATH_LENGTH: usize = 250;

pub fn file_name(_id: i32, _title: &str, _title_max: usize) -> String {
    String::new()
}

pub fn shard_of(_created: Option<&Date>) -> String {
    String::new()
}

pub fn place_of(_tickets_dir: &Path, _f: &Found) -> (String, String) {
    (String::new(), String::new())
}

pub fn read_props(_path: &Path) -> Option<Vec<(String, String)>> {
    None
}

pub fn unquote(_s: &str) -> String {
    String::new()
}

pub fn disk_name(_file_id: i32, _name: Option<&str>, _max: usize) -> String {
    String::new()
}

pub fn size(_bytes: u64) -> String {
    String::new()
}

pub fn write_atomic(_path: &Path, _text: &str) -> std::io::Result<()> {
    Ok(())
}

pub fn sweep_temps(_folder: &Path) {}

pub fn ignore_temps_in_git(_dir: &Path) -> String {
    String::new()
}

pub fn write_indexes(_dir: &Path, _shards: &[String]) -> String {
    String::new()
}
