//! Окно и данные: доска (`board::Board`) → модели Slint; выбор, перенос, правки, панель, быстрое добавление, ⟳ из
//! Интрасервиса, «Проверить подключение», цифры. `tickets.json` пишется, только когда доска своя (`single`): правки
//! собираются 600 мс и пишутся разом, как `_saveTimer`. Сеть — в потоках; их ответы идут в очередь (`Message`), окно
//! забирает их в своём потоке (`deliver`) — состояние доски живёт только там.

use crate::board::{AgeState, Board, Date, Filter, Priority, Status, Ticket, delete_question, now, one_line, plural};
use crate::capture::{self, Capture};
use crate::link::LinkParser;
use crate::refresh::{self, IMPORT_TITLE, MOVE_TITLE, NOTHING_TO_REFRESH, READ_ONLY, REFRESH_TITLE};
use crate::settings::{Settings, load_settings};
use crate::store::Store;
use crate::{CaptureWindow, CardData, ColumnData, MainWindow, NoteData, PanelData, clipboard, dpapi, intraservice, journal, metrics, sync};
use chrono::Local;
use slint::{
    ComponentHandle, DataTransfer, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Сколько ждать ответа Интрасервиса — как обычный запрос C#-клиента.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Правки собираются столько и пишутся разом (`_saveTimer`).
pub const SAVE_DELAY: Duration = Duration::from_millis(600);
/// Пауза после выбора карточки перед тихим чтением: стрелками пролистывают — читать каждую незачем (`LookupPanel`).
pub const PANEL_PAUSE: Duration = Duration::from_millis(400);
/// Сколько карточка считается свежей после чтения (⟳, F5, тихое чтение) — `CardFresh`.
pub const CARD_FRESH: Duration = Duration::from_secs(120);
/// Сколько карточек F5 читает разом (`RecheckAsync`).
const REFRESH_PARALLEL: usize = 4;

/// Ответы потоков окну.
enum Message {
    Synced {
        key: String,
        n: i32,
        card: Result<Box<intraservice::Task>, String>,
        life: Option<Result<intraservice::LifetimePage, String>>,
        mark: Mark,
    },
    Title {
        generation: u64,
        result: Result<String, String>,
    },
    Report(String),
    Imported(Box<intraservice::MyOpen>),
    Refreshed(Box<Refreshed>),
    /// Тихое чтение открытой карточки; `mark` — её отметка свежести (не прочиталась — прежняя обратно); `client` — кто
    /// читал: сменили сервер, пока шёл запрос, — ответ не про эту доску.
    QuietRead {
        key: String,
        card: Result<Box<intraservice::Task>, String>,
        mark: Mark,
        client: Arc<intraservice::Client>,
    },
}

/// Ответ F5: карточки (ключ, номер, ответ), справочник статусов, переписка открытой карточки — если её показывали.
struct Refreshed {
    results: Vec<(String, i32, Result<Box<intraservice::Task>, String>)>,
    statuses: Result<Vec<intraservice::Status>, String>,
    life: Option<(String, Result<intraservice::LifetimePage, String>)>,
}

/// Что спрошено в окне поверх доски.
enum Ask {
    Nothing,
    Delete(String),
    MoveClosed(Vec<String>),
}

/// Окно поверх доски: что спрошено, заголовок, текст, действие («Удалить», «Перенести»; пусто — итог) и отказ.
struct Dialog {
    asked: Ask,
    heading: String,
    text: String,
    action: String,
    decline: String,
}

/// Что перерисовать после правки: ничего (ссылка, описание — на карточке их нет), одну карточку, всю доску (перенос,
/// удаление, новая).
#[derive(Clone, Copy)]
enum Scope {
    Nothing,
    Card,
    Board,
}

struct State {
    /// Папка `errors.log` — только когда доска своя: в режиме чтения прототип ничего не пишет.
    journal: Option<PathBuf>,
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
    /// Этапы запуска — отметки от создания процесса (main.rs): где ушло время до окна.
    stages: Vec<(&'static str, Duration)>,
    /// Отбор доски: поле поиска, приоритет, старые «Готово».
    filter: Filter,
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
    /// Что спрошено в окне поверх доски.
    asking: Ask,
    /// Окна, пришедшие, пока открыто другое (итог импорта при вопросе об удалении): по одному, по порядку — ни вопрос, ни
    /// итог не теряются.
    waiting: VecDeque<Dialog>,
    /// Идут импорт и F5: второй раз не запускаются.
    importing: bool,
    refreshing: bool,
    /// Когда карточку читали (⟳, F5, тихое чтение): свежая — тихое чтение её не спрашивает (`CardReadAt`).
    card_read: HashMap<String, Instant>,
    /// Шапка: откуда доска и что с ней; не записалось — почему, впереди.
    notice: String,
    save_problem: String,
}

/// Карточку начали читать (⟳, тихое чтение): когда и какая отметка была до того — не прочиталась, вернётся прежняя.
#[derive(Clone, Copy)]
struct Mark {
    at: Instant,
    was: Option<Instant>,
}

impl State {
    /// Карточку читали меньше `CARD_FRESH` назад (или читают сейчас): тихое чтение её не спросит.
    fn fresh(&self, key: &str) -> bool {
        self.card_read.get(key).is_some_and(|read| read.elapsed() < CARD_FRESH)
    }

    /// Начали читать карточку — свежая уже сейчас: панель, открывшая её, второй раз не читает (`CardReadAt = now`).
    fn mark_read(&mut self, key: &str) -> Mark {
        let at = Instant::now();
        Mark { at, was: self.card_read.insert(key.to_string(), at) }
    }

    /// Не прочиталась — свежей не стала: прежняя отметка обратно, если за это время карточку не прочёл кто-то другой.
    fn unmark_read(&mut self, key: &str, mark: Mark) {
        if self.card_read.get(key) == Some(&mark.at) {
            match mark.was {
                Some(was) => self.card_read.insert(key.to_string(), was),
                None => self.card_read.remove(key),
            };
        }
    }
}

pub struct App {
    pub window: MainWindow,
    pub capture: CaptureWindow,
    state: Rc<RefCell<State>>,
    inbox: Arc<Mutex<Vec<Message>>>,
    save_timer: Rc<Timer>,
    /// Пауза тихого чтения: один таймер — каждый выбор начинает её заново.
    lookup_timer: Rc<Timer>,
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
        let journal = writable.then(|| dir.to_path_buf());
        let client = client_for(journal.as_deref(), &settings);
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
            journal,
            board,
            settings,
            parser,
            store: writable.then_some(store),
            client,
            loaded,
            shown_after: None,
            stages: Vec::new(),
            filter: Filter::default(),
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
            asking: Ask::Nothing,
            waiting: VecDeque::new(),
            importing: false,
            refreshing: false,
            card_read: HashMap::new(),
            notice,
            save_problem: String::new(),
        };
        if !problem.is_empty() {
            log(&st, &problem);
        }
        st.capture.hint = capture::EMPTY_HINT.into();
        let app = App {
            window,
            capture,
            state: Rc::new(RefCell::new(st)),
            inbox: Arc::default(),
            save_timer: Rc::default(),
            lookup_timer: Rc::default(),
        };
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
        st.card_read.clear(); // свежесть — у прежнего сервера
        drop(st);
        self.show_panel(false);
        // открытая карточка — с нового сервера, тихо (`ForgetPanelLookups`)
        let selected = self.window.get_selected_key().to_string();
        if !selected.is_empty() {
            App::look_up(&self.window, &self.state, &self.inbox, &self.lookup_timer, &selected);
        }
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
            parts.push(metrics::startup_text(d, &st.stages));
        }
        parts.push(st.loaded.clone());
        self.window.set_metrics(parts.join(" · ").into());
    }

    /// Отметка этапа запуска (main.rs): сколько прошло от создания процесса к его концу. Нет данных — нет отметки.
    pub fn startup_stage(&self, name: &'static str, at: Option<Duration>) {
        if let Some(at) = at {
            self.state.borrow_mut().stages.push((name, at));
        }
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

    /// Карточку читали меньше двух минут назад или читают сейчас (⟳, F5, тихое чтение): открытие её не перечитывает.
    pub fn fresh(&self, key: &str) -> bool {
        self.state.borrow().fresh(key)
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

        let (weak, state, inbox, lookup) = (self.window.as_weak(), self.state.clone(), self.inbox.clone(), self.lookup_timer.clone());
        self.window.on_select(move |key| {
            let Some(w) = weak.upgrade() else { return };
            w.set_selected_key(key.clone());
            w.set_panel_open(true);
            App::show_panel_in(&w, &state.borrow(), true);
            App::look_up(&w, &state, &inbox, &lookup, &key);
        });

        // название — одной строкой, как у C#: перевод строки (вставка) — пробел, и в поле тоже
        let set_title = App::edit(self, Scope::Card, |st, key, text: String| st.board.find_mut(key).map(|t| t.title = text).is_some());
        let weak = self.window.as_weak();
        self.window.on_edit_title(move |text| {
            let line = one_line(&text);
            if line != text.as_str()
                && let Some(w) = weak.upgrade()
            {
                w.set_title_text(line.as_str().into());
            }
            set_title(line);
        });

        // поиск по доске (`Matches`): не правка — файл не пишется, выбор остаётся
        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_filter_text(move |text| App::filter(&weak, &state, |f| f.text = text.into()));
        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_filter_priority(move |i| {
            let priority = match i {
                1 => Some(Priority::High),
                2 => Some(Priority::Mid),
                3 => Some(Priority::Low),
                _ => None,
            };
            App::filter(&weak, &state, |f| f.priority = priority);
        });
        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_toggle_old_done(move || App::filter(&weak, &state, |f| f.hide_old_done = !f.hide_old_done));
        self.window.on_edit_url(App::edit(self, Scope::Nothing, |st, key, text: SharedString| {
            st.board.find_mut(key).map(|t| t.url = text.into()).is_some()
        }));
        self.window.on_edit_description(App::edit(self, Scope::Nothing, |st, key, text: SharedString| {
            st.board.find_mut(key).map(|t| t.description = text.into()).is_some()
        }));
        self.window.on_add_note(App::edit(self, Scope::Card, |st, key, text: SharedString| {
            st.board.find_mut(key).is_some_and(|t| t.add_note(&text, now()))
        }));
        self.window.on_delete_note(App::edit(self, Scope::Card, |st, key, id: SharedString| {
            st.board.find_mut(key).is_some_and(|t| t.delete_note(&id))
        }));
        self.window.on_move_to(App::edit(self, Scope::Board, |st, key, index: i32| {
            Status::from_index(index as usize).is_some_and(|to| st.board.move_card(key, to, now()))
        }));
        self.window.on_shift(App::edit(self, Scope::Board, |st, key, delta: i32| st.board.shift(key, delta, now())));
        self.window.on_set_priority(App::edit(self, Scope::Card, |st, key, level: i32| {
            let p = Priority::ALL.get(level as usize).copied();
            st.board.find_mut(key).filter(|t| p.is_some_and(|p| t.priority != p)).zip(p).map(|(t, p)| t.priority = p).is_some()
        }));

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
                App::changed(&w, &state, &timer, Scope::Board, &key);
            }
        });

        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_delete_card(move || {
            let Some(w) = weak.upgrade() else { return };
            let key = w.get_selected_key().to_string();
            let mut st = state.borrow_mut();
            let Some(t) = st.board.find(&key).filter(|_| st.store.is_some()) else { return };
            let text = delete_question(t);
            App::ask(&w, &mut st, Ask::Delete(key), "Удалить заявку с доски?", &text, "Удалить", "Отмена");
        });

        // «Удалить» / «Перенести»
        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_dialog_accept(move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_dialog_open(false);
            let asked = std::mem::replace(&mut state.borrow_mut().asking, Ask::Nothing);
            match asked {
                Ask::Delete(key) => {
                    let removed = state.borrow_mut().board.remove(&key).is_some();
                    if removed {
                        w.set_selected_key(SharedString::new());
                        w.set_panel_open(false);
                        App::changed(&w, &state, &timer, Scope::Board, "");
                    }
                }
                Ask::MoveClosed(keys) => {
                    let writable = state.borrow().store.is_some();
                    if writable {
                        refresh::move_closed(&mut state.borrow_mut().board, &keys, now());
                        App::changed(&w, &state, &timer, Scope::Board, "");
                    }
                }
                Ask::Nothing => {}
            }
            App::next_dialog(&w, &mut state.borrow_mut());
        });

        // «Оставить» (у удаления — «Отмена»: ничего)
        let (weak, state, timer) = (self.window.as_weak(), self.state.clone(), self.save_timer.clone());
        self.window.on_dialog_decline(move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_dialog_open(false);
            let asked = std::mem::replace(&mut state.borrow_mut().asking, Ask::Nothing);
            if let Ask::MoveClosed(keys) = asked {
                let kept = {
                    let mut st = state.borrow_mut();
                    st.store.is_some() && refresh::keep_open(&mut st.board, &keys)
                };
                if kept {
                    App::changed(&w, &state, &timer, Scope::Nothing, "");
                }
            }
            App::next_dialog(&w, &mut state.borrow_mut());
        });

        // Esc и «Закрыть»: ничего не решено
        let (weak, state) = (self.window.as_weak(), self.state.clone());
        self.window.on_dialog_close(move || {
            let Some(w) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            st.asking = Ask::Nothing;
            w.set_dialog_open(false);
            App::next_dialog(&w, &mut st);
        });

        let (weak, state, inbox) = (self.window.as_weak(), self.state.clone(), self.inbox.clone());
        self.window.on_import_mine(move || {
            if let Some(w) = weak.upgrade() {
                App::import_mine(&w, &state, &inbox);
            }
        });

        let (weak, state, inbox) = (self.window.as_weak(), self.state.clone(), self.inbox.clone());
        self.window.on_refresh_all(move || {
            if let Some(w) = weak.upgrade() {
                App::refresh_all(&w, &state, &inbox);
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
            App::changed(&w, &state, &timer, Scope::Board, &key);
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
                w.set_reading(false);
                App::tell(w, &mut state.borrow_mut(), "Подключение к Интрасервису", &text);
            }
            Message::Imported(mine) => {
                let report = {
                    let mut st = state.borrow_mut();
                    st.importing = false;
                    let st = &mut *st;
                    st.store.is_some().then(|| refresh::import(&mut st.board, &mine, &st.settings, now()))
                };
                w.set_importing(false);
                let Some(r) = report else { return };
                if !r.added.is_empty() {
                    App::changed(w, state, timer, Scope::Board, "");
                }
                App::tell(w, &mut state.borrow_mut(), IMPORT_TITLE, &r.text);
            }
            Message::Refreshed(r) => App::refreshed(w, state, timer, *r),
            Message::QuietRead { key, card, mark, client } => {
                let changed = {
                    let mut st = state.borrow_mut();
                    let current = st.client.as_ref().is_ok_and(|c| Arc::ptr_eq(c, &client));
                    let writable = st.store.is_some() && current;
                    match &card {
                        Ok(task) => st.board.find_mut(&key).filter(|_| writable).is_some_and(|t| {
                            let before = t.clone();
                            sync::apply_card(t, task);
                            *t != before
                        }),
                        Err(_) => {
                            st.unmark_read(&key, mark); // откроют снова — спросим снова
                            false
                        }
                    }
                };
                if changed {
                    App::changed(w, state, timer, Scope::Card, &key);
                }
            }
            Message::Synced { key, n, card, life, mark } => {
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
                    let applied = match (&card, st.board.find_mut(&key)) {
                        (Ok(task), Some(t)) if writable => {
                            sync::apply(t, n, task, now());
                            true
                        }
                        _ => false,
                    };
                    if applied {
                        st.card_read.insert(key.clone(), Instant::now());
                    } else if card.is_err() {
                        st.unmark_read(&key, mark);
                    }
                    applied
                };
                if applied {
                    App::changed(w, state, timer, Scope::Card, &key);
                }
                if w.get_selected_key() == key.as_str() {
                    App::show_panel_in(w, &state.borrow(), applied);
                }
            }
        }
    }

    /// Итог поверх доски (`AskWindow.Tell`): заголовок, текст, «Закрыть».
    fn tell(w: &MainWindow, st: &mut State, heading: &str, text: &str) {
        App::ask(w, st, Ask::Nothing, heading, text, "", "");
    }

    /// Вопрос поверх доски (`AskWindow.Ask`): действие и отказ; Esc — ничего не решено. Открыто другое окно — этот ждёт
    /// своей очереди (`next_dialog`).
    fn ask(w: &MainWindow, st: &mut State, asked: Ask, heading: &str, text: &str, action: &str, decline: &str) {
        let d = Dialog { asked, heading: heading.into(), text: text.into(), action: action.into(), decline: decline.into() };
        if w.get_dialog_open() {
            st.waiting.push_back(d);
        } else {
            App::show_dialog(w, st, d);
        }
    }

    fn show_dialog(w: &MainWindow, st: &mut State, d: Dialog) {
        st.asking = d.asked;
        w.set_dialog_heading(d.heading.into());
        w.set_dialog_text(d.text.into());
        w.set_dialog_action(d.action.into());
        w.set_dialog_decline_text(d.decline.into());
        w.set_dialog_open(true);
    }

    /// Окно закрыли — следующее, если ждёт.
    fn next_dialog(w: &MainWindow, st: &mut State) {
        if let Some(d) = st.waiting.pop_front() {
            App::show_dialog(w, st, d);
        }
    }

    /// Импорт моих (`ImportMine`): только своя доска (чужая — так и сказано: трей и F5 доступны и у неё) и не второй раз;
    /// без API — так и сказано; ответ — `receive`.
    fn import_mine(w: &MainWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Message>>>) {
        let mut st = state.borrow_mut();
        if st.store.is_none() {
            return App::tell(w, &mut st, IMPORT_TITLE, READ_ONLY);
        }
        if st.importing {
            return;
        }
        let client = match st.client.clone() {
            Ok(client) => client,
            Err(why) => return App::tell(w, &mut st, IMPORT_TITLE, &why),
        };
        st.importing = true;
        w.set_importing(true);
        let closed = st.settings.closed_names();
        let (weak, inbox) = (w.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let mine = client.my_open(&closed);
            inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::Imported(Box::new(mine)));
            let _ = weak.upgrade_in_event_loop(|w| w.invoke_deliver());
        });
    }

    /// F5 (`RefreshAll`): карточки с номером, кроме «Готово», — по четыре разом, справочник — параллельно; у открытой, если
    /// в панели её переписка, — и переписка. Ответ — `refreshed`.
    fn refresh_all(w: &MainWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Message>>>) {
        let mut st = state.borrow_mut();
        if st.store.is_none() {
            return App::tell(w, &mut st, REFRESH_TITLE, READ_ONLY);
        }
        if st.refreshing {
            return;
        }
        let client = match st.client.clone() {
            Ok(client) => client,
            Err(why) => return App::tell(w, &mut st, REFRESH_TITLE, &why),
        };
        let cards = refresh::to_refresh(&st.board);
        if cards.is_empty() {
            return App::tell(w, &mut st, REFRESH_TITLE, NOTHING_TO_REFRESH);
        }
        st.refreshing = true;
        w.set_refreshing(true);
        let selected = w.get_selected_key().to_string();
        let open = (st.conversation.0 == selected).then_some(selected);
        let (weak, inbox) = (w.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let statuses = {
                let client = client.clone();
                std::thread::spawn(move || client.get_statuses())
            };
            let queue = Mutex::new(cards.into_iter().enumerate());
            let results = Mutex::new(Vec::new());
            std::thread::scope(|s| {
                for _ in 0..REFRESH_PARALLEL {
                    s.spawn(|| {
                        loop {
                            let next = queue.lock().unwrap_or_else(|e| e.into_inner()).next();
                            let Some((i, (key, n))) = next else { break };
                            let card = client.get_task(n).map(Box::new);
                            results.lock().unwrap_or_else(|e| e.into_inner()).push((i, key, n, card));
                        }
                    });
                }
            });
            // в порядке доски, а не ответов: «первая ошибка» от раза к разу та же
            let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());
            results.sort_by_key(|r| r.0);
            let results: Vec<_> = results.into_iter().map(|(_, key, n, card)| (key, n, card)).collect();
            let life = open.and_then(|key| {
                let (_, n, card) = results.iter().find(|(k, ..)| *k == key)?;
                let life = card.is_ok().then(|| client.get_lifetime_page(*n, 1))?;
                Some((key, life))
            });
            let statuses = statuses.join().unwrap_or_else(|_| Err("справочник статусов не прочитан".into()));
            let answer = Refreshed { results, statuses, life };
            inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::Refreshed(Box::new(answer)));
            let _ = weak.upgrade_in_event_loop(|w| w.invoke_deliver());
        });
    }

    /// Ответ F5: `Apply` к каждой прочитанной, итог; закрытые — вопрос «Перенести закрытые в «Готово»?».
    fn refreshed(w: &MainWindow, state: &Rc<RefCell<State>>, timer: &Rc<Timer>, r: Refreshed) {
        let (fresh, text, to_move) = {
            let mut st = state.borrow_mut();
            st.refreshing = false;
            let (now, writable) = (now(), st.store.is_some());
            let (mut fresh, mut failed) = (Vec::new(), Vec::new());
            for (key, n, card) in &r.results {
                match card {
                    Ok(task) => {
                        if let Some(t) = st.board.find_mut(key).filter(|_| writable) {
                            sync::apply(t, *n, task, now);
                            fresh.push(key.clone());
                        }
                    }
                    Err(e) => failed.push((*n, e.clone())),
                }
            }
            for key in &fresh {
                st.card_read.insert(key.clone(), Instant::now());
            }
            if let Some((key, life)) = &r.life {
                st.conversation = (key.clone(), conversation_text(life));
            }
            let closed = refresh::closed_names(&st.settings, &r.statuses);
            let status_error = r.statuses.as_ref().err().map(String::as_str);
            let to_move = refresh::closed_to_move(&st.board, &fresh, &closed);
            // «Обновлено» — прочитанные, и удалённую за это время тоже (как у C#): иначе «обновлено» и «не удалось» не сходятся
            // с тем, что читали
            let read = r.results.iter().filter(|(.., card)| card.is_ok()).count();
            // в окне без вопроса — ошибки целиком; в вопросе — одной строкой: там главное — список
            let text = if to_move.is_empty() {
                refresh::summary(read, &failed, status_error, |e| e.to_string())
            } else {
                let preface = format!("{}\n\n", refresh::summary(read, &failed, status_error, intraservice::brief));
                refresh::move_question(&st.board, &to_move, &preface)
            };
            (fresh, text, to_move)
        };
        w.set_refreshing(false);
        // F5 карточки не переносит — перерисовать только перечитанные: вся доска пересоздала бы карточки, а под курсором может
        // быть перенос
        for key in &fresh {
            App::changed(w, state, timer, Scope::Card, key);
        }
        let mut st = state.borrow_mut();
        if to_move.is_empty() {
            App::tell(w, &mut st, REFRESH_TITLE, &text);
        } else {
            App::ask(w, &mut st, Ask::MoveClosed(to_move), MOVE_TITLE, &text, "Перенести", "Оставить");
        }
    }

    /// Тихое чтение открытой карточки (`LookupPanel`): через паузу — каждый выбор начинает её заново (таймер один, как
    /// `_panelLookup?.Cancel()`), — если карточка всё ещё выбрана (выбор мог смениться и без этого: добавили новую), доска
    /// своя, номер и API есть и карточка не свежая. Берутся люди, поля и вложения (`ApplyCard`), статус — нет; в панель —
    /// ни слова.
    fn look_up(w: &MainWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Message>>>, timer: &Timer, key: &str) {
        let (weak, state, inbox, key) = (w.as_weak(), state.clone(), inbox.clone(), key.to_string());
        timer.start(TimerMode::SingleShot, PANEL_PAUSE, move || {
            let Some(w) = weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            if w.get_selected_key() != key.as_str() || st.store.is_none() {
                return;
            }
            let Some(n) = st.board.find(&key).and_then(|t| t.intraservice_id) else { return };
            let Ok(client) = st.client.clone() else { return };
            if st.fresh(&key) {
                return;
            }
            let mark = st.mark_read(&key);
            let (weak, inbox, key) = (w.as_weak(), inbox.clone(), key.clone());
            std::thread::spawn(move || {
                let card = client.get_task(n).map(Box::new);
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::QuietRead { key, card, mark, client });
                let _ = weak.upgrade_in_event_loop(|w| w.invoke_deliver());
            });
        });
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
                let mark = st.mark_read(key);
                let (weak, inbox, key) = (w.as_weak(), inbox.clone(), key.to_string());
                std::thread::spawn(move || {
                    let card = client.get_task(n).map(Box::new);
                    let life = card.is_ok().then(|| client.get_lifetime_page(n, 1));
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(Message::Synced { key, n, card, life, mark });
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

    /// Правка выбранной карточки из окна: только своя доска; изменилось — перерисовать `scope` и записать через паузу.
    fn edit<A: 'static>(app: &App, scope: Scope, f: impl Fn(&mut State, &str, A) -> bool + 'static) -> impl Fn(A) + 'static {
        let (weak, state, timer) = (app.window.as_weak(), app.state.clone(), app.save_timer.clone());
        move |arg: A| {
            let Some(w) = weak.upgrade() else { return };
            let key = w.get_selected_key().to_string();
            let changed = {
                let mut st = state.borrow_mut();
                st.store.is_some() && f(&mut st, &key, arg)
            };
            if changed {
                App::changed(&w, &state, &timer, scope, &key);
            }
        }
    }

    /// Доска изменилась: перерисовать, что задело (`scope`; панель — если задета карточка или доска), записать — через
    /// паузу, одной записью на серию правок.
    fn changed(w: &MainWindow, state: &Rc<RefCell<State>>, timer: &Rc<Timer>, scope: Scope, key: &str) {
        {
            let mut st = state.borrow_mut();
            st.dirty = true;
            match scope {
                Scope::Nothing => {}
                Scope::Card => App::refresh_card(&st, key, now()),
                Scope::Board => App::refresh_in(&mut st, now()),
            }
        }
        if !matches!(scope, Scope::Nothing) {
            App::show_panel_in(w, &state.borrow(), false);
        }
        let (weak, state) = (w.as_weak(), state.clone());
        timer.start(TimerMode::SingleShot, SAVE_DELAY, move || {
            if let Some(w) = weak.upgrade() {
                App::save_in(&w, &mut state.borrow_mut());
            }
        });
    }

    /// Одна карточка на доске заново (название, приоритет, заметки, поля с сервера): остальные модели не трогаются — под
    /// курсором может быть перенос.
    fn refresh_card(st: &State, key: &str, now: Date) {
        let Some(t) = st.board.find(key) else { return };
        let model = &st.cards[t.status.index()];
        if let Some(row) = (0..model.row_count()).find(|&i| model.row_data(i).is_some_and(|c| c.key == key)) {
            model.set_row_data(row, card_data(t, now.date_naive(), &st.settings));
        }
    }

    /// Записать доску, если есть что: копия за день, потом файл. Не вышло — в шапке и в журнале; правки остаются и
    /// запишутся со следующей.
    fn save_in(w: &MainWindow, st: &mut State) {
        let Some(store) = &st.store else { return };
        if !st.dirty {
            return;
        }
        if let Err(e) = store.backup(now().date_naive()) {
            log(st, &e);
        }
        let problem = match store.write(&st.board) {
            Ok(()) => {
                st.dirty = false;
                String::new()
            }
            Err(e) => {
                log(st, &e);
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

    /// Сменился отбор — колонки заново (модели те же: прокрутка не сбрасывается).
    fn filter(weak: &slint::Weak<MainWindow>, state: &Rc<RefCell<State>>, change: impl FnOnce(&mut Filter)) {
        if weak.upgrade().is_none() {
            return;
        }
        let mut st = state.borrow_mut();
        change(&mut st.filter);
        App::refresh_in(&mut st, now());
    }

    /// Колонки и карточки — заново из доски на момент `now`. Модели те же: прокрутка колонок не сбрасывается.
    fn refresh_in(st: &mut State, now: Date) {
        let today = now.date_naive();
        st.refreshed_on = today;
        let rules = st.settings.rules();
        for status in Status::ALL {
            let cards: Vec<CardData> =
                st.board.visible(status, now, rules, &st.filter).iter().map(|t| card_data(t, today, &st.settings)).collect();
            let count = cards.len() as i32;
            st.cards[status.index()].set_vec(cards);
            let column = ColumnData {
                title: status.title().into(),
                count,
                hint: Board::hint(status, rules, &st.filter).into(),
                overloaded: st.board.overloaded(status, now, rules, &st.filter),
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
        w.set_panel_status(t.status.index() as i32);
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

/// Клиент Интрасервиса из настроек: пароль расшифровывается DPAPI; не настроен или не расшифровался — почему. Ответы,
/// которые не разобрались, — в `errors.log`, когда доска своя.
fn client_for(journal: Option<&Path>, settings: &Settings) -> Result<Arc<intraservice::Client>, String> {
    if !settings.has_api() {
        return Err("API не настроен: адрес, логин и пароль — в настройках TicketBoard".into());
    }
    let password = dpapi::unprotect(&settings.password_protected)?;
    let client = intraservice::Client::new(&settings.base_url, &settings.login, &password, READ_TIMEOUT);
    Ok(Arc::new(match journal {
        Some(dir) => {
            let dir = dir.to_path_buf();
            client.with_log(intraservice::Log::new(move |text: &str| journal::append(&dir, text)))
        }
        None => client,
    }))
}

/// В `errors.log`, если доска своя.
fn log(st: &State, text: &str) {
    if let Some(dir) = &st.journal {
        journal::append(dir, text);
    }
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
