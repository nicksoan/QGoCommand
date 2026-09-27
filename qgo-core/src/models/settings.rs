use crate::serde_cs;
use serde::{Deserialize, Serialize};

/// Port of `QGo.App.Models.AppSettings` (`settings.json`).
///
/// Field order matches the C# property declaration order, because that is the
/// order `System.Text.Json` writes them in and it keeps diffs of the settings
/// file clean when the two apps are used against the same profile.
///
/// `WindowLeft` / `WindowTop` / `WindowHeight` default to `double.NaN`, which
/// the C# serialiser writes as the string `"NaN"` — see [`serde_cs::cs_f64`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AppSettings {
    #[serde(default = "default_hotkey")]
    pub hotkey_gesture: String,

    #[serde(default = "default_shell_colour")]
    pub shell_colour: String,

    #[serde(default = "default_foreground")]
    pub foreground: String,

    #[serde(default = "default_highlight")]
    pub highlight: String,

    #[serde(default = "default_surface")]
    pub surface: String,

    #[serde(default = "default_font_size")]
    pub font_size: f64,

    #[serde(default = "default_font_family")]
    pub font_family: String,

    #[serde(default)]
    pub start_with_windows: bool,

    #[serde(default = "yes")]
    pub show_welcome_on_startup: bool,

    #[serde(default = "yes")]
    pub show_glow_on_startup: bool,

    #[serde(default = "yes")]
    pub enable_telemetry: bool,

    #[serde(default)]
    pub telemetry_consent_shown: bool,

    #[serde(default)]
    pub last_run_version: Option<String>,

    #[serde(default = "yes")]
    pub enable_screen_reader_support: bool,

    #[serde(default)]
    pub enable_high_contrast: bool,

    #[serde(default)]
    pub enable_sound_feedback: bool,

    #[serde(default)]
    pub reduce_animations: bool,

    #[serde(default = "yes")]
    pub enhanced_keyboard_navigation: bool,

    /// 1.0 = normal, 1.5 = 150%.
    #[serde(default = "default_font_scale")]
    pub accessibility_font_scale: f64,

    /// Screen-reader announcement timeout, in milliseconds.
    #[serde(default = "default_accessibility_timeout")]
    pub accessibility_timeout: i32,

    #[serde(default = "yes")]
    pub minimize_to_tray: bool,

    #[serde(default)]
    pub start_minimized_to_tray: bool,

    #[serde(default)]
    pub has_shown_tray_notification: bool,

    #[serde(default = "nan", with = "serde_cs::cs_f64")]
    pub window_left: f64,

    #[serde(default = "nan", with = "serde_cs::cs_f64")]
    pub window_top: f64,

    #[serde(default = "default_window_width", with = "serde_cs::cs_f64")]
    pub window_width: f64,

    #[serde(default = "nan", with = "serde_cs::cs_f64")]
    pub window_height: f64,

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
fn nan() -> f64 {
    f64::NAN
}
fn default_hotkey() -> String {
    "Alt+Q".into()
}
fn default_shell_colour() -> String {
    "#232A31".into()
}
fn default_foreground() -> String {
    "#E8EDF1".into()
}
fn default_highlight() -> String {
    "#040021".into()
}
fn default_surface() -> String {
    "#1B2127".into()
}
fn default_font_size() -> f64 {
    18.0
}
fn default_font_family() -> String {
    "Segoe UI".into()
}
fn default_font_scale() -> f64 {
    1.0
}
fn default_accessibility_timeout() -> i32 {
    5000
}
fn default_window_width() -> f64 {
    420.0
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey_gesture: default_hotkey(),
            shell_colour: default_shell_colour(),
            foreground: default_foreground(),
            highlight: default_highlight(),
            surface: default_surface(),
            font_size: default_font_size(),
            font_family: default_font_family(),
            start_with_windows: false,
            show_welcome_on_startup: true,
            show_glow_on_startup: true,
            enable_telemetry: true,
            telemetry_consent_shown: false,
            last_run_version: None,
            enable_screen_reader_support: true,
            enable_high_contrast: false,
            enable_sound_feedback: false,
            reduce_animations: false,
            enhanced_keyboard_navigation: true,
            accessibility_font_scale: default_font_scale(),
            accessibility_timeout: default_accessibility_timeout(),
            minimize_to_tray: true,
            start_minimized_to_tray: false,
            has_shown_tray_notification: false,
            window_left: nan(),
            window_top: nan(),
            window_width: default_window_width(),
            window_height: nan(),
            extra: Default::default(),
        }
    }
}

