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

/// Текст — в буфер обмена (`ClipboardWatcher.TrySetText`): «Копировать» в окне поиска. Буфер держит другая программа (менеджер
/// буфера, перенаправление RDP) — до пяти попыток через 40 мс, как у C#; не вышло — false (пусть нажмут ещё раз).
#[cfg(windows)]
pub fn set_text(text: &str) -> bool {
    (0..5).any(|attempt| {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        set_once(text)
    })
}

#[cfg(windows)]
fn set_once(text: &str) -> bool {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: память выделена под units целиком и заполнена до разблокировки; отдана буферу — ею владеет он, не отдана —
    // освобождается здесь; буфер открыт и закрыт здесь же
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let done = (|| {
            EmptyClipboard().ok()?;
            let global = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2).ok()?;
            let ptr = GlobalLock(global) as *mut u16;
            if ptr.is_null() {
                let _ = GlobalFree(Some(global));
                return None;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
            let _ = GlobalUnlock(global);
            if SetClipboardData(CF_UNICODETEXT, Some(HANDLE(global.0))).is_err() {
                let _ = GlobalFree(Some(global));
                return None;
            }
            Some(())
        })();
        let _ = CloseClipboard();
        done.is_some()
    }
}

#[cfg(not(windows))]
pub fn set_text(_text: &str) -> bool {
    false
}
