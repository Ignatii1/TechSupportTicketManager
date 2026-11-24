//! Окно «Поиск в Интрасервисе» (`SearchWindow.xaml(.cs)` + `SearchViewModel`): условия → запрос (`search::resolve`) →
//! страницы списка → просмотр выбранной → «+ На доску». Правила — в `search.rs`; здесь — окно, потоки и доска. Ответ
//! потока, пришедший после нового поиска или смены клиента, отбрасывается (`generation`, как `_lookup` у C#).

use super::*;
use crate::export;
use crate::intraservice::Ref;
use crate::search::{self, Resolved, Results, SearchFilter, SearchStatus};
use crate::{FoundRow, SearchWindow};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// «+ На доску» на доске только для чтения.
pub const ADD_READ_ONLY: &str = "Доска только для чтения (почему — в шапке доски): добавить на неё заявку нельзя";

/// Что выгружать: выбранные строки или всё найденное по условиям последнего поиска (предел; сколько найдено — для итога).
enum Work {
    Rows(Vec<intraservice::Found>),
    Found { query: Box<intraservice::query::TaskQuery>, limit: i32, found: i32 },
}

/// Выгрузка найденного, что ждёт ответа на вопрос: запрос, предел, найдено, папка.
#[derive(Clone)]
struct Pending {
    query: intraservice::query::TaskQuery,
    limit: i32,
    found: i32,
    dir: String,
}

/// Состояние окна поиска: живёт, пока жива программа (окно только прячется).
#[derive(Default)]
pub(super) struct Finder {
    /// Условия последнего удавшегося поиска человека (`LastSearch`, только в памяти): их возвращает открытие окна после
    /// быстрого поиска; None — ещё не искали, тогда «мои закрытые».
    last: Option<SearchFilter>,
    /// На экране быстрый поиск с доски (`_quickMode`): спрятанное окно откроется с `last`.
    quick: bool,
    /// Условия идущего поиска и начат ли он человеком: удастся — «условия изменены» сверяется с ними (`IsStale`), а поиск
    /// человека станет `last` и закончит быстрый (`remember`).
    searched: Option<(SearchFilter, bool)>,
    /// Номер поиска: новый поиск, «ещё» и смена клиента его меняют — ответы с прежним номером не нужны.
    generation: u64,
    resolved: Option<Resolved>,
    results: Results,
    busy: bool,
    message: String,
    notes: String,
    /// Условия поменяли после поиска — список ещё по прежним.
    stale: bool,
    /// Строка в просмотре (`SetSelection`: только что выбранная или последняя из выбранных).
    selected: Option<usize>,
    /// Выбранные строки в порядке выбора (`SelectedItems`): «Копировать» и «Выгрузить выбранные» — о них.
    picked: Vec<usize>,
    /// От какой строки Shift выбирает подряд: последняя, по которой щёлкнули без Shift.
    anchor: Option<usize>,
    /// «Выгрузить выбранные» или «найденные» идёт: её «Остановить» и номер (ответ принимает только своя).
    exporting: bool,
    /// Вопрос перед большой выгрузкой найденного (`Confirm`): заголовок, текст и что начнётся по «Выгрузить».
    asking: Option<(String, String, Pending)>,
    export_stop: Arc<AtomicBool>,
    /// «Остановить» и смена клиента обрывают и копирование: оставшиеся заявки уже не читаются (`_work?.Cancel()`).
    copy_stop: Arc<AtomicBool>,
    export_generation: u64,
    preview_title: String,
    preview_text: String,
    preview_busy: bool,
    preview_generation: u64,
    /// Текст в просмотре — заявка, её «изменена» по строке списка и когда прочитан: «Копировать» её же не читает снова, пока
    /// она не менялась и текст не старше срока (кэш текстов C#, `CachedText`, — здесь только показанный).
    shown: Option<(i32, Option<intraservice::Date>, String, Instant)>,
    /// Сколько показанный текст годится для «Копировать» (`TextTtl` у C#, 3 мин): список мог устареть, а с ним и «изменена»
    /// строки. None — по умолчанию; проверки окна ставят свой (`App::use_text_ttl`).
    text_ttl: Option<Duration>,
    /// Кто я — для роли в тексте (`MeAsync` у C#): прочитан раз для клиента; не прочитался — спросим при следующем тексте.
    me: Option<intraservice::User>,
    /// «Копировать»: идёт, номер (смена клиента его меняет — ответ прежнего сервера в буфер не кладём), ход и итог.
    copying: bool,
    copy_generation: u64,
    work_message: String,
    /// Справочник статусов — для «открытых», «закрытых» и выбора одного; читается при первом показе окна (или первым
    /// таким поиском).
    statuses: Option<Vec<intraservice::Status>>,
    /// Справочники окна (`ReadReferencesAsync`): None — ещё не прочитан или не прочитался (следующий показ спросит снова).
    services: Option<Vec<Ref>>,
    /// Сервисы — не весь справочник (назначенные, из заявок): подписи без отступов, вложенные могут не все войти.
    services_partial: bool,
    /// Под полем сервиса: какой это список или почему он пуст.
    service_note: String,
    types: Option<Vec<Ref>>,
    filters: Option<Vec<Ref>>,
    /// Какие справочники не загрузились и почему («статусы», «типы», «сохранённые фильтры») — строка над условиями.
    failures: Vec<(&'static str, String)>,
    /// Справочники читаются — второй раз не спрашивать.
    references_reading: bool,
    /// Номер чтения справочников: смена клиента его меняет — ответ прежнего сервера не нужен.
    references_generation: u64,
}

impl Finder {
    /// Справочник `name` прочитан (None) или нет (почему) — строка над условиями (`Notes` у C#) следует за этим.
    fn failed(&mut self, name: &'static str, why: Option<String>) {
        self.failures.retain(|(n, _)| *n != name);
        // порядок C# (статусы, типы, сохранённые фильтры) — порядок разбора ответа: каждый не прочитанный спрашивается
        // снова и разбирается заново, так что строка всегда собирается в этом порядке
        if let Some(why) = why {
            self.failures.push((name, why));
        }
    }

    fn reference_notes(&self) -> String {
        if self.failures.is_empty() {
            return String::new();
        }
        let list: Vec<String> = self.failures.iter().map(|(n, why)| format!("{n} ({why})")).collect();
        format!("Не загрузились: {}. Поиск работает и без них.", list.join(", "))
    }

