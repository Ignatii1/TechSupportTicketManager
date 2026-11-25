//! Выгрузка выбранных заявок файлами (`KnowledgeExport.ExportRowsAsync`, `Job`, `Conclude`, `Retrying`; `SearchViewModel.Summary`):
//! по четыре заявки разом, не менявшиеся пропускаются без запросов, остальные читаются целиком (карточка и вся история, сбои
//! сети и сервера — с повторами) и ложатся файлом на место; выключатель при серии сбоев, ход для окна, «Остановить», проверка,
//! что записанное не пропало, оглавления в конце. Вложения не качаются (область «Никакие»): лежащие в базе — ссылкой.

use super::files::{self, FilesSeen, Known};
use super::{Attachment, Context, MAX_LIFETIME_PAGES, format_with, with_details};
use crate::intraservice::query::{CHANGED_ASC_SORT, DEFAULT_SORT, STABLE_SORT, TaskQuery};
use crate::intraservice::{Client, Event, Found, LIFETIME_PAGE_SIZE, SearchPage, Task, brief, capped, headline, http_code, is_transient};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub const STARTING: &str = "Начинаю…";

/// Запросов к серверу разом: он общий, с остальными пользователями (`MaxParallel`).
const MAX_PARALLEL: usize = 4;
/// Подряд столько заявок не прочиталось из-за сети или сервера, и ни одной удачной между ними — сервер лёг или пропала сеть
/// (`TransientLimit`).
const TRANSIENT_LIMIT: usize = 30;
/// Подряд столько ответов 500 без единой удачи — сервер сломался (`ServerErrorLimit`). Заявке с 500 повтор один, не три.
const SERVER_ERROR_LIMIT: usize = 100;
/// Столько отказов (не сеть), а ни одна заявка не записалась и не оказалась готовой с прошлого раза — нет доступа к карточкам,
/// сменился адрес (`NothingWorksLimit`).
const NOTHING_WORKS_LIMIT: usize = 25;
/// Подряд столько файлов не записалось — диск полон или папка недоступна (`WriteFailLimit`).
const WRITE_FAIL_LIMIT: usize = 10;
/// Ход для окна — не чаще, чем раз в столько мс (четыре раза в секунду).
const REPORT_EVERY_MS: u64 = 250;
/// Скорость в ходе — не раньше стольких мс от первого замера (и десяти заявок) (`RateWarmup`).
const RATE_WARMUP_MS: u64 = 5_000;
/// Окно скорости — полминуты (`RateMeter.Window`).
const RATE_WINDOW_MS: u64 = 30_000;

const STOPPED_BEFORE_START: &str = "Остановлено до начала — ничего не записано.";
const CHECKING: &str = "Проверяю, что файлы на месте…";

/// Паузы перед повторами сбойного запроса (сеть, 5xx): 2, 5 и 15 секунд (`RetryDelays`); и сколько ждать, прежде чем проверять,
/// что записанные файлы на месте: антивирус удаляет не в ту же миллисекунду (`VanishWait`).
#[derive(Clone, Debug)]
pub struct Options {
    pub retry_delays: Vec<Duration>,
    pub vanish_wait: Duration,
    /// Срок одной страницы списка при выгрузке по отбору (`ListTimeout`): сервер сортирует и считает весь список.
    pub list_timeout: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            retry_delays: vec![Duration::from_secs(2), Duration::from_secs(5), Duration::from_secs(15)],
            vanish_wait: Duration::from_secs(3),
            list_timeout: Duration::from_secs(60),
        }
    }
}

/// Итог выгрузки (`ExportResult`).
#[derive(Clone, Debug, Default)]
pub struct ExportResult {
    pub found: usize,
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub first_error: String,
    pub error: String,
    pub complete: bool,
}

fn not_started(why: String) -> ExportResult {
    ExportResult { error: why, ..ExportResult::default() }
}

/// Скорость выгрузки, заявок в секунду: за последние полминуты, а пока их не прошло — средняя с первого замера, не раньше
/// десяти заявок и `RATE_WARMUP_MS`. Полминуты без прибавки — ноль: выгрузка стоит (`RateMeter`).
#[derive(Default)]
pub struct RateMeter {
    start: Option<(i64, i64)>,
    window: (i64, i64),
    window_rate: Option<f64>,
}

impl RateMeter {
    /// Время и счёт — со знаком, как `long` у C#: два работника, прошедшие отсечку хода разом, берут замок не по порядку.
    pub fn per_second(&mut self, now: u64, count: usize) -> f64 {
        let (now, count) = (now as i64, count as i64);
        let Some((start_at, start_count)) = self.start else {
            self.start = Some((now, count));
            self.window = (now, count);
            return 0.0;
        };
        if now - self.window.0 >= RATE_WINDOW_MS as i64 {
            self.window_rate = Some((count - self.window.1) as f64 / ((now - self.window.0) as f64 / 1000.0));
            self.window = (now, count);
        }
        if let Some(rate) = self.window_rate {
            return rate;
        }
        let (counted, elapsed) = (count - start_count, now - start_at);
        if counted >= 10 && elapsed >= RATE_WARMUP_MS as i64 { counted as f64 / (elapsed.max(1) as f64 / 1000.0) } else { 0.0 }
    }
}

/// Строка хода (`ProgressLine`): легло, из скольких, не выгружено и «осталось» либо скорость. Скорость — `rate(сколько)`:
/// по прошедшим, когда общее число известно, по легшим, когда нет; прошли все — без неё.
pub fn progress_line(landed: usize, done: usize, target: usize, failed: usize, rate: impl FnOnce(usize) -> f64) -> String {
    let mut line = if target > 0 && target >= done {
        format!("Выгружено {landed} из {target}")
    } else {
        format!("Выгружено {landed}")
    };
    if failed > 0 {
        line.push_str(&format!(" · не выгружено {failed}"));
    }
    // «осталось» — по прошедшим (их и ждём); скорость без общего числа — по легшим, как «выгружено» рядом с ней
    let speed = if target == 0 {
        rate(landed)
    } else if target > done {
        rate(done)
    } else {
        0.0
    };
    if speed > 0.0 {
        if target > done {
            let left = ((target - done) as f64 / speed).min(1e9);
            line.push_str(&format!(" · осталось ~{}", human_span(Duration::from_secs_f64(left))));
        } else {
            line.push_str(&format!(" · ~{} в минуту", (speed * 60.0).round_ties_even() as i64));
        }
    }
    line
}

/// Срок для человека: «меньше минуты», «42 мин», «3 ч 05 мин» (`HumanSpan`).
pub fn human_span(t: Duration) -> String {
    let minutes = t.as_secs() / 60;
    match minutes {
        0 => "меньше минуты".to_string(),
        1..=59 => format!("{minutes} мин"),
        _ => format!("{} ч {:02} мин", minutes / 60, minutes % 60),
    }
}

/// Места для запросов к серверу — их `MAX_PARALLEL`, и страница списка занимает одно, как запрос по заявке: «не больше четырёх
/// разом» держится и когда следующая страница запрошена заранее (`Gate`). Очередь честная: четыре работника, вернув место и тут
/// же взяв следующую заявку, не оставят страницу без места.
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

struct GateState {
    free: usize,
    next: u64,
    queue: VecDeque<u64>,
}

impl Gate {
    fn new(places: usize) -> Gate {
        Gate { state: Mutex::new(GateState { free: places, next: 0, queue: VecDeque::new() }), changed: Condvar::new() }
    }

