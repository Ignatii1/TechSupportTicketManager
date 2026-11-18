//! Файлы базы знаний на диске (`KnowledgeExport.Files.cs`, `.Attachments.cs`): где лежит заявка, как называется, какой файл
//! наш, запись через временный файл, оглавления, вложения, что уже лежат в базе (ссылкой; не качаются). Длины и обрезка — в
//! единицах UTF-16, как у C#: один и тот же путь и имя у обоих приложений, иначе файл переписывался бы каждый раз.

use super::{Attachment, AttachmentState, mask, md, one_line, visible};
use crate::intraservice::{Date, Found, TaskFile};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

pub const TICKETS_FOLDER: &str = "tickets";
pub const INDEX_FILE: &str = "_index.md";
pub const NO_DATE_FOLDER: &str = "без даты";
pub const FILES_FOLDER: &str = "files";
pub const LFS_FOLDER: &str = "files-lfs";
pub const MAX_PATH_LENGTH: usize = 250;

/// Название файла, в котором выгрузка узнаёт своё (`SourceName`), и версия формата (`FormatVersion`).
/// Свойства файла по порядку: ключ и значение без кавычек.
pub type Props = Vec<(String, String)>;

const SOURCE_NAME: &str = "intraservice";
const SEPARATOR: &str = " — ";
/// Паузы перед повторами записи, когда файл на миг держит антивирус или индексатор Windows (`WriteRetryDelays`).
const WRITE_RETRY: [Duration; 3] = [Duration::from_millis(100), Duration::from_millis(300), Duration::from_secs(1)];
/// Старше — точно не пишется сейчас: убираем (`StaleTemp`).
const STALE_TEMP: Duration = Duration::from_secs(10 * 60);

// ---------- длины в единицах UTF-16 ----------

fn u16len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// Начало строки не длиннее n единиц UTF-16; пару-заменитель не режет (`Cut`).
fn cut(s: &str, n: usize) -> String {
    let mut units = 0;
    s.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= n
        })
        .collect()
}

fn trim_end_dots(s: &str) -> &str {
    s.trim_end_matches(['.', ' '])
}

/// Знаки, запрещённые Windows, ломающие ссылки Obsidian (# ^ [ ] |) и код в ссылке (обратная кавычка), — пробелами; «%%» —
/// «%»; пробелы подряд — один; края и точки на конце убраны.
fn clean_name(s: &str) -> String {
    let spaced: String = s.chars().map(|c| if "\\/:*?\"<>|#^[]`".contains(c) || c < ' ' { ' ' } else { c }).collect();
    let mut single = String::with_capacity(spaced.len());
    let mut percents = 0;
    for c in spaced.chars() {
        if c == '%' {
            percents += 1;
            if percents == 1 {
                single.push('%');
            }
            continue;
        }
        percents = 0;
        single.push(c);
    }
    trim_end_dots(one_line(&single).trim()).to_string()
}

// ---------- имена и места ----------

/// Имя файла: «номер — название.md» (`FileName`); длинное название — до `title_max` знаков; пустое — «номер.md».
pub fn file_name(id: i32, title: &str, title_max: usize) -> String {
    let mut clean = clean_name(&mask(&one_line(title)));
    if u16len(&clean) > title_max {
        clean = trim_end_dots(&cut(&clean, title_max)).to_string();
    }
    if clean.is_empty() { format!("{id}.md") } else { format!("{id}{SEPARATOR}{clean}.md") }
}

/// Папка заявки — месяц создания в поясе, в каком его прислал сервер; даты нет — «без даты» (`ShardOf`).
pub fn shard_of(created: Option<&Date>) -> String {
    created.map_or_else(|| NO_DATE_FOLDER.to_string(), |d| d.format("%Y-%m").to_string())
}

/// Папка месяца и имя файла заявки; имя укорачивается, чтобы путь не превысил `MAX_PATH_LENGTH` (`PlaceOf`).
pub fn place_of(tickets_dir: &Path, f: &Found) -> (String, String) {
    let shard = shard_of(f.created.as_ref());
    let fixed =
        u16len(&tickets_dir.to_string_lossy()) + 1 + u16len(&shard) + 1 + f.id.to_string().len() + SEPARATOR.chars().count() + ".md".len();
    let title_max = MAX_PATH_LENGTH.saturating_sub(fixed).min(80);
    (shard, file_name(f.id, &f.name, title_max))
}

