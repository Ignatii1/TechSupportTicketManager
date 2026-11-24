//! Файлы базы знаний на диске (`KnowledgeExport.Files.cs`, `.Attachments.cs`): где лежит заявка, как называется, какой файл
//! наш, запись через временный файл, оглавления, вложения, что уже лежат в базе (ссылкой; не качаются). Длины и обрезка — в
//! единицах UTF-16, как у C#: один и тот же путь и имя у обоих приложений, иначе файл переписывался бы каждый раз.

use super::table;
use super::{Attachment, AttachmentState, mask, md, one_line, visible};
use crate::intraservice::{Date, Found, TaskFile};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
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

// ---------- сравнение без регистра ----------

/// Знак для сравнения без регистра, как `OrdinalIgnoreCase` у .NET: простое верхнее написание по одному знаку (σ и ς — один
/// знак; знак Кельвина — не «k»); то, у чего верхнее — несколько знаков (ß), остаётся как есть.
fn fold_char(c: char) -> char {
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

pub(super) fn fold(s: &str) -> String {
    s.chars().map(fold_char).collect()
}

/// `a.EndsWith(b, OrdinalIgnoreCase)`.
fn ends_with_ci(a: &str, b: &str) -> bool {
    fold(a).ends_with(&fold(b))
}

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

/// Цифры в начале, дальше — конец или « — …» (`^(\d+)( — .*)?$`): сами цифры.
fn id_digits(s: &str) -> Option<&str> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let rest = &s[digits..];
    (digits > 0 && (rest.is_empty() || (rest.starts_with(SEPARATOR) && !rest.contains('\n')))).then(|| &s[..digits])
}

/// Номер в начале как число; не влезает в 32 бита — не наш (`int.TryParse`).
fn leading_id(s: &str) -> Option<i32> {
    id_digits(s)?.parse().ok()
}

/// Имя подходит под «номер — название.md» или «номер.md» (`IdInName.IsMatch`) — номер не разбирается: оглавления считают и
/// перечисляют файлы по шаблону, как C#.
fn is_ticket_name(name: &str) -> bool {
    name.strip_suffix(".md").and_then(id_digits).is_some()
}

/// Номер заявки в имени файла (`TryIdOf`): шаблон и номер в 32 бита.
fn id_in_name(name: &str) -> Option<i32> {
    name.strip_suffix(".md").and_then(leading_id)
}

/// Номер файла вложения по его имени на диске; временный файл закачки и чужие — нет (`Attachments.TryIdOf`).
fn attachment_id(name: &str) -> Option<i32> {
    (!ends_with_ci(name, ".tmp")).then(|| leading_id(name)).flatten()
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
        shards.iter().filter(|s| seen.insert(fold(s))).map(|s| (format!("{s}/"), tickets_dir.join(s))).collect();
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
    fold(&a.replace('\\', "/")) == fold(&b.replace('\\', "/"))
}

pub(super) fn prop<'a>(props: &'a [(String, String)], key: &str) -> Option<&'a str> {
    props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// Число из свойства: только цифры; нет или не число — 0.
