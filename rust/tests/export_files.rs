//! Файлы базы знаний на диске без сети (`KnowledgeExport.Files.cs`, `.Attachments.cs`, `.Format.cs`): имя и место файла
//! заявки, свойства файла, запись через временный файл, уборка оборванных, `.gitignore`, оглавления, имя вложения,
//! размер, текст с вложениями (карточка 4.2-1). Таблицы сверки — ответы пишет код C#; остальное — сценарии
//! самопроверки C# на временных папках.

mod common;

use chrono::DateTime;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};
use ticketboard::export::files::*;
use ticketboard::export::*;
use ticketboard::intraservice::{Date, TaskFile, User, parse};
use ticketboard::rules::ClosedNames;

fn date(text: &str) -> Date {
    DateTime::parse_from_rfc3339(text).unwrap()
}

/// Временная папка проверки; убирается, чем бы проверка ни кончилась.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("tb-files-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir_all(&path).unwrap();
        Tmp(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn join(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Текст файла для сравнения: нет файла — об этом и сказано в сообщении проверки, а не паникой.
fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref()).unwrap_or_else(|e| format!("(не прочитать {}: {e})", path.as_ref().display()))
}

fn bytes(path: impl AsRef<Path>) -> Vec<u8> {
    fs::read(path.as_ref()).unwrap_or_default()
}

/// Записать файл вместе с его папкой.
fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Сделать файл старше на столько-то секунд (дата записи).
fn age(path: &Path, seconds: u64) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(seconds)).unwrap();
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Файл заявки в формате базы: свойства по порядку, как их пишет выгрузка.
fn ticket_text(id: i32, created: Option<&str>, status: &str) -> String {
    let created = created.map(|c| format!("created: {c}\n")).unwrap_or_default();
    format!("---\nid: {id}\ntitle: \"x\"\nstatus: \"{status}\"\n{created}source: intraservice\nformat: 8\n---\n\n# x\n")
}

/// Оглавления без таблицы: выгрузка не остановлена, ход никому не нужен (карточка 4.3-1: у `write_indexes` два параметра).
fn write_indexes(dir: &Path, shards: &[String]) -> String {
    ticketboard::export::files::write_indexes(dir, shards, &|| false, &|_| {})
}

// ---------- таблицы сверки ----------

#[test]
fn file_names_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-names", |c| file_name(c[0].parse().unwrap(), &c[1], c[2].parse().unwrap()));
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn places_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-place", |c| match parse::search(&c[1]).and_then(|p| p.found.into_iter().next()) {
        Some(found) => {
            let (shard, name) = place_of(Path::new(&c[0]), &found);
            format!("{shard}|{name}")
        }
        None => "НЕ РАЗОБРАН".into(),
    });
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn file_properties_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-props", |c| {
        let dir = Tmp::new("props");
        let file = dir.join("файл.md");
        fs::write(&file, &c[0]).unwrap();
        match read_props(&file) {
            Some(props) => props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n"),
            None => "НЕТ".into(),
        }
    });
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

/// Ответ строки таблицы `export-index`: файлы базы ({"месяц": {"имя.md": "текст"}}) → оглавления после `write_indexes`.
fn index_answer(inputs: &[String]) -> String {
    let dir = Tmp::new("index");
    let doc: serde_json::Value = serde_json::from_str(&inputs[0]).expect("входная ячейка — JSON");
    let tickets = dir.join(TICKETS_FOLDER);
    let mut months = Vec::new();
    for (month, files) in doc.as_object().unwrap() {
        months.push(month.clone());
        fs::create_dir_all(tickets.join(month)).unwrap();
        for (name, text) in files.as_object().unwrap() {
            fs::write(tickets.join(month).join(name), text.as_str().unwrap()).unwrap();
        }
    }
    write_indexes(dir.path(), &months);
    // порядок месяцев — как в оглавлении: свежие сверху, «без даты» в конце
    months.sort_by(|a, b| (a == NO_DATE_FOLDER).cmp(&(b == NO_DATE_FOLDER)).then_with(|| b.cmp(a)));
    let read = |path: PathBuf| fs::read_to_string(path).unwrap_or_else(|_| "(нет)\n".to_string());
    let mut out = String::new();
    for m in &months {
        out.push_str(&format!("== {m}/{INDEX_FILE} ==\n{}", read(tickets.join(m).join(INDEX_FILE))));
    }
    out.push_str(&format!("== {INDEX_FILE} ==\n{}", read(dir.join(INDEX_FILE))));
    regex_lite::Regex::new(r"обновлено \d\d\.\d\d\.\d{4} \d\d:\d\d").unwrap().replace_all(&out, "обновлено ДАТА").into_owned()
}

