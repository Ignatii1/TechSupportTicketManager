//! Цифры прототипа: размер exe, память процесса, время от запуска процесса. Ради них прототип и сделан.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    /// Рабочий набор: сколько сейчас в физической памяти (Диспетчер задач, «Подробности» → «Рабочий набор»).
    pub working: u64,
    /// Частная: только своё, без общих библиотек (Диспетчер задач, колонка «Память» — близко к ней).
    pub private: u64,
}

pub fn exe_size() -> Option<u64> {
    std::fs::metadata(std::env::current_exe().ok()?).ok().map(|m| m.len())
}

/// «65,6 МБ» — мегабайты по 10⁶, как размер выпуска на GitHub.
pub fn megabytes(bytes: u64) -> String {
    let tenths = (bytes + 50_000) / 100_000;
    format!("{},{} МБ", tenths / 10, tenths % 10)
}

#[cfg(windows)]
pub fn memory() -> Option<Memory> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    // SAFETY: структура EX начинается с PROCESS_MEMORY_COUNTERS, размер передаётся
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
    }
    .ok()?;
    Some(Memory { working: c.WorkingSetSize as u64, private: c.PrivateUsage as u64 })
}

/// На Linux — для проверок: VmRSS и RssAnon из /proc.
#[cfg(not(windows))]
pub fn memory() -> Option<Memory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = |name: &str| -> Option<u64> {
        let line = status.lines().find(|l| l.starts_with(name))?;
        line[name.len()..].trim().trim_end_matches("kB").trim().parse::<u64>().ok().map(|k| k * 1024)
    };
    Some(Memory { working: kb("VmRSS:")?, private: kb("RssAnon:")? })
}

/// «окно на экране через 4141 мс (до main 3800, доска и окна 250, …, первый кадр 41)» — куда ушло время запуска. `marks` —
/// отметки этапов от создания процесса, по порядку; этап — от прежней отметки до своей, «первый кадр» — от последней до
/// окна на экране; отметка раньше прежней (часы шагнули) — ноль. Отметок нет — один итог.
pub fn startup_text(shown: Duration, marks: &[(&str, Duration)]) -> String {
    let total = format!("окно на экране через {} мс", shown.as_millis());
    let Some(&(_, last)) = marks.last() else { return total };
    let mut before = Duration::ZERO;
    let mut stages: Vec<String> = marks
        .iter()
        .map(|&(name, at)| {
            let stage = at.saturating_sub(before);
            before = at;
            format!("{name} {}", stage.as_millis())
        })
        .collect();
    stages.push(format!("первый кадр {}", shown.saturating_sub(last).as_millis()));
    format!("{total} ({})", stages.join(", "))
}

/// Сколько прошло с создания процесса (не с `main`: загрузка exe и библиотек — тоже запуск).
#[cfg(windows)]
pub fn since_start() -> Option<Duration> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let ticks = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: четыре выходных FILETIME
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    let now = unsafe { GetSystemTimeAsFileTime() };
    Some(Duration::from_nanos(ticks(now).checked_sub(ticks(created))? * 100))
}

/// На Linux — для проверок: время старта процесса из /proc/self/stat (в тиках от загрузки) и /proc/uptime.
#[cfg(not(windows))]
pub fn since_start() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after_name = &stat[stat.rfind(')')? + 2..];
    let start_ticks: f64 = after_name.split_whitespace().nth(19)?.parse().ok()?; // поле 22 — starttime
    let uptime: f64 = std::fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
    Some(Duration::from_secs_f64((uptime - start_ticks / 100.0).max(0.0)))
}
