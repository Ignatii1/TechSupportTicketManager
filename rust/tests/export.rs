//! Текст заявки для агента без сети (`KnowledgeExport.Format.cs`, `Secrets`, `TicketHistory`, `WithDetails`, `RowOf`):
//! таблицы сверки с кодом C# и то, что таблица не вмещает (карточка 4.1-1).

mod common;

use chrono::DateTime;
use ticketboard::export::*;
use ticketboard::intraservice::{Date, Event, Extra, Found, Task, User, parse};
use ticketboard::rules::ClosedNames;

fn date(text: &str) -> Date {
    DateTime::parse_from_rfc3339(text).unwrap()
}

// ---------- таблицы сверки ----------

#[test]
fn passwords_are_hidden_as_in_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-mask", |inputs| mask(&inputs[0]));
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn markdown_escaping_agrees_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-escape", |inputs| escape(&inputs[0]));
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

/// Ответ строки таблицы `export-text`: список (или «-»), карточка, страница истории, я, закрытые названия, урезана ли.
fn text_answer(inputs: &[String]) -> String {
    const UNREADABLE: &str = "НЕ РАЗОБРАН";
    let Some(task) = parse::task(&inputs[1], 0) else { return UNREADABLE.into() };
    let Some(page) = parse::lifetime(&inputs[2]) else { return UNREADABLE.into() };
    let row = if inputs[0] == "-" {
        row_of(&task)
    } else {
        match parse::search(&inputs[0]).and_then(|p| p.found.into_iter().next()) {
            Some(found) => found,
            None => return UNREADABLE.into(),
        }
    };
    let me = (inputs[3] != "-").then(|| {
        let (id, name) = inputs[3].split_once('|').expect("я — «номер|имя»");
        User { id: id.parse().expect("номер"), name: name.to_string() }
    });
    let names: Vec<String> = inputs[4].split('|').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    let context = Context { me, closed: ClosedNames::new(&names) };
    let url = format!("https://is.example/Task/View/{}", task.id);
    format(&with_details(&row, &task), &page.events, &url, &date("2026-11-17T10:00:00+03:00"), inputs[5] == "да", &context)
}

#[test]
fn the_agent_text_agrees_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-text", text_answer);
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

// ---------- постоянные и мелкие правила ----------

#[test]
fn the_format_version_and_the_hidden_mark_are_those_of_csharp() {
    assert_eq!(FORMAT_VERSION, 8);
    assert_eq!(HIDDEN, "[скрыт]");
    assert_eq!(MAX_LIFETIME_PAGES, 100);
}

#[test]
fn empty_text_stays_empty() {
    assert_eq!((mask(""), escape(""), one_line("")), (String::new(), String::new(), String::new()));
}

#[test]
fn a_masked_text_is_masked_the_same_again() {
    let once = mask("Пароль: 1\nPIN=2");
    assert_eq!(once, "Пароль: [скрыт]\nPIN=[скрыт]");
    assert_eq!(mask(&once), once);
}

#[test]
fn one_line_joins_every_run_of_whitespace_into_one_space() {
    assert_eq!(one_line("  a \n\t b\r\nc  "), "a b c");
    assert_eq!(one_line("a\u{A0}\u{2003}b"), "a b");
    assert_eq!(one_line(" \n\t "), "");
}

#[test]
fn a_machine_date_has_whole_seconds_and_the_offset_of_the_date() {
    let live = date("2026-10-16T09:45:39.362972+03:00");
    assert_eq!(machine_date(&live), "2026-10-16T09:45:39+03:00");
    // доли отбрасываются, не округляются: 39,999 — всё ещё 39-я секунда
    assert_eq!(machine_date(&date("2026-10-16T09:45:39.999+03:00")), "2026-10-16T09:45:39+03:00");
    assert_eq!(machine_date(&date("2026-10-16T09:45:39+05:00")), "2026-10-16T09:45:39+05:00");
    assert_eq!(machine_date(&date("2026-10-16T06:45:39Z")), "2026-10-16T06:45:39+00:00");
    assert_eq!(machine_date(&date("2026-10-16T06:45:39-02:30")), "2026-10-16T06:45:39-02:30");
}

