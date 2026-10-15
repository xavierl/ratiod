//! Language of the native Windows dialogs (the web UI has its own switch): French when the Windows
//! UI language is French, English otherwise.

/// True when `langid` (a Windows LANGID) is French, whatever the sublanguage (France, Canada, Belgium...).
pub fn is_french(langid: u16) -> bool {
    const LANG_FRENCH: u16 = 0x0c;
    langid & 0x3ff == LANG_FRENCH
}

/// Picks `french` or `english` according to the Windows UI language (English off Windows).
pub fn pick(french: impl Into<String>, english: impl Into<String>) -> String {
    if ui_is_french() { french.into() } else { english.into() }
}

#[cfg(windows)]
fn ui_is_french() -> bool {
    // SAFETY: plain Win32 call without arguments.
    is_french(unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() })
}

#[cfg(not(windows))]
fn ui_is_french() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::is_french;

    #[test]
    fn french_is_detected_by_primary_language() {
        assert!(is_french(0x040c), "fr-FR");
        assert!(is_french(0x0c0c), "fr-CA");
        assert!(is_french(0x080c), "fr-BE");
        assert!(!is_french(0x0409), "en-US");
        assert!(!is_french(0x0407), "de-DE");
        assert!(!is_french(0));
    }
}