    /// Занять место; false — остановили, пока ждали.
    fn enter(&self, cancelled: impl Fn() -> bool) -> bool {
        let mut state = locked(&self.state);
        let ticket = state.next;
        state.next += 1;
        state.queue.push_back(ticket);
        loop {
            if state.queue.front() == Some(&ticket) && state.free > 0 {
                state.queue.pop_front();
                state.free -= 1;
                self.changed.notify_all();
                return true;
            }
            if cancelled() {
                state.queue.retain(|t| *t != ticket);
                self.changed.notify_all();
                return false;
            }
            state = self.changed.wait_timeout(state, Duration::from_millis(10)).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn leave(&self) {
        locked(&self.state).free += 1;
        self.changed.notify_all();
    }
}

/// Что прочиталось о заявке: карточка, вся история и урезана ли она.
type Read = (Task, Vec<Event>, bool);

/// Одна выгрузка: общие счётчики, выключатель и всё, что нужно шагу одной заявки (`Job`). Счётчики трогают до четырёх заявок
/// разом; `existing` после подготовки только читается.
struct Job<'a> {
    client: &'a Client,
    root: &'a Path,
    tickets: PathBuf,
    ticket_url: &'a (dyn Fn(i32) -> String + Sync),
    context: &'a Context,
    progress: &'a (dyn Fn(String) + Sync),
    stop: &'a AtomicBool,
    options: &'a Options,
    gate: Gate,
    /// Выключатель: ставится и пользователем (`stop`), и серией сбоев (`trip`) — всё, что идёт к серверу, смотрит на него.
    abort: AtomicBool,
    /// Сколько заявок будет всего; 0 — неизвестно (при выгрузке по отбору узнаётся по ходу списка).
    target: AtomicUsize,
    clock: Instant,
    /// Месяцы, где что-то записано или убрано, — их оглавления перепишутся.
    touched: Mutex<Vec<String>>,
    /// Записанные файлы: номер, путь и когда записан (мс от начала): на месте ли они, проверяется в конце.
    written: Mutex<Vec<(i32, PathBuf, u64)>>,
    /// Записанное, чего на месте уже нет (`Vanished`).
    vanished: Mutex<Vec<i32>>,
    /// Месяцы, чьи оборванные записи уже убраны, — раз за выгрузку.
    swept: Mutex<HashSet<String>>,
    created: AtomicUsize,
    updated: AtomicUsize,
    unchanged: AtomicUsize,
    failed: AtomicUsize,
    done: AtomicUsize,
    /// Подряд сбоев сети и сервера; подряд ответов 500; отказов за всю выгрузку; подряд ошибок записи.
    transient: AtomicUsize,
    server_errors: AtomicUsize,
    refused: AtomicUsize,
    write_fails: AtomicUsize,
    tripped: AtomicBool,
    first_error: Mutex<String>,
    fatal: Mutex<String>,
    files: Mutex<FilesSeen>,
    last_report: AtomicU64,
    rate: Mutex<RateMeter>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Job<'_> {
    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    fn cancelled(&self) -> bool {
        self.abort.load(SeqCst) || self.stop.load(SeqCst)
    }

    /// Подождать, но не дольше, чем идёт выгрузка; false — остановили.
    fn sleep(&self, pause: Duration) -> bool {
        let until = Instant::now() + pause;
        while Instant::now() < until {
            if self.cancelled() {
                return false;
            }
            std::thread::sleep(until.saturating_duration_since(Instant::now()).min(Duration::from_millis(10)));
        }
        !self.cancelled()
    }

    fn touch(&self, shard: &str) {
        let mut touched = locked(&self.touched);
        if !shard.is_empty() && !touched.iter().any(|t| files::fold(t) == files::fold(shard)) {
            touched.push(shard.to_string());
        }
    }

    /// Выключатель: останавливает выгрузку по причине (первая причина остаётся, как бы ни совпали две заявки).
    fn trip(&self, reason: String) {
        if self.tripped.swap(true, SeqCst) {
            return;
        }
        *locked(&self.fatal) = reason;
        self.abort.store(true, SeqCst);
    }

    /// Заявка не прочиталась. Неверный логин — сразу стоп; серия сбоев сети без единого успеха, или ни одной удачной из первых
    /// многих — тоже: так выгрузка не долбит лежащий сервер и не жжёт часы впустую (`Fail`).
    fn fail(&self, id: i32, error: &str) {
        if self.failed.fetch_add(1, SeqCst) == 0 {
            *locked(&self.first_error) = format!("#{id}: {error}");
        }
        let code = http_code(error);
        if code == Some(401) {
            self.trip("Выгрузка прервана: сервер не принимает логин и пароль (HTTP 401) — проверьте их в настройках.".into());
        } else if is_transient(error) {
            if code == Some(500) {
                if self.server_errors.fetch_add(1, SeqCst) + 1 >= SERVER_ERROR_LIMIT {
                    self.trip(format!(
                        "Выгрузка прервана: {SERVER_ERROR_LIMIT} заявок подряд сервер ответил ошибкой 500 (последняя — #{id}: {}). Похоже, он не справляется или сломан: подождите и запустите выгрузку снова.",
                        brief(error)
                    ));
                }
            } else if self.transient.fetch_add(1, SeqCst) + 1 >= TRANSIENT_LIMIT {
                self.trip(format!(
                    "Выгрузка прервана: {TRANSIENT_LIMIT} заявок подряд не прочитались из-за сети или сервера (последняя — #{id}: {}). Проверьте связь; если компьютер засыпал — не давайте ему.",
                    brief(error)
                ));
            }
        } else if self.refused.fetch_add(1, SeqCst) + 1 >= NOTHING_WORKS_LIMIT
            && self.created.load(SeqCst) + self.updated.load(SeqCst) + self.unchanged.load(SeqCst) == 0
        {
            // в причине — отказ, на котором оборвалось: первая ошибка выгрузки могла быть и записью файла
            self.trip(format!(
                "Выгрузка прервана: ни одна из {NOTHING_WORKS_LIMIT} заявок не прочиталась (последняя — #{id}: {}).",
                brief(error)
            ));
        }
    }

    fn write_failed(&self, id: i32, message: &str) {
        if self.failed.fetch_add(1, SeqCst) == 0 {
            *locked(&self.first_error) = format!("#{id}: файл не записан: {message}");
        }
        if self.write_fails.fetch_add(1, SeqCst) + 1 >= WRITE_FAIL_LIMIT {
            self.trip(format!(
                "Выгрузка прервана: файлы не записываются ({WRITE_FAIL_LIMIT} подряд) — {message}. Проверьте место на диске, доступ к папке и длину пути к ней."
            ));
        }
    }

    /// Ход для окна: «Выгружено 1250 из 98000 · осталось ~3 ч 12 мин», не чаще четырёх раз в секунду; сколько всего,
    /// неизвестно — вместо «осталось» скорость (`Report`).
    fn report(&self, force: bool) {
        let now = self.now();
        if !force && now.saturating_sub(self.last_report.load(SeqCst)) < REPORT_EVERY_MS {
            return;
        }
        self.last_report.store(now, SeqCst);
        let (done, target) = (self.done.load(SeqCst), self.target.load(SeqCst));
        let landed = self.created.load(SeqCst) + self.updated.load(SeqCst) + self.unchanged.load(SeqCst);
        let line = progress_line(landed, done, target, self.failed.load(SeqCst), |n| locked(&self.rate).per_second(now, n));
        (self.progress)(line);
    }

    /// Вызвать `call`; сбой сети или сервера повторить после пауз из `retry_delays`: 500 — один раз, остальное — все паузы.
    /// Логин, доступ, «нет такой», непонятный ответ повтор не лечит — ответ возвращается сразу. None — остановили (`Retrying`).
    fn retrying<T>(&self, call: impl Fn() -> Result<T, String>) -> Option<Result<T, String>> {
        self.retrying_noting(call, |_| {})
    }

    /// То же; `noting` — что сказать перед паузой (ход для окна: «Список: … — повторяю…»).
    fn retrying_noting<T>(&self, call: impl Fn() -> Result<T, String>, noting: impl Fn(&str)) -> Option<Result<T, String>> {
        for attempt in 0.. {
            if self.cancelled() {
                return None;
            }
            let result = call();
            if self.cancelled() {
                return None;
            }
            let Err(error) = &result else { return Some(result) };
            let delays = &self.options.retry_delays;
            let limit = if http_code(error) == Some(500) { delays.len().min(1) } else { delays.len() };
            if !is_transient(error) || attempt >= limit {
                return Some(result);
            }
            noting(error);
            if !self.sleep(delays[attempt]) {
                return None;
            }
        }
        None
    }

    /// Карточка и вся история заявки — из них пишется файл; не прочиталось одно из двух — ошибка (`ReadAsync`,
    /// `ReadLifetimeAsync`). Сервер присылает номера страниц — верим ему; не присылает — конец по неполной странице; та же
    /// страница снова — новых записей нет, стоп. Дошли до `MAX_LIFETIME_PAGES`, а страницы ещё есть, — история урезана.
    fn read(&self, id: i32) -> Option<Result<Read, String>> {
        let task = match self.retrying(|| self.client.get_task(id))? {
            Ok(task) => task,
            Err(e) => return Some(Err(e)),
        };
        let mut all: Vec<Event> = Vec::new();
        let mut keys = HashSet::new();
        for page in 1..=MAX_LIFETIME_PAGES {
            let got = match self.retrying(|| self.client.get_lifetime_page(id, page))? {
                Ok(got) => got,
                Err(e) => return Some(Err(e)),
            };
            let (before, count) = (all.len(), got.events.len());
            for e in got.events {
                if keys.insert((e.date, e.author.clone(), e.status.clone(), e.comment.clone())) {
                    all.push(e);
                }
            }
            let more = if got.paged { got.has_more } else { count >= LIFETIME_PAGE_SIZE as usize };
            if all.len() == before || !more {
                break;
            }
            if page == MAX_LIFETIME_PAGES {
                return Some(Ok((task, all, true)));
            }
        }
        Some(Ok((task, all, false)))
    }

    /// Одна заявка: не менялась — пропуск; иначе карточка и переписка с сервера и файл на место. Сбой одной заявки считается
    /// и не останавливает остальные (до выключателя). false — остановили: заявка не обработана и не считается (`ExportCoreAsync`).
    fn export_core(&self, f: &Found, existing: &HashMap<i32, Known>) -> bool {
        let (shard, name) = files::place_of(&self.tickets, f);
        // наши файлы с номером этой заявки; заметка пользователя с номером в имени не удаляется и не мешает заявке считаться
        // неизменной; не менялась с прошлой выгрузки — переписку не перечитываем
        let (ours, unchanged) = files::ours_of(self.root, &self.tickets, existing, f);
        if unchanged {
            self.unchanged.fetch_add(1, SeqCst);
            return true;
        }
        let (task, events, truncated) = match self.read(f.id) {
            None => return false,
            Some(Err(error)) => {
                self.fail(f.id, &error);
                return true;
            }
            Some(Ok(read)) => read,
        };
        self.transient.store(0, SeqCst); // сервер отвечает — серии сбоев прервались
        self.server_errors.store(0, SeqCst);
        let row = with_details(f, &task);
        // вложения — до файла заявки: ссылки в нём ведут на уже лежащие файлы
        let mut seen = FilesSeen::default();
        let attachments: Vec<Attachment> = files::save_files(self.root, &row, &shard, &mut seen);
        self.add_files(seen);
        let text = format_with(&row, &events, &(self.ticket_url)(f.id), &crate::board::now(), truncated, self.context, Some(&attachments));
        let path = self.tickets.join(&shard).join(&name);
        let wrote = match path.parent() {
            Some(folder) => fs::create_dir_all(folder).and_then(|()| files::write_atomic(&path, &text)),
            None => files::write_atomic(&path, &text),
        };
        if let Err(e) = wrote {
            self.write_failed(f.id, &format!("{e}: '{}'", path.display())); // занят редактором, антивирусом — в следующий раз; много подряд — диск
            return true;
        }
        locked(&self.written).push((f.id, path, self.now()));
        self.touch(&shard);
        // переименовали заявку (или остался дубль) — прежние наши файлы с этим номером больше не нужны
        let rel = format!("{shard}/{name}");
        for stale in ours.iter().filter(|s| !files::same_path(s, &rel)) {
            let _ = fs::remove_file(self.tickets.join(stale));
            if let Some(folder) = Path::new(stale).parent().and_then(Path::file_name) {
                self.touch(&folder.to_string_lossy());
            }
        }
        self.write_fails.store(0, SeqCst);
        if ours.is_empty() {
            self.created.fetch_add(1, SeqCst)
        } else {
            self.updated.fetch_add(1, SeqCst)
        };
        files::prune_files(self.root, &row, &shard, &attachments);
        true
    }

    fn add_files(&self, seen: FilesSeen) {
        let mut all = locked(&self.files);
        all.present += seen.present;
        all.skipped += seen.skipped;
        all.failed += seen.failed;
        if all.first_error.is_empty() {
            all.first_error = seen.first_error;
        }
    }

    /// Проверить записанные файлы, которым уже `vanish_wait` (all — все): на месте ли они. Зовётся между страницами и в конце
    /// (`CheckWritten`); проверенные из списка уходят — на сотнях тысяч заявок он не растёт.
    fn check_written(&self, all: bool) {
        let ripe = self.now() as i64 - self.options.vanish_wait.as_millis() as i64;
        let mut written = locked(&self.written);
        let n = written.iter().take_while(|w| all || w.2 as i64 <= ripe).count();
        let gone: Vec<i32> = written.drain(..n).filter(|w| !w.1.exists()).map(|w| w.0).collect();
        locked(&self.vanished).extend(gone);
    }

    /// Дождаться, пока последнему записанному файлу исполнится `vanish_wait`; остановили — не ждать (`SettleAsync`).
    fn settle(&self) {
        let last = locked(&self.written).iter().map(|w| w.2).max();
        let Some(last) = last else { return };
        let until = last + self.options.vanish_wait.as_millis() as u64;
        if until <= self.now() {
            return;
        }
        (self.progress)(CHECKING.to_string());
        while self.now() < until && !self.stop.load(SeqCst) {
            std::thread::sleep(Duration::from_millis(until.saturating_sub(self.now()).min(10)));
        }
        self.report(true); // ход — снова «выгружено N»
    }

    /// Строка итога о вложениях этой выгрузки; ничего с ними не было — пусто (`FilesNote`).
    fn files_note(&self) -> String {
        let seen = locked(&self.files);
        let mut parts = Vec::new();
        if seen.present > 0 {
            parts.push(format!("уже были {}", seen.present));
        }
        if seen.skipped > 0 {
            parts.push(format!("не скачано (настройка «Вложения в базу знаний»): {}", seen.skipped));
        }
        let failed = if seen.failed > 0 {
            format!("не скачались {} — повторная выгрузка попробует их снова: {}", seen.failed, seen.first_error)
        } else {
            String::new()
        };
        if parts.is_empty() && failed.is_empty() {
            return String::new();
        }
        let both = !parts.is_empty() && !failed.is_empty();
        format!("Вложения: {}{}{failed}.", parts.join(", "), if both { "; " } else { "" })
    }

    /// Конец любой выгрузки: оглавления тронутых месяцев (даже если остановили или прервали — файлы уже лежат) и строки итога
    /// о том, как она кончилась (`Conclude`).
    fn conclude(&self, dir: &Path, stopped: bool, mut complete: bool, git_note: &str, mut notes: Vec<String>) -> ExportResult {
        // записанное, которого уже нет: файл удалили после записи — обычно антивирус (карантин) по содержимому переписки
        self.check_written(true);
        let mut vanished: Vec<i32> = locked(&self.vanished).clone();
        vanished.sort_unstable();
        vanished.dedup();
        if !vanished.is_empty() {
            let ids: Vec<String> = vanished.iter().take(10).map(|id| format!("#{id}")).collect();
            let more = if vanished.len() > 10 { format!(" и ещё {}", vanished.len() - 10) } else { String::new() };
            notes.push(format!(
                "Записано, но уже нет на диске: {} ({}{more}). Похоже, их удаляет антивирус — посмотрите его журнал или карантин («Безопасность Windows» → «Журнал защиты»), добавьте папку выгрузки в исключения и выгрузите снова.",
                vanished.len(),
                ids.join(", ")
            ));
            complete = false;
        }
        let done = self.done.load(SeqCst);
        let target = self.target.load(SeqCst);
        let how_many = if target > 0 { format!("{done} из {target}") } else { done.to_string() };
        if stopped {
            notes.insert(0, format!("Остановлено — что успели, сохранено (обработано {how_many}). Запустите выгрузку снова: готовое пропустится, она продолжится с этого места."));
        }
        let fatal = locked(&self.fatal).clone();
        if !fatal.is_empty() {
            notes.insert(0, format!("{fatal} Что успели, сохранено (обработано {how_many}); исправьте причину и запустите выгрузку снова — она продолжится с этого места."));
        }
        if !git_note.is_empty() {
            notes.push(format!(
                "Папка — в git-репозитории, а {git_note}: допишите строку *.tmp в этот .gitignore сами, иначе коммит может поймать недописанный файл."
            ));
        }
        let files_note = self.files_note();
        let index_error = files::write_indexes(dir, &locked(&self.touched), &|| self.cancelled(), self.progress);
        let elapsed = self.clock.elapsed();
        ExportResult {
            found: done,
            created: self.created.load(SeqCst),
            updated: self.updated.load(SeqCst),
            unchanged: self.unchanged.load(SeqCst),
            failed: self.failed.load(SeqCst),
            first_error: locked(&self.first_error).clone(),
            error: {
                let mut all = notes;
                all.push(files_note);
                all.push(index_error);
                if !stopped && elapsed >= Duration::from_secs(60) {
                    all.push(format!("Заняло {}.", human_span(elapsed)));
                }
                all
            }
            .into_iter()
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
            complete,
        }
    }
}

impl<'a> Job<'a> {
    /// Начало любой выгрузки в папку: остановили до начала — не идём; оборванные записи убраны; папка tickets есть; `*.tmp` — в
    /// .gitignore до первого файла, не вышло — строка для итога. Ошибка — готовый итог «не начата» (`InTurnAsync`, `Job.Begin`).
    #[allow(clippy::too_many_arguments)]
    fn begin(
        client: &'a Client,
        dir: &'a Path,
        ticket_url: &'a (dyn Fn(i32) -> String + Sync),
        context: &'a Context,
        progress: &'a (dyn Fn(String) + Sync),
        stop: &'a AtomicBool,
        options: &'a Options,
        target: usize,
    ) -> Result<(Job<'a>, String), ExportResult> {
        if stop.load(SeqCst) {
            return Err(not_started(STOPPED_BEFORE_START.into()));
        }
        let tickets = dir.join(files::TICKETS_FOLDER);
        // оборванные записи: в корне — только оглавления (корень — хранилище пользователя), в tickets — и заявки
        files::sweep_index_temps(dir);
        files::sweep_temps(&tickets);
        if stop.load(SeqCst) {
            return Err(not_started(STOPPED_BEFORE_START.into())); // остановили, пока убирали, — дальше не идём
        }
        if let Err(e) = fs::create_dir_all(&tickets) {
            return Err(not_started(format!("Не удалось записать в папку {}:\n{e}", dir.display())));
        }
        // *.tmp — в .gitignore до первого файла; не вышло — в итоге выгрузки
        let git_note = files::ignore_temps_in_git(dir);
        let job = Job {
            client,
            root: dir,
            tickets,
            ticket_url,
            context,
            progress,
            stop,
            options,
            gate: Gate::new(MAX_PARALLEL),
            abort: AtomicBool::new(false),
            target: AtomicUsize::new(target),
            clock: Instant::now(),
            touched: Mutex::new(Vec::new()),
            written: Mutex::new(Vec::new()),
            vanished: Mutex::new(Vec::new()),
            swept: Mutex::new(HashSet::new()),
            created: AtomicUsize::new(0),
            updated: AtomicUsize::new(0),
            unchanged: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            transient: AtomicUsize::new(0),
            server_errors: AtomicUsize::new(0),
            refused: AtomicUsize::new(0),
            write_fails: AtomicUsize::new(0),
            tripped: AtomicBool::new(false),
            first_error: Mutex::new(String::new()),
            fatal: Mutex::new(String::new()),
            files: Mutex::new(FilesSeen::default()),
            last_report: AtomicU64::new(0),
            rate: Mutex::new(RateMeter::default()),
        };
        Ok((job, git_note))
    }

