//! Окно и данные: доска (`board::Board`) → модели Slint; выбор, перенос, правки, панель, быстрое добавление, ⟳ из
//! Интрасервиса, «Проверить подключение», цифры. `tickets.json` пишется, только когда доска своя (`single`): правки
//! собираются 600 мс и пишутся разом, как `_saveTimer`. Сеть — в потоках; их ответы идут в очередь (`Message`), окно
//! забирает их в своём потоке (`deliver`) — состояние доски живёт только там.

use crate::board::{AgeState, Board, Date, Priority, Status, Ticket, delete_question, now, plural};
use crate::capture::{self, Capture};
use crate::link::LinkParser;
use crate::settings::{Settings, load_settings};
use crate::store::Store;
use crate::{CaptureWindow, CardData, ColumnData, MainWindow, NoteData, PanelData, clipboard, dpapi, intraservice, journal, metrics, sync};
use chrono::Local;
use slint::{
    ComponentHandle, DataTransfer, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Сколько ждать ответа Интрасервиса — как обычный запрос C#-клиента.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Правки собираются столько и пишутся разом (`_saveTimer`).
pub const SAVE_DELAY: Duration = Duration::from_millis(600);

/// Ответы потоков окну.
enum Message {
    Synced { key: String, n: i32, card: Result<Box<intraservice::Task>, String>, life: Option<Result<intraservice::LifetimePage, String>> },
    Title { generation: u64, result: Result<String, String> },
    Report(String),
}

struct State {
    dir: PathBuf,
    board: Board,
    settings: Settings,
    parser: LinkParser,
    /// None — доска только для чтения: её держит TicketBoard.exe или файл не прочитался.
    store: Option<Store>,
    /// Клиент Интрасервиса или почему его нет.
    client: Result<Arc<intraservice::Client>, String>,
    /// «42 карточки за 3 мс» или почему доски нет.
    loaded: String,
    /// Окно на экране через столько после создания процесса (ставит `mark_shown`).
    shown_after: Option<Duration>,
    /// Размер exe — один раз: пока процесс жив, он не меняется.
    exe_bytes: Option<u64>,
    /// День, на который посчитан возраст карточек: сменился — пересчитать (`tick_at`).
    refreshed_on: chrono::NaiveDate,
    cards: [Rc<VecModel<CardData>>; 4],
    columns: Rc<VecModel<ColumnData>>,
    notes: Rc<VecModel<NoteData>>,
    capture: Capture,
    /// Правки ещё не записаны.
    dirty: bool,
    /// Карточки, которые сейчас читаются из Интрасервиса.
    syncing: HashSet<String>,
    /// Сообщение ⟳ у карточки (`SyncMessage`): видно, пока она выбрана.
    sync_message: (String, String),
    /// Переписка выбранной карточки после ⟳: (карточка, текст).
    conversation: (String, String),
    /// Окно вопроса: удалить эту карточку (None — отчёт, не вопрос).
    asking_delete: Option<String>,
    /// Шапка: откуда доска и что с ней; не записалось — почему, впереди.
    notice: String,
    save_problem: String,
}

pub struct App {
    pub window: MainWindow,
    pub capture: CaptureWindow,
    state: Rc<RefCell<State>>,
    inbox: Arc<Mutex<Vec<Message>>>,
    save_timer: Rc<Timer>,
}

fn card_data(t: &Ticket, today: chrono::NaiveDate, settings: &Settings) -> CardData {
    CardData {
        key: t.id.as_str().into(),
        number: t.number().into(),
        title: (if t.title.trim().is_empty() { "без названия" } else { t.title.as_str() }).into(),
        context: t.context().into(),
        priority: t.priority.title().into(),
        priority_level: t.priority as i32,
        age: t.age_label(today).into(),
        age_state: match t.age_state(today, settings.rules()) {
            AgeState::Neutral => 0,
            AgeState::Warn => 1,
            AgeState::Overdue => 2,
        },
        unread: t.unread_comments,
        files: t.files.as_ref().map_or(0, Vec::len) as i32,
        notes: t.notes.len() as i32,
    }
}

/// Дата для человека, по часам этого компьютера: «12.10.2026 09:42».
fn shown(d: &Date) -> String {
    d.with_timezone(&Local).format("%d.%m.%Y %H:%M").to_string()
}

fn panel_data(t: &Ticket, today: chrono::NaiveDate, settings: &Settings) -> PanelData {
    let or_empty = |v: &Option<String>| v.clone().unwrap_or_default();
    PanelData {
        number: t.number().into(),
        has_number: t.intraservice_id.is_some(),
        status_index: t.status.index() as i32,
        priority_level: t.priority as i32,
        created: t.created_at.with_timezone(&Local).format("%d.%m.%Y").to_string().into(),
        age: t.age_text(today, settings.rules()).into(),
        external_status: or_empty(&t.external_status).into(),
        synced: t.last_sync_at.as_ref().map_or("ещё не обновлялась".to_string(), |d| format!("обновлено {}", shown(d))).into(),
        creator: or_empty(&t.creator).into(),
        creator_role: t.creator_role().into(),
        creator_contacts: t.creator_contacts().into(),
        executors: or_empty(&t.executors).into(),
        executor_group: or_empty(&t.executor_group).into(),
        service: t.service_text().into(),
        categories: or_empty(&t.categories).into(),
        assets: or_empty(&t.assets).into(),
    }
}

/// Шапка: чья доска и где она; не прочитались настройки — сначала это.
pub fn notice(dir: &Path, problem: &str, writable: bool, unread: Option<&str>) -> String {
    let base = match (writable, unread) {
        (_, Some(e)) => format!("{e} — доска только для чтения, файл не тронут: TicketBoard.exe отложит его в сторону сам."),
        (true, None) => format!("Доска — {}; пока открыт прототип, TicketBoard.exe не запустится.", dir.display()),
        (false, None) => "TicketBoard.exe открыт — доска только для чтения. Закройте его и запустите прототип снова.".to_string(),
    };
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

/// Переписка из Интрасервиса текстом (страница истории, свежие сверху): комментарии, внутренние — помечены, смены статуса —
/// числом.
fn conversation_text(life: &Result<intraservice::LifetimePage, String>) -> String {
    let page = match life {
        Ok(page) => page,
        Err(e) => return format!("Переписка не прочиталась: {}", intraservice::brief(e)),
    };
    let comments: Vec<&intraservice::Event> = page.events.iter().filter(|e| e.comment.is_some()).collect();
    let (n, changes) = (comments.len() as i64, (page.events.len() - comments.len()) as i64);
    let mut lines = vec![format!(
        "Свежие сверху: {n} {}, {changes} {}{}",
        plural(n, "комментарий", "комментария", "комментариев"),
        plural(changes, "смена статуса", "смены статуса", "смен статуса"),
        if page.has_more { " (есть ещё страницы)" } else { "" }
    )];
    for e in comments {
        lines.push(String::new());
        let mut head: Vec<String> = e.date.as_ref().map(shown).into_iter().collect();
        head.push(if e.author.is_empty() { "—".into() } else { e.author.clone() });
        if e.is_public == Some(false) {
            head.push("внутренний".into());
        }
        lines.push(head.join(" · "));
        lines.push(e.comment.clone().unwrap_or_default());
    }
    lines.join("\n")
}

/// Ссылку — в браузер; только http(s): ссылку правят руками, а оболочка открыла бы и файл, и программу.
#[cfg(windows)]
fn open_url(url: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    if is_web(url) {
        // SAFETY: строки живут до конца вызова
        unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url.trim()), None, None, SW_SHOWNORMAL) };
    }
}

