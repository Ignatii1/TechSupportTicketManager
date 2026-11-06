//! Пароль из `settings.json`: DPAPI CurrentUser без энтропии, base64 — как `AppSettings.Protect`.

#[cfg(windows)]
#[test]
fn round_trip_on_windows() {
    let stored = ticketboard::dpapi::protect("пароль 123").unwrap();
    assert_ne!(stored, "пароль 123");
    assert_eq!(ticketboard::dpapi::unprotect(&stored).as_deref(), Ok("пароль 123"));
    assert!(ticketboard::dpapi::unprotect("не base64!").is_err());
    assert!(ticketboard::dpapi::unprotect("AAAA").is_err(), "не DPAPI — ошибка, не паника");
    assert_eq!(ticketboard::dpapi::unprotect("").as_deref(), Ok(""), "пустой — пустой, как у C#");
}

#[cfg(not(windows))]
#[test]
fn says_windows_only_elsewhere() {
    let err = ticketboard::dpapi::unprotect("AQAAANCMnd8=").unwrap_err();
    assert!(err.contains("Windows"), "{err}");
}
