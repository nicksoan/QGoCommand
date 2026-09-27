//! Read-only compatibility check against a real QGo profile written by the WPF
//! app: `%LOCALAPPDATA%\QGo\Data\*.json`.
//!
//! Ignored by default because it depends on the machine having used QGo. Run it
//! on a box with a real profile after touching anything in `serde_cs` or
//! `models`:
//!
//! ```text
//! cargo test -p qgo-core --test live_profile -- --ignored --nocapture
//! ```
//!
//! Nothing here writes: the point is to prove the port parses what the C#
//! produced, and that re-serialising it round-trips.

use qgo_core::models::{AppSettings, ChainedShortcut, Shortcut, UsageStatistics};
use qgo_core::paths;

fn read(path: std::path::PathBuf) -> Option<String> {
    if !path.exists() {
        eprintln!("skipping {}: not present", path.display());
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[test]
#[ignore = "requires a real QGo profile written by the WPF app"]
fn parses_the_live_links_file() {
    let Some(text) = read(paths::links_path()) else {
        return;
    };
    let shortcuts: Vec<Shortcut> =
        serde_json::from_str(&text).expect("links.json should deserialise");
    println!("links.json: {} shortcuts", shortcuts.len());
    assert!(!shortcuts.is_empty());

    // Re-serialising and reading back must be lossless.
    let round_tripped: Vec<Shortcut> =
        serde_json::from_str(&serde_json::to_string_pretty(&shortcuts).unwrap()).unwrap();
    assert_eq!(round_tripped, shortcuts);
}

#[test]
#[ignore = "requires a real QGo profile written by the WPF app"]
fn parses_the_live_settings_file() {
    let Some(text) = read(paths::settings_path()) else {
        return;
    };
    let settings: AppSettings =
        serde_json::from_str(&text).expect("settings.json should deserialise");
    println!(
        "settings.json: hotkey={} font={}",
        settings.hotkey_gesture, settings.font_size
    );
    assert!(!settings.hotkey_gesture.is_empty());

    let again: AppSettings =
        serde_json::from_str(&serde_json::to_string_pretty(&settings).unwrap()).unwrap();
    assert_eq!(again.hotkey_gesture, settings.hotkey_gesture);
    assert_eq!(again.window_left.is_nan(), settings.window_left.is_nan());
}

#[test]
#[ignore = "requires a real QGo profile written by the WPF app"]
fn parses_the_live_chained_shortcuts_file() {
    let Some(text) = read(paths::chained_shortcuts_path()) else {
        return;
    };
    let chains: Vec<ChainedShortcut> =
        serde_json::from_str(&text).expect("chained-shortcuts.json should deserialise");
    println!("chained-shortcuts.json: {} chains", chains.len());
    for chain in &chains {
        println!(
            "  {} ({:?}, {} items)",
            chain.key,
            chain.execution_mode,
            chain.chain_items.len()
        );
    }

    let round_tripped: Vec<ChainedShortcut> =
        serde_json::from_str(&serde_json::to_string_pretty(&chains).unwrap()).unwrap();
    assert_eq!(round_tripped, chains);
}

#[test]
#[ignore = "requires a real QGo profile written by the WPF app"]
fn parses_the_live_usage_stats_file() {
    let Some(text) = read(paths::usage_stats_path()) else {
        return;
    };
    let stats: UsageStatistics =
        serde_json::from_str(&text).expect("usage-stats.json should deserialise");
    println!(
        "usage-stats.json: {} executions over {} launches, streak {}",
        stats.total_shortcuts_executed,
        stats.app_launches,
        stats.current_streak()
    );
    assert!(stats.app_launches >= 0);
}