#[test]
fn the_revision_is_the_exact_moment_in_ticks() {
    // образец `MachineDate.SelfCheck`: тик — 100 нс от 0001-01-01 по UTC
    assert_eq!(revision(&date("2026-10-16T09:45:39+03:00")), "639277299390000000");
    // микросекунды сервера — в версии: две правки в одну секунду различаются
    assert_eq!(revision(&date("2026-10-16T09:45:39.362972+03:00")), "639277299393629720");
    // тот же миг в другом поясе — та же версия
    assert_eq!(revision(&date("2026-10-16T06:45:39.362972Z")), "639277299393629720");
}

// ---------- WithDetails и RowOf ----------

fn card() -> Task {
    Task {
        id: 80010,
        name: "Из карточки".into(),
        status: "Закрыта".into(),
        description: Some("описание карточки".into()),
        creator: Some("Создатель карточки".into()),
        executors: Some("Исполнитель карточки".into()),
        executor_group: Some("Группа карточки".into()),
        changed: Some(date("2026-10-15T10:00:00+03:00")),
        creator_phone: Some("+7 900".into()),
        creator_email: Some("a@b.example".into()),
        created: Some(date("2026-10-13T09:00:00+03:00")),
        observer_ids: Some(vec![901]),
        observers: Some("Наблюдатель карточки".into()),
        extra: Some(Extra {
            service: Some("Сервис карточки".into()),
            task_type: Some("Тип карточки".into()),
            categories: Some("Кат карточки".into()),
            resolved: Some(date("2026-10-15T09:30:00+03:00")),
            company: Some("Фирма карточки".into()),
            assets: Some("Актив карточки".into()),
            files: None,
            closed: Some(date("2026-10-15T09:40:00+03:00")),
        }),
        ..Task::default()
    }
}

fn row() -> Found {
    Found {
        id: 80010,
        name: "Из списка".into(),
        status: "В работе".into(),
        creator: Some("Создатель списка".into()),
        created: Some(date("2026-10-14T09:00:00+03:00")),
        description: Some("описание списка".into()),
        executors: Some("Исполнитель списка".into()),
        executor_group: Some("Группа списка".into()),
        changed: Some(date("2026-10-14T10:00:00+03:00")),
        creator_id: Some(100),
        executor_ids: Some(vec![200]),
        observer_ids: Some(vec![900]),
        observers: Some("Наблюдатель списка".into()),
        ..Found::default()
    }
}

#[test]
fn a_card_makes_a_row_with_the_same_fields_and_no_numbers_of_people() {
    let task = card();
    let f = row_of(&task);
    assert_eq!(f.id, 80010);
    assert_eq!((f.name.as_str(), f.status.as_str()), ("Из карточки", "Закрыта"));
    assert_eq!(f.creator.as_deref(), Some("Создатель карточки"));
    assert_eq!((f.created, f.changed), (task.created, task.changed));
    assert_eq!(f.description.as_deref(), Some("описание карточки"));
    assert_eq!((f.executors.as_deref(), f.executor_group.as_deref()), (Some("Исполнитель карточки"), Some("Группа карточки")));
    assert_eq!((f.creator_phone.as_deref(), f.creator_email.as_deref()), (Some("+7 900"), Some("a@b.example")));
    assert_eq!(f.extra, task.extra);
    assert_eq!((f.observer_ids, f.observers.as_deref()), (Some(vec![901]), Some("Наблюдатель карточки")));
    // карточка не присылает номеров инициатора и исполнителей: роль и реакция пойдут по именам
    assert_eq!((f.creator_id, f.executor_ids), (None, None));
}