/// Папка месяца, а не пользовательская папка со своими заметками (`IsShard`): «гггг-мм» или «без даты».
fn is_shard(name: &str) -> bool {
    let b = name.as_bytes();
    name == NO_DATE_FOLDER
        || (b.len() == 7 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-' && b[5..].iter().all(u8::is_ascii_digit))
}

/// Номер в начале: цифры, дальше — конец или « — …» (`^(\d+)( — .*)?$`).
fn leading_id(s: &str) -> Option<i32> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let rest = &s[digits..];
    (digits > 0 && (rest.is_empty() || (rest.starts_with(SEPARATOR) && !rest.contains('\n')))).then(|| s[..digits].parse().ok()).flatten()
}

/// Номер заявки в имени файла: «номер — название.md» или «номер.md» (`IdInName`).
fn id_in_name(name: &str) -> Option<i32> {
    name.strip_suffix(".md").and_then(leading_id)
}

/// Номер файла вложения по его имени на диске; временный файл закачки и чужие — нет (`Attachments.TryIdOf`).
fn attachment_id(name: &str) -> Option<i32> {
    (!name.to_lowercase().ends_with(".tmp")).then(|| leading_id(name)).flatten()
}

// ---------- что уже лежит ----------

/// Выгруженное раньше: путь файла от папки tickets и остальные файлы той же заявки (дубли) (`Known`).
#[derive(Clone, Debug, Default)]
pub(super) struct Known {
    pub rel: Option<String>,
    pub others: Vec<String>,
}

impl Known {
    fn all(&self) -> impl Iterator<Item = &String> {
        self.rel.iter().chain(self.others.iter())
    }
}

/// Что лежит в папках месяцев `shards` (и прямо в tickets): номер из `wanted` → файлы заявки, только по именам (`ScanNames`).
/// Дубль заявки в чужом месяце так не найдётся.
pub(super) fn scan_names(tickets_dir: &Path, wanted: &HashSet<i32>, shards: &[String]) -> HashMap<i32, Known> {
    let mut map: HashMap<i32, Known> = HashMap::new();
    let mut seen = HashSet::new();
    let mut folders: Vec<(String, PathBuf)> =
        shards.iter().filter(|s| seen.insert(s.to_lowercase())).map(|s| (format!("{s}/"), tickets_dir.join(s))).collect();
    folders.push((String::new(), tickets_dir.to_path_buf()));
    for (prefix, folder) in folders {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let Some(id) = id_in_name(&name).filter(|id| wanted.contains(id)) else { continue };
            let rel = format!("{prefix}{name}");
            let known = map.entry(id).or_default();
            if known.rel.is_none() {
                known.rel = Some(rel);
            } else {
                known.others.push(rel);
            }
        }
    }
    map
}

pub(super) fn same_path(a: &str, b: &str) -> bool {
    a.replace('\\', "/").to_lowercase() == b.replace('\\', "/").to_lowercase()
}

fn prop<'a>(props: &'a [(String, String)], key: &str) -> Option<&'a str> {
    props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Число из свойства: только цифры; нет или не число — 0.
fn number(props: &[(String, String)], key: &str) -> i32 {
    prop(props, key).filter(|v| v.bytes().all(|b| b.is_ascii_digit())).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Файл — наш, выгрузки: его можно переписать и удалить как лишний (`IsOurs`).
pub(super) fn is_ours(props: &Option<Props>) -> bool {
    props.as_deref().and_then(|p| prop(p, "source")) == Some(SOURCE_NAME)
}

/// Файл с прошлой выгрузки ещё годится: формат тот же и заявку с тех пор не менялали (`IsCurrent`).
pub(super) fn is_current(props: &[(String, String)], changed: &Date) -> bool {
    let format_ok = prop(props, "format")
        .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v.parse::<i32>() == Ok(super::FORMAT_VERSION));
    format_ok && prop(props, "revision") == Some(super::revision(changed).as_str())
}

/// Наши файлы заявки (путь от tickets и свойства) и не менялась ли она: наш файл один, лежит на своём месте, формат и версия
/// те же, вложения на месте (`OursOf`). Нашими считаются только файлы со `source: intraservice` — заметку пользователя и
/// испорченный файл с номером в имени не трогаем.
pub(super) fn ours_of(root: &Path, tickets_dir: &Path, existing: &HashMap<i32, Known>, f: &Found) -> (Vec<String>, bool) {
    let (shard, name) = place_of(tickets_dir, f);
    let rel = format!("{shard}/{name}");
    let ours: Vec<(String, Option<Props>)> = existing
        .get(&f.id)
        .into_iter()
        .flat_map(Known::all)
        .map(|p| (p.clone(), read_props(&tickets_dir.join(p))))
        .filter(|(_, props)| is_ours(props))
        .collect();
    let unchanged = match (ours.as_slice(), &f.changed) {
        ([(only, Some(props))], Some(changed)) => {
            same_path(only, &rel) && is_current(props, changed) && files_in_base(root, &shard, f.id, props)
        }
        _ => false,
    };
    (ours.into_iter().map(|(p, _)| p).collect(), unchanged)
}

