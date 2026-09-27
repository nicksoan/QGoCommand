//! Port of `QGo.App.Services.LocalUsageStatsService`.
//!
//! Everything here is on-disk only. The remote half — `TelemetryService`, which
//! POSTs aggregated events to the Azure Functions backend — is **not** ported;
//! see the README's "Not ported" section. Consent still gates local recording so
//! that turning telemetry off in Settings has the same visible effect.

use crate::models::{DailyUsage, ShortcutUsageInfo, UsageStatistics};
use crate::paths;
use chrono::{DateTime, Duration, Utc};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Keep a month of daily rows, like the C#.
const DAILY_RETENTION_DAYS: i64 = 30;
/// `TopShortcuts` is capped at ten entries.
const TOP_SHORTCUTS: usize = 10;

/// `LocalUsageStatsService`, as a process-wide singleton behind a mutex — the
/// C# guards every method with `lock (_lockObject)`.
pub struct LocalUsageStats {
    stats: UsageStatistics,
}

static INSTANCE: OnceLock<Mutex<LocalUsageStats>> = OnceLock::new();

pub fn instance() -> MutexGuard<'static, LocalUsageStats> {
    let cell = INSTANCE.get_or_init(|| {
        Mutex::new(LocalUsageStats {
            stats: load_stats(),
        })
    });
    cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl LocalUsageStats {
    /// `GetStats()`.
    pub fn stats(&self) -> &UsageStatistics {
        &self.stats
    }

    /// `RecordAppLaunch()` — `FirstUsage` is written once and never touched again.
    pub fn record_app_launch(&mut self) {
        if self.stats.first_usage == crate::serde_cs::min_value() {
            self.stats.first_usage = Utc::now();
        }
        self.stats.app_launches += 1;
        self.stats.last_updated = Utc::now();
        self.update_daily(1, 0);
        self.save();
    }

    /// `RecordShortcutExecution(shortcutType, hasParameters, isChained)`.
    pub fn record_shortcut_execution(&mut self, shortcut_type: &str) {
        self.stats.total_shortcuts_executed += 1;
        self.stats.last_updated = Utc::now();
        *self
            .stats
            .shortcut_type_usage
            .entry(shortcut_type.to_string())
            .or_insert(0) += 1;
        self.update_daily(0, 1);
        self.save();
    }

    /// `RecordChainedShortcutExecution(executionMode, chainItemCount, continueOnError)`.
    pub fn record_chained_shortcut_execution(
        &mut self,
        execution_mode: &str,
        chain_item_count: i32,
    ) {
        let chained = &mut self.stats.chained_shortcuts;
        chained.total_executed += 1;
        chained.total_chain_items_executed += chain_item_count;
        *chained
            .execution_mode_usage
            .entry(execution_mode.to_string())
            .or_insert(0) += 1;

        if chained.total_executed > 0 {
            chained.average_chain_length =
                chained.total_chain_items_executed as f64 / chained.total_executed as f64;
        }

        self.stats.last_updated = Utc::now();
        self.save();
    }

    /// `RecordChainItemExecution(...)` — recomputes the rolling success rate.
    ///
    /// The arithmetic is copied verbatim from the C#, including the fact that it
    /// reconstructs the success count from the stored percentage rather than
    /// keeping a counter, which makes the figure drift over time.
    pub fn record_chain_item_execution(&mut self, success: bool) {
        let chained = &mut self.stats.chained_shortcuts;
        let total_attempts = chained.total_chain_items_executed;
        let mut success_count = (chained.success_rate * total_attempts as f64 / 100.0) as i32;
        if success {
            success_count += 1;
        }
        let total_attempts = total_attempts + 1;
        chained.success_rate = if total_attempts > 0 {
            success_count as f64 / total_attempts as f64 * 100.0
        } else {
            0.0
        };
        self.stats.last_updated = Utc::now();
        self.save();
    }

    /// `RecordFeatureUsage(featureName, properties)` — the property bag is only
    /// used by the remote telemetry, so it is dropped here as it is in the C#.
    pub fn record_feature_usage(&mut self, feature_name: &str) {
        *self
            .stats
            .feature_usage
            .entry(feature_name.to_string())
            .or_insert(0) += 1;

        let today = Utc::now().date_naive();
        if let Some(day) = self
            .stats
            .daily_stats
            .iter_mut()
            .find(|d| d.date.date_naive() == today)
        {
            if !day.features_used.iter().any(|f| f == feature_name) {
                day.features_used.push(feature_name.to_string());
            }
        }

        self.stats.last_updated = Utc::now();
        self.save();
    }

    /// `UpdateShortcutUsage(shortcuts)` — rebuilds the top-ten table.
    pub fn update_shortcut_usage(&mut self, shortcuts: &[crate::models::Shortcut]) {
        let mut used: Vec<&crate::models::Shortcut> =
            shortcuts.iter().filter(|s| s.usage_count > 0).collect();
        used.sort_by_key(|s| std::cmp::Reverse(s.usage_count));
        used.truncate(TOP_SHORTCUTS);

        self.stats.top_shortcuts = used
            .into_iter()
            .map(|s| ShortcutUsageInfo {
                key: s.key.clone(),
                execution_count: s.usage_count,
                last_used: s.last_used,
                is_chained: s.is_chained,
                shortcut_type: if s.is_chained {
                    "chained".to_string()
                } else {
                    determine_shortcut_type_basic(&s.template).to_string()
                },
            })
            .collect();

        self.stats.last_updated = Utc::now();
        self.save();
    }

    /// `GetUsageSummary()`.
    pub fn usage_summary(&self) -> String {
        let s = &self.stats;
        let days_used = (Utc::now() - s.first_usage).num_days() + 1;
        format!(
            "QGo Usage Summary\n\
             -----------------\n\
             Days using QGo: {days_used}\n\
             Total shortcuts executed: {}\n\
             Average per day: {:.1}\n\
             Current streak: {} days\n\
             App launches: {}\n\
             Chained shortcuts: {}\n\
             Chain success rate: {:.1}%",
            s.total_shortcuts_executed,
            s.average_shortcuts_per_day(),
            s.current_streak(),
            s.app_launches,
            s.chained_shortcuts.total_executed,
            s.chained_shortcuts.success_rate,
        )
    }

    /// `UpdateDailyStats(launches, shortcuts)` — upsert today's row, then drop
    /// anything older than 30 days.
    fn update_daily(&mut self, launches: i32, shortcuts: i32) {
        let today = Utc::now().date_naive();

        match self
            .stats
            .daily_stats
            .iter_mut()
            .find(|d| d.date.date_naive() == today)
        {
            Some(day) => {
                day.app_launches += launches;
                day.shortcuts_executed += shortcuts;
            }
            None => {
                let midnight = today
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight is a valid time");
                self.stats.daily_stats.push(DailyUsage {
                    date: DateTime::from_naive_utc_and_offset(midnight, Utc),
                    app_launches: launches,
                    shortcuts_executed: shortcuts,
                    features_used: Vec::new(),
                });
            }
        }

        let cutoff = Utc::now() - Duration::days(DAILY_RETENTION_DAYS);
        let cutoff_day = cutoff.date_naive();
        self.stats
            .daily_stats
            .retain(|d| d.date.date_naive() >= cutoff_day);
    }

    /// `SaveStats()` — back up, write to a temp file, then move over the target.
    fn save(&self) {
        let path = paths::usage_stats_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() > 0 {
                let _ = fs::copy(&path, paths::usage_stats_backup_path());
            }
        }

        let Ok(json) = serde_json::to_string_pretty(&self.stats) else {
            log::warn!("usage stats could not be serialised");
            return;
        };
        if json.len() < 10 {
            log::warn!("refusing to write a suspiciously short usage-stats.json");
            return;
        }

        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, &json).is_ok() && fs::rename(&tmp, &path).is_ok() {
            return;
        }

        log::warn!("failed to save usage stats; restoring the backup");
        let backup = paths::usage_stats_backup_path();
        if backup.exists() {
            let _ = fs::copy(&backup, &path);
        }
    }
}

