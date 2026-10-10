use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{
        HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RRF_ZEROONFAILURE, RegGetValueW,
    },
};

/// Reads the current user's application theme with one fixed-size, read-only query.
/// Missing, malformed or inaccessible preferences leave the system theme unknown.
pub fn system_dark() -> Option<bool> {
    let subkey: Vec<u16> = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<u16> = "AppsUseLightTheme".encode_utf16().chain(Some(0)).collect();
    let mut kind = 0;
    let mut value = 0_u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the UTF-16 strings are terminated and remain alive for the call;
    // the writable DWORD buffer has exactly the byte length supplied in size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD | RRF_ZEROONFAILURE,
            &mut kind,
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    decode_preference(status, kind, size, value)
}

pub fn resolve(theme: &str, dark: Option<bool>) -> &'static str {
    match (theme, dark) {
        ("dark", _) | ("system", Some(true)) => "dark",
        _ => "light",
    }
}

fn decode_preference(status: u32, kind: u32, size: u32, value: u32) -> Option<bool> {
    if status != ERROR_SUCCESS || kind != REG_DWORD || size != size_of::<u32>() as u32 {
        return None;
    }
    match value {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_theme_overrides_system_preference() {
        for dark in [None, Some(false), Some(true)] {
            assert_eq!(resolve("light", dark), "light");
            assert_eq!(resolve("dark", dark), "dark");
        }
    }

    #[test]
    fn system_theme_follows_dark_preference_and_defaults_to_light() {
        assert_eq!(resolve("system", Some(true)), "dark");
        assert_eq!(resolve("system", Some(false)), "light");
        assert_eq!(resolve("system", None), "light");
        assert_eq!(resolve("unexpected", Some(true)), "light");
    }

    #[test]
    fn only_successful_exact_boolean_dwords_are_preferences() {
        // ERROR_SUCCESS, REG_DWORD and its exact byte length.
        assert_eq!(decode_preference(0, 4, 4, 0), Some(true));
        assert_eq!(decode_preference(0, 4, 4, 1), Some(false));
        for (status, kind, size, value) in [
            (2, 4, 4, 0),
            (5, 4, 4, 1),
            (234, 4, 8, 0),
            (0, 1, 4, 0),
            (0, 11, 4, 1),
            (0, 4, 0, 0),
            (0, 4, 3, 1),
            (0, 4, 8, 0),
            (0, 4, 4, 2),
            (0, 4, 4, u32::MAX),
        ] {
            assert_eq!(decode_preference(status, kind, size, value), None);
        }
    }
}