fn file_dirs(root: &Path, shard: &str, id: i32) -> (PathBuf, PathBuf) {
    (root.join(FILES_FOLDER).join(shard).join(id.to_string()), root.join(LFS_FOLDER).join(shard).join(id.to_string()))
}

/// Путь от корня базы через «/» (`InBase`).
fn in_base(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Вложения неизменной заявки на месте: скачанные (files минус files_missing) лежат в базе. Папку не прочитать — не повод
/// перечитывать (`FilesInBase`; вложения не качаются — «недостающие» не повод перечитывать и по настройке).
fn files_in_base(root: &Path, shard: &str, id: i32, props: &[(String, String)]) -> bool {
    let saved = number(props, "files") - number(props, "files_missing");
    if saved <= 0 {
        return true;
    }
    let (plain, large) = file_dirs(root, shard, id);
    let mut count = 0;
    for dir in [plain, large].iter().filter(|d| d.is_dir()) {
        let Ok(entries) = fs::read_dir(dir) else { return true };
        count += entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && attachment_id(&e.file_name().to_string_lossy()).is_some())
            .count();
    }
    count >= saved as usize
}

// ---------- свойства файла ----------

/// Свойства из начала файла (между «---»): ключ → значение без кавычек, в порядке первого появления; не наш файл — None
/// (`ReadProps`). Строки — как у `StreamReader`: «\n», «\r», «\r\n»; не больше 60 строк свойств.
pub fn read_props(path: &Path) -> Option<Vec<(String, String)>> {
    use std::io::Read;
    // ponytail: читаются первые 256 КБ — свойства файла заявки короче в сотни раз; длиннее строки у C# прочлись бы целиком
    let mut raw = Vec::new();
    fs::File::open(path).ok()?.take(256 * 1024).read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let mut rest: &str = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    // строка — как у `StreamReader.ReadLine`: до «\n», «\r» или «\r\n»; после последнего перевода пусто — строк больше нет
    let mut next = || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
        let line = &rest[..end];
        let tail = &rest[end..];
        rest = tail.strip_prefix("\r\n").or_else(|| tail.strip_prefix(['\n', '\r'])).unwrap_or(tail);
        Some(line)
    };
    if next()? != "---" {
        return None;
    }
    let mut props: Vec<(String, String)> = Vec::new();
    for _ in 0..60 {
        let Some(line) = next() else { break };
        if line == "---" {
            return Some(props);
        }
        if let Some(colon) = line.find(':').filter(|&c| c > 0) {
            let (key, value) = (line[..colon].trim().to_string(), unquote(line[colon + 1..].trim()));
            match props.iter_mut().find(|(k, _)| *k == key) {
                Some(p) => p.1 = value,
                None => props.push((key, value)),
            }
        }
    }
    None
}

