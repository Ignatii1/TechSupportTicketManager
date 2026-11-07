//! Импорт моих и F5 (`MainViewModel.Intraservice.cs`) — правила без окна: что добавить и с каким итогом, какие карточки
//! перечитать, что закрыто, текст итога и вопроса, «Перенести» и «Оставить». Читает сеть и спрашивает окно — `app.rs`.

use crate::board::{Board, Date, Priority, Status, Ticket};
use crate::intraservice::{Found, MyOpen};
use crate::rules::{ClosedNames, seen_from};
use crate::settings::Settings;

pub const IMPORT_TITLE: &str = "Импорт моих заявок";
pub const REFRESH_TITLE: &str = "Обновление статусов";
pub const MOVE_TITLE: &str = "Перенести закрытые в «Готово»?";
pub const NOTHING_TO_REFRESH: &str = "на доске нет заявок с номером — обновлять нечего";

/// Карточка из строки списка (`NewCard`): всё, что в строке есть; переписка до появления на доске — прочитана (новым будет
/// только то, что напишут после этого `Changed`); «моя» — пришла из списка «мои открытые».
pub fn new_card(f: &Found, settings: &Settings, now: Date) -> Ticket {
    let mut t = Ticket::new(now);
    t.title = f.name.clone();
    t.intraservice_id = Some(f.id);
    t.url = settings.ticket_url(f.id);
    t.description = f.description.clone().unwrap_or_default();
    t.external_status = Some(f.status.clone());
    t.last_sync_at = Some(now);
    t.priority = Priority::Mid; // приоритеты сервера в компании не заполняют
    t.creator = f.creator.clone();
    t.creator_phone = f.creator_phone.clone();
    t.creator_email = f.creator_email.clone();
    t.executors = f.executors.clone();
    t.executor_group = f.executor_group.clone();
    t.comments_checked_for = f.changed;
    t.comments_seen_at = f.changed.map(seen_from);
    t.assigned_to_me = Some(true);
    t
}

/// Итог импорта: номера добавленных, сколько уже было, текст для окна.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub added: Vec<i32>,
    pub had: usize,
    pub text: String,
}

/// Импорт (`ImportMine`): мои открытые — в конец «Входящих» те, которых на доске нет; ту, что уже есть (и пришедшую в
/// списке дважды), не трогает вовсе. Молчаливый обрыв хуже недогруза: и ошибка страницы, и потолок — в итоге.
pub fn import(board: &mut Board, mine: &MyOpen, settings: &Settings, now: Date) -> Imported {
    if mine.rows.is_empty() {
        let text = if mine.error.is_empty() {
            "открытых заявок, где вы исполнитель, не нашлось".to_string()
        } else {
            mine.error.clone()
        };
        return Imported { added: Vec::new(), had: 0, text };
    }
    let mut on_board = board.numbers();
    let (mut added, mut had) = (Vec::new(), 0);
    for f in &mine.rows {
        if !on_board.insert(f.id) {
            had += 1;
            continue;
        }
        board.push_inbox(new_card(f, settings, now));
        added.push(f.id);
    }
    let partial = if !mine.error.is_empty() {
        format!("\nЗагружены не все страницы: {}", mine.error)
    } else if !mine.complete {
        format!("\nВзяты первые {} из {} — запустите импорт ещё раз", mine.rows.len(), mine.total)
    } else {
        String::new()
    };
    let text = format!("Добавлено: {}, уже было: {had}{partial}", added.len());
    Imported { added, had, text }
}

/// Какие перечитать по F5: с номером, кроме «Готово», — (ключ, номер), в порядке доски.
pub fn to_refresh(board: &Board) -> Vec<(String, i32)> {
    board.all().filter(|t| t.status != Status::Done).filter_map(|t| Some((t.id.clone(), t.intraservice_id?))).collect()
}

/// Закрытые: названия из настроек и статусы справочника с признаком «выполнена» или «конечный» (`ClosedBy`); справочник не
/// пришёл — только список.
pub fn closed_names(settings: &Settings, statuses: &Result<Vec<crate::intraservice::Status>, String>) -> ClosedNames {
    let mut closed = settings.closed_names();
    if let Ok(list) = statuses {
        let flagged: Vec<String> = list.iter().filter(|s| s.closed_by(&closed)).map(|s| s.name.clone()).collect();
        for name in flagged {
            closed.add(&name);
        }
    }
    closed
}

/// Итог F5: сколько перечитано, первая ошибка с номером, справочник; `show` — ошибка целиком (окно итога) или одной
/// строкой (вопрос: там главное — список).
pub fn summary(fresh: usize, failed: &[(i32, String)], status_error: Option<&str>, show: impl Fn(&str) -> String) -> String {
    let mut text = format!("Обновлено: {fresh}");
    if let Some((id, error)) = failed.first() {
        text.push_str(&format!(", не удалось: {}. Первая ошибка — #{id}: {}", failed.len(), show(error)));
    }
    if let Some(error) = status_error {
        text.push_str(&format!("\n\nСправочник статусов не получен — закрытые определены только по списку:\n{}", show(error)));
    }
    text
}

/// К переносу (`ClosedToMove`): перечитанные сейчас, ещё на доске, не в «Готово», закрытые в Интрасервисе и не оставленные
/// с этим же статусом — иначе каждое F5 спрашивало бы о них снова.
pub fn closed_to_move(board: &Board, fresh: &[String], closed: &ClosedNames) -> Vec<String> {
    fresh
        .iter()
        .filter(|k| {
            board.find(k).is_some_and(|t| {
                t.status != Status::Done
                    && t.external_status.as_deref().is_some_and(|st| closed.contains(st) && t.kept_open_status.as_deref() != Some(st))
            })
        })
        .cloned()
        .collect()
}

/// Вопрос о переносе: итог, потом — какие закрыты (до десяти): «да» на неизвестно что — не согласие.
pub fn move_question(board: &Board, keys: &[String], preface: &str) -> String {
    let lines: Vec<String> = keys.iter().filter_map(|k| board.find(k)).take(10).map(|t| format!("{}  {}", t.number(), t.title)).collect();
    let more = if keys.len() > 10 { format!("\n…и ещё {}", keys.len() - 10) } else { String::new() };
    format!("{preface}Закрыты в Интрасервисе ({}):\n{}{more}", keys.len(), lines.join("\n"))
}

/// «Оставить»: каждая помнит статус, с которым её оставили (`KeptOpenStatus`). true — что-то изменилось.
pub fn keep_open(board: &mut Board, keys: &[String]) -> bool {
    let mut changed = false;
    for k in keys {
        if let Some(t) = board.find_mut(k).filter(|t| t.kept_open_status != t.external_status) {
            t.kept_open_status = t.external_status.clone();
            changed = true;
        }
    }
    changed
}

/// «Перенести»: в «Готово» (наверх) те, что ещё на доске — пока висел вопрос, их могли удалить. Сколько перенесено.
pub fn move_closed(board: &mut Board, keys: &[String], now: Date) -> usize {
    keys.iter().filter(|k| board.move_card(k, Status::Done, now)).count()
}
