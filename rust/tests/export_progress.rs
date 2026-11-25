//! Ход выгрузки: скорость, срок для человека и строка хода (`RateMeter`, `HumanSpan`, `ProgressLine` у C#; карточка 4.2).
//! Ответы в таблицах `tests/parity/export-{rate,span,progress}.tsv` пишет код C#; здесь они сверяются со строками на каждой.

mod common;

use std::cell::Cell;
use std::time::Duration;
use ticketboard::export::run::{RateMeter, human_span, progress_line};

#[test]
fn the_rate_meter_answers_like_csharp_on_every_row() {
    // один замерщик на строку, замеры по порядку; ответ сравнивается как число, до последнего бита, а не как текст
    let mut wrong = Vec::new();
    for (line, cells) in common::parity("export-rate") {
        let (inputs, expected) = (&cells[0], &cells[1]);
        let mut meter = RateMeter::default();
        let got: Vec<f64> = inputs
            .split(' ')
            .map(|pair| {
                let (now, count) = pair.split_once(':').expect("«мс:заявок»");
                meter.per_second(now.parse().unwrap(), count.parse().unwrap())
            })
            .collect();
        let want: Vec<f64> = expected.split(' ').map(|v| v.parse().unwrap()).collect();
        if got != want {
            wrong.push(format!("export-rate.tsv, строка {line}: замеры «{inputs}»\n   здесь: {got:?}\n   C#:    {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "скорость расходится с C#:\n{}", wrong.join("\n"));
}

#[test]
fn the_time_span_reads_like_csharp_on_every_row() {
    let wrong = common::parity_mismatches("export-span", |c| human_span(Duration::from_secs_f64(c[0].parse().unwrap())));
    assert!(wrong.is_empty(), "срок расходится с C#:\n{}", wrong.join("\n"));
}

#[test]
fn the_progress_line_reads_like_csharp_on_every_row() {
    // ответ — «по скольким спросили скорость|строка»; «-» — скорость не спрашивали
    let wrong = common::parity_mismatches("export-progress", |c| {
        let n = |i: usize| c[i].parse::<usize>().unwrap();
        let asked = Cell::new(None);
        let rate: f64 = c[4].parse().unwrap();
        let line = progress_line(n(0), n(1), n(2), n(3), 0, |count| {
            asked.set(Some(count));
            rate
        });
        format!("{}|{line}", asked.get().map_or("-".to_string(), |count| count.to_string()))
    });
    assert!(wrong.is_empty(), "строка хода расходится с C#:\n{}", wrong.join("\n"));
}
