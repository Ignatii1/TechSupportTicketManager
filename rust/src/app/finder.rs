//! Окно «Поиск в Интрасервисе» (`SearchWindow.xaml(.cs)` + `SearchViewModel`): условия → запрос (`search::resolve`) →
//! страницы списка → просмотр выбранной → «+ На доску». Правила — в `search.rs`; здесь — окно, потоки и доска. Ответ
//! потока, пришедший после нового поиска или смены клиента, отбрасывается (`generation`, как `_lookup` у C#).

use super::*;
use crate::intraservice::Ref;
use crate::search::{self, Resolved, Results, SearchFilter, SearchStatus};
use crate::{FoundRow, SearchWindow};

/// «+ На доску» на доске только для чтения.
pub const ADD_READ_ONLY: &str = "Доска только для чтения (почему — в шапке доски): добавить на неё заявку нельзя";

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
    selected: Option<usize>,
    preview_title: String,
    preview_text: String,
    preview_busy: bool,
    preview_generation: u64,
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
        if let Some(why) = why {
            self.failures.push((name, why));
            // порядок C#: статусы, типы, сохранённые фильтры
            let rank = |n: &str| ["статусы", "типы", "сохранённые фильтры"].iter().position(|x| *x == n);
            self.failures.sort_by_key(|(n, _)| rank(n));
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
        self.preview_title.clear();
        self.preview_text.clear();
        self.preview_busy = false;
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
    /// Просмотр выбранной: карточка и первая страница переписки.
    Preview { generation: u64, card: Result<Box<intraservice::Task>, String>, life: Option<Result<intraservice::LifetimePage, String>> },
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
fn rows_of(results: &Results, on_board: &HashSet<i32>) -> Vec<FoundRow> {
    results
        .rows
        .iter()
        .map(|f| FoundRow {
            id: f.id,
            number: format!("#{}", f.id).into(),
            title: f.name.as_str().into(),
            status: f.status.as_str().into(),
            service: search::service_of(f).into(),
            details: search::details_of(f).into(),
            on_board: on_board.contains(&f.id),
        })
        .collect()
}

/// Просмотр: описание и переписка, как в панели доски. ponytail: просмотр «как его получит агент» (Markdown выгрузки) —
/// со слоем выгрузки, где переносится её форматер.
fn preview_of(card: &Result<Box<intraservice::Task>, String>, life: &Option<Result<intraservice::LifetimePage, String>>) -> String {
    let description = match card {
        Ok(t) => t.description.clone().filter(|d| !d.trim().is_empty()).unwrap_or_else(|| "Описания нет.".into()),
        Err(e) => return format!("Карточка не прочиталась: {}", intraservice::brief(e)),
    };
    match life {
        Some(life) => format!("{description}\n\n{}", conversation_text(life)),
        None => description,
    }
}

impl App {
    /// Показать окно поиска. С доски (`words`) — слова из поля, остальное по умолчанию, поиск сразу; кнопкой и из трея —
    /// окно как было, а спрятанное после быстрого поиска — с прежними условиями (`ShowSearch`/`OpenAsync(restoreQuick)`).
    fn show_search_in(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, words: Option<&str>) {
        {
            let mut st = state.borrow_mut();
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
            let id = row.id;
            match st.client.clone() {
                Ok(client) => {
                    f.preview_busy = true;
                    Some((client, id, f.preview_generation))
                }
                Err(why) => {
                    f.preview_busy = false;
                    f.preview_text = why;
                    None
                }
            }
        };
        App::show_finder_state(s, &state.borrow());
        let Some((client, id, generation)) = ready else { return };
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            let card = client.get_task(id).map(Box::new);
            // карточка не прочиталась — переписку не спрашиваем: второй срок ожидания впустую (как ⟳ на доске)
            let life = card.is_ok().then(|| client.get_lifetime_page(id, 1));
            send(&inbox, &weak, Found::Preview { generation, card, life });
        });
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
                Found::Preview { generation, card, life } => {
                    if generation != f.preview_generation {
                        return;
                    }
                    f.preview_busy = false;
                    f.preview_text = preview_of(&card, &life);
                    drop(st);
                    return App::show_finder_state(s, &state.borrow());
                }
            }
        }
        App::show_finder_in(s, &state.borrow());
    }

    /// Состояние — в окно.
    fn show_finder_in(s: &SearchWindow, st: &State) {
        s.set_rows(ModelRc::new(VecModel::from(rows_of(&st.finder.results, &st.board.numbers()))));
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
                App::preview(&s, &state, &inbox, i);
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

    /// Другой клиент Интрасервиса: идущие поиск и просмотр — уже не про этот сервер, найденное и справочник забываются.
    pub(super) fn forget_found(&self) {
        {
            let mut st = self.state.borrow_mut();
            let f = &mut st.finder;
            f.start_over();
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