#[cfg(not(windows))]
fn open_url(_url: &str) {}

pub fn is_web(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    (u.starts_with("https://") || u.starts_with("http://")) && !u.contains(['"', '\n', '\r'])
}

impl App {
    /// Доска из `tickets.json` и `settings.json` этой папки (TicketBoard.exe держит их рядом с собой). `owner` — доска
    /// своя (`single::take`): правки пишутся; иначе только чтение.
    pub fn open(dir: &Path, owner: bool) -> Result<App, slint::PlatformError> {
        let window = MainWindow::new()?;
        let capture = CaptureWindow::new()?;
        let started = Instant::now();
        let store = Store::new(dir);
        let (board, loaded, unread) = match store.load() {
            Ok(tickets) => {
                let n = tickets.len();
                let took = started.elapsed().as_millis();
                (
                    Board::new(tickets),
                    format!("tickets.json: {n} {} за {took} мс", plural(n as i64, "карточка", "карточки", "карточек")),
                    None,
                )
            }
            Err(e) => (Board::default(), e.clone(), Some(e)),
        };
        let writable = owner && unread.is_none();
        let (settings, problem) = load_settings(dir);
        let client = client_for(dir, &settings);
        let cards: [Rc<VecModel<CardData>>; 4] = Default::default();
        let columns = Rc::new(VecModel::<ColumnData>::default());
        let notes = Rc::new(VecModel::<NoteData>::default());
        window.set_columns(ModelRc::from(columns.clone()));
        window.set_notes(ModelRc::from(notes.clone()));
        // акцент — как Accent в Tokens.Light/Dark.xaml
        window.set_drag_image(drag_image((0x00, 0x67, 0xc0)));
        window.set_drag_image_dark(drag_image((0x4c, 0xc2, 0xff)));
        let notice = notice(dir, &problem, writable, unread.as_deref());
        window.set_notice(notice.as_str().into());
        window.set_writable(writable);
        window.set_can_check(settings.has_api());
        let parser = LinkParser::new(&settings.id_pattern);
        let mut st = State {
            dir: dir.to_path_buf(),
            board,
            settings,
            parser,
            store: writable.then_some(store),
            client,
            loaded,
            shown_after: None,
            exe_bytes: metrics::exe_size(),
            refreshed_on: now().date_naive(),
            cards,
            columns,
            notes,
            capture: Capture::default(),
            dirty: false,
            syncing: HashSet::new(),
            sync_message: Default::default(),
            conversation: Default::default(),
            asking_delete: None,
            notice,
            save_problem: String::new(),
        };
        if !problem.is_empty() {
            journal::append(dir, &problem);
        }
        if let Some(e) = &unread {
            journal::append(dir, e);
        }
        st.capture.hint = capture::EMPTY_HINT.into();
        let app = App { window, capture, state: Rc::new(RefCell::new(st)), inbox: Arc::default(), save_timer: Rc::new(Timer::default()) };
        app.refresh();
        app.show_panel(true);
        app.show_capture_state();
        app.update_metrics();
        app.wire();
        Ok(app)
    }

