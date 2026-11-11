//! Таблицы сверки с кодом C# (`tests/parity/*.tsv`): сам читатель таблиц — что ячейки приходят без экранирования,
//! а расхождения называются все и по строкам. Ответы в таблицах пишет и сверяет `TicketBoard.SelfCheck` (область `parity`).

mod common;

#[test]
fn a_table_row_comes_unescaped() {
    let rows = common::parity("search-dates");
    let cells = |first: &str| rows.iter().find(|(_, c)| c[0] == first).map(|(_, c)| c.clone());
    // пробел на краю, табуляция с переводом строки, неразрывный пробел — в файле знаками, здесь — как есть
    assert_eq!(cells(" ").expect("строка с одним пробелом")[3], "OK - -");
    assert!(cells("\t01.02.2026\n").is_some(), "табуляция и перевод строки");
    assert!(cells("\u{A0}01.02.2026\u{A0}").is_some(), "неразрывный пробел");
    // у каждой строки три входа и ответ; номер строки — по файлу (первые три — пояснения)
    assert!(rows.iter().all(|(_, c)| c.len() == 4));
    assert_eq!(rows[0].0, 4);
}

#[test]
fn every_mismatch_is_named() {
    // ответ, который подходит только пустым полям, расходится с C# на всех остальных строках — и каждая названа
    let rows = common::parity("search-dates");
    let empty = rows.iter().filter(|(_, c)| c[3] == "OK - -").count();
    let wrong = common::parity_mismatches("search-dates", |_| "OK - -".into());
    assert!(empty > 0 && wrong.len() == rows.len() - empty, "{} из {}", wrong.len(), rows.len());
    assert!(
        wrong[0].contains("search-dates.tsv, строка 4") && wrong[0].contains("C#:    OK 2026-02-01T00:00:00 2026-03-01T00:00:00"),
        "{}",
        wrong[0]
    );
    // а ответ самой таблицы не расходится нигде
    assert!(
        common::parity_mismatches("search-dates", |inputs| {
            rows.iter().find(|(_, c)| c[..3] == *inputs).map(|(_, c)| c[3].clone()).unwrap_or_default()
        })
        .is_empty()
    );
}