    /// Страница (или весь выбранный список): по четыре заявки разом, каждая — на месте из четырёх. false — остановили или
    /// прервал выключатель, часть заявок осталась необработанной (`ExportBatchAsync`).
    fn export_batch(&self, rows: &[Found], existing: &HashMap<i32, Known>) -> bool {
        // оборванные записи в папках месяцев — раз за выгрузку
        for shard in rows.iter().map(|f| files::shard_of(f.created.as_ref())) {
            if locked(&self.swept).insert(files::fold(&shard)) {
                files::sweep_temps(&self.tickets.join(shard));
            }
        }
        let (next, cancelled) = (AtomicUsize::new(0), AtomicBool::new(false));
        std::thread::scope(|scope| {
            for _ in 0..MAX_PARALLEL.min(rows.len()) {
                scope.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, SeqCst);
                        if i >= rows.len() {
                            return;
                        }
                        // не обработанная остановкой заявка не считается — как отменённая у C#
                        if self.cancelled() || !self.gate.enter(|| self.cancelled()) {
                            cancelled.store(true, SeqCst);
                            return;
                        }
                        let handled = self.export_core(&rows[i], existing);
                        self.gate.leave();
                        if !handled {
                            cancelled.store(true, SeqCst);
                            return;
                        }
                        self.done.fetch_add(1, SeqCst);
                        self.report(false);
                    }
                });
            }
        });
        !cancelled.load(SeqCst)
    }

    /// Страница выгрузки по отбору: что из неё уже лежит, смотрим по номерам страницы — во всех папках месяцев (дубль заявки в
    /// чужом месяце находится и убирается). ponytail: папки читаются заново на каждой странице — на сотне тысяч файлов это
    /// десятки миллисекунд против минуты на сами карточки; одним проходом по всем именам нужен разбор номера из files.rs.
    fn export_listed(&self, rows: &[Found]) -> bool {
        let wanted: HashSet<i32> = rows.iter().map(|f| f.id).collect();
        let shards: Vec<String> = fs::read_dir(&self.tickets)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        self.export_batch(rows, &files::scan_names(&self.tickets, &wanted, &shards))
    }

    /// Найденное вторым проходом — выгрузить и прибавить к ходу (его цель — сколько всего, если известно) (`ExportFoundAsync`).
    fn export_recovered(&self, rows: &[Found]) -> bool {
        if self.target.load(SeqCst) > 0 {
            self.target.fetch_max(self.done.load(SeqCst) + rows.len(), SeqCst);
        }
        if !self.export_listed(rows) {
            return false;
        }
        self.check_written(false);
        self.report(true);
        true
    }

    /// Страница списка под одним из четырёх мест; паузы между повторами место не держат. None — остановили (`Fetch`).
    fn fetch_list(&self, query: &TaskQuery, page: i32, counted: bool) -> Option<Result<SearchPage, String>> {
        self.retrying_noting(
            || {
                if !self.gate.enter(|| self.cancelled()) {
                    return Err(String::new()); // остановили — `retrying` это видит и отвечает None
                }
                let page = self.client.get_task_list(query, page, counted, self.options.list_timeout);
                self.gate.leave();
                page
            },
            |error| (self.progress)(format!("Список: {} — повторяю…", brief(error))),
        )
    }

    /// Список страницами и сразу файлы по каждой странице, затем второй проход (`RunInTurnAsync`). Что вышло — числа и флаги
    /// для `found_notes`.
    fn walk_found(&self, query: &TaskQuery, limit: i32) -> FoundRun {
        let everything = limit == 0;
        let mut list_query = if everything { TaskQuery { sort: STABLE_SORT.into(), ..query.clone() } } else { query.clone() };
        let (outside_where, outside_which) = query.outside_words();
        let mut r = FoundRun { limit, outside_where: outside_where.into(), outside_which: outside_which.into(), ..FoundRun::default() };
        let mut seen: HashSet<i32> = HashSet::new();
        let (mut taken, mut sort_fallback, mut counting) = (0, false, false);
        let mut first_repeats: HashSet<i32> = HashSet::new();
        let mut last_created = None;
        std::thread::scope(|scope| {
            // следующая страница, запрошенная заранее: пока выгружается эта, сервер готовит следующую
            let fetch = |page: i32, query: &TaskQuery, counting: bool| {
                let query = query.clone();
                scope.spawn(move || self.fetch_list(&query, page, page == 1 || counting))
            };
            let mut ahead: Option<std::thread::ScopedJoinHandle<'_, Option<Result<SearchPage, String>>>> = None;
            (self.progress)("Смотрю, что уже выгружено…".into());
            (self.progress)("Читаю список заявок…".into());
            let mut page = 0;
            loop {
                page += 1;
                let got = match ahead.take() {
                    Some(handle) => handle.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
                    None => self.fetch_list(&list_query, page, page == 1 || counting),
                };
                let Some(got) = got else {
                    r.cancelled = true;
                    break;
                };
                let listed = match got {
                    Ok(listed) => listed,
                    Err(error) => {
                        let code = http_code(&error);
                        // порядок по созданию сервер, возможно, не принял (отказ бывает и 4xx, и 500 на незнакомое поле): с первой
                        // страницы — к порядку по изменению. Логин и доступ тут ни при чём, а срок и «слишком часто» (408, 429)
                        // повторы уже исчерпали — это не сортировка
                        if page == 1
                            && everything
                            && !sort_fallback
                            && code.is_some_and(|c| c >= 400 && !matches!(c, 401 | 403 | 408 | 429))
                        {
                            sort_fallback = true;
                            list_query = query.clone();
                            page = 0;
                            continue;
                        }
                        // count=false живой сервер принять должен, но не проверено — отказ (400) на странице после первой не
                        // обрывает выгрузку: та же страница ещё раз со счётом, и дальше так же
                        if page > 1 && !counting && code == Some(400) {
                            counting = true;
                            page -= 1;
                            continue;
                        }
                        r.list_error = error;
                        break;
                    }
                };
                r.fallback_worked |= sort_fallback; // тот же запрос без сортировки по созданию прошёл — дело было в ней
                r.counting_worked |= counting; // а со счётом страница пришла — дело было в count=false
                r.saw_has_next |= listed.has_next.is_some();
                r.total = r.total.max(listed.total);
                let (mut batch, mut fresh, mut page_repeats) = (Vec::new(), 0, Vec::new());
                for f in &listed.found {
                    if !seen.insert(f.id) {
                        page_repeats.push(f.id); // список сдвинулся, порядок не постоянный или заявка в нём дважды
                        continue;
                    }
                    fresh += 1;
                    if list_query.sort == STABLE_SORT
                        && let Some(created) = f.created
                    {
                        r.unsorted |= last_created.is_some_and(|before| created < before); // заказанный порядок не соблюдён
                        last_created = Some(created);
                    }
                    if list_query.past_end(f) {
                        r.reached_end = true;
                        break;
                    }
                    if list_query.outside(f) {
                        r.outside += 1;
                        continue;
                    }
                    if limit > 0 && taken + batch.len() as i32 >= limit {
                        r.hit_limit = true;
                        break;
                    }
                    batch.push(f.clone());
                }
                taken += batch.len() as i32;
                // общее число «тысяча или больше» — лишь нижняя граница: сколько всего, неизвестно (0 — ход без «из»), разве что
                // «Не больше, заявок» не выше неё
                let exact = !capped(r.total);
                let target = if exact {
                    (if everything { r.total } else { limit.min(r.total) }).max(taken)
                } else if limit > 0 && limit <= r.total {
                    limit
                } else {
                    0
                };
                self.target.store(target as usize, SeqCst);
                // ни одной новой: на второй странице или полная — сервер отдаёт ту же страницу снова, дальше не пройти; неполная
                // дальше второй — он повторил последнюю страницу за концом списка: это конец. Повторы на странице без новых — это
                // и есть такой повтор, а не стык страниц: в счёт повторов не идут
                let on_page = listed.found.len();
                let repeated_page = fresh == 0 && on_page > 0;
                r.stuck = repeated_page && (page == 2 || on_page >= LIST_PAGE_SIZE as usize);
                if !repeated_page {
                    r.repeated += page_repeats.len() as i32;
                    first_repeats.extend(page_repeats);
                }
                if limit > 0 && taken >= limit {
                    r.hit_limit = true;
                }
                // Нужна ли следующая страница, известно до выгрузки этой — запрос уходит сразу. Сказал сервер — верим. Не сказал:
                // последняя страница — неполная, когда набрано всё обещанное; но без счёта или с «тысячей» неполная страница ничего
                // не доказывает — её мог урезать сам сервер, и конец подтвердит только пустая
                let more = !(r.reached_end || r.hit_limit || repeated_page || on_page == 0)
                    && listed.has_next.unwrap_or(
                        !(exact && seen.len() as i32 >= r.total && on_page < LIST_PAGE_SIZE as usize && r.total > on_page as i32),
                    );
                if more {
                    ahead = Some(fetch(page + 1, &list_query, counting));
                }
                if !batch.is_empty() && !self.export_listed(&batch) {
                    r.cancelled = true;
                    break;
                }
                self.check_written(false);
                self.report(true);
                if !more {
                    break;
                }
            }

            // Второй проход — весь список в другом порядке, выгружаются только не встреченные в первом: номера страниц теряют
            // заявки, когда порядок у сервера не постоянный или список за время выгрузки укоротился. Нужен, когда первый
            // недосчитался по точному счёту или были повторы; а при «тысяче или больше» — всегда. С пределом — так же, когда
            // весь список в него умещается; тогда порядок по изменению от давних, а найденное выгружается в конце — самые свежие
            r.first_seen = seen.len() as i32;
            let whole = if everything { list_query.sort == STABLE_SORT } else { !capped(r.total) && r.total <= limit };
            // (предел первый проход уже набрал — второму добавить нечего)
            if whole
                && !r.hit_limit
                && !r.cancelled
                && r.list_error.is_empty()
                && !r.stuck
                && !r.reached_end
                && r.outside == 0
                && locked(&self.fatal).is_empty()
                && (capped(r.total) || r.repeated > 0 || (seen.len() as i32) < r.total)
            {
                r.second_pass = true;
                list_query = TaskQuery { sort: (if everything { DEFAULT_SORT } else { CHANGED_ASC_SORT }).into(), ..query.clone() };
                let mut found: Vec<Found> = Vec::new(); // с пределом: найденное — в конце, свежие
                let mut pass_seen: HashSet<i32> = HashSet::new();
                let mut page = 0;
                loop {
                    page += 1;
                    let tail =
                        if everything { format!("дочитано {}", r.recovered) } else { format!("найдено {}", found.len()) };
                    (self.progress)(format!("Сверяю список вторым проходом: страница {page}, {tail}…"));
                    let got = match ahead.take() {
                        Some(handle) => handle.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
                        None => self.fetch_list(&list_query, page, page == 1 || counting),
                    };
                    let Some(got) = got else {
                        r.cancelled = true;
                        break;
                    };
                    let listed = match got {
                        Ok(listed) => listed,
                        Err(error) => {
                            // count=false не принят (400) — та же страница со счётом, как в первом проходе
                            if page > 1 && !counting && http_code(&error) == Some(400) {
                                counting = true;
                                page -= 1;
                                continue;
                            }
                            r.second_error = error;
                            break;
                        }
                    };
                    if listed.found.is_empty() {
                        break;
                    }
                    // ни одной новой для этого прохода: неполная — сервер повторил последнюю страницу (за концом), это конец;
                    // полная — он не понимает page, дальше не пройти
                    let pass_repeats: Vec<i32> = listed.found.iter().filter(|f| !pass_seen.insert(f.id)).map(|f| f.id).collect();
                    if pass_repeats.len() == listed.found.len() {
                        r.second_stuck = page == 2 || listed.found.len() >= LIST_PAGE_SIZE as usize;
                        break;
                    }
                    // повтор на стыке страниц — сдвиг списка; одна и та же заявка повторилась в обоих проходах — она в списке дважды
                    if r.twice.is_none() {
                        r.twice = pass_repeats.iter().find(|id| first_repeats.contains(id)).copied();
                    }
                    let mut batch = Vec::new();
                    for f in &listed.found {
                        if !seen.insert(f.id) {
                            continue;
                        }
                        if query.outside(f) {
                            r.outside += 1; // сервер, похоже, не применил условие по дате — как в первом проходе
                        } else {
                            batch.push(f.clone());
                        }
                    }
                    if !everything {
                        found.extend(batch.iter().cloned());
                        // список не длиннее предела (так насчитал сервер): прочитано на страницу больше — счёт неверен, а найденное
                        // сверх предела всё равно не выгрузить; дальше не читаем
                        // у C# сумма у самого i32::MAX переполняется; здесь — нет
                        if pass_seen.len() as i32 > limit.saturating_add(LIST_PAGE_SIZE) && listed.has_next != Some(false) {
                            r.second_long = pass_seen.len() as i32;
                            break;
                        }
                    }
                    if listed.has_next != Some(false) {
                        ahead = Some(fetch(page + 1, &list_query, counting)); // следующую — пока выгружается эта
                    }
                    if everything && !batch.is_empty() {
                        r.recovered += batch.len() as i32;
                        if !self.export_recovered(&batch) {
                            r.cancelled = true;
                            break;
                        }
                    }
                    if listed.has_next == Some(false) {
                        break;
                    }
                }
                if !r.cancelled && !found.is_empty() {
                    found.sort_by(|a, b| b.changed.cmp(&a.changed).then(b.id.cmp(&a.id)));
                    let room = (limit - taken).max(0) as usize;
                    r.overflow = found.len().saturating_sub(room) as i32;
                    found.truncate(room);
                    if !found.is_empty() {
                        r.recovered += found.len() as i32;
                        if !self.export_recovered(&found) {
                            r.cancelled = true;
                        }
                    }
                }
                self.report(true); // ход — снова «выгружено N», а не «сверяю»
            }
            // страница, запрошенная вперёд, не понадобилась (остановили, прервали, список кончился) — дожидаемся её, не бросая
            if let Some(handle) = ahead {
                let _ = handle.join();
            }
        });
        r.seen = seen.len() as i32;
        r.twice = r.twice.filter(|_| r.second_pass);
        r.fatal = locked(&self.fatal).clone();
        r
    }
}

