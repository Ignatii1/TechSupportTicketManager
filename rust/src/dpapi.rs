//! Пароль Интрасервиса в `settings.json` зашифрован DPAPI под пользователя Windows (как `ProtectedData.Protect` с
//! `CurrentUser` и без энтропии) и записан base64. Расшифровать его может только тот же пользователь на том же ПК.

use base64::Engine;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Расшифровать сохранённый пароль; пустой — пустой (как `AppSettings.Unprotect`).
pub fn unprotect(stored: &str) -> Result<String, String> {
    if stored.is_empty() {
        return Ok(String::new());
    }
    let blob = B64.decode(stored.trim()).map_err(|e| format!("пароль в settings.json — не base64: {e}"))?;
    let plain = native::unprotect(&blob)?;
    String::from_utf8(plain).map_err(|_| "пароль расшифрован, но это не UTF-8".to_string())
}

/// Зашифровать — как это делает TicketBoard.exe (для проверки «туда и обратно»).
#[cfg(windows)]
pub fn protect(plain: &str) -> Result<String, String> {
    Ok(B64.encode(native::protect(plain.as_bytes())?))
}

#[cfg(windows)]
mod native {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI вернул буфер out.cbData байт, выделенный LocalAlloc; копируем и освобождаем
        unsafe {
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            LocalFree(Some(HLOCAL(out.pbData.cast())));
            bytes
        }
    }

    pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        let input = CRYPT_INTEGER_BLOB { cbData: blob.len() as u32, pbData: blob.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: входной буфер жив до конца вызова, выход освобождает take
        unsafe { CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| format!("пароль не расшифровался (DPAPI): {e} — он зашифрован другим пользователем Windows или на другом ПК?"))?;
        Ok(take(out))
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: как выше
        unsafe { CryptProtectData(&input, windows::core::PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| format!("DPAPI: {e}"))?;
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod native {
    pub fn unprotect(_blob: &[u8]) -> Result<Vec<u8>, String> {
        Err("пароль расшифровывается только в Windows (DPAPI)".into())
    }
}
