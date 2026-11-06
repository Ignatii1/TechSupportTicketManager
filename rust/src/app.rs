//! Окно и данные: доска (`board::Board`) → модели Slint; выбор, перенос, чтение заявки из Интрасервиса, цифры.
//! Перенос меняет только память: файлы не пишутся.

use crate::board::{AgeState, Board, Priority, Status, Ticket, load_tickets, plural};
use crate::settings::{Settings, load_settings};
use crate::{CardData, ColumnData, MainWindow, dpapi, intraservice, metrics};
use chrono::{DateTime, FixedOffset, Local};
use slint::{ComponentHandle, DataTransfer, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Сколько ждать ответа Интрасервиса — как обычный запрос C#-клиента.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

struct State {
    board: Board,
    settings: Settings,
    /// «42 карточки за 3 мс» или почему доски нет.
    loaded: String,
    /// Окно на экране через столько после создания процесса (ставит `mark_shown`).
    shown_after: Option<Duration>,
    /// Размер exe — один раз: пока процесс жив, он не меняется.
    exe_bytes: Option<u64>,
    /// День, на который посчитан возраст карточек: сменился — пересчитать (`tick_at`).
    refreshed_on: std::cell::Cell<chrono::NaiveDate>,
    cards: [Rc<VecModel<CardData>>; 4],
    columns: Rc<VecModel<ColumnData>>,
}

pub struct App {
    pub window: MainWindow,
    state: Rc<RefCell<State>>,
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

fn card_data(t: &Ticket, today: chrono::NaiveDate, settings: &Settings) -> CardData {
    CardData {
        key: t.id.as_str().into(),
        number: t.number().into(),
        title: (if t.title.trim().is_empty() { "без названия" } else { t.title.as_str() }).into(),
        context: t.context().into(),
        priority: t.priority.title().into(),
        priority_level: match t.priority {
            Priority::Low => 0,
            Priority::Mid => 1,
            Priority::High => 2,
        },
        age: t.age_label(today).into(),
        age_state: match t.age_state(today, settings.rules()) {
            AgeState::Neutral => 0,
            AgeState::Warn => 1,
            AgeState::Overdue => 2,
        },
        unread: t.unread_comments,
        files: t.files.len() as i32,
        notes: t.notes.len() as i32,
    }
}

/// Строка шапки: откуда данные и что прототип ничего не пишет; не прочитались настройки — сначала это.
fn notice(dir: &Path, problem: &str) -> String {
    let base =
        format!("Прототип: читает tickets.json и settings.json из {} и ничего не пишет — переносы держатся до выхода.", dir.display());
    if problem.is_empty() { base } else { format!("{problem}. {base}") }
}

/// Полупрозрачная «карточка» под курсором, пока карточку тащат, — цветом акцента темы.
fn drag_image((cr, cg, cb): (u8, u8, u8)) -> Image {
    let (w, h, r) = (200u32, 56u32, 8i64);
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    for (i, px) in buf.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = ((i as u32 % w) as i64, (i as u32 / w) as i64);
        // скруглённые углы: точка вне круга угла — прозрачная
        let cx = if x < r {
            r
        } else if x >= w as i64 - r {
            w as i64 - r - 1
        } else {
            x
        };
        let cy = if y < r {
            r
        } else if y >= h as i64 - r {
            h as i64 - r - 1
        } else {
            y
        };
        let inside = (x - cx).pow(2) + (y - cy).pow(2) <= r * r;
        let border =
            inside && (x < 2 || y < 2 || x >= w as i64 - 2 || y >= h as i64 - 2 || (x - cx).pow(2) + (y - cy).pow(2) > (r - 2).pow(2));
        *px = match (inside, border) {
            (false, _) => Rgba8Pixel { r: 0, g: 0, b: 0, a: 0 },
            (true, true) => Rgba8Pixel { r: cr, g: cg, b: cb, a: 0xff },
            (true, false) => Rgba8Pixel { r: cr, g: cg, b: cb, a: 0x40 },
        };
    }
    Image::from_rgba8(buf)
}

impl App {
    /// Доска из `tickets.json` и `settings.json` этой папки (TicketBoard.exe держит их рядом с собой).
    pub fn open(dir: &Path) -> Result<App, slint::PlatformError> {
        let window = MainWindow::new()?;
        let started = Instant::now();
        let (board, loaded) = match load_tickets(&dir.join("tickets.json")) {
            Ok(tickets) => {
                let n = tickets.len();
                (
                    Board::new(tickets),
                    format!(
                        "tickets.json: {n} {} за {} мс",
                        plural(n as i64, "карточка", "карточки", "карточек"),
                        started.elapsed().as_millis()
                    ),
                )
            }
            Err(e) => (Board::new(Vec::new()), e),
        };
        let (settings, problem) = load_settings(dir);
        let cards: [Rc<VecModel<CardData>>; 4] = Default::default();
        let columns = Rc::new(VecModel::<ColumnData>::default());
        window.set_columns(ModelRc::from(columns.clone()));
        // акцент — как Accent в Tokens.Light/Dark.xaml
        window.set_drag_image(drag_image((0x00, 0x67, 0xc0)));
        window.set_drag_image_dark(drag_image((0x4c, 0xc2, 0xff)));
        window.set_notice(notice(dir, &problem).into());
        let state = Rc::new(RefCell::new(State {
            board,
            settings,
            loaded,
            shown_after: None,
            exe_bytes: metrics::exe_size(),
            refreshed_on: std::cell::Cell::new(now().date_naive()),
            cards,
            columns,
        }));
        let app = App { window, state };
        app.refresh();
        app.show_detail();
        app.update_metrics();
        app.wire();
        Ok(app)
    }

    /// Раз в минуту (main.rs): сменился день — возраст карточек и скрытые «Готово» пересчитать. Чаще не трогаем: пересчёт
    /// пересоздаёт карточки, а под курсором может быть перенос.
    pub fn tick(&self) {
        self.tick_at(now());
    }

    pub fn tick_at(&self, now: DateTime<FixedOffset>) {
        let st = self.state.borrow();
        if st.refreshed_on.get() != now.date_naive() {
            App::refresh_in(&st, now);
            App::show_detail_in(&self.window, &st);
        }
    }

    /// Ещё строка в шапку (например, что глобальная клавиша не заработала).
    pub fn add_notice(&self, text: &str) {
        let notice = self.window.get_notice();
        self.window.set_notice(format!("{text}. {notice}").into());
    }

    /// Окно показано — запомнить, сколько прошло с запуска процесса.
    pub fn mark_shown(&self) {
        self.state.borrow_mut().shown_after = metrics::since_start();
        self.update_metrics();
    }

    /// Строка цифр: размер exe, память, запуск, чтение доски.
    pub fn update_metrics(&self) {
        let st = self.state.borrow();
        let mut parts = Vec::new();
        if let Some(size) = st.exe_bytes {
            parts.push(format!("exe {}", metrics::megabytes(size)));
        }
        if let Some(m) = metrics::memory() {
            parts.push(format!("память {} (частная {})", metrics::megabytes(m.working), metrics::megabytes(m.private)));
        }
        if let Some(d) = st.shown_after {
            parts.push(format!("окно на экране через {} мс после запуска", d.as_millis()));
        }
        parts.push(st.loaded.clone());
        self.window.set_metrics(parts.join(" · ").into());
    }

    fn wire(&self) {
        self.window.on_payload(|key| {
            let mut data = DataTransfer::default();
            data.set_user_data(Rc::new(key.to_string()));
            data
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_select(move |key| {
            let Some(w) = weak.upgrade() else { return };
            w.set_selected_key(key);
            w.set_read_result(SharedString::new());
            App::show_detail_in(&w, &state.borrow());
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_drop_card(move |data, column| {
            let Some(w) = weak.upgrade() else { return };
            let Some(key) = data.user_data().and_then(|d| d.downcast::<String>().ok()) else { return };
            let Some(to) = Status::from_index(column as usize) else { return };
            let moved = state.borrow_mut().board.move_card(&key, to, now());
            if moved {
                App::refresh_in(&state.borrow(), now());
                App::show_detail_in(&w, &state.borrow());
            }
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_read_ticket(move || {
            let Some(w) = weak.upgrade() else { return };
            let st = state.borrow();
            let key = w.get_selected_key().to_string();
            let Some(id) = st.board.find(&key).and_then(|t| t.intraservice_id) else { return };
            let s = st.settings.clone();
            w.set_reading(true);
            w.set_read_result("Читаю заявку из Интрасервиса…".into());
            let weak = w.as_weak();
            // сеть — не в потоке окна; ответ — обратно в него, и только если выбрана всё та же карточка
            std::thread::spawn(move || {
                let started = Instant::now();
                let answer = match dpapi::unprotect(&s.password_protected) {
                    Err(e) => Err(e),
                    Ok(password) => intraservice::Client::new(&s.base_url, &s.login, &password, READ_TIMEOUT).get_task(id),
                };
                let took = started.elapsed().as_millis();
                let text = match answer {
                    Ok(t) => format!(
                        "#{} «{}»\nСтатус в Интрасервисе: {}\nИзменена: {}\nИнициатор: {}\nИсполнители: {}\nОтвет за {took} мс",
                        t.id,
                        t.name,
                        t.status,
                        t.changed.unwrap_or_else(|| "—".into()),
                        t.creator.unwrap_or_else(|| "—".into()),
                        t.executors.unwrap_or_else(|| "—".into()),
                    ),
                    Err(e) => format!("Не прочиталась ({took} мс): {e}"),
                };
                let text = format!("{text}\n\nСеть: напрямую, без прокси; {}", intraservice::system_proxy());
                let _ = weak.upgrade_in_event_loop(move |w| {
                    if w.get_selected_key() == key.as_str() {
                        w.set_read_result(text.into());
                    }
                    w.set_reading(false);
                });
            });
        });
    }

    fn refresh(&self) {
        App::refresh_in(&self.state.borrow(), now());
    }

    /// Колонки и карточки — заново из доски на момент `now`. Модели те же: прокрутка колонок не сбрасывается.
    fn refresh_in(st: &State, now: DateTime<FixedOffset>) {
        let today = now.date_naive();
        st.refreshed_on.set(today);
        let rules = st.settings.rules();
        for status in Status::ALL {
            let cards: Vec<CardData> = st.board.visible(status, now, rules).iter().map(|t| card_data(t, today, &st.settings)).collect();
            let count = cards.len() as i32;
            st.cards[status.index()].set_vec(cards);
            let column = ColumnData {
                title: status.title().into(),
                count,
                hint: Board::hint(status, rules).into(),
                overloaded: st.board.overloaded(status, now, rules),
                cards: ModelRc::from(st.cards[status.index()].clone()),
            };
            if status.index() < st.columns.row_count() {
                st.columns.set_row_data(status.index(), column);
            } else {
                st.columns.push(column);
            }
        }
    }

    fn show_detail(&self) {
        App::show_detail_in(&self.window, &self.state.borrow());
    }

    fn show_detail_in(w: &MainWindow, st: &State) {
        let key = w.get_selected_key();
        let Some(t) = st.board.find(&key) else {
            w.set_detail_title("Выберите карточку".into());
            w.set_detail_text("Щелчок — выбор, перетаскивание — перенос в другую колонку (до выхода).".into());
            w.set_can_read(false);
            return;
        };
        w.set_detail_title(format!("{} {}", t.number(), t.title).into());
        // «В работе 3 дня — просрочена · в Интрасервисе «Открыта»», как строка возраста в панели C#
        let mut lines = vec![format!(
            "{} {} · {}",
            t.status.title(),
            t.age_text(now().date_naive(), st.settings.rules()),
            t.external_status
                .as_deref()
                .map_or("статус в Интрасервисе неизвестен".to_string(), |s| format!("в Интрасервисе «{s}»")),
        )];
        if let Some(c) = &t.creator {
            lines.push(format!("Инициатор: {c}"));
        }
        if let Some(why) = why_not_read(t, &st.settings) {
            lines.push(why.into());
        }
        w.set_detail_text(lines.join("\n").into());
        w.set_can_read(t.intraservice_id.is_some() && st.settings.has_api());
    }
}

/// Почему кнопка чтения не нажимается.
fn why_not_read(t: &Ticket, settings: &Settings) -> Option<&'static str> {
    if t.intraservice_id.is_none() {
        Some("Без номера — читать из Интрасервиса нечего.")
    } else if !settings.has_api() {
        Some("API Интрасервиса не настроен в TicketBoard (адрес, логин, пароль).")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::parse_tickets;

    #[test]
    fn why_the_read_button_is_off() {
        let api = Settings { base_url: "https://hd".into(), login: "me".into(), password_protected: "AQAA".into(), ..Settings::default() };
        let numbered = &parse_tickets(r#"[{"Id": "a", "IntraserviceId": 5}]"#).unwrap()[0];
        let own = &parse_tickets(r#"[{"Id": "b"}]"#).unwrap()[0];
        assert_eq!(why_not_read(numbered, &api), None);
        assert_eq!(why_not_read(own, &api), Some("Без номера — читать из Интрасервиса нечего."));
        assert_eq!(
            why_not_read(numbered, &Settings::default()),
            Some("API Интрасервиса не настроен в TicketBoard (адрес, логин, пароль).")
        );
    }

    #[test]
    fn the_notice_names_the_folder_and_a_settings_problem_first() {
        let dir = Path::new("/data/tb");
        assert_eq!(
            notice(dir, ""),
            "Прототип: читает tickets.json и settings.json из /data/tb и ничего не пишет — переносы держатся до выхода."
        );
        assert!(notice(dir, "settings.json не прочитан: x").starts_with("settings.json не прочитан: x. Прототип: читает"));
    }
}