    /// Начать заново: найденное и просмотр забыты, ответы идущих поиска, «ещё» и просмотра — уже не нужны.
    fn start_over(&mut self) {
        next(&mut self.generation);
        next(&mut self.preview_generation);
        self.results = Results::default();
        self.resolved = None;
        self.busy = false;
        self.message.clear();
        self.notes.clear();
        self.stale = false;
        self.selected = None;
        self.picked.clear();
        self.anchor = None;
        self.preview_title.clear();
        self.preview_text.clear();
        self.preview_busy = false;
        self.shown = None;
    }
}

/// Больше заявок в буфер не кладём (`MaxCopy`): в чат больше не поместится.
const MAX_COPY: usize = 30;

/// Доска только для чтения — прототип ничего не пишет, и файлы базы знаний тоже.
pub const EXPORT_READ_ONLY: &str =
    "Доска только для чтения: выгрузка в файлы — только когда доску ведёт прототип (он ничего не пишет, пока её держит TicketBoard.exe).";

/// Итог «Копировать» (`Copy` у C#): сколько заявок легло и сколько знаков (`Length` — в UTF-16), не прочитавшиеся —
/// сколько и первая; не легло ни одной — «Ничего не скопировано»; буфер занят — нажать ещё раз.
fn summary(put: bool, texts: &[String], errors: &[String]) -> String {
    let failed = errors.first().map_or(String::new(), |first| format!("\nНе прочитались: {}. Первая ошибка — {first}", errors.len()));
    if texts.is_empty() {
        return format!("Ничего не скопировано.{failed}");
    }
    if !put {
        return "Буфер обмена занят другой программой — нажмите «Копировать» ещё раз.".into();
    }
    let chars = texts.iter().map(|t| t.encode_utf16().count()).sum::<usize>() + 2 * (texts.len() - 1);
    let (n, chars) = (i32::try_from(texts.len()).unwrap_or(i32::MAX), i32::try_from(chars).unwrap_or(i32::MAX));
    format!("В буфере обмена — заявок: {}, {} символов. Вставьте в чат.{failed}", search::thousands(n), search::thousands(chars))
}

/// Тексты — в буфер обмена одним куском, через пустую строку, и итог — в окно. Запись идёт без занятого состояния:
/// писатель (буфер Windows с повторами или подмена в проверках) не держит окно и может к нему обратиться.
fn put_and_tell(state: &Rc<RefCell<State>>, texts: &[String], errors: &[String]) {
    let clipboard = state.borrow().clipboard.clone();
    let put = !texts.is_empty() && clipboard(&texts.join("\n\n"));
    state.borrow_mut().finder.work_message = summary(put, texts, errors);
}

/// Сколько показанный текст годится для «Копировать» (`TextTtl` у C#): дольше — список мог устареть вместе с «изменена».
const TEXT_TTL: Duration = Duration::from_secs(180);

/// Текст заявки для агента — что нужно потоку (`TextOfAsync` и `ContextAsync` у C#): клиент, строка, ссылка, закрытые
/// названия из настроек и «я», если уже знаем.
struct TextJob {
    client: Arc<intraservice::Client>,
    row: intraservice::Found,
    url: String,
    closed: crate::rules::ClosedNames,
    me: Option<intraservice::User>,
}

impl TextJob {
    fn of(st: &State, client: Arc<intraservice::Client>, row: intraservice::Found) -> TextJob {
        TextJob { url: st.settings.ticket_url(row.id), closed: st.settings.closed_names(), me: st.finder.me.clone(), client, row }
    }

    /// В потоке: «я» не знаем — спросить (не вышло — текст без роли, не ошибка), потом текст.
    fn run(self) -> (Option<intraservice::User>, Result<String, String>) {
        let me = self.me.clone().or_else(|| self.client.get_current_user().ok());
        let text = self.text(&me);
        (me, text)
    }

    /// Текст при уже известном «я» (пачка заявок спрашивает «кто я» один раз).
    fn text(&self, me: &Option<intraservice::User>) -> Result<String, String> {
        let context = export::Context { me: me.clone(), closed: self.closed.clone() };
        export::build(&self.client, &self.row, &self.url, &context, &crate::board::now())
    }
}

/// Следующий номер: всегда новый (только растёт) — ответ на любой прежний запрос с ним не совпадёт. Одно место, где
/// номера меняются, — его и ловят проверки устаревших ответов.
fn next(n: &mut u64) {
    *n += 1;
}

/// Ответ потока окна поиска.
pub(super) enum Found {
    /// Справочники, которых не было (None — не спрашивали): статусы, сервисы, типы, сохранённые фильтры.
    References {
        generation: u64,
        statuses: Option<Result<Vec<intraservice::Status>, String>>,
        services: Option<intraservice::Services>,
        types: Option<Result<Vec<Ref>, String>>,
        filters: Option<Result<Vec<Ref>, String>>,
    },
    /// Поиск: справочник (если читали), условия → запрос или ошибка, первая страница. «Ещё»: только страница.
    Page {
        generation: u64,
        statuses: Option<Result<Vec<intraservice::Status>, String>>,
        resolved: Option<Result<Box<Resolved>, String>>,
        page: Option<Result<intraservice::SearchPage, String>>,
    },
    /// Просмотр выбранной: «я» (если спрашивали) и её текст для агента — или почему его нет.
    Preview { generation: u64, id: i32, changed: Option<intraservice::Date>, me: Option<intraservice::User>, text: Result<String, String> },
    /// «Копировать»: «я» и текст выбранной заявки — или почему его нет.
    Copied { generation: u64, me: Option<intraservice::User>, texts: Vec<String>, errors: Vec<String> },
    /// «Копировать»: готово столько из стольких.
    Copying { generation: u64, done: usize, total: usize },
    /// «Выгрузить выбранные»: строка хода и итог (или почему не начата).
    Exporting { generation: u64, line: String },
    /// `client` — номер клиента (`references_generation`) на старте: «кто я» прежнего сервера новому не нужен.
    Exported { generation: u64, client: u64, me: Option<intraservice::User>, summary: String },
}

/// Условия из окна; выбор в списках — номерами справочников (`Finder`).
fn form_of(w: &SearchWindow, f: &Finder) -> SearchFilter {
    let pick = |list: &Option<Vec<Ref>>, index: i32| -> i32 {
        usize::try_from(index - 1).ok().and_then(|i| list.as_ref()?.get(i)).map_or(0, |r| r.id)
    };
    let (status, status_id) = match w.get_status_index() {
        1 => (SearchStatus::Closed, 0),
        2 => (SearchStatus::Open, 0),
        i => match usize::try_from(i - 3).ok().and_then(|i| f.statuses.as_ref()?.get(i)) {
            Some(s) => (SearchStatus::One, s.id),
            None => (SearchStatus::Any, 0),
        },
    };
    SearchFilter {
        words: w.get_words().into(),
        mine: w.get_mine(),
        executor: w.get_executor().into(),
        creator: w.get_creator().into(),
        observer_mine: w.get_observer_mine(),
        observer: w.get_observer().into(),
        status,
        status_id,
        service_id: pick(&f.services, w.get_service_index()),
        with_children: w.get_with_children(),
        type_id: pick(&f.types, w.get_type_index()),
        created_from: w.get_created_from().into(),
        created_to: w.get_created_to().into(),
        changed_from: w.get_changed_from().into(),
        changed_to: w.get_changed_to().into(),
        closed_from: w.get_closed_from().into(),
        closed_to: w.get_closed_to().into(),
        saved_filter_id: pick(&f.filters, w.get_filter_index()),
        include_archived: w.get_archived(),
    }
}

/// Условия — в окно; чего нет в списке (не загрузился, нет такого) — «любой» (`SelectWanted`).
fn set_form(w: &SearchWindow, filter: &SearchFilter, f: &Finder) {
    let place = |list: &Option<Vec<Ref>>, id: i32| -> i32 {
        list.as_ref().and_then(|l| l.iter().position(|r| r.id == id && id > 0)).map_or(0, |i| i as i32 + 1)
    };
    w.set_words(filter.words.as_str().into());
    w.set_mine(filter.mine);
    w.set_executor(filter.executor.as_str().into());
    w.set_creator(filter.creator.as_str().into());
    w.set_observer_mine(filter.observer_mine);
    w.set_observer(filter.observer.as_str().into());
    w.set_status_index(match filter.status {
        SearchStatus::Any => 0,
        SearchStatus::Closed => 1,
        SearchStatus::Open => 2,
        SearchStatus::One => f.statuses.as_ref().and_then(|l| l.iter().position(|s| s.id == filter.status_id)).map_or(0, |i| i as i32 + 3),
    });
    w.set_service_index(place(&f.services, filter.service_id));
    w.set_with_children(filter.with_children);
    w.set_type_index(place(&f.types, filter.type_id));
    w.set_filter_index(place(&f.filters, filter.saved_filter_id));
    w.set_created_from(filter.created_from.as_str().into());
    w.set_created_to(filter.created_to.as_str().into());
    w.set_changed_from(filter.changed_from.as_str().into());
    w.set_changed_to(filter.changed_to.as_str().into());
    w.set_closed_from(filter.closed_from.as_str().into());
    w.set_closed_to(filter.closed_to.as_str().into());
    w.set_archived(filter.include_archived);
}

/// Списки выбора из справочников (`FillChoices`): непрочитанные — только «любой» / «нет».
fn show_choices(w: &SearchWindow, f: &Finder) {
    let model = |items: Vec<String>| ModelRc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>()));
    let mut statuses: Vec<String> = ["Любой", "Закрытые", "Открытые"].map(String::from).to_vec();
    statuses.extend(f.statuses.iter().flatten().map(|s| s.name.clone()));
    w.set_status_choices(model(statuses));
    let mut services = vec!["— любой —".to_string()];
    // неполный список: родителя в нём может не быть — без отступов, иначе вложенный выглядел бы вложенным в соседа
    services.extend(
        f.services
            .iter()
            .flatten()
            .map(|s| if f.services_partial { search::service_label(&Ref { path: None, ..s.clone() }) } else { search::service_label(s) }),
    );
    w.set_service_choices(model(services));
    let mut types = vec!["— любой —".to_string()];
    types.extend(f.types.iter().flatten().map(search::type_label));
    w.set_type_choices(model(types));
    let mut filters = vec!["— нет —".to_string()];
    filters.extend(f.filters.iter().flatten().map(search::filter_label));
    w.set_filter_choices(model(filters));
    w.set_service_note(f.service_note.as_str().into());
    w.set_reference_notes(f.reference_notes().as_str().into());
}

