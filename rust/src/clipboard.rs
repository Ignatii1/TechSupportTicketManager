//! Текст из буфера обмена — для быстрого добавления (скопировал ссылку в браузере → клавиша → Enter). Через уже
//! подключённый `windows`, без своей библиотеки. Буфер занят или в нём не текст — None.

#[cfg(windows)]
pub fn text() -> Option<String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    // SAFETY: буфер открыт и закрыт здесь же; данные читаются не дальше их размера и до первого нуля
    unsafe {
        OpenClipboard(None).ok()?;
        let text = (|| {
            let data: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            let global = HGLOBAL(data.0);
            let ptr = GlobalLock(global) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let units = GlobalSize(global) / 2;
            let slice = std::slice::from_raw_parts(ptr, units);
            let len = slice.iter().position(|&c| c == 0).unwrap_or(units);
            let s = String::from_utf16_lossy(&slice[..len]);
            let _ = GlobalUnlock(global);
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}

#[cfg(not(windows))]
pub fn text() -> Option<String> {
    None
}