/// Значение свойства без кавычек — только когда обе на месте; внутри `\"` и `\\` (`Unquote`: сперва кавычки).
pub fn unquote(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

// ---------- вложения ----------

const SHELL_READS: [&str; 10] =
    [".lnk", ".url", ".website", ".scf", ".library-ms", ".searchconnector-ms", ".search-ms", ".theme", ".themepack", ".deskthemepack"];
/// Окончания, после которых имени вложения дописывается «_»: .tmp (его прячет *.tmp из .gitignore), заметки Obsidian (она
/// выполнила бы команды плагинов из текста вложения) и то, что читает Проводник, — `Neutralized`.
const NEUTRALIZED: [&str; 5] = [".tmp", ".md", ".markdown", ".canvas", ".base"];

/// Имя не длиннее `budget` знаков: укорачивается основа, расширение (до 10 знаков, без пробелов) цело; не влезает и
/// расширение — пусто (`Fit`).
fn fit(name: &str, budget: i64) -> String {
    if (u16len(name) as i64) <= budget {
        return name.to_string();
    }
    let chars: Vec<char> = name.chars().collect();
    let dot = chars.iter().rposition(|&c| c == '.').filter(|&d| d > 0);
    let ext: String =
        dot.map(|d| chars[d..].iter().collect::<String>()).filter(|e| u16len(e) <= 10 && !e.contains(' ')).unwrap_or_default();
    let stem_part: String = chars[..chars.len() - ext.chars().count()].iter().collect();
    let room = budget - u16len(&ext) as i64;
    let stem = if room <= 0 { String::new() } else { trim_end_dots(&cut(&stem_part, room as usize)).to_string() };
    if ext.is_empty() || !stem.is_empty() { stem + &ext } else { String::new() }
}

/// Имя файла вложения в папке заявки: «номер файла — имя» (`DiskName`). Номер впереди: имена в одной заявке повторяются, а по
/// номеру выгрузка узнаёт свой файл. От номера, имени и `max` зависит только это — файл не качается заново.
pub fn disk_name(file_id: i32, name: Option<&str>, max: usize) -> String {
    let id = file_id.to_string();
    let mut clean = clean_name(&mask(&one_line(&visible(name.unwrap_or("")))));
    let budget = max as i64 - id.len() as i64 - SEPARATOR.chars().count() as i64;
    clean = fit(&clean, budget);
    let lower = clean.to_lowercase();
    if NEUTRALIZED.iter().chain(SHELL_READS.iter()).any(|e| lower.ends_with(e)) {
        // окончание — ASCII: последний знак одна единица UTF-16
        clean = if (u16len(&clean) as i64) < budget {
            clean + "_"
        } else {
            let mut kept = clean;
            kept.pop();
            kept + "_"
        };
    }
    if clean.is_empty() { id } else { format!("{id}{SEPARATOR}{clean}") }
}

/// Картинка — Obsidian встроит её в заметку (`IsImage`).
pub(super) fn is_image(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"].iter().any(|e| lower.ends_with(e))
}

/// Размер для человека: «512 Б», «12 КБ», «3,4 МБ» (по 1024; до десяти — с десятыми, дальше — целое) (`Size`).
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = if unit == 0 || value >= 10.0 {
        // Math.Round у C# — к чётному
        format!("{}", value.round_ties_even())
    } else {
        // «0.#» у C#: десятые, половина — вверх (значение точное: делили на степень двойки)
        let tenths = (value * 10.0 + 0.5).floor() as u64;
        if tenths.is_multiple_of(10) { format!("{}", tenths / 10) } else { format!("{},{}", tenths / 10, tenths % 10) }
    };
    format!("{text} {}", UNITS[unit])
}

/// Лежащий файл вложения — под нынешним именем (лежит под другим — переименовать; занят — останется как был) и его размер;
/// пропал между списком папки и этим мигом — None: качать заново (`TryKeep`).
fn try_keep(existing: &Path, name: &str) -> Option<(PathBuf, u64)> {
    let target = existing.with_file_name(name);
    let path =
        if existing != target && !target.exists() && fs::rename(existing, &target).is_ok() { target } else { existing.to_path_buf() };
    fs::metadata(&path).ok().map(|m| (path, m.len()))
}

/// Что стало с вложениями заявки при выгрузке (счётчики для итога — `FilesNote`).
#[derive(Debug, Default)]
pub(super) struct FilesSeen {
    pub present: usize,
    pub skipped: usize,
    pub failed: usize,
    pub first_error: String,
}

/// Вложения заявки: лежащие в files/ и files-lfs/ — ссылкой (под другим именем — переименованы), остальные — «не скачано»;
/// ничего не качается. Папку не прочитать — «не скачались»: файл заявки пишем, а «не скачано» сделало бы его верным, и
/// лежащие файлы пропали бы из него навсегда (`SaveFilesAsync` при download = false).
pub(super) fn save_files(root: &Path, f: &Found, shard: &str, seen: &mut FilesSeen) -> Vec<Attachment> {
    let files: &[TaskFile] = match f.extra.as_ref().and_then(|x| x.files.as_deref()) {
        Some(files) if !files.is_empty() => files,
        _ => return Vec::new(),
    };
    let (plain, large) = file_dirs(root, shard, f.id);
    let max = MAX_PATH_LENGTH.saturating_sub(u16len(&large.to_string_lossy()) + 1);
    let mut on_disk: HashMap<i32, Vec<PathBuf>> = HashMap::new();
    for folder in [&plain, &large].into_iter().filter(|d| d.is_dir()) {
        sweep_with(folder, is_file_temp);
        let listed = fs::read_dir(folder).and_then(|entries| entries.map(|e| e.map(|e| e.path())).collect::<io::Result<Vec<_>>>());
        match listed {
            Ok(paths) => {
                for path in paths.into_iter().filter(|p| p.is_file()) {
                    if let Some(id) = attachment_id(&path.file_name().unwrap_or_default().to_string_lossy()) {
                        on_disk.entry(id).or_default().push(path);
                    }
                }
            }
            Err(e) => {
                let why = format!("папка вложений не читается: {e}");
                return files.iter().map(|file| failed(f.id, file, &why, "папка вложений не читается", seen)).collect();
            }
        }
    }
    files
        .iter()
        .map(|file| {
            let name = disk_name(file.id, file.name.as_deref(), max);
            let kept = on_disk.get(&file.id).and_then(|found| {
                let first = found.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy() == name)).or(found.first())?;
                try_keep(first, &name)
            });
            let state = match kept {
                Some((path, bytes)) => {
                    seen.present += 1;
                    AttachmentState::Saved { path: in_base(root, &path), size: bytes }
                }
                None => {
                    seen.skipped += 1;
                    AttachmentState::Skipped
                }
            };
            Attachment { file: file.clone(), state }
        })
        .collect()
}