/// Строки найденного для окна: уже на доске — приглушены (`OnBoard`).
fn rows_of(results: &Results, on_board: &HashSet<i32>, picked: &[usize]) -> Vec<FoundRow> {
    results
        .rows
        .iter()
        .enumerate()
        .map(|(i, f)| FoundRow {
            id: f.id,
            number: format!("#{}", f.id).into(),
            title: f.name.as_str().into(),
            status: f.status.as_str().into(),
            service: search::service_of(f).into(),
            details: search::details_of(f).into(),
            on_board: on_board.contains(&f.id),
            picked: picked.contains(&i),
        })
        .collect()
}

impl App {
    /// Показать окно поиска. С доски (`words`) — слова из поля, остальное по умолчанию, поиск сразу; кнопкой и из трея —
    /// окно как было, а спрятанное после быстрого поиска — с прежними условиями (`ShowSearch`/`OpenAsync(restoreQuick)`).
    fn show_search_in(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, words: Option<&str>) {
        {
            let mut st = state.borrow_mut();
            // папка выгрузки — из настроек (`KnowledgePath`); правка в окне живёт до выхода (настройки прототип не пишет)
            if s.get_folder().trim().is_empty() {
                s.set_folder(st.settings.knowledge_path(&st.data_dir).to_string_lossy().as_ref().into());
            }
            match words {
                Some(words) => {
                    st.finder.quick = true;
                    set_form(s, &SearchFilter::quick(words), &st.finder);
                }
                // показанное окно только выходит вперёд; спрятанное человеком после быстрого поиска — с условиями последнего
                // поиска человека (`OpenAsync(restoreQuick: reopened)`)
                None if st.finder.quick && !s.window().is_visible() => {
                    st.finder.quick = false;
                    let last = st.finder.last.clone().unwrap_or_else(SearchFilter::usual);
                    set_form(s, &last, &st.finder);
                    st.finder.start_over();
                }
                None => {}
            }
        }
        let _ = s.show();
        s.invoke_focus_words();
        // справочники — в фоне: быстрый поиск их не ждёт (`StartWithAsync`), а окно спросит только недостающие
        App::read_references(s, state, inbox);
        if words.is_some() {
            App::find(s, state, inbox, false);
        } else {
            App::show_finder_in(s, &state.borrow());
        }
    }

