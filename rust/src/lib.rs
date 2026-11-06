//! TicketBoard на Rust — прототип: размер, память, скорость. Логика — здесь, её гоняют проверки (`cargo test`); окно —
//! `ui/board.slint`; запуск, трей и глобальная клавиша — `main.rs`. Пишет только `tickets.json` (и `errors.log`), когда
//! доска его (`single`); `settings.json` и Интрасервис — только читаются.

pub mod app;
pub mod board;
pub mod capture;
pub mod clipboard;
pub mod dpapi;
pub mod hotkey;
pub mod intraservice;
pub mod journal;
pub mod link;
pub mod metrics;
pub mod rules;
pub mod settings;
pub mod single;
pub mod store;
pub mod sync;

slint::include_modules!();