#[test]
fn with_details_takes_the_card_group_observers_and_extra_and_the_rest_from_the_row() {
    let f = with_details(&row(), &card());
    // из строки: название, статус, даты, люди, описание, номера — по её Changed выгрузка узнаёт неизменные заявки
    assert_eq!((f.id, f.name.as_str(), f.status.as_str()), (80010, "Из списка", "В работе"));
    assert_eq!(f.creator.as_deref(), Some("Создатель списка"));
    assert_eq!((f.created, f.changed), (row().created, row().changed));
    assert_eq!((f.description.as_deref(), f.executors.as_deref()), (Some("описание списка"), Some("Исполнитель списка")));
    assert_eq!((f.creator_id, f.executor_ids), (Some(100), Some(vec![200])));
    // из карточки: группа, наблюдатели, сервис и остальное дополнение
    assert_eq!(f.executor_group.as_deref(), Some("Группа карточки"));
    assert_eq!((f.observer_ids, f.observers.as_deref()), (Some(vec![901]), Some("Наблюдатель карточки")));
    let x = f.extra.expect("дополнение есть всегда");
    assert_eq!(
        (x.service.as_deref(), x.task_type.as_deref(), x.categories.as_deref()),
        (Some("Сервис карточки"), Some("Тип карточки"), Some("Кат карточки"))
    );
    assert_eq!((x.company.as_deref(), x.assets.as_deref()), (Some("Фирма карточки"), Some("Актив карточки")));
    assert_eq!((x.resolved, x.closed), (Some(date("2026-10-15T09:30:00+03:00")), Some(date("2026-10-15T09:40:00+03:00"))));
}

#[test]
fn with_details_takes_from_the_card_only_what_it_sent_and_keeps_the_rest_of_the_row() {
    // карточка прислала один тип; группы и наблюдателей не прислала вовсе (не «пусто», а «не знаю»)
    let task = Task {
        id: 80010,
        name: "x".into(),
        extra: Some(Extra { task_type: Some("Тип карточки".into()), ..Extra::default() }),
        ..Task::default()
    };
    let mut f = row();
    f.extra = Some(Extra {
        service: Some("Сервис списка".into()),
        task_type: Some("Тип списка".into()),
        resolved: Some(date("2026-10-14T09:30:00+03:00")),
        files: Some(vec![ticketboard::intraservice::TaskFile { id: 1, name: Some("список.txt".into()) }]),
        ..Extra::default()
    });
    let got = with_details(&f, &task);
    assert_eq!(
        (got.executor_group, got.observer_ids, got.observers),
        (f.executor_group.clone(), f.observer_ids.clone(), f.observers.clone())
    );
    let mut want = f.extra.clone().unwrap();
    want.task_type = Some("Тип карточки".into());
    assert_eq!(got.extra, Some(want));
}

#[test]
fn with_details_gives_an_extra_even_when_neither_side_has_one() {
    let got = with_details(&Found { id: 1, name: "x".into(), ..Found::default() }, &Task { id: 1, name: "x".into(), ..Task::default() });
    assert_eq!(got.extra, Some(Extra::default()));
}

#[test]
fn with_details_lets_an_empty_card_group_clear_the_row_group() {
    // карточка прислала поле пустым — это «группы нет», а не «не знаю»
    let task = Task { id: 80010, name: "x".into(), executor_group: Some(String::new()), observers: Some(String::new()), ..Task::default() };
    let got = with_details(&row(), &task);
    assert_eq!((got.executor_group.as_deref(), got.observers.as_deref()), (Some(""), Some("")));
}

// ---------- факты истории (образец `TicketHistory.SelfCheck`, обезличенный) ----------

fn at(day: u32, hour: u32, minute: u32, second: u32) -> Date {
    date(&format!("2026-10-{day:02}T{hour:02}:{minute:02}:{second:02}+03:00"))
}

fn ev(when: Date, author: &str, id: i32, status: &str, comment: Option<&str>, is_public: Option<bool>, is_final: bool) -> Event {
    Event {
        date: Some(when),
        author: author.into(),
        status: status.into(),
        comment: comment.map(String::from),
        is_public,
        author_id: Some(id),
        is_final,
    }
}

