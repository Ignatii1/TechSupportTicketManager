//! Окно «Поиск в Интрасервисе» без экрана (`common::window`) против поддельного сервера: Enter на доске, кнопка, «Найти»,
//! «Показать ещё», устаревший ответ, просмотр, «+ На доску», доска только для чтения, нет API (список проверок 3.2, п. 7).
//! Одна проверка на всё: платформа Slint ставится один раз на процесс.

mod common;

use common::window::pump_until;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use ticketboard::FoundRow;
use ticketboard::app::{App, SAVE_DELAY};
use ticketboard::board::{Status, Ticket, parse_tickets};
use ticketboard::intraservice::Client;

/// Путь запроса: сервер записывает его целиком («GET /api/task?… HTTP/1.1…»).
fn path(request: &str) -> String {
    request.split(' ').nth(1).unwrap_or("").to_string()
}

fn rows(app: &App) -> Vec<FoundRow> {
    app.finder.get_rows().iter().collect()
}

fn ids(app: &App) -> Vec<i32> {
    rows(app).iter().map(|r| r.id).collect()
}

fn file_tickets(dir: &Path) -> Vec<Ticket> {
    parse_tickets(&std::fs::read_to_string(dir.join("tickets.json")).unwrap()).unwrap()
}

/// Набрать текст в окне — в поле с фокусом (как клавиатура), по знаку.
fn type_text(window: &slint::Window, text: &str) {
    for c in text.chars() {
        let s = SharedString::from(c.to_string());
        window.dispatch_event(WindowEvent::KeyPressed { text: s.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text: s });
    }
}

/// Щелчок левой кнопкой в точке окна.
fn click(window: &slint::Window, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left });
    window.dispatch_event(WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left });
    slint::platform::update_timers_and_animations();
}

/// Строки списка заявок: номера с названием «Заявка N».
fn tasks(ids: impl Iterator<Item = i32>, count: i32) -> common::Reply {
    let rows: Vec<String> = ids.map(|id| format!(r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":28}}"#)).collect();
    common::Reply::json(&format!(
        r#"{{"Tasks":[{}],"Statuses":[{{"Id":28,"Name":"Закрыта"}}],"Paginator":{{"Count":{count},"Page":1,"PageCount":1}}}}"#,
        rows.join(",")
    ))
}

/// На доске уже есть заявка 700002 — её строка в списке приглушена.
const BOARD: &str =
    r#"[{"Id":"8f0b6f3e-0000-4000-8000-000000000001","Title":"Принтер в бухгалтерии","Status":"Inbox","IntraserviceId":700002}]"#;

fn board_dir(name: &str, url: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), BOARD).unwrap();
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, "IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();
    dir
}

