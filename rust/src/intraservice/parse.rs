//! Разбор ответов Интрасервиса — `HttpIntraserviceClient.Parse.cs`: все имена полей API — только здесь. Форма json у
//! половины методов в документации не показана (только xml), поэтому разборщики терпят несколько обёрток и пропускают
//! негодные строки, а не весь ответ. Не json или не та форма — None (клиент назовёт это «непонятным ответом»).

use super::errors::auto_title;
use super::html::{html_to_text, legacy_text};
use super::*;
use chrono::{FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc};
use serde_json::Value;
use std::collections::HashMap;

/// Ответ `api/task/{id}?include=status`: `{"Task": {...}, "Statuses": [...]}`, без обёртки — корень. Заявка — с номером
/// (больше нуля) или названием; без названия (из почты без темы) — «Заявка #N» по запрошенному номеру.
pub fn task(json: &str, id: i32) -> Option<Task> {
    let root: Value = serde_json::from_str(json).ok()?;
    if !root.is_object() {
        return None;
    }
    let t = prop(&root, "Task").filter(|t| t.is_object()).unwrap_or(&root);
    let number = int(t, "Id").filter(|&n| n > 0);
    if number.is_none() && text(t, "Name").is_none() {
        return None;
    }
    let description = text(t, "Description");
    Some(Task {
        id: number.unwrap_or(id),
        name: title(t).unwrap_or_else(|| auto_title(id)),
        status: status_of(t, Some(&root)),
        description: description.and_then(html_to_text),
        creator: field(t, "Creator"),
        executors: names(t, "Executors"),
        executor_group: field(t, "ExecutorGroup"),
        changed: date_of(t, "Changed"),
        creator_phone: field(t, "CreatorPhone"),
        creator_email: field(t, "CreatorEmail"),
        extra: extra_of(t, None),
        cut_description: description.and_then(legacy_text),
        created: date_of(t, "Created"),
        creator_position: field(t, "CreatorPosition"),
        observer_ids: ids(t, "ObserverIds"),
        observers: names(t, "Observers"),
        changed_text: text(t, "Changed").map(str::trim).filter(|s| !s.is_empty()).map(String::from),
        status_id: int(t, "StatusId"),
    })
}