fn failed(ticket: i32, file: &TaskFile, error: &str, note: &str, seen: &mut FilesSeen) -> Attachment {
    seen.failed += 1;
    if seen.failed == 1 {
        let title = super::attachment_title(file);
        seen.first_error = format!("#{ticket} «{title}»: {}", crate::intraservice::headline(error));
    }
    Attachment { file: file.clone(), state: AttachmentState::Failed { note: note.to_string() } }
}

/// Убрать наши файлы вложений, которых в карточке больше нет, и лишние копии того, что лежит в базе; опустевшую папку заявки —
/// тоже. Не скачавшиеся и не скачанные вложения не трогаем: их файл, может, и лежит. Карточка не прислала список — ничего
/// не трогаем (`PruneFiles`).
pub(super) fn prune_files(root: &Path, f: &Found, shard: &str, kept: &[Attachment]) {
    let Some(files) = f.extra.as_ref().and_then(|x| x.files.as_ref()) else { return };
    let listed: HashSet<i32> = files.iter().map(|x| x.id).collect();
    let saved: HashMap<i32, String> = kept
        .iter()
        .filter_map(|a| match &a.state {
            AttachmentState::Saved { path, .. } => Some((a.file.id, path.to_lowercase())),
            _ => None,
        })
        .collect();
    let (plain, large) = file_dirs(root, shard, f.id);
    for folder in [plain, large].iter().filter(|d| d.is_dir()) {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            let Some(id) = attachment_id(&path.file_name().unwrap_or_default().to_string_lossy()) else { continue };
            let extra = !listed.contains(&id) || saved.get(&id).is_some_and(|keep| *keep != in_base(root, &path).to_lowercase());
            if extra {
                let _ = fs::remove_file(&path);
            }
        }
        // не пуста — останется
        if fs::read_dir(folder).is_ok_and(|mut e| e.next().is_none()) {
            let _ = fs::remove_dir(folder);
        }
    }
}

// ---------- запись ----------

/// Файл занят на время: нарушение совместного доступа (32) или блокировки (33) в Windows, отказ в доступе. Другая ошибка (на
/// месте файла папка, диск полон, путь длинный) от повтора не пройдёт (`IsBusy`).
fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(32 | 33))
}

/// Четыре шестнадцатеричных знака — имя временного своё у каждой попытки: оставшийся от сбоя или чужой «X.md.tmp» записи не
/// мешает.
fn random4() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(N.fetch_add(1, Ordering::Relaxed));
    format!("{:04x}", h.finish() & 0xFFFF)
}

