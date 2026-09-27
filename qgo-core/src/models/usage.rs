use crate::serde_cs;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Port of `QGo.App.Models.UsageStatistics` (`usage-stats.json`).
///
/// The C# dictionaries are `Dictionary<string, int>`, whose JSON key order is
/// insertion order. `BTreeMap` sorts instead, so a file rewritten by this port
/// has its counters alphabetised — the values are identical and both apps read
/// either ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UsageStatistics {
    #[serde(default = "serde_cs::min_value", with = "serde_cs::datetime_utc")]
    pub first_usage: DateTime<Utc>,

    #[serde(default = "serde_cs::min_value", with = "serde_cs::datetime_utc")]
    pub last_updated: DateTime<Utc>,

    #[serde(default)]
    pub total_shortcuts_executed: i32,

    #[serde(default)]
    pub app_launches: i32,

    /// Executions broken down by the `DetermineShortcutType` classification.
    #[serde(default)]
    pub shortcut_type_usage: BTreeMap<String, i32>,

    #[serde(default)]
    pub chained_shortcuts: ChainedShortcutStats,

    #[serde(default)]
    pub feature_usage: BTreeMap<String, i32>,

    /// Rolling window of the last 30 days.
    #[serde(default)]
    pub daily_stats: Vec<DailyUsage>,

    #[serde(default)]
    pub top_shortcuts: Vec<ShortcutUsageInfo>,
}

impl Default for UsageStatistics {
    fn default() -> Self {
        Self {
            first_usage: Utc::now(),
            last_updated: Utc::now(),
            total_shortcuts_executed: 0,
            app_launches: 0,
            shortcut_type_usage: BTreeMap::new(),
            chained_shortcuts: ChainedShortcutStats::default(),
            feature_usage: BTreeMap::new(),
            daily_stats: Vec::new(),
            top_shortcuts: Vec::new(),
        }
    }
}

impl UsageStatistics {
    /// `[JsonIgnore]` computed property on the C# side, so it is not persisted.
    pub fn average_shortcuts_per_day(&self) -> f64 {
        let days_used = (Utc::now() - self.first_usage).num_days() + 1;
        if days_used > 0 {
            self.total_shortcuts_executed as f64 / days_used as f64
        } else {
            0.0
        }
    }

    /// `[JsonIgnore]` computed property: consecutive days ending today on which
    /// at least one shortcut was executed.
    pub fn current_streak(&self) -> i32 {
        if self.daily_stats.is_empty() {
            return 0;
        }
        let mut sorted: Vec<&DailyUsage> = self.daily_stats.iter().collect();
        sorted.sort_by_key(|d| std::cmp::Reverse(d.date));

        let today = Utc::now().date_naive();
        let mut streak = 0;
        for (i, day) in sorted.iter().enumerate() {
            let expected = today - Duration::days(i as i64);
            if day.date.date_naive() == expected && day.shortcuts_executed > 0 {
                streak += 1;
            } else {
                break;
            }
        }
        streak
    }
}

/// Port of `QGo.App.Models.ChainedShortcutStats`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChainedShortcutStats {
    #[serde(default)]
    pub total_executed: i32,

    #[serde(default)]
    pub total_chain_items_executed: i32,

    #[serde(default)]
    pub execution_mode_usage: BTreeMap<String, i32>,

    #[serde(default)]
    pub average_chain_length: f64,

    /// Percentage, 0-100.
    #[serde(default)]
    pub success_rate: f64,
}

/// Port of `QGo.App.Models.DailyUsage`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DailyUsage {
    #[serde(default = "serde_cs::min_value", with = "serde_cs::datetime_utc")]
    pub date: DateTime<Utc>,

    #[serde(default)]
    pub shortcuts_executed: i32,

    #[serde(default)]
    pub app_launches: i32,

    #[serde(default)]
    pub features_used: Vec<String>,
}

impl Default for DailyUsage {
    fn default() -> Self {
        Self {
            date: serde_cs::min_value(),
            shortcuts_executed: 0,
            app_launches: 0,
            features_used: Vec::new(),
        }
    }
}

/// Port of `QGo.App.Models.ShortcutUsageInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ShortcutUsageInfo {
    #[serde(default)]
    pub key: String,

    #[serde(default)]
    pub execution_count: i32,

    #[serde(default = "serde_cs::min_value", with = "serde_cs::datetime_local")]
    pub last_used: DateTime<Utc>,

    /// `web_url`, `executable`, `local_path`, ... — see `command::determine_shortcut_type`.
    #[serde(default)]
    pub shortcut_type: String,

    #[serde(default)]
    pub is_chained: bool,
}

impl Default for ShortcutUsageInfo {
    fn default() -> Self {
        Self {
            key: String::new(),
            execution_count: 0,
            last_used: serde_cs::min_value(),
            shortcut_type: String::new(),
            is_chained: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_users_usage_stats_file_shape() {
        let json = r#"{
            "FirstUsage": "2025-11-30T19:05:00.8496632Z",
            "LastUpdated": "2026-08-05T06:45:35.0177265Z",
            "TotalShortcutsExecuted": 154,
            "AppLaunches": 302,
            "ShortcutTypeUsage": { "chained": 22, "local_path": 52 },
            "ChainedShortcuts": {
                "TotalExecuted": 22,
                "TotalChainItemsExecuted": 47,
                "ExecutionModeUsage": { "Sequential": 22 },
                "AverageChainLength": 2.1363636363636362,
                "SuccessRate": 45.83333333333333
            },
            "FeatureUsage": { "command_execution": 22 },
            "DailyStats": [
                { "Date": "2026-07-07T00:00:00Z", "ShortcutsExecuted": 1, "AppLaunches": 1, "FeaturesUsed": [] }
            ],
            "TopShortcuts": []
        }"#;
        let u: UsageStatistics = serde_json::from_str(json).unwrap();
        assert_eq!(u.total_shortcuts_executed, 154);
        assert_eq!(u.app_launches, 302);
        assert_eq!(u.shortcut_type_usage["local_path"], 52);
        assert_eq!(u.chained_shortcuts.total_chain_items_executed, 47);
        assert_eq!(u.daily_stats.len(), 1);
        assert_eq!(u.daily_stats[0].shortcuts_executed, 1);
    }

    #[test]
    fn streak_counts_back_from_today() {
        let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        let mut u = UsageStatistics::default();
        for back in 0..3 {
            u.daily_stats.push(DailyUsage {
                date: DateTime::from_naive_utc_and_offset(today - Duration::days(back), Utc),
                shortcuts_executed: 1,
                app_launches: 1,
                features_used: vec![],
            });
        }
        assert_eq!(u.current_streak(), 3);
    }

    #[test]
    fn streak_stops_at_the_first_gap() {
        let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        let mut u = UsageStatistics::default();
        u.daily_stats.push(DailyUsage {
            date: DateTime::from_naive_utc_and_offset(today, Utc),
            shortcuts_executed: 1,
            ..Default::default()
        });
        u.daily_stats.push(DailyUsage {
            date: DateTime::from_naive_utc_and_offset(today - Duration::days(5), Utc),
            shortcuts_executed: 1,
            ..Default::default()
        });
        assert_eq!(u.current_streak(), 1);
    }
}