/// Права на заявку (`include=usertaskrights`): в какие статусы её можно перевести. Блок `UserTaskRights` или `Rights` —
/// рядом с Task или в нём; `ToStatuses` — строкой номеров, массивом номеров или объектов с Id. Нет блока или списка (или
/// он null) — None: решит сервер; пустой — переходов нет.
pub fn rights(json: &str) -> Option<Vec<i32>> {
    let root: Value = serde_json::from_str(json).ok()?;
    if !root.is_object() {
        return None;
    }
    let holders = [Some(&root), prop(&root, "Task").filter(|t| t.is_object())];
    for holder in holders.into_iter().flatten() {
        for name in ["UserTaskRights", "Rights"] {
            let Some(rights) = prop(holder, name).filter(|r| r.is_object()) else { continue };
            let Some(to) = prop(rights, "ToStatuses").filter(|v| !v.is_null()) else { continue };
            return match to {
                Value::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(|i| match i {
                            Value::Object(_) => int(i, "Id"),
                            Value::Number(n) => n.as_i64().and_then(|n| i32::try_from(n).ok()),
                            Value::String(s) => digits(s),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => ids(rights, "ToStatuses"),
            };
        }
    }
    None
}

/// Справочник (`api/service`, `api/tasktype`, `api/filter`, `api/user`): `{"ServiceList": {"Services": [...],
/// "Paginator": {...}}}`, `{"Services": [...]}` или голый массив. Строка без номера или названия — пропуск. Total —
/// `Paginator.Count` (нет его — сколько пришло), has_more — есть ли следующая страница.
pub fn refs(json: &str, name: &str, wrapper: &str) -> Option<Refs> {
    let root: Value = serde_json::from_str(json).ok()?;
    let (rows, blocks) = unwrap(&root, name, wrapper)?;
    let items = refs_of(rows);
    let total = paginator(blocks).and_then(|p| int(p, "Count")).unwrap_or_else(|| count(items.len()));
    Some(Refs { items, total, has_more: page_info(blocks).is_some_and(|(page, pages)| page < pages) })
}

/// Сервисы из блока Services списка заявок (`include=service`) и сколько заявок на странице. Блока нет — сервисов нет;
/// голый массив (строки его — заявки, не сервисы) или не список — None.
pub fn task_services(json: &str) -> Option<TaskServices> {
    let root: Value = serde_json::from_str(json).ok()?;
    let (rows, blocks) = unwrap(&root, "Tasks", "TaskList")?;
    let services = prop(blocks?, "Services").and_then(Value::as_array).map(|list| refs_of(list)).unwrap_or_default();
    Some(TaskServices { services, rows: rows.len() })
}

/// Страница истории (`api/tasklifetime?include=status`): `{"TaskLifetimeList": {"TaskLifetimes": [...], "Statuses": [...],
/// "Paginator": {...}}}`, обёртка попроще или голый массив. Поля записи: Date, Editor, EditorId, StatusId, Comments, IsPublic.
pub fn lifetime(json: &str) -> Option<LifetimePage> {
    let root: Value = serde_json::from_str(json).ok()?;
    let (rows, blocks) = unwrap(&root, "TaskLifetimes", "TaskLifetimeList")?;
    let events = rows
        .iter()
        .filter(|e| e.is_object())
        .map(|e| Event {
            date: date_of(e, "Date"),
            author: text(e, "Editor").map(|a| a.trim().to_string()).unwrap_or_default(),
            status: status_of(e, blocks),
            comment: text(e, "Comments").and_then(html_to_text),
            is_public: boolean(e, "IsPublic"),
            author_id: int(e, "EditorId"),
            is_final: status_final(e, blocks),
        })
        .collect();
    let pages = page_info(blocks);
    Some(LifetimePage { events, has_more: pages.is_some_and(|(page, pages)| page < pages), paged: pages.is_some() })
}

/// Страница списка заявок (`api/task?…`): `{"TaskList": {"Tasks": [...], "Statuses": [...], "Paginator": {...}}}`,
/// `{"Tasks": [...]}` или голый массив. Строка без номера — пропуск; без названия — «Заявка #N». Общее число — из Paginator,
/// нет его — сколько пришло; `HasNextPage` — в Paginator или рядом со списком.
pub fn search(json: &str) -> Option<SearchPage> {
    let root: Value = serde_json::from_str(json).ok()?;
    let (rows, blocks) = unwrap(&root, "Tasks", "TaskList")?;
    let services = service_names(blocks);
    let found: Vec<Found> = rows
        .iter()
        .filter(|t| t.is_object())
        .filter_map(|t| {
            let id = int(t, "Id")?;
            let description = text(t, "Description");
            Some(Found {
                id,
                name: title(t).unwrap_or_else(|| auto_title(id)),
                status: status_of(t, blocks),
                creator: field(t, "Creator"),
                created: date_of(t, "Created"),
                description: description.and_then(html_to_text),
                executors: names(t, "Executors"),
                executor_group: field(t, "ExecutorGroup"),
                changed: date_of(t, "Changed"),
                creator_phone: field(t, "CreatorPhone"),
                creator_email: field(t, "CreatorEmail"),
                extra: extra_of(t, services.as_ref()),
                creator_id: int(t, "CreatorId"),
                executor_ids: ids(t, "ExecutorIds"),
                cut_description: description.and_then(legacy_text),
                observer_ids: ids(t, "ObserverIds"),
                observers: names(t, "Observers"),
            })
        })
        .collect();
    let p = paginator(blocks);
    let total = p.and_then(|p| int(p, "Count")).unwrap_or_else(|| count(found.len()));
    let has_next = p.and_then(|p| boolean(p, "HasNextPage")).or_else(|| blocks.and_then(|b| boolean(b, "HasNextPage")));
    Some(SearchPage { found, total, has_next })
}

/// Текущий пользователь (`api/user?getcurrentuserinfo=true`): голый объект или обёртка — в документации корень назван
/// `CurrenUserInfo` (буква потеряна в ней самой), понимаем оба написания. Номер больше нуля обязателен.
pub fn current_user(json: &str) -> Option<User> {
    let root: Value = serde_json::from_str(json).ok()?;
    if !root.is_object() {
        return None;
    }
    let wrapper = prop(&root, "CurrenUserInfo").or_else(|| prop(&root, "CurrentUserInfo"));
    let user = wrapper.filter(|w| w.is_object()).unwrap_or(&root);
    let id = int(user, "Id").filter(|&id| id > 0)?;
    Some(User { id, name: text(user, "Name").map(|n| n.trim().to_string()).unwrap_or_default() })
}

/// Статусы (`api/taskstatus`): голый массив, `{"ArrayOfTaskStatusView": {"TaskStatusView": [...]}}` или
/// `{"Statuses": [...]}`. Строка без номера — пропуск.
pub fn statuses(json: &str) -> Option<Vec<Status>> {
    let root: Value = serde_json::from_str(json).ok()?;
    let (rows, _) = unwrap(&root, "TaskStatusView", "ArrayOfTaskStatusView").or_else(|| unwrap(&root, "Statuses", "TaskStatusList"))?;
    Some(
        rows.iter()
            .filter(|s| s.is_object())
            .filter_map(|s| {
                Some(Status {
                    id: int(s, "Id")?,
                    name: text(s, "Name").map(|n| n.trim().to_string()).unwrap_or_default(),
                    is_fixed: boolean(s, "IsFixed").unwrap_or(false),
                    is_final: boolean(s, "IsFinal").unwrap_or(false),
                    is_comment_required: boolean(s, "IsCommentRequired").unwrap_or(false),
                })
            })
            .collect(),
    )
}

/// Статус разобран в название: не пусто и не заглушка «статус N» (имени по номеру не нашлось). Только по такому можно решать
/// «закрыта или открыта». Как `^статус \d+$` у .NET: перевод строки в самом конце — всё ещё заглушка.
pub fn is_resolved_status(status: &str) -> bool {
    if status.trim().is_empty() {
        return false;
    }
    let s = status.strip_suffix('\n').unwrap_or(status);
    !s.strip_prefix("статус ").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Дата из ответа: WCF «/Date(1447335893000)/» (UTC, ±hhmm за числом — не в счёт), формат документации «дд.мм.гггг[ чч:мм[:сс]]»
/// (первым: иначе 12.11 прочиталось бы как 11 декабря), ISO 8601 («T» или пробел, доли до тика, Z или ±чч[:мм]). Без пояса —
/// местное время. Непонятная — None. ponytail: `DateTimeOffset.TryParse` понимает и другие виды («8/27/2026 3:54 PM»);
/// сервер их не шлёт.
pub fn date(text: &str) -> Option<Date> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(ms) = wcf_ms(s) {
        // границы DateTimeOffset: дальше FromUnixTimeMilliseconds бросает — и разбор говорит «нет даты»
        const MIN: i64 = -62_135_596_800_000;
        const MAX: i64 = 253_402_300_799_999;
        return (MIN..=MAX)
            .contains(&ms)
            .then(|| Utc.timestamp_millis_opt(ms).single())
            .flatten()
            .map(|d| d.with_timezone(&Local).fixed_offset());
    }
    if let Some(naive) = documented(s) {
        return local(naive);
    }
    iso(s)
}

// ---------- поля ----------

/// Свойство объекта по имени без учёта регистра (`Prop`). ponytail: при дубле ключа serde оставляет последний, C# брал первый.
fn prop<'a>(e: &'a Value, name: &str) -> Option<&'a Value> {
    let obj = e.as_object()?;
    obj.get(name).or_else(|| obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v))
}

