//! Один заход автовыгрузки закрытых заявок (`ClosedExport.RunAsync`): страница списка после отметки, строки по порядку,
//! не больше `budget` заявок с запросом, взятые — тем же движком, что и окно поиска; отметка — за разобранными подряд.

use super::closed::{ExportFailure, ExportFailures, ExportMark, ExportScope, before_mark, disorder, of_ticket, page_size};
use super::run::{Options, export_rows, read_lifetime, unchanged};
use crate::intraservice::{Client, Date, Found, capped};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Итог захода (`ClosedExportPass`).
#[derive(Clone, Debug)]
pub struct ClosedExportPass {
    pub next: ExportMark,
    pub not_mine: BTreeMap<i32, Date>,
    pub failures: ExportFailures,
    pub exported: usize,
    pub more: bool,
    pub error: String,
    pub given_up: Vec<ExportFailure>,
    pub waited: Duration,
}

/// Что решено о строке страницы (`Verdict`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Take,
    NotMine,
    Current,
    Failed,
    GivenUp,
}

/// Заход (`RunAsync`): одна страница списка после отметки; строки по порядку — разобранные, верные, пропущенные, мои по строке
/// или по истории, известные «не мои»; не больше `budget` заявок, которым нужен запрос; взятые выгружаются тем же движком, что
/// и в окне поиска; отметка — за разобранными подряд, сбойные считаются и пропускаются.
#[allow(clippy::too_many_arguments)]
pub fn run_pass(
    client: &Client,
    scope: &ExportScope,
    mark: &ExportMark,
    not_mine: &BTreeMap<i32, Date>,
    failures: &ExportFailures,
    dir: &Path,
    ticket_url: &(dyn Fn(i32) -> String + Sync),
    budget: usize,
    now: Date,
    stop: &AtomicBool,
    options: &Options,
) -> ClosedExportPass {
    let stopped = |error: String| ClosedExportPass {
        next: mark.clone(),
        not_mine: not_mine.clone(),
        failures: failures.clone(),
        exported: 0,
        more: false,
        error,
        given_up: Vec::new(),
        waited: Duration::ZERO,
    };

    // одна страница за заход, одним запросом — снимок: читай мы дальше, заявка, изменённая между страницами, сдвинула бы список,
    // и строка со следующей страницы ускользнула бы на прочитанную. Страница — 500 строк и номера минуты отметки (они придут
    // снова); следующий заход начнёт с новой отметки
    let size = page_size(mark);
    let list = match client.get_tasks_page(&scope.query(mark), size as i32, options.list_timeout) {
        Ok(list) => list,
        Err(error) => return stopped(error),
    };
    // порядок и даты — до разбора: строка не по порядку, разобранная первой, увела бы отметку через более раннюю
    let wrong = disorder(&list.found);
    if !wrong.is_empty() {
        return stopped(wrong);
    }
    // вся страница раньше отметки, и это не сдвиг поясов (страница полная или уходит дальше него) — сервер не применил условие
    // по дате: так и сказать, а не молча пропускать страницу за страницей или вечно отвечать «новых нет»
    let beyond = list.found.len() >= size || list.has_next == Some(true) || list.total > list.found.len() as i32 || capped(list.total);
    let early = before_mark(&list.found, mark, beyond);
    if !early.is_empty() {
        return stopped(early);
    }

    // уже верные файлы (полная выгрузка из «Поиска заявок», прошлый заход) — ни запросов, ни места в порции: правило пропуска самой
    // выгрузки, по диску, по строке — только тех, до кого заход дойдёт; только чтение — вне очереди писать
    let uncovered: Vec<Found> = list.found.iter().filter(|r| !mark.covers(r)).cloned().collect();
    let mut current = unchanged(dir, &uncovered);
    let mut known = not_mine.clone();
    let mut rows: Vec<(&Found, Verdict)> = Vec::new(); // разобранные — по порядку списка
    let mut missed: BTreeMap<i32, (String, bool)> = BTreeMap::new(); // номер → (что сказать, в счёт ли заявке)
    let (mut cost, mut more, mut listed) = (0, false, HashSet::new());
    for row in &list.found {
        if mark.covers(row) || !listed.insert(row.id) {
            continue;
        }
        if current(row, scope.current_for(row)) {
            rows.push((row, Verdict::Current));
            continue;
        }
        if failures.given_up(row, now) {
            rows.push((row, Verdict::GivenUp));
            continue;
        }
        let Some(changed) = row.changed else { continue }; // `disorder` строки без даты уже отверг
        let mut take = scope.takes_by_row(row);
        let known_not_mine = !take && known.get(&row.id) == Some(&changed);
        if !known_not_mine {
            if cost == budget {
                more = true;
                break;
            }
            cost += 1;
        }
        if !take && !known_not_mine {
            let (events, error, truncated) = read_lifetime(client, row.id);
            if !error.is_empty() {
                let own = of_ticket(&error);
                missed.insert(row.id, (format!("#{}: история не читается — {error}", row.id), own));
                rows.push((row, Verdict::Failed));
                if own {
                    continue;
                }
                break; // сеть или учётная запись: остальные упадут так же — до следующего захода
            }
            // история обрезана (длиннее, чем читаем) — моё действие могло остаться в непрочитанной, ранней части: выгружаем, а не
            // записываем в «не мои» навсегда (лишний файл лучше потерянной заявки)
            take = truncated || scope.mine_by_history(&events);
            if !take {
                known.insert(row.id, changed);
            }
        }
        rows.push((row, if take { Verdict::Take } else { Verdict::NotMine }));
    }

    let matched: Vec<Found> = rows.iter().filter(|(_, v)| *v == Verdict::Take).map(|(r, _)| (*r).clone()).collect();
    // выгрузка не началась — `tickets/` не создать: у C# это исключение внутри очереди, заход «не удался» и ничего не сдвинул (ни
    // отметку, ни «не мои», ни счёт сбойных): строки до взятых не считаются разобранными
    if !matched.is_empty()
        && let Err(e) = std::fs::create_dir_all(dir.join(super::files::TICKETS_FOLDER))
    {
        return stopped(format!("Не удалось записать в папку {}:\n{e}", dir.display()));
    }
    let export = (!matched.is_empty()).then(|| {
        export_rows(client, &matched, dir, ticket_url, &scope.context(), &|_| {}, stop, &Options { listens: false, ..options.clone() })
    });
    // файл не записался: временный рядом записался, а на месте файла — папка или файл только для чтения — беда этой заявки, в
    // счёт; не записался и временный — беда папки (диск полон, нет доступа): ждём, не пропускаем
    for row in matched.iter().filter(|r| !export.as_ref().is_some_and(|e| e.current.contains(&r.id))) {
        let entry = match export.as_ref() {
            Some(e) => match e.errors.get(&row.id) {
                Some(error) => (
                    format!("не выгружена #{}: {error}", row.id),
                    if e.not_written.contains(&row.id) { e.path_blocked.contains(&row.id) } else { of_ticket(error) },
                ),
                // своей ошибки нет — выгрузку остановил выключатель (логин, сеть): причина — в её итоге, и она не в счёт
                None => (format!("не выгружена #{}: {}", row.id, if e.error.is_empty() { "файл не записан" } else { &e.error }), false),
            },
            None => (format!("не выгружена #{}: файл не записан", row.id), false),
        };
        missed.insert(row.id, entry);
    }

    // отметка — за разобранными подряд: неразобранная или моя, чей файл не стал актуальным, останавливает её на себе
    let done: Vec<&Found> = rows.iter().take_while(|(r, _)| !missed.contains_key(&r.id)).map(|(r, _)| *r).collect();
    let next = mark.after(&done);
    let mut error = rows.iter().find_map(|(r, _)| missed.get(&r.id)).map(|(message, _)| message.clone()).unwrap_or_default();
    if error.is_empty() && beyond && rows.is_empty() {
        error = format!(
            "в минуту {} изменилось больше заявок, чем сервер отдаёт за раз ({}), и все отданные уже разобраны — отметка дальше не сдвинется",
            ExportMark::minute(mark.since).format("%d.%m.%Y %H:%M"),
            list.found.len()
        );
    }
    let minute = ExportMark::minute(next.since);
    let keep: BTreeMap<i32, Date> = known.into_iter().filter(|(_, at)| *at >= minute).collect();
    // пропущенная остаётся в счёте, пока отметка её не прошла (впереди споткнулась другая): следующий заход пропустит её снова,
    // без запросов, а в лог она попадёт один раз — когда отметка пройдёт
    let failed: Vec<(&Found, String)> =
        rows.iter().filter_map(|(r, _)| missed.get(&r.id).filter(|(_, own)| *own).map(|(message, _)| (*r, message.clone()))).collect();
    let passed: Vec<i32> = rows
        .iter()
        .enumerate()
        .filter(|(i, (r, v))| !missed.contains_key(&r.id) && (*v != Verdict::GivenUp || *i < done.len()))
        .map(|(_, (r, _))| r.id)
        .collect();
    ClosedExportPass {
        next,
        not_mine: keep,
        failures: failures.after(&failed, &passed, now),
        exported: export.as_ref().map_or(0, |e| e.created + e.updated),
        more: (more || beyond) && error.is_empty(),
        error,
        given_up: rows
            .iter()
            .take(done.len())
            .filter(|(_, v)| *v == Verdict::GivenUp)
            .filter_map(|(r, _)| failures.items.get(&r.id).cloned())
            .collect(),
        waited: export.map_or(Duration::ZERO, |e| e.waited),
    }
}
