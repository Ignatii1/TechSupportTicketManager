//! Автовыгрузка закрытых заявок без окна: таймер, файл отметки, строка трея, лог (`MainViewModel.AutoExport.cs`, карточка 4.5-3).
//! Сам заход — `export::pass::run_pass`, правила — `export::closed`; окно только зовёт `AutoExport::pass` в потоке и показывает
//! строку итога в подсказке трея (`app.rs`).

use crate::export::closed::{BUDGET, ExportFailures, ExportMark, ExportReach, ExportScope, due, mark_serves, took};
use crate::export::files::{ignore_temps_in_git, write_atomic};
use crate::export::pass::run_pass;
use crate::export::run::Options;
use crate::intraservice::{Client, Date, User};
use crate::journal;
use crate::settings::Settings;
use chrono::DateTime;
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Первый заход — через столько после запуска (`ApplyAutoSync`).
pub const FIRST_PASS: Duration = Duration::from_secs(15);

/// Файл отметки прототипа — рядом с exe: `settings.json` он не пишет, а отметки C# (`AutoExportSince`…) не читает — одно
/// приложение за раз.
pub const MARK_FILE: &str = "autoexport-rs.json";

/// Заголовок подсказки трея.
const TRAY_HEAD: &str = "Заявки — прототип на Rust";

/// Период заходов: `max(1, минуты)` минут (таймер автообновления).
pub fn next_pass_in(minutes: i32) -> Duration {
    Duration::from_secs(minutes.max(1) as u64 * 60)
}

/// Подсказка трея: заголовок и строка автовыгрузки, не длиннее 127 единиц UTF-16, иначе первые 126 и «…» (`UpdateTrayIcon`).
pub fn tray_tip(state: &str) -> String {
    let tip = if state.is_empty() { TRAY_HEAD.to_string() } else { format!("{TRAY_HEAD}\n{state}") };
    if tip.encode_utf16().count() <= 127 {
        return tip;
    }
    // знак, что не влезает в 126 единиц целиком, не режем пополам
    let mut cut = String::new();
    let mut units = 0;
    for c in tip.chars() {
        units += c.len_utf16();
        if units > 126 {
            break;
        }
        cut.push(c);
    }
    cut + "…"
}

/// Учётная запись (`AccountKey`): адрес без «/» на конце и логин, без учёта регистра.
pub fn account_key(settings: &Settings) -> String {
    format!("{}|{}", settings.base_url.trim().trim_end_matches('/'), settings.login.trim()).to_lowercase()
}

/// Та же папка выгрузки (`SameFolder`): пусто — «knowledge» рядом с данными; по полному пути, без учёта регистра и косой на конце.
pub fn same_folder(a: &str, b: &str, data_dir: &Path) -> bool {
    full_folder(a, data_dir) == full_folder(b, data_dir)
}

fn full_folder(setting: &str, data_dir: &Path) -> String {
    let path = match setting.trim() {
        "" => data_dir.join("knowledge"),
        dir => PathBuf::from(dir),
    };
    let path = std::path::absolute(&path).unwrap_or(path);
    // «..» и «.» разбираем сами: `GetFullPath` их убирает, а `absolute` — нет
    let mut full = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                full.pop();
            }
            Component::CurDir => {}
            part => full.push(part),
        }
    }
    full.to_string_lossy().to_lowercase()
}

/// Отметка на диске: папка как в настройках, учётная запись, охват, время, номера минуты.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedMark {
    pub folder: String,
    pub account: String,
    pub reach: ExportReach,
    pub since: Date,
    pub seen: Vec<i32>,
}

