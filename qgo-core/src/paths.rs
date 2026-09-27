//! Every path the WPF app touches, so both builds share one profile.
//!
//! `Storage.BaseDir` is `%LOCALAPPDATA%\QGo\Data`; log files sit one level up
//! in `%LOCALAPPDATA%\QGo`.

use std::path::PathBuf;

/// `Environment.GetFolderPath(SpecialFolder.LocalApplicationData)`.
pub fn local_app_data() -> PathBuf {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile).join("AppData").join("Local");
    }
    // Non-Windows fallback so `cargo test` works anywhere.
    std::env::temp_dir().join("QGo-LocalAppData")
}

/// `%LOCALAPPDATA%\QGo`
pub fn qgo_dir() -> PathBuf {
    local_app_data().join("QGo")
}

/// `Storage.StorageDirectory` — `%LOCALAPPDATA%\QGo\Data`
pub fn data_dir() -> PathBuf {
    qgo_dir().join("Data")
}

pub fn links_path() -> PathBuf {
    data_dir().join("links.json")
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

pub fn chained_shortcuts_path() -> PathBuf {
    data_dir().join("chained-shortcuts.json")
}

/// Written by `LocalUsageStatsService`.
pub fn usage_stats_path() -> PathBuf {
    data_dir().join("usage-stats.json")
}

pub fn usage_stats_backup_path() -> PathBuf {
    data_dir().join("usage-stats.json.backup")
}

/// `%LOCALAPPDATA%\QGo\logs`
pub fn logs_dir() -> PathBuf {
    qgo_dir().join("logs")
}

/// Creates the data directory. Mirrors `Storage.EnsureDir`.
pub fn ensure_data_dir() -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_files_all_live_under_the_data_directory() {
        let data = data_dir();
        assert!(links_path().starts_with(&data));
        assert!(settings_path().starts_with(&data));
        assert!(chained_shortcuts_path().starts_with(&data));
        assert!(usage_stats_path().starts_with(&data));
        assert_eq!(data.file_name().unwrap(), "Data");
    }
}
