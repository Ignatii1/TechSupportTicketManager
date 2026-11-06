//! `tickets.json` на диске — как `TicketStore`: запись через `.tmp` и переименование (оборванная запись не портит файл),
//! копия прежнего файла раз в день в `backups/`, их не больше 30. Писать может только хозяин доски (`single`).

use crate::board::{Board, Ticket, load_tickets, tickets_json};
use chrono::NaiveDate;
use std::path::{Path, PathBuf};

/// Сколько дневных копий держать (`BackupOncePerDay`).
pub const BACKUPS_KEPT: usize = 30;

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Store {
        Store { dir: dir.to_path_buf() }
    }

    pub fn file(&self) -> PathBuf {
        self.dir.join("tickets.json")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.dir.join("backups")
    }

    /// Карточки; файла нет — пустая доска, не прочитался — почему (файл не трогается: откладывает его TicketBoard.exe).
    pub fn load(&self) -> Result<Vec<Ticket>, String> {
        load_tickets(&self.file())
    }

    /// Копия нынешнего файла за сегодня, если её ещё нет, и лишние старые — долой (по имени: `tickets-гггг-мм-дд.json`).
    pub fn backup(&self, today: NaiveDate) -> Result<(), String> {
        let file = self.file();
        if !file.exists() {
            return Ok(());
        }
        let dir = self.backup_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("папка backups не создана: {e}"))?;
        let copy = dir.join(format!("tickets-{}.json", today.format("%Y-%m-%d")));
        if copy.exists() {
            return Ok(());
        }
        std::fs::copy(&file, &copy).map_err(|e| format!("копия tickets.json за день не сделана: {e}"))?;
        let mut old: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|e| format!("папка backups не читается: {e}"))?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("tickets-") && n.ends_with(".json"))
            .collect();
        old.sort_unstable_by(|a, b| b.cmp(a));
        for name in old.iter().skip(BACKUPS_KEPT) {
            let _ = std::fs::remove_file(dir.join(name)); // занят — удалится в другой день
        }
        Ok(())
    }

    /// Записать доску: во временный файл рядом, потом поверх `tickets.json`.
    pub fn write(&self, board: &Board) -> Result<(), String> {
        let file = self.file();
        let tmp = self.dir.join("tickets.json.tmp");
        let text = tickets_json(board)?;
        std::fs::write(&tmp, text).map_err(|e| format!("tickets.json не записан: {e}"))?;
        std::fs::rename(&tmp, &file).map_err(|e| format!("tickets.json не записан: {e}"))
    }
}
