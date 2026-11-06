//! Хозяин доски один: тот же мьютекс, что у TicketBoard.exe. На Windows (CI): второй захват — «занят», после
//! освобождения — снова свободен. Вне Windows TicketBoard.exe не бывает — доска всегда своя.

use ticketboard::single::{NAME, take};

#[test]
fn one_owner_at_a_time() {
    assert_eq!(NAME, r"Local\TicketBoard.SingleInstance", "имя — как у C# App.OnStartup");
    let first = take();
    assert!(first.is_some(), "свободен — наш");
    #[cfg(windows)]
    {
        assert!(take().is_none(), "занят — только чтение");
        drop(first);
        assert!(take().is_some(), "освободили — снова наш");
    }
    #[cfg(not(windows))]
    assert!(take().is_some());
}
