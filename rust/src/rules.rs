//! Правила автообновления (`AutoSyncRules`), которые проверяются без окна, и закрытые статусы из настроек
//! (`AppSettings.ClosedNames`).

use crate::intraservice::{Date, Event, Found, SearchPage, User};
use std::collections::{HashMap, HashSet};

/// Что случилось с заявкой карточки за заход: ничего, снова открыта (вернулась в мои открытые, пока карточка в «Готово»)
/// или больше не на мне (выпала из моих открытых, а сама открыта).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    None,
    Reopened,
    Reassigned,
}

/// Жизненный цикл карточки за заход (`Track`). assigned — была ли в моих открытых по прошлому целому списку (None — ещё не
/// видели: только отсчёт); listed — есть в списке сейчас; complete — список целый; done — карточка в «Готово»; closed — для
/// выпавших и перечитанных: закрыта ли (None — не перечитана). По неполному списку ничего не меняем.
pub fn track(assigned: Option<bool>, listed: bool, complete: bool, done: bool, closed: Option<bool>) -> (Option<bool>, Lifecycle) {
    if listed {
        return (Some(true), if done && assigned == Some(false) { Lifecycle::Reopened } else { Lifecycle::None });
    }
    if !complete {
        return (assigned, Lifecycle::None);
    }
    if assigned != Some(true) || done {
        return (Some(false), Lifecycle::None);
    }
    match closed {
        None => (Some(true), Lifecycle::None),               // не перечитана в этот заход — узнаем в следующий
        Some(true) => (Some(false), Lifecycle::None),        // закрыли — об этом своё уведомление
        Some(false) => (Some(false), Lifecycle::Reassigned), // открыта, но уже не моя
    }
}

/// Номера, которые автообновление не добавляет на доску, и стоит ли их сохранить (`Skip`). Первый запуск (saved — None):
/// всё открытое моё, чего нет на доске, — прошлое, но только по целому списку; дальше — только ещё открытые мои, и снова
/// только по целому списку: неполный «забыл» бы удалённую.
pub fn skip(saved: Option<&[i32]>, listed: &[i32], on_board: &HashSet<i32>, complete: bool) -> (HashSet<i32>, bool) {
    match (saved, complete) {
        (None, true) => (listed.iter().copied().filter(|id| !on_board.contains(id)).collect(), true),
        (None, false) => (listed.iter().copied().collect(), false),
        (Some(saved), true) => {
            let listed: HashSet<i32> = listed.iter().copied().collect();
            (saved.iter().copied().filter(|id| listed.contains(id)).collect(), true)
        }
        (Some(saved), false) => (saved.iter().copied().collect(), false),
    }
}

/// Что положить во «Входящие»: мои открытые, которых нет на доске и которые не в skip (порядок и повторы — как пришли).
pub fn to_add(listed: &[i32], on_board: &HashSet<i32>, skip: &HashSet<i32>) -> Vec<i32> {
    listed.iter().copied().filter(|id| !on_board.contains(id) && !skip.contains(id)).collect()
}

/// Весь список по страницам (`ReadAllPagesAsync`): строки (номера различны), общее число, целый ли и ошибка.
#[derive(Clone, Debug, Default)]
pub struct AllPages {
    pub rows: Vec<Found>,
    pub total: i32,
    pub complete: bool,
    pub error: String,
}

/// Весь список по страницам, fetch(номер с первой). Конец — пустая страница или неполная, когда забрано всё обещанное
/// (Total): одного Total мало (без Paginator он равен пришедшему), одной неполной страницы тоже (сервер вправе отдать за раз
/// меньше). Повтор номера на стыке страниц пропускается; целый — дошли до конца без ошибки и строк не меньше Total. Для
/// выгрузки: stop_at — дальше список не нужен, max_rows — не больше стольких, skip — не из выборки (ни в список, ни в счёт;
/// stop_at проверяется первым). Остановились по ним — список намеренно не целый.
pub fn read_all_pages(
    mut fetch: impl FnMut(i32) -> Result<SearchPage, String>,
    page_size: i32,
    max_pages: i32,
    stop_at: Option<&dyn Fn(&Found) -> bool>,
    max_rows: usize,
    skip: Option<&dyn Fn(&Found) -> bool>,
) -> AllPages {
    let mut all = AllPages::default();
    let mut seen = HashSet::new();
    let mut read: i64 = 0; // строк пришло всего — для правила конца списка
    for page in 1..=max_pages {
        let r = match fetch(page) {
            Ok(r) => r,
            Err(error) => return AllPages { error, ..all },
        };
        read += r.found.len() as i64;
        all.total = all.total.max(r.total);
        let short = (r.found.len() as i64) < i64::from(page_size);
        let empty = r.found.is_empty();
        for f in r.found {
            if !seen.insert(f.id) {
                continue; // список сдвинулся, пока листали: заявка на стыке страниц пришла дважды
            }
            if stop_at.is_some_and(|stop| stop(&f)) {
                return all;
            }
            if skip.is_some_and(|skip| skip(&f)) {
                continue;
            }
            if all.rows.len() >= max_rows {
                return all;
            }
            all.rows.push(f);
        }
        if empty || (read >= i64::from(all.total) && short) {
            all.complete = all.rows.len() as i64 >= i64::from(all.total);
            return all;
        }
        if all.rows.len() >= max_rows {
            return all; // набрали сколько просили — следующая страница не нужна
        }
    }
    all
}

