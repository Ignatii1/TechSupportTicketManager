//! Правила автовыгрузки закрытых заявок и зеркала (карточка 4.5-1; `ClosedExport.cs`, `AppSettings.cs`): пять таблиц сверки
//! (ответы пишет код C#) и `ClosedExport.SelfCheck` один в один — там, где таблица не держит типы, состояние и порядок.

mod common;

use chrono::{Duration as Span, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone};
use std::time::Duration;
use ticketboard::export::AttachmentScope;
use ticketboard::export::closed::*;
use ticketboard::intraservice::query::CHANGED_ASC_SORT;
use ticketboard::intraservice::{Date, Event, Found, User};
use ticketboard::rules::ClosedNames;
use ticketboard::settings::{Settings, parse_settings};

// ---------------------------------------------------------------------------------------------------------------------
// Помощники
// ---------------------------------------------------------------------------------------------------------------------

fn yes(b: bool) -> &'static str {
    if b { "да" } else { "нет" }
}

fn or_empty(s: String) -> String {
    if s.is_empty() { "(пусто)".to_string() } else { s }
}

/// Местное время, как `new DateTimeOffset(DateTime.Parse(…))` в C#: дата без пояса — по смещению этого компьютера.
fn local_naive(n: NaiveDateTime) -> Date {
    Local.from_local_datetime(&n).single().unwrap().fixed_offset()
}

fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Date {
    local_naive(NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, s).unwrap())
}

/// Дата из ячейки таблицы: «2026-10-01T12:05:30», с долями секунды или без.
fn cell_date(text: &str) -> Date {
    local_naive(NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").unwrap_or_else(|e| panic!("дата «{text}»: {e}")))
}

fn ids(text: &str) -> Vec<i32> {
    text.split(',').filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect()
}

/// 2026-10-01 12:05:30 — как `at` в `ClosedExport.SelfCheck`.
fn at() -> Date {
    local(2026, 10, 1, 12, 5, 30)
}

fn secs(n: i64) -> Span {
    Span::seconds(n)
}

fn mins(n: i64) -> Span {
    Span::minutes(n)
}

fn me() -> User {
    User { id: 7, name: "Я Сам".into() }
}

/// `new ExportScope(new[] { 29, 30 }, me, new HashSet<string>())`: охват «мои», вложения — не качать.
fn scope() -> ExportScope {
    ExportScope {
        closed_status_ids: vec![29, 30],
        me: me(),
        closed_names: ClosedNames::default(),
        attachments: AttachmentScope::None,
        reach: ExportReach::Mine,
    }
}

fn with_reach(reach: ExportReach) -> ExportScope {
    ExportScope { reach, ..scope() }
}

/// `Row(id, changed, creatorId, executorIds, creator, executors)` из SelfCheck.
fn row(id: i32, changed: Option<Date>) -> Found {
    Found { id, name: "x".into(), status: "Закрыта".into(), changed, ..Found::default() }
}

fn row_by(id: i32, changed: Option<Date>, creator_id: Option<i32>, executor_ids: Option<&[i32]>) -> Found {
    Found { creator_id, executor_ids: executor_ids.map(<[i32]>::to_vec), ..row(id, changed) }
}

fn named(id: i32, creator: Option<&str>, executors: Option<&str>) -> Found {
    Found { creator: creator.map(String::from), executors: executors.map(String::from), ..row(id, None) }
}

/// `E(author, id, text)` из SelfCheck: запись истории со сменой статуса.
fn event(author: &str, id: Option<i32>, text: Option<&str>) -> Event {
    Event {
        date: Some(at()),
        author: author.into(),
        status: "Закрыта".into(),
        comment: text.map(String::from),
        is_public: Some(true),
        author_id: id,
        is_final: true,
    }
}

fn mark_at(since: Date, seen: &[i32]) -> ExportMark {
    ExportMark { since, seen: seen.to_vec() }
}

/// `new ExportMark(new DateTimeOffset(at), new[] { 5 })` из SelfCheck.
fn mark() -> ExportMark {
    mark_at(at(), &[5])
}

fn seen_of(m: &ExportMark) -> Vec<i32> {
    m.seen.clone()
}

// ---------------------------------------------------------------------------------------------------------------------
// Таблицы сверки
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn how_long_the_pass_took_is_written_like_in_the_app() {
    // секунды захода → хвост строки трея; короче секунды — ничего
    let wrong = common::parity_mismatches("autoexport-took", |c| or_empty(took(Duration::from_secs_f64(c[0].parse().expect("секунды")))));
    assert!(wrong.is_empty(), "хвост «за N» не как у C#:\n{}", wrong.join("\n"));
}

#[test]
fn which_errors_are_the_tickets_own() {
    let wrong = common::parity_mismatches("autoexport-of-ticket", |c| yes(of_ticket(&c[0])).to_string());
    assert!(wrong.is_empty(), "«в счёт пропуска ли» не как у C#:\n{}", wrong.join("\n"));
}

#[test]
fn the_list_page_query_is_the_one_of_the_app() {
    // охват, закрытые статусы, отметка (часы ПК), номера её минуты → адрес первой страницы захода
    let wrong = common::parity_mismatches("autoexport-query", |c| {
        let reach = ExportReach::parse(&c[0]).expect("охват");
        let scope = ExportScope { closed_status_ids: ids(&c[1]), ..with_reach(reach) };
        let mark = mark_at(cell_date(&c[2]), &ids(&c[3]));
        scope.query(&mark).to_url(1, page_size(&mark) as i32, false, true)
    });
    assert!(wrong.is_empty(), "адрес страницы не как у C#:\n{}", wrong.join("\n"));
}

#[test]
fn what_is_wrong_with_a_list_page_is_said_like_in_the_app() {
    // отметка, строки «номер@изменение» («-» — без даты), полная ли → «порядок: …» или «отметка: …» или «(пусто)»
    let wrong = common::parity_mismatches("autoexport-page", |c| {
        let mark = mark_at(cell_date(&c[0]), &[]);
        let rows: Vec<Found> = c[1]
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|r| {
                let (id, changed) = r.split_once('@').expect("номер@изменение");
                row(id.parse().unwrap(), (changed != "-").then(|| cell_date(changed)))
            })
            .collect();
        let order = disorder(&rows);
        if !order.is_empty() {
            return format!("порядок: {order}");
        }
        let early = before_mark(&rows, &mark, c[2] == "да");
        if early.is_empty() { "(пусто)".to_string() } else { format!("отметка: {early}") }
    });
    assert!(wrong.is_empty(), "страница списка оценена не как у C#:\n{}", wrong.join("\n"));
}