fn ticket() -> Found {
    Found {
        id: 715007,
        name: "Настроить ТСД".into(),
        status: "Закрыта по названию".into(),
        creator: Some("Петров Пётр".into()),
        created: Some(at(14, 11, 2, 0)),
        executors: Some("Иванов Иван, Сидоров Сидор".into()),
        extra: Some(Extra {
            service: Some("Склад".into()),
            task_type: Some("Запрос".into()),
            resolved: Some(at(15, 9, 2, 0)),
            closed: Some(at(15, 9, 14, 0)),
            ..Extra::default()
        }),
        creator_id: Some(100),
        executor_ids: Some(vec![200, 300]),
        ..Found::default()
    }
}

/// Как присылает сервер — свежие сверху.
fn history() -> Vec<Event> {
    vec![
        ev(at(15, 9, 14, 0), "Сидоров Сидор", 300, "Закрыта по названию", None, None, false),
        ev(at(15, 9, 5, 0), "Петров Пётр", 100, "", Some("Спасибо, работает"), Some(true), false), // инициатор — не закрывающий
        ev(at(15, 9, 0, 0), "Сидоров Сидор", 300, "", Some("Проверить завтра"), Some(false), false), // внутренний — тоже
        ev(at(15, 8, 50, 0), "Сидоров Сидор", 300, "", Some("Перенастроил принтер"), Some(true), false),
        ev(at(14, 17, 0, 0), "Петров Пётр", 100, "Открыта", Some("Не работает"), Some(true), false),
        ev(at(14, 16, 0, 0), "Иванов Иван", 200, "Выполнена", Some("Настроил ТСД, логин: ТСД_3"), Some(true), true),
        ev(at(14, 15, 22, 0), "Сидоров Сидор", 300, "", Some("Внутренняя заметка"), Some(false), false),
        ev(at(14, 15, 13, 0), "Петров Пётр", 100, "", Some("Спасибо"), Some(true), false),
        ev(at(14, 12, 6, 7), "Иванов Иван", 200, "Ожидание ответа", Some("Давай попробуем как в прошлый раз"), Some(true), false),
        ev(at(14, 11, 40, 0), "Диспетчерова Дарья", 600, "Передана исполнителю", None, None, false), // статус тот же — не действие
        ev(at(14, 11, 32, 0), "Диспетчерова Дарья", 600, "Передана исполнителю", None, None, false),
        ev(at(14, 11, 2, 0), "Диспетчер Денис", 500, "Открыта", None, None, false),
    ]
}

fn as_user(id: i32, name: &str) -> Context {
    Context { me: Some(User { id, name: name.into() }), closed: ClosedNames::new(&["Закрыта по названию".to_string()]) }
}

fn me_as_ivanov() -> Context {
    as_user(200, "Иванов Иван")
}

#[test]
fn participants_are_ordered_by_the_number_of_actions_then_by_who_came_first() {
    let f = facts(&ticket(), &history(), &me_as_ivanov());
    assert_eq!(f.participants, ["Сидоров Сидор", "Петров Пётр", "Иванов Иван", "Диспетчер Денис", "Диспетчерова Дарья"]);
}

#[test]
fn my_role_is_the_first_that_fits_and_empty_when_unknown() {
    let (t, h) = (ticket(), history());
    let role = |f: &Found, ctx: &Context| facts(f, &h, ctx).my_role;
    assert_eq!(role(&t, &me_as_ivanov()), "исполнитель");
    assert_eq!(role(&t, &as_user(100, "Петров Пётр")), "инициатор");
    assert_eq!(role(&t, &as_user(500, "Диспетчер Денис")), "участник");
    assert_eq!(role(&t, &as_user(999, "Чужой")), "");
    assert_eq!(role(&t, &Context { me: None, closed: ClosedNames::new(&["Закрыта по названию".to_string()]) }), "");
    // и исполнитель, и инициатор — исполнитель
    assert_eq!(role(&Found { creator_id: Some(200), ..t.clone() }, &me_as_ivanov()), "исполнитель");
}