fn text<'a>(e: &'a Value, name: &str) -> Option<&'a str> {
    prop(e, name)?.as_str()
}

/// Целое в 32 бита (`TryGetInt32`): дробное, огромное, строка — None.
fn int(e: &Value, name: &str) -> Option<i32> {
    prop(e, name)?.as_i64().and_then(|n| i32::try_from(n).ok())
}

/// Название строки без пробелов по краям; пустое — None.
fn title(e: &Value) -> Option<String> {
    text(e, "Name").map(str::trim).filter(|n| !n.is_empty()).map(String::from)
}

/// Строковое поле, где «нет в ответе» (None) — не «пусто» (""): json null — «пусто» (`Field`).
fn field(e: &Value, name: &str) -> Option<String> {
    match prop(e, name)? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Null => Some(String::new()),
        _ => None,
    }
}

/// Люди или категории в поле: строка через «,», «;» или «|» («||» карточной формы), массив строк или объектов с Name — к виду
/// «Иванов И. И., Петров П.». Поля нет — None; пусто или json null — "". ponytail: имя с запятой внутри разобьётся на два.
fn names(e: &Value, name: &str) -> Option<String> {
    let parts: Vec<String> = match prop(e, name)? {
        Value::String(s) => s.split([',', ';', '|']).map(String::from).collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| match i {
                Value::String(s) => Some(s.clone()),
                Value::Object(_) => text(i, "Name").map(String::from),
                _ => None,
            })
            .collect(),
        Value::Null => Vec::new(),
        _ => return None,
    };
    Some(parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", "))
}

