//! Быстрое добавление (`QuickCaptureViewModel` + `MainViewModel.AddFromCapture`): что в поле — ссылка, номер, подсказка,
//! название по номеру из Интрасервиса; и карточка из введённого. Ждать и спрашивать сервер — дело окна (`app.rs`): здесь
//! — что спросить и какой ответ ещё в счёт.

use crate::board::{Date, Priority, Ticket};
use crate::intraservice::{auto_title, brief};
use crate::link::LinkParser;
use crate::settings::Settings;
use std::time::Duration;

/// Подсказка под пустым полем.
pub const EMPTY_HINT: &str = "Ссылка вида …/Task/View/702180 или просто номер заявки";
/// …и под заполненным.
pub const TYPED_HINT: &str = "Будет создана заявка с этим названием";
pub const LOOKING: &str = "Ищу в Интрасервисе…";
/// Пауза после ввода перед запросом названия: номер ещё набирают.
pub const PAUSE: Duration = Duration::from_millis(400);

/// Спросить название заявки n; ответ в счёт, только если `generation` ещё та же.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lookup {
    pub id: i32,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct Capture {
    pub text: String,
    pub priority: Priority,
    pub number: Option<i32>,
    pub hint: String,
    /// Название из Интрасервиса, «Ищу в Интрасервисе…» или ошибка одной строкой.
    pub preview: String,
    has_url: bool,
    lookup_id: Option<i32>,
    generation: u64,
}

impl Default for Capture {
    fn default() -> Self {
        Capture {
            text: String::new(),
            priority: Priority::Mid,
            number: None,
            hint: EMPTY_HINT.into(),
            preview: String::new(),
            has_url: false,
            lookup_id: None,
            generation: 0,
        }
    }
}

impl Capture {
    /// Цифры 1/2/3 меняют приоритет, только когда в поле распознанная ссылка: с пустого поля начинают набирать номер.
    pub fn digits_set_priority(&self) -> bool {
        self.has_url
    }

    pub fn number_text(&self) -> String {
        self.number.map(|n| format!("#{n}")).unwrap_or_default()
    }

    /// Текст в поле сменился. Номер другой — прежний запрос не в счёт; есть API — вернуть, что спросить после паузы.
    pub fn set_text(&mut self, text: &str, parser: &LinkParser, api: bool) -> Option<Lookup> {
        self.text = text.to_string();
        let link = parser.parse(text);
        self.has_url = link.has_url();
        self.number = link.id;
        self.hint = if text.is_empty() { EMPTY_HINT } else { TYPED_HINT }.into();
        if link.id == self.lookup_id {
            return None;
        }
        self.generation += 1;
        self.lookup_id = link.id;
        self.preview.clear();
        let id = link.id.filter(|_| api)?;
        self.preview = LOOKING.into();
        Some(Lookup { id, generation: self.generation })
    }

    /// Ответ сервера; на прежний ввод — мимо. Ошибка — одной строкой (`Brief`): сырой ответ раздул бы окно.
    pub fn answer(&mut self, generation: u64, result: Result<String, String>) {
        if generation == self.generation {
            self.preview = match result {
                Ok(title) => title,
                Err(e) => brief(&e),
            };
        }
    }

    /// Ещё в счёт ли этот запрос (окно проверяет перед тем, как спросить).
    pub fn current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// Окно открыли заново: пусто, «средний».
    pub fn reset(&mut self, parser: &LinkParser) {
        self.set_text("", parser, false);
        self.priority = Priority::Mid;
    }

    /// Настройки применены (другой сервер): тот же номер спросим заново.
    pub fn forget_lookup(&mut self) {
        self.lookup_id = None;
    }
}

/// Карточка из окна быстрого добавления: ссылка и/или текст + приоритет. Номер и ссылка — из ссылки; ввели только номер
/// — ссылка из адреса Интрасервиса; название — что осталось от текста, а если там только номер — «Заявка #N».
pub fn add_from_capture(input: &str, priority: Priority, parser: &LinkParser, settings: &Settings, now: Date) -> Ticket {
    let mut t = Ticket::new(now);
    t.priority = priority;
    let link = parser.parse(input);
    t.url = if link.has_url() { link.url.clone() } else { link.id.map(|n| settings.ticket_url(n)).unwrap_or_default() };
    t.intraservice_id = link.id;
    let rest = if link.has_url() { input.replace(&link.url, "") } else { input.to_string() };
    let rest = rest.trim().trim_matches(['—', '-', '·', ' ']);
    let just_number = link.id.is_some_and(|n| rest.trim_start_matches(['#', '№', ' ']) == n.to_string());
    t.title = match link.id {
        _ if !rest.is_empty() && !just_number => rest.to_string(),
        Some(n) => auto_title(n),
        None => "Заявка".into(),
    };
    t
}
