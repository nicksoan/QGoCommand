use crate::serde_cs;
use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Port of `QGo.App.Models.ExecutionMode`.
///
/// No `JsonStringEnumConverter` is registered in `Storage.Opt`, so this is an
/// integer on disk. The discriminants must not be reordered — the user's
/// `chained-shortcuts.json` already contains `"ExecutionMode": 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    /// Execute all shortcuts simultaneously (parallel execution).
    Simultaneous = 0,
    /// Execute shortcuts one after another with the configured delays.
    #[default]
    Sequential = 1,
}

impl ExecutionMode {
    pub fn from_i64(v: i64) -> Self {
        match v {
            0 => Self::Simultaneous,
            _ => Self::Sequential,
        }
    }

    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// Matches `ExecutionMode.ToString()`, which telemetry uses as a dictionary key.
    pub fn name(self) -> &'static str {
        match self {
            Self::Simultaneous => "Simultaneous",
            Self::Sequential => "Sequential",
        }
    }
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for ExecutionMode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.as_i64())
    }
}

impl<'de> Deserialize<'de> for ExecutionMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ModeVisitor;

        impl<'de> Visitor<'de> for ModeVisitor {
            type Value = ExecutionMode;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("0, 1, or the name \"Simultaneous\" / \"Sequential\"")
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(ExecutionMode::from_i64(v))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(ExecutionMode::from_i64(v as i64))
            }

            // Tolerated so a file hand-edited into the string form still loads.
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v.trim().to_ascii_lowercase().as_str() {
                    "simultaneous" | "parallel" | "0" => Ok(ExecutionMode::Simultaneous),
                    "sequential" | "1" => Ok(ExecutionMode::Sequential),
                    other => Err(de::Error::custom(format!("unknown ExecutionMode: {other}"))),
                }
            }
        }

        d.deserialize_any(ModeVisitor)
    }
}

/// Port of `QGo.App.Models.ShortcutChainItem`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ShortcutChainItem {
    /// The key of an existing (non-chained) shortcut to execute.
    #[serde(default)]
    pub shortcut_key: String,

    /// Parameters substituted into the referenced shortcut's template.
    #[serde(default)]
    pub parameters: String,

    /// Delay before executing this item. Sequential mode only.
    #[serde(default)]
    pub delay_ms: i32,

    #[serde(default = "yes")]
    pub enabled: bool,

    /// Any member written by a build that knows more than this one.
    ///
    /// Without this, running the Rust build against a profile written by a
    /// newer `QGo.App` would silently delete the members it does not know,
    /// because serde drops unrecognised keys on the way in.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn yes() -> bool {
    true
}

impl Default for ShortcutChainItem {
    fn default() -> Self {
        Self {
            shortcut_key: String::new(),
            parameters: String::new(),
            delay_ms: 0,
            enabled: true,
            extra: Default::default(),
        }
    }
}

impl ShortcutChainItem {
    /// C#'s setter clamps to zero rather than rejecting; do the same here.
    pub fn set_delay_ms(&mut self, value: i32) {
        self.delay_ms = value.max(0);
    }
}

impl fmt::Display for ShortcutChainItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(format!("{} {}", self.shortcut_key, self.parameters).trim())
    }
}

/// Port of `QGo.App.Models.ChainedShortcut` (`chained-shortcuts.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ChainedShortcut {
    #[serde(default)]
    pub key: String,

    #[serde(default)]
    pub description: String,

    #[serde(default)]
    pub execution_mode: ExecutionMode,

    /// Delay applied between items in Sequential mode, on top of each item's own delay.
    #[serde(default = "default_global_delay")]
    pub global_delay_ms: i32,

    #[serde(default = "yes")]
    pub continue_on_error: bool,

    #[serde(default)]
    pub chain_items: Vec<ShortcutChainItem>,

    #[serde(default)]
    pub usage_count: i32,

    #[serde(default = "serde_cs::min_value", with = "serde_cs::datetime_local")]
    pub last_used: DateTime<Utc>,

    /// Any member written by a build that knows more than this one.
    ///
    /// Without this, running the Rust build against a profile written by a
    /// newer `QGo.App` would silently delete the members it does not know,
    /// because serde drops unrecognised keys on the way in.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_global_delay() -> i32 {
    1000
}

impl Default for ChainedShortcut {
    fn default() -> Self {
        Self {
            key: String::new(),
            description: String::new(),
            execution_mode: ExecutionMode::Sequential,
            global_delay_ms: 1000,
            continue_on_error: true,
            chain_items: Vec::new(),
            usage_count: 0,
            last_used: serde_cs::min_value(),
            extra: Default::default(),
        }
    }
}

impl ChainedShortcut {
    /// C#'s setter clamps to zero rather than rejecting; do the same here.
    pub fn set_global_delay_ms(&mut self, value: i32) {
        self.global_delay_ms = value.max(0);
    }

    pub fn enabled_items(&self) -> impl Iterator<Item = &ShortcutChainItem> {
        self.chain_items.iter().filter(|i| i.enabled)
    }

    /// The template the WPF app stores on the proxy `Shortcut` in `links.json`.
    pub fn proxy_template(&self) -> String {
        format!("[Chain: {}]", self.description)
    }
}

impl fmt::Display for ChainedShortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_is_an_integer_on_disk() {
        assert_eq!(
            serde_json::to_string(&ExecutionMode::Sequential).unwrap(),
            "1"
        );
        assert_eq!(
            serde_json::to_string(&ExecutionMode::Simultaneous).unwrap(),
            "0"
        );
        assert_eq!(
            serde_json::from_str::<ExecutionMode>("1").unwrap(),
            ExecutionMode::Sequential
        );
    }

    #[test]
    fn deserialises_a_real_chain_from_the_users_file() {
        let json = r#"{
            "Key": "work",
            "Description": "Apps useful for work",
            "ExecutionMode": 1,
            "GlobalDelayMs": 200,
            "ContinueOnError": true,
            "ChainItems": [
                { "ShortcutKey": "notepad", "Parameters": "", "DelayMs": 0, "Enabled": true },
                { "ShortcutKey": "temp", "Parameters": "", "DelayMs": 300, "Enabled": true }
            ],
            "UsageCount": 3,
            "LastUsed": "2025-12-07T22:18:27.3342515+00:00"
        }"#;
        let c: ChainedShortcut = serde_json::from_str(json).unwrap();
        assert_eq!(c.key, "work");
        assert_eq!(c.execution_mode, ExecutionMode::Sequential);
        assert_eq!(c.global_delay_ms, 200);
        assert_eq!(c.chain_items.len(), 2);
        assert_eq!(c.enabled_items().count(), 2);
        assert_eq!(c.proxy_template(), "[Chain: Apps useful for work]");
    }

    #[test]
    fn delays_clamp_at_zero_like_the_csharp_setters() {
        let mut c = ChainedShortcut::default();
        c.set_global_delay_ms(-5);
        assert_eq!(c.global_delay_ms, 0);

        let mut i = ShortcutChainItem::default();
        i.set_delay_ms(-1);
        assert_eq!(i.delay_ms, 0);
    }
}
