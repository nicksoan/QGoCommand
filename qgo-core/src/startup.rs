//! Implements the "Start with Windows" setting.
//!
//! Installed MSIX builds use the package's StartupTask so Windows can expose
//! and enforce the state in Settings and Task Manager. Unpackaged development
//! builds retain the per-user Run-key fallback.

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const APP_NAME: &str = "QGo";
pub const TASK_ID: &str = "QGoStartup";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupState {
    Enabled,
    Disabled,
    DisabledByUser,
    DisabledByPolicy,
    EnabledByPolicy,
}

impl StartupState {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::EnabledByPolicy)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("Windows startup registration failed: {0}")]
    Windows(String),
    #[error("the startup registry entry could not be updated: {0}")]
    Registry(String),
}

#[cfg(windows)]
fn is_packaged() -> bool {
    windows::ApplicationModel::Package::Current().is_ok()
}

#[cfg(windows)]
fn remove_legacy_run_entry() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE) {
        let _ = key.delete_value(APP_NAME);
    }
}

#[cfg(windows)]
fn legacy_run_enabled() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(APP_NAME))
        .is_ok()
}

#[cfg(windows)]
fn packaged_task() -> Result<windows::ApplicationModel::StartupTask, StartupError> {
    use windows::core::HSTRING;
    use windows::ApplicationModel::StartupTask;

    StartupTask::GetAsync(&HSTRING::from(TASK_ID))
        .and_then(|operation| operation.join())
        .map_err(|error| StartupError::Windows(error.to_string()))
}

#[cfg(windows)]
fn packaged_state(
    state: windows::ApplicationModel::StartupTaskState,
) -> Result<StartupState, StartupError> {
    use windows::ApplicationModel::StartupTaskState;

    match state {
        StartupTaskState::Enabled => Ok(StartupState::Enabled),
        StartupTaskState::Disabled => Ok(StartupState::Disabled),
        StartupTaskState::DisabledByUser => Ok(StartupState::DisabledByUser),
        StartupTaskState::DisabledByPolicy => Ok(StartupState::DisabledByPolicy),
        StartupTaskState::EnabledByPolicy => Ok(StartupState::EnabledByPolicy),
        other => Err(StartupError::Windows(format!(
            "Windows returned unknown startup state {}",
            other.0
        ))),
    }
}

#[cfg(windows)]
fn set_packaged_startup(enable: bool) -> Result<StartupState, StartupError> {
    use windows::ApplicationModel::StartupTaskState;

    let task = packaged_task()?;
    let current = task
        .State()
        .map_err(|error| StartupError::Windows(error.to_string()))?;

    if enable && current == StartupTaskState::Disabled {
        let state = task
            .RequestEnableAsync()
            .and_then(|operation| operation.join())
            .map_err(|error| StartupError::Windows(error.to_string()))
            .and_then(packaged_state)?;
        remove_legacy_run_entry();
        return Ok(state);
    }

    if !enable && current == StartupTaskState::Enabled {
        task.Disable()
            .map_err(|error| StartupError::Windows(error.to_string()))?;
    }

    let state = task
        .State()
        .map_err(|error| StartupError::Windows(error.to_string()))
        .and_then(packaged_state)?;
    remove_legacy_run_entry();
    Ok(state)
}

#[cfg(windows)]
fn registry_state() -> Result<StartupState, StartupError> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let enabled = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(APP_NAME))
        .is_ok();
    Ok(if enabled {
        StartupState::Enabled
    } else {
        StartupState::Disabled
    })
}

#[cfg(windows)]
fn set_registry_startup(enable: bool) -> Result<StartupState, StartupError> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .map_err(|error| StartupError::Registry(error.to_string()))?;

    if enable {
        let exe =
            std::env::current_exe().map_err(|error| StartupError::Registry(error.to_string()))?;
        key.set_value(APP_NAME, &format!("\"{}\"", exe.display()))
            .map_err(|error| StartupError::Registry(error.to_string()))?;
    } else {
        match key.delete_value(APP_NAME) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StartupError::Registry(error.to_string())),
        }
    }

    registry_state()
}

#[cfg(windows)]
pub fn set_run_at_startup(enable: bool) -> Result<StartupState, StartupError> {
    if is_packaged() {
        set_packaged_startup(enable)
    } else {
        set_registry_startup(enable)
    }
}

#[cfg(not(windows))]
pub fn set_run_at_startup(_enable: bool) -> Result<StartupState, StartupError> {
    Ok(StartupState::Disabled)
}

#[cfg(windows)]
pub fn startup_state() -> Result<StartupState, StartupError> {
    if is_packaged() {
        use windows::ApplicationModel::StartupTaskState;

        let migrate_enabled = legacy_run_enabled();
        let task = packaged_task()?;
        let mut state = task
            .State()
            .map_err(|error| StartupError::Windows(error.to_string()))?;
        if migrate_enabled && state == StartupTaskState::Disabled {
            state = task
                .RequestEnableAsync()
                .and_then(|operation| operation.join())
                .map_err(|error| StartupError::Windows(error.to_string()))?;
        }
        let state = packaged_state(state)?;
        remove_legacy_run_entry();
        Ok(state)
    } else {
        registry_state()
    }
}

#[cfg(not(windows))]
pub fn startup_state() -> Result<StartupState, StartupError> {
    Ok(StartupState::Disabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_stable() {
        assert_eq!(APP_NAME, "QGo");
        assert_eq!(TASK_ID, "QGoStartup");
    }

    #[test]
    fn policy_enabled_counts_as_enabled() {
        assert!(StartupState::Enabled.is_enabled());
        assert!(StartupState::EnabledByPolicy.is_enabled());
        assert!(!StartupState::DisabledByUser.is_enabled());
    }
}