/// Новые комментарии заявки: сколько, «прочитано до» и сами (свежие сверху).
#[derive(Clone, Debug, Default)]
pub struct Unread {
    pub count: usize,
    pub seen: Option<Date>,
    pub unread: Vec<Event>,
}

/// Новые комментарии для бейджа (`Unread`): чужие (не мои — по номеру автора, без него — по имени) и новее seen. Свой ответ
/// сдвигает seen: ответил — значит, прочитал всё до него. seen ещё нет — прочитано всё, что есть. Даты — сервера.
pub fn unread(events: &[Event], seen: Option<Date>, me: Option<&User>) -> Unread {
    let mine = |e: &Event| {
        me.is_some_and(|me| match e.author_id {
            Some(id) => id == me.id,
            None => !me.name.is_empty() && same_name(&e.author, &me.name),
        })
    };
    let comments: Vec<&Event> = events.iter().filter(|e| e.comment.is_some() && e.date.is_some()).collect();
    let Some(seen) = seen else {
        return Unread { count: 0, seen: comments.iter().filter_map(|e| e.date).max(), unread: Vec::new() };
    };
    let since = comments.iter().filter(|e| mine(e)).filter_map(|e| e.date).max().filter(|&mine| mine > seen).unwrap_or(seen);
    let mut unread: Vec<Event> = comments.into_iter().filter(|e| !mine(e) && e.date.is_some_and(|d| d > since)).cloned().collect();
    unread.sort_by_key(|e| std::cmp::Reverse(e.date)); // устойчиво: равные — в порядке ответа
    Unread { count: unread.len(), seen: Some(since), unread }
}

/// «Прочитано до» по Changed заявки, когда переписку ещё не читали, — с запасом в секунду: Changed грубее дат комментариев,
/// и комментарий, который его сдвинул, иначе сошёл бы за новый.
pub fn seen_from(changed: Date) -> Date {
    changed + chrono::Duration::seconds(1)
}

/// Ответ инициатора среди новых чужих комментариев (свежие первыми) — самый свежий его, иначе None. Узнаём по имени;
/// инициатор неизвестен — не решаем. ponytail: тёзку примем за него.
pub fn requester_reply<'a>(fresh: &'a [Event], requester: Option<&str>) -> Option<&'a Event> {
    let requester = requester.filter(|r| !r.trim().is_empty())?;
    fresh.iter().find(|e| same_name(&e.author, requester))
}

/// Один ли человек по имени: без учёта регистра и лишних пробелов.
pub fn same_name(a: &str, b: &str) -> bool {
    let norm = |s: &str| fold(&s.split_whitespace().collect::<Vec<_>>().join(" "));
    norm(a) == norm(b)
}

/// Строка для сравнения без учёта регистра, как `OrdinalIgnoreCase`: каждый символ — заглавным, если заглавная — один символ
/// (у «ß» её нет — остаётся как есть).
pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

/// Названия закрытых статусов (`ClosedStatusNames`; правятся руками: пустые и пробелы — не в счёт, регистр — тоже).
#[derive(Clone, Debug, Default)]
pub struct ClosedNames(HashMap<String, String>);

impl ClosedNames {
    pub fn new(names: &[String]) -> ClosedNames {
        let mut closed = ClosedNames::default();
        for name in names {
            closed.add(name);
        }
        closed
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(&fold(name.trim()))
    }

    /// Ещё одно название (закрытый по признаку справочника — для F5 и отчётов); пустое — нет.
    pub fn add(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            self.0.entry(fold(name)).or_insert_with(|| name.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Названия, как их написали, по алфавиту — для отчёта.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.0.values().map(String::as_str).collect();
        names.sort();
        names
    }
}