#[test]
fn indexes_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-index", index_answer);
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn attachment_names_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-disk-names", |c| disk_name(c[0].parse().unwrap(), Some(&c[1]), c[2].parse().unwrap()));
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn sizes_agree_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-size", |c| size(c[0].parse().unwrap()));
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

/// Состояния вложений из шестой ячейки `export-file-text`: «saved=путь=байты», «skipped», «failed=пояснение»; остальное и
/// недостающее — только названо (как `ParityCheck.ExportText`).
fn attachments_of(cell: &str, listed: &[TaskFile]) -> Vec<Attachment> {
    let states: Vec<&str> = if cell.is_empty() { Vec::new() } else { cell.split('|').collect() };
    listed
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let parts: Vec<&str> = states.get(i).map(|s| s.split('=').collect()).unwrap_or_default();
            let state = match parts.as_slice() {
                ["saved", path, size] => AttachmentState::Saved { path: (*path).into(), size: size.parse().expect("байты") },
                ["skipped"] => AttachmentState::Skipped,
                ["failed", note] => AttachmentState::Failed { note: (*note).into() },
                _ => AttachmentState::Listed,
            };
            Attachment { file: file.clone(), state }
        })
        .collect()
}

fn file_text_answer(c: &[String]) -> String {
    const UNREADABLE: &str = "НЕ РАЗОБРАН";
    let Some(task) = parse::task(&c[1], 0) else { return UNREADABLE.into() };
    let Some(page) = parse::lifetime(&c[2]) else { return UNREADABLE.into() };
    let row = if c[0] == "-" {
        row_of(&task)
    } else {
        match parse::search(&c[0]).and_then(|p| p.found.into_iter().next()) {
            Some(found) => found,
            None => return UNREADABLE.into(),
        }
    };
    let me = (c[3] != "-").then(|| {
        let (id, name) = c[3].split_once('|').expect("я — «номер|имя»");
        User { id: id.parse().expect("номер"), name: name.to_string() }
    });
    let names: Vec<String> = c[4].split('|').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    let context = Context { me, closed: ClosedNames::new(&names), attachments: AttachmentScope::None };
    let full = with_details(&row, &task);
    let listed = full.extra.as_ref().and_then(|x| x.files.clone()).unwrap_or_default();
    let attachments = attachments_of(&c[5], &listed);
    let url = format!("https://is.example/Task/View/{}", task.id);
    format_with(&full, &page.events, &url, &date("2026-11-17T10:00:00+03:00"), false, &context, Some(&attachments))
}

#[test]
fn the_text_with_attachments_agrees_with_csharp_on_every_table_row() {
    let wrong = common::parity_mismatches("export-file-text", file_text_answer);
    assert!(wrong.is_empty(), "расхождений с C#: {}\n{}", wrong.len(), wrong.join("\n"));
}

// ---------- имена и места ----------

#[test]
fn the_names_of_folders_are_those_of_csharp() {
    assert_eq!((TICKETS_FOLDER, INDEX_FILE, NO_DATE_FOLDER), ("tickets", "_index.md", "без даты"));
    assert_eq!((FILES_FOLDER, LFS_FOLDER, MAX_PATH_LENGTH), ("files", "files-lfs", 250));
}

#[test]
fn the_month_is_taken_in_the_zone_the_server_sent_and_no_date_has_its_own_folder() {
    // образец C#: 31 августа 23:30 по Москве — август, хотя по Гринвичу ещё август, а 1 сентября 01:00 по Москве — сентябрь
    assert_eq!(shard_of(Some(&date("2026-08-31T23:30:00+03:00"))), "2026-08");
    assert_eq!(shard_of(Some(&date("2026-09-01T01:00:00+03:00"))), "2026-09");
    assert_eq!(shard_of(Some(&date("2026-08-31T22:30:00-05:00"))), "2026-08");
    assert_eq!(shard_of(Some(&date("2026-12-31T23:59:59.999+03:00"))), "2026-12");
    assert_eq!(shard_of(Some(&date("2026-01-01T00:00:00+03:00"))), "2026-01");
    assert_eq!(shard_of(None), "без даты");
}