/// Файлы по готовому списку заявок (выбранные в окне): по четыре разом; не менявшиеся с прошлой выгрузки пропускаются без
/// запросов. Остановили, прервалась или оборвалась сеть — что записано, остаётся, повторный запуск продолжит (готовое
/// пропускается). `stop` — «Остановить»; в очередь писать (`Writer`) выгрузка не встаёт: окно запускает одну за раз.
#[allow(clippy::too_many_arguments)]
pub fn export_rows(
    client: &Client,
    rows: &[Found],
    dir: &Path,
    ticket_url: &(dyn Fn(i32) -> String + Sync),
    context: &Context,
    progress: &(dyn Fn(String) + Sync),
    stop: &AtomicBool,
    options: &Options,
) -> ExportResult {
    let (job, git_note) = match Job::begin(client, dir, ticket_url, context, progress, stop, options, rows.len()) {
        Ok(begun) => begun,
        Err(result) => return result,
    };
    let shards: Vec<String> = rows.iter().map(|f| files::shard_of(f.created.as_ref())).collect();
    let wanted: HashSet<i32> = rows.iter().map(|f| f.id).collect();
    let finished = job.export_batch(rows, &files::scan_names(&job.tickets, &wanted, &shards));
    if finished {
        job.settle(); // антивирус удаляет не сразу
    }
    let complete = finished && locked(&job.fatal).is_empty();
    job.conclude(dir, stop.load(SeqCst) && !finished, complete, &git_note, Vec::new())
}