/// `LoadStats()` — falls back to the `.backup` copy, then to fresh statistics.
fn load_stats() -> UsageStatistics {
    let path = paths::usage_stats_path();

    if let Some(stats) = read_stats_file(&path) {
        return stats;
    }
    if let Some(stats) = read_stats_file(&paths::usage_stats_backup_path()) {
        log::warn!("usage-stats.json was unusable; recovered from the backup");
        return stats;
    }

    UsageStatistics::default()
}

fn read_stats_file(path: &Path) -> Option<UsageStatistics> {
    let text = fs::read_to_string(path).ok()?;
    let stats: UsageStatistics = serde_json::from_str(&text).ok()?;
    is_valid_stats(&stats).then_some(stats)
}

/// `IsValidStats(stats)` — guards against a corrupted or clock-skewed file.
fn is_valid_stats(stats: &UsageStatistics) -> bool {
    if stats.first_usage == crate::serde_cs::min_value() {
        return false;
    }
    if stats.first_usage > Utc::now() + Duration::days(1) {
        return false;
    }
    if stats.total_shortcuts_executed < 0 || stats.app_launches < 0 {
        return false;
    }
    true
}

/// The `DetermineShortcutType` copy inside `LocalUsageStatsService`, which —
/// unlike the one in `MainViewModel` — does **not** classify `cmd:` / `ps:`
/// targets. Kept separate so the two files keep producing the same buckets.
pub fn determine_shortcut_type_basic(template: &str) -> &'static str {
    if template.is_empty() {
        return "empty";
    }
    let lower = template.to_lowercase();

    if lower.starts_with("http://") || lower.starts_with("https://") {
        return "web_url";
    }
    if lower.contains("://") {
        return "protocol_url";
    }
    if lower.ends_with(".exe") || lower.contains(".exe ") {
        return "executable";
    }
    if lower.starts_with('%') {
        return "environment_path";
    }
    if lower.starts_with("\\\\") {
        return "network_path";
    }
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() >= 3 && chars[1] == ':' && chars[2] == '\\' {
        return "local_path";
    }
    if Path::new(template).exists() {
        return "file_system";
    }
    "other"
}

