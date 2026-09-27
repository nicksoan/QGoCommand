//! Port of `QGo.App.Services.TrayIconService`.
//!
//! The menu mirrors the launcher's main commands.
//!
//! `tray-icon` piggybacks on the thread's Win32 message loop, which is the one
//! winit already runs, so the tray must be built on the main thread after
//! eframe has created the event loop — i.e. inside the app creator.

use tray_icon::menu::{ContextMenu, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// The tray commands, mapped from muda's opaque menu ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Settings,
    ManageShortcuts,
    UsageStats,
    Updates,
    About,
    Exit,
}

/// Keeps the tray icon alive; dropping it removes the icon.
pub struct Tray {
    _icon: TrayIcon,
    launcher_menu: Menu,
    items: Vec<(tray_icon::menu::MenuId, TrayCommand)>,
}

impl Tray {
    /// `InitializeTrayIcon()`. The tooltip carries the current hotkey, as in the C#.
    pub fn new(hotkey_gesture: &str) -> Option<Self> {
        let menu = Menu::new();
        let mut items = Vec::new();

        let mut add = |menu: &Menu, label: &str, command: TrayCommand, enabled: bool| {
            let item = MenuItem::new(label, enabled, None);
            let id = item.id().clone();
            if menu.append(&item).is_ok() {
                items.push((id, command));
            }
        };

        add(&menu, "Show QGo", TrayCommand::Show, true);
        let _ = menu.append(&PredefinedMenuItem::separator());
        add(&menu, "Settings...", TrayCommand::Settings, true);
        add(
            &menu,
            "Manage Shortcuts...",
            TrayCommand::ManageShortcuts,
            true,
        );
        add(&menu, "Usage Statistics...", TrayCommand::UsageStats, true);

        let _ = menu.append(&PredefinedMenuItem::separator());
        add(&menu, "About QGo...", TrayCommand::About, true);
        let _ = menu.append(&PredefinedMenuItem::separator());
        add(&menu, "Exit QGo", TrayCommand::Exit, true);

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(format!("QGo - Quick Launcher ({hotkey_gesture})"))
            .with_icon(load_icon()?)
            .build()
            .map_err(|e| log::warn!("could not create the tray icon: {e}"))
            .ok()?;

        let launcher_menu = Menu::new();
        add(&launcher_menu, "Settings...", TrayCommand::Settings, true);
        add(
            &launcher_menu,
            "Manage Shortcuts...",
            TrayCommand::ManageShortcuts,
            true,
        );
        add(
            &launcher_menu,
            "Usage Statistics...",
            TrayCommand::UsageStats,
            true,
        );
        add(&launcher_menu, "About...", TrayCommand::About, true);
        add(&launcher_menu, "Updates", TrayCommand::Updates, true);
        let _ = launcher_menu.append(&PredefinedMenuItem::separator());
        add(&launcher_menu, "Exit", TrayCommand::Exit, true);

        Some(Self {
            _icon: icon,
            launcher_menu,
            items,
        })
    }

    /// Maps an event forwarded by the menu event handler to an app command.
    pub fn command_for(&self, id: &MenuId) -> Option<TrayCommand> {
        self.items
            .iter()
            .find_map(|(item_id, command)| (item_id == id).then_some(*command))
    }

    #[cfg(target_os = "windows")]
    pub fn show_launcher_menu(&self, hwnd: isize) -> bool {
        unsafe { self.launcher_menu.show_context_menu_for_hwnd(hwnd, None) }
    }
}

/// Decodes `Assets\QGoIcon.ico` into the RGBA buffer `tray-icon` wants.
fn load_icon() -> Option<Icon> {
    let bytes = include_bytes!("../assets/QGoIcon.ico");
    let image = image::load_from_memory(bytes)
        .map_err(|e| log::warn!("could not decode the tray icon: {e}"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|e| log::warn!("could not build the tray icon: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_icon_decodes() {
        let bytes = include_bytes!("../assets/QGoIcon.ico");
        let image = image::load_from_memory(bytes).unwrap().into_rgba8();
        assert!(image.dimensions().0 > 0);
        // ICO decoding must produce a square RGBA buffer of the expected length.
        let (w, h) = image.dimensions();
        assert_eq!(image.as_raw().len(), (w * h * 4) as usize);
    }

    #[test]
    fn tray_commands_are_distinct() {
        let all = [
            TrayCommand::Show,
            TrayCommand::Settings,
            TrayCommand::ManageShortcuts,
            TrayCommand::UsageStats,
            TrayCommand::Updates,
            TrayCommand::About,
            TrayCommand::Exit,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