/// Страница списка при выгрузке по отбору — та, что спрашивает `get_task_list` (`ListPageSize = ExecutorPageSize`).
pub const LIST_PAGE_SIZE: i32 = crate::intraservice::EXECUTOR_PAGE_SIZE;

/// Чем кончилась выгрузка по отбору (`FoundRun`): числа и флаги хода, из них — примечания к итогу и «закончена ли».
#[derive(Clone, Debug, Default)]
pub struct FoundRun {
    pub total: i32,
    pub seen: i32,
    pub first_seen: i32,
    pub list_error: String,
    pub stuck: bool,
    pub second_pass: bool,
    pub second_error: String,
    pub second_stuck: bool,
    pub cancelled: bool,
    pub twice: Option<i32>,
    pub recovered: i32,
    pub repeated: i32,
    pub overflow: i32,
    pub second_long: i32,
    pub limit: i32,
    pub outside: i32,
    pub outside_where: String,
    pub outside_which: String,
    pub counting_worked: bool,
    pub fallback_worked: bool,
    pub unsorted: bool,
    pub saw_has_next: bool,
    pub reached_end: bool,
    pub hit_limit: bool,
    pub fatal: String,
}

/// Примечания к итогу выгрузки по отбору и закончена ли она (`FoundNotes`).
pub fn found_notes(r: &FoundRun) -> (Vec<String>, bool) {
    let mut notes: Vec<String> = Vec::new();
    let mut short_list = false;
    let second_clean = r.second_pass && r.second_error.is_empty() && !r.second_stuck && !r.cancelled && !capped(r.total);
    if !r.list_error.is_empty() {
        notes.push(format!("Список пришёл не целиком: {}", r.list_error));
    } else if r.stuck {
        notes.push(format!("Сервер отдаёт одну и ту же страницу списка — дальше не пройти (прочитано {})", r.seen));
    } else if second_clean && r.seen < r.total && r.total - r.seen <= (r.total / 100).max(1) {
        let who = match r.twice {
            Some(id) => format!("заявка #{id} пришла в списке дважды (в обоих проходах), похоже, сервер её и посчитал дважды. "),
            None => "похоже, сервер считает какую-то заявку дважды или её удалили либо скрыли, пока шла выгрузка. ".into(),
        };
        notes.push(format!(
            "Сервер насчитал {}, а разных заявок в списке {}: недостающих второй проход, в другом порядке, не нашёл — {who}Выгружено всё, что есть в списке.",
            r.total, r.seen
        ));
    } else if second_clean && r.seen < r.total {
        notes.push(format!(
            "Список закончился раньше, чем обещал сервер ({} из {}), и второй проход, в другом порядке, недостающих не нашёл — выгружено то, что пришло. Пришлите этот итог.",
            r.seen, r.total
        ));
        short_list = true;
    } else if !r.cancelled && r.fatal.is_empty() && !r.reached_end && !r.hit_limit && r.seen < r.total {
        notes.push(format!(
            "Список закончился раньше, чем обещал сервер ({} из {}) — выгружено то, что пришло. Повторите выгрузку позже: недостающее подтянется",
            r.seen, r.total
        ));
    }
    if r.recovered > 0 {
        let repeats = if r.repeated > 0 {
            format!(" (сервер повторял заявки на стыке страниц: {})", r.repeated)
        } else {
            String::new()
        };
        notes.push(format!(
            "Второй проход по списку нашёл заявок, которых не было в первом: {} — пропущенные на стыке страниц или появившиеся за время выгрузки{repeats}; они выгружены вместе с остальными.",
            r.recovered
        ));
    }
    if r.overflow > 0 && r.second_long == 0 {
        // не ошибка: просили не больше стольких — столько и выгружено
        notes.push(format!(
            "В списке оказалось больше заявок, чем «Не больше, заявок» ({}): из найденных вторым проходом выгружены самые свежие, ещё {} — за пределом. Нужны все — поднимите предел и выгрузите снова.",
            r.limit, r.overflow
        ));
    }
    if r.second_long > 0 {
        notes.push(format!(
            "Второй проход прочёл {} заявок — больше, чем насчитал сервер ({}) и вмещает предел ({}): список сильно изменился за время выгрузки, дальше он не читан, и недавно изменённые могли не попасть — повторите выгрузку.",
            r.second_long, r.total, r.limit
        ));
    }
    if !r.second_error.is_empty() {
        notes.push(format!(
            "Второй проход по списку не удался: {} — пропущенное первым проходом могло не попасть.",
            headline(&r.second_error)
        ));
    }
    if r.second_stuck {
        notes.push("Второй проход по списку: сервер отдаёт одну и ту же страницу — дальше не пройти, пропущенное первым проходом могло не попасть.".into());
    }
    if r.outside > 0 {
        notes.push(format!(
            "Сервер вернул заявки {} ({}) — они пропущены: {} он, похоже, не применил",
            r.outside_where, r.outside, r.outside_which
        ));
    }
    if r.counting_worked {
        notes.push("Сервер не принял список без счёта (count=false) — страницы списка шли со счётом.".into());
    }
    if r.fallback_worked {
        notes.push("Сервер не принял сортировку по дате создания — список читался по дате изменения. Заявки, тронутые за время выгрузки, могли не попасть: запустите выгрузку ещё раз, она дозагрузит".into());
    } else if r.unsorted {
        notes.push("Сервер отдал список не по дате создания, как заказано. Заявки, тронутые за время выгрузки, могли не попасть: запустите выгрузку ещё раз, она дозагрузит".into());
    }
    // счёт упёрся в потолок, ни разу не сказано, есть ли следующая страница, и список кончился, не перевалив за потолок: похоже,
    // сервер обрезал сам список — «всё» выгружено не всё. Ровно тысяча найденных выглядит так же — потому «похоже», и выгрузка не
    // названа законченной
    let cut = capped(r.total)
        && !r.saw_has_next
        && r.first_seen == r.total
        && !r.reached_end
        && !r.hit_limit
        && !r.stuck
        && r.list_error.is_empty()
        && !r.cancelled
        && r.fatal.is_empty();
    if cut {
        let more = if r.recovered > 0 {
            format!(" (второй проход, в другом порядке, дочитал ещё {})", r.recovered)
        } else {
            String::new()
        };
        notes.push(format!(
            "Список кончился ровно на потолке счёта сервера ({0}): если найдено больше, сервер, похоже, отдаёт только первые {0} заявок списка{more} — часть могла не попасть. Пришлите этот итог.",
            r.total
        ));
    }
    let complete = !r.cancelled
        && r.fatal.is_empty()
        && r.list_error.is_empty()
        && !r.stuck
        && !cut
        && r.second_error.is_empty()
        && !r.second_stuck
        && !short_list
        && r.second_long == 0;
    (notes, complete)
}