/// Отметка из `autoexport-rs.json`; нет файла или он не читается — отметки нет (неделя назад).
pub fn read_mark(data_dir: &Path) -> Option<SavedMark> {
    let text = std::fs::read_to_string(data_dir.join(MARK_FILE)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let seen = json.get("seen")?.as_array()?.iter().map(|n| n.as_i64().and_then(|n| i32::try_from(n).ok())).collect::<Option<Vec<_>>>()?;
    Some(SavedMark {
        folder: json.get("folder")?.as_str()?.to_string(),
        account: json.get("account")?.as_str()?.to_string(),
        reach: ExportReach::parse(json.get("reach")?.as_str()?)?,
        since: DateTime::parse_from_rfc3339(json.get("since")?.as_str()?).ok()?,
        seen,
    })
}

/// Записать отметку атомарно: оборванная запись не оставит полфайла.
pub fn write_mark(data_dir: &Path, mark: &SavedMark) -> io::Result<()> {
    let json = serde_json::json!({
        "folder": mark.folder,
        "account": mark.account,
        "reach": mark.reach.to_string(),
        "since": mark.since.to_rfc3339(),
        "seen": mark.seen,
    });
    write_atomic(&data_dir.join(MARK_FILE), &serde_json::to_string_pretty(&json).map_err(io::Error::other)?)
}

/// Состояние автовыгрузки между заходами (поля `MainViewModel`: отметка, «не мои», сбойные, отдых зеркала, прежняя ошибка).
/// Настройки читаются один раз при запуске: окна настроек нет, а TicketBoard.exe параллельно не работает.
pub struct AutoExport {
    settings: Settings,
    data_dir: PathBuf,
    /// Папка `errors.log` — только у доски, что пишется.
    journal: Option<PathBuf>,
    mark: Option<SavedMark>,
    not_mine: BTreeMap<i32, Date>,
    failures: ExportFailures,
    /// Зеркало отдыхает: охват и начало его последнего захода, догнавшего без ошибок (`_exportRest`).
    rest: Option<(ExportReach, Date)>,
    me: Option<User>,
    /// В лог — только новая ошибка, а не одна и та же каждые 5 минут (`_autoExportError`).
    last_error: String,
    /// Ошибки .gitignore, что лог уже знает (`_gitignoreLogged`), в нижнем регистре.
    gitignore_logged: HashSet<String>,
}

impl AutoExport {
    /// Читает отметку и забывает чужую: другая папка, другая учётная запись или охват, которому она не служит.
    pub fn new(settings: &Settings, data_dir: &Path, journal: Option<PathBuf>) -> AutoExport {
        let mark = read_mark(data_dir).filter(|m| {
            same_folder(&m.folder, &settings.knowledge_dir, data_dir)
                && m.account == account_key(settings)
                && mark_serves(Some(m.reach), settings.auto_export_reach)
        });
        AutoExport {
            settings: settings.clone(),
            data_dir: data_dir.to_path_buf(),
            journal,
            mark,
            not_mine: BTreeMap::new(),
            failures: ExportFailures::default(),
            rest: None,
            me: None,
            last_error: String::new(),
            gitignore_logged: HashSet::new(),
        }
    }

    /// Автовыгрузка может идти (`ExportCanRun`): включена, автообновление не на нуле, сервер задан; и доска пишется —
    /// иначе прототип нигде ничего не пишет.
    pub fn can_run(&self, has_client: bool) -> bool {
        self.settings.auto_export_closed && self.settings.auto_sync_minutes > 0 && has_client && self.journal.is_some()
    }

    /// Один заход, синхронно (окно гонит его в потоке): строка итога или None — пропущен (нельзя, зеркало отдыхает, остановили).
    /// `now` — часы: время в итоге и отдых зеркала считаются по ним.
    pub fn pass(&mut self, client: &Client, now: Date, stop: &AtomicBool, options: &Options) -> Option<String> {
        let start = Instant::now();
        self.pass_at(client, now, &|| start.elapsed(), stop, options)
    }

    /// То же, но сколько прошло с начала захода — от `lapse` (шов часов: проверка не ждёт минуту по-настоящему); время в строке
    /// трея — конец захода, `now` + `lapse()`.
    pub fn pass_at(&mut self, client: &Client, now: Date, lapse: &dyn Fn() -> Duration, stop: &AtomicBool, options: &Options) -> Option<String> {
        let _ = lapse; // ponytail: шов пока ничего не делает
        if !self.can_run(true) {
            return None;
        }
        let settings = self.settings.clone();
        let reach = settings.auto_export_reach;
        let dir = settings.knowledge_path(&self.data_dir);
        // зеркало после догнавшего захода отдыхает час; мои — каждым заходом. Отдых — от начала захода: от конца он пропускал бы
        // ближайший тик таймера
        if !due(reach, self.rest, now) {
            return None;
        }
        // отметка — первым делом: в первый заход «неделя назад» записывается сразу, а не после первого удачного: иначе, пока
        // заходы не удаются (папки нет), окно ползло бы вперёд
        let mark = match &self.mark {
            Some(m) => ExportMark { since: m.since, seen: m.seen.clone() },
            None => self.keep_mark(ExportMark::start(now), reach),
        };
        // папка — до любых запросов, одним шагом с .gitignore (строка *.tmp — до первого файла): папки нет и не создать (диск
        // без VPN) — сразу «не удалось», сервер не спрошен
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return self.failed(format!("{}: {e}", dir.display()), now, stop);
        }
        self.keep_temp_files_out_of_git(&dir);
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        // кто я — раз за запуск (`MeAsync`): не ответил — спросим снова
        let me = match &self.me {
            Some(me) => me.clone(),
            None => match client.get_current_user() {
                Ok(me) => self.me.insert(me).clone(),
                Err(e) => return self.failed(e, now, stop),
            },
        };
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        // закрытые статусы — справочник каждый раз; «всем заявкам» он ни к чему
        let closed_names = settings.closed_names();
        let mut closed = Vec::new();
        if reach != ExportReach::All {
            let statuses = match client.get_statuses() {
                Ok(statuses) => statuses,
                Err(e) => return self.failed(e, now, stop),
            };
            // пустой справочник и «ни одного закрытого» — разные беды: первая на сервере, вторую чинит сам пользователь
            if statuses.is_empty() {
                return self.failed("сервер не вернул ни одного статуса заявок".into(), now, stop);
            }
            closed = statuses.iter().filter(|s| s.closed_by(&closed_names)).map(|s| s.id).collect();
            if closed.is_empty() {
                return self.failed("ни один статус не считается закрытым — проверьте ClosedStatusNames в settings.json".into(), now, stop);
            }
        }

        let scope = ExportScope { closed_status_ids: closed, me, closed_names, attachments: settings.knowledge_attachments, reach };
        let clock = Instant::now();
        let url = |id: i32| settings.ticket_url(id);
        let pass = run_pass(client, &scope, &mark, &self.not_mine, &self.failures, &dir, &url, BUDGET, now, stop, options);
        // остановили (выход) — итог не пишется и в лог на свою остановку не жалуемся
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        // зеркало догнало без ошибок — следующий через час; порция кончилась или сбой — следующим заходом, как у моих: иначе одна
        // сломанная заявка замедлила бы догон до раза в час на часы
        self.rest = (reach != ExportReach::Mine && !pass.more && pass.error.is_empty()).then_some((reach, now));
        self.not_mine = pass.not_mine;
        self.failures = pass.failures;
        for f in &pass.given_up {
            self.log(&format!(
                "Автовыгрузка: заявка #{} пропущена — не далась {} заходов подряд, с {}. Если она ваша — выгрузите её из окна «Поиск заявок»; \
                 изменится — автовыгрузка попробует снова. Последняя ошибка: {}",
                f.id,
                f.times,
                f.first.format("%d.%m %H:%M"),
                f.error
            ));
        }
        // отметка — с охватом этого захода: сменили охват посреди него — следующий решит, годится ли она (`MarkServes`)
        if pass.next != mark {
            self.keep_mark(pass.next, reach);
        }
        if !pass.error.is_empty() {
            return self.failed(pass.error, now, stop);
        }
        self.last_error.clear();
        let done = if pass.exported > 0 {
            format!("выгружено {}{}", pass.exported, took(clock.elapsed().saturating_sub(pass.waited)))
        } else {
            "новых нет".into()
        };
        let given_up =
            if pass.given_up.is_empty() { String::new() } else { format!(", пропущено {} (errors.log)", pass.given_up.len()) };
        Some(format!("база знаний: {}, {done}{given_up}{}", now.format("%H:%M"), if pass.more { ", догоняю" } else { "" }))
    }

    /// Отметку — в память и в `autoexport-rs.json`, вместе с папкой и охватом, к которым она относится (`KeepMark`).
    fn keep_mark(&mut self, mark: ExportMark, reach: ExportReach) -> ExportMark {
        let saved = SavedMark {
            folder: self.settings.knowledge_dir.trim().to_string(),
            account: account_key(&self.settings),
            reach,
            since: mark.since,
            seen: mark.seen.clone(),
        };
        if let Err(e) = write_mark(&self.data_dir, &saved) {
            self.log(&format!("Не удалось сохранить {MARK_FILE}: {e}")); // в памяти верно — до следующего раза
        }
        self.mark = Some(saved);
        mark
    }

    /// Итог «не удалось» и запись в лог — только новой ошибки (`AutoExportFailed`); остановленный заход молчит.
    fn failed(&mut self, error: String, now: Date, stop: &AtomicBool) -> Option<String> {
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        if error != self.last_error {
            self.log(&format!("Автовыгрузка: {error}")); // сервер лежит — одна запись в логе, а не каждые 5 минут
            self.last_error = error;
        }
        Some(format!("база знаний: не удалось ({}) — подробности в errors.log", now.format("%H:%M")))
    }

    /// *.tmp — в .gitignore папки базы, если она в git-репозитории; проверяется каждым заходом. Не вышло — в лог по разу на
    /// каждую причину, а пробуем каждым заходом (`KeepTempFilesOutOfGit`).
    fn keep_temp_files_out_of_git(&mut self, dir: &Path) {
        let why = ignore_temps_in_git(dir);
        if !why.is_empty() && self.gitignore_logged.insert(why.to_lowercase()) {
            self.log(&format!(
                "Автовыгрузка: {why}. Пробую каждым заходом; так и не выйдет — допишите строку *.tmp в этот .gitignore сами."
            ));
        }
    }

    fn log(&self, text: &str) {
        if let Some(dir) = &self.journal {
            journal::append(dir, text);
        }
    }
}