    /// Справочники окна, которых нет (`EnsureReferencesAsync`): все сразу, каждый своим потоком; ответ — `References`.
    fn read_references(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>) {
        let ready = {
            let mut st = state.borrow_mut();
            let st = &mut *st;
            let f = &mut st.finder;
            let missing = f.statuses.is_none() || f.services.is_none() || f.types.is_none() || f.filters.is_none();
            match (&st.client, missing && !f.references_reading) {
                (Ok(client), true) => {
                    f.references_reading = true;
                    let ask = (f.statuses.is_none(), f.services.is_none(), f.types.is_none(), f.filters.is_none());
                    Some((client.clone(), f.references_generation, ask))
                }
                _ => None,
            }
        };
        let Some((client, generation, (statuses, services, types, filters))) = ready else { return };
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let c = &client;
            let found = std::thread::scope(|scope| {
                let statuses = statuses.then(|| scope.spawn(|| c.get_statuses()));
                let services = services.then(|| scope.spawn(|| c.get_services()));
                let types = types.then(|| scope.spawn(|| c.get_task_types()));
                let filters = filters.then(|| scope.spawn(|| c.get_saved_filters()));
                // поток упал — как отказ сервера: этот справочник не загрузился, окно спросит его снова
                let fell = || "поток чтения упал".to_string();
                Found::References {
                    generation,
                    statuses: statuses.map(|h| h.join().unwrap_or_else(|_| Err(fell()))),
                    services: services.map(|h| h.join().unwrap_or_else(|_| intraservice::Services { error: fell(), ..Default::default() })),
                    types: types.map(|h| h.join().unwrap_or_else(|_| Err(fell()))),
                    filters: filters.map(|h| h.join().unwrap_or_else(|_| Err(fell()))),
                }
            });
            send(&inbox, &weak, found);
        });
    }

    /// «Найти» (`Search`): условия → запрос → первая страница, в потоке. `by_user` — поиск человека: удастся — его условия
    /// станут последними (`last`), быстрый поиск кончится.
    fn find(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, by_user: bool) {
        let filter = form_of(s, &state.borrow().finder);
        let ready = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let f = &mut st.finder;
            f.start_over();
            f.searched = Some((filter.clone(), by_user));
            match st.client.clone() {
                Ok(client) => {
                    f.busy = true;
                    f.message.clear();
                    Some((client, f.generation, f.statuses.clone(), f.services.clone().unwrap_or_default(), st.settings.closed_names()))
                }
                Err(why) => {
                    f.busy = false;
                    f.message = why;
                    None
                }
            }
        };
        App::show_finder_in(s, &state.borrow());
        let Some((client, generation, cached, services, closed)) = ready else { return };
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            // «открытые» и «закрытые» без справочника не определить: не читали — читаем (не вышло — пробуем при следующем)
            let read = (filter.status != SearchStatus::Any && cached.is_none()).then(|| client.get_statuses());
            let statuses = cached.or_else(|| read.as_ref().and_then(|r| r.as_ref().ok().cloned())).unwrap_or_default();
            let resolved = search::resolve(&client, &filter, &statuses, &services, &closed).map(Box::new);
            let page = resolved.as_ref().ok().map(|r| client.find_tasks(&r.query, 1));
            send(&inbox, &weak, Found::Page { generation, statuses: read, resolved: Some(resolved), page });
        });
    }

    /// «Показать ещё» (`ShowMore`): следующая страница того же запроса.
    fn more(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>) {
        let (client, query, page, generation) = {
            let mut st = state.borrow_mut();
            let st = &mut *st;
            let f = &mut st.finder;
            let (Ok(client), Some(r)) = (st.client.clone(), f.resolved.as_ref()) else { return };
            if f.busy || !f.results.has_more {
                return;
            }
            // номер поиска тот же: «ещё» обгоняют только новый поиск и смена клиента, и они меняют его сами
            f.busy = true;
            (client, r.query.clone(), f.results.pages + 1, f.generation)
        };
        App::show_finder_state(s, &state.borrow());
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let page = client.find_tasks(&query, page);
            send(&inbox, &weak, Found::Page { generation, statuses: None, resolved: None, page: Some(page) });
        });
    }

    /// Выбрали строку: просмотр — её карточка и переписка, в потоке.
    fn preview(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, index: usize) {
        let ready = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let f = &mut st.finder;
            let Some(row) = f.results.rows.get(index) else { return };
            f.selected = Some(index);
            next(&mut f.preview_generation);
            f.preview_title = format!("#{} {}", row.id, row.name);
            f.preview_text.clear();
            f.shown = None;
            let row = row.clone();
            match st.client.clone() {
                Ok(client) => {
                    f.preview_busy = true;
                    let (id, changed, generation) = (row.id, row.changed, f.preview_generation);
                    Some((TextJob::of(st, client, row), id, changed, generation))
                }
                Err(why) => {
                    f.preview_busy = false;
                    f.preview_text = why;
                    None
                }
            }
        };
        App::show_finder_state(s, &state.borrow());
        let Some((job, id, changed, generation)) = ready else { return };
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let (me, text) = job.run();
            send(&inbox, &weak, Found::Preview { generation, id, changed, me, text });
        });
    }

    /// Выбор строк (`SetSelection`): щелчок — одна, Ctrl — добавить или убрать, Shift — подряд от прежней; просмотр — только что
    /// выбранная, а убрали её — последняя из выбранных; не осталось ни одной — просмотр пуст.
    fn pick(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, index: usize, ctrl: bool, shift: bool) {
        let shown = {
            let mut st = state.borrow_mut();
            let f = &mut st.finder;
            if index >= f.results.rows.len() {
                return;
            }
            if shift {
                let from = f.anchor.unwrap_or(index);
                let range = from.min(index)..=from.max(index);
                // Shift — подряд вместо выбранного; Ctrl+Shift — подряд в добавок к нему (`SelectionMode.Extended`)
                if ctrl {
                    let new: Vec<usize> = range.filter(|i| !f.picked.contains(i)).collect();
                    f.picked.extend(new);
                } else {
                    f.picked = range.collect();
                }
                Some(index)
            } else if ctrl {
                f.anchor = Some(index);
                match f.picked.iter().position(|&i| i == index) {
                    Some(at) => {
                        f.picked.remove(at);
                        f.picked.last().copied()
                    }
                    None => {
                        f.picked.push(index);
                        Some(index)
                    }
                }
            } else {
                f.anchor = Some(index);
                f.picked = vec![index];
                Some(index)
            }
        };
        App::show_finder_in(s, &state.borrow());
        match shown {
            // та же строка уже в просмотре (текст есть или читается) — не перечитывать; не прочиталась — читать снова
            Some(i) if App::shows(&state.borrow().finder, i) => {}
            Some(i) => App::preview(s, state, inbox, i),
            None => {
                {
                    let mut st = state.borrow_mut();
                    let f = &mut st.finder;
                    f.selected = None;
                    next(&mut f.preview_generation);
                    f.preview_title.clear();
                    f.preview_text.clear();
                    f.preview_busy = false;
                    f.shown = None;
                }
                App::show_finder_state(s, &state.borrow());
            }
        }
    }

    /// Строка `i` в просмотре и её текст показан или читается.
    fn shows(f: &Finder, i: usize) -> bool {
        f.selected == Some(i) && (f.shown.is_some() || f.preview_busy)
    }

    /// «Копировать» (`Copy`): тексты выбранных заявок — в буфер обмена, одна за другой, не больше 30. Текст в просмотре
    /// (заявка с тех пор не менялась) берётся как есть, остальные читаются в потоке — по 4 разом, «кто я» — один раз.
    fn copy(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>) {
        /// Текст строки: уже есть или его читать.
        enum Text {
            Ready(String),
            Read(Box<TextJob>),
        }
        let ready = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let f = &st.finder;
            if f.copying || f.exporting {
                return;
            }
            let rows: Vec<intraservice::Found> = f.picked.iter().filter_map(|&i| f.results.rows.get(i).cloned()).collect();
            match st.client.clone() {
                Err(why) => {
                    st.finder.work_message = why;
                    None
                }
                Ok(_) if rows.is_empty() => return,
                Ok(_) if rows.len() > MAX_COPY => {
                    st.finder.work_message = format!(
                        "В буфер — не больше {MAX_COPY} заявок за раз: в чат больше не поместится. Для большего — выгрузка файлами."
                    );
                    None
                }
                Ok(client) => {
                    let ttl = f.text_ttl.unwrap_or(TEXT_TTL);
                    let texts: Vec<Text> = rows
                        .into_iter()
                        .map(|row| match &f.shown {
                            // без «изменена» в строке «не менялась» не доказать — читаем заново, как C#
                            Some((id, Some(changed), text, at)) if *id == row.id && row.changed == Some(*changed) && at.elapsed() < ttl => {
                                Text::Ready(text.clone())
                            }
                            _ => Text::Read(Box::new(TextJob::of(st, client.clone(), row))),
                        })
                        .collect();
                    if texts.iter().all(|t| matches!(t, Text::Ready(_))) {
                        let texts: Vec<String> =
                            texts.into_iter().filter_map(|t| if let Text::Ready(t) = t { Some(t) } else { None }).collect();
                        drop(guard);
                        put_and_tell(state, &texts, &[]);
                        return App::show_finder_state(s, &state.borrow());
                    }
                    let f = &mut st.finder;
                    f.copying = true;
                    f.copy_stop = Arc::new(AtomicBool::new(false));
                    next(&mut f.copy_generation);
                    f.work_message = format!("Готовлю текст: 0 из {}…", texts.len());
                    Some((texts, f.me.clone(), client, f.copy_generation, f.copy_stop.clone()))
                }
            }
        };
        App::show_finder_state(s, &state.borrow());
        let Some((texts, me, client, generation, stop)) = ready else { return };
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let me = me.or_else(|| client.get_current_user().ok());
            let total = texts.len();
            let (queue, done) = (Mutex::new(texts.into_iter().enumerate().collect::<Vec<_>>()), AtomicUsize::new(0));
            // тексты — по порядку строк, ошибки — по порядку прихода (`errors.Add` у C#): первая — кто кончил первым
            let (results, errors) = (Mutex::new(vec![None; total]), Mutex::new(Vec::new()));
            // по 4 запроса разом, как у C#: сервер общий
            std::thread::scope(|scope| {
                for _ in 0..total.min(4) {
                    scope.spawn(|| {
                        loop {
                            // очередь — только на время «взять следующую»: читаются четыре разом, а не по одной
                            let next = queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
                            let Some((i, text)) = next else { break };
                            if stop.load(Ordering::SeqCst) {
                                break;
                            }
                            let result = match text {
                                Text::Ready(t) => Ok(t),
                                Text::Read(job) => job.text(&me).map_err(|e| format!("#{}: {e}", job.row.id)),
                            };
                            match result {
                                Ok(t) => results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(t),
                                Err(e) => errors.lock().unwrap_or_else(|e| e.into_inner()).push(e),
                            }
                            let done = done.fetch_add(1, Ordering::SeqCst) + 1;
                            send(&inbox, &weak, Found::Copying { generation, done, total });
                        }
                    });
                }
            });
            let texts = results.into_inner().unwrap_or_else(|e| e.into_inner()).into_iter().flatten().collect();
            let errors = errors.into_inner().unwrap_or_else(|e| e.into_inner());
            send(&inbox, &weak, Found::Copied { generation, me, texts, errors });
        });
    }

    /// «Выгрузить выбранные» (`ExportSelected`): файлы выбранных заявок — в папку базы знаний. Доска только для чтения —
    /// нет: прототип тогда ничего не пишет. «Кто я» не узнан — не начата: файл без роли сочли бы готовым.
    fn export_selected(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>) {
        let ready = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let f = &st.finder;
            if f.copying || f.exporting || f.asking.is_some() || f.picked.is_empty() {
                return;
            }
            let rows: Vec<intraservice::Found> = f.picked.iter().filter_map(|&i| f.results.rows.get(i).cloned()).collect();
            App::export_target(s, st).map(|(_, dir)| (Work::Rows(rows), dir))
        };
        match ready {
            Some((work, dir)) => App::start_export(s, state, inbox, work, dir),
            None => App::show_finder_state(s, &state.borrow()),
        }
    }

    /// «Выгрузить найденные» (`ExportFound`): всё найденное по этим условиям (не больше «Не больше, заявок»; 0 — все) —
    /// файлами в ту же папку. Много заявок (`CONFIRM_FROM` и больше) — сначала вопрос: это часы работы и тысячи запросов.
    fn export_found(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>) {
        let ready = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let f = &st.finder;
            let Some(resolved) = f.resolved.as_ref().filter(|_| App::can_export_found(f)) else { return };
            let query = resolved.query.clone();
            let limit = search::limit_of(&s.get_limit());
            let found = f.results.total; // за время выгрузки можно искать снова: к концу это уже другой список
            match export::run::invalid_limit(limit) {
                Some(bad) => {
                    st.finder.work_message = bad.into();
                    None
                }
                None => App::export_target(s, st).map(|(_, dir)| Pending { query, limit, found, dir }),
            }
        };
        let Some(pending) = ready else { return App::show_finder_state(s, &state.borrow()) };
        // сколько выгрузится самое большее: неизвестно (найдено «1 000+») — столько, сколько задано, а при 0 — сколько угодно
        let (found, limit) = (pending.found, pending.limit);
        let up_to = search::export_count(found, limit).unwrap_or(if limit == 0 { i32::MAX } else { limit });
        if up_to >= search::CONFIRM_FROM {
            let (title, text) = (search::confirm_title(found, limit), search::confirm_text(found, limit, &pending.dir));
            state.borrow_mut().finder.asking = Some((title, text, pending));
            return App::show_finder_state(s, &state.borrow());
        }
        App::start_found(s, state, inbox, pending);
    }

    /// Ответ на вопрос перед большой выгрузкой: «Выгрузить» — начать, «Отмена» и Esc — ничего не начато (`Confirm`).
    fn answer(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, yes: bool) {
        let pending = state.borrow_mut().finder.asking.take().map(|(_, _, p)| p);
        match (pending, yes) {
            (Some(p), true) => App::start_found(s, state, inbox, p),
            (Some(_), false) => {
                state.borrow_mut().finder.work_message = "Выгрузка отменена.".into();
                App::show_finder_state(s, &state.borrow());
            }
            (None, _) => App::show_finder_state(s, &state.borrow()),
        }
    }

    fn start_found(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, p: Pending) {
        // пока думали, могло стать нельзя (доска, клиент): проверить ещё раз
        let target = App::export_target(s, &mut state.borrow_mut());
        if target.is_none() {
            return App::show_finder_state(s, &state.borrow());
        }
        App::start_export(s, state, inbox, Work::Found { query: Box::new(p.query), limit: p.limit, found: p.found }, p.dir);
    }

    /// «Выгрузить найденные» можно (`CanExportFound`): не занято, поиск был, условия с тех пор не меняли, найдено что-то.
    fn can_export_found(f: &Finder) -> bool {
        !f.copying && !f.exporting && f.asking.is_none() && f.resolved.is_some() && !f.stale && f.results.total > 0
    }

    /// Куда писать: доска своя, клиент есть, папка — полный путь без части с пробелом или точкой в конце; нет — причина
    /// строкой под кнопками и None.
    fn export_target(s: &SearchWindow, st: &mut State) -> Option<(Arc<intraservice::Client>, String)> {
        let dir = s.get_folder().trim().to_string();
        let why = match (&st.store, st.client.clone()) {
            (None, _) => EXPORT_READ_ONLY.to_string(),
            (_, Err(why)) => why,
            (_, Ok(_)) if !Path::new(&dir).is_absolute() => "Папка — полный путь, например D:\\Obsidian\\База\\Заявки".into(),
            (_, Ok(_)) if let Some(part) = export::files::bad_folder_part(Path::new(&dir)) => export::files::bad_folder_message(&part),
            (_, Ok(client)) => return Some((client, dir)),
        };
        st.finder.work_message = why;
        None
    }

    /// Начать выгрузку в потоке: «кто я» (не узнан — не начата), потом движок; итог — строкой под кнопками (`FinishWork`).
    fn start_export(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, work: Work, dir: String) {
        let (client, closed, settings, me, stop, generation, epoch) = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let Ok(client) = st.client.clone() else { return };
            let (closed, settings) = (st.settings.closed_names(), st.settings.clone());
            let f = &mut st.finder;
            f.exporting = true;
            f.export_stop = Arc::new(AtomicBool::new(false));
            next(&mut f.export_generation);
            f.work_message = export::run::STARTING.into();
            (client, closed, settings, f.me.clone(), f.export_stop.clone(), f.export_generation, f.references_generation)
        };
        App::show_finder_state(s, &state.borrow());
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let me = match me.map_or_else(|| client.get_current_user(), Ok) {
                Ok(me) => me,
                Err(e) => {
                    let summary = format!("Выгрузка не начата: {e}");
                    return send(&inbox, &weak, Found::Exported { generation, client: epoch, me: None, summary });
                }
            };
            let context = export::Context { me: Some(me.clone()), closed };
            let progress = |line: String| send(&inbox, &weak, Found::Exporting { generation, line });
            let url = |id: i32| settings.ticket_url(id);
            let options = export::run::Options::default();
            let dir = Path::new(&dir);
            let summary = match work {
                Work::Rows(rows) => export::run::summary(
                    &export::run::export_rows(&client, &rows, dir, &url, &context, &progress, &stop, &options),
                    "Выбрано",
                ),
                Work::Found { query, limit, found } => {
                    let r = export::run::export_found(&client, &query, limit, dir, &url, &context, &progress, &stop, &options);
                    export::run::summary(&r, "По отбору") + &search::found_tail(found, limit)
                }
            };
            send(&inbox, &weak, Found::Exported { generation, client: epoch, me: Some(me), summary });
        });
    }

    /// «Остановить» (`Stop`): выгрузка дописывает начатое и останавливается; копирование дочитывает начатое, в буфер — ничего.
    fn stop(s: &SearchWindow, state: &Rc<RefCell<State>>) {
        {
            let mut st = state.borrow_mut();
            let f = &mut st.finder;
            if f.exporting {
                f.export_stop.store(true, Ordering::SeqCst);
                f.work_message = "Останавливаю — дописываю начатое…".into();
            } else if f.copying {
                // начатые чтения не прервать (у ureq нет отмены) — окно занято, пока они не кончатся, новых нет
                f.copy_stop.store(true, Ordering::SeqCst);
                f.work_message = "Останавливаю — дочитываю начатое…".into();
            }
        }
        App::show_finder_state(s, &state.borrow());
    }

    /// «Открыть» (`OpenFolder`): папку — в Проводнике (нет — создать).
    fn open_folder(s: &SearchWindow, state: &Rc<RefCell<State>>) {
        let dir = s.get_folder().trim().to_string();
        if !Path::new(&dir).is_absolute() {
            return;
        }
        if let Some(part) = export::files::bad_folder_part(Path::new(&dir)) {
            state.borrow_mut().finder.work_message = export::files::bad_folder_message(&part);
            return App::show_finder_state(s, &state.borrow());
        }
        // доска только для чтения — прототип ничего не пишет, и папку не создаёт: открывает, только если она есть
        let writable = state.borrow().store.is_some();
        let opened = if writable { std::fs::create_dir_all(&dir) } else { Ok(()) };
        let opened = opened.and_then(|()| match Path::new(&dir).is_dir() {
            true => open_in_explorer(Path::new(&dir)),
            false => Err(std::io::Error::other("папки нет")),
        });
        if let Err(e) = opened {
            state.borrow_mut().finder.work_message = format!("Не открыть папку: {e}");
            App::show_finder_state(s, &state.borrow());
        }
    }

    /// Ответ потока — в окно, если он ещё про то, что на экране.
    fn found(s: &SearchWindow, state: &Rc<RefCell<State>>, m: Found) {
        {
            let mut st = state.borrow_mut();
            let f = &mut st.finder;
            match m {
                Found::Page { generation, statuses, resolved, page } => {
                    if generation != f.generation {
                        return;
                    }
                    // пустой справочник не запоминаем: «открытые» и «закрытые» без него не определить — спросим снова; прочитанный
                    // поиском — и в список выбора, и долой из строки о несчитанных (у C# такой поиск идёт через
                    // `LoadReferencesAsync` → `FillReferences`)
                    if let Some(Ok(list)) = statuses.filter(|r| r.as_ref().is_ok_and(|l| !l.is_empty())) {
                        let wanted = form_of(s, f);
                        f.statuses = Some(list);
                        f.failed("статусы", None);
                        show_choices(s, f);
                        set_form(s, &wanted, f);
                    }
                    f.busy = false;
                    match resolved {
                        Some(Err(e)) => f.message = e,
                        Some(Ok(r)) => {
                            f.notes = r.notes.join("\n");
                            f.resolved = Some(*r);
                            if let Some((searched, by_user)) = f.searched.take() {
                                // вложенные берутся из списка сервисов: неполный или не загрузился — сказать, что вошли не все
                                if let Some(note) = search::children_note(&searched, f.services.is_some(), f.services_partial) {
                                    f.notes = if f.notes.is_empty() { note } else { format!("{}\n{note}", f.notes) };
                                }
                                // пока искали, условия могли поменять — и вернуть как было: «изменены» — по сравнению
                                f.stale = form_of(s, f) != searched;
                                if by_user {
                                    f.last = Some(searched);
                                    f.quick = false;
                                }
                            }
                        }
                        None => {}
                    }
                    if let (Some(page), Some(r)) = (page, f.resolved.as_ref()) {
                        match page {
                            Ok(p) => {
                                f.results.add_page(p, &r.query);
                                f.message = f.results.message();
                            }
                            Err(e) => f.message = e,
                        }
                    }
                }
                Found::References { generation, statuses, services, types, filters } => {
                    if generation != f.references_generation {
                        return;
                    }
                    // выбор в списках — номерами: списки сейчас заменятся, выбранное должно остаться выбранным
                    let wanted = form_of(s, f);
                    f.references_reading = false;
                    match statuses {
                        // пока справочник читался, его прочитал поиск («Закрытые») — поздний отказ уже ни о чём
                        Some(Err(_)) if f.statuses.is_some() => {}
                        Some(Err(e)) => f.failed("статусы", Some(intraservice::headline(&e))),
                        Some(Ok(list)) => {
                            f.failed("статусы", None);
                            // пустой не запоминаем — как и при поиске: спросим снова
                            if !list.is_empty() {
                                f.statuses = Some(list);
                            }
                        }
                        None => {}
                    }
                    if let Some(sv) = services {
                        // о сервисах — прямо под их списком: пустой список без объяснения выглядит как «выбрать нельзя»
                        if sv.error.is_empty() {
                            f.services_partial = !sv.note.is_empty();
                            f.service_note = sv.note;
                            f.services = Some(sv.items);
                        } else {
                            f.service_note = format!(
                                "Список сервисов не загрузился: {}. Сервис можно задать сохранённым фильтром.",
                                intraservice::headline(&sv.error)
                            );
                        }
                    }
                    match types {
                        Some(Err(e)) => f.failed("типы", Some(intraservice::headline(&e))),
                        Some(Ok(list)) => {
                            f.failed("типы", None);
                            f.types = Some(list);
                        }
                        None => {}
                    }
                    match filters {
                        Some(Err(e)) => f.failed("сохранённые фильтры", Some(intraservice::headline(&e))),
                        Some(Ok(list)) => {
                            f.failed("сохранённые фильтры", None);
                            f.filters = Some(list);
                        }
                        None => {}
                    }
                    let f = &*f;
                    show_choices(s, f);
                    set_form(s, &wanted, f);
                    drop(st);
                    return App::show_finder_state(s, &state.borrow());
                }
                Found::Preview { generation, id, changed, me, text } => {
                    if generation != f.preview_generation {
                        return;
                    }
                    f.me = f.me.take().or(me);
                    f.preview_busy = false;
                    match text {
                        Ok(text) => {
                            f.preview_text = text.clone();
                            f.shown = Some((id, changed, text, Instant::now()));
                        }
                        Err(e) => f.preview_text = format!("Не удалось прочитать заявку:\n{e}"),
                    }
                    drop(st);
                    return App::show_finder_state(s, &state.borrow());
                }
                Found::Copying { generation, done, total } => {
                    if generation != f.copy_generation || !f.copying || f.copy_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    f.work_message = format!("Готовлю текст: {done} из {total}…");
                    drop(st);
                    return App::show_finder_state(s, &state.borrow());
                }
                Found::Copied { generation, me, texts, errors } => {
                    if generation != f.copy_generation {
                        return;
                    }
                    f.me = f.me.take().or(me);
                    f.copying = false;
                    if f.copy_stop.load(Ordering::SeqCst) {
                        f.work_message = "Остановлено, в буфер ничего не положено.".into();
                        drop(st);
                        return App::show_finder_state(s, &state.borrow());
                    }
                    drop(st);
                    put_and_tell(state, &texts, &errors);
                    return App::show_finder_state(s, &state.borrow());
                }
                Found::Exporting { generation, line } => {
                    // после «Остановить» ход уже не пишем — итог скажет, что успели (`ProgressInto`)
                    if generation != f.export_generation || !f.exporting || f.export_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    f.work_message = line;
                    drop(st);
                    return App::show_finder_state(s, &state.borrow());
                }
                Found::Exported { generation, client, me, summary } => {
                    if generation != f.export_generation {
                        return;
                    }
                    // итог остановленной сменой клиента — показать, но «кто я» прежнего сервера не запоминать
                    if client == f.references_generation {
                        f.me = f.me.take().or(me);
                    }
                    f.exporting = false;
                    f.work_message = summary;
                    drop(st);
                    return App::show_finder_state(s, &state.borrow());
                }
            }
        }
        App::show_finder_in(s, &state.borrow());
    }

    /// Состояние — в окно.
    fn show_finder_in(s: &SearchWindow, st: &State) {
        s.set_rows(ModelRc::new(VecModel::from(rows_of(&st.finder.results, &st.board.numbers(), &st.finder.picked))));
        App::show_finder_state(s, st);
    }

    /// Всё, кроме строк: правка условия, выбор строки, просмотр не пересоздают до тысячи строк списка.
    fn show_finder_state(s: &SearchWindow, st: &State) {
        let f = &st.finder;
        s.set_selected(f.selected.map_or(-1, |i| i as i32));
        s.set_busy(f.busy);
        s.set_message(f.message.as_str().into());
        s.set_notes(f.notes.as_str().into());
        s.set_warning(f.results.warning.as_str().into());
        s.set_stale(f.stale && !f.results.rows.is_empty());
        s.set_has_more(f.results.has_more);
        s.set_writable(st.store.is_some());
        s.set_preview_title(f.preview_title.as_str().into());
        s.set_preview_text(f.preview_text.as_str().into());
        s.set_preview_busy(f.preview_busy);
        let (free, n) = (!f.copying && !f.exporting, f.picked.len());
        s.set_can_copy(n > 0 && free);
        s.set_can_export(n > 0 && free);
        s.set_working(!free);
        s.set_copy_label(if n > 1 { format!("Копировать ({n})") } else { "Копировать".into() }.into());
        s.set_export_label(if n > 0 { format!("Выгрузить выбранные ({n})") } else { "Выгрузить выбранные".into() }.into());
        s.set_found_label(search::found_label(f.results.total, &s.get_limit()).into());
        s.set_can_export_found(App::can_export_found(f));
        let (title, text) = f.asking.as_ref().map_or((String::new(), String::new()), |(t, x, _)| (t.clone(), x.clone()));
        s.set_asking(f.asking.is_some());
        s.set_ask_title(title.into());
        s.set_ask_text(text.into());
        s.set_work_message(f.work_message.as_str().into());
    }
}