#[test]
fn my_role_without_numbers_goes_by_names_ignoring_case_and_spaces() {
    let named = Found { creator_id: None, executor_ids: None, ..ticket() };
    let h = history();
    assert_eq!(facts(&named, &h, &as_user(0, "Сидоров Сидор")).my_role, "исполнитель");
    assert_eq!(facts(&named, &h, &as_user(0, "петров  пётр")).my_role, "инициатор");
}

#[test]
fn an_observer_is_found_by_number_then_by_name_and_loses_to_the_other_roles() {
    let watched = Found {
        observer_ids: Some(vec![900, 500, 100]),
        observers: Some("Смотрящий Семён, Диспетчер Денис, Петров Пётр".into()),
        ..ticket()
    };
    let h = history();
    assert_eq!(facts(&watched, &h, &as_user(900, "Смотрящий Семён")).my_role, "наблюдатель");
    let by_name = Found { observer_ids: None, ..watched.clone() };
    assert_eq!(facts(&by_name, &h, &as_user(0, "смотрящий  семён")).my_role, "наблюдатель");
    // кто писал — участник; инициатор и исполнитель — важнее
    assert_eq!(facts(&watched, &h, &as_user(500, "Диспетчер Денис")).my_role, "участник");
    assert_eq!(facts(&watched, &h, &as_user(100, "Петров Пётр")).my_role, "инициатор");
    // не в списке наблюдателей
    assert_eq!(facts(&ticket(), &h, &as_user(900, "Смотрящий Семён")).my_role, "");
}

#[test]
fn the_role_does_not_depend_on_the_order_of_events() {
    let mut oldest_first = history();
    oldest_first.reverse();
    assert_eq!(role_of(&ticket(), &history(), &as_user(500, "Диспетчер Денис")), "участник");
    assert_eq!(role_of(&ticket(), &oldest_first, &as_user(500, "Диспетчер Денис")), "участник");
    assert_eq!(role_of(&ticket(), &history(), &me_as_ivanov()), "исполнитель");
    assert_eq!(role_of(&ticket(), &[], &as_user(500, "Диспетчер Денис")), "");
    assert_eq!(role_of(&ticket(), &history(), &Context::default()), "");
}

#[test]
fn the_first_reaction_is_not_the_registration_and_not_the_initiator() {
    // передача через полчаса после создания
    assert_eq!(facts(&ticket(), &history(), &me_as_ivanov()).first_reaction_hours, Some(0.5));
    // передачу делает сам инициатор — тогда реакция Иванова: 12:06:07 − 11:02 = 1,1 ч — по номеру и по имени
    let by_creator: Vec<Event> = history()
        .into_iter()
        .map(|e| {
            if e.author == "Диспетчерова Дарья" {
                Event { author: "Петров Пётр".into(), author_id: Some(100), ..e }
            } else {
                e
            }
        })
        .collect();
    assert_eq!(facts(&ticket(), &by_creator, &me_as_ivanov()).first_reaction_hours, Some(1.1));
    let named = Found { creator_id: None, executor_ids: None, ..ticket() };
    assert_eq!(facts(&named, &by_creator, &me_as_ivanov()).first_reaction_hours, Some(1.1));
}

#[test]
fn resolve_hours_run_from_the_resolution_date_else_from_the_closing_date() {
    assert_eq!(facts(&ticket(), &history(), &me_as_ivanov()).resolve_hours, Some(22.0));
    let mut no_resolved = ticket();
    no_resolved.extra.as_mut().unwrap().resolved = None;
    assert_eq!(facts(&no_resolved, &history(), &me_as_ivanov()).resolve_hours, Some(22.2));
}

#[test]
fn reopened_and_comments_are_counted_and_the_closing_comment_is_the_last_public_one_of_another_person() {
    let f = facts(&ticket(), &history(), &me_as_ivanov());
    // «Выполнена» (признак сервера) → «Открыта»: переоткрыта раз; закрыта статусом, закрытым по названию из настроек
    assert_eq!(f.reopened, 1);
    assert_eq!((f.public_comments, f.internal_comments), (6, 2));
    assert_eq!(f.closing_comment, "Перенастроил принтер");
}

