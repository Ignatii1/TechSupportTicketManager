//! Хозяин доски один: тот же именованный мьютекс, что у TicketBoard.exe (`Local\TicketBoard.SingleInstance`). Взяли —
//! доска наша, TicketBoard.exe, запущенный следом, молча выходит (так он поступает со вторым собой); не взяли — он уже
//! работает, и доска только читается: два писателя одного `tickets.json` затирали бы правки друг друга.

pub const NAME: &str = r"Local\TicketBoard.SingleInstance";

/// Пока жив — доска наша.
pub struct Owner {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for Owner {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::ReleaseMutex;
        // SAFETY: свой мьютекс, взятый этим потоком в `take`
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Взять доску; None — её держит другой (TicketBoard.exe или второй прототип).
#[cfg(windows)]
pub fn take() -> Option<Owner> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::HSTRING;
    // SAFETY: имя — строка; ответ проверяется
    let handle = unsafe { CreateMutexW(None, true, &HSTRING::from(NAME)) }.ok()?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: свой дескриптор чужого мьютекса — закрыть
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(Owner { handle })
}

/// Вне Windows TicketBoard.exe не бывает — доска всегда наша (проверки пишут во временную папку).
#[cfg(not(windows))]
pub fn take() -> Option<Owner> {
    Some(Owner {})
}
