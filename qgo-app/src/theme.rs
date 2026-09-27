//! The colour and font half of `MainViewModel.ApplyTheme` plus the WPF resource
//! dictionary at the top of `MainWindow.xaml`.
//!
//! WPF stores colours as `#RRGGBB` / `#AARRGGBB` strings and converts them with
//! `BrushConverter`; the same strings are parsed here so `settings.json` means
//! the same thing in both builds.

use egui::Color32;
use qgo_core::AppSettings;

/// `MainWindow.xaml`'s `BorderBrush1`.
pub const BORDER: Color32 = Color32::from_rgb(0x3A, 0x42, 0x48);
/// `MainWindow.xaml`'s `Accent`, used on the focused result.
pub const ACCENT: Color32 = Color32::from_rgb(0x2D, 0x81, 0xFF);
/// The green of the startup glow gradient.
pub const GLOW: Color32 = Color32::from_rgb(0x10, 0xB9, 0x81);

/// Parses a WPF brush string: `#RGB`, `#RRGGBB` or `#AARRGGBB`.
///
/// Note the alpha-first ordering — WPF is `#AARRGGBB`, not `#RRGGBBAA`.
pub fn parse_color(value: &str) -> Option<Color32> {
    let hex = value.trim().strip_prefix('#')?;
    let digits: Result<Vec<u8>, _> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("zz"), 16))
        .collect();

    match hex.len() {
        3 => {
            // #RGB expands each nibble, as WPF does.
            let mut n = hex.chars().map(|c| c.to_digit(16).map(|d| (d * 17) as u8));
            Some(Color32::from_rgb(n.next()??, n.next()??, n.next()??))
        }
        6 => {
            let d = digits.ok()?;
            Some(Color32::from_rgb(d[0], d[1], d[2]))
        }
        8 => {
            let d = digits.ok()?;
            Some(Color32::from_rgba_unmultiplied(d[1], d[2], d[3], d[0]))
        }
        _ => None,
    }
}

/// The four brushes `MainViewModel` exposes to the XAML bindings.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// The rounded outer shell.
    pub shell: Color32,
    /// Text.
    pub foreground: Color32,
    /// The selected row.
    pub highlight: Color32,
    /// The search box and results panel background.
    pub surface: Color32,
    /// `Settings.FontSize * Settings.AccessibilityFontScale`.
    pub font_size: f32,
}

impl Theme {
    /// `ApplyTheme()`.
    ///
    /// High contrast substitutes the system colours in the C#; egui has no
    /// access to those, so a fixed black-on-white scheme with the WPF accent
    /// stands in for `SystemColors`.
    pub fn from_settings(settings: &AppSettings) -> Self {
        let font_size = settings.scaled_font_size().clamp(8.0, 72.0) as f32;

        if settings.enable_high_contrast {
            return Self {
                shell: Color32::BLACK,
                foreground: Color32::WHITE,
                highlight: ACCENT,
                surface: Color32::from_gray(20),
                font_size,
            };
        }

        Self {
            shell: parse_color(&settings.shell_colour)
                .unwrap_or(Color32::from_rgb(0x23, 0x2A, 0x31)),
            foreground: parse_color(&settings.foreground)
                .unwrap_or(Color32::from_rgb(0xE8, 0xED, 0xF1)),
            highlight: parse_color(&settings.highlight)
                .unwrap_or(Color32::from_rgb(0x04, 0x00, 0x21)),
            surface: parse_color(&settings.surface).unwrap_or(Color32::from_rgb(0x1B, 0x21, 0x27)),
            font_size,
        }
    }

