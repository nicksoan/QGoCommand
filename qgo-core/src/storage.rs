//! Port of `QGo.App.Storage` — JSON files under `%LOCALAPPDATA%\QGo\Data`.
//!
//! Output is pretty-printed with two-space indentation to match
//! `JsonSerializerOptions { WriteIndented = true }`, and the same de-duplication
//! and legacy-format fallbacks are applied on the way in and out, so the WPF app
//! and this port can be run against one profile interchangeably.

use crate::models::{AppSettings, ChainedShortcut, Shortcut};
use crate::paths;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// `JsonSerializer.Serialize(value, Opt)` — indented, named float literals.
fn to_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}

fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Write beside the target then rename, so an interrupted save cannot leave a
    // truncated links.json behind. The C# writes in place; this is strictly safer
    // and produces the same end state.
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

/// `Storage.SettingsFileExists()` — used to detect a first run.
pub fn settings_file_exists() -> bool {
    paths::settings_path().exists()
}

/// The five shortcuts a brand-new profile starts with, verbatim from
/// `Storage.LoadLinksOrDefaults`.
pub fn default_shortcuts() -> Vec<Shortcut> {
    vec![
        Shortcut::new("google", "https://www.google.com/search?q={keyword}"),
        Shortcut::new("documents", "%HOMEPATH%"),
        Shortcut::new(
            "youtube",
            "https://www.youtube.com/results?search_query={search}",
        ),
        Shortcut::new("qgo", "https://qgocommand.com/"),
        Shortcut::new("notepad", "notepad.exe"),
    ]
}

/// De-duplicates by key, case-insensitively, keeping the **last** occurrence and
/// dropping blank keys — `Storage.SaveLinks` / `SaveChainedShortcuts`.
///
/// `GroupBy` preserves the order in which groups are first seen, so a shortcut
/// that is overwritten keeps its original position in the file.
fn dedupe_by_key<T, F>(items: Vec<T>, key_of: F) -> Vec<T>
where
    F: Fn(&T) -> String,
{
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, T> = HashMap::new();

    for item in items {
        let key = key_of(&item);
        if key.trim().is_empty() {
            continue;
        }
        let lookup = key.to_lowercase();
        if !latest.contains_key(&lookup) {
            order.push(lookup.clone());
        }
        latest.insert(lookup, item);
    }

    order
        .into_iter()
        .filter_map(|k| latest.remove(&k))
        .collect()
}

/// `Storage.LoadLinksOrDefaults()`.
///
/// Falls back to the pre-2.0 `Dictionary<string, string>` format and rewrites
/// the file in the current shape, then to the built-in defaults.
pub fn load_links_or_defaults() -> Vec<Shortcut> {
    let _ = paths::ensure_data_dir();
    let path = paths::links_path();

    if path.exists() {
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(shortcuts) = serde_json::from_str::<Vec<Shortcut>>(&text) {
                return shortcuts;
            }
            // Legacy format: a flat { "key": "template" } map.
            if let Ok(legacy) = serde_json::from_str::<HashMap<String, String>>(&text) {
                let mut shortcuts: Vec<Shortcut> = legacy
                    .into_iter()
                    .map(|(k, v)| Shortcut::new(k, v))
                    .collect();
                shortcuts.sort_by(|a, b| a.key.cmp(&b.key));
                let _ = save_links(&shortcuts);
                return shortcuts;
            }
            log::warn!("links.json could not be parsed; falling back to defaults");
        }
    }

    let defaults = default_shortcuts();
    let _ = save_links(&defaults);
    defaults
}

/// `Storage.SaveLinks(items)`.
pub fn save_links(items: &[Shortcut]) -> std::io::Result<()> {
    let shortcuts = dedupe_by_key(items.to_vec(), |s| s.key.clone());
    let json = to_json(&shortcuts).map_err(std::io::Error::other)?;
    write_atomic(&paths::links_path(), &json)
}

/// `Storage.LoadChainedShortcuts()` — an unreadable file yields an empty list
/// rather than an error, matching the C#.
pub fn load_chained_shortcuts() -> Vec<ChainedShortcut> {
    let _ = paths::ensure_data_dir();
    let path = paths::chained_shortcuts_path();

    if !path.exists() {
        log::debug!("chained-shortcuts.json does not exist yet");
        return Vec::new();
    }

    match fs::read_to_string(&path).map(|t| serde_json::from_str::<Vec<ChainedShortcut>>(&t)) {
        Ok(Ok(chains)) => {
            log::debug!("loaded {} chained shortcuts", chains.len());
            chains
        }
        Ok(Err(e)) => {
            log::warn!("chained-shortcuts.json deserialisation failed: {e}");
            Vec::new()
        }
        Err(e) => {
            log::warn!("chained-shortcuts.json could not be read: {e}");
            Vec::new()
        }
    }
}