/// Номера в поле: строка «11, 12», массив чисел или строк с числами (`Ids`). Поля нет — None; пусто или null — пустой
/// список; не число (знак, дробь, слово) — пропуск.
fn ids(e: &Value, name: &str) -> Option<Vec<i32>> {
    let parts: Vec<Option<String>> = match prop(e, name)? {
        Value::String(s) => s.split([',', ';', '|']).map(|p| Some(p.to_string())).collect(),
        Value::Array(items) => items
            .iter()
            .map(|i| match i {
                Value::Number(n) => Some(n.to_string()),
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Value::Null => Vec::new(),
        _ => return None,
    };
    Some(parts.into_iter().filter_map(|p| digits(p?.trim())).collect())
}

/// Только цифры ASCII, в 32 бита (`int.TryParse` с `NumberStyles.None`).
fn digits(s: &str) -> Option<i32> {
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// Логическое: true/false или строкой «True»/«False» (регистр и пробелы по краям — как `bool.TryParse`). Иное — None.
fn boolean(e: &Value, name: &str) -> Option<bool> {
    match prop(e, name)? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => {
            let t = s.trim_matches(|c: char| c.is_whitespace() || c == '\0');
            if t.eq_ignore_ascii_case("true") {
                Some(true)
            } else if t.eq_ignore_ascii_case("false") {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn date_of(e: &Value, name: &str) -> Option<Date> {
    text(e, name).and_then(date)
}

fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

// ---------- блоки рядом со списком ----------

/// Название статуса строки: своё StatusName, иначе по StatusId из блока Statuses (`include=status`), иначе «статус N»;
/// статуса нет вовсе — "".
fn status_of(row: &Value, blocks: Option<&Value>) -> String {
    let own = text(row, "StatusName").filter(|n| !n.trim().is_empty());
    let name = match (own, int(row, "StatusId")) {
        (Some(n), _) => Some(n.to_string()),
        (None, Some(id)) => Some(
            statuses_block(blocks)
                .and_then(|list| list.iter().find(|s| s.is_object() && int(s, "Id") == Some(id)))
                .and_then(|s| text(s, "Name"))
                .map_or_else(|| format!("статус {id}"), String::from),
        ),
        (None, None) => text(row, "StatusName").map(String::from),
    };
    name.map(|n| n.trim().to_string()).unwrap_or_default()
}

/// У статуса строки признак «выполнена» или «конечный» — из блока Statuses того же ответа; блока или статуса в нём нет — нет.
fn status_final(row: &Value, blocks: Option<&Value>) -> bool {
    let Some(id) = int(row, "StatusId") else { return false };
    statuses_block(blocks).is_some_and(|list| {
        list.iter().any(|s| {
            s.is_object() && int(s, "Id") == Some(id) && (boolean(s, "IsFinal") == Some(true) || boolean(s, "IsFixed") == Some(true))
        })
    })
}

fn statuses_block(blocks: Option<&Value>) -> Option<&Vec<Value>> {
    prop(blocks?, "Statuses")?.as_array()
}

/// Строки ответа и объект, рядом с которым лежат блоки Statuses и Paginator: `{"Обёртка": {"Имя": [...]}}`, `{"Имя": [...]}`
/// или голый массив (у него блоков нет). None — не тот ответ.
fn unwrap<'a>(root: &'a Value, name: &str, wrapper: &str) -> Option<(&'a [Value], Option<&'a Value>)> {
    match root {
        Value::Array(rows) => Some((rows, None)),
        Value::Object(_) => {
            let blocks = prop(root, wrapper).filter(|w| w.is_object()).unwrap_or(root);
            prop(blocks, name)?.as_array().map(|rows| (rows.as_slice(), Some(blocks)))
        }
        _ => None,
    }
}

/// Блок Paginator: объект или (в одном примере документации) массив из одного объекта.
fn paginator(blocks: Option<&Value>) -> Option<&Value> {
    let p = prop(blocks?, "Paginator")?;
    match p {
        Value::Object(_) => Some(p),
        Value::Array(items) => items.iter().find(|i| i.is_object()),
        _ => None,
    }
}

/// Номер страницы и число страниц; нет обоих — None: о следующей странице блок ничего не говорит.
fn page_info(blocks: Option<&Value>) -> Option<(i32, i32)> {
    let p = paginator(blocks)?;
    Some((int(p, "Page")?, int(p, "PageCount")?))
}

/// Строки справочника; без номера или названия — пропуск.
fn refs_of(rows: &[Value]) -> Vec<Ref> {
    rows.iter()
        .filter(|r| r.is_object())
        .filter_map(|r| {
            let id = int(r, "Id")?;
            let name = text(r, "Name")?.trim();
            (!name.is_empty()).then(|| Ref {
                id,
                name: name.to_string(),
                path: text(r, "Path").map(|p| p.trim().to_string()),
                is_archive: boolean(r, "IsArchive").unwrap_or(false),
                is_default: boolean(r, "IsDefault").unwrap_or(false),
            })
        })
        .collect()
}

/// Названия сервисов из блока Services списка заявок: номер → название (позднее главнее).
fn service_names(blocks: Option<&Value>) -> Option<HashMap<i32, String>> {
    let list = prop(blocks?, "Services")?.as_array()?;
    Some(refs_of(list).into_iter().map(|s| (s.id, s.name)).collect())
}

/// Сервис, тип, категории, решена, организация, оборудование, вложения, закрыта — для выгрузки и панели. Сервис —
/// ServiceName, иначе по ServiceId из блока Services. Ни одного поля — None.
fn extra_of(t: &Value, services: Option<&HashMap<i32, String>>) -> Option<Extra> {
    let service = match field(t, "ServiceName") {
        Some(named) if !named.is_empty() => Some(named),
        _ => int(t, "ServiceId").and_then(|id| services?.get(&id).cloned()),
    };
    let x = Extra {
        service,
        task_type: field(t, "Type"),
        categories: names(t, "Categories"),
        resolved: date_of(t, "ResolutionDateFact"),
        company: field(t, "CreatorCompanyName"),
        assets: names(t, "Assets"),
        files: files_of(t),
        closed: date_of(t, "Closed"),
    };
    (x != Extra::default()).then_some(x)
}

/// Вложения: номера из FileIds, имена из FileNames через «||» (запятая — часть имени). Имён не столько, сколько номеров, —
/// номера без имён; номер дважды — одно вложение. FileIds нет — None (не прислано); пусто — вложений нет.
fn files_of(t: &Value) -> Option<Vec<TaskFile>> {
    let numbers = ids(t, "FileIds")?;
    let names: Option<Vec<String>> =
        field(t, "FileNames").filter(|n| !n.is_empty()).map(|n| n.split("||").map(|p| p.trim().to_string()).collect());
    let names = names.filter(|n| n.len() == numbers.len());
    let mut seen = std::collections::HashSet::new();
    Some(
        numbers
            .iter()
            .enumerate()
            .filter(|&(_, id)| seen.insert(*id))
            .map(|(i, &id)| TaskFile { id, name: names.as_ref().map(|n| n[i].clone()).filter(|n| !n.is_empty()) })
            .collect(),
    )
}

// ---------- даты ----------

/// Число из «/Date(…)/» — `^/Date\((-?\d+)(?:[+-]\d{4})?\)/$`; не тот вид или не влезает в 64 бита — None.
fn wcf_ms(s: &str) -> Option<i64> {
    let inner = s.strip_prefix("/Date(")?.strip_suffix(")/")?;
    let b = inner.as_bytes();
    let start = usize::from(b.first() == Some(&b'-'));
    let end = start + b[start..].iter().take_while(|c| c.is_ascii_digit()).count();
    let zone = &b[end..];
    let zone_ok = zone.is_empty() || (zone.len() == 5 && matches!(zone[0], b'+' | b'-') && zone[1..].iter().all(u8::is_ascii_digit));
    (end > start && zone_ok).then(|| inner[..end].parse().ok()).flatten()
}

/// «дд.мм.гггг», «дд.мм.гггг чч:мм», «дд.мм.гггг чч:мм:сс» — ровно столько цифр, как `TryParseExact`.
fn documented(s: &str) -> Option<NaiveDateTime> {
    let b = s.as_bytes();
    if !matches!(b.len(), 10 | 16 | 19) || b[2] != b'.' || b[5] != b'.' {
        return None;
    }
    let (day, month, year) = (number(b, 0, 2)?, number(b, 3, 2)?, number(b, 6, 4)?);
    let (mut hour, mut minute, mut second) = (0, 0, 0);
    if b.len() >= 16 {
        if b[10] != b' ' || b[13] != b':' {
            return None;
        }
        (hour, minute) = (number(b, 11, 2)?, number(b, 14, 2)?);
    }
    if b.len() == 19 {
        if b[16] != b':' {
            return None;
        }
        second = number(b, 17, 2)?;
    }
    (year > 0).then(|| NaiveDate::from_ymd_opt(year as i32, month, day)?.and_hms_opt(hour, minute, second)).flatten()
}

/// ISO 8601: «гггг-мм-дд», за ним «T» или пробел и «чч:мм[:сс[.доли]]», за ними «Z» или «±чч[[:]мм]».
fn iso(s: &str) -> Option<Date> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (year, month, day) = (number(b, 0, 4)?, number(b, 5, 2)?, number(b, 8, 2)?);
    let (mut hour, mut minute, mut second, mut nanos) = (0, 0, 0, 0i64);
    let mut i = 10;
    if i < b.len() && matches!(b[i], b'T' | b't' | b' ') {
        if b.len() < i + 6 || b[i + 3] != b':' {
            return None;
        }
        (hour, minute) = (number(b, i + 1, 2)?, number(b, i + 4, 2)?);
        i += 6;
        if b.get(i) == Some(&b':') {
            second = number(b, i + 1, 2)?;
            i += 3;
            if b.get(i) == Some(&b'.') {
                let n = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
                if n == 0 {
                    return None;
                }
                nanos = ticks(&b[i + 1..i + 1 + n]) * 100;
                i += 1 + n;
            }
        }
    }
    let naive = (year > 0).then(|| NaiveDate::from_ymd_opt(year as i32, month, day)?.and_hms_opt(hour, minute, second)).flatten()?
        + chrono::Duration::nanoseconds(nanos);
    let zone = &s[i..];
    if zone.is_empty() {
        return local(naive);
    }
    let offset = if zone.eq_ignore_ascii_case("z") {
        0
    } else {
        let z = zone.as_bytes();
        let sign = match z[0] {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let (h, m) = match z.len() {
            3 => (number(z, 1, 2)?, 0),
            5 => (number(z, 1, 2)?, number(z, 3, 2)?),
            6 if z[3] == b':' => (number(z, 1, 2)?, number(z, 4, 2)?),
            _ => return None,
        };
        if h > 14 || m > 59 {
            return None;
        }
        sign * (h as i32 * 3600 + m as i32 * 60)
    };
    FixedOffset::east_opt(offset)?.from_local_datetime(&naive).single()
}

/// Доли секунды в тиках (100 нс), как .NET: до семи знаков точно, дальше — округление (половина — к чётному).
fn ticks(digits: &[u8]) -> i64 {
    let value = |d: &[u8]| d.iter().fold(0i64, |v, c| v * 10 + i64::from(c - b'0'));
    let mut t = value(&digits[..digits.len().min(7)]);
    for _ in digits.len()..7 {
        t *= 10;
    }
    if digits.len() > 7 {
        let rest = &digits[7..];
        let half = rest[0]
            .cmp(&b'5')
            .then_with(|| if rest[1..].iter().any(|&c| c != b'0') { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
        if half.is_gt() || (half.is_eq() && t % 2 == 1) {
            t += 1;
        }
    }
    t
}

/// Ровно `len` цифр ASCII с позиции i.
fn number(b: &[u8], i: usize, len: usize) -> Option<u32> {
    let part = b.get(i..i + len)?;
    part.iter().all(u8::is_ascii_digit).then(|| part.iter().fold(0, |v, c| v * 10 + u32::from(c - b'0')))
}

/// Время без пояса — по смещению этого компьютера (`AssumeLocal`). Двойное (осенний перевод) — по зимнему, как .NET; в
/// пропуске (весенний) — смещение до него.
fn local(naive: NaiveDateTime) -> Option<Date> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(d) => Some(d.fixed_offset()),
        LocalResult::Ambiguous(a, b) => {
            Some(if a.offset().local_minus_utc() <= b.offset().local_minus_utc() { a } else { b }.fixed_offset())
        }
        LocalResult::None => {
            let before = Local.from_local_datetime(&(naive - chrono::Duration::hours(3))).earliest()?;
            before.offset().fix().from_local_datetime(&naive).single()
        }
    }
}