fn number(props: &[(String, String)], key: &str) -> i32 {
    prop(props, key).filter(|v| v.bytes().all(|b| b.is_ascii_digit())).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Файл — наш, выгрузки: его можно переписать и удалить как лишний (`IsOurs`).
pub(super) fn is_ours(props: &Option<Props>) -> bool {
    props.as_deref().is_some_and(ours_props)
}

pub(super) fn ours_props(props: &[(String, String)]) -> bool {
    prop(props, "source") == Some(SOURCE_NAME)
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
    if NEUTRALIZED.iter().chain(SHELL_READS.iter()).any(|e| ends_with_ci(&clean, e)) {
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
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"].iter().any(|e| ends_with_ci(name, e))
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
            AttachmentState::Saved { path, .. } => Some((a.file.id, fold(path))),
            _ => None,
        })
        .collect();
    let (plain, large) = file_dirs(root, shard, f.id);
    for folder in [plain, large].iter().filter(|d| d.is_dir()) {
        let Ok(entries) = fs::read_dir(folder) else { continue };
        for path in entries.flatten().map(|e| e.path()).filter(|p| p.is_file()) {
            let Some(id) = attachment_id(&path.file_name().unwrap_or_default().to_string_lossy()) else { continue };
            let extra = !listed.contains(&id) || saved.get(&id).is_some_and(|keep| *keep != fold(&in_base(root, &path)));
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

// ---------- папка ----------

/// Часть пути к папке, что кончается пробелом или точкой («…\\папка \\внутри»), — или None. Windows обрезает их только у
/// последней части пути: такую папку создаст без пробела, а путь через неё не найдёт («The system cannot find the path
/// specified»; найдено на рабочем ПК 2026-11-24, у C# — то же, текстом ОС). Последняя часть — тоже: к ней допишется
/// `tickets`. «.» и «..» — ходы по пути, не имена.
pub fn bad_folder_part(dir: &Path) -> Option<String> {
    dir.components().find_map(|c| match c {
        std::path::Component::Normal(name) => {
            let name = name.to_string_lossy();
            name.ends_with([' ', '.']).then(|| name.into_owned())
        }
        _ => None,
    })
}

/// Что сказать о такой части пути (строкой под кнопками).
pub fn bad_folder_message(part: &str) -> String {
    format!("Папка «{part}» в пути кончается пробелом или точкой — Windows её не найдёт. Уберите их из поля.")
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
    write_atomic_with(path, text.as_bytes(), |tmp, bytes| fs::write(tmp, bytes), |from, to| fs::rename(from, to), &WRITE_RETRY)
}

/// `write_atomic` для таблицы: дата записи файла — начало записи, а не конец (`FromStart`). Временный файл создаётся первым,
/// его дата (по часам файловой системы — тем же, что у файлов заявок, даже на сетевом диске) запоминается; `build` собирает
/// байты уже после этого, и дата возвращается временному перед подменой: файл заявки, что поменялся, пока собиралась таблица,
/// окажется новее её, и его месяц перечитается в следующий раз. `build`: `None` — остановили, ничего не пишется (`Ok(false)`);
/// ошибка — как у записи. Собирается один раз: повторяются только запись временного и подмена.
pub fn write_atomic_from_start(path: &Path, build: impl FnOnce() -> io::Result<Option<Vec<u8>>>) -> io::Result<bool> {
    use std::io::Write;
    let (mut started, mut built, mut build, mut stopped) = (None, None, Some(build), false);
    let result = write_atomic_with(
        path,
        &[],
        |tmp, _| {
            let mut file = fs::File::create(tmp)?;
            let start = *started.get_or_insert(file.metadata()?.modified()?);
            if built.is_none()
                && let Some(build) = build.take()
            {
                built = build()?;
                stopped = built.is_none();
            }
            let Some(bytes) = &built else { return Err(io::Error::other("остановлено")) };
            file.write_all(bytes)?;
            file.set_modified(start)
        },
        |from, to| fs::rename(from, to),
        &WRITE_RETRY,
    );
    match result {
        Ok(()) => Ok(true),
        Err(_) if stopped => Ok(false),
        Err(e) => Err(e),
    }
}

/// `write_atomic` с подменой записи, переименования и пауз: занятый на время файл на Linux не устроить, а повтор проверить
/// надо.
fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    mut write: impl FnMut(&Path, &[u8]) -> io::Result<()>,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    pauses: &[Duration],
) -> io::Result<()> {
    let tmp_of = || {
        let mut name = path.as_os_str().to_os_string();
        name.push(format!(".{}.tmp", random4()));
        PathBuf::from(name)
    };
    let mut attempt = 0;
    let tmp = loop {
        let tmp = tmp_of();
        match write(&tmp, bytes) {
            Ok(()) => break tmp,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                if !is_busy(&e) || attempt >= pauses.len() {
                    return Err(e);
                }
            }
        }
        std::thread::sleep(pauses[attempt]);
        attempt += 1;
    };
    for pause in pauses.iter().map(Some).chain([None]) {
        // на Windows rename заменяет существующий файл (MoveFileEx с заменой)
        match (rename(&tmp, path), pause) {
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
    temp_base(name).is_some_and(|b| is_ticket_name(&b))
}

/// Временный файл закачки вложения «номер — имя.xxxx.tmp» (`FileTemp`).
fn is_file_temp(name: &str) -> bool {
    temp_base(name).is_some_and(|b| id_digits(&b).is_some())
}

fn sweep_with(folder: &Path, ours: impl Fn(&str) -> bool) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !ends_with_ci(&name, ".tmp") || !ours(&name) || !entry.file_type().is_ok_and(|t| t.is_file()) {
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
        // ReadAllText не падает на чужой кодировке и снимает метку порядка байтов; дописывается байтами — прежние целы
        let text = if path.is_file() {
            String::from_utf8_lossy(&fs::read(path)?).trim_start_matches('\u{FEFF}').to_string()
        } else {
            String::new()
        };
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
pub(super) fn month_entries(folder: &Path) -> io::Result<Vec<(String, Props)>> {
    let mut entries: Vec<(String, Props)> = fs::read_dir(folder)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_ticket_name(n))
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

/// Папка месяца: сколько в ней файлов заявок и когда в ней что-то менялось последний раз (`MonthOnDisk`).
#[derive(Clone, Debug, PartialEq)]
pub struct MonthOnDisk {
    pub name: String,
    pub count: usize,
    pub newest: SystemTime,
}

/// Месяцы в tickets в порядке оглавления — свежие сверху, «без даты» в конце — одним обходом: сколько в каждом файлов заявок
/// и дата последней перемены (папки или файла заявки: добавили, удалили, переименовали) (`MonthsOnDisk`).
pub fn months_on_disk(tickets_dir: &Path) -> io::Result<Vec<MonthOnDisk>> {
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
            let folder = tickets_dir.join(&n);
            let mut newest = fs::metadata(&folder)?.modified()?;
            let mut count = 0;
            for e in fs::read_dir(&folder)?.flatten() {
                if e.file_type().is_ok_and(|t| t.is_file()) && is_ticket_name(&e.file_name().to_string_lossy()) {
                    count += 1;
                    newest = newest.max(e.metadata()?.modified()?);
                }
            }
            Ok(MonthOnDisk { name: n, count, newest })
        })
        .collect()
}

/// Общее оглавление: как читать файлы (для агента, что разбирает выгрузку) и месяцы со ссылками на их оглавления.
fn write_root_index(dir: &Path, on_disk: &[MonthOnDisk]) -> io::Result<()> {
    let months: Vec<&MonthOnDisk> = on_disk.iter().filter(|m| m.count > 0).collect();
    let mut sb = String::from("# Заявки из Интрасервиса\n\n");
    sb.push_str(&format!(
        "Выгрузка TicketBoard для базы знаний — обновлено {}, заявок: {}.\n\n",
        chrono::Local::now().format("%d.%m.%Y %H:%M"),
        months.iter().map(|m| m.count).sum::<usize>()
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
    for MonthOnDisk { name, count, .. } in months {
        sb.push_str(&format!("- [{name}]({TICKETS_FOLDER}/{}/{INDEX_FILE}) — заявок: {count}\n", escape_data(name)));
    }
    write_atomic(&dir.join(INDEX_FILE), &sb)
}

/// Оглавления: в каждой тронутой за выгрузку папке месяца — свой `_index.md` (и в тех, где его нет вовсе), в корне — общий.
/// Месяц переписывается целиком, остальные не трогаются. Возвращает первую ошибку записи («» — всё записано): не вышло с
/// одним месяцем — остальные пишутся (`WriteIndexes`).
pub fn write_indexes(dir: &Path, shards: &[String], stop: &AtomicBool, progress: &(dyn Fn(String) + Sync)) -> String {
    let tickets_dir = dir.join(TICKETS_FOLDER);
    if !tickets_dir.is_dir() {
        return String::new();
    }
    let mut error = String::new();
    let mut first = |message: String| {
        if error.is_empty() {
            error = message;
        }
    };
    let mut need: Vec<String> = Vec::new();
    for s in shards {
        if !need.iter().any(|n| fold(n) == fold(s)) {
            need.push(s.clone());
        }
    }
    let mut months = Vec::new();
    match fs::read_dir(&tickets_dir) {
        Ok(entries) => {
            for e in entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
                let name = e.file_name().to_string_lossy().into_owned();
                if is_shard(&name) {
                    if !e.path().join(INDEX_FILE).exists() && !need.iter().any(|n| fold(n) == fold(&name)) {
                        need.push(name.clone());
                    }
                    months.push(name);
                }
            }
        }
        Err(e) => first(format!("Оглавление tickets не записано: {e}")),
    }
    // ничего не тронуто и всё оглавлено — оглавления не переписываем (прервавшаяся сразу выгрузка не должна плодить файлы)
    let indexes = !need.is_empty() || (!months.is_empty() && !dir.join(INDEX_FILE).exists());
    if indexes {
        for shard in &need {
            if let Err(e) = write_month_index(&tickets_dir, shard) {
                first(format!("Оглавление {shard}/{INDEX_FILE} не записано: {e}"));
            }
        }
    }
    // после оглавлений месяцев (опустевший месяц они убирают): один обход папок — и на общее оглавление, и на таблицу
    let on_disk = match months_on_disk(&tickets_dir) {
        Ok(on_disk) => on_disk,
        Err(e) => {
            first(format!("Оглавление tickets не записано: {e}"));
            Vec::new()
        }
    };
    if indexes && let Err(e) = write_root_index(dir, &on_disk) {
        first(format!("Оглавление {INDEX_FILE} не записано: {e}"));
    }
    // таблица — и когда ничего не тронуто: её прошлая запись могла не удаться (файл был открыт в Excel); не отстала — не
    // пишется. Остановили — не собирается: прежняя цела, догонит следующая выгрузка
    if !on_disk.is_empty() || dir.join(table::TABLE_FILE).is_file() {
        match table::write(dir, &need, &on_disk, stop, progress) {
            Ok(note) => first(note), // пустое примечание ничего не меняет
            Err(e) => first(format!("Таблица {} не записана: {e}", table::TABLE_FILE)),
        }
    }
    if !on_disk.is_empty() {
        match table::write_guides(dir) {
            Ok(note) => first(note), // пустое примечание ничего не меняет
            Err(e) => first(format!("{} или {} не записан: {e}", table::SCHEMA_FILE, table::AGENT_FILE)),
        }
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intraservice::Extra;

    /// Временная папка проверки (без неё `Drop` не убрал бы за упавшей проверкой — убирает следующий запуск тем же именем).
    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tb-files-unit-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn ticket_with(id: i32, files: &[(i32, &str)]) -> Found {
        let files = files.iter().map(|(id, name)| TaskFile { id: *id, name: Some((*name).to_string()) }).collect();
        Found { id, extra: Some(Extra { files: Some(files), ..Extra::default() }), ..Found::default() }
    }

    #[test]
    fn only_a_year_month_or_no_date_is_a_month_folder() {
        for yes in ["2026-09", "1999-12", NO_DATE_FOLDER] {
            assert!(is_shard(yes), "{yes}");
        }
        // длина, цифры в начале, дефис, цифры в конце — каждое условие само по себе
        for no in ["2026-0", "2026-091", "ab26-09", "2026x09", "2026-0x", "2026/09", "", "notes", "20260-9"] {
            assert!(!is_shard(no), "{no}");
        }
    }

    #[test]
    fn leading_digits_are_a_number_alone_or_before_the_separator_on_one_line() {
        assert_eq!(id_digits("12"), Some("12"));
        assert_eq!(id_digits("12 — заметка"), Some("12"));
        for no in ["", " — заметка", "12abc", "12 — a\nb", "abc", "12 - a"] {
            assert_eq!(id_digits(no), None, "{no:?}");
        }
    }

    #[test]
    fn a_name_that_starts_with_a_dot_has_no_extension_to_keep() {
        // C#: `dot > 0` — точка в самом начале не расширение; основа режется, а не пропадает вместе с «расширением»
        assert_eq!(disk_name(1, Some(".gitignore"), 9), "1 — .giti");
    }

    #[test]
    fn the_name_budget_follows_the_length_of_the_longer_folder_path() {
        let root = tmp("budget");
        let long = format!("{}.txt", "a".repeat(300));
        let f = ticket_with(7001, &[(5, &long)]);
        let (plain, large) = file_dirs(&root, "2026-09", 7001);
        put(&plain.join("5 — старое имя.bin"), "xyz");
        let max = MAX_PATH_LENGTH - large.to_string_lossy().encode_utf16().count() - 1;
        let want = disk_name(5, Some(&long), max);
        let mut seen = FilesSeen::default();
        let got = save_files(&root, &f, "2026-09", &mut seen);
        let expected = AttachmentState::Saved { path: in_base(&root, &plain.join(&want)), size: 3 };
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].state, expected, "файл переименован под имя по бюджету пути");
        assert!(plain.join(&want).is_file());
        assert_eq!((seen.present, seen.skipped, seen.failed), (1, 0, 0));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_file_with_the_current_name_is_kept_among_several_with_the_same_number() {
        let root = tmp("exact");
        let f = ticket_with(7002, &[(5, "b.txt")]);
        let (plain, _) = file_dirs(&root, "2026-09", 7002);
        put(&plain.join("5 — a.txt"), "a");
        put(&plain.join("5 — b.txt"), "bb");
        let got = save_files(&root, &f, "2026-09", &mut FilesSeen::default());
        assert_eq!(got[0].state, AttachmentState::Saved { path: in_base(&root, &plain.join("5 — b.txt")), size: 2 });
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_card_with_an_empty_file_list_touches_nothing_in_the_files_folders() {
        // C#: `Count: > 0` — пустой список вложений выходит до папок: даже старый временный файл закачки остаётся
        let root = tmp("empty-list");
        let f = ticket_with(7003, &[]);
        let (plain, _) = file_dirs(&root, "2026-09", 7003);
        let stale = plain.join("5 — a.pdf.1a2b.tmp");
        put(&stale, "половина");
        let file = fs::OpenOptions::new().write(true).open(&stale).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        assert!(save_files(&root, &f, "2026-09", &mut FilesSeen::default()).is_empty());
        assert!(stale.exists(), "устаревший временный файл не тронут: списка вложений нет");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_attachments_are_counted_and_the_first_error_is_the_first_one() {
        let mut seen = FilesSeen::default();
        let file = TaskFile { id: 3, name: Some("a.txt".into()) };
        let one = failed(7, &file, "не открылось", "заметка", &mut seen);
        assert_eq!(one.state, AttachmentState::Failed { note: "заметка".into() });
        failed(8, &file, "другая беда", "заметка", &mut seen);
        assert_eq!(seen.failed, 2);
        assert!(seen.first_error.starts_with("#7 «") && !seen.first_error.contains("#8"), "{}", seen.first_error);
    }

    #[test]
    fn pruning_removes_what_the_card_no_longer_lists_and_copies_beside_the_kept_one_and_empty_folders() {
        let root = tmp("prune");
        let f = ticket_with(7004, &[(5, "a.txt")]);
        let (plain, large) = file_dirs(&root, "2026-09", 7004);
        put(&plain.join("5 — a.txt"), "keep");
        put(&plain.join("6 — gone.txt"), "gone");
        put(&plain.join("заметка.txt"), "mine");
        put(&large.join("5 — a.txt"), "copy");
        let kept = vec![Attachment {
            file: TaskFile { id: 5, name: Some("a.txt".into()) },
            state: AttachmentState::Saved { path: in_base(&root, &plain.join("5 — a.txt")), size: 4 },
        }];
        prune_files(&root, &f, "2026-09", &kept);
        assert!(plain.join("5 — a.txt").is_file(), "лежащее в базе остаётся");
        assert!(plain.join("заметка.txt").is_file(), "чужое не тронуто");
        assert!(!plain.join("6 — gone.txt").exists(), "в карточке такого вложения больше нет");
        assert!(!large.join("5 — a.txt").exists(), "лишняя копия того, что лежит в другой папке");
        assert!(!large.exists(), "опустевшая папка убрана");
        // карточка список не прислала — ничего не трогаем
        let none = Found { id: 7004, ..Found::default() };
        put(&plain.join("9 — x.txt"), "x");
        prune_files(&root, &none, "2026-09", &[]);
        assert!(plain.join("9 — x.txt").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_busy_file_is_a_sharing_violation_a_lock_or_a_refusal_and_nothing_else() {
        assert!(is_busy(&io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(is_busy(&io::Error::from_raw_os_error(32)));
        assert!(is_busy(&io::Error::from_raw_os_error(33)));
        assert!(!is_busy(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!is_busy(&io::Error::from_raw_os_error(2)));
        assert!(!is_busy(&io::Error::other("диск полон")));
    }

    #[test]
    fn the_temp_suffix_is_four_hex_digits() {
        for _ in 0..50 {
            let s = random4();
            assert!(s.len() == 4 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "{s:?}");
        }
    }

    #[test]
    fn an_attachment_temp_is_a_number_a_name_four_hex_digits_and_tmp() {
        for yes in ["5 — a.pdf.1a2b.tmp", "12.1A2B.tmp"] {
            assert!(is_file_temp(yes), "{yes}");
        }
        for no in ["notes.md.1a2b.tmp", "5 — a.pdf.tmp", "5 — a.pdf.zzzz.tmp", "5 — a.pdf", "заметка.1a2b.tmp"] {
            assert!(!is_file_temp(no), "{no}");
        }
    }

    #[test]
    fn properties_come_from_the_first_quarter_megabyte_not_the_first_kilobyte() {
        let dir = tmp("props");
        let path = dir.join("1 — x.md");
        put(&path, &format!("---\nsource: intraservice\ntitle: {}\nrevision: 5\n---\nтекст\n", "x".repeat(3000)));
        let props = read_props(&path).expect("свойства прочитаны: закрывающая черта в пределах 256 КБ");
        assert_eq!(props.iter().find(|(k, _)| k == "revision").map(|(_, v)| v.as_str()), Some("5"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_month_index_writes_a_link_unless_the_old_file_name_would_break_it() {
        let dir = tmp("month-index");
        let month = dir.join("2026-09");
        let props = |id: i32| format!("---\nid: {id}\ncreated: \"2026-09-0{id}T10:00:00+03:00\"\nstatus: \"Открыта\"\n---\n");
        put(&month.join("1 — a`b.md"), &props(1)); // только обратная кавычка
        put(&month.join("2 — c%%d.md"), &props(2)); // только «%%»
        put(&month.join("3 — plain.md"), &props(3));
        write_month_index(&dir, "2026-09").unwrap();
        let index = fs::read_to_string(month.join(INDEX_FILE)).unwrap();
        assert!(index.contains("- 1 — a b · "), "{index}");
        assert!(index.contains("- [[3 — plain]] · "), "{index}");
        assert!(!index.contains("[[1 — ") && !index.contains("[[2 — ") && !index.contains("%%") && !index.contains('`'), "{index}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Подставная запись или подмена: первые `busy` вызовов — «занят» (или `fail` — другая ошибка), дальше — настоящая.
    struct Flaky {
        calls: usize,
        busy: usize,
        fail: bool,
    }

    impl Flaky {
        fn new(busy: usize, fail: bool) -> Self {
            Flaky { calls: 0, busy, fail }
        }

        fn next(&mut self) -> Option<io::Error> {
            self.calls += 1;
            if self.calls > self.busy {
                None
            } else if self.fail {
                Some(io::Error::other("диск полон"))
            } else {
                Some(io::Error::from(io::ErrorKind::PermissionDenied))
            }
        }
    }

    const NO_PAUSES: [Duration; 3] = [Duration::ZERO; 3];

    /// Запись через подставные: (итог, вызовов записи, вызовов подмены, что лежит в папке).
    fn write_flaky(tag: &str, mut w: Flaky, mut r: Flaky) -> (io::Result<()>, usize, usize, Vec<String>) {
        let dir = tmp(tag);
        let path = dir.join("1 — заявка.md");
        let result = write_atomic_with(
            &path,
            "текст".as_bytes(),
            |tmp, bytes| w.next().map_or_else(|| fs::write(tmp, bytes), Err),
            |from, to| r.next().map_or_else(|| fs::rename(from, to), Err),
            &NO_PAUSES,
        );
        let mut names: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let _ = fs::remove_dir_all(&dir);
        (result, w.calls, r.calls, names)
    }

    #[test]
    fn busy_temp_is_written_again_after_pauses() {
        // временный занят дважды — третья попытка ложится, подмена одна
        let (result, writes, renames, names) = write_flaky("busy-write", Flaky::new(2, false), Flaky::new(0, false));
        assert!(result.is_ok(), "{result:?}");
        assert_eq!((writes, renames, names), (3, 1, vec!["1 — заявка.md".to_string()]));
    }

    #[test]
    fn busy_temp_gives_up_after_the_last_pause() {
        // занят дольше всех пауз: попытка и по одной после каждой паузы, потом ошибка; подменять нечего
        let (result, writes, renames, names) = write_flaky("busy-write-long", Flaky::new(10, false), Flaky::new(0, false));
        assert_eq!(result.map_err(|e| e.kind()), Err(io::ErrorKind::PermissionDenied));
        assert_eq!((writes, renames, names), (1 + NO_PAUSES.len(), 0, Vec::<String>::new()));
    }

    #[test]
    fn other_temp_error_is_not_retried() {
        // не «занят» (диск полон) — от повтора не пройдёт, сразу ошибка
        let (result, writes, renames, _) = write_flaky("full-write", Flaky::new(10, true), Flaky::new(0, false));
        assert_eq!(result.map_err(|e| e.to_string()), Err("диск полон".to_string()));
        assert_eq!((writes, renames), (1, 0));
    }

    #[test]
    fn busy_target_repeats_only_the_rename() {
        // файл на месте занят (открыт) дважды — временный не пишется заново, повторяется подмена
        let (result, writes, renames, names) = write_flaky("busy-rename", Flaky::new(0, false), Flaky::new(2, false));
        assert!(result.is_ok(), "{result:?}");
        assert_eq!((writes, renames, names), (1, 3, vec!["1 — заявка.md".to_string()]));
    }

    #[test]
    fn busy_target_gives_up_and_removes_the_temp() {
        // занят дольше всех пауз: подмена и по одной после каждой паузы, ошибка, временный убран
        let (result, writes, renames, names) = write_flaky("busy-rename-long", Flaky::new(0, false), Flaky::new(10, false));
        assert_eq!(result.map_err(|e| e.kind()), Err(io::ErrorKind::PermissionDenied));
        assert_eq!((writes, renames, names), (1, 1 + NO_PAUSES.len(), Vec::<String>::new()));
    }

    #[test]
    fn other_rename_error_is_not_retried_and_removes_the_temp() {
        let (result, writes, renames, names) = write_flaky("full-rename", Flaky::new(0, false), Flaky::new(10, true));
        assert_eq!(result.map_err(|e| e.to_string()), Err("диск полон".to_string()));
        assert_eq!((writes, renames, names), (1, 1, Vec::<String>::new()));
    }

    #[test]
    fn a_folder_part_ending_in_a_space_or_a_dot_is_named() {
        let part = |p: &str| bad_folder_part(Path::new(p));
        assert_eq!(part("/база/папка /внутри"), Some("папка ".to_string()));
        assert_eq!(part("/база/папка./внутри"), Some("папка.".to_string()));
        assert_eq!(part("/база/папка/внутри "), Some("внутри ".to_string()), "к последней части допишется tickets");
        assert_eq!(part("/база/папка с пробелом/внутри"), None);
        assert_eq!(part("/база/.скрытая/внутри"), None, "точка в начале — можно");
        assert_eq!(part("/база/../внутри"), None, "«..» — ход наверх, не имя");
        assert_eq!(part("/база/./внутри"), None);
        assert_eq!(part("/база/внутри"), None);
        assert_eq!(
            bad_folder_message("папка "),
            "Папка «папка » в пути кончается пробелом или точкой — Windows её не найдёт. Уберите их из поля."
        );
    }
}
