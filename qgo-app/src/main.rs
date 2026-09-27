//! QGo - keyboard-driven shortcut launcher.
//!
//! Rust port of `QGo.App` (WPF). This file is the equivalent of `App.xaml.cs`:
//! enforce a single instance, register the global hotkey, put an icon in the
//! tray, then hand off to the launcher window.

// No console window, matching `<OutputType>WinExe</OutputType>`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod hotkey;
mod single_instance;
mod theme;
mod tray;
mod windows;

use app::{AppEvent, QGoApp};
use global_hotkey::HotKeyState;
use hotkey::HotkeyManager;
use qgo_core::{storage, telemetry, LauncherSession};
use single_instance::Acquisition;
use std::sync::mpsc;

/// `MainWindow.xaml`'s `Width="420"`.
const DEFAULT_WIDTH: f32 = 420.0;
const DEFAULT_HEIGHT: f32 = 56.0;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // `App.xaml.cs::OnStartup` — a second launch just wakes the first one.
    let _guard = match single_instance::acquire() {
        Acquisition::AlreadyRunning => {
            log::info!("QGo is already running; asked the existing instance to show itself");
            return Ok(());
        }
        Acquisition::Acquired(guard) => guard,
    };

    let session = LauncherSession::load();
    let settings = session.settings.clone();

    telemetry::instance().record_app_launch();
    let updated_from = record_version_if_changed(settings.last_run_version.as_deref());

    let (sender, receiver) = mpsc::channel::<AppEvent>();

    let width = if settings.window_width.is_finite() {
        (settings.window_width as f32).clamp(300.0, 800.0)
    } else {
        DEFAULT_WIDTH
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("QGo - Quick Launcher")
        .with_inner_size([width, DEFAULT_HEIGHT])
        .with_min_inner_size([300.0, DEFAULT_HEIGHT])
        .with_max_inner_size([800.0, DEFAULT_HEIGHT])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(true)
        .with_always_on_top()
        // `ShowInTaskbar="False"` — the launcher lives in the tray.
        .with_taskbar(false)
        .with_visible(!settings.start_minimized_to_tray);

    if settings.has_window_position() {
        viewport =
            viewport.with_position([settings.window_left as f32, settings.window_top as f32]);
    }

    let options = eframe::NativeOptions {
        viewport,
        // The launcher's own settings.json is the source of truth; eframe's
        // window-state persistence would fight it.
        persist_window: false,
        ..Default::default()
    };

    let hotkey_gesture = settings.hotkey_gesture.clone();
    eframe::run_native(
        "QGo",
        options,
        Box::new(move |cc| {
            // `tray-icon` and `global-hotkey` both need the main thread's
            // message loop, which winit has just created.
            let mut hotkey_manager = HotkeyManager::new()?;
            if !hotkey_manager.register(&hotkey_gesture) {
                log::warn!("'{hotkey_gesture}' is unavailable - another app may already own it");
            }

            // Event handlers replace the crates' receiver channels. Forward
            // the actual events into the app as well as waking the render loop.
            let hotkey_id = hotkey_manager.id();
            let ctx = cc.egui_ctx.clone();
            let hotkey_sender = sender.clone();
            global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(
                move |event: global_hotkey::GlobalHotKeyEvent| {
                    if event.state == HotKeyState::Pressed && Some(event.id) == hotkey_id {
                        let _ = hotkey_sender.send(AppEvent::ShowRequested);
                        ctx.request_repaint();
                    }
                },
            ));

            let ctx = cc.egui_ctx.clone();
            let tray_sender = sender.clone();
            tray_icon::menu::MenuEvent::set_event_handler(Some(
                move |event: tray_icon::menu::MenuEvent| {
                    let _ = tray_sender.send(AppEvent::TrayMenu(event.id));
                    ctx.request_repaint();
                },
            ));

            let tray = tray::Tray::new(&hotkey_gesture);
            if tray.is_none() {
                log::warn!("running without a tray icon");
            }

            // A second launch signals a named event; wake up and show.
            let ctx = cc.egui_ctx.clone();
            let show_sender = sender.clone();
            single_instance::listen_for_show_requests(move || {
                let _ = show_sender.send(AppEvent::ShowRequested);
                ctx.request_repaint();
            });

            Ok(Box::new(QGoApp::new(
                cc,
                session,
                hotkey_manager,
                tray,
                receiver,
                sender,
                updated_from,
            )))
        }),
    )
}

/// `MainWindow` writes `LastRunVersion` so the update notification only appears
/// once per version change.
///
/// Re-reads from disk so settings changed during startup are preserved.
fn record_version_if_changed(previous: Option<&str>) -> Option<String> {
    let current = qgo_core::version::VERSION;
    let updated_from = update_notification_previous(previous);
    if previous == Some(current) {
        return None;
    }
    let mut settings = storage::load_settings();
    settings.last_run_version = Some(current.to_string());
    if let Err(e) = storage::save_settings(&settings) {
        log::warn!("could not record the last-run version: {e}");
    }
    updated_from
}

fn update_notification_previous(previous: Option<&str>) -> Option<String> {
    previous
        .filter(|version| *version != qgo_core::version::VERSION)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_window_size_matches_the_xaml() {
        // `Width="420"` in MainWindow.xaml; the height is driven by content.
        assert_eq!(DEFAULT_WIDTH, 420.0);
        assert_eq!(DEFAULT_HEIGHT, 56.0);
    }

    #[test]
    fn a_matching_version_is_not_rewritten() {
        // The early return is the behaviour under test: it must not touch disk.
        assert_eq!(
            record_version_if_changed(Some(qgo_core::version::VERSION)),
            None
        );
    }

    #[test]
    fn update_notification_only_has_a_previous_version_after_a_change() {
        assert_eq!(update_notification_previous(None), None);
        assert_eq!(
            update_notification_previous(Some(qgo_core::version::VERSION)),
            None
        );
        assert_eq!(
            update_notification_previous(Some("2.0.0")),
            Some("2.0.0".to_string())
        );
    }
}
