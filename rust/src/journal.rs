//! `errors.log` рядом с exe — как `App.AppendLog`: `[гггг-мм-дд чч:мм:сс] текст` и пустая строка. Пишут и окно, и потоки
//! чтения — под замком, иначе вторая запись пропала бы. ponytail: без ротации, как у C#.

use chrono::{Local, NaiveDateTime};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

pub fn append(dir: &Path, text: &str) {
    append_at(dir, text, Local::now().naive_local());
}

pub fn append_at(dir: &Path, text: &str, at: NaiveDateTime) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let line = format!("[{}] {text}\n\n", at.format("%Y-%m-%d %H:%M:%S"));
    // журнал не должен ронять приложение
    let _ =
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join("errors.log")).and_then(|mut f| f.write_all(line.as_bytes()));
}
