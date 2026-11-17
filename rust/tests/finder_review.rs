//! Окно «Поиск в Интрасервисе» без экрана: пять находок ревью, каждая — со своим сценарием (ожидания — из C#:
//! `SearchWindow.xaml.cs` `ShowSearch`, `SearchViewModel.Search`/`ClearResults`/`AddToBoard`). Одна проверка на всё:
//! платформа Slint ставится один раз на процесс, поэтому пять сценариев идут друг за другом в одном окне.

mod common;

use common::window::{Jobs, pump_until};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, PhysicalSize, SharedString};
use std::path::Path;
use std::time::{Duration, Instant};
use ticketboard::FoundRow;
use ticketboard::app::App;
use ticketboard::board::{Ticket, parse_tickets};
use ticketboard::intraservice::Client;

fn rows(app: &App) -> Vec<FoundRow> {
    app.finder.get_rows().iter().collect()
}

fn ids(app: &App) -> Vec<i32> {
    rows(app).iter().map(|r| r.id).collect()
}

fn file_tickets(dir: &Path) -> Vec<Ticket> {
    parse_tickets(&std::fs::read_to_string(dir.join("tickets.json")).unwrap()).unwrap()
}

/// Клавиша в окне: нажать и отпустить.
fn key(window: &slint::Window, text: impl Into<SharedString>) {
    let text = text.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    slint::platform::update_timers_and_animations();
}

/// Напечатать строку по символам в поле с фокусом.
fn type_text(window: &slint::Window, text: &str) {
    for c in text.chars() {
        key(window, c.to_string());
    }
}