    /// Pushes the theme into egui's style so every widget picks it up.
    pub fn apply(&self, ctx: &egui::Context) {
        use egui::{FontFamily, FontId, TextStyle};

        // `all_styles_mut` covers both the light and dark style sets, so the
        // configured colours win regardless of the OS theme.
        ctx.all_styles_mut(|style| {
            style.text_styles = [
                (
                    TextStyle::Body,
                    FontId::new(self.font_size, FontFamily::Proportional),
                ),
                (
                    TextStyle::Button,
                    FontId::new(self.font_size, FontFamily::Proportional),
                ),
                (
                    TextStyle::Heading,
                    FontId::new(self.font_size * 1.3, FontFamily::Proportional),
                ),
                (
                    TextStyle::Small,
                    FontId::new(self.font_size * 0.8, FontFamily::Proportional),
                ),
                (
                    TextStyle::Monospace,
                    FontId::new(self.font_size, FontFamily::Monospace),
                ),
            ]
            .into();

            style.visuals.override_text_color = Some(self.foreground);
            style.visuals.panel_fill = self.shell;
            style.visuals.window_fill = self.shell;
            style.visuals.extreme_bg_color = self.surface;
            style.visuals.widgets.noninteractive.bg_fill = self.surface;
            style.visuals.widgets.inactive.bg_fill = self.surface;
            style.visuals.widgets.hovered.bg_fill = self.highlight;
            style.visuals.widgets.active.bg_fill = self.highlight;
            style.visuals.selection.bg_fill = self.highlight;
            style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        });
    }
}

/// `MainViewModel.LogoSource` — the December variant is selected seasonally.
pub fn logo_bytes(month: u32) -> &'static [u8] {
    if month == 12 {
        include_bytes!("../assets/QGoLogoSmall_Xmas1.png")
    } else {
        include_bytes!("../assets/QGoLogoSmall.png")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_colours_from_settings_json() {
        assert_eq!(
            parse_color("#232A31"),
            Some(Color32::from_rgb(0x23, 0x2A, 0x31))
        );
        assert_eq!(
            parse_color("#E8EDF1"),
            Some(Color32::from_rgb(0xE8, 0xED, 0xF1))
        );
        assert_eq!(
            parse_color("  #1B2127  "),
            Some(Color32::from_rgb(0x1B, 0x21, 0x27))
        );
    }

    #[test]
    fn wpf_puts_alpha_first_in_eight_digit_colours() {
        // `StartupGlowBrush` from MainWindow.xaml. Color32 stores premultiplied
        // channels, so compare against the same constructor rather than picking
        // the components apart.
        assert_eq!(
            parse_color("#8048B5FF"),
            Some(Color32::from_rgba_unmultiplied(0x48, 0xB5, 0xFF, 0x80))
        );
        assert_eq!(parse_color("#8048B5FF").unwrap().a(), 0x80);
    }

    #[test]
    fn short_form_and_junk_are_handled() {
        assert_eq!(parse_color("#F00"), Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(parse_color("232A31"), None); // no leading '#'
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color(""), None);
    }

    #[test]
    fn a_broken_colour_string_falls_back_to_the_default() {
        let s = AppSettings {
            shell_colour: "not a colour".into(),
            ..Default::default()
        };
        let theme = Theme::from_settings(&s);
        assert_eq!(theme.shell, Color32::from_rgb(0x23, 0x2A, 0x31));
    }

    #[test]
    fn accessibility_scaling_reaches_the_theme() {
        let s = AppSettings {
            font_size: 18.0,
            accessibility_font_scale: 1.5,
            ..Default::default()
        };
        assert_eq!(Theme::from_settings(&s).font_size, 27.0);
    }

    #[test]
    fn high_contrast_overrides_the_configured_colours() {
        let s = AppSettings {
            enable_high_contrast: true,
            ..Default::default()
        };
        let theme = Theme::from_settings(&s);
        assert_eq!(theme.shell, Color32::BLACK);
        assert_eq!(theme.foreground, Color32::WHITE);
    }

    #[test]
    fn the_logo_changes_with_the_season() {
        assert_ne!(logo_bytes(12), logo_bytes(6));
    }
}