#[test]
fn the_three_settings_are_read_like_net_reads_them() {
    // ключ и значение в settings.json → «AutoExportClosed|AutoExportReach|AutoSyncMinutes» или ОШИБКА (файл не читается)
    let wrong = common::parity_mismatches("settings-autoexport", |c| match parse_settings(&format!("{{\"{}\": {}}}", c[0], c[1])) {
        Ok(s) => format!("{}|{}|{}", if s.auto_export_closed { "True" } else { "False" }, s.auto_export_reach, s.auto_sync_minutes),
        Err(_) => "ОШИБКА".to_string(),
    });
    assert!(wrong.is_empty(), "настройки читаются не как у C#:\n{}", wrong.join("\n"));
}

// ---------------------------------------------------------------------------------------------------------------------
// Настройки и охват как значение
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_settings_default_like_app_settings() {
    // C#: AutoSyncMinutes = 5, AutoExportClosed = false, AutoExportReach = Mine; нет поля в файле — то же
    let d = Settings::default();
    assert!(!d.auto_export_closed, "автовыгрузка по умолчанию выключена");
    assert_eq!(d.auto_export_reach, ExportReach::Mine, "охват по умолчанию — мои");
    assert_eq!(d.auto_sync_minutes, 5, "автообновление по умолчанию — раз в 5 минут");
    let s = parse_settings(r#"{"WipLimit": 9}"#).unwrap();
    assert!(
        !s.auto_export_closed && s.auto_export_reach == ExportReach::Mine && s.auto_sync_minutes == 5,
        "поля не названы — по умолчанию"
    );
    assert_eq!(ExportReach::default(), ExportReach::Mine, "значение по умолчанию у типа — мои");
}

#[test]
fn the_three_settings_come_from_the_file_independently() {
    let s = parse_settings(r#"{"AutoExportClosed": true, "AutoExportReach": "All", "AutoSyncMinutes": 15, "WipLimit": 9}"#).unwrap();
    assert!(s.auto_export_closed, "AutoExportClosed: true");
    assert_eq!(s.auto_export_reach, ExportReach::All, "AutoExportReach: All");
    assert_eq!(s.auto_sync_minutes, 15, "AutoSyncMinutes: 15");
    assert_eq!(s.wip_limit, 9, "остальные поля читаются как прежде");
    // комментарии и висячая запятая — как у остальных настроек
    let s = parse_settings("{\"AutoSyncMinutes\": 2, // раз в две минуты\n}").unwrap();
    assert_eq!(s.auto_sync_minutes, 2, "комментарий и запятая в файле прощаются");
}

#[test]
fn the_reach_is_written_and_read_like_a_net_enum() {
    let shown: Vec<String> = [ExportReach::Mine, ExportReach::Closed, ExportReach::All, ExportReach::Other(3), ExportReach::Other(-1)]
        .iter()
        .map(|r| r.to_string())
        .collect();
    assert_eq!(shown, ["Mine", "Closed", "All", "3", "-1"], "ToString() у enum C#");
    assert_eq!(
        [0, 1, 2, 3, -1].map(ExportReach::from_number),
        [ExportReach::Mine, ExportReach::Closed, ExportReach::All, ExportReach::Other(3), ExportReach::Other(-1)],
        "число: 0, 1, 2 — имена, иное — без имени"
    );
    assert_eq!(ExportReach::parse("closed"), Some(ExportReach::Closed), "имя без учёта регистра");
    assert_eq!(ExportReach::parse(" All "), Some(ExportReach::All), "пробелы по краям не мешают");
    assert_eq!(ExportReach::parse("Closed, All"), Some(ExportReach::Other(3)), "имена через запятую — как флаги");
    assert_eq!(ExportReach::parse("2"), Some(ExportReach::All), "число в строке");
    assert_eq!(ExportReach::parse("Everything"), None, "незнакомое имя");
    assert_eq!(ExportReach::parse(""), None, "пустая строка");
}

// ---------------------------------------------------------------------------------------------------------------------
// ClosedExport.SelfCheck: запрос, «моё ли», вложения
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn the_attachments_of_the_pass_are_only_mine() {
    // автовыгрузка берёт только мои (своим правилом, с урезанной историей тоже), так что «моих» — это все её
    let of = |reach, setting| ExportScope { reach, attachments: setting, ..scope() }.context().attachments;
    assert_eq!(of(ExportReach::Mine, AttachmentScope::Mine), AttachmentScope::All, "вложения: Mine, Mine");
    assert_eq!(of(ExportReach::Mine, AttachmentScope::None), AttachmentScope::None, "вложения: Mine, None");
    assert_eq!(scope().context().attachments, AttachmentScope::None, "вложения автовыгрузки: по умолчанию не качать");
    // мои закрытые — «моих» значит все их; зеркало — только моих, даже с «всех выгружаемых»; «не скачивать» — ничего
    for (reach, setting, want) in [
        (ExportReach::Mine, AttachmentScope::Mine, AttachmentScope::All),
        (ExportReach::Mine, AttachmentScope::All, AttachmentScope::All),
        (ExportReach::Closed, AttachmentScope::All, AttachmentScope::Mine),
        (ExportReach::All, AttachmentScope::Mine, AttachmentScope::Mine),
        (ExportReach::All, AttachmentScope::All, AttachmentScope::Mine),
        (ExportReach::Closed, AttachmentScope::None, AttachmentScope::None),
        (ExportReach::Mine, AttachmentScope::None, AttachmentScope::None),
    ] {
        assert_eq!(of(reach, setting), want, "вложения: {reach}, {setting}");
    }
}

#[test]
fn the_context_carries_me_and_the_closed_names() {
    let mut names = ClosedNames::default();
    names.add("Закрыта");
    let c = ExportScope { closed_names: names, ..scope() }.context();
    assert_eq!(c.me, Some(me()), "в контексте выгрузки — я");
    assert!(c.closed.contains("закрыта") && !c.closed.contains("Открыта"), "закрытые названия из настроек");
}

#[test]
fn the_page_query_asks_for_closed_changed_since_the_marks_minute() {
    // закрытые статусы, изменённые с минуты отметки (сервер сравнивает до минуты), по возрастанию изменения; и архивные
    let url = scope().query(&mark()).to_url(1, 60, false, true);
    assert_eq!(
        url,
        "api/task?StatusIds=29,30&ChangedMoreThan=2026-10-01%2012%3A05&archive=true&inactive=true&include=status&sort=Changed%20asc,%20Id%20asc&pagesize=60&page=1"
    );
    // все закрытые — тот же запрос, что у моих; все заявки — без статусов
    assert_eq!(with_reach(ExportReach::Closed).query(&mark()).to_url(1, 60, false, true), url, "зеркало закрытых — тот же запрос");
    let all = with_reach(ExportReach::All).query(&mark()).to_url(1, 60, false, true);
    assert_eq!(
        all,
        "api/task?ChangedMoreThan=2026-10-01%2012%3A05&archive=true&inactive=true&include=status&sort=Changed%20asc,%20Id%20asc&pagesize=60&page=1"
    );
}

#[test]
fn the_query_is_built_of_the_marks_own_clock() {
    // решение карточки: changed.from — часы отметки без пояса; без «до»; архивные включены; порядок — по изменению
    let q = scope().query(&mark());
    assert_eq!(q.status_ids, vec![29, 30], "закрытые статусы");
    assert_eq!(q.changed.from, Some(at().naive_local()), "с отметки, не округлённой до минуты");
    assert_eq!(q.changed.to, None, "верхней границы нет");
    assert!(q.include_archived, "архивные и неактивные сервисы — тоже");
    assert_eq!(q.sort, CHANGED_ASC_SORT, "по возрастанию изменения");
    assert!(
        q.executor_ids.is_empty() && q.search.is_empty() && q.created.from.is_none() && q.filter_id.is_none(),
        "остальное — по умолчанию"
    );
    // отметка с чужим смещением: берутся её собственные часы, не пересчёт в пояс ПК
    let east = FixedOffset::east_opt(5 * 3600).unwrap().with_ymd_and_hms(2026, 10, 1, 12, 5, 30).unwrap();
    assert_eq!(scope().query(&mark_at(east, &[])).changed.from, Some(east.naive_local()), "часы отметки, а не ПК");
    assert!(with_reach(ExportReach::All).query(&mark()).status_ids.is_empty(), "все заявки — без статусов");
}

#[test]
fn mine_by_row_goes_by_ids_and_then_by_names() {
    let s = scope();
    // исполнитель и инициатор по номерам; номеров нет — по именам (без учёта регистра и лишних пробелов)
    assert!(s.mine_by_row(&row_by(1, None, None, Some(&[9, 7]))), "исполнитель по номерам");
    assert!(s.mine_by_row(&row_by(2, None, Some(7), Some(&[9]))), "инициатор по номерам");
    let by_ids = Found { creator: Some("Я Сам".into()), executors: Some("Я Сам".into()), ..row_by(3, None, Some(5), Some(&[9])) };
    assert!(!s.mine_by_row(&by_ids), "номера есть — имена не решают");
    assert!(s.mine_by_row(&named(4, None, Some("Иванов И., я  сам"))), "номеров нет — по именам, через запятую");
    assert!(s.mine_by_row(&named(5, Some("Я Сам"), None)), "номеров нет — инициатор по имени");
    assert!(!s.mine_by_row(&named(6, Some("Петрова А."), Some("Иванов И."))), "чужая по именам");
    let nameless = ExportScope { me: User { id: 7, name: String::new() }, ..scope() };
    assert!(!nameless.mine_by_row(&named(7, Some(""), Some(""))), "моего имени сервер не прислал — пустое с пустым не совпадает");
}

#[test]
fn the_role_in_a_row_is_one_of_three() {
    // Myself: исполнитель, инициатор, наблюдатель — каждый по номерам, нет номеров — по именам
    let me = me();
    let i = Myself { me: &me };
    let executor = row_by(1, None, None, Some(&[7]));
    assert!(i.executes(&executor) && !i.initiated(&executor) && !i.observes(&executor), "исполнитель по номеру");
    let creator = row_by(2, None, Some(7), None);
    assert!(i.initiated(&creator) && !i.executes(&creator), "инициатор по номеру");
    let observer = Found { observer_ids: Some(vec![3, 7]), ..row(3, None) };
    assert!(i.observes(&observer) && !i.executes(&observer), "наблюдатель по номеру");
    let by_name = Found { observers: Some("Петров П., Я Сам".into()), ..row(4, None) };
    assert!(i.observes(&by_name), "наблюдатель по имени, нет номеров");
    let other_ids = Found { observer_ids: Some(vec![3]), observers: Some("Я Сам".into()), ..row(5, None) };
    assert!(!i.observes(&other_ids), "номера наблюдателей есть — имя не решает");
    let empty_ids = Found { executor_ids: Some(vec![]), executors: Some("Я Сам".into()), ..row(6, None) };
    assert!(!i.executes(&empty_ids), "пустой список номеров — это «никто», а не «номеров нет»");
    assert!(!i.executes(&row(7, None)) && !i.initiated(&row(7, None)) && !i.observes(&row(7, None)), "ни номеров, ни имён — не я");
    // мой наблюдатель — «мои по строке»
    assert!(scope().mine_by_row(&observer), "наблюдатель — тоже мои");
}

#[test]
fn mine_by_history_is_any_action_of_mine() {
    let s = scope();
    // любое моё действие — комментарий или смена статуса; по номеру, нет номера — по имени
    assert!(
        s.mine_by_history(&[event("Петрова А.", Some(5), Some("текст")), event("Я Сам", Some(7), None)]),
        "смена статуса — тоже участие"
    );
    assert!(s.mine_by_history(&[event("я сам", None, Some("текст"))]), "без номера — по имени");
    assert!(
        !s.mine_by_history(&[event("Петрова А.", Some(5), Some("Я Сам, посмотрите"))]),
        "моё имя в тексте чужого комментария — не участие"
    );
    assert!(!s.mine_by_history(&[event("Я Сам", Some(8), Some("текст"))]), "номер автора чужой — имя не решает");
    assert!(!s.mine_by_history(&[]), "пустая история");
    let nameless = ExportScope { me: User { id: 7, name: String::new() }, ..scope() };
    assert!(
        !nameless.mine_by_history(&[event("", None, Some("текст"))]),
        "моего имени сервер не прислал — событие без автора и номера не моё"
    );
    let me = me();
    assert!(
        Myself { me: &me }.wrote(&event("Я Сам", Some(7), None)) && !Myself { me: &me }.wrote(&event("Петрова А.", Some(5), None)),
        "wrote"
    );
}

#[test]
fn the_mirror_takes_any_row_and_mine_only_its_own() {
    // берётся по строке: зеркало — любая, без чтения истории; мои — как было
    let stranger = row_by(50, Some(at()), Some(5), Some(&[9]));
    assert!(with_reach(ExportReach::Closed).takes_by_row(&stranger), "зеркало закрытых берёт любую строку");
    assert!(with_reach(ExportReach::All).takes_by_row(&stranger), "зеркало всех берёт любую строку");
    assert!(!scope().takes_by_row(&stranger), "мои — только свои");
    assert!(scope().takes_by_row(&row_by(51, Some(at()), None, Some(&[7]))), "мои берут свою");
}

#[test]
fn a_file_is_right_with_the_attachments_the_row_would_get() {
    // взятой по строке — как при выгрузке (Context); у моих прочих (мои ли, ещё не ясно) — только если файл говорит, что
    // заявка моя (Mine: по роли в файле); «не скачивать» — ничего
    let stranger = row_by(50, Some(at()), Some(5), Some(&[9]));
    let mine_pass = ExportScope { attachments: AttachmentScope::All, ..scope() };
    let executor_row = row_by(52, Some(at()), None, Some(&[7]));
    assert_eq!(mine_pass.current_for(&executor_row), AttachmentScope::All, "моя по строке — как при выгрузке");
    assert_eq!(mine_pass.current_for(&stranger), AttachmentScope::Mine, "чужая по строке — если файл говорит, что она моя");
    assert_eq!(
        ExportScope { attachments: AttachmentScope::None, ..mine_pass.clone() }.current_for(&stranger),
        AttachmentScope::None,
        "не скачивать — ничего"
    );
    assert_eq!(
        ExportScope { reach: ExportReach::Closed, ..mine_pass.clone() }.current_for(&stranger),
        AttachmentScope::Mine,
        "зеркало закрытых"
    );
    assert_eq!(
        ExportScope { reach: ExportReach::All, attachments: AttachmentScope::None, ..mine_pass }.current_for(&executor_row),
        AttachmentScope::None,
        "зеркало, не скачивать — ничего"
    );
}

#[test]
fn a_number_without_a_name_acts_as_a_mirror() {
    // решение карточки: Other(n) ведёт себя как значение n у enum C#
    let odd = with_reach(ExportReach::Other(3));
    let stranger = row_by(50, Some(at()), Some(5), Some(&[9]));
    assert!(odd.takes_by_row(&stranger), "Other берёт любую строку, как зеркало (≠ Mine)");
    assert_eq!(odd.query(&mark()).status_ids, vec![29, 30], "Other не «все заявки»: закрытые статусы остаются");
    let of = |setting| ExportScope { attachments: setting, ..odd.clone() }.context().attachments;
    assert_eq!(of(AttachmentScope::All), AttachmentScope::Mine, "вложения — как у зеркала");
    assert_eq!(of(AttachmentScope::None), AttachmentScope::None, "не скачивать — ничего");
    assert!(!mark_serves(Some(ExportReach::Other(3)), ExportReach::Closed), "отметка Other (ветка `_`) служит лишь Mine");
    assert!(mark_serves(Some(ExportReach::Other(3)), ExportReach::Mine), "отметка Other служит охвату Mine");
    assert!(!mark_serves(Some(ExportReach::Other(3)), ExportReach::Other(3)), "Other ≠ Mine — не служит и себе");
    assert!(!mark_serves(Some(ExportReach::Mine), ExportReach::Other(3)), "мои не служат Other");
    let t0 = at();
    assert!(!due(ExportReach::Other(3), Some((ExportReach::Other(3), t0)), t0 + mins(1)), "Other отдыхает, как зеркало");
    assert!(due(ExportReach::Other(3), Some((ExportReach::Other(3), t0)), t0 + mins(59)), "Other: через 59 минут пора");
}

// ---------------------------------------------------------------------------------------------------------------------
// ClosedExport.SelfCheck: отметка
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_marked_row_is_skipped_until_it_changes() {
    // разобранные строки её минуты пропускаются, пока не изменились
    let m = mark();
    assert!(m.covers(&row(5, Some(at()))) && m.covers(&row(5, Some(at() - secs(20)))), "разобранная в минуту отметки");
    assert!(!m.covers(&row(5, Some(at() + secs(1)))), "изменилась ещё раз — разбирается заново");
    assert!(!m.covers(&row(6, Some(at()))) && !m.covers(&row(5, None)), "не разобранная и без даты — не пропускаются");
    // строка раньше минуты отметки — уже разобрана (правильный сервер её и не вернул бы; вернул — сдвиг поясов): пропуск
    assert!(m.covers(&row(99, Some(at() - mins(1)))), "раньше минуты отметки — пропускается");
}

#[test]
fn the_marks_minute_edge_is_exact() {
    // граница минуты: ровно в 12:05:00 — уже не «раньше минуты», и без номера в Seen строка разбирается
    let minute = local(2026, 10, 1, 12, 5, 0);
    let m = mark();
    assert!(!m.covers(&row(6, Some(minute))), "ровно в минуту отметки, номера нет среди разобранных — не пропускается");
    assert!(m.covers(&row(5, Some(minute))), "ровно в минуту отметки, номер среди разобранных — пропускается");
    assert!(m.covers(&row(6, Some(minute - Span::milliseconds(1)))), "на миллисекунду раньше минуты — пропускается");
    assert!(!mark_at(at(), &[]).covers(&row(5, Some(at()))), "номеров минуты нет — ничего не разобрано");
}

#[test]
fn the_mark_moves_forward_over_the_rows_done() {
    let m = mark();
    // после разобранных: время последней и номера её минуты; минута та же — прежние номера остаются
    let a = row(6, Some(at() + secs(10)));
    let b = row(8, Some(at() + secs(20)));
    let next = m.after(&[&a, &b]);
    assert_eq!(next.since, at() + secs(20), "та же минута: время последней");
    assert_eq!(seen_of(&next), [5, 6, 8], "та же минута: прежние номера и новые");
    let c = row(9, Some(at() + mins(1)));
    let d = row(10, Some(at() + mins(2) + secs(5)));
    let later = next.after(&[&c, &d]);
    assert_eq!(later.since, at() + mins(2) + secs(5), "минута сменилась: время последней");
    assert_eq!(seen_of(&later), [10], "минута сменилась — только её номера");
    assert_eq!(m.after(&[]), m, "никого не разобрали — та же отметка");
    assert_ne!(next, m, "SameAs: другое время и номера");
    // отметка с теми же номерами, но другим временем и наоборот — не та же
    assert_ne!(mark_at(at(), &[5]), mark_at(at() + secs(1), &[5]), "SameAs: время");
    assert_ne!(mark_at(at(), &[5]), mark_at(at(), &[5, 6]), "SameAs: номера");
}

#[test]
fn the_mark_never_goes_back() {
    // поздно показавшаяся строка минуты отметки, раньше самой отметки: отметка назад не идёт, номера минуты — вместе
    let m = mark();
    let late = m.after(&[&row(11, Some(at() - secs(20)))]);
    assert_eq!(late.since, at(), "отметка назад не идёт");
    assert_eq!(seen_of(&late), [5, 11], "номера минуты — вместе");
    assert!(late.covers(&row(5, Some(at()))), "прежняя разобранная по-прежнему пропускается");
}

#[test]
fn the_seen_numbers_are_sorted_distinct_and_of_the_last_minute() {
    // выводится из `After`: Distinct().Order(); строки раньше минуты в номера не идут
    let m = mark_at(at(), &[]);
    let first = row(9, Some(at() + mins(1)));
    let dup = row(4, Some(at() + mins(1) + secs(5)));
    let same = row(9, Some(at() + mins(1) + secs(10)));
    let next = m.after(&[&first, &dup, &same]);
    assert_eq!(seen_of(&next), [4, 9], "по возрастанию, без повторов");
    let early = row(1, Some(at() + secs(5)));
    let moved = m.after(&[&early, &row(2, Some(at() + mins(3)))]);
    assert_eq!(seen_of(&moved), [2], "строка из прошлой минуты в номера не идёт");
}

#[test]
fn the_first_run_looks_a_week_back() {
    assert_eq!(ExportMark::start(at()).since, at() - Span::days(7), "первое включение — неделя назад");
    assert!(ExportMark::start(at()).seen.is_empty(), "никого не разобрали");
}

#[test]
fn the_minute_keeps_the_offset() {
    assert_eq!(ExportMark::minute(at()), local(2026, 10, 1, 12, 5, 0), "секунды долой");
    let d = FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap() + Span::milliseconds(900);
    let m = ExportMark::minute(d);
    assert_eq!(m, FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 12, 31, 23, 59, 0).unwrap(), "доли секунды долой");
    assert_eq!(m.offset(), d.offset(), "смещение остаётся");
}

// ---------------------------------------------------------------------------------------------------------------------
// ClosedExport.SelfCheck: страница списка
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_page_all_before_the_mark_means_the_condition_was_not_applied() {
    let m = mark();
    // «сервер не применил условие по дате» — только когда раньше отметки вся страница; часть — пропускается
    let all_before = [row(1, Some(at() - Span::days(3))), row(2, Some(at() - mins(2)))];
    assert!(before_mark(&all_before, &m, true).contains("ChangedMoreThan"), "полная страница — вся раньше");
    assert_eq!(before_mark(&[row(1, Some(at() - mins(2))), row(2, Some(at() + mins(1)))], &m, true), "", "раньше — только часть");
    // страница не полная, и вся раньше (новых нет, а сервер со сдвигом отдал прежние) — не ошибка
    assert_eq!(before_mark(&[row(1, Some(at() - mins(2)))], &m, false), "", "короткая страница ранних — не ошибка");
    assert_eq!(
        before_mark(&[row(1, Some(at() - Span::hours(26))), row(2, Some(at() - mins(2)))], &m, false),
        "",
        "короткая страница на 26 часов раньше — крайний сдвиг поясов, не ошибка"
    );
    // короткая, но её первая строка раньше отметки дальше любого сдвига поясов — условие не применено
    assert!(before_mark(&all_before, &m, false).contains("ChangedMoreThan"), "короткая страница из давних — не применил условие");
    assert_eq!(before_mark(&[], &m, true), "", "пустая страница — в порядке");
}

#[test]
fn the_list_must_be_in_order_and_dated() {
    // порядок страницы: по возрастанию изменения; равные — в любом порядке номеров; без даты — не разобрать
    assert_eq!(disorder(&[row(3, Some(at())), row(2, Some(at())), row(4, Some(at() + secs(1)))]), "", "равные — в любом порядке номеров");
    assert!(disorder(&[row(3, Some(at() + secs(1))), row(4, Some(at()))]).contains("не по возрастанию"), "раньше после позже");
    assert!(disorder(&[row(3, Some(at())), row(4, None)]).contains("нет даты"), "строка без даты");
    assert_eq!(disorder(&[]), "", "пустая страница — в порядке");
}

#[test]
fn the_page_holds_the_marks_numbers_and_never_more_than_the_server_gives() {
    // страница — 500 строк и номера минуты отметки, у любого охвата; не больше 2000
    let many = mark_at(at(), &(1..=1600).collect::<Vec<_>>());
    assert_eq!(page_size(&mark()), 501, "500 и один номер");
    assert_eq!(page_size(&many), MAX_PAGE, "не больше, чем сервер отдаёт за раз");
    assert_eq!(page_size(&ExportMark::start(at())), PAGE_ROWS, "без номеров — 500");
    assert_eq!((PAGE_ROWS, MAX_PAGE, BUDGET, GIVE_UP_TIMES), (500, 2000, 30, 5), "константы захода");
    assert_eq!(page_size(&mark_at(at(), &(1..=1500).collect::<Vec<_>>())), 2000, "ровно до потолка");
    assert_eq!(page_size(&mark_at(at(), &(1..=1499).collect::<Vec<_>>())), 1999, "на один меньше потолка");
}

// ---------------------------------------------------------------------------------------------------------------------
// ClosedExport.SelfCheck: охват отметки, отдых зеркала, пропуск, ошибки
// ---------------------------------------------------------------------------------------------------------------------

#[test]
fn a_mark_serves_a_reach_not_wider_than_its_own() {
    use ExportReach::{All, Closed, Mine};
    // отметка годится охвату не шире своего; без охвата — «мои» (0.21 и раньше)
    assert!(mark_serves(None, Mine), "без охвата — мои: мои");
    assert!(!mark_serves(None, Closed), "без охвата — мои: не закрытые");
    assert!(!mark_serves(Some(Closed), All), "отметка закрытых не служит всем");
    assert!(mark_serves(Some(All), Closed), "отметка всех служит закрытым");
    assert!(mark_serves(Some(All), Mine), "отметка всех служит моим");
    assert!(mark_serves(Some(Closed), Closed), "закрытые — закрытым");
    assert!(!mark_serves(Some(Mine), All), "отметка моих не служит всем");
    // остальные сочетания той же таблицы
    assert!(mark_serves(Some(All), All), "все — всем");
    assert!(mark_serves(Some(Closed), Mine), "отметка закрытых служит моим");
    assert!(mark_serves(Some(Mine), Mine), "мои — моим");
    assert!(!mark_serves(Some(Mine), Closed), "отметка моих не служит закрытым");
}

#[test]
fn the_mirror_rests_a_bit_less_than_an_hour() {
    use ExportReach::{All, Closed, Mine};
    let t0 = at();
    // у моих — никогда; у зеркала — без минуты час от начала догнавшего захода, и только для того же охвата
    assert!(due(Mine, Some((Mine, t0)), t0), "мои — каждым заходом");
    assert!(due(Closed, None, t0), "зеркало, отдыха не было — пора");
    assert!(!due(Closed, Some((Closed, t0)), t0 + mins(58)), "через 58 минут ещё отдыхает");
    assert!(due(Closed, Some((Closed, t0)), t0 + mins(59)), "через 59 минут — пора");
    assert!(due(All, Some((Closed, t0)), t0 + mins(1)), "охват сменили — отдых не в счёт");
    assert!(!due(Closed, Some((Closed, t0)), t0 + mins(58) + secs(59)), "без секунды 59 минут — ещё отдыхает");
    assert!(due(Mine, Some((Closed, t0)), t0), "мои и при чужом отдыхе — пора");
    assert_eq!(MIRROR_EVERY, Duration::from_secs(59 * 60), "без минуты час");
}

#[test]
fn only_the_tickets_own_errors_count_toward_giving_up() {
    // в счёт пропуска — сбои самой заявки; сеть, сроки, 502–504 и логин — нет
    for e in [
        "ошибка сервера (HTTP 500):\n{}",
        "нет доступа (HTTP 403)",
        "заявка не найдена (HTTP 404)",
        "сервер ответил, но не так (HTTP 200): разбор не удался",
        "файл записан, но пропал с диска — похоже, его удалил антивирус",
    ] {
        assert!(of_ticket(e), "{e}");
    }
    for e in [
        "сервер не ответил за 10 с",
        "сервер недоступен: connection refused",
        "соединение оборвалось",
        "сервер перегружен (HTTP 503)",
        "шлюз (HTTP 502)",
        "срок шлюза (HTTP 504)",
        "неверный логин или пароль (HTTP 401)",
        "слишком много запросов (HTTP 429)",
    ] {
        assert!(!of_ticket(e), "{e}");
    }
}

#[test]
fn a_broken_ticket_is_given_up_after_five_passes_and_an_hour() {
    // пропуск: GiveUpTimes заходов подряд на том же изменении, и первый — не меньше GiveUpAfter назад
    let t0 = at();
    let broken = row(40, Some(at()));
    let mut failures = ExportFailures::default();
    for i in 0..GIVE_UP_TIMES {
        failures = failures.after(&[(&broken, format!("сбой {i}"))], &[], t0 + mins(5 * i as i64));
    }
    let f40 = &failures.items[&40];
    assert_eq!((f40.times, f40.error.as_str(), f40.first), (GIVE_UP_TIMES, "сбой 4", t0), "счёт и последняя ошибка");
    assert_eq!((f40.id, f40.changed), (40, at()), "номер и изменение, на котором спотыкается");
    assert!(!failures.given_up(&broken, t0 + mins(59)), "пять заходов — но не раньше часа от первого: 59 минут");
    assert!(failures.given_up(&broken, t0 + mins(60)), "пять заходов и час — пропускаем");
    assert_eq!(GIVE_UP_AFTER, Duration::from_secs(3600), "час");
    assert!(!failures.given_up(&row(40, Some(at() + mins(1))), t0 + Span::hours(2)), "заявка изменилась — пробуем снова");
    assert!(!failures.given_up(&row(41, Some(at())), t0 + Span::hours(2)), "другая заявка — не в счёте");
    let mut four = ExportFailures::default();
    for i in 0..GIVE_UP_TIMES - 1 {
        four = four.after(&[(&broken, "сбой".into())], &[], t0 + mins(i as i64));
    }
    assert!(!four.given_up(&broken, t0 + Span::days(1)), "на заход меньше — хоть сутки спустя, пробуем");
}

#[test]
fn the_failure_count_restarts_resets_and_waits() {
    let t0 = at();
    let broken = row(40, Some(at()));
    let mut failures = ExportFailures::default();
    for i in 0..GIVE_UP_TIMES {
        failures = failures.after(&[(&broken, format!("сбой {i}"))], &[], t0 + mins(5 * i as i64));
    }
    let again = failures.after(&[(&row(40, Some(at() + mins(1))), "новый сбой".into())], &[], t0 + Span::hours(2));
    let f = &again.items[&40];
    assert_eq!(
        (f.times, f.error.as_str(), f.first, f.changed),
        (1, "новый сбой", t0 + Span::hours(2), at() + mins(1)),
        "изменилась — счёт заново"
    );
    assert!(!failures.after(&[], &[40], t0).items.contains_key(&40), "прошла — из счёта вон");
    assert_eq!(failures.after(&[], &[41], t0).items[&40].times, GIVE_UP_TIMES, "не дошли до неё — счёт как был");
    assert!(
        ExportFailures::default().items.is_empty() && !ExportFailures::default().given_up(&broken, t0 + Span::days(1)),
        "пусто — ничего не пропускаем"
    );
    // первый сбой другой заявки не трогает счёт этой; две в одном заходе — по отдельности
    let other = row(41, Some(at()));
    let both = failures.after(&[(&other, "ошибка".into())], &[], t0 + mins(30));
    assert_eq!(
        (both.items[&40].times, both.items[&41].times, both.items[&41].first),
        (GIVE_UP_TIMES, 1, t0 + mins(30)),
        "счёт — по заявкам"
    );
}

#[test]
fn the_time_of_a_pass_is_nothing_short_of_a_second() {
    // короче секунды — ничего; минуты и часы — как в итоге выгрузки (HumanSpan)
    assert_eq!(took(Duration::from_millis(999)), "");
    assert_eq!(took(Duration::from_secs_f64(41.7)), " за 41 с");
    assert_eq!(took(Duration::from_secs(125)), " за 2 мин");
    assert_eq!(took(Duration::from_secs(185 * 60)), " за 3 ч 05 мин");
}
