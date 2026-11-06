//! Что синхронизация с Интрасервисом меняет в карточке (`MainViewModel.Apply`/`ApplyCard`): статус Интрасервиса, люди,
//! поля карточки и вложения — что прислал сервер (поля нет в ответе — остаётся, что было); название — только пока оно
//! автоматическое «Заявка #N»; описание — только пустое или обрезанное разбором до 0.15.1. Колонку, заметки и приоритет
//! не трогает.

use crate::board::{Date, Ticket, TicketFile};
use crate::intraservice::{Task, auto_title};
use serde_json::Map;

pub fn apply(t: &mut Ticket, n: i32, x: &Task, now: Date) {
    if t.title == auto_title(n) && !x.name.is_empty() {
        t.title = x.name.clone();
    }
    // описание — только пустое; или ровно то, что разбор до 0.15.1 сделал из того же текста сервера (он принимал «< 5 шт,
    // заказ >» за тег и терял кусок): руками такое не правили — чиним раз
    if let Some(d) = x.description.as_deref().filter(|d| !d.is_empty())
        && (t.description.trim().is_empty() || (t.description != d && x.cut_description.as_deref() == Some(t.description.as_str())))
    {
        t.description = d.to_string();
    }
    t.external_status = Some(x.status.clone());
    apply_card(t, x);
    t.last_sync_at = Some(now);
}

fn set(field: &mut Option<String>, value: &Option<String>) {
    if value.is_some() {
        field.clone_from(value);
    }
}

/// Люди, поля карточки и вложения. Строка списка (`extra` нет) полей карточки и вложений не несёт — их не стирает.
pub fn apply_card(t: &mut Ticket, x: &Task) {
    set(&mut t.creator, &x.creator);
    set(&mut t.creator_phone, &x.creator_phone);
    set(&mut t.creator_email, &x.creator_email);
    set(&mut t.creator_position, &x.creator_position);
    set(&mut t.executors, &x.executors);
    set(&mut t.executor_group, &x.executor_group);
    let Some(e) = &x.extra else { return };
    set(&mut t.creator_company, &e.company);
    set(&mut t.service, &e.service);
    set(&mut t.task_type, &e.task_type);
    set(&mut t.categories, &e.categories);
    set(&mut t.assets, &e.assets);
    // список целиком: вложения на сервере удаляют, а номер файла не меняется — тот же список не переписываем
    if let Some(files) = &e.files {
        let same = t
            .files
            .as_ref()
            .is_some_and(|old| old.len() == files.len() && old.iter().zip(files).all(|(a, b)| a.id == b.id && a.name == b.name));
        if !same {
            t.files = Some(files.iter().map(|f| TicketFile { id: f.id, name: f.name.clone(), other: Map::new() }).collect());
        }
    }
}
