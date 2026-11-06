//! TicketBoard на Rust — прототип: размер, память, скорость. Логика — здесь, её гоняют проверки (`cargo test`); окно —
//! `ui/board.slint`; запуск, трей и глобальная клавиша — `main.rs`. Прототип ничего не пишет: `tickets.json` и
//! `settings.json` только читаются.

pub mod app;
pub mod board;
pub mod dpapi;
pub mod intraservice;
pub mod link;
pub mod metrics;
pub mod rules;
pub mod settings;

slint::include_modules!();