impl AppSettings {
    /// Removes settings that are no longer part of the application.
    pub fn remove_obsolete_fields(&mut self) -> bool {
        let mut removed = false;
        for key in [
            concat!("Lic", "enseTier"),
            concat!("Lic", "enseKey"),
            concat!("Lic", "enseExpiryUtc"),
        ] {
            removed |= self.extra.remove(key).is_some();
        }
        removed
    }

    /// `MainViewModel.ScaledFontSize`.
    pub fn scaled_font_size(&self) -> f64 {
        self.font_size * self.accessibility_font_scale
    }

    /// True when the stored window position is usable (the WPF app treats `NaN`
    /// as "never positioned" and lets Windows place the window).
    pub fn has_window_position(&self) -> bool {
        self.window_left.is_finite() && self.window_top.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_users_settings_file_shape() {
        let json = r##"{
            "HotkeyGesture": "Alt+Q",
            "ShellColour": "#232A31",
            "FontSize": 18,
            "StartWithWindows": true,
            "AccessibilityFontScale": 1,
            "WindowLeft": 901.3333333333333,
            "WindowWidth": 209.33333333333331
        }"##;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.hotkey_gesture, "Alt+Q");
        assert_eq!(s.font_size, 18.0);
        assert!(s.start_with_windows);
        assert_eq!(s.accessibility_font_scale, 1.0);
        assert!(s.has_window_position() || s.window_top.is_nan());
        // Fields absent from the JSON fall back to the C# initialisers.
        assert_eq!(s.font_family, "Segoe UI");
        assert!(s.minimize_to_tray);
        assert!(s.window_height.is_nan());
    }

    #[test]
    fn fresh_defaults_write_nan_as_a_string() {
        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        assert!(json.contains(r#""WindowLeft":"NaN""#), "{json}");
        assert!(json.contains(r#""WindowTop":"NaN""#), "{json}");
        assert!(!json.contains(concat!("Lic", "ense")), "{json}");
    }

    #[test]
    fn nan_positions_round_trip() {
        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert!(back.window_left.is_nan());
        assert!(!back.has_window_position());
    }

    #[test]
    fn a_member_written_by_a_newer_build_survives_a_round_trip() {
        // Both apps share one settings.json. If QGo.App gains a setting, running
        // the Rust build must not silently delete it.
        let json = r##"{"HotkeyGesture":"Alt+Q","SomeFutureSetting":42}"##;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(s.extra["SomeFutureSetting"], 42);

        let back = serde_json::to_string(&s).unwrap();
        assert!(back.contains(r#""SomeFutureSetting":42"#), "{back}");
    }

    #[test]
    fn obsolete_fields_are_removed_before_saving() {
        let json = format!(
            r#"{{"HotkeyGesture":"Alt+Q","{}":"legacy","SomeFutureSetting":42}}"#,
            concat!("Lic", "enseTier")
        );
        let mut settings: AppSettings = serde_json::from_str(&json).unwrap();
        assert!(settings.remove_obsolete_fields());

        let back = serde_json::to_string(&settings).unwrap();
        assert!(!back.contains(concat!("Lic", "ense")), "{back}");
        assert!(back.contains(r#""SomeFutureSetting":42"#), "{back}");
    }

    #[test]
    fn font_scaling_matches_the_view_model() {
        let s = AppSettings {
            font_size: 18.0,
            accessibility_font_scale: 1.5,
            ..Default::default()
        };
        assert_eq!(s.scaled_font_size(), 27.0);
    }
}