#[test]
fn a_property_value_loses_its_quotes_only_when_both_are_there() {
    assert_eq!(unquote("\"Закрыта\""), "Закрыта");
    assert_eq!(unquote("Закрыта"), "Закрыта");
    assert_eq!(unquote(""), "");
    assert_eq!(unquote("\""), "\"", "одна кавычка — ещё не пара");
    assert_eq!(unquote("\"\""), "");
    assert_eq!(unquote("\"a"), "\"a");
    assert_eq!(unquote("a\""), "a\"");
    // внутри: \" и \\ — как у C# (сперва кавычки, потом обратные черты)
    assert_eq!(unquote(r#""a\"b""#), "a\"b");
    assert_eq!(unquote(r#""a\\b""#), "a\\b");
    // «\\"» — сперва кавычка, потом черты: черта остаётся одна, а не «обратная черта и кавычка»
    assert_eq!(unquote(r#""a\\"b""#), "a\\\"b");
}

// ---------- свойства файла ----------

#[test]
fn a_folder_or_a_missing_file_has_no_properties() {
    let dir = Tmp::new("props-none");
    put(&dir.join("есть.md"), "---\nid: 1\n---\n");
    assert_eq!(read_props(&dir.join("есть.md")), Some(vec![("id".to_string(), "1".to_string())]), "для сравнения: настоящий файл");
    assert_eq!(read_props(dir.path()), None, "папка вместо файла");
    assert_eq!(read_props(&dir.join("нет.md")), None, "файла нет");
}

#[test]
fn properties_are_read_through_a_byte_order_mark_and_broken_utf8() {
    let dir = Tmp::new("props-bytes");
    // StreamReader у C# узнаёт метку порядка байтов и пропускает её
    let bom = dir.join("с меткой.md");
    fs::write(&bom, b"\xEF\xBB\xBF---\nid: 5\n---\n").unwrap();
    assert_eq!(read_props(&bom), Some(vec![("id".into(), "5".into())]));
    // не UTF-8 — знак замены, а не отказ
    let broken = dir.join("битый.md");
    fs::write(&broken, b"---\nid: 5\ntitle: \"\xFF\"\n---\n").unwrap();
    assert_eq!(read_props(&broken), Some(vec![("id".into(), "5".into()), ("title".into(), "\u{FFFD}".into())]));
}

// ---------- запись ----------

#[test]
fn a_file_is_written_as_utf8_without_a_mark_replaces_the_old_one_and_leaves_no_temp() {
    let dir = Tmp::new("atomic");
    let path = dir.join("501 — Принтер.md");
    write_atomic(&path, "первый текст\n").unwrap();
    assert_eq!(bytes(&path), "первый текст\n".as_bytes());
    write_atomic(&path, "второй, длиннее первого текст\n").unwrap();
    assert_eq!(bytes(&path), "второй, длиннее первого текст\n".as_bytes());
    assert!(!bytes(&path).starts_with(b"\xEF\xBB\xBF"), "метки порядка байтов нет");
    assert_eq!(names_in(dir.path()), ["501 — Принтер.md"], "временного рядом не осталось");
}

#[test]
fn a_write_onto_a_folder_fails_and_leaves_neither_a_temp_nor_a_changed_folder() {
    let dir = Tmp::new("atomic-folder");
    let blocked = dir.join("занято.md");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("внутри.txt"), "моё").unwrap();
    assert!(write_atomic(&blocked, "текст").is_err());
    assert_eq!(names_in(dir.path()), ["занято.md"], "временный файл убран");
    assert_eq!(text(blocked.join("внутри.txt")), "моё");
}

#[test]
fn a_write_into_a_missing_folder_fails_and_does_not_make_it() {
    let dir = Tmp::new("atomic-missing");
    assert!(write_atomic(&dir.join("нет такой/файл.md"), "текст").is_err());
    assert!(names_in(dir.path()).is_empty(), "папку и файл записи не создаёт");
}

#[test]
fn a_leftover_tmp_of_the_same_name_does_not_stop_a_write() {
    // имя временного — своё у каждой записи: «X.md.tmp» (папка, чужой файл) записи не мешает
    let dir = Tmp::new("atomic-leftover");
    let path = dir.join("свободно.md");
    fs::create_dir(dir.join("свободно.md.tmp")).unwrap();
    write_atomic(&path, "текст").unwrap();
    assert_eq!(text(&path), "текст");
    assert!(dir.join("свободно.md.tmp").is_dir());
}

// ---------- оборванные записи ----------

#[test]
fn stale_temps_of_ours_are_swept_and_fresh_and_foreign_ones_stay() {
    let dir = Tmp::new("sweep");
    let stale = |name: &str| {
        let path = dir.join(name);
        put(&path, "");
        age(&path, 3600);
        path
    };
    let ours = [
        stale("501 — старое.md.1a2b.tmp"),
        stale("502.md.1A2B.tmp"),
        stale("_index.md.9a8b.tmp"),
        stale("tickets.csv.1c2d.tmp"),
        stale("SCHEMA.md.1c2d.tmp"),
        stale("CLAUDE.md.1c2d.tmp"),
    ];
    let stays = [
        stale("501 — мои выводы.md.tmp"),     // так сохраняют редакторы
        stale("заметка.tmp"),                 // чужой
        stale(".syncthing.501 — VPN.md.tmp"), // синхронизатор
        stale("черновик.md.tmp"),
        stale("501 — x.md.12345.tmp"), // пять знаков — не наши
        stale("501 — x.md.zzzz.tmp"),
        stale("501 — x.png.1a2b.tmp"), // закачка вложения — не запись заявки
        stale("501 — заметка.md"),
    ];
    let fresh = dir.join("501 — пишется.md.3c4d.tmp");
    put(&fresh, "");
    let nine_minutes = dir.join("503 — недавно.md.5e6f.tmp");
    put(&nine_minutes, "");
    age(&nine_minutes, 9 * 60);
    let eleven_minutes = dir.join("504 — давно.md.5e6f.tmp");
    put(&eleven_minutes, "");
    age(&eleven_minutes, 11 * 60);
    // папка с таким именем и файл в подпапке — не трогаются
    let folder = dir.join("505.md.1a2b.tmp");
    fs::create_dir(&folder).unwrap();
    let nested = dir.join("глубже/506.md.1a2b.tmp");
    put(&nested, "");
    age(&nested, 3600);

    sweep_temps(dir.path());

    for path in &ours {
        assert!(!path.exists(), "наш оборванный старше десяти минут должен быть убран: {}", path.display());
    }
    for path in stays.iter().chain([&fresh, &nine_minutes, &folder, &nested]) {
        assert!(path.exists(), "не наш или свежий не трогаем: {}", path.display());
    }
    assert!(!eleven_minutes.exists(), "одиннадцать минут — уже старый");
}

#[test]
fn sweeping_a_folder_that_is_not_there_is_not_an_error() {
    let dir = Tmp::new("sweep-missing");
    sweep_temps(&dir.join("нет такой"));
    let file = dir.join("файл");
    put(&file, "");
    sweep_temps(&file);
    assert!(file.exists());
}

// ---------- .gitignore ----------

#[test]
fn a_folder_in_a_git_repository_gets_tmp_into_its_gitignore_once() {
    let dir = Tmp::new("git");
    fs::create_dir_all(dir.join("repo/.git")).unwrap();
    let kb = dir.join("repo/kb");
    fs::create_dir(&kb).unwrap();
    assert_eq!(ignore_temps_in_git(&kb), "");
    assert_eq!(text(kb.join(".gitignore")), "*.tmp\n");
    // без памяти и без повтора: строка есть — файл не меняется
    assert_eq!(ignore_temps_in_git(&kb), "");
    assert_eq!(text(kb.join(".gitignore")), "*.tmp\n");
}

#[test]
fn lines_of_the_user_stay_and_the_new_one_starts_on_its_own_line() {
    let dir = Tmp::new("git-lines");
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(dir.join(".gitignore"), "*.psd\nbuild").unwrap();
    assert_eq!(ignore_temps_in_git(dir.path()), "", "сама папка — корень репозитория");
    assert_eq!(text(dir.join(".gitignore")), "*.psd\nbuild\n*.tmp\n");
}

#[test]
fn a_gitignore_that_already_has_the_line_is_left_byte_for_byte() {
    let dir = Tmp::new("git-has");
    fs::create_dir_all(dir.join(".git")).unwrap();
    let content = "a\r\n  *.tmp  \r\nb\r\n";
    fs::write(dir.join(".gitignore"), content).unwrap();
    assert_eq!(ignore_temps_in_git(dir.path()), "");
    assert_eq!(text(dir.join(".gitignore")), content, "строка сравнивается без пробелов и «\\r» по краям");
}

#[test]
fn a_gitignore_in_cp1251_gets_the_line_once_and_keeps_its_bytes() {
    // C#: File.ReadAllText не падает на чужой кодировке, дописывает байтами (AppendAllText) — «пользователь» не теряет файл
    let dir = Tmp::new("git-cp1251");
    fs::create_dir_all(dir.join(".git")).unwrap();
    let mut old = vec![b'#', b' '];
    old.extend([0xEF, 0xF0, 0xE8, 0xEC, 0xE5, 0xF7, 0xE0, 0xED, 0xE8, 0xE5]); // «пример» в cp1251 (не UTF-8)
    fs::write(dir.join(".gitignore"), &old).unwrap();
    assert_eq!(ignore_temps_in_git(dir.path()), "", "не UTF-8 — не ошибка");
    let mut want = old.clone();
    want.extend(b"\n*.tmp\n");
    assert_eq!(bytes(dir.join(".gitignore")), want, "строка дописана, прежние байты целы");
    assert_eq!(ignore_temps_in_git(dir.path()), "");
    assert_eq!(bytes(dir.join(".gitignore")), want, "второй раз — без повтора");
}

#[test]
fn a_gitignore_with_a_byte_order_mark_and_the_line_first_is_left_as_it_is() {
    // C#: ReadAllText снимает метку — первая строка «*.tmp» узнана
    let dir = Tmp::new("git-bom");
    fs::create_dir_all(dir.join(".git")).unwrap();
    let content = b"\xEF\xBB\xBF*.tmp\r\nbuild\r\n";
    fs::write(dir.join(".gitignore"), content).unwrap();
    assert_eq!(ignore_temps_in_git(dir.path()), "");
    assert_eq!(bytes(dir.join(".gitignore")), content, "строка уже есть (под меткой) — файл не менялся");
}

#[test]
fn a_git_file_instead_of_a_folder_is_also_a_repository() {
    // рабочее дерево и подмодуль: «.git» — файл
    let dir = Tmp::new("git-file");
    fs::write(dir.join(".git"), "gitdir: ../elsewhere\n").unwrap();
    let kb = dir.join("kb");
    fs::create_dir(&kb).unwrap();
    assert_eq!(ignore_temps_in_git(&kb), "");
    assert_eq!(text(kb.join(".gitignore")), "*.tmp\n");
}

#[test]
fn a_folder_outside_any_repository_gets_no_gitignore() {
    let dir = Tmp::new("git-none");
    let mut up = Some(dir.path());
    while let Some(p) = up {
        if p.join(".git").exists() {
            eprintln!("над временной папкой лежит репозиторий ({}) — проверка не к чему", p.display());
            return;
        }
        up = p.parent();
    }
    assert_eq!(ignore_temps_in_git(dir.path()), "");
    assert!(!dir.join(".gitignore").exists());
}

#[test]
fn a_gitignore_that_cannot_be_written_is_said_with_its_path() {
    let dir = Tmp::new("git-stuck");
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::create_dir(dir.join(".gitignore")).unwrap();
    let note = ignore_temps_in_git(dir.path());
    assert!(note.starts_with("не удалось дописать *.tmp в "), "{note}");
    assert!(note.contains(&dir.join(".gitignore").display().to_string()), "в фразе — путь файла: {note}");
    assert!(!note.ends_with('.') && !note.ends_with(' '), "точку ставит тот, кто продолжает фразу: «{note}»");
}

// ---------- оглавления ----------

fn month_with(dir: &Tmp, month: &str, tickets: &[(i32, &str)]) {
    for (id, created) in tickets {
        put(&dir.join(&format!("{TICKETS_FOLDER}/{month}/{id} — Заявка {id}.md")), &ticket_text(*id, Some(created), "Открыта"));
    }
}

#[test]
fn without_a_tickets_folder_nothing_is_written() {
    let dir = Tmp::new("idx-none");
    assert_eq!(write_indexes(dir.path(), &["2026-09".to_string()]), "");
    assert!(names_in(dir.path()).is_empty());
}

#[test]
fn only_the_named_months_are_rewritten_and_what_is_missing_is_written_anyway() {
    let dir = Tmp::new("idx-touched");
    month_with(&dir, "2026-09", &[(1, "2026-09-10T10:00:00+03:00")]);
    month_with(&dir, "2026-10", &[(2, "2026-10-10T10:00:00+03:00")]);
    let (a, b, root) = (dir.join("tickets/2026-09/_index.md"), dir.join("tickets/2026-10/_index.md"), dir.join("_index.md"));
    assert_eq!(write_indexes(dir.path(), &["2026-09".into(), "2026-10".into()]), "");
    let fresh = regex_lite::Regex::new(
        r"^# Заявки: 2026-09\n\nЗаявок: 1 · обновлено \d\d\.\d\d\.\d{4} \d\d:\d\d\n\n- \[\[1 — Заявка 1\]\] · 10\.09\.2026 · Открыта\n$",
    )
    .unwrap();
    assert!(fresh.is_match(&text(&a)), "{}", text(&a));
    assert!(text(&root).contains("заявок: 2."));

    // названа одна: она переписана (и сама `_index.md` в счёт заявок не идёт), вторая и так оглавлена — не тронута
    fs::write(&a, "СТАРОЕ\n").unwrap();
    fs::write(&b, "МОЁ\n").unwrap();
    fs::write(&root, "КОРЕНЬ\n").unwrap();
    assert_eq!(write_indexes(dir.path(), &["2026-09".into()]), "");
    assert!(fresh.is_match(&text(&a)), "{}", text(&a));
    assert_eq!(text(&b), "МОЁ\n", "месяц, которого не касались, не переписывается");
    assert!(text(&root).starts_with("# Заявки из Интрасервиса\n"), "общее — переписано");

    // ничего не тронуто и всё оглавлено — не переписывается и общее
    fs::write(&root, "КОРЕНЬ\n").unwrap();
    assert_eq!(write_indexes(dir.path(), &[]), "");
    assert_eq!(text(&root), "КОРЕНЬ\n");
    assert_eq!(text(&b), "МОЁ\n");
}

#[test]
fn a_missing_root_index_or_month_index_is_written_even_when_nothing_was_touched() {
    let dir = Tmp::new("idx-missing");
    month_with(&dir, "2026-09", &[(1, "2026-09-10T10:00:00+03:00")]);
    month_with(&dir, "2026-10", &[(2, "2026-10-10T10:00:00+03:00")]);
    assert_eq!(write_indexes(dir.path(), &[]), "");
    assert!(dir.join("tickets/2026-09/_index.md").exists() && dir.join("tickets/2026-10/_index.md").exists(), "месяцы без оглавления");
    assert!(text(dir.join("_index.md")).contains("заявок: 2."), "общее");
    // общего нет, месяцы оглавлены — общее записано
    fs::remove_file(dir.join("_index.md")).unwrap();
    assert_eq!(write_indexes(dir.path(), &[]), "");
    assert!(dir.join("_index.md").exists());
    // оглавления месяца нет — записано, хоть месяц и не назван, а общее — следом
    fs::remove_file(dir.join("tickets/2026-10/_index.md")).unwrap();
    fs::write(dir.join("_index.md"), "КОРЕНЬ\n").unwrap();
    assert_eq!(write_indexes(dir.path(), &[]), "");
    assert!(dir.join("tickets/2026-10/_index.md").exists());
    assert!(text(dir.join("_index.md")).starts_with("# Заявки из"), "общее — следом за месяцем");
}

#[test]
fn a_month_without_tickets_loses_its_index_and_goes_away_unless_it_holds_something_else() {
    let dir = Tmp::new("idx-empty");
    month_with(&dir, "2026-06", &[(1, "2026-06-10T10:00:00+03:00")]);
    put(&dir.join("tickets/2026-08/_index.md"), "старое оглавление\n");
    put(&dir.join("tickets/2026-07/_index.md"), "старое оглавление\n");
    put(&dir.join("tickets/2026-07/заметка.md"), "моя заметка\n");
    assert_eq!(write_indexes(dir.path(), &["2026-08".into(), "2026-07".into(), "2026-06".into()]), "");
    assert!(!dir.join("tickets/2026-08").exists(), "опустевший месяц убран вместе с оглавлением");
    assert!(!dir.join("tickets/2026-07/_index.md").exists());
    assert_eq!(text(dir.join("tickets/2026-07/заметка.md")), "моя заметка\n", "заметка и её папка — на месте");
    let root = text(dir.join("_index.md"));
    assert!(root.contains("- [2026-06](tickets/2026-06/_index.md) — заявок: 1\n") && root.contains("заявок: 1."), "{root}");
    assert!(!root.contains("- [2026-07]") && !root.contains("- [2026-08]"), "в общем — только месяцы с заявками: {root}");
}

#[test]
fn folders_of_the_user_among_the_months_are_neither_indexed_nor_removed() {
    let dir = Tmp::new("idx-user");
    month_with(&dir, "2026-09", &[(1, "2026-09-10T10:00:00+03:00")]);
    put(&dir.join("tickets/свои/10005 — заметка про принтер.md"), "---\ntitle: \"моё\"\n---\n\nтекст\n");
    fs::create_dir(dir.join("tickets/пустая")).unwrap();
    assert_eq!(write_indexes(dir.path(), &["2026-09".into()]), "");
    assert!(!dir.join("tickets/свои/_index.md").exists(), "оглавления в папке пользователя нет");
    assert!(dir.join("tickets/свои/10005 — заметка про принтер.md").exists() && dir.join("tickets/пустая").is_dir());
    let root = text(dir.join("_index.md"));
    assert!(!root.contains("свои") && !root.contains("пустая") && root.contains("заявок: 1."), "{root}");
}

#[test]
fn an_index_that_cannot_be_written_is_the_first_error_and_the_others_are_still_written() {
    let dir = Tmp::new("idx-error");
    month_with(&dir, "2026-09", &[(1, "2026-09-10T10:00:00+03:00")]);
    month_with(&dir, "2026-10", &[(2, "2026-10-10T10:00:00+03:00")]);
    fs::create_dir(dir.join("tickets/2026-09/_index.md")).unwrap();
    let error = write_indexes(dir.path(), &["2026-09".into(), "2026-10".into()]);
    assert!(error.starts_with("Оглавление 2026-09/_index.md не записано: "), "{error}");
    assert!(error.len() > "Оглавление 2026-09/_index.md не записано: ".len(), "за двоеточием — причина: {error}");
    assert!(text(dir.join("tickets/2026-10/_index.md")).starts_with("# Заявки: 2026-10\n"), "соседний месяц записан");
    assert!(text(dir.join("_index.md")).contains("заявок: 2."), "общее — тоже");
    assert!(dir.join("tickets/2026-09/_index.md").is_dir(), "то, что на месте оглавления, цело");
}

#[test]
fn a_root_index_that_cannot_be_written_is_said_and_the_months_are_written() {
    let dir = Tmp::new("idx-root-error");
    month_with(&dir, "2026-09", &[(1, "2026-09-10T10:00:00+03:00")]);
    fs::create_dir(dir.join("_index.md")).unwrap();
    let error = write_indexes(dir.path(), &["2026-09".into()]);
    assert!(error.starts_with("Оглавление _index.md не записано: "), "{error}");
    assert!(dir.join("tickets/2026-09/_index.md").is_file());
}

#[test]
fn the_first_error_is_the_one_of_the_month_before_the_one_of_the_root() {
    let dir = Tmp::new("idx-first");
    month_with(&dir, "2026-09", &[(1, "2026-09-10T10:00:00+03:00")]);
    fs::create_dir(dir.join("tickets/2026-09/_index.md")).unwrap();
    fs::create_dir(dir.join("_index.md")).unwrap();
    let error = write_indexes(dir.path(), &["2026-09".into()]);
    assert!(error.starts_with("Оглавление 2026-09/_index.md не записано: "), "{error}");
}