    /// Доска своя: правки пишутся.
    pub fn writable(&self) -> bool {
        self.state.borrow().store.is_some()
    }

    /// Глобальная клавиша быстрого добавления из настроек.
    pub fn hotkey(&self) -> String {
        self.state.borrow().settings.hotkey.clone()
    }

    /// Другой клиент Интрасервиса (проверки — поддельный сервер; None — API нет): как `ApplySettings`.
    pub fn use_client(&self, client: Option<intraservice::Client>) {
        let mut st = self.state.borrow_mut();
        st.client = client.map(Arc::new).ok_or_else(|| "API не настроен: адрес, логин и пароль — в настройках TicketBoard".to_string());
        st.capture.forget_lookup();
        drop(st);
        self.show_panel(false);
    }

    /// Раз в минуту (main.rs): сменился день — возраст карточек и скрытые «Готово» пересчитать. Чаще не трогаем: пересчёт
    /// пересоздаёт карточки, а под курсором может быть перенос.
    pub fn tick(&self) {
        self.tick_at(now());
    }

    pub fn tick_at(&self, now: Date) {
        if self.state.borrow().refreshed_on != now.date_naive() {
            App::refresh_in(&mut self.state.borrow_mut(), now);
            App::show_panel_in(&self.window, &self.state.borrow(), false);
        }
    }