fn send(inbox: &Arc<Mutex<Vec<Found>>>, weak: &slint::Weak<SearchWindow>, m: Found) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(m);
    let _ = weak.upgrade_in_event_loop(|s| s.invoke_deliver());
}

impl App {
    /// Окно поиска: его кнопки и поля, кнопка и Enter доски (`App.OnStartup`: `SearchRequested`, `ServerSearchRequested`).
    pub(super) fn wire_finder(&self) {
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.window.on_open_search(move || {
            if let Some(s) = sw.upgrade() {
                App::show_search_in(&s, &state, &inbox, None);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.window.on_server_search(move |text| {
            if let Some(s) = sw.upgrade() {
                App::show_search_in(&s, &state, &inbox, Some(&text));
            }
        });

        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_search(move || {
            if let Some(s) = sw.upgrade() {
                App::find(&s, &state, &inbox, true);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_more(move || {
            if let Some(s) = sw.upgrade() {
                App::more(&s, &state, &inbox);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_select(move |i| {
            if let (Some(s), Ok(i)) = (sw.upgrade(), usize::try_from(i)) {
                App::pick(&s, &state, &inbox, i, false, false);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_pick(move |i, ctrl, shift| {
            if let (Some(s), Ok(i)) = (sw.upgrade(), usize::try_from(i)) {
                App::pick(&s, &state, &inbox, i, ctrl, shift);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_export_selected(move || {
            if let Some(s) = sw.upgrade() {
                App::export_selected(&s, &state, &inbox);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_export_found(move || {
            if let Some(s) = sw.upgrade() {
                App::export_found(&s, &state, &inbox);
            }
        });
        for yes in [true, false] {
            let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
            let answer = move || {
                if let Some(s) = sw.upgrade() {
                    App::answer(&s, &state, &inbox, yes);
                }
            };
            if yes {
                self.finder.on_ask_yes(answer);
            } else {
                self.finder.on_ask_no(answer);
            }
        }
        // поле «Не больше, заявок» поменялось — подпись кнопки следом (условием поиска оно не считается: список не устарел)
        let (sw, state) = (self.finder.as_weak(), self.state.clone());
        self.finder.on_limit_edited(move |_| {
            if let Some(s) = sw.upgrade() {
                App::show_finder_state(&s, &state.borrow());
            }
        });
        let (sw, state) = (self.finder.as_weak(), self.state.clone());
        self.finder.on_stop(move || {
            if let Some(s) = sw.upgrade() {
                App::stop(&s, &state);
            }
        });
        let (sw, state) = (self.finder.as_weak(), self.state.clone());
        self.finder.on_open_folder(move || {
            if let Some(s) = sw.upgrade() {
                App::open_folder(&s, &state);
            }
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_copy(move || {
            if let Some(s) = sw.upgrade() {
                App::copy(&s, &state, &inbox);
            }
        });
        let (sw, state) = (self.finder.as_weak(), self.state.clone());
        self.finder.on_clear(move || {
            let Some(s) = sw.upgrade() else { return };
            set_form(&s, &SearchFilter::default(), &state.borrow().finder);
            {
                let mut st = state.borrow_mut();
                st.finder.stale = true; // условия сброшены, а список на экране — по прежним
                st.finder.message.clear();
            }
            App::show_finder_state(&s, &state.borrow());
        });
        let (sw, state) = (self.finder.as_weak(), self.state.clone());
        self.finder.on_edited(move || {
            let Some(s) = sw.upgrade() else { return };
            state.borrow_mut().finder.stale = true;
            App::show_finder_state(&s, &state.borrow());
        });
        let sw = self.finder.as_weak();
        self.finder.on_cancel(move || {
            if let Some(s) = sw.upgrade() {
                let _ = s.hide();
            }
        });
        let state = self.state.clone();
        self.finder.on_open(move |i| {
            let st = state.borrow();
            if let Some(f) = usize::try_from(i).ok().and_then(|i| st.finder.results.rows.get(i)) {
                open_url(&st.settings.ticket_url(f.id));
            }
        });
        let (sw, main, state, inbox, timer) =
            (self.finder.as_weak(), self.window.as_weak(), self.state.clone(), self.inbox.clone(), self.save_timer.clone());
        self.finder.on_add(move |i| {
            let (Some(s), Some(w), Ok(i)) = (sw.upgrade(), main.upgrade(), usize::try_from(i)) else { return };
            App::add_found(&s, &w, &state, &inbox, &timer, i);
        });
        let (sw, state, inbox) = (self.finder.as_weak(), self.state.clone(), self.finder_inbox.clone());
        self.finder.on_deliver(move || {
            let Some(s) = sw.upgrade() else { return };
            let messages = std::mem::take(&mut *inbox.lock().unwrap_or_else(|e| e.into_inner()));
            for m in messages {
                App::found(&s, &state, m);
            }
        });
        // окно начинается с «моих закрытых», как у C# без запомненных условий
        show_choices(&self.finder, &self.state.borrow().finder);
        set_form(&self.finder, &SearchFilter::usual(), &self.state.borrow().finder);
        App::show_finder_in(&self.finder, &self.state.borrow());
    }

    /// «+ На доску» (`MainViewModel.AddKnown`): номер, ссылка и название известны — без разбора текста; карточка наверху
    /// «Входящих», выбрана, читается из Интрасервиса (⟳). Уже на доске — ничего; доска только для чтения — сказано.
    fn add_found(
        s: &SearchWindow,
        w: &MainWindow,
        state: &Rc<RefCell<State>>,
        inbox: &Arc<Mutex<Vec<Message>>>,
        timer: &Rc<Timer>,
        i: usize,
    ) {
        let key = {
            let mut guard = state.borrow_mut();
            let st = &mut *guard;
            let Some(f) = st.finder.results.rows.get(i) else { return };
            if st.store.is_none() {
                st.finder.message = ADD_READ_ONLY.into();
                drop(guard);
                return App::show_finder_in(s, &state.borrow());
            }
            if st.board.numbers().contains(&f.id) {
                // попала на доску другим путём — строка просто покажет «на доске»
                drop(guard);
                return App::show_finder_in(s, &state.borrow());
            }
            let mut t = Ticket::new(now());
            t.priority = Priority::Mid;
            t.intraservice_id = Some(f.id);
            t.url = st.settings.ticket_url(f.id);
            // пустое название разбор списка уже заменил на «Заявка #N»
            t.title = f.name.clone();
            let key = t.id.clone();
            st.board.insert(t);
            key
        };
        w.set_selected_key(key.as_str().into());
        w.set_panel_open(true);
        App::changed(w, state, timer, Scope::Board, &key);
        App::show_panel_in(w, &state.borrow(), true);
        if state.borrow().client.is_ok() {
            App::sync_card(w, state, inbox, &key);
        }
        App::show_finder_in(s, &state.borrow());
    }

    /// Сколько текст из просмотра годится для «Копировать» без нового чтения (`TextTtl` у C#; проверки окна — свой срок).
    pub fn use_text_ttl(&self, ttl: Duration) {
        self.state.borrow_mut().finder.text_ttl = Some(ttl);
    }

    /// Другой клиент Интрасервиса: идущие поиск и просмотр — уже не про этот сервер, найденное и справочник забываются.
    pub(super) fn forget_found(&self) {
        {
            let mut st = self.state.borrow_mut();
            let f = &mut st.finder;
            f.start_over();
            f.asking = None; // вопрос был о списке прежнего сервера
            // выгрузка шла со старого сервера — останавливается, итог скажет, что успели (`_work?.Cancel()` у C#)
            if f.exporting {
                f.export_stop.store(true, Ordering::SeqCst);
            }
            // копирование шло со старого сервера — его текст в буфер не кладём и дальше не читаем (`_work?.Cancel()` у C#)
            f.copy_stop.store(true, Ordering::SeqCst);
            if f.copying {
                f.work_message = "Остановлено, в буфер ничего не положено.".into();
            }
            f.copying = false;
            next(&mut f.copy_generation);
            f.me = None;
            next(&mut f.references_generation);
            f.references_reading = false;
            f.statuses = None;
            f.services = None;
            f.services_partial = false;
            f.service_note.clear();
            f.types = None;
            f.filters = None;
            f.failures.clear();
            // номера прежнего сервера в прежних условиях — у другого сервера под ними другое (`ApplySettings` у C#)
            if let Some(last) = &mut f.last {
                last.service_id = 0;
                last.type_id = 0;
                last.saved_filter_id = 0;
                if last.status == SearchStatus::One {
                    last.status = SearchStatus::Any;
                    last.status_id = 0;
                }
            }
        }
        let st = self.state.borrow();
        let wanted = form_of(&self.finder, &st.finder);
        show_choices(&self.finder, &st.finder);
        set_form(&self.finder, &wanted, &st.finder);
        App::show_finder_in(&self.finder, &st);
    }
}

/// Папка — в Проводнике (Windows): оболочкой, как у C# (`UseShellExecute`), а не программой «explorer» — её Windows искала бы
/// сначала в папке прототипа. На других системах прототип не живёт.
#[cfg(windows)]
fn open_in_explorer(dir: &Path) -> std::io::Result<()> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};
    // SAFETY: строки живут до конца вызова
    let code = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(dir.as_os_str()), None, None, SW_SHOWNORMAL) };
    // больше 32 — открыто; иначе ответ и есть код ошибки (ShellExecute не ставит last error)
    if code.0 as usize > 32 { Ok(()) } else { Err(std::io::Error::other(format!("ShellExecute: код {}", code.0 as usize))) }
}

#[cfg(not(windows))]
fn open_in_explorer(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}
