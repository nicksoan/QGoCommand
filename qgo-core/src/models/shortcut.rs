use crate::serde_cs;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Port of `QGo.App.Models.Shortcut` (`links.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Shortcut {
    #[serde(default)]
    pub key: String,

    #[serde(default)]
    pub template: String,

    /// Number of times this shortcut has been executed (for most-used tracking).
    #[serde(default)]
    pub usage_count: i32,

    /// Assigned from `DateTime.Now` in C#, so it is written with a numeric offset.
    #[serde(default = "serde_cs::min_value", with = "serde_cs::datetime_local")]
    pub last_used: DateTime<Utc>,

    /// Whether this entry is a proxy for a [`super::ChainedShortcut`].
    #[serde(default)]
    pub is_chained: bool,

    /// For chained shortcuts, the key of the `ChainedShortcut` definition.
    #[serde(default)]
    pub chained_shortcut_key: Option<String>,

    /// Any member written by a build that knows more than this one.
    ///
    /// Without this, running the Rust build against a profile written by a
    /// newer `QGo.App` would silently delete the members it does not know,
    /// because serde drops unrecognised keys on the way in.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Shortcut {
    fn default() -> Self {
        Self {
            key: String::new(),
            template: String::new(),
            usage_count: 0,
            last_used: serde_cs::min_value(),
            is_chained: false,
            chained_shortcut_key: None,
            extra: Default::default(),
        }
    }
}

impl Shortcut {
    pub fn new(key: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            template: template.into(),
            ..Default::default()
        }
    }

    /// Mirrors `MainViewModel.IsParameterized`: a chained shortcut never takes a
    /// parameter, anything else does if its template contains `{` and `}`.
    pub fn is_parameterized(&self) -> bool {
        !self.is_chained && self.template.contains('{') && self.template.contains('}')
    }

    /// Mirrors `MainViewModel.GetPlaceholderText` — the hint drawn after the
    /// typed key, e.g. `" query"` for `https://google.com/search?q={Query}`.
    pub fn placeholder_text(&self) -> String {
        if !self.is_parameterized() {
            return String::new();
        }
        let start = self.template.find('{');
        let end = self.template.find('}');
        if let (Some(start), Some(end)) = (start, end) {
            if end > start {
                return format!(" {}", self.template[start + 1..end].to_lowercase());
            }
        }
        " enter parameter...".to_string()
    }
}

impl std::fmt::Display for Shortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_the_shape_on_disk() {
        let json = r#"{
            "Key": "google",
            "Template": "https://www.google.com/search?q={Query}",
            "UsageCount": 24,
            "LastUsed": "2025-12-04T20:28:22.2660282+00:00",
            "IsChained": false,
            "ChainedShortcutKey": null
        }"#;
        let s: Shortcut = serde_json::from_str(json).unwrap();
        assert_eq!(s.key, "google");
        assert_eq!(s.usage_count, 24);
        assert!(!s.is_chained);
        assert!(s.is_parameterized());
        assert_eq!(s.placeholder_text(), " query");
    }

    #[test]
    fn missing_fields_fall_back_to_the_csharp_initialisers() {
        let s: Shortcut = serde_json::from_str(r#"{"Key":"k","Template":"t"}"#).unwrap();
        assert_eq!(s.usage_count, 0);
        assert_eq!(s.last_used, serde_cs::min_value());
        assert_eq!(s.chained_shortcut_key, None);
    }

    #[test]
    fn serialises_back_to_pascal_case() {
        let json = serde_json::to_string(&Shortcut::new("qgo", "https://qgocommand.com/")).unwrap();
        assert!(json.contains(r#""Key":"qgo""#));
        assert!(json.contains(r#""UsageCount":0"#));
        assert!(json.contains(r#""LastUsed":"0001-01-01T00:00:00""#));
    }

    #[test]
    fn a_member_written_by_a_newer_build_survives_a_round_trip() {
        let json = r#"{"Key":"k","Template":"t","FutureFlag":true}"#;
        let s: Shortcut = serde_json::from_str(json).unwrap();
        assert_eq!(s.extra["FutureFlag"], true);
        assert!(serde_json::to_string(&s)
            .unwrap()
            .contains(r#""FutureFlag":true"#));
    }

    #[test]
    fn chain_proxies_are_never_parameterized() {
        let mut s = Shortcut::new("work", "[Chain: {stuff}]");
        s.is_chained = true;
        assert!(!s.is_parameterized());
        assert_eq!(s.placeholder_text(), "");
    }
}