/// `Storage.SaveChainedShortcuts(chains)`.
pub fn save_chained_shortcuts(chains: &[ChainedShortcut]) -> std::io::Result<()> {
    let chains = dedupe_by_key(chains.to_vec(), |c| c.key.clone());
    let json = to_json(&chains).map_err(std::io::Error::other)?;
    write_atomic(&paths::chained_shortcuts_path(), &json)
}

/// `Storage.LoadSettings()`.
///
/// Two behaviours are load-bearing and reproduced exactly:
///
/// * if the file is missing any key the current build knows about, the merged
///   settings are written straight back, so new defaults land on disk;
/// * if the file is corrupt or locked, in-memory defaults are returned **without
///   saving** so the existing file is not destroyed.
pub fn load_settings() -> AppSettings {
    let _ = paths::ensure_data_dir();
    let path = paths::settings_path();

    if path.exists() {
        let Ok(text) = fs::read_to_string(&path) else {
            return AppSettings::default();
        };

        let parsed: Result<Value, _> = serde_json::from_str(&text);
        let mut settings: AppSettings = serde_json::from_str(&text).unwrap_or_default();
        let removed_obsolete_fields = settings.remove_obsolete_fields();

        if let Ok(Value::Object(on_disk)) = parsed {
            if removed_obsolete_fields || is_missing_any_key(&settings, &on_disk) {
                let _ = save_settings(&settings);
            }
            return settings;
        }

        // Corrupt file: return defaults but leave the file alone.
        return AppSettings::default();
    }

    let defaults = AppSettings::default();
    let _ = save_settings(&defaults);
    defaults
}

/// Stands in for the C#'s reflection over `typeof(AppSettings).GetProperties()`:
/// serialise the merged settings and check every key it produces is present on
/// disk. Equivalent, and it cannot drift out of sync with the struct.
fn is_missing_any_key(settings: &AppSettings, on_disk: &Map<String, Value>) -> bool {
    match serde_json::to_value(settings) {
        Ok(Value::Object(expected)) => expected.keys().any(|k| !on_disk.contains_key(k)),
        _ => false,
    }
}

/// `Storage.SaveSettings(s)`.
pub fn save_settings(settings: &AppSettings) -> std::io::Result<()> {
    let json = to_json(settings).map_err(std::io::Error::other)?;
    write_atomic(&paths::settings_path(), &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_keys_collapse_to_the_last_occurrence_in_first_seen_order() {
        let items = vec![
            Shortcut::new("a", "first"),
            Shortcut::new("b", "b1"),
            Shortcut::new("A", "second"), // same key, different case
            Shortcut::new("  ", "blank"), // dropped
        ];
        let out = dedupe_by_key(items, |s| s.key.clone());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].template, "second");
        assert_eq!(out[0].key, "A");
        assert_eq!(out[1].key, "b");
    }

    #[test]
    fn defaults_match_the_csharp_seed_list() {
        let d = default_shortcuts();
        let keys: Vec<&str> = d.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["google", "documents", "youtube", "qgo", "notepad"]);
        assert_eq!(d[1].template, "%HOMEPATH%");
    }

    #[test]
    fn a_settings_file_missing_new_keys_is_detected() {
        let settings = AppSettings::default();
        let sparse: Map<String, Value> =
            serde_json::from_str(r#"{"HotkeyGesture":"Alt+Q"}"#).unwrap();
        assert!(is_missing_any_key(&settings, &sparse));

        let complete = match serde_json::to_value(&settings).unwrap() {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        assert!(!is_missing_any_key(&settings, &complete));
    }

    #[test]
    fn output_is_indented_like_write_indented() {
        let json = to_json(&vec![Shortcut::new("k", "t")]).unwrap();
        assert!(json.contains("\n  {\n"), "{json}");
    }

    #[test]
    fn links_round_trip_through_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("links.json");
        let original = vec![Shortcut::new("qgo", "https://qgocommand.com/")];
        write_atomic(&path, &to_json(&original).unwrap()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let back: Vec<Shortcut> = serde_json::from_str(&text).unwrap();
        assert_eq!(back, original);
    }
}