/// Что не так с числом заявок (`InvalidLimit`); None — всё в порядке. 0 — без ограничения, все найденные.
pub fn invalid_limit(limit: i32) -> Option<&'static str> {
    (limit < 0).then_some("«Не больше, заявок» — число заявок или 0, если нужны все")
}

/// Выгрузка по отбору (`KnowledgeExport.RunAsync`): список страницами и сразу файлы по каждой странице. `limit` — не больше
/// стольких заявок (свежие по изменению первыми, строки вне периода не в счёт); 0 — все, тогда список идёт по дате создания от
/// старых к новым: порядок не «едет», пока выгрузка идёт часами. Остановили, прервалась или оборвалась сеть — что записано,
/// остаётся, повторный запуск продолжит. Условия по дате сервер, возможно, не применит — чтение обрывается за границей периода,
/// строки вне него не берутся, о чём сказано в итоге.
#[allow(clippy::too_many_arguments)]
pub fn export_found(
    client: &Client,
    query: &TaskQuery,
    limit: i32,
    dir: &Path,
    ticket_url: &(dyn Fn(i32) -> String + Sync),
    context: &Context,
    progress: &(dyn Fn(String) + Sync),
    stop: &AtomicBool,
    options: &Options,
) -> ExportResult {
    if let Some(why) = invalid_limit(limit) {
        return not_started(why.into()); // неверный ввод — сразу
    }
    let (job, git_note) = match Job::begin(client, dir, ticket_url, context, progress, stop, options, 0) {
        Ok(begun) => begun,
        Err(result) => return result,
    };
    let run = job.walk_found(query, limit);
    let stopped = run.cancelled && stop.load(SeqCst); // остановили не на самом последнем шаге, когда всё уже готово
    let (notes, complete) = found_notes(&run);
    if job.done.load(SeqCst) == 0 && !run.list_error.is_empty() {
        return not_started(run.list_error); // список не пришёл вовсе — ответ сервера как есть
    }
    if !run.cancelled {
        job.settle(); // записанное последним — проверить не сразу: антивирус удаляет через мгновение
    }
    job.conclude(dir, stopped, complete, &git_note, notes)
}