/// Appends one line to a monthly log file, skipping the write once the file
/// passes `max_bytes`.
pub fn append_capped_log(dir: &Path, file_name: &str, line: &str, max_bytes: u64) {
    use std::io::Write;

    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let path = dir.join(file_name);
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() >= max_bytes {
            return;
        }
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_with_no_first_usage_are_rejected() {
        let stats = UsageStatistics {
            first_usage: crate::serde_cs::min_value(),
            ..Default::default()
        };
        assert!(!is_valid_stats(&stats));
    }

    #[test]
    fn stats_from_the_future_are_rejected() {
        let stats = UsageStatistics {
            first_usage: Utc::now() + Duration::days(7),
            ..Default::default()
        };
        assert!(!is_valid_stats(&stats));
    }

    #[test]
    fn negative_counters_are_rejected() {
        let stats = UsageStatistics {
            app_launches: -1,
            ..Default::default()
        };
        assert!(!is_valid_stats(&stats));
    }

    #[test]
    fn plausible_stats_are_accepted() {
        let stats = UsageStatistics {
            first_usage: Utc::now() - Duration::days(30),
            app_launches: 302,
            ..Default::default()
        };
        assert!(is_valid_stats(&stats));
    }

    #[test]
    fn the_basic_classifier_does_not_recognise_shell_commands() {
        // The MainViewModel copy returns "powershell_command" here; this one does not.
        assert_eq!(determine_shortcut_type_basic("ps: Get-Date"), "other");
        assert_eq!(
            crate::command::determine_shortcut_type("ps: Get-Date"),
            "powershell_command"
        );
        // Everything else agrees.
        assert_eq!(determine_shortcut_type_basic("https://x.com"), "web_url");
        assert_eq!(
            determine_shortcut_type_basic("%HOMEPATH%"),
            "environment_path"
        );
    }

    #[test]
    fn capped_logs_stop_growing_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        append_capped_log(dir.path(), "test.log", "first", 1024);
        append_capped_log(dir.path(), "test.log", "second", 1024);
        let text = fs::read_to_string(dir.path().join("test.log")).unwrap();
        assert!(text.contains("first") && text.contains("second"));

        // A tiny cap blocks any further appends.
        append_capped_log(dir.path(), "test.log", "third", 1);
        let text = fs::read_to_string(dir.path().join("test.log")).unwrap();
        assert!(!text.contains("third"));
    }
}