/// Сначала во временный файл («X.md.1a2b.tmp»), потом подменой: оборванная запись не оставит полфайла. Не вышло — временный
/// убираем. Занят на время (антивирус, индексатор) — повторяем после пауз: временный пишется заново, только если занят был он
/// сам, а занят файл на месте — повторяется одна подмена. UTF-8 без метки (`WriteAtomic`).
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp_of = || {
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{}.tmp", random4()));
        PathBuf::from(name)
    };
    let mut attempt = 0;
    let tmp = loop {
        let tmp = tmp_of();
        match fs::write(&tmp, text.as_bytes()) {
            Ok(()) => break tmp,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                if !is_busy(&e) || attempt >= WRITE_RETRY.len() {
                    return Err(e);
                }
            }
        }
        std::thread::sleep(WRITE_RETRY[attempt]);
        attempt += 1;
    };
    for pause in WRITE_RETRY.iter().map(Some).chain([None]) {
        // на Windows rename заменяет существующий файл (MoveFileEx с заменой)
        match (fs::rename(&tmp, path), pause) {
            (Ok(()), _) => return Ok(()),
            (Err(e), Some(pause)) if is_busy(&e) => std::thread::sleep(*pause),
            (Err(e), _) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
    unreachable!("последний круг без паузы возвращает результат")
}

// ---------- оборванные записи ----------

/// Имя после «.xxxx.tmp»: четыре шестнадцатеричных знака; что осталось до них — основа (маленькие буквы: сравнение без
/// регистра).
fn temp_base(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".tmp")?;
    let at = stem.len().checked_sub(5)?;
    let hex = stem.get(at..)?;
    (hex.starts_with('.') && hex[1..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then(|| stem[..at].to_string())
}

/// Наш временный файл записи оглавления, таблицы или памятки (`IndexTemp`).
fn is_index_temp(name: &str) -> bool {
    temp_base(name).is_some_and(|b| matches!(b.as_str(), "_index.md" | "tickets.csv" | "schema.md" | "claude.md"))
}

/// Наш временный файл записи заявки: «номер — название.md.1a2b.tmp» (`TicketTemp`). «X.md.tmp» без четырёх знаков — не наш:
/// так сохраняют и редакторы.
fn is_ticket_temp(name: &str) -> bool {
    temp_base(name).is_some_and(|b| id_in_name(&b).is_some())
}

/// Временный файл закачки вложения «номер — имя.xxxx.tmp» (`FileTemp`).
fn is_file_temp(name: &str) -> bool {
    temp_base(name).is_some_and(|b| leading_id(&b).is_some())
}

fn sweep_with(folder: &Path, ours: impl Fn(&str) -> bool) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().ends_with(".tmp") || !ours(&name) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| SystemTime::now().duration_since(t).is_ok_and(|age| age > STALE_TEMP));
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Убрать в папке временные файлы оборванных записей — оглавлений и заявок, — старше десяти минут: свежие (их, может, пишут
/// прямо сейчас) и чужие не трогаем (`SweepTemps`). Уборка — не повод срывать выгрузку: ошибок наружу нет.
pub fn sweep_temps(folder: &Path) {
    sweep_with(folder, |n| is_index_temp(n) || is_ticket_temp(n));
}

/// То же, но только оглавления: корень папки выгрузки — уже хранилище пользователя (`SweepTemps(indexOnly: true)`).
pub fn sweep_index_temps(folder: &Path) {
    sweep_with(folder, is_index_temp);
}

// ---------- .gitignore ----------

static GIT_IGNORE_LOCK: Mutex<()> = Mutex::new(());

/// Папка или одна из папок над ней — репозиторий: в ней .git, папка или файл (рабочее дерево, подмодуль).
fn in_git_repository(dir: &Path) -> bool {
    std::path::absolute(dir).is_ok_and(|d| d.ancestors().any(|a| a.join(".git").exists()))
}

/// Дописать строку в файл, если такой строки ещё нет; файла нет — создать. Пусто — сделано; иначе «путь: причина».
fn append_once(path: &Path, line: &str) -> String {
    let _lock = GIT_IGNORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let result = (|| {
        let text = if path.is_file() { fs::read_to_string(path)? } else { String::new() };
        if !text.split('\n').any(|l| l.trim() == line) {
            use std::io::Write;
            let mut file = fs::OpenOptions::new().append(true).create(true).open(path)?;
            let lead = if !text.is_empty() && !text.ends_with('\n') { "\n" } else { "" };
            file.write_all(format!("{lead}{line}\n").as_bytes())?;
        }
        Ok::<(), io::Error>(())
    })();
    match result {
        Ok(()) => String::new(),
        Err(e) => format!("{}: {}", path.display(), e.to_string().trim_end_matches(['.', ' '])),
    }
}

/// Папка выгрузки — в git-репозитории: временные файлы записи (*.tmp) — в её .gitignore, иначе коммит раз в минуту может
/// поймать недописанный файл. Пусто — сделано или не репозиторий; иначе — почему не вышло (`IgnoreTempsInGit`).
pub fn ignore_temps_in_git(dir: &Path) -> String {
    if !in_git_repository(dir) {
        return String::new();
    }
    match append_once(&dir.join(".gitignore"), "*.tmp") {
        why if !why.is_empty() => format!("не удалось дописать *.tmp в {why}"),
        _ => String::new(),
    }
}

// ---------- оглавления ----------

/// Знак, не требующий кодирования в адресе: A-Z a-z 0-9 - . _ ~ (`Uri.EscapeDataString`).
fn escape_data(s: &str) -> String {
    s.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

/// Файлы заявок месяца со свойствами — в порядке оглавления: по созданию, свежие сверху, при равном — по имени. Без свойств и
/// без номера в них — пропускается (`MonthEntries`).
fn month_entries(folder: &Path) -> io::Result<Vec<(String, Props)>> {
    let mut entries: Vec<(String, Props)> = fs::read_dir(folder)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| id_in_name(n).is_some())
        .filter_map(|n| read_props(&folder.join(&n)).filter(|p| prop(p, "id").is_some()).map(|p| (n, p)))
        .collect();
    entries.sort_by(|a, b| {
        let created = |e: &(String, Props)| prop(&e.1, "created").unwrap_or("").to_string();
        let stem = |e: &(String, Props)| e.0.strip_suffix(".md").unwrap_or(&e.0).to_string();
        created(b).cmp(&created(a)).then_with(|| stem(a).cmp(&stem(b)))
    });
    Ok(entries)
}

/// Оглавление месяца: заявки по созданию, свежие сверху. Месяц опустел — оглавление и папка убираются (`WriteMonthIndex`).
fn write_month_index(tickets_dir: &Path, shard: &str) -> io::Result<()> {
    let folder = tickets_dir.join(shard);
    if !folder.is_dir() {
        return Ok(());
    }
    let entries = month_entries(&folder)?;
    let index = folder.join(INDEX_FILE);
    if entries.is_empty() {
        let _ = fs::remove_file(&index);
        let _ = fs::remove_dir(&folder); // не пуста — останется
        return Ok(());
    }
    let mut sb =
        format!("# Заявки: {shard}\n\nЗаявок: {} · обновлено {}\n\n", entries.len(), chrono::Local::now().format("%d.%m.%Y %H:%M"));
    for (file, props) in &entries {
        let name = file.strip_suffix(".md").unwrap_or(file);
        let created = prop(props, "created")
            .and_then(crate::intraservice::parse::date)
            .map_or_else(String::new, |c| format!(" · {}", c.format("%d.%m.%Y")));
        let status = prop(props, "status").filter(|s| !s.is_empty()).map_or_else(String::new, |s| format!(" · {}", md(s)));
        // имя прежнего файла с обратными кавычками или «%%» — текстом: код внутри [[…]] и комментарий Obsidian
        let entry = if name.contains('`') || name.contains("%%") { md(&name.replace('`', " ")) } else { format!("[[{name}]]") };
        sb.push_str(&format!("- {entry}{created}{status}\n"));
    }
    write_atomic(&index, &sb)
}

/// Месяцы в tickets в порядке оглавления — свежие сверху, «без даты» в конце — и сколько в каждом файлов заявок.
fn months_on_disk(tickets_dir: &Path) -> io::Result<Vec<(String, usize)>> {
    let mut names: Vec<String> = fs::read_dir(tickets_dir)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_shard(n))
        .collect();
    names.sort_by(|a, b| (a == NO_DATE_FOLDER).cmp(&(b == NO_DATE_FOLDER)).then_with(|| b.cmp(a)));
    names
        .into_iter()
        .map(|n| {
            let count = fs::read_dir(tickets_dir.join(&n))?
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && id_in_name(&e.file_name().to_string_lossy()).is_some())
                .count();
            Ok((n, count))
        })
        .collect()
}