/// Выполнить ровно `n` ответов потоков (каждый ответ — одна задача очереди) — без пауз «на глаз».
#[track_caller]
fn run_jobs(jobs: &Jobs, n: usize) {
    let until = Instant::now() + Duration::from_secs(15);
    let mut done = 0;
    while done < n {
        let pending = std::mem::take(&mut *jobs.lock().unwrap());
        done += pending.len();
        for job in pending {
            job();
        }
        assert!(done >= n || Instant::now() < until, "потоки ответили {done} раз из {n}");
        if done < n {
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
    }
}

/// Пути всех запросов, пришедших на сервер.
fn paths(requests: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Vec<String> {
    requests.lock().unwrap().iter().map(|r| r.split(' ').nth(1).unwrap_or("").to_string()).collect()
}

/// Как `run_jobs`, но по одной задаче и с проверкой после каждой: промежуточное состояние окна тоже видно.
#[track_caller]
fn run_jobs_each(jobs: &Jobs, n: usize, after_each: impl Fn()) {
    let until = Instant::now() + Duration::from_secs(15);
    let mut done = 0;
    while done < n {
        let pending = std::mem::take(&mut *jobs.lock().unwrap());
        for job in pending {
            job();
            done += 1;
            after_each();
        }
        if done < n {
            assert!(Instant::now() < until, "потоки ответили {done} раз из {n}");
            std::thread::sleep(Duration::from_millis(10));
            slint::platform::update_timers_and_animations();
        }
    }
}

fn tasks(ids: impl Iterator<Item = i32>) -> common::Reply {
    let rows: Vec<String> = ids.map(|id| format!(r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":28}}"#)).collect();
    let count = rows.len();
    common::Reply::json(&format!(
        r#"{{"Tasks":[{}],"Statuses":[{{"Id":28,"Name":"Закрыта"}}],"Paginator":{{"Count":{count},"Page":1,"PageCount":1}}}}"#,
        rows.join(",")
    ))
}

#[test]
fn the_search_window_review() {
    let (windows, jobs) = common::window::install();
    let (url, requests) = common::route(move |p| match p {
        "/api/user?getcurrentuserinfo=true" => common::Reply::json(r#"{"Id":7,"Name":"Я Сам"}"#),
        "/api/taskstatus" => common::Reply::json(r#"[{"Id":31,"Name":"Открыта"},{"Id":28,"Name":"Закрыта","IsFinal":true}]"#),
        // карточки 8xxxxx читаются медленно: пока сервер думает, человек успевает искать заново
        p if p.starts_with("/api/task/8") => {
            let mut slow =
                common::Reply::json(r#"{"Task":{"Id":800001,"Name":"Медленная","StatusName":"Закрыта","Description":"Долго читалась"}}"#);
            slow.delay = Duration::from_millis(400);
            slow
        }
        p if p.starts_with("/api/task/700005?") => {
            common::Reply::json(r#"{"Task":{"Id":700005,"Name":"Быстрая","StatusName":"Закрыта","Description":"Быстро прочиталась"}}"#)
        }
        p if p.starts_with("/api/tasklifetime?") => common::Reply::json(common::LIVE_LIFE),
        p if p.starts_with("/api/task?") && p.contains("search=openq&") => tasks(820001..820002),
        p if p.starts_with("/api/task?") && p.contains("search=pair&") => tasks([800001, 700005].into_iter()),
        p if p.starts_with("/api/task?") && p.contains("search=slowsearch&") => {
            let mut slow = tasks(840001..840002);
            slow.delay = Duration::from_millis(300);
            slow
        }
        p if p.starts_with("/api/task?") && p.contains("search=slowcard&") => tasks(800001..800002),
        p if p.starts_with("/api/task?") && p.contains("search=quick&") => tasks(810001..810002),
        p if p.starts_with("/api/task?") && p.contains("search=newone&") => tasks(900001..900003),
        p if p.starts_with("/api/task?") => tasks(0..0),
        _ => common::Reply::new(404, "text/plain", "мимо"),
    });
    let dir = std::env::temp_dir().join(format!("tb-rs-finder-review-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    std::fs::write(
        dir.join("settings.json"),
        format!(r#"{{"WipLimit": 0, "IntraserviceBaseUrl": "{url}", "IntraserviceLogin": "me", "IntraservicePasswordProtected": "AQAA"}}"#),
    )
    .unwrap();
    let app = App::open(&dir, true).unwrap();
    app.use_client(Some(Client::new(&url, "me", "p", Duration::from_secs(10))));
    // окна по порядку создания: доска, быстрое добавление, поиск
    windows.borrow()[2].set_size(PhysicalSize::new(1120, 740));
    let search = app.finder.window();

    // 1. Поля дат: после набора текста «Сбросить» очищает поле на экране, а не только свойство
    app.window.invoke_open_search();
    assert!(search.is_visible());
    app.finder.invoke_focus_words();
    let mut found = false;
    for _ in 0..16 {
        key(search, Key::Tab);
        key(search, "Z");
        if app.finder.get_created_from() == "Z" {
            found = true;
            break;
        }
    }
    assert!(found, "Tab довёл фокус до поля «Создана с»");
    key(search, Key::Backspace);
    type_text(search, "01.03.2026");
    assert_eq!(app.finder.get_created_from(), "01.03.2026", "набранное попало в условие");
    app.finder.invoke_clear();
    assert_eq!(app.finder.get_created_from(), "", "«Сбросить» очистило условие");
    key(search, "5");
    assert_eq!(
        app.finder.get_created_from(),
        "5",
        "после «Сбросить» поле даты показывает пустое: новый набор начинается с чистого поля, а не дописывается к старому тексту"
    );

    app.finder.set_created_from("".into()); // чтобы остальные сценарии не зависели от итога первого
    // 2. Ответ просмотра, пришедший после нового поиска или после возврата прежних условий, в окно не попадает
    app.finder.set_words("slowcard".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![800001]);
    app.finder.invoke_select(0); // карточка читается 400 мс
    assert!(app.finder.get_preview_busy());
    app.finder.invoke_search(); // новый поиск, пока просмотр не ответил
    run_jobs(&jobs, 2); // ответ просмотра и ответ поиска
    assert!(!app.finder.get_busy() && ids(&app) == vec![800001], "новый поиск дошёл до конца");
    assert_eq!(app.finder.get_preview_text(), "", "ответ просмотра прежней строки после нового «Найти» не показан");
    assert!(!app.finder.get_preview_busy(), "после нового «Найти» просмотр не занят");
    // то же при возврате прежних условий: быстрый поиск → строка → Esc → кнопка
    app.window.invoke_server_search("quick".into());
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![810001]);
    app.finder.invoke_select(0);
    assert!(app.finder.get_preview_busy());
    app.finder.invoke_cancel();
    app.window.invoke_open_search();
    // прежние — последний поиск человека («slowcard»: «Я» снято «Сбросить» в п. 1), как `LastSearch` у C#
    assert!(
        app.finder.get_words() == "slowcard" && !app.finder.get_mine() && ids(&app).is_empty(),
        "прежние условия вернулись, список быстрого поиска забыт"
    );
    run_jobs(&jobs, 1); // ответ просмотра
    assert_eq!(app.finder.get_preview_text(), "", "ответ просмотра после возврата прежних условий не показан");
    assert!(!app.finder.get_preview_busy(), "после возврата прежних условий просмотр не занят");

    // 3. Кнопка при видимом окне после быстрого поиска только выводит его вперёд: условия и список на месте
    app.window.invoke_server_search("quick".into());
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![810001]);
    assert!(search.is_visible() && app.finder.get_words() == "quick");
    app.window.invoke_open_search();
    assert!(search.is_visible());
    assert_eq!(app.finder.get_words(), "quick", "окно не прятали — условия быстрого поиска остались");
    assert_eq!(ids(&app), vec![810001], "окно не прятали — список остался");

    // 4. Поиск человека с ошибкой (дата 01.13.2026) не заканчивает быстрый режим: после Esc кнопка возвращает прежние условия
    app.window.invoke_server_search("quick".into());
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![810001]);
    app.finder.set_created_from("01.13.2026".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert!(!app.finder.get_message().is_empty(), "дата 01.13.2026 — ошибка условий, сказана словами");
    assert!(ids(&app).is_empty(), "при ошибке список пуст");
    app.finder.invoke_cancel();
    assert!(!search.is_visible());
    app.window.invoke_open_search();
    // прежние — последний удавшийся поиск человека («slowcard», п. 2), как `LastSearch` у C#; не «quick» и не дата с ошибкой
    assert!(
        app.finder.get_words() == "slowcard" && app.finder.get_created_from().is_empty(),
        "поиск с ошибкой не закончил быстрый режим: прежние условия вернулись (слова «{}», создана с «{}»)",
        app.finder.get_words(),
        app.finder.get_created_from()
    );
    assert_eq!(app.finder.get_created_from(), "", "и дата быстрого поиска не осталась");

    // 5. «+ На доску» у строки, заявку которой добавили другим путём после показа списка: строка становится «на доске»
    app.finder.set_created_from("".into()); // условия сценария 4 не должны мешать
    app.finder.set_mine(false);
    app.finder.set_status_index(0);
    app.finder.set_words("newone".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![900001, 900002]);
    assert!(!rows(&app)[0].on_board, "до добавления заявки на доске нет");
    app.show_capture();
    app.capture.set_text("900001".into());
    app.capture.invoke_edited("900001".into());
    app.capture.invoke_accepted();
    app.save_now();
    assert_eq!(
        file_tickets(&dir).iter().filter(|t| t.intraservice_id == Some(900001)).count(),
        1,
        "быстрое добавление положило #900001 на доску"
    );
    assert!(!rows(&app)[0].on_board, "окно поиска про добавление не знает: строка ещё с кнопкой");
    app.finder.invoke_add(0);
    assert!(rows(&app)[0].on_board, "заявка уже на доске (добавлена через быстрое добавление): строка помечена «на доске»");
    app.save_now();
    let on_board = file_tickets(&dir).iter().filter(|t| t.intraservice_id == Some(900001)).count();
    assert_eq!(on_board, 1, "на доске ровно одна карточка #900001");

    // 6. Статус «Открытые» уходит на сервер открытыми статусами справочника (StatusIds=31), а не «любым»
    run_jobs(&jobs, 1); // ответ чтения карточки из сценария 5
    app.finder.set_mine(false);
    app.finder.set_created_from("".into());
    app.finder.set_status_index(2);
    app.finder.set_words("openq".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![820001]);
    assert!(
        paths(&requests).iter().any(|p| p.contains("search=openq&") && p.contains("StatusIds=31")),
        "«Открытые» ушли на сервер как открытые статусы (StatusIds=31): {:?}",
        paths(&requests).iter().filter(|p| p.contains("search=openq&")).collect::<Vec<_>>()
    );

    // 7. «Показать ещё», когда страниц больше нет: ничего не спрашивается, окно не занято
    let before = paths(&requests).len();
    assert!(!app.finder.get_has_more());
    app.finder.invoke_more();
    assert!(!app.finder.get_busy(), "страниц больше нет — «Показать ещё» ничего не начинает");
    assert_eq!(paths(&requests).len(), before, "страниц больше нет — запроса нет");

    // 8. Строка не выбрана — выбранной нет (-1)
    assert_eq!(app.finder.get_selected(), -1, "после поиска ничего не выбрано");

    // 9. Правка условия до поиска, когда списка нет, «условия изменены» не показывает; со списком — показывает
    app.finder.invoke_edited();
    assert!(app.finder.get_stale(), "список есть, условия правили — «условия изменены»");
    app.finder.set_status_index(0);
    app.finder.set_words("nothing".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert!(ids(&app).is_empty());
    app.finder.invoke_edited();
    assert!(!app.finder.get_stale(), "списка нет — «условия изменены» не показывается");

    // 11. Из двух просмотров верен последний: ответ медленной первой строки не показывается ни на миг
    app.finder.set_words("pair".into());
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy() && ids(&app) == vec![800001, 700005]);
    app.finder.invoke_select(0); // карточка читается 400 мс
    app.finder.invoke_select(1);
    run_jobs_each(&jobs, 2, || {
        assert!(
            !app.finder.get_preview_text().contains("id: 800001\n"),
            "ответ просмотра прежней строки не показан: {}",
            app.finder.get_preview_text()
        );
    });
    assert!(app.finder.get_preview_text().starts_with("---\nid: 700005\n"), "{}", app.finder.get_preview_text());
    assert!(app.finder.get_preview_title().contains("700005"));

    // 10. Другой клиент: список, просмотр и справочник забыты; ответы начатого до смены не нужны
    app.window.set_selected_key("".into());
    app.finder.invoke_select(0);
    assert!(!app.finder.get_preview_title().is_empty());
    app.use_client(Some(Client::new(&url, "other", "p", Duration::from_secs(10))));
    assert!(ids(&app).is_empty() && app.finder.get_selected() == -1, "после смены клиента список забыт");
    assert!(app.finder.get_preview_title().is_empty(), "после смены клиента просмотр очищен");
    run_jobs(&jobs, 1); // ответ просмотра прежнего клиента
    assert_eq!(app.finder.get_preview_text(), "", "ответ просмотра прежнего клиента не показан");
    assert!(!app.finder.get_preview_busy());
    app.finder.set_words("slowsearch".into());
    app.finder.invoke_search();
    assert!(app.finder.get_busy());
    app.use_client(Some(Client::new(&url, "other2", "p", Duration::from_secs(10))));
    assert!(!app.finder.get_busy(), "смена клиента остановила идущий поиск");
    run_jobs(&jobs, 1); // ответ поиска прежнего клиента
    assert!(ids(&app).is_empty() && !app.finder.get_busy(), "ответ поиска прежнего клиента отброшен: {:?}", ids(&app));
    // справочник читается заново
    app.finder.set_status_index(1);
    app.finder.set_words("closedq".into());
    let reads = |r: &_| paths(r).iter().filter(|p| p.contains("/api/taskstatus")).count();
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    let first = reads(&requests);
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert_eq!(reads(&requests), first, "справочник статусов читается один раз");
    app.use_client(Some(Client::new(&url, "other3", "p", Duration::from_secs(10))));
    app.finder.invoke_search();
    pump_until(&jobs, || !app.finder.get_busy());
    assert_eq!(reads(&requests), first + 1, "после смены клиента справочник статусов читается заново");
}
