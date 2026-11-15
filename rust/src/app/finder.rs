//! Окно «Поиск в Интрасервисе» (`SearchWindow.xaml(.cs)` + `SearchViewModel`): условия → запрос (`search::resolve`) →
//! страницы списка → просмотр выбранной → «+ На доску». Правила — в `search.rs`; здесь — окно, потоки и доска. Ответ
//! потока, пришедший после нового поиска или смены клиента, отбрасывается (`generation`, как `_lookup` у C#).

use super::*;
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
    /// Справочник статусов — для «открытых» и «закрытых»; читается один раз, при первом таком поиске.
    statuses: Option<Vec<intraservice::Status>>,
}

impl Finder {
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

/// Условия из окна.
fn form_of(w: &SearchWindow) -> SearchFilter {
    SearchFilter {
        words: w.get_words().into(),
        mine: w.get_mine(),
        executor: w.get_executor().into(),
        creator: w.get_creator().into(),
        observer_mine: w.get_observer_mine(),
        observer: w.get_observer().into(),
        status: match w.get_status_index() {
            1 => SearchStatus::Open,
            2 => SearchStatus::Closed,
            _ => SearchStatus::Any,
        },
        created_from: w.get_created_from().into(),
        created_to: w.get_created_to().into(),
        changed_from: w.get_changed_from().into(),
        changed_to: w.get_changed_to().into(),
        closed_from: w.get_closed_from().into(),
        closed_to: w.get_closed_to().into(),
        include_archived: w.get_archived(),
        ..SearchFilter::default() // сервис, тип, фильтр, статус — окно 3.3-2
    }
}

/// Условия — в окно.
fn set_form(w: &SearchWindow, f: &SearchFilter) {
    w.set_words(f.words.as_str().into());
    w.set_mine(f.mine);
    w.set_executor(f.executor.as_str().into());
    w.set_creator(f.creator.as_str().into());
    w.set_observer_mine(f.observer_mine);
    w.set_observer(f.observer.as_str().into());
    w.set_status_index(match f.status {
        SearchStatus::Any => 0,
        SearchStatus::Open => 1,
        SearchStatus::Closed => 2,
        SearchStatus::One => 0, // окно 3.3-2
    });
    w.set_created_from(f.created_from.as_str().into());
    w.set_created_to(f.created_to.as_str().into());
    w.set_changed_from(f.changed_from.as_str().into());
    w.set_changed_to(f.changed_to.as_str().into());
    w.set_closed_from(f.closed_from.as_str().into());
    w.set_closed_to(f.closed_to.as_str().into());
    w.set_archived(f.include_archived);
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
                    set_form(s, &SearchFilter::quick(words));
                }
                // показанное окно только выходит вперёд; спрятанное человеком после быстрого поиска — с условиями последнего
                // поиска человека (`OpenAsync(restoreQuick: reopened)`)
                None if st.finder.quick && !s.window().is_visible() => {
                    st.finder.quick = false;
                    set_form(s, &st.finder.last.clone().unwrap_or_else(SearchFilter::usual));
                    st.finder.start_over();
                }
                None => {}
            }
        }
        let _ = s.show();
        s.invoke_focus_words();
        if words.is_some() {
            App::find(s, state, inbox, false);
        } else {
            App::show_finder_in(s, &state.borrow());
        }
    }

    /// «Найти» (`Search`): условия → запрос → первая страница, в потоке. `by_user` — поиск человека: удастся — его условия
    /// станут последними (`last`), быстрый поиск кончится.
    fn find(s: &SearchWindow, state: &Rc<RefCell<State>>, inbox: &Arc<Mutex<Vec<Found>>>, by_user: bool) {
        let filter = form_of(s);
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
                    Some((client, f.generation, f.statuses.clone(), st.settings.closed_names()))
                }
                Err(why) => {
                    f.busy = false;
                    f.message = why;
                    None
                }
            }
        };
        App::show_finder_in(s, &state.borrow());
        let Some((client, generation, cached, closed)) = ready else { return };
        let (weak, inbox) = (s.as_weak(), inbox.clone());
        std::thread::spawn(move || {
            // «открытые» и «закрытые» без справочника не определить: не читали — читаем (не вышло — пробуем при следующем)
            let read = (filter.status != SearchStatus::Any && cached.is_none()).then(|| client.get_statuses());
            let statuses = cached.or_else(|| read.as_ref().and_then(|r| r.as_ref().ok().cloned())).unwrap_or_default();
            let resolved = search::resolve(&client, &filter, &statuses, &[], &closed).map(Box::new);
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
                    // пустой справочник не запоминаем: «открытые» и «закрытые» без него не определить — спросим снова
                    if let Some(Ok(list)) = statuses.filter(|r| r.as_ref().is_ok_and(|l| !l.is_empty())) {
                        f.statuses = Some(list);
                    }
                    f.busy = false;
                    match resolved {
                        Some(Err(e)) => f.message = e,
                        Some(Ok(r)) => {
                            f.notes = r.notes.join("\n");
                            f.resolved = Some(*r);
                            if let Some((searched, by_user)) = f.searched.take() {
                                // пока искали, условия могли поменять — и вернуть как было: «изменены» — по сравнению
                                f.stale = form_of(s) != searched;
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
            set_form(&s, &SearchFilter::default());
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
        set_form(&self.finder, &SearchFilter::usual());
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
            st.finder.start_over();
            st.finder.statuses = None;
        }
        App::show_finder_in(&self.finder, &self.state.borrow());
    }
}