/// Итог выгрузки для строки под кнопками: «Готово» или «Не закончено» (остановили, прервалась — повтор продолжит)
/// (`SearchViewModel.Summary`).
pub fn summary(r: &ExportResult, what: &str) -> String {
    let num = |n: usize| crate::search::thousands(n as i32);
    if r.found == 0 {
        return if r.error.is_empty() {
            "Под отбор не попало ни одной заявки.".to_string()
        } else {
            r.error.clone()
        };
    }
    let head = if r.complete {
        format!("Готово. {what} — {}", num(r.found))
    } else {
        format!("Не закончено. {what}, обработано — {}", num(r.found))
    };
    let mut text = format!("{head}: новых файлов {}, обновлено {}, без изменений {}.", num(r.created), num(r.updated), num(r.unchanged));
    // без догадок о причине и средстве: что случилось, говорит сама первая ошибка
    if r.failed > 0 {
        text.push_str(&format!(
            "\nНе выгружены: {} — повторная выгрузка попробует их снова. Первая ошибка — {}",
            num(r.failed),
            r.first_error
        ));
    }
    if !r.error.is_empty() {
        text.push_str(&format!("\n{}", r.error));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::ClosedNames;

    /// Выгрузка без сети и диска: проверяются правила самого `Job` (выключатель, итог, оглавления тронутых месяцев).
    fn job(target: usize) -> Job<'static> {
        let ticket_url: &'static (dyn Fn(i32) -> String + Sync) = Box::leak(Box::new(|id: i32| format!("https://hd/Task/View/{id}")));
        let progress: &'static (dyn Fn(String) + Sync) = Box::leak(Box::new(|_: String| {}));
        Job {
            client: Box::leak(Box::new(Client::new("http://127.0.0.1:9/", "u", "p", Duration::from_secs(1)))),
            root: Path::new("/нет-такой-папки"),
            tickets: PathBuf::from("/нет-такой-папки/tickets"),
            ticket_url,
            context: Box::leak(Box::new(Context { me: None, closed: ClosedNames::default() })),
            progress,
            stop: Box::leak(Box::new(AtomicBool::new(false))),
            options: Box::leak(Box::new(Options::default())),
            gate: Gate::new(MAX_PARALLEL),
            abort: AtomicBool::new(false),
            target: AtomicUsize::new(target),
            clock: Instant::now(),
            touched: Mutex::new(Vec::new()),
            written: Mutex::new(Vec::new()),
            vanished: Mutex::new(Vec::new()),
            swept: Mutex::new(HashSet::new()),
            created: AtomicUsize::new(0),
            updated: AtomicUsize::new(0),
            unchanged: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            transient: AtomicUsize::new(0),
            server_errors: AtomicUsize::new(0),
            refused: AtomicUsize::new(0),
            write_fails: AtomicUsize::new(0),
            tripped: AtomicBool::new(false),
            first_error: Mutex::new(String::new()),
            fatal: Mutex::new(String::new()),
            files: Mutex::new(FilesSeen::default()),
            last_report: AtomicU64::new(0),
            rate: Mutex::new(RateMeter::default()),
        }
    }

    const REFUSED: &str = "заявка не найдена (HTTP 404)";

    fn refuse(job: &Job, times: usize) {
        for id in 1..=times as i32 {
            job.fail(id, REFUSED);
        }
    }

    #[test]
    fn a_month_is_touched_once_and_an_empty_name_is_not_a_month() {
        let j = job(3);
        for shard in ["2026-09", "2026-10", "2026-09", "", "2026-09", "2026-10"] {
            j.touch(shard);
        }
        assert_eq!(*locked(&j.touched), vec!["2026-09".to_string(), "2026-10".to_string()]);
    }

    #[test]
    fn twenty_five_refusals_with_nothing_landed_trip_the_breaker() {
        let j = job(100);
        refuse(&j, 24);
        assert!(!j.tripped.load(SeqCst), "двадцать четыре отказа — ещё не выключатель");
        refuse(&j, 1);
        assert!(j.tripped.load(SeqCst), "двадцать пятый — выключатель");
        assert!(locked(&j.fatal).starts_with("Выгрузка прервана: ни одна из 25 заявок не прочиталась"), "{}", locked(&j.fatal));
    }

    #[test]
    fn a_single_landed_ticket_of_any_kind_keeps_the_breaker_off_however_many_refusals_follow() {
        // «ни одной удачной»: хоть одна новая, хоть одна обновлённая, хоть одна без изменений — и вместе тоже
        for (created, updated, unchanged) in [(1, 0, 0), (0, 1, 0), (0, 0, 1), (1, 1, 0), (1, 0, 1), (0, 1, 1), (1, 1, 1)] {
            let j = job(200);
            j.created.store(created, SeqCst);
            j.updated.store(updated, SeqCst);
            j.unchanged.store(unchanged, SeqCst);
            refuse(&j, 100);
            assert!(
                !j.tripped.load(SeqCst),
                "легло: новых {created}, обновлено {updated}, без изменений {unchanged} — выключатель не нужен"
            );
        }
    }

    #[test]
    fn attachment_counters_add_up_over_tickets_and_the_note_names_each_kind() {
        let j = job(2);
        j.add_files(FilesSeen { present: 2, skipped: 1, failed: 1, first_error: "A".into() });
        j.add_files(FilesSeen { present: 1, skipped: 2, failed: 2, first_error: "B".into() });
        assert_eq!(
            j.files_note(),
            "Вложения: уже были 3, не скачано (настройка «Вложения в базу знаний»): 3; не скачались 3 — повторная выгрузка попробует их снова: A."
        );
        let only_failed = job(1);
        only_failed.add_files(FilesSeen { failed: 2, first_error: "A".into(), ..FilesSeen::default() });
        assert_eq!(only_failed.files_note(), "Вложения: не скачались 2 — повторная выгрузка попробует их снова: A.");
        let only_present = job(1);
        only_present.add_files(FilesSeen { present: 4, ..FilesSeen::default() });
        assert_eq!(only_present.files_note(), "Вложения: уже были 4.");
        assert_eq!(job(1).files_note(), "", "с вложениями ничего не было — строки нет");
    }

    fn vanished(j: &Job, n: i32) {
        let mut written = locked(&j.written);
        for id in 1..=n {
            written.push((id, PathBuf::from(format!("/нет-такой-папки/{id}.md")), 0));
        }
    }

    #[test]
    fn ten_vanished_files_are_named_and_the_eleventh_is_counted() {
        let ten = job(10);
        vanished(&ten, 10);
        let r = ten.conclude(Path::new("/нет-такой-папки"), false, true, "", Vec::new());
        assert!(r.error.starts_with("Записано, но уже нет на диске: 10 (#1, #2, #3, #4, #5, #6, #7, #8, #9, #10). "), "{}", r.error);
        let eleven = job(11);
        vanished(&eleven, 11);
        let r = eleven.conclude(Path::new("/нет-такой-папки"), false, true, "", Vec::new());
        assert!(
            r.error.starts_with("Записано, но уже нет на диске: 11 (#1, #2, #3, #4, #5, #6, #7, #8, #9, #10 и ещё 1). "),
            "{}",
            r.error
        );
        assert!(!r.complete);
    }

    #[test]
    fn the_time_is_told_for_a_run_of_a_minute_or_more_but_not_for_a_stopped_one() {
        let long_ago = |secs| Instant::now().checked_sub(Duration::from_secs(secs)).expect("часы идут дольше");
        let dir = Path::new("/нет-такой-папки");
        let mut j = job(1);
        j.clock = long_ago(61);
        assert_eq!(j.conclude(dir, false, true, "", Vec::new()).error, "Заняло 1 мин.");
        let mut j = job(1);
        j.clock = long_ago(3 * 3600 + 5 * 60 + 10);
        assert_eq!(j.conclude(dir, false, true, "", Vec::new()).error, "Заняло 3 ч 05 мин.");
        let mut j = job(1);
        j.clock = long_ago(61);
        assert!(!j.conclude(dir, true, false, "", Vec::new()).error.contains("Заняло"), "остановленной — не говорим");
        let mut j = job(1);
        j.clock = long_ago(30);
        assert_eq!(j.conclude(dir, false, true, "", Vec::new()).error, "", "полминуты — не срок");
    }

    #[test]
    fn a_stop_names_how_many_were_handled_with_the_total_when_it_is_known() {
        let dir = Path::new("/нет-такой-папки");
        let j = job(8);
        j.done.store(5, SeqCst);
        assert!(
            j.conclude(dir, true, false, "", Vec::new()).error.starts_with("Остановлено — что успели, сохранено (обработано 5 из 8). ")
        );
        let j = job(0);
        j.done.store(5, SeqCst);
        assert!(j.conclude(dir, true, false, "", Vec::new()).error.starts_with("Остановлено — что успели, сохранено (обработано 5). "));
    }

    #[test]
    fn the_gate_lets_waiters_in_by_arrival() {
        // ворота на одно место заняты; в очередь первым встал B, потом C — место освободилось: входит B (по приходу: страница
        // списка не ждёт дольше карточек, вставших после неё)
        let gate = Gate::new(1);
        assert!(gate.enter(|| false));
        let order = Mutex::new(Vec::new());
        let queued = |n: usize| {
            let until = Instant::now() + Duration::from_secs(5);
            while locked(&gate.state).queue.len() < n && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let tickets = std::thread::scope(|s| {
            for (name, ahead) in [("B", 1), ("C", 2)] {
                let (gate, order) = (&gate, &order);
                s.spawn(move || {
                    assert!(gate.enter(|| false));
                    locked(order).push(name);
                    gate.leave();
                });
                queued(ahead);
            }
            let tickets = Vec::from(locked(&gate.state).queue.clone());
            gate.leave(); // до проверок: упавшая проверка не должна оставить B и C ждать вечно
            tickets
        });
        assert_eq!(tickets, vec![1, 2], "оба в очереди, у каждого свой номер, по приходу");
        assert_eq!(*locked(&order), vec!["B", "C"]);
    }
}