#[test]
fn without_the_closed_name_from_settings_the_last_status_is_open_so_no_closing_comment() {
    let f =
        facts(&ticket(), &history(), &Context { me: Some(User { id: 200, name: "Иванов Иван".into() }), closed: ClosedNames::default() });
    assert_eq!((f.closing_comment.as_str(), f.reopened), ("", 1));
}

#[test]
fn a_long_closing_comment_is_one_line_cut_to_300_characters_with_an_ellipsis() {
    let long: Vec<String> = (0..60).map(|i| format!("шаг {i:02} сделан")).collect();
    let h: Vec<Event> = history()
        .into_iter()
        .map(|e| {
            if e.comment.as_deref() == Some("Перенастроил принтер") {
                Event { comment: Some(long.join("\r\n")), ..e }
            } else {
                e
            }
        })
        .collect();
    let closing = facts(&ticket(), &h, &me_as_ivanov()).closing_comment;
    assert_eq!(closing.encode_utf16().count(), 301);
    assert!(closing.ends_with('…') && !closing.contains('\n') && closing.starts_with("шаг 00 сделан шаг 01"), "{closing}");
}

#[test]
fn a_cut_never_splits_a_surrogate_pair() {
    let closing_with = |text: String| {
        let h = vec![ev(at(15, 9, 14, 0), "Сидоров Сидор", 300, "Закрыта по названию", Some(&text), Some(true), false)];
        facts(&ticket(), &h, &me_as_ivanov()).closing_comment
    };
    // знак за границей (его первая половинка — 300-я) уходит целиком
    assert_eq!(closing_with(format!("{}😀хвост", "я".repeat(299))), format!("{}…", "я".repeat(299)));
    // ровно 300 — не режется
    assert_eq!(closing_with("я".repeat(300)), "я".repeat(300));
    assert_eq!(closing_with("😀".repeat(150)), "😀".repeat(150));
}

#[test]
fn the_closing_comment_hides_a_password_made_by_joining_lines() {
    let h = vec![ev(at(15, 9, 14, 0), "Сидоров Сидор", 300, "Закрыта по названию", Some("Пароль\n: secret"), Some(true), false)];
    assert_eq!(facts(&ticket(), &h, &me_as_ivanov()).closing_comment, "Пароль : [скрыт]");
}

#[test]
fn no_history_is_empty_not_an_error() {
    let f = facts(&ticket(), &[], &me_as_ivanov());
    // роль — по строке, истории для неё не нужно
    assert_eq!(f.my_role, "исполнитель");
    assert_eq!(f.participants, Vec::<String>::new());
    assert_eq!((f.first_reaction_hours, f.reopened, f.closing_comment.as_str()), (None, 0, ""));
}

#[test]
fn facts_use_a_closed_name_without_regard_to_case() {
    let ctx = Context { me: None, closed: ClosedNames::new(&["закрыта ПО названию".to_string()]) };
    assert_eq!(facts(&ticket(), &history(), &ctx).closing_comment, "Перенастроил принтер");
}

/// Нарочное расхождение с C# (рецензия 4.1): знак, о котором правило не уверено, — не знак слова, и пароль за ним скрыт.
/// У .NET U+FE0F после «⚠» — знак слова (Mn), граница пропадает, и C# пароль оставляет; здесь лишнее «[скрыт]» лучше.
#[test]
fn a_doubtful_sign_next_to_the_key_hides_the_password_rather_than_shows_it() {
    assert_eq!(mask("\u{26A0}\u{FE0F}Пароль: 1"), "\u{26A0}\u{FE0F}Пароль: [скрыт]");
    assert_eq!(mask("\u{661}Пароль: 1"), "\u{661}Пароль: [скрыт]", "цифра не ASCII перед ключом — граница");
}

/// Буква другой письменности у ключа — тоже не знак слова (у .NET — знак, и C# пароль оставляет): здесь скрыт.
#[test]
fn a_letter_of_another_script_next_to_the_key_hides_the_password() {
    assert_eq!(mask("\u{4E2D}Пароль: 1"), "\u{4E2D}Пароль: [скрыт]");
}
