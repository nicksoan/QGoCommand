//! Port of `QGo.Hotkey` — the global `Alt+Q`.
//!
//! The C# calls `RegisterHotKey` against the main window's HWND and handles
//! `WM_HOTKEY` in a `WndProc` hook. `global-hotkey` does the same thing on its
//! own thread, which suits eframe: the hotkey arrives whether or not the
//! launcher window is visible, and waking the render loop is an explicit
//! `Context::request_repaint`.

use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
use std::str::FromStr;

/// Translates a WPF gesture string into the spelling `global-hotkey` parses.
///
/// WPF writes `System.Windows.Input.Key` names — `Q`, `D1`, `NumPad3`, `OemPlus`
/// — and allows `Win` as a modifier, none of which `global-hotkey` knows.
pub fn normalise_gesture(gesture: &str) -> String {
    gesture
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let upper = part.to_uppercase();
            match upper.as_str() {
                // `global-hotkey` spells the Windows key SUPER.
                "WIN" | "WINDOWS" | "LWIN" | "RWIN" => "SUPER".to_string(),
                "CONTROL" => "CTRL".to_string(),
                // WPF names the number row D0..D9.
                d if d.len() == 2 && d.starts_with('D') && d.as_bytes()[1].is_ascii_digit() => {
                    format!("Digit{}", &d[1..])
                }
                // WPF names the numeric keypad NumPad0..NumPad9.
                n if n.starts_with("NUMPAD") => format!("Numpad{}", &n[6..]),
                "OEMPLUS" | "ADD" => "Equal".to_string(),
                "OEMMINUS" | "SUBTRACT" => "Minus".to_string(),
                "OEMCOMMA" => "Comma".to_string(),
                "OEMPERIOD" | "DECIMAL" => "Period".to_string(),
                "OEMQUESTION" => "Slash".to_string(),
                "OEMTILDE" => "Backquote".to_string(),
                "RETURN" => "Enter".to_string(),
                "PRIOR" => "PageUp".to_string(),
                "NEXT" => "PageDown".to_string(),
                "CAPITAL" => "CapsLock".to_string(),
                "SNAPSHOT" => "PrintScreen".to_string(),
                _ => part.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Parses a gesture such as `Alt+Q`, falling back to the default on anything
/// unrecognised so a bad `settings.json` cannot leave the app with no hotkey.
pub fn parse_gesture(gesture: &str) -> HotKey {
    HotKey::from_str(&normalise_gesture(gesture)).unwrap_or_else(|e| {
        log::warn!("could not parse hotkey '{gesture}' ({e}); falling back to Alt+Q");
        HotKey::from_str("Alt+KeyQ").expect("Alt+Q is a valid hotkey")
    })
}

/// Owns the registration. Dropping it unregisters, like `Hotkey.Unregister`.
pub struct HotkeyManager {
    manager: GlobalHotKeyManager,
    current: Option<HotKey>,
}

impl HotkeyManager {
    pub fn new() -> Result<Self, global_hotkey::Error> {
        Ok(Self {
            manager: GlobalHotKeyManager::new()?,
            current: None,
        })
    }

    /// `Hotkey.TryRegister(hwnd, id, gesture)`. Replaces any existing binding so
    /// changing the hotkey in Settings takes effect without a restart.
    pub fn register(&mut self, gesture: &str) -> bool {
        if let Some(previous) = self.current.take() {
            let _ = self.manager.unregister(previous);
        }

        let hotkey = parse_gesture(gesture);
        match self.manager.register(hotkey) {
            Ok(()) => {
                self.current = Some(hotkey);
                log::info!("registered global hotkey '{gesture}'");
                true
            }
            Err(e) => {
                // Almost always means another application already owns it.
                log::warn!("could not register global hotkey '{gesture}': {e}");
                false
            }
        }
    }

    pub fn id(&self) -> Option<u32> {
        self.current.map(|h| h.id)
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        if let Some(hotkey) = self.current.take() {
            let _ = self.manager.unregister(hotkey);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_gesture_parses() {
        assert_eq!(normalise_gesture("Alt+Q"), "Alt+Q");
        let hotkey = parse_gesture("Alt+Q");
        assert_eq!(hotkey, HotKey::from_str("Alt+KeyQ").unwrap());
    }

    #[test]
    fn wpf_key_names_are_translated() {
        assert_eq!(normalise_gesture("Ctrl+D1"), "Ctrl+Digit1");
        assert_eq!(normalise_gesture("Alt+NumPad5"), "Alt+Numpad5");
        assert_eq!(normalise_gesture("Win+Space"), "SUPER+Space");
        assert_eq!(normalise_gesture("Control+Shift+Q"), "CTRL+Shift+Q");
        assert_eq!(normalise_gesture("Ctrl+OemPlus"), "Ctrl+Equal");
    }

    #[test]
    fn spacing_and_empty_segments_are_tolerated() {
        assert_eq!(normalise_gesture(" Alt + Q "), "Alt+Q");
        assert_eq!(normalise_gesture("Alt++Q"), "Alt+Q");
    }

    #[test]
    fn a_nonsense_gesture_falls_back_to_alt_q() {
        assert_eq!(parse_gesture("!!!not a hotkey!!!"), parse_gesture("Alt+Q"));
        assert_eq!(parse_gesture(""), parse_gesture("Alt+Q"));
    }

    #[test]
    fn function_keys_and_modifier_combinations_survive() {
        assert_eq!(
            parse_gesture("Ctrl+Shift+F12"),
            HotKey::from_str("Ctrl+Shift+F12").unwrap()
        );
    }
}
