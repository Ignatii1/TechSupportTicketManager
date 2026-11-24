//! Перерисовка на месте (rust-0.9.1, найдено на рабочем ПК: Ctrl+щелчок — «Копировать (N)» растёт, на «Выгрузить
//! выбранные (N)» число прежнее, хотя подпись в окне уже новая). На Windows программная отрисовка Slint рисует в тот же
//! буфер только изменившееся (возраст буфера 1 — `ReusedBuffer`, 2 — `SwappedBuffers`). Три одинаковых приложения делают
//! одно и то же: одно рисует с нуля (эталон), другое — на месте в свой буфер, третье — в два буфера по очереди; после
//! каждого щелчка картинки равны. Одна проверка на процесс — платформа ставится раз.

mod common;

use common::window::{install, pump_until};
use slint::platform::WindowEvent;
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::{Model, PhysicalSize};
use std::rc::Rc;
use std::time::Duration;
use ticketboard::app::App;
use ticketboard::intraservice::Client;

fn tasks(count: i32) -> common::Reply {
    let rows: Vec<String> = (1..=count).map(|id| format!(r#"{{"Id":{id},"Name":"Заявка {id}","StatusId":28}}"#)).collect();
    common::Reply::json(&format!(
        r#"{{"Tasks":[{}],"Statuses":[{{"Id":28,"Name":"Закрыта"}}],"Paginator":{{"Count":{count},"Page":1,"PageCount":1}}}}"#,
        rows.join(",")
    ))
}

type Screen = Vec<PremultipliedRgbaColor>;

/// Одно приложение: окно поиска и его буфер(ы) — два, если буферы меняются по очереди. Тип отрисовки — на каждый кадр, как
/// у Slint на Windows (`i-slint-backend-winit` `renderer/sw.rs`: по возрасту буфера): новый буфер — с нуля, прежний — на
/// месте (`mode`).
struct Run {
    app: App,
    search: Rc<MinimalSoftwareWindow>,
    mode: RepaintBufferType,
    screens: Vec<Screen>,
    fresh: Vec<bool>,
    frame: usize,
}

impl Run {
    fn draw(&mut self, size: PhysicalSize) -> &Screen {
        let (width, n) = (size.width as usize, self.screens.len());
        if self.screens[0].len() != width * size.height as usize {
            // окно стало другого размера — буферы новые (возраст 0)
            self.screens = vec![vec![PremultipliedRgbaColor::default(); width * size.height as usize]; n];
            self.fresh = vec![true; n];
        }
        let i = self.frame % n;
        let (screen, fresh, mode) = (&mut self.screens[i], &mut self.fresh[i], self.mode);
        self.search.draw_if_needed(|r| {
            r.set_repaint_buffer_type(if *fresh { RepaintBufferType::NewBuffer } else { mode });
            r.render(screen, width);
            *fresh = false;
        });
        self.frame += 1;
        &self.screens[i]
    }
}

fn differ(a: &Screen, b: &Screen) -> usize {
    a.iter().zip(b).filter(|(x, y)| (x.red, x.green, x.blue, x.alpha) != (y.red, y.green, y.blue, y.alpha)).count()
}

/// Где картинки разошлись: (x0, y0, x1, y1) в пикселях окна — в сообщении проверки (на CI картинок не посмотреть).
fn where_differ(a: &Screen, b: &Screen, width: usize) -> Option<(usize, usize, usize, usize)> {
    let mut boxed: Option<(usize, usize, usize, usize)> = None;
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if (x.red, x.green, x.blue, x.alpha) != (y.red, y.green, y.blue, y.alpha) {
            let (px, py) = (i % width, i / width);
            boxed = Some(boxed.map_or((px, py, px, py), |(x0, y0, x1, y1)| (x0.min(px), y0.min(py), x1.max(px), y1.max(py))));
        }
    }
    boxed
}

fn open(windows: &common::window::Windows, url: &str, n: usize) -> (App, Rc<MinimalSoftwareWindow>) {
    let dir = std::env::temp_dir().join(format!("tb-rs-repaint-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tickets.json"), "[]").unwrap();
    let app = App::open(&dir, true).unwrap();
    app.use_client(Some(Client::new(url, "me", "p", Duration::from_secs(10))));
    let search = windows.borrow()[3 * n + 2].clone();
    app.finder.set_folder(r"D:\База".into()); // папка — одна у всех: своя временная разнилась бы в картинке
    (app, search)
}

#[test]
fn picked_rows_are_repainted_where_they_stand() {
    let (windows, jobs) = install();
    use RepaintBufferType::*;
    // окна: доска, быстрое добавление, поиск — у каждого приложения по три
    let (url, _) = common::route(|p| match p {
        p if p.starts_with("/api/task?") => tasks(6),
        _ => common::Reply::new(404, "text/plain", "мимо"),
    });
    let mut runs: Vec<Run> = [NewBuffer, ReusedBuffer, SwappedBuffers]
        .into_iter()
        .enumerate()
        .map(|(n, mode)| {
            let (app, search) = open(&windows, &url, n);
            let count = if mode == SwappedBuffers { 2 } else { 1 };
            Run { app, search, mode, screens: vec![Vec::new(); count], fresh: vec![true; count], frame: 0 }
        })
        .collect();

    for scale in [1.0f32, 1.25, 1.5] {
        let size = PhysicalSize::new((1000.0 * scale) as u32, (700.0 * scale) as u32);
        for r in &mut runs {
            r.search.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
            r.search.set_size(size);
            r.app.window.invoke_server_search("принтер".into());
        }
        pump_until(&jobs, || runs.iter().all(|r| r.app.finder.get_rows().row_count() == 6 && !r.app.finder.get_busy()));
        for r in &mut runs {
            r.search.request_redraw();
            r.draw(size);
            if r.screens.len() == 2 {
                r.search.request_redraw();
                r.draw(size);
            }
        }
        // щелчок, Ctrl+щелчок, ещё Ctrl, Ctrl снимает, Shift — подряд, щелчок
        for (step, (row, ctrl, shift)) in
            [(0, false, false), (2, true, false), (4, true, false), (2, true, false), (5, false, true), (1, false, false)]
                .into_iter()
                .enumerate()
        {
            for r in &runs {
                r.app.finder.invoke_pick(row, ctrl, shift);
            }
            pump_until(&jobs, || runs.iter().all(|r| !r.app.finder.get_preview_busy()));
            slint::platform::update_timers_and_animations();
            let labels: Vec<String> = runs.iter().map(|r| r.app.finder.get_export_label().to_string()).collect();
            assert!(labels.iter().all(|l| *l == labels[0]), "подписи одинаковы у всех трёх: {labels:?}");
            let reference = runs[0].draw(size).clone();
            for (i, name) in [(1, "на месте"), (2, "два буфера")] {
                let screen = runs[i].draw(size);
                assert_eq!(
                    differ(screen, &reference),
                    0,
                    "масштаб {scale}, шаг {step} ({}): {name} нарисовано не то, что с нуля, место {:?} из {}×{}",
                    labels[0],
                    where_differ(screen, &reference, size.width as usize),
                    size.width,
                    size.height
                );
            }
        }
    }
}
