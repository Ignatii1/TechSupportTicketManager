//! Окно и данные: доска (`board::Board`) → модели Slint; выбор, перенос, чтение заявки из Интрасервиса, цифры.
//! Перенос меняет только память: файлы не пишутся.

use crate::board::{AgeState, Board, Priority, Status, Ticket, load_tickets, plural};
use crate::settings::{Settings, load_settings};
use crate::{CardData, ColumnData, MainWindow, dpapi, intraservice, metrics};
use chrono::{DateTime, FixedOffset, Local};
use slint::{ComponentHandle, DataTransfer, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashSet;
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
    /// Внизу — отчёт «Проверить подключение», а не прочитанная карточка: выбор другой его не стирает.
    report_shown: std::cell::Cell<bool>,
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
        window.set_can_check(settings.has_api());
        let state = Rc::new(RefCell::new(State {
            board,
            settings,
            loaded,
            shown_after: None,
            exe_bytes: metrics::exe_size(),
            refreshed_on: std::cell::Cell::new(now().date_naive()),
            report_shown: std::cell::Cell::new(false),
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
            if !state.borrow().report_shown.get() {
                w.set_read_result(SharedString::new());
            }
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
            let Some(id) = st.board.find(&key).and_then(|t| t.intraservice_id).and_then(|n| i32::try_from(n).ok()) else { return };
            let s = st.settings.clone();
            w.set_reading(true);
            st.report_shown.set(false);
            w.set_read_result("Читаю заявку из Интрасервиса…".into());
            let weak = w.as_weak();
            // сеть — не в потоке окна; ответ — обратно в него, и только если выбрана всё та же карточка
            std::thread::spawn(move || {
                let started = Instant::now();
                let text = match dpapi::unprotect(&s.password_protected) {
                    Err(e) => format!("Не прочиталась ({} мс): {e}", started.elapsed().as_millis()),
                    Ok(password) => {
                        let client = intraservice::Client::new(&s.base_url, &s.login, &password, READ_TIMEOUT);
                        match client.get_task(id) {
                            Ok(task) => {
                                let life = client.get_lifetime_page(id, 1);
                                card_text(&task, &life, started.elapsed().as_millis())
                            }
                            Err(e) => format!("Не прочиталась ({} мс): {e}", started.elapsed().as_millis()),
                        }
                    }
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

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_check_connection(move || {
            let Some(w) = weak.upgrade() else { return };
            let st = state.borrow();
            let s = st.settings.clone();
            let on_board: HashSet<i32> =
                st.board.tickets.iter().filter_map(|t| t.intraservice_id).filter_map(|n| i32::try_from(n).ok()).collect();
            w.set_reading(true);
            st.report_shown.set(true);
            w.set_read_result("Проверяю подключение к Интрасервису…".into());
            let weak = w.as_weak();
            std::thread::spawn(move || {
                let started = Instant::now();
                let (check, mine) = match dpapi::unprotect(&s.password_protected) {
                    Err(e) => (Err(e), None),
                    Ok(password) => {
                        let client = intraservice::Client::new(&s.base_url, &s.login, &password, READ_TIMEOUT);
                        let check = client.check();
                        let mine = check.is_ok().then(|| client.my_open(&s.closed_names()));
                        (check, mine)
                    }
                };
                let text = connection_text(&check, mine.as_ref(), &on_board, started.elapsed().as_millis());
                let text = format!("{text}\n\nСеть: напрямую, без прокси; {}", intraservice::system_proxy());
                let _ = weak.upgrade_in_event_loop(move |w| {
                    w.set_read_result(text.into());
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

/// Дата для человека, по часам этого компьютера: «12.10.2026 09:42».
fn shown(d: &intraservice::Date) -> String {
    d.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string()
}

/// Карточка и переписка из Интрасервиса — текстом для окна, как их показывает панель C#: поля (пустые — не строкой),
/// описание без html, комментарии (свежие сверху, внутренние — помечены), смены статуса — числом.
fn card_text(t: &intraservice::Task, life: &Result<intraservice::LifetimePage, String>, took: u128) -> String {
    let some = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let join = |parts: Vec<Option<String>>| {
        let parts: Vec<String> = parts.into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    };
    let x = t.extra.clone().unwrap_or_default();
    let mut lines = vec![format!("#{} «{}»", t.id, t.name)];
    lines.push(match &t.changed {
        Some(changed) => format!("Статус в Интрасервисе: {} · изменена {}", t.status, shown(changed)),
        None => format!("Статус в Интрасервисе: {}", t.status),
    });
    if let Some(creator) = some(&t.creator) {
        let parts = vec![Some(creator), some(&t.creator_position), some(&x.company), some(&t.creator_phone), some(&t.creator_email)];
        lines.push(format!("Инициатор: {}", join(parts).unwrap_or_default()));
    }
    if let Some(people) = join(vec![some(&t.executors), some(&t.executor_group).map(|g| format!("группа «{g}»"))]) {
        lines.push(format!("Исполнители: {people}"));
    }
    if let Some(observers) = some(&t.observers) {
        lines.push(format!("Наблюдатели: {observers}"));
    }
    if let Some(service) = join(vec![some(&x.service), some(&x.task_type), some(&x.categories)]) {
        lines.push(format!("Сервис: {service}"));
    }
    if let Some(files) = x.files.as_ref().filter(|f| !f.is_empty()) {
        let names: Vec<String> = files.iter().map(|f| f.name.clone().unwrap_or_else(|| format!("№{}", f.id))).collect();
        lines.push(format!("Вложения ({}): {}", files.len(), names.join(", ")));
    }
    lines.push(String::new());
    match &t.description {
        Some(d) => lines.extend(["Описание:".to_string(), d.clone()]),
        None => lines.push("Описания нет.".into()),
    }
    lines.push(String::new());
    match life {
        Ok(page) => {
            let comments: Vec<&intraservice::Event> = page.events.iter().filter(|e| e.comment.is_some()).collect();
            let (n, changes) = (comments.len() as i64, (page.events.len() - comments.len()) as i64);
            lines.push(format!(
                "Переписка, свежие сверху: {n} {}, {changes} {}{}",
                plural(n, "комментарий", "комментария", "комментариев"),
                plural(changes, "смена статуса", "смены статуса", "смен статуса"),
                if page.has_more { " (есть ещё страницы)" } else { "" }
            ));
            for (k, e) in comments.iter().enumerate() {
                if k > 0 {
                    lines.push(String::new());
                }
                let mut head: Vec<String> = e.date.as_ref().map(shown).into_iter().collect();
                head.push(if e.author.is_empty() { "—".into() } else { e.author.clone() });
                if e.is_public == Some(false) {
                    head.push("внутренний".into());
                }
                lines.push(head.join(" · "));
                lines.push(e.comment.clone().unwrap_or_default());
            }
        }
        Err(e) => lines.push(format!("Переписка не прочиталась: {}", intraservice::brief(e))),
    }
    lines.push(String::new());
    lines.push(format!("Ответ за {took} мс"));
    lines.join("\n")
}

/// «Проверить подключение» — текстом: есть ли (`CheckAsync`), кто я, что считается закрытым, мои открытые (как их читает
/// импорт) и сколько из них нет на доске. Только чтение.
fn connection_text(check: &Result<(), String>, mine: Option<&intraservice::MyOpen>, on_board: &HashSet<i32>, took: u128) -> String {
    if let Err(e) = check {
        return format!("Подключения нет ({took} мс): {e}");
    }
    let mut lines = vec![format!("Подключение есть, ответ за {took} мс.")];
    let Some(mine) = mine else { return lines.join("\n") };
    let Some(me) = &mine.me else {
        lines.push(format!("Мои открытые заявки не прочитались: {}", mine.error));
        return lines.join("\n");
    };
    lines.push(format!("Вы — {} (№{}).", me.name, me.id));
    lines.push(format!("Закрытыми считаются: {}.", mine.closed.names().join(", ")));
    if !mine.error.is_empty() {
        lines.push(format!("Мои открытые заявки прочитались не все ({}): {}", mine.rows.len(), mine.error));
        return lines.join("\n");
    }
    let missing: Vec<i32> = mine.rows.iter().map(|f| f.id).filter(|id| !on_board.contains(id)).collect();
    let count = if mine.complete {
        mine.rows.len().to_string()
    } else {
        format!("взяты первые {} из {}", mine.rows.len(), mine.total)
    };
    let tail = if missing.is_empty() {
        "все на доске".to_string()
    } else {
        let first: Vec<String> = missing.iter().take(10).map(|id| format!("#{id}")).collect();
        let more = if missing.len() > 10 { format!(" и ещё {}", missing.len() - 10) } else { String::new() };
        format!("на доске {}, нет на доске {}: {}{more}", mine.rows.len() - missing.len(), missing.len(), first.join(", "))
    };
    lines.push(format!("Мои открытые заявки: {count} — {tail}."));
    lines.join("\n")
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
    use crate::intraservice::{MyOpen, User, parse};
    use crate::rules::ClosedNames;

    const CARD: &str = r#"{"Task":{"Id":712182,"Name":"Не проводятся упаковки","StatusName":"Ожидание ответа пользователя","Changed":"2026-10-12T09:42:10.85","Creator":"Иванов И. И.","CreatorPosition":"Специалист","CreatorCompanyName":"Отдел поддержки","CreatorPhone":null,"CreatorEmail":"ivanov@example.ru","Executors":"Петров П. П., Иванов И. И.","ExecutorGroup":"ТП склад","ServiceName":"Маркировка","Type":"Запрос на обслуживание","Categories":null,"FileIds":"1, 2","FileNames":"a.xlsx||","Observers":"Орлова К. А.","Description":"<p>Просьба помочь.</p><p>Примеры: 0004</p>"}}"#;
    const LIFE: &str = r#"[{"Comments":"Добрый день!\r\nда, свяжусь.","IsPublic":true,"Date":"2026-10-02T14:07:07","Editor":"Сидорова М. И."},{"Comments":"Свяжитесь с поставщиком","IsPublic":false,"Date":"2026-10-02T14:02:22","Editor":"Иванов И. И.","StatusId":46},{"Date":"2026-10-02T14:00:48","Editor":"Иванов И. И.","StatusId":31}]"#;

    #[test]
    fn the_card_as_read() {
        let task = parse::task(CARD, 712182).unwrap();
        let life = parse::lifetime(LIFE).unwrap();
        assert_eq!(
            card_text(&task, &Ok(life), 42),
            "#712182 «Не проводятся упаковки»
Статус в Интрасервисе: Ожидание ответа пользователя · изменена 12.10.2026 09:42
Инициатор: Иванов И. И. · Специалист · Отдел поддержки · ivanov@example.ru
Исполнители: Петров П. П., Иванов И. И. · группа «ТП склад»
Наблюдатели: Орлова К. А.
Сервис: Маркировка · Запрос на обслуживание
Вложения (2): a.xlsx, №2

Описание:
Просьба помочь.
Примеры: 0004

Переписка, свежие сверху: 2 комментария, 1 смена статуса
02.10.2026 14:07 · Сидорова М. И.
Добрый день!
да, свяжусь.

02.10.2026 14:02 · Иванов И. И. · внутренний
Свяжитесь с поставщиком

Ответ за 42 мс"
        );
        // карточка без подробностей; переписка не прочиталась — сказано одной строкой, есть ещё страницы — тоже
        let bare = parse::task(r#"{"Id":5,"Name":"Голая","StatusName":"Открыта"}"#, 5).unwrap();
        assert_eq!(
            card_text(&bare, &Err("сервер не ответил за 10 с".into()), 7),
            "#5 «Голая»\nСтатус в Интрасервисе: Открыта\n\nОписания нет.\n\nПереписка не прочиталась: сервер не ответил за 10 с\n\nОтвет за 7 мс"
        );
        let more = parse::lifetime(
            r#"{"TaskLifetimes":[{"Date":"2026-10-02T14:00:48","Editor":"И","StatusId":31}],"Paginator":{"Page":1,"PageCount":2}}"#,
        )
        .unwrap();
        assert!(
            card_text(&bare, &Ok(more), 7)
                .contains("Переписка, свежие сверху: 0 комментариев, 1 смена статуса (есть ещё страницы)\n\nОтвет")
        );
    }

    fn found(ids: &[i32]) -> Vec<crate::intraservice::Found> {
        ids.iter().map(|&id| crate::intraservice::Found { id, ..Default::default() }).collect()
    }

    #[test]
    fn the_connection_report() {
        let me = Some(User { id: 40800, name: "Иванов И. И.".into() });
        let closed = ClosedNames::new(&["Закрыта".to_string(), "Выполнена".to_string()]);
        let board: HashSet<i32> = [1, 2, 99].into();
        let mine =
            MyOpen { rows: found(&[1, 2, 3]), total: 3, complete: true, error: String::new(), closed: closed.clone(), me: me.clone() };
        assert_eq!(
            connection_text(&Ok(()), Some(&mine), &board, 42),
            "Подключение есть, ответ за 42 мс.\nВы — Иванов И. И. (№40800).\nЗакрытыми считаются: Выполнена, Закрыта.\nМои открытые заявки: 3 — на доске 2, нет на доске 1: #3."
        );
        // все на доске; больше десяти не на доске — первые десять и сколько ещё
        let all = MyOpen { rows: found(&[1, 2]), total: 2, ..mine.clone() };
        assert!(connection_text(&Ok(()), Some(&all), &board, 1).ends_with("Мои открытые заявки: 2 — все на доске."));
        let many = MyOpen { rows: found(&(100..112).collect::<Vec<_>>()), total: 12, ..mine.clone() };
        assert!(
            connection_text(&Ok(()), Some(&many), &board, 1)
                .ends_with("нет на доске 12: #100, #101, #102, #103, #104, #105, #106, #107, #108, #109 и ещё 2."),
            "{}",
            connection_text(&Ok(()), Some(&many), &board, 1)
        );
        // список неполный (потолок страниц); не прочитался; кто я — неизвестно
        let partial = MyOpen { rows: found(&[1, 3]), total: 2500, complete: false, ..mine.clone() };
        assert!(
            connection_text(&Ok(()), Some(&partial), &board, 1)
                .ends_with("Мои открытые заявки: взяты первые 2 из 2500 — на доске 1, нет на доске 1: #3.")
        );
        let broken =
            MyOpen { rows: found(&[1]), error: "ошибка сервера (HTTP 500):\n{}".into(), complete: false, ..mine.clone() };
        assert!(
            connection_text(&Ok(()), Some(&broken), &board, 1)
                .ends_with("Мои открытые заявки прочитались не все (1): ошибка сервера (HTTP 500):\n{}"),
            "{}",
            connection_text(&Ok(()), Some(&broken), &board, 1)
        );
        let nobody = MyOpen { error: "неверный логин или пароль (HTTP 401)".into(), ..MyOpen::default() };
        assert_eq!(
            connection_text(&Ok(()), Some(&nobody), &board, 5),
            "Подключение есть, ответ за 5 мс.\nМои открытые заявки не прочитались: неверный логин или пароль (HTTP 401)"
        );
        // подключения нет — только это, целиком
        assert_eq!(
            connection_text(&Err("сервер недоступен:\nNo such host".into()), None, &board, 9),
            "Подключения нет (9 мс): сервер недоступен:\nNo such host"
        );
    }

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
