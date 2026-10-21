//! Every path the WPF app touches, so both builds share one profile.
//!
//! `Storage.BaseDir` is `%USERPROFILE%\AppData\Local\QGo\Data`; log files sit
//! one level up in `%USERPROFILE%\AppData\Local\QGo`.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// The normal, user-visible local application data directory.
pub fn local_app_data() -> PathBuf {
    resolve_local_app_data(
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )
}

fn resolve_local_app_data(
    local_app_data: Option<PathBuf>,
    user_profile: Option<PathBuf>,
) -> PathBuf {
    if let Some(dir) = local_app_data.as_ref().filter(|dir| !is_package_path(dir)) {
        return dir.clone();
    }
    if let Some(profile) = user_profile.filter(|profile| !profile.as_os_str().is_empty()) {
        return profile.join("AppData").join("Local");
    }
    if let Some(dir) = local_app_data {
        return dir;
    }
    // Non-Windows fallback so `cargo test` works anywhere.
    std::env::temp_dir().join("QGo-LocalAppData")
}

fn is_package_path(path: &Path) -> bool {
    path.to_string_lossy()
        .split(['\\', '/'])
        .any(|part| part.eq_ignore_ascii_case("Packages"))
}

fn legacy_data_dir() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    is_package_path(&local_app_data).then(|| local_app_data.join("QGo").join("Data"))
}

/// `%USERPROFILE%\AppData\Local\QGo`
pub fn qgo_dir() -> PathBuf {
    local_app_data().join("QGo")
}

/// `Storage.StorageDirectory` — `%USERPROFILE%\AppData\Local\QGo\Data`
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

/// Trust decisions for executable shortcuts. Kept separate from the shared
/// profile so older QGo builds cannot accidentally erase them.
pub fn trusted_launches_path() -> PathBuf {
    data_dir().join("trusted-launches.json")
}

/// `%USERPROFILE%\AppData\Local\QGo\logs`
pub fn logs_dir() -> PathBuf {
    qgo_dir().join("logs")
}

/// Creates the data directory and recovers files from the previous MSIX path.
pub fn ensure_data_dir() -> std::io::Result<()> {
    let destination = data_dir();
    fs::create_dir_all(&destination)?;

    if let Some(source) = legacy_data_dir().filter(|source| source != &destination) {
        copy_missing_files(&source, &destination);
    }
    Ok(())
}

fn copy_missing_files(source: &Path, destination: &Path) {
    let entries = match fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            log::warn!("could not read previous QGo data directory: {error}");
            return;
        }
    };

    for entry in entries.flatten() {
        let source_file = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let destination_file = destination.join(entry.file_name());
        if destination_file.exists() {
            continue;
        }

        let temporary_file = destination_file.with_extension("migration-tmp");
        if let Err(error) = fs::copy(&source_file, &temporary_file)
            .and_then(|_| fs::rename(&temporary_file, &destination_file))
        {
            let _ = fs::remove_file(&temporary_file);
            log::warn!(
                "could not migrate {} to {}: {error}",
                source_file.display(),
                destination_file.display()
            );
        }
    }
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
        assert!(trusted_launches_path().starts_with(&data));
        assert_eq!(data.file_name().unwrap(), "Data");
    }

    #[test]
    fn package_local_app_data_resolves_to_the_normal_user_folder() {
        let package_local =
            PathBuf::from(r"C:\Users\user\AppData\Local\Packages\QGo\LocalCache\Local");
        let profile = PathBuf::from(r"C:\Users\user");
        let expected = profile.join("AppData").join("Local");
        assert_eq!(
            resolve_local_app_data(Some(package_local), Some(profile)),
            expected
        );
    }

    #[test]
    fn a_custom_local_app_data_path_is_preserved() {
        let scratch = PathBuf::from(r"C:\Temp\qgo-scratch");
        let profile = PathBuf::from(r"C:\Users\user");
        assert_eq!(
            resolve_local_app_data(Some(scratch.clone()), Some(profile)),
            scratch
        );
    }

    #[test]
    fn migration_copies_missing_files_without_overwriting_existing_files() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::write(source.path().join("links.json"), "old links").unwrap();
        fs::write(source.path().join("settings.json"), "old settings").unwrap();
        fs::write(destination.path().join("links.json"), "new links").unwrap();

        copy_missing_files(source.path(), destination.path());

        assert_eq!(
            fs::read_to_string(destination.path().join("links.json")).unwrap(),
            "new links"
        );
        assert_eq!(
            fs::read_to_string(destination.path().join("settings.json")).unwrap(),
            "old settings"
        );
    }
}
