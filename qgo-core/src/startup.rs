//! Implements the "Start with Windows" setting.
//!
//! The standalone application registers its own executable under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! Failures are swallowed: not being able to set startup is never fatal.

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The value name written under `Run`, matching the C#'s assembly name.
pub const APP_NAME: &str = "QGo";

/// `StartupManager.SetRunAtStartup(enable)`.
#[cfg(windows)]
pub fn set_run_at_startup(enable: bool) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    if enable {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            let _ = key.set_value(APP_NAME, &format!("\"{}\"", exe.display()));
        }
    } else {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
            // Deleting a value that is not there is not an error here.
            let _ = key.delete_value(APP_NAME);
        }
    }
}

#[cfg(not(windows))]
pub fn set_run_at_startup(_enable: bool) {}

/// Reads back what [`set_run_at_startup`] wrote, so Settings can show the real
/// state rather than trusting `settings.json`.
#[cfg(windows)]
pub fn is_run_at_startup() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(APP_NAME))
        .is_ok()
}

#[cfg(not(windows))]
pub fn is_run_at_startup() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_value_name_matches_the_csharp_assembly_name() {
        assert_eq!(APP_NAME, "QGo");
    }
}