    /// Ещё строка в шапку (например, что глобальная клавиша не заработала).
    pub fn add_notice(&self, text: &str) {
        let mut st = self.state.borrow_mut();
        st.notice = format!("{text}. {}", st.notice);
        App::show_notice(&self.window, &st);
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
            parts.push(format!("окно на экране через {} мс", d.as_millis()));
        }
        parts.push(st.loaded.clone());
        self.window.set_metrics(parts.join(" · ").into());
    }

    /// Записать правки сейчас (выход; проверки), не дожидаясь паузы.
    pub fn save_now(&self) {
        self.save_timer.stop();
        App::save_in(&self.window, &mut self.state.borrow_mut());
    }

    /// Есть незаписанные правки.
    pub fn dirty(&self) -> bool {
        self.state.borrow().dirty
    }

    /// Быстрое добавление (клавиша из настроек, N, «+»): поле пустое, в нём ссылка из буфера, если она там.
    pub fn show_capture(&self) {
        App::show_capture_in(&self.capture, &self.state, &self.inbox, &self.window.as_weak());
    }

    fn show_capture_in(
        capture: &CaptureWindow,
        state: &Rc<RefCell<State>>,
        inbox: &Arc<Mutex<Vec<Message>>>,
        main: &slint::Weak<MainWindow>,
    ) {
        if state.borrow().store.is_none() {
            return;
        }
        {
            let mut st = state.borrow_mut();
            let parser = st.parser.clone();
            st.capture.reset(&parser);
        }
        // скопировал ссылку в браузере → клавиша → Enter: самый частый случай
        if let Some(text) = clipboard::text() {
            let link = state.borrow().parser.parse(&text);
            if link.has_url() {
                App::capture_edited(state, inbox, main, &link.url);
            }
        }
        App::show_capture_in_window(capture, &state.borrow());
        let _ = capture.show();
        capture.invoke_focus_input();
    }

    fn show_capture_in_window(w: &CaptureWindow, st: &State) {
        let c = &st.capture;
        w.set_text(c.text.as_str().into());
        w.set_number_text(c.number_text().into());
        w.set_preview(c.preview.as_str().into());
        w.set_hint(c.hint.as_str().into());
        w.set_priority_level(c.priority as i32);
        w.set_digits_set_priority(c.digits_set_priority());
    }

    fn show_capture_state(&self) {
        App::show_capture_in_window(&self.capture, &self.state.borrow());
    }

    /// Текст в окне добавления сменился: номер другой — после паузы спросить название (если ввод тот же).
    fn capture_edited(state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Message>>>, main: &slint::Weak<MainWindow>, text: &str) {
        let lookup = {
            let mut st = state.borrow_mut();
            let api = st.client.is_ok();
            let parser = st.parser.clone();
            st.capture.set_text(text, &parser, api)
        };
        let Some(lookup) = lookup else { return };
        let (state, inbox, main) = (state.clone(), inbox.clone(), main.clone());
        Timer::single_shot(capture::PAUSE, move || {
            let st = state.borrow();
            let Ok(client) = st.client.clone() else { return };
            if !st.capture.current(lookup.generation) {
                return;
            }
            std::thread::spawn(move || {
                let result = client.get_task(lookup.id).map(|t| t.name);
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::Title { generation: lookup.generation, result });
                let _ = main.upgrade_in_event_loop(|w| w.invoke_deliver());
            });
        });
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
            w.set_panel_open(true);
            App::show_panel_in(&w, &state.borrow(), true);
        });

        // правка выбранной карточки; `panel` — и панель заново (заметки), не только колонки
        let edit = |app: &App, panel: bool, f: fn(&mut State, &str, &str) -> bool| {
            let (weak, state, timer) = (app.window.as_weak(), app.state.clone(), app.save_timer.clone());
            move |text: SharedString| {
                let Some(w) = weak.upgrade() else { return };
                let key = w.get_selected_key().to_string();
                let changed = {
                    let mut st = state.borrow_mut();
                    st.store.is_some() && f(&mut st, &key, &text)
                };
                if changed {
                    App::changed(&w, &state, &timer, panel);
                }
            }
        };
        self.window.on_edit_title(edit(self, false, |st, key, text| st.board.find_mut(key).map(|t| t.title = text.into()).is_some()));
        self.window.on_edit_url(edit(self, false, |st, key, text| st.board.find_mut(key).map(|t| t.url = text.into()).is_some()));
        self.window
            .on_edit_description(edit(self, false, |st, key, text| st.board.find_mut(key).map(|t| t.description = text.into()).is_some()));
        self.window.on_add_note(edit(self, true, |st, key, text| st.board.find_mut(key).is_some_and(|t| t.add_note(text, now()))));
        self.window.on_delete_note(edit(self, true, |st, key, id| st.board.find_mut(key).is_some_and(|t| t.delete_note(id))));

        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_drop_card(move |data, column| {
            let Some(w) = weak.upgrade() else { return };
            let Some(key) = data.user_data().and_then(|d| d.downcast::<String>().ok()) else { return };
            let Some(to) = Status::from_index(column as usize) else { return };
            let moved = {
                let mut st = state.borrow_mut();
                st.store.is_some() && st.board.move_card(&key, to, now())
            };
            if moved {
                App::changed(&w, &state, &timer, true);
            }
        });

        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_move_to(move |index| {
            let Some(w) = weak.upgrade() else { return };
            let Some(to) = Status::from_index(index as usize) else { return };
            let key = w.get_selected_key().to_string();
            let moved = {
                let mut st = state.borrow_mut();
                st.store.is_some() && st.board.move_card(&key, to, now())
            };
            if moved {
                App::changed(&w, &state, &timer, true);
            }
        });

        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_shift(move |delta| {
            let Some(w) = weak.upgrade() else { return };
            let key = w.get_selected_key().to_string();
            let moved = {
                let mut st = state.borrow_mut();
                st.store.is_some() && st.board.shift(&key, delta, now())
            };
            if moved {
                App::changed(&w, &state, &timer, true);
            }
        });

        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_set_priority(move |level| {
            let Some(w) = weak.upgrade() else { return };
            let Some(&p) = Priority::ALL.get(level as usize) else { return };
            let key = w.get_selected_key().to_string();
            let changed = {
                let mut st = state.borrow_mut();
                let writable = st.store.is_some();
                st.board.find_mut(&key).filter(|t| writable && t.priority != p).map(|t| t.priority = p).is_some()
            };
            if changed {
                App::changed(&w, &state, &timer, true);
            }
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_delete_card(move || {
            let Some(w) = weak.upgrade() else { return };
            let key = w.get_selected_key().to_string();
            let mut st = state.borrow_mut();
            let Some(t) = st.board.find(&key).filter(|_| st.store.is_some()) else { return };
            let text = delete_question(t);
            st.asking_delete = Some(key);
            w.set_dialog_heading("Удалить заявку с доски?".into());
            w.set_dialog_text(text.into());
            w.set_dialog_action("Удалить".into());
            w.set_dialog_open(true);
        });

        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_dialog_accept(move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_dialog_open(false);
            let removed = {
                let mut st = state.borrow_mut();
                let key = st.asking_delete.take();
                key.and_then(|k| st.board.remove(&k)).is_some()
            };
            if removed {
                w.set_selected_key(SharedString::new());
                w.set_panel_open(false);
                App::changed(&w, &state, &timer, true);
            }
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_dialog_close(move || {
            state.borrow_mut().asking_delete = None;
            if let Some(w) = weak.upgrade() {
                w.set_dialog_open(false);
            }
        });

        let (weak, state, inbox) = (self.window.as_weak(), self.state.clone(), self.inbox.clone());
        self.window.on_sync(move || {
            let Some(w) = weak.upgrade() else { return };
            let key = w.get_selected_key().to_string();
            App::sync_card(&w, &state, &inbox, &key);
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_open_link(move || {
            let Some(w) = weak.upgrade() else { return };
            if let Some(t) = state.borrow().board.find(&w.get_selected_key()) {
                open_url(&t.url);
            }
        });

        let (capture, state, inbox, main) = (self.capture.as_weak(), self.state.clone(), self.inbox.clone(), self.window.as_weak());
        self.window.on_new_ticket(move || {
            if let Some(c) = capture.upgrade() {
                App::show_capture_in(&c, &state, &inbox, &main);
            }
        });

        let (weak, state, inbox) = (self.window.as_weak(), self.state.clone(), self.inbox.clone());
        self.window.on_check_connection(move || {
            let Some(w) = weak.upgrade() else { return };
            let st = state.borrow();
            let on_board = st.board.numbers();
            let closed = st.settings.closed_names();
            let client = st.client.clone();
            w.set_reading(true);
            let (weak, inbox) = (w.as_weak(), inbox.clone());
            std::thread::spawn(move || {
                let started = Instant::now();
                let (check, mine) = match client {
                    Err(e) => (Err(e), None),
                    Ok(client) => {
                        let check = client.check();
                        let mine = check.is_ok().then(|| client.my_open(&closed));
                        (check, mine)
                    }
                };
                let text = connection_text(&check, mine.as_ref(), &on_board, started.elapsed().as_millis());
                let text = format!("{text}\n\nСеть: напрямую, без прокси; {}", intraservice::system_proxy());
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::Report(text));
                let _ = weak.upgrade_in_event_loop(|w| w.invoke_deliver());
            });
        });

        let (weak, cw, state, inbox, timer) =
            (self.window.as_weak(), self.capture.as_weak(), self.state.clone(), self.inbox.clone(), self.save_timer.clone());
        self.window.on_deliver(move || {
            let (Some(w), Some(c)) = (weak.upgrade(), cw.upgrade()) else { return };
            let messages = std::mem::take(&mut *inbox.lock().unwrap_or_else(|e| e.into_inner()));
            for m in messages {
                App::receive(&w, &c, &state, &timer, m);
            }
        });

        // окно быстрого добавления
        let (state, inbox, main, cw) = (self.state.clone(), self.inbox.clone(), self.window.as_weak(), self.capture.as_weak());
        self.capture.on_edited(move |text| {
            App::capture_edited(&state, &inbox, &main, &text);
            if let Some(c) = cw.upgrade() {
                App::show_capture_in_window(&c, &state.borrow());
            }
        });

        let (state, cw) = (self.state.clone(), self.capture.as_weak());
        self.capture.on_set_priority(move |level| {
            let Some(&p) = Priority::ALL.get(level as usize) else { return };
            state.borrow_mut().capture.priority = p;
            if let Some(c) = cw.upgrade() {
                c.set_priority_level(level);
            }
        });

        let cw = self.capture.as_weak();
        let hide = move || {
            if let Some(c) = cw.upgrade() {
                let _ = c.hide();
            }
        };
        self.capture.on_cancel(hide.clone());
        self.capture.on_deactivated(hide.clone());

        let (weak, state, inbox, timer) = (self.window.as_weak(), self.state.clone(), self.inbox.clone(), self.save_timer.clone());
        self.capture.on_accepted(move || {
            hide();
            let Some(w) = weak.upgrade() else { return };
            let key = {
                let mut st = state.borrow_mut();
                let text = st.capture.text.trim().to_string();
                if text.is_empty() || st.store.is_none() {
                    return;
                }
                let t = capture::add_from_capture(&text, st.capture.priority, &st.parser, &st.settings, now());
                let key = t.id.clone();
                st.board.insert(t);
                key
            };
            w.set_selected_key(key.as_str().into());
            w.set_panel_open(true);
            App::changed(&w, &state, &timer, true);
            App::show_panel_in(&w, &state.borrow(), true);
            let numbered = state.borrow().board.find(&key).is_some_and(|t| t.intraservice_id.is_some());
            if numbered && state.borrow().client.is_ok() {
                App::sync_card(&w, &state, &inbox, &key);
            }
        });
    }

    /// Ответ потока — в доску и в окно.
    fn receive(w: &MainWindow, c: &CaptureWindow, state: &Rc<RefCell<State>>, timer: &Rc<Timer>, m: Message) {
        match m {
            Message::Title { generation, result } => {
                state.borrow_mut().capture.answer(generation, result);
                App::show_capture_in_window(c, &state.borrow());
            }
            Message::Report(text) => {
                state.borrow_mut().asking_delete = None;
                w.set_reading(false);
                w.set_dialog_heading("Подключение к Интрасервису".into());
                w.set_dialog_text(text.into());
                w.set_dialog_action(SharedString::new());
                w.set_dialog_open(true);
            }
            Message::Synced { key, n, card, life } => {
                let applied = {
                    let mut st = state.borrow_mut();
                    st.syncing.remove(&key);
                    let message = match &card {
                        Ok(_) => String::new(),
                        Err(e) => format!("{e}\n\nСеть: напрямую, без прокси; {}", intraservice::system_proxy()),
                    };
                    st.sync_message = (key.clone(), message);
                    if let Some(life) = &life {
                        st.conversation = (key.clone(), conversation_text(life));
                    }
                    let writable = st.store.is_some();
                    match (&card, st.board.find_mut(&key)) {
                        (Ok(task), Some(t)) if writable => {
                            sync::apply(t, n, task, now());
                            true
                        }
                        _ => false,
                    }
                };
                if applied {
                    App::changed(w, state, timer, true);
                }
                if w.get_selected_key() == key.as_str() {
                    App::show_panel_in(w, &state.borrow(), applied);
                }
            }
        }
    }

    /// ⟳ карточки (`SyncAsync`): номер и API есть — прочитать в потоке, «обновляю…» пока; ответ — `receive`.
    fn sync_card(w: &MainWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Message>>>, key: &str) {
        let mut st = state.borrow_mut();
        let Some(t) = st.board.find(key) else { return };
        let message = match (t.intraservice_id, st.client.clone()) {
            (None, _) => "у заявки нет номера".to_string(),
            (_, Err(why)) => why,
            (Some(n), Ok(client)) => {
                st.syncing.insert(key.to_string());
                let (weak, inbox, key) = (w.as_weak(), inbox.clone(), key.to_string());
                std::thread::spawn(move || {
                    let card = client.get_task(n).map(Box::new);
                    let life = card.is_ok().then(|| client.get_lifetime_page(n, 1));
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::Synced { key, n, card, life });
                    let _ = weak.upgrade_in_event_loop(|w| w.invoke_deliver());
                });
                "обновляю…".to_string()
            }
        };
        st.sync_message = (key.to_string(), message);
        drop(st);
        if w.get_selected_key() == key {
            App::show_panel_in(w, &state.borrow(), false);
        }
    }

    /// Доска изменилась: колонки заново, запись — через паузу (одна на серию правок), панель — если `panel`.
    fn changed(w: &MainWindow, state: &Rc<RefCell<State>>, timer: &Rc<Timer>, panel: bool) {
        {
            let mut st = state.borrow_mut();
            st.dirty = true;
            App::refresh_in(&mut st, now());
        }
        if panel {
            App::show_panel_in(w, &state.borrow(), false);
        }
        let (weak, state) = (w.as_weak(), state.clone());
        timer.start(TimerMode::SingleShot, SAVE_DELAY, move || {
            if let Some(w) = weak.upgrade() {
                App::save_in(&w, &mut state.borrow_mut());
            }
        });
    }

    /// Записать доску, если есть что: копия за день, потом файл. Не вышло — в шапке и в журнале; правки остаются и
    /// запишутся со следующей.
    fn save_in(w: &MainWindow, st: &mut State) {
        let Some(store) = &st.store else { return };
        if !st.dirty {
            return;
        }
        if let Err(e) = store.backup(now().date_naive()) {
            journal::append(&st.dir, &e);
        }
        let problem = match store.write(&st.board) {
            Ok(()) => {
                st.dirty = false;
                String::new()
            }
            Err(e) => {
                journal::append(&st.dir, &e);
                e
            }
        };
        if problem != st.save_problem {
            st.save_problem = problem;
            App::show_notice(w, st);
        }
    }

    fn show_notice(w: &MainWindow, st: &State) {
        let text = if st.save_problem.is_empty() { st.notice.clone() } else { format!("{}. {}", st.save_problem, st.notice) };
        w.set_notice(text.into());
    }

    fn refresh(&self) {
        App::refresh_in(&mut self.state.borrow_mut(), now());
    }

    /// Колонки и карточки — заново из доски на момент `now`. Модели те же: прокрутка колонок не сбрасывается.
    fn refresh_in(st: &mut State, now: Date) {
        let today = now.date_naive();
        st.refreshed_on = today;
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

    fn show_panel(&self, fields: bool) {
        App::show_panel_in(&self.window, &self.state.borrow(), fields);
    }

    /// Панель выбранной карточки. `fields` — и поля ввода (название, ссылка, описание): не на каждой правке, иначе курсор
    /// прыгал бы в начало, пока печатают.
    fn show_panel_in(w: &MainWindow, st: &State, fields: bool) {
        let key = w.get_selected_key().to_string();
        let Some(t) = st.board.find(&key) else {
            st.notes.set_vec(Vec::new());
            return;
        };
        let today = now().date_naive();
        w.set_panel(panel_data(t, today, &st.settings));
        if fields {
            w.set_title_text(t.title.as_str().into());
            w.set_url_text(t.url.as_str().into());
            w.set_description_text(t.description.as_str().into());
        }
        st.notes.set_vec(
            t.notes
                .iter()
                .map(|n| NoteData {
                    id: n.id.as_str().into(),
                    when: n.created_at.with_timezone(&Local).format("%d.%m %H:%M").to_string().into(),
                    text: n.text.as_str().into(),
                })
                .collect::<Vec<_>>(),
        );
        let syncing = st.syncing.contains(&key);
        w.set_syncing(syncing);
        w.set_can_sync(t.intraservice_id.is_some() && st.client.is_ok());
        let message = if st.sync_message.0 == key { st.sync_message.1.as_str() } else { "" };
        w.set_sync_message(message.into());
        let conversation = if st.conversation.0 == key { st.conversation.1.as_str() } else { "" };
        w.set_conversation(conversation.into());
    }
}

/// Клиент Интрасервиса из настроек: пароль расшифровывается DPAPI; не настроен или не расшифровался — почему.
fn client_for(dir: &Path, settings: &Settings) -> Result<Arc<intraservice::Client>, String> {
    if !settings.has_api() {
        return Err("API не настроен: адрес, логин и пароль — в настройках TicketBoard".into());
    }
    let password = dpapi::unprotect(&settings.password_protected)?;
    let journal_dir = dir.to_path_buf();
    let log = intraservice::Log::new(move |text: &str| journal::append(&journal_dir, text));
    Ok(Arc::new(intraservice::Client::new(&settings.base_url, &settings.login, &password, READ_TIMEOUT).with_log(log)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intraservice::{MyOpen, User, parse};
    use crate::rules::ClosedNames;

    const CARD: &str = r#"{"Task":{"Id":712182,"Name":"Не проводятся упаковки","StatusName":"Ожидание ответа пользователя","Changed":"2026-10-12T09:42:10.85","Creator":"Иванов И. И.","CreatorPosition":"Специалист","CreatorCompanyName":"Отдел поддержки","CreatorPhone":null,"CreatorEmail":"ivanov@example.ru","Executors":"Петров П. П., Иванов И. И.","ExecutorGroup":"ТП склад","ServiceName":"Маркировка","Type":"Запрос на обслуживание","Categories":null,"FileIds":"1, 2","FileNames":"a.xlsx||","Observers":"Орлова К. А.","Description":"<p>Просьба помочь.</p><p>Примеры: 0004</p>"}}"#;
    const LIFE: &str = r#"[{"Comments":"Добрый день!\r\nда, свяжусь.","IsPublic":true,"Date":"2026-10-02T14:07:07","Editor":"Сидорова М. И."},{"Comments":"Свяжитесь с поставщиком","IsPublic":false,"Date":"2026-10-02T14:02:22","Editor":"Иванов И. И.","StatusId":46},{"Date":"2026-10-02T14:00:48","Editor":"Иванов И. И.","StatusId":31}]"#;

    #[test]
    fn the_conversation_as_read() {
        let life = parse::lifetime(LIFE).unwrap();
        assert_eq!(
            conversation_text(&Ok(life)),
            "Свежие сверху: 2 комментария, 1 смена статуса

02.10.2026 14:07 · Сидорова М. И.
Добрый день!
да, свяжусь.

02.10.2026 14:02 · Иванов И. И. · внутренний
Свяжитесь с поставщиком"
        );
        // не прочиталась — одной строкой; есть ещё страницы — сказано
        assert_eq!(conversation_text(&Err("сервер не ответил за 10 с".into())), "Переписка не прочиталась: сервер не ответил за 10 с");
        let more = parse::lifetime(
            r#"{"TaskLifetimes":[{"Date":"2026-10-02T14:00:48","Editor":"И","StatusId":31}],"Paginator":{"Page":1,"PageCount":2}}"#,
        )
        .unwrap();
        assert_eq!(conversation_text(&Ok(more)), "Свежие сверху: 0 комментариев, 1 смена статуса (есть ещё страницы)");
        let anonymous = parse::lifetime(r#"[{"Comments":"без автора и даты","IsPublic":true}]"#).unwrap();
        assert_eq!(conversation_text(&Ok(anonymous)), "Свежие сверху: 1 комментарий, 0 смен статуса\n\n—\nбез автора и даты");
        let _ = CARD;
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
        // ровно десять — все названы, «и ещё» нет
        let ten = MyOpen { rows: found(&(100..110).collect::<Vec<_>>()), total: 10, ..mine.clone() };
        assert!(connection_text(&Ok(()), Some(&ten), &board, 1).ends_with("#108, #109."));
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
    fn links_open_only_on_the_web() {
        assert!(is_web("https://hd/Task/View/1") && is_web(" HTTP://hd/x "));
        assert!(!is_web("file:///C:/Windows/notepad.exe") && !is_web("C:\\Windows\\notepad.exe") && !is_web(""));
        assert!(!is_web("https://hd/\"&calc"), "кавычки и переводы строк — нет");
    }

    #[test]
    fn the_notice_names_the_owner_and_a_settings_problem_first() {
        let dir = Path::new("/data/tb");
        assert_eq!(notice(dir, "", true, None), "Доска — /data/tb; пока открыт прототип, TicketBoard.exe не запустится.");
        assert_eq!(
            notice(dir, "", false, None),
            "TicketBoard.exe открыт — доска только для чтения. Закройте его и запустите прототип снова."
        );
        assert_eq!(
            notice(dir, "", true, Some("tickets.json не прочитан: x")),
            "tickets.json не прочитан: x — доска только для чтения, файл не тронут: TicketBoard.exe отложит его в сторону сам."
        );
        assert!(notice(dir, "settings.json не прочитан: x", true, None).starts_with("settings.json не прочитан: x. Доска —"));
    }
}