#[test]
fn the_search_window() {
    let (windows, jobs) = common::window::install();
    let statuses_fail = Arc::new(AtomicBool::new(false));
    let fail = statuses_fail.clone();
    let statuses_empty = Arc::new(AtomicBool::new(false));
    let empty = statuses_empty.clone();
    let (url, requests) = common::route(move |p| match p {
        "/api/user?getcurrentuserinfo=true" => common::Reply::json(r#"{"Id":7,"Name":"Я Сам"}"#),
        "/api/taskstatus" if fail.load(Ordering::SeqCst) => common::Reply::new(500, "text/plain", "справочник упал"),
        "/api/taskstatus" if empty.load(Ordering::SeqCst) => common::Reply::json("[]"),
        "/api/taskstatus" => common::Reply::json(r#"[{"Id":31,"Name":"Открыта"},{"Id":28,"Name":"Закрыта","IsFinal":true}]"#),
        // справочники окна (3.3): пустые типы и фильтры — без строки «не загрузились» над условиями (щелчки ниже — по месту)
        p if p.starts_with("/api/tasktype?") => common::Reply::json(r#"{"TaskTypeList":{"TaskTypes":[]}}"#),
        p if p.starts_with("/api/filter?") => common::Reply::json("[]"),
        p if p.starts_with("/api/user?") && p.contains("search=%D0%BC%D0%B0%D0%BA%D1%81%D0%B8%D0%BC%D0%BE%D0%B2&") => {
            common::Reply::json(r#"{"Users":[{"Id":38472,"Name":"Максимов М. С."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"#)
        }
        p if p.starts_with("/api/user?") && p.contains("search=%D0%B8%D0%B2%D0%B0%D0%BD%D0%BE%D0%B2&") => common::Reply::json(
            r#"{"Users":[{"Id":5,"Name":"Иванов А."},{"Id":6,"Name":"Иванов Б."}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}"#,
        ),
        p if p.starts_with("/api/user?") && p.contains("search=%D1%81%D0%B8%D0%B4%D0%BE%D1%80%D0%BE%D0%B2&") => {
            common::Reply::json(r#"{"Users":[{"Id":41,"Name":"Сидоров С."}],"Paginator":{"Count":1,"Page":1,"PageCount":1}}"#)
        }
        // все условия формы: строка создана раньше периода — сервер его, похоже, не применил
        p if p.starts_with("/api/task?") && p.contains("search=%D0%B2%D1%81%D0%B5%20%D1%83%D1%81%D0%BB%D0%BE%D0%B2%D0%B8%D1%8F&") => {
            common::Reply::json(
                r#"{"Tasks":[{"Id":720001,"Name":"Старая","StatusId":31,"Created":"2025-06-01T10:00:00","ObserverIds":"41"}],
                "Paginator":{"Count":1,"Page":1,"PageCount":1}}"#,
            )
        }
        p if p.starts_with("/api/task?") && p.contains("search=%D1%81%D0%B1%D0%BE%D0%B9%20%D1%81%D0%B5%D1%80%D0%B2%D0%B5%D1%80%D0%B0&") => {
            common::Reply::new(500, "text/plain", "упал")
        }
        p if p.starts_with("/api/task?") && p.contains("search=%D0%BC%D0%B5%D0%B4%D0%BB%D0%B5%D0%BD%D0%BD%D0%BE&") => {
            // «медленно»: пока сервер думает, человек ищет другое — этот ответ в список попасть не должен (сервер отвечает по
            // очереди: ответ на новый поиск придёт после этого)
            let mut slow = tasks(900..901, 1);
            slow.delay = Duration::from_millis(300);
            slow
        }
        p if p.starts_with("/api/task?") && p.contains("search=%D0%BC%D0%BD%D0%BE%D0%B3%D0%BE&") && p.ends_with("&page=1") => {
            tasks(1..51, 120)
        }
        // вторая страница «много»: список сдвинулся — заявка 50 пришла снова
        p if p.starts_with("/api/task?") && p.contains("search=%D0%BC%D0%BD%D0%BE%D0%B3%D0%BE&") && p.ends_with("&page=2") => {
            tasks(50..100, 120)
        }
        p if p.starts_with("/api/task?") && p.contains("search=%D0%BF%D1%80%D0%B8%D0%BD%D1%82%D0%B5%D1%80&") => common::Reply::json(
            r#"{"Tasks":[{"Id":700001,"Name":"Принтер не печатает","StatusId":28,"Creator":"Петрова А.","ServiceName":"Печать"},
                {"Id":700002,"Name":"Принтер в бухгалтерии","StatusId":28}],
                "Statuses":[{"Id":28,"Name":"Закрыта"}],"Paginator":{"Count":2,"Page":1,"PageCount":1}}"#,
        ),
        p if p.starts_with("/api/task?") && p.contains("ExecutorIds=7&StatusIds=28&") => tasks(700003..700004, 1),
        p if p.starts_with("/api/task?") => tasks(0..0, 0),
        p if p.starts_with("/api/task/700001?") => common::Reply::json(
            r#"{"Task":{"Id":700001,"Name":"Принтер не печатает","StatusName":"Закрыта","Description":"Не печатает с утра","Creator":"Петрова А."}}"#,
        ),
        p if p.starts_with("/api/tasklifetime?") => common::Reply::json(common::LIVE_LIFE),
        _ => common::Reply::new(404, "text/plain", "мимо"),
    });
    let asked = |what: &str| requests.lock().unwrap().iter().map(|r| path(r)).filter(|p| p.contains(what)).count();
    let dir = board_dir("own", &url);
    let app = App::open(&dir, true).unwrap();
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));

    // 1. Enter в поле поиска доски: окно со словами из поля, прочие условия по умолчанию, поиск сразу
    assert!(!app.finder.window().is_visible(), "окно поиска до вызова не показано");
    app.window.invoke_server_search(" принтер ".into());
    assert!(app.finder.window().is_visible(), "Enter на доске показывает окно поиска");
    assert_eq!(app.finder.get_words(), "принтер", "слова — из поля доски, без пробелов по краям");
    assert!(!app.finder.get_mine() && app.finder.get_status_index() == 0, "быстрый поиск — без «мои» и без статуса");
    pump_until(&jobs, || ids(&app) == vec![700001, 700002]);
    assert_eq!(app.finder.get_message(), "Найдено: 2");
    assert!(!app.finder.get_busy() && !app.finder.get_has_more());
    let found = rows(&app);
    assert_eq!((found[0].number.as_str(), found[0].title.as_str()), ("#700001", "Принтер не печатает"));
    assert_eq!(
        (found[0].status.as_str(), found[0].service.as_str(), found[0].details.as_str()),
        ("Закрыта", "Печать", "автор: Петрова А.")
    );
    assert!(!found[0].on_board && found[1].on_board, "заявка с доски приглушена");
    assert!(app.finder.get_writable(), "доска своя — «+ На доску» есть");
    // справочники окно читает при показе, в фоне (3.3): статусы — один раз, поиск «любого» статуса их не ждёт и сам не просит
    pump_until(&jobs, || app.finder.get_status_choices().row_count() > 3);
    assert_eq!(asked("/api/taskstatus"), 1, "справочник статусов прочитан окном один раз");

    // 2. выбор строки: карточка и переписка — в просмотр
    app.finder.invoke_select(0);
    assert_eq!(app.finder.get_selected(), 0);
    assert_eq!(app.finder.get_preview_title(), "#700001 Принтер не печатает", "как у C#: номер и название");
    pump_until(&jobs, || !app.finder.get_preview_busy() && !app.finder.get_preview_text().is_empty());
    let preview = app.finder.get_preview_text().to_string();
    assert!(preview.starts_with("Не печатает с утра"), "описание — первым: {preview}");
    assert!(preview.contains("Добрый день!") && preview.contains("Свежие сверху: 2 комментария"), "и переписка: {preview}");

    // 3. «+ На доску»: карточка наверху «Входящих» с номером, названием и ссылкой, строка приглушена, карточка читается (⟳)
    let cards_before = asked("/api/task/700001?");
    app.finder.invoke_add(0);
    assert!(rows(&app)[0].on_board, "добавленная — уже на доске");
    let key = app.window.get_selected_key().to_string();
    assert!(!key.is_empty() && app.window.get_panel_open(), "новая карточка выбрана, панель открыта");
    pump_until(&jobs, || asked("/api/task/700001?") > cards_before);
    std::thread::sleep(SAVE_DELAY + Duration::from_millis(200));
    slint::platform::update_timers_and_animations();
    pump_until(&jobs, || file_tickets(&dir).iter().any(|t| t.intraservice_id == Some(700001)));
    let added = file_tickets(&dir).into_iter().find(|t| t.intraservice_id == Some(700001)).unwrap();
    assert_eq!(
        (added.title.as_str(), added.status, added.url.as_str()),
        ("Принтер не печатает", Status::Inbox, format!("{}/Task/View/700001", url.trim_end_matches('/')).as_str())
    );
    app.finder.invoke_add(1); // уже на доске — второй раз не добавляется
    app.save_now();
    assert_eq!(file_tickets(&dir).iter().filter(|t| t.intraservice_id == Some(700002)).count(), 1);

    // 4. спрятали (Esc) и открыли кнопкой: условия — прежние «мои закрытые», список по быстрому поиску забыт (видимое окно
    // кнопка только выводит вперёд — `tests/finder_review.rs`, п. 3)
    app.finder.invoke_cancel();
    app.window.invoke_open_search();
    assert!(app.finder.get_mine() && app.finder.get_status_index() == 1 && app.finder.get_words().is_empty(), "прежние условия вернулись");
    assert!(rows(&app).is_empty() && app.finder.get_message().is_empty(), "список быстрого поиска забыт");
    // «Найти»: «я» и закрытые статусы — из справочника, который читается один раз
    app.finder.invoke_search();
    pump_until(&jobs, || ids(&app) == vec![700003]);
    assert_eq!(asked("/api/taskstatus"), 1);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![700003]);
    assert_eq!(asked("/api/taskstatus"), 1, "справочник второй раз не читается");
    // следующее открытие кнопкой — условия человека остаются (вернуть нечего)
    app.window.invoke_open_search();
    assert!(app.finder.get_mine() && ids(&app) == vec![700003], "свои условия и список на месте");

    // 5. «Показать ещё»: вторая страница; заявка на стыке страниц — один раз
    app.finder.set_mine(false);
    app.finder.set_status_index(0);
    app.finder.set_words("много".into());
    app.finder.invoke_edited();
    assert!(app.finder.get_stale(), "условия изменены — список ещё по прежним");
    app.finder.invoke_search();
    assert!(!app.finder.get_stale());
    pump_until(&jobs, || ids(&app).len() == 50);
    assert!(app.finder.get_has_more());
    assert_eq!(app.finder.get_message(), "Найдено: 120 · показано 50");
    app.finder.invoke_more();
    assert!(app.finder.get_busy(), "«ещё» грузится — «Показать ещё» недоступна");
    pump_until(&jobs, || ids(&app).len() == 99);
    assert_eq!(ids(&app), (1..100).collect::<Vec<_>>(), "повтор на стыке не показан");
    assert!(requests.lock().unwrap().iter().any(|r| path(r).ends_with("&pagesize=50&page=2")));

    // 6. ответ прежнего поиска в новый список не попадает
    app.finder.set_words("медленно".into());
    app.finder.invoke_search();
    assert!(app.finder.get_busy() && rows(&app).is_empty(), "новый поиск начинается с пустого списка");
    pump_until(&jobs, || asked("search=%D0%BC%D0%B5%D0%B4") > 0);
    app.finder.set_words("принтер".into());
    app.finder.invoke_search();
    pump_until(&jobs, || ids(&app) == vec![700001, 700002]);
    std::thread::sleep(Duration::from_millis(100));
    pump_until(&jobs, || true);
    assert_eq!(ids(&app), vec![700001, 700002], "ответ «медленно» опоздал и отброшен");

    // 7. ошибка условий — словами, без запроса заявок
    let lists = asked("/api/task?");
    app.finder.set_words("аб".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert_eq!(app.finder.get_message(), "Слова — минимум 3 символа");
    assert_eq!(asked("/api/task?"), lists);

    // 7а. все поля формы доходят до сервера (ревью «проверки» 3.2); пояснения и предупреждение — над списком
    app.finder.set_words("все условия".into());
    app.finder.set_mine(true);
    app.finder.set_executor("максимов".into());
    app.finder.set_creator("иванов".into());
    app.finder.set_observer_mine(true);
    app.finder.set_observer("сидоров".into());
    app.finder.set_status_index(2);
    app.finder.set_created_from("01.01.2026".into());
    app.finder.set_created_to("31.01.2026".into());
    app.finder.set_changed_from("01.02.2026".into());
    app.finder.set_changed_to("28.02.2026".into());
    app.finder.set_closed_from("01.03.2026".into());
    app.finder.set_closed_to("31.03.2026".into());
    app.finder.set_archived(false);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    let sent = requests.lock().unwrap().iter().map(|r| path(r)).rfind(|p| p.starts_with("/api/task?")).unwrap();
    for part in [
        "ExecutorIds=7,38472&",
        "StatusIds=31&",
        "CreatorIds=5,6&",
        "ObserverIds=7,41&",
        "CreatedMoreThan=2026-01-01%2000%3A00&CreatedLessThan=2026-02-01%2000%3A00&",
        "ChangedMoreThan=2026-02-01%2000%3A00&ChangedLessThan=2026-03-01%2000%3A00&",
        "ClosedMoreThan=2026-03-01%2000%3A00&ClosedLessThan=2026-04-01%2000%3A00&",
    ] {
        assert!(sent.contains(part), "в запросе нет «{part}»: {sent}");
    }
    assert!(!sent.contains("archive=true"), "без архивных: {sent}");
    let notes = app.finder.get_notes().to_string();
    for note in [
        "Исполнитель: Максимов М. С.",
        "Заявитель «иванов»: подошли 2 — Иванов А., Иванов Б.",
        "Наблюдатель: Сидоров С.",
        "«Я» и у исполнителя, и у наблюдателя",
    ] {
        assert!(notes.contains(note), "над списком нет «{note}»: {notes}");
    }
    assert!(
        app.finder
            .get_warning()
            .starts_with("Сервер вернул заявки вне выбранного периода или без выбранного наблюдателя (1 из 1 загруженных)"),
        "{}",
        app.finder.get_warning()
    );
    // «Сбросить» возвращает каждое поле
    app.finder.invoke_clear();
    assert!(
        app.finder.get_words().is_empty()
            && !app.finder.get_mine()
            && app.finder.get_executor().is_empty()
            && app.finder.get_creator().is_empty()
            && !app.finder.get_observer_mine()
            && app.finder.get_observer().is_empty()
            && app.finder.get_status_index() == 0
            && [
                app.finder.get_created_from(),
                app.finder.get_created_to(),
                app.finder.get_changed_from(),
                app.finder.get_changed_to(),
                app.finder.get_closed_from(),
                app.finder.get_closed_to()
            ]
            .iter()
            .all(|d| d.is_empty())
            && app.finder.get_archived(),
        "«Сбросить» — все поля по умолчанию"
    );

    // 7б. сбой сервера на странице — его текстом, не «ничего не найдено»
    app.finder.set_words("сбой сервера".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert!(app.finder.get_message().starts_with("ошибка сервера (HTTP 500)"), "{}", app.finder.get_message());

    // 7в. справочник статусов не прочитался — сказано; следующий поиск читает его снова
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));
    statuses_fail.store(true, Ordering::SeqCst);
    let read = asked("/api/taskstatus");
    app.finder.set_words("".into());
    app.finder.set_status_index(1);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert!(app.finder.get_message().starts_with("Список статусов не загрузился"), "{}", app.finder.get_message());
    statuses_fail.store(false, Ordering::SeqCst);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert_eq!(asked("/api/taskstatus"), read + 2, "неудача не запомнена — справочник прочитан снова");

    // 7г. клавиатура: правка в поле при списке — «условия изменены», Enter в поле — поиск
    let search = windows.borrow()[2].clone();
    app.finder.set_words("принтер".into());
    app.finder.invoke_search();
    pump_until(&jobs, || ids(&app) == vec![700001, 700002]);
    app.finder.invoke_focus_words(); // слова выделены — набор их заменяет
    type_text(search.as_ref(), "принтер");
    assert!(app.finder.get_stale(), "набрали в поле — список ещё по прежним условиям");
    let lists = asked("/api/task?");
    type_text(search.as_ref(), SharedString::from(Key::Return).as_str());
    pump_until(&jobs, || asked("/api/task?") > lists && !app.finder.get_busy());
    assert!(!app.finder.get_stale(), "Enter в поле — поиск по новым условиям");

    // 7д. вернулись прежние условия — это последний поиск человека, а не набранное без поиска (`LastSearch` у C#)
    app.finder.set_words("принтер".into());
    app.finder.invoke_search();
    pump_until(&jobs, || ids(&app) == vec![700001, 700002]);
    app.finder.set_words("черновик".into());
    app.window.invoke_server_search("много".into());
    pump_until(&jobs, || !app.finder.get_busy());
    app.finder.invoke_cancel();
    app.window.invoke_open_search();
    assert_eq!(app.finder.get_words(), "принтер", "вернулся последний поиск, а не «черновик»");

    // 7е. Enter в поле поиска доски («/» — к полю)
    app.finder.invoke_cancel();
    let board = windows.borrow()[0].clone();
    app.window.show().unwrap();
    board.set_size(slint::PhysicalSize::new(1100, 720));
    // щелчок по первой карточке «Входящих» — клавиши доски у неё (как в tests/ui.rs)
    click(board.as_ref(), 100.0, 110.0);
    type_text(board.as_ref(), "/");
    type_text(board.as_ref(), "принтер");
    type_text(board.as_ref(), SharedString::from(Key::Return).as_str());
    assert!(app.finder.window().is_visible() && app.finder.get_words() == "принтер", "Enter в поле доски — окно поиска со словами");
    pump_until(&jobs, || ids(&app) == vec![700001, 700002]);

    // 7ж. последняя проверка ведущего (/code-review high, 3.2)
    // «Я» у наблюдателя снимает «Я» у исполнителя (`OnObserverMineChanged`): иначе нашлись бы только заявки, где вы и то и другое
    let search = windows.borrow()[2].clone();
    search.set_size(slint::PhysicalSize::new(1120, 740));
    app.finder.set_mine(true);
    app.finder.set_observer_mine(false);
    click(search.as_ref(), 25.0, 245.0); // «Я» под «Наблюдатель» (окно 1120×740)
    assert!(app.finder.get_observer_mine(), "щелчок попал в «Я» наблюдателя");
    assert!(!app.finder.get_mine(), "«Я» у наблюдателя сняло «Я» у исполнителя");
    // карточка не прочиталась — переписку не спрашиваем (как ⟳ на доске)
    app.finder.set_observer_mine(false);
    app.finder.set_status_index(0);
    app.finder.set_words("принтер".into());
    app.finder.invoke_search();
    pump_until(&jobs, || ids(&app) == vec![700001, 700002]);
    let lives = asked("/api/tasklifetime?");
    app.finder.invoke_select(1); // карточки 700002 на сервере нет — 404
    pump_until(&jobs, || !app.finder.get_preview_busy());
    assert!(app.finder.get_preview_text().starts_with("Карточка не прочиталась"), "{}", app.finder.get_preview_text());
    assert_eq!(asked("/api/tasklifetime?"), lives, "переписку без карточки не спрашивали");
    // пустой справочник статусов не запоминается: следующий поиск читает его снова
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));
    statuses_empty.store(true, Ordering::SeqCst);
    let read = asked("/api/taskstatus");
    app.finder.set_status_index(1);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert!(app.finder.get_message().starts_with("Список статусов не загрузился"), "{}", app.finder.get_message());
    statuses_empty.store(false, Ordering::SeqCst);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert_eq!(asked("/api/taskstatus"), read + 2, "пустой справочник не запомнен");
    // правку, отменённую, пока шёл поиск, «условиями изменены» не считать (`IsStale` сравнивает условия)
    app.finder.set_status_index(0);
    app.finder.invoke_search();
    app.finder.set_words("принтерX".into());
    app.finder.invoke_edited();
    app.finder.set_words("принтер".into());
    app.finder.invoke_edited();
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![700001, 700002]);
    assert!(!app.finder.get_stale(), "условия те же, по которым искали");

    // 8. Esc — окно прячется
    app.finder.invoke_cancel();
    assert!(!app.finder.window().is_visible());

    // 9. доска только для чтения: «+ На доску» нет и ничего не пишется
    let other = board_dir("ro", &url);
    let ro = App::open(&other, false).unwrap();
    ro.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));
    let before = std::fs::read(other.join("tickets.json")).unwrap();
    ro.window.invoke_server_search("принтер".into());
    pump_until(&jobs, || ro.finder.get_rows().row_count() == 2);
    assert!(!ro.finder.get_writable(), "только чтение — кнопки нет");
    let cards = |a: &App| a.window.get_columns().iter().map(|c| c.cards.row_count()).sum::<usize>();
    let on_ro = cards(&ro);
    ro.finder.invoke_add(0);
    assert!(ro.finder.get_message().contains("только для чтения"), "{}", ro.finder.get_message());
    assert_eq!(cards(&ro), on_ro, "и в памяти доска не тронута");
    std::thread::sleep(SAVE_DELAY + Duration::from_millis(200));
    slint::platform::update_timers_and_animations();
    assert_eq!(std::fs::read(other.join("tickets.json")).unwrap(), before, "файл не тронут");

    // 10. API нет — сказано, запросов нет
    let total = requests.lock().unwrap().len();
    ro.use_client(None);
    ro.finder.invoke_search();
    assert!(ro.finder.get_message().starts_with("API не настроен"), "{}", ro.finder.get_message());
    assert!(!ro.finder.get_busy() && ro.finder.get_rows().row_count() == 0);
    assert_eq!(requests.lock().unwrap().len(), total);
}
