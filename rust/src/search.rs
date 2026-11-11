//! Форма поиска без окна (`TicketSearch.cs`): даты полей «с» и «по» → период.

use chrono::{Days, NaiveDate};

use crate::intraservice::query::DateSpan;

/// Число из цифр ASCII ровно такой длины (как точные форматы .NET: ни знака, ни пробела, ни чужих цифр).
fn digits(s: &str, min: usize, max: usize) -> Option<u32> {
    if s.len() < min || s.len() > max || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Дата в одной из записей `DateFormats`: `dd.MM.yyyy`, `d.M.yyyy`, `dd.MM.yy`, `yyyy-MM-dd`. Своя проверка записи, а не
/// `parse_from_str` chrono: он мягче точного разбора .NET (необязательные нули, знаки, пробелы внутри).
fn read_date(text: &str) -> Option<NaiveDate> {
    let (d, m, y) = if let [d, m, y] = text.split('.').collect::<Vec<_>>()[..] {
        let year = if y.len() == 2 {
            // двухзначный год, как в инвариантной культуре .NET: до 49 — 20xx, иначе 19xx (потом не пройдёт диапазон)
            let yy = digits(y, 2, 2)?;
            if d.len() != 2 || m.len() != 2 {
                return None;
            }
            if yy < 50 { 2000 + yy } else { 1900 + yy }
        } else {
            digits(y, 4, 4)?
        };
        (digits(d, 1, 2)?, digits(m, 1, 2)?, year)
    } else if let [y, m, d] = text.split('-').collect::<Vec<_>>()[..] {
        (digits(d, 2, 2)?, digits(m, 2, 2)?, digits(y, 4, 4)?)
    } else {
        return None;
    };
    NaiveDate::from_ymd_opt(y as i32, m, d)
}

/// `TryDate`: пусто — подходит (даты нет); непусто — только если читается и год 2000–2100.
fn try_date(text: &str) -> Option<Option<NaiveDate>> {
    let t = text.trim();
    if t.is_empty() {
        return Some(None);
    }
    let d = read_date(t)?;
    (2000..=2100).contains(&chrono::Datelike::year(&d)).then_some(Some(d))
}

/// Период из двух дат формы (`TicketSearch.ParseSpan`): с начала первого дня до начала дня после последнего; ошибка —
/// текст для окна (проверяется сначала «с», потом «по», потом порядок).
pub fn parse_span(from: &str, to: &str, label: &str) -> Result<DateSpan, String> {
    let bad = |t: &str| format!("{label}: дата «{}» не понятна — нужно дд.мм.гггг, год 2000–2100", t.trim());
    let a = try_date(from).ok_or_else(|| bad(from))?;
    let b = try_date(to).ok_or_else(|| bad(to))?;
    if let (Some(start), Some(end)) = (a, b)
        && start > end
    {
        return Err(format!("{label}: начало периода позже его конца"));
    }
    let midnight = |d: NaiveDate| d.and_hms_opt(0, 0, 0);
    Ok(DateSpan {
        from: a.and_then(midnight),
        // год ≤ 2100, день после него всегда есть
        to: b.and_then(|d| d.checked_add_days(Days::new(1))).and_then(midnight),
    })
}