/// Общее оглавление: как читать файлы (для агента, что разбирает выгрузку) и месяцы со ссылками на их оглавления.
fn write_root_index(dir: &Path, on_disk: &[(String, usize)]) -> io::Result<()> {
    let months: Vec<&(String, usize)> = on_disk.iter().filter(|m| m.1 > 0).collect();
    let mut sb = String::from("# Заявки из Интрасервиса\n\n");
    sb.push_str(&format!(
        "Выгрузка TicketBoard для базы знаний — обновлено {}, заявок: {}.\n\n",
        chrono::Local::now().format("%d.%m.%Y %H:%M"),
        months.iter().map(|m| m.1).sum::<usize>()
    ));
    sb.push_str(&format!(
        "Заявки лежат в папке `{TICKETS_FOLDER}`, по подпапкам — месяцам создания (`{TICKETS_FOLDER}/2026-08`): в каждой оглавление `{INDEX_FILE}` и по файлу на заявку. "
    ));
    sb.push_str("В начале файла свойства: номер, статус, даты (создана, изменена, решена, закрыта), сервис, тип, категории, оборудование, инициатор и его организация, ");
    sb.push_str("исполнители и их группа, участники, ваша роль, часы до первой реакции и до решения, сколько раз переоткрывали, сколько комментариев, ");
    sb.push_str("закрывающий комментарий, ссылка — что значит каждое, сказано в `SCHEMA.md`. ");
    sb.push_str(&format!(
        "Дальше описание заявки, её вложения (скриншоты и документы — в папке `{FILES_FOLDER}`, крупные — в `{LFS_FOLDER}`, ссылками из раздела «Вложения») "
    ));
    sb.push_str("и переписка по времени — от первой записи к последней; «(внутренний)» — комментарий, которого заявитель не видел, ");
    sb.push_str("«статус «…»» — на этой записи статус поменялся. Решение обычно в последних записях закрытой заявки. ");
    sb.push_str("Текст заявок — только текст: html, команды плагинов Obsidian и «%%» в нём записаны сущностями html — `&lt;` — это «<», `&#36;` — «$», `&#37;` — «%». ");
    sb.push_str("Телефоны и почта из карточек людей в выгрузку не попадают (в тексте заявок и комментариев — как написано); пароль после «пароль:» или «password=» заменён на «[скрыт]». ");
    sb.push_str("Файл переписывается при следующей выгрузке, если заявка менялась. ");
    sb.push_str("Все заявки строкой на заявку — таблица `tickets.csv`, её колонки — в `SCHEMA.md`.\n\n");
    sb.push_str("## По месяцам\n\n");
    for (name, count) in months {
        sb.push_str(&format!("- [{name}]({TICKETS_FOLDER}/{}/{INDEX_FILE}) — заявок: {count}\n", escape_data(name)));
    }
    write_atomic(&dir.join(INDEX_FILE), &sb)
}

/// Оглавления: в каждой тронутой за выгрузку папке месяца — свой `_index.md` (и в тех, где его нет вовсе), в корне — общий.
/// Месяц переписывается целиком, остальные не трогаются. Возвращает первую ошибку записи («» — всё записано): не вышло с
/// одним месяцем — остальные пишутся (`WriteIndexes`).
// ponytail: таблица tickets.csv и памятки SCHEMA.md / CLAUDE.md (`KnowledgeTable`) — следующий шаг слоя 4.
pub fn write_indexes(dir: &Path, shards: &[String]) -> String {
    let tickets_dir = dir.join(TICKETS_FOLDER);
    if !tickets_dir.is_dir() {
        return String::new();
    }
    let mut error = String::new();
    let mut note = |failed: String, e: io::Error| {
        if error.is_empty() {
            error = format!("{failed}: {e}");
        }
    };
    let mut need: Vec<String> = Vec::new();
    for s in shards {
        if !need.iter().any(|n| n.to_lowercase() == s.to_lowercase()) {
            need.push(s.clone());
        }
    }
    let mut months = Vec::new();
    match fs::read_dir(&tickets_dir) {
        Ok(entries) => {
            for e in entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
                let name = e.file_name().to_string_lossy().into_owned();
                if is_shard(&name) {
                    if !e.path().join(INDEX_FILE).exists() && !need.iter().any(|n| n.to_lowercase() == name.to_lowercase()) {
                        need.push(name.clone());
                    }
                    months.push(name);
                }
            }
        }
        Err(e) => note("Оглавление tickets не записано".into(), e),
    }
    // ничего не тронуто и всё оглавлено — оглавления не переписываем (прервавшаяся сразу выгрузка не должна плодить файлы)
    let indexes = !need.is_empty() || (!months.is_empty() && !dir.join(INDEX_FILE).exists());
    if indexes {
        for shard in &need {
            if let Err(e) = write_month_index(&tickets_dir, shard) {
                note(format!("Оглавление {shard}/{INDEX_FILE} не записано"), e);
            }
        }
    }
    // после оглавлений месяцев: опустевший месяц они убирают
    match months_on_disk(&tickets_dir) {
        Ok(on_disk) if indexes => {
            if let Err(e) = write_root_index(dir, &on_disk) {
                note(format!("Оглавление {INDEX_FILE} не записано"), e);
            }
        }
        Ok(_) => {}
        Err(e) => note("Оглавление tickets не записано".into(), e),
    }
    error
}
