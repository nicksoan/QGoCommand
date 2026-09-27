//! Port of `MainWindow.xaml` / `MainWindow.xaml.cs`.
//!
//! The window is borderless, always on top, kept out of the taskbar and hidden
//! rather than closed — the same shape as the WPF launcher. eframe 0.35 splits
//! the frame into `logic()`, which runs even while the window is hidden whenever
//! `Context::request_repaint` is called, and `ui()`, which only runs when there
//! is something to draw. That split is what lets the hotkey thread wake a hidden
//! window: `logic()` sees the press and issues `ViewportCommand::Visible(true)`.

use crate::hotkey::HotkeyManager;
use crate::theme::{self, Theme};
use crate::tray::{Tray, TrayCommand};
use crate::windows::{SecondaryWindows, WindowKind};
use egui::{CornerRadius, CursorIcon, Key, Modifiers, ResizeDirection, RichText, Stroke, Vec2};
use qgo_core::matching::Autocomplete;
use qgo_core::{matching, storage, telemetry, LaunchOutcome, LauncherSession, Prepared};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Width and paddings taken from `MainWindow.xaml`.
const MIN_WIDTH: f32 = 300.0;
const MAX_WIDTH: f32 = 800.0;
const SHELL_PADDING: f32 = 3.0;
const CORNER: u8 = 12;
const ROW_CORNER: u8 = 8;
/// `MaxHeight="360"` on the results `ListBox`.
const MAX_LIST_HEIGHT: f32 = 360.0;
const TOAST_DURATION: Duration = Duration::from_secs(4);
const RESIZE_EDGE: f32 = 8.0;

fn clamp_window_width(width: f32) -> f32 {
    width.clamp(MIN_WIDTH, MAX_WIDTH)
}

fn fit_axis(start: i32, size: i32, work_start: i32, work_end: i32) -> (i32, i32) {
    let work_size = (work_end - work_start).max(1);
    let size = size.clamp(1, work_size);
    (start.clamp(work_start, work_end - size), size)
}

fn keep_axis_visible(
    start: i32,
    size: i32,
    work_start: i32,
    work_end: i32,
    minimum_visible: i32,
) -> (i32, i32) {
    let work_size = (work_end - work_start).max(1);
    let size = size.clamp(1, work_size);
    let visible = ((start + size).min(work_end) - start.max(work_start)).clamp(0, size);
    if visible >= minimum_visible.min(size) {
        (start, size)
    } else {
        fit_axis(start, size, work_start, work_end)
    }
}

/// Takes the session lock, tolerating poisoning.
///
/// A panic in a launch worker would otherwise poison the mutex and take the
/// whole app down on the next frame; the data behind it stays consistent
/// because every writer finishes its update before returning.
fn lock(session: &Arc<Mutex<LauncherSession>>) -> std::sync::MutexGuard<'_, LauncherSession> {
    session
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn remove_native_chrome(frame: &eframe::Frame) -> bool {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use std::ffi::c_void;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMNCRENDERINGPOLICY, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR,
            DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE,
            DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
            SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, WS_BORDER,
            WS_CAPTION, WS_DLGFRAME, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE,
            WS_EX_WINDOWEDGE,
        };
        use winit::platform::windows::WindowExtWindows;

        let Some(window) = frame.winit_window() else {
            return false;
        };
        // egui-winit enables this for every undecorated Windows viewport.
        // Winit documents that the shadow adds a thin one-pixel top line.
        window.set_undecorated_shadow(false);

        let Ok(handle) = window.window_handle() else {
            return false;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return false;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut c_void);

        // Keep WS_THICKFRAME, which powers BeginResize, but strip every style
        // that can contribute a one-pixel non-client edge.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let border_styles = (WS_BORDER | WS_CAPTION | WS_DLGFRAME).0 as isize;
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !border_styles);

            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let edge_styles =
                (WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME | WS_EX_STATICEDGE | WS_EX_WINDOWEDGE).0
                    as isize;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style & !edge_styles);

            if let Err(error) = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER,
            ) {
                log::warn!("could not refresh the borderless window frame: {error}");
            }
        }

        // An undecorated winit window still has DWM-managed non-client chrome
        // while it is active. Disable that layer so Windows does not add its
        // own rounded shadow/"bubble" outside the launcher's painted shell.
        let rendering_policy: DWMNCRENDERINGPOLICY = DWMNCRP_DISABLED;
        let border_color = DWMWA_COLOR_NONE;
        let corner_preference: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND;
        let results = unsafe {
            [
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_POLICY,
                    (&rendering_policy as *const DWMNCRENDERINGPOLICY).cast(),
                    std::mem::size_of::<DWMNCRENDERINGPOLICY>() as u32,
                ),
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE,
                    (&corner_preference as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                    std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
                ),
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    (&border_color as *const u32).cast(),
                    std::mem::size_of::<u32>() as u32,
                ),
            ]
        };
        for result in results {
            if let Err(error) = result {
                log::warn!("could not disable native window chrome: {error}");
            }
        }
        true
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = frame;
        true
    }
}

#[cfg(target_os = "windows")]
fn native_hwnd(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = frame.winit_window()?.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy)]
struct WindowDrag {
    cursor_offset_x: i32,
    cursor_offset_y: i32,
}

#[cfg(target_os = "windows")]
fn begin_window_drag(frame: &eframe::Frame) -> Option<WindowDrag> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetCapture;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect};

    let hwnd = HWND(native_hwnd(frame)? as *mut c_void);
    let mut cursor = POINT::default();
    let mut rect = RECT::default();
    unsafe {
        GetCursorPos(&mut cursor).ok()?;
        GetWindowRect(hwnd, &mut rect).ok()?;
        SetCapture(hwnd);
    }

    Some(WindowDrag {
        cursor_offset_x: cursor.x - rect.left,
        cursor_offset_y: cursor.y - rect.top,
    })
}

#[cfg(target_os = "windows")]
fn move_window_with_cursor(frame: &eframe::Frame, drag: WindowDrag) {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    let Some(raw_hwnd) = native_hwnd(frame) else {
        return;
    };
    let hwnd = HWND(raw_hwnd as *mut c_void);
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return;
    }

    if let Err(error) = unsafe {
        SetWindowPos(
            hwnd,
            None,
            cursor.x - drag.cursor_offset_x,
            cursor.y - drag.cursor_offset_y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
        )
    } {
        log::warn!("could not move the launcher window: {error}");
    }
}

#[cfg(target_os = "windows")]
fn keep_window_on_screen(frame: &eframe::Frame) {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    };

    let Some(window) = frame.winit_window() else {
        return;
    };
    let Some(raw_hwnd) = native_hwnd(frame) else {
        return;
    };
    let hwnd = HWND(raw_hwnd as *mut c_void);

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return;
    }

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return;
    }

    let work = monitor_info.rcWork;
    let scale = window.scale_factor() as f32;
    let work_width = work.right - work.left;
    let min_width = ((MIN_WIDTH * scale).round() as i32).min(work_width);
    let max_width = ((MAX_WIDTH * scale).round() as i32)
        .min(work_width)
        .max(min_width);
    let width = (rect.right - rect.left).clamp(min_width, max_width);
    let height = (rect.bottom - rect.top).min(work.bottom - work.top).max(1);
    let minimum_visible_width = (64.0 * scale).round() as i32;
    let minimum_visible_height = (32.0 * scale).round() as i32;
    let (left, width) = keep_axis_visible(
        rect.left,
        width,
        work.left,
        work.right,
        minimum_visible_width,
    );
    let (top, height) = keep_axis_visible(
        rect.top,
        height,
        work.top,
        work.bottom,
        minimum_visible_height,
    );

    if left != rect.left
        || top != rect.top
        || width != rect.right - rect.left
        || height != rect.bottom - rect.top
    {
        if let Err(error) = unsafe {
            SetWindowPos(
                hwnd,
                None,
                left,
                top,
                width,
                height,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOZORDER,
            )
        } {
            log::warn!("could not keep the launcher inside the monitor work area: {error}");
        }
    }
}

/// Things background threads need to tell the UI about.
pub enum AppEvent {
    /// A second process launched and asked us to come to the front.
    ShowRequested,
    /// A system-tray menu item was selected.
    TrayMenu(tray_icon::menu::MenuId),
    /// A launch finished on a worker thread.
    Launched(LaunchOutcome),
}

pub struct QGoApp {
    /// Shared with launch worker threads, which need `&mut` to bump usage counters.
    session: Arc<Mutex<LauncherSession>>,
    search_text: String,
    /// Index into [`Self::visible`], not into the shortcut list.
    selected_row: usize,
    visible: Vec<usize>,
    showing_most_used: bool,
    placeholder: String,
    autocomplete: Autocomplete,
    theme: Theme,
    hotkey: HotkeyManager,
    tray: Option<Tray>,
    window_visible: bool,
    activate_window: bool,
    focus_input: bool,
    /// Set when showing, cleared once the saved position has been applied.
    restore_position: bool,
    logo: Option<egui::TextureHandle>,
    events: Receiver<AppEvent>,
    sender: Sender<AppEvent>,
    windows: SecondaryWindows,
    toast: Option<(String, Instant)>,
    last_height: f32,
    current_width: f32,
    last_scale_factor: Option<f32>,
    last_position: Option<egui::Pos2>,
    geometry_dirty_since: Option<Instant>,
    native_style_applied: bool,
    #[cfg(target_os = "windows")]
    window_drag: Option<WindowDrag>,
    open_context_menu: bool,
    /// `--diagnose`: dump layout and a coarse pixel map, then exit.
    diagnose: bool,
}

impl QGoApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        session: LauncherSession,
        hotkey: HotkeyManager,
        tray: Option<Tray>,
        events: Receiver<AppEvent>,
        sender: Sender<AppEvent>,
        updated_from: Option<String>,
    ) -> Self {
        let theme = Theme::from_settings(&session.settings);
        theme.apply(&cc.egui_ctx);

        let start_hidden = session.settings.start_minimized_to_tray;
        let current_width = if session.settings.window_width.is_finite() {
            clamp_window_width(session.settings.window_width as f32)
        } else {
            420.0
        };
        let logo = load_logo(&cc.egui_ctx);

        let mut windows = SecondaryWindows::default();
        if let Some(previous_version) = updated_from {
            windows.open_update(Some(previous_version));
        }

        let mut app = Self {
            session: Arc::new(Mutex::new(session)),
            search_text: String::new(),
            selected_row: 0,
            visible: Vec::new(),
            showing_most_used: false,
            placeholder: String::new(),
            autocomplete: Autocomplete::default(),
            theme,
            hotkey,
            tray,
            window_visible: !start_hidden,
            activate_window: !start_hidden,
            focus_input: !start_hidden,
            restore_position: false,
            logo,
            events,
            sender,
            windows,
            toast: None,
            last_height: 0.0,
            current_width,
            last_scale_factor: cc.egui_ctx.native_pixels_per_point(),
            last_position: None,
            geometry_dirty_since: None,
            native_style_applied: false,
            #[cfg(target_os = "windows")]
            window_drag: None,
            open_context_menu: false,
            diagnose: std::env::args().any(|a| a == "--diagnose"),
        };

        if !start_hidden {
            app.set_visible(&cc.egui_ctx, true);
        }
        app
    }

    /// `MainWindow.ShowMainWindow()` — show, focus, clear, select.
    fn show_window(&mut self, ctx: &egui::Context) {
        self.search_text.clear();
        self.placeholder.clear();
        self.showing_most_used = false;
        self.selected_row = 0;
        self.autocomplete.reset();
        self.refresh_visible();
        self.set_visible(ctx, true);
        self.activate_window = true;
        self.focus_input = true;
        self.restore_position = true;
    }

    /// `MainWindow.HideMainWindow()`.
    fn hide_window(&mut self, ctx: &egui::Context) {
        self.save_window_geometry(ctx);
        self.search_text.clear();
        self.placeholder.clear();
        self.showing_most_used = false;
        self.autocomplete.reset();
        self.visible.clear();
        self.activate_window = false;
        self.focus_input = false;
        self.set_visible(ctx, false);
    }

    fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        self.window_visible = visible;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
        if visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    fn save_window_geometry(&mut self, ctx: &egui::Context) {
        let (position, width, scale_factor) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.outer_rect.map(|rect| rect.min),
                viewport.inner_rect.map(|rect| rect.width()),
                viewport.native_pixels_per_point,
            )
        });

        let scale_changed = scale_factor
            .zip(self.last_scale_factor)
            .map(|(current, previous)| (current - previous).abs() > 0.01)
            .unwrap_or(false);
        if let Some(scale_factor) = scale_factor {
            self.last_scale_factor = Some(scale_factor);
        }

        let width = if scale_changed {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                self.current_width,
                self.last_height.max(40.0),
            )));
            ctx.request_repaint();
            Some(self.current_width)
        } else {
            width.map(clamp_window_width)
        };
        if let Some(width) = width {
            self.current_width = width;
        }

        let mut session = lock(&self.session);
        if let Some(position) = position {
            session.settings.window_left = position.x as f64;
            session.settings.window_top = position.y as f64;
        }
        if let Some(width) = width {
            session.settings.window_width = width as f64;
        }
        if let Err(error) = storage::save_settings(&session.settings) {
            log::warn!("could not save launcher window geometry: {error}");
        }
        self.geometry_dirty_since = None;
    }

    fn observe_window_geometry(&mut self, ctx: &egui::Context) {
        let (position, width) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.outer_rect.map(|rect| rect.min),
                viewport.inner_rect.map(|rect| rect.width()),
            )
        });

        let width = width.map(clamp_window_width);
        let position_changed = position
            .zip(self.last_position)
            .map(|(current, previous)| current.distance(previous) > 0.5)
            .unwrap_or(position.is_some() != self.last_position.is_some());
        let width_changed = width
            .map(|width| (width - self.current_width).abs() > 0.5)
            .unwrap_or(false);

        if position_changed || width_changed {
            self.last_position = position;
            if let Some(width) = width {
                self.current_width = width;
            }

            let mut session = lock(&self.session);
            if let Some(position) = position {
                session.settings.window_left = position.x as f64;
                session.settings.window_top = position.y as f64;
            }
            if let Some(width) = width {
                session.settings.window_width = width as f64;
            }
            self.geometry_dirty_since = Some(Instant::now());
        }

        if let Some(changed_at) = self.geometry_dirty_since {
            if changed_at.elapsed() >= Duration::from_millis(250) {
                let session = lock(&self.session);
                if let Err(error) = storage::save_settings(&session.settings) {
                    log::warn!("could not save launcher window geometry: {error}");
                }
                self.geometry_dirty_since = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    fn refresh_visible(&mut self) {
        let session = lock(&self.session);
        self.visible = session.filter(&self.search_text, self.showing_most_used);
        self.placeholder = matching::placeholder_for(&session.all, &self.search_text);
        drop(session);
        if self.selected_row >= self.visible.len() {
            self.selected_row = 0;
        }
    }

    /// `MainViewModel.IsListVisible`.
    fn list_visible(&self) -> bool {
        (!self.search_text.trim().is_empty() && !self.visible.is_empty()) || self.showing_most_used
    }

    fn selected_index(&self) -> Option<usize> {
        self.visible.get(self.selected_row).copied()
    }

    fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
    }

    /// Enter: launch, then hide.
    ///
    /// The worker resolves the launch under the lock, **drops it**, and only
    /// then does the blocking part. A sequential chain sleeps for the sum of its
    /// delays, and the render loop locks the same mutex every frame — holding it
    /// across those sleeps would freeze the app for the length of the chain.
    fn launch(&mut self, ctx: &egui::Context) {
        let input = self.search_text.clone();
        let selected = self.selected_index();
        let session = Arc::clone(&self.session);
        let sender = self.sender.clone();
        let ctx = ctx.clone();

        std::thread::spawn(move || {
            let prepared = {
                let mut guard = lock(&session);
                guard.prepare_launch(&input, selected)
            };
            let outcome = match prepared {
                Prepared::Ready(pending) => pending.run(),
                Prepared::Done(outcome) => outcome,
            };
            let _ = sender.send(AppEvent::Launched(outcome));
            ctx.request_repaint();
        });
    }

    /// `HandleUpDownNavigation` — arrows on an empty box reveal the most-used
    /// list, then move within it.
    fn navigate(&mut self, up: bool) {
        if self.search_text.trim().is_empty() && !self.showing_most_used {
            self.showing_most_used = true;
            self.selected_row = 0;
            self.refresh_visible();
            telemetry::instance().record_feature_usage("most_used_shortcuts_shown");
            return;
        }
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len();
        self.selected_row = if up {
            (self.selected_row + len - 1) % len
        } else {
            (self.selected_row + 1) % len
        };
    }

    /// Tab. Matches `MainWindow_KeyDown`: if the highlighted row is not already
    /// what is typed, drop that row into the box; otherwise cycle.
    fn autocomplete(&mut self) {
        let session = lock(&self.session);
        let selected = self.selected_index();

        let direct_fill = selected.filter(|&i| {
            let key = &session.all[i].key;
            self.search_text.is_empty()
                || !self
                    .search_text
                    .to_lowercase()
                    .starts_with(&key.to_lowercase())
        });

        if let Some(i) = direct_fill {
            let shortcut = &session.all[i];
            if shortcut.is_parameterized() {
                self.search_text = format!("{} ", shortcut.key);
                self.placeholder = shortcut.placeholder_text();
            } else {
                self.search_text = shortcut.key.clone();
                self.placeholder.clear();
            }
            return;
        }

        let result =
            self.autocomplete
                .advance(&session.all, &self.search_text, selected, &self.visible);
        drop(session);

        if let Some(result) = result {
            self.search_text = result.search_text;
            self.placeholder = result.placeholder;
            if let Some(row) = self.visible.iter().position(|&i| i == result.chosen) {
                self.selected_row = row;
            }
            telemetry::instance().record_feature_usage("autocomplete");
        }
    }

    fn handle_command(&mut self, ctx: &egui::Context, command: TrayCommand) {
        match command {
            TrayCommand::Show => self.show_window(ctx),
            TrayCommand::Settings => self.windows.open(WindowKind::Settings),
            TrayCommand::ManageShortcuts => self.windows.open(WindowKind::ManageShortcuts),
            TrayCommand::UsageStats => self.windows.open(WindowKind::UsageStats),
            TrayCommand::Updates => self.windows.open(WindowKind::Update),
            TrayCommand::About => self.windows.open(WindowKind::About),
            TrayCommand::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                AppEvent::ShowRequested => self.show_window(ctx),
                AppEvent::TrayMenu(id) => {
                    if let Some(command) = self.tray.as_ref().and_then(|tray| tray.command_for(&id))
                    {
                        self.handle_command(ctx, command);
                    }
                }
                AppEvent::Launched(outcome) => match outcome {
                    LaunchOutcome::NoMatch => {}
                    LaunchOutcome::Launched { .. } => {}
                    LaunchOutcome::ChainLaunched { key, failed } if failed > 0 => {
                        self.toast(format!("Chain '{key}': {failed} item(s) failed to start."));
                        self.set_visible(ctx, true);
                    }
                    LaunchOutcome::ChainLaunched { .. } => {}
                    LaunchOutcome::Failed { key, error } => {
                        self.toast(format!("Could not launch '{key}': {error}"));
                        self.set_visible(ctx, true);
                    }
                },
            }
        }
    }

    fn sync_theme(&mut self, ctx: &egui::Context) {
        let settings_theme = {
            let session = lock(&self.session);
            Theme::from_settings(&session.settings)
        };
        if settings_theme.font_size != self.theme.font_size
            || settings_theme.shell != self.theme.shell
            || settings_theme.foreground != self.theme.foreground
            || settings_theme.surface != self.theme.surface
            || settings_theme.highlight != self.theme.highlight
        {
            self.theme = settings_theme;
            self.theme.apply(ctx);
        }
    }

    #[cfg(target_os = "windows")]
    fn update_window_drag(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(drag) = self.window_drag else {
            return;
        };

        if ctx.input(|input| input.pointer.primary_down()) {
            move_window_with_cursor(frame, drag);
            ctx.set_cursor_icon(CursorIcon::Grabbing);
            ctx.request_repaint();
        } else {
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
            }
            self.window_drag = None;
        }
    }

    /// Consumes the navigation keys before the text box can act on them.
    ///
    /// Tab in particular has to be taken away from egui, which would otherwise
    /// use it to move focus between widgets.
    fn read_keys(&mut self, ctx: &egui::Context) -> Option<LauncherKey> {
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                Some(LauncherKey::Escape)
            } else if i.consume_key(Modifiers::NONE, Key::Enter) {
                Some(LauncherKey::Enter)
            } else if i.consume_key(Modifiers::NONE, Key::Tab) {
                Some(LauncherKey::Tab)
            } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                Some(LauncherKey::Up)
            } else if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                Some(LauncherKey::Down)
            } else if i.consume_key(Modifiers::NONE, Key::F10) {
                Some(LauncherKey::ContextMenu)
            } else {
                None
            }
        })
    }

    /// Applies the saved position and the measured height to the OS window.
    fn size_and_place(&mut self, ctx: &egui::Context, content_height: f32) {
        let (left, top) = {
            let session = lock(&self.session);
            let s = &session.settings;
            (s.window_left as f32, s.window_top as f32)
        };

        let height = (content_height + SHELL_PADDING * 2.0).max(40.0);
        if (height - self.last_height).abs() > 1.0 {
            self.last_height = height;
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(Vec2::new(
                MIN_WIDTH, height,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(Vec2::new(
                MAX_WIDTH, height,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                self.current_width,
                height,
            )));
        }

        // Restore the stored position once per show. This cannot key off
        // `focus_input`: `search_row` clears that earlier in the same frame.
        if self.restore_position {
            self.restore_position = false;
            if left.is_finite() && top.is_finite() {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(left, top)));
                ctx.request_repaint();
            }
        }
    }

    fn handle_window_edges(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let pointer = ctx.input(|input| {
            (
                input.pointer.hover_pos(),
                input.pointer.primary_pressed(),
                input.pointer.secondary_pressed(),
            )
        });

        let Some(pointer_position) = pointer.0 else {
            return;
        };

        if pointer.2 && rect.contains(pointer_position) {
            self.open_context_menu = true;
            ctx.request_repaint();
        }

        let left_edge = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.left() + RESIZE_EDGE, rect.bottom()),
        );
        let right_edge =
            egui::Rect::from_min_max(egui::pos2(rect.right() - RESIZE_EDGE, rect.top()), rect.max);

        let direction = if left_edge.contains(pointer_position) {
            Some(ResizeDirection::West)
        } else if right_edge.contains(pointer_position) {
            Some(ResizeDirection::East)
        } else {
            None
        };

        if let Some(direction) = direction {
            ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
            if pointer.1 {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
            }
        }
    }
}

enum LauncherKey {
    Escape,
    Enter,
    Tab,
    Up,
    Down,
    ContextMenu,
}

impl eframe::App for QGoApp {
    /// Runs even while the window is hidden, whenever something calls
    /// `request_repaint` — which is exactly how the hotkey gets in.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.handle_events(ctx);
        self.sync_theme(ctx);
        #[cfg(target_os = "windows")]
        self.update_window_drag(ctx, frame);
        if self.window_visible {
            self.observe_window_geometry(ctx);
        }

        if !self.native_style_applied {
            self.native_style_applied = remove_native_chrome(frame);
        }

        #[cfg(target_os = "windows")]
        if self.window_visible && !ctx.input(|input| input.pointer.primary_down()) {
            keep_window_on_screen(frame);
        }

        if self.open_context_menu {
            self.open_context_menu = false;
            #[cfg(target_os = "windows")]
            if let (Some(tray), Some(hwnd)) = (&self.tray, native_hwnd(frame)) {
                tray.show_launcher_menu(hwnd);
            }
        }

        if self.window_visible && self.activate_window {
            if let Some(window) = frame.winit_window() {
                window.set_visible(true);
                window.focus_window();
                if window.has_focus() {
                    self.activate_window = false;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(25));
                }
            }
        }

        if let Some((_, shown_at)) = &self.toast {
            if shown_at.elapsed() > TOAST_DURATION {
                self.toast = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }

        // Settings / Manage Shortcuts / ... are drawn here rather than in
        // `ui()`, because eframe skips `ui()` altogether while the root
        // viewport is hidden (`if is_visible { app.ui(..) }` in
        // `epi_integration.rs`). Opening one from the tray is the normal case,
        // and the launcher is hidden exactly then.
        self.render_secondary(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent, so the rounded shell's corners are genuinely see-through
        // — `AllowsTransparency="True"` on the WPF window.
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // A hidden window still gets one frame after `Visible(false)`; skip it.
        if !self.window_visible {
            return;
        }

        match self.read_keys(&ctx) {
            Some(LauncherKey::Escape) => {
                self.hide_window(&ctx);
                return;
            }
            Some(LauncherKey::Enter) => {
                self.launch(&ctx);
                self.hide_window(&ctx);
                return;
            }
            Some(LauncherKey::Tab) => self.autocomplete(),
            Some(LauncherKey::Up) => self.navigate(true),
            Some(LauncherKey::Down) => self.navigate(false),
            Some(LauncherKey::ContextMenu) => {
                self.open_context_menu = true;
                ctx.request_repaint();
            }
            None => {}
        }

        let theme = self.theme;
        let shell = egui::Frame::new()
            .fill(theme.shell)
            .corner_radius(CornerRadius::same(CORNER))
            .inner_margin(SHELL_PADDING);

        if self.diagnose {
            self.dump_frame(&ctx, ui, theme);
        }

        let response = shell.show(ui, |ui| {
            self.search_row(ui, _frame, theme);
            if self.list_visible() {
                ui.add_space(8.0);
                self.results(ui, theme);
            }
            if let Some((message, _)) = self.toast.clone() {
                ui.add_space(6.0);
                ui.colored_label(theme::GLOW, RichText::new(message).small());
            }
        });

        self.handle_window_edges(&ctx, response.response.rect);

        self.size_and_place(&ctx, response.response.rect.height());
    }
}

impl QGoApp {
    /// The `SearchRow` border from `MainWindow.xaml`: logo, text box, hint.
    fn search_row(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, theme: Theme) {
        egui::Frame::new()
            .fill(theme.surface)
            .corner_radius(CornerRadius::same(ROW_CORNER))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(logo) = &self.logo {
                        let logo = ui.add(
                            egui::Image::new(logo)
                                .fit_to_exact_size(Vec2::splat(24.0))
                                .corner_radius(4)
                                .sense(egui::Sense::drag()),
                        );
                        if logo.hovered() && ui.input(|input| input.pointer.primary_pressed()) {
                            #[cfg(target_os = "windows")]
                            {
                                self.window_drag = begin_window_drag(frame);
                                ui.ctx().request_repaint();
                            }
                            #[cfg(not(target_os = "windows"))]
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        let cursor = if logo.dragged() {
                            CursorIcon::Grabbing
                        } else {
                            CursorIcon::Grab
                        };
                        logo.on_hover_cursor(cursor);
                        ui.add_space(6.0);
                    }

                    let previous = self.search_text.clone();
                    let available = ui.available_width() - 4.0;
                    let edit = ui.add_sized(
                        Vec2::new(available, theme.font_size + 6.0),
                        egui::TextEdit::singleline(&mut self.search_text)
                            // The surrounding Frame already draws the search row.
                            .frame(egui::Frame::NONE)
                            .desired_width(available)
                            .text_color(theme.foreground)
                            .hint_text(
                                RichText::new(if self.placeholder.is_empty() {
                                    "Type to search..."
                                } else {
                                    self.placeholder.trim()
                                })
                                .color(theme.foreground.gamma_multiply(0.6)),
                            ),
                    );

                    if self.focus_input {
                        edit.request_focus();
                        self.focus_input = !edit.has_focus();
                        if self.focus_input {
                            ui.ctx().request_repaint();
                        }
                    }

                    if self.search_text != previous {
                        if self.showing_most_used && !self.search_text.is_empty() {
                            self.showing_most_used = false;
                        }
                        self.autocomplete.reset();
                        self.selected_row = 0;
                        self.refresh_visible();
                    }
                });
            });
    }

    /// The results `ListBox`, capped at `MaxHeight="360"`.
    fn results(&mut self, ui: &mut egui::Ui, theme: Theme) {
        let rows: Vec<(usize, String, bool)> = {
            let session = lock(&self.session);
            self.visible
                .iter()
                .filter_map(|&i| session.all.get(i).map(|s| (i, s.key.clone(), s.is_chained)))
                .collect()
        };

        egui::Frame::new()
            .fill(theme.surface)
            .corner_radius(CornerRadius::same(ROW_CORNER))
            .stroke(Stroke::new(1.0, theme::BORDER))
            .inner_margin(4)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(MAX_LIST_HEIGHT)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for (row, (_, key, is_chained)) in rows.iter().enumerate() {
                            let selected = row == self.selected_row;
                            let label = if *is_chained {
                                RichText::new(format!("{key}  ⛓")).color(theme.foreground)
                            } else {
                                RichText::new(key).color(theme.foreground)
                            };

                            let response = ui.selectable_label(selected, label);
                            if response.clicked() {
                                self.selected_row = row;
                            }
                            if response.double_clicked() {
                                let ctx = ui.ctx().clone();
                                self.selected_row = row;
                                self.launch(&ctx);
                                self.hide_window(&ctx);
                            }
                        }
                    });
            });
    }

    /// Settings / Manage Shortcuts / Chains / Stats / About, each as its own OS
    /// window via an immediate viewport.
    fn render_secondary(&mut self, ctx: &egui::Context) {
        let reload = self.windows.render(ctx, &self.session, self.theme);
        if reload {
            let mut session = lock(&self.session);
            let items = storage::load_links_or_defaults();
            session.reload_from(items);
            session.settings = storage::load_settings();
            drop(session);
            self.hotkey.register(&self.hotkey_gesture());
            self.refresh_visible();
        }
    }

    fn hotkey_gesture(&self) -> String {
        self.session
            .lock()
            .map(|s| s.settings.hotkey_gesture.clone())
            .unwrap_or_else(|_| "Alt+Q".to_string())
    }
}

impl QGoApp {
    /// `--diagnose`: prove what the app actually painted.
    ///
    /// Logs the resolved geometry and theme, then grabs the framebuffer and
    /// prints it as coarse ASCII luminance, so a rendering complaint can be
    /// checked without anyone having to describe what they see. Exits when the
    /// screenshot arrives.
    fn dump_frame(&mut self, ctx: &egui::Context, ui: &egui::Ui, theme: Theme) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static FRAME: AtomicU32 = AtomicU32::new(0);

        let n = FRAME.fetch_add(1, Ordering::SeqCst);
        if n < 4 {
            log::info!(
                "diagnose frame {n}: viewport={:?} ui_max={:?} avail_w={}                  pixels_per_point={} font={} shell={:?} fg={:?} visible={} list_visible={}",
                ctx.viewport_rect(),
                ui.max_rect(),
                ui.available_width(),
                ctx.pixels_per_point(),
                theme.font_size,
                theme.shell,
                theme.foreground,
                self.window_visible,
                self.list_visible(),
            );
        }
        if n == 3 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }

        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });

        if let Some(image) = shot {
            let (w, h) = (image.width(), image.height());
            let px = image.as_raw();
            let (xstep, ystep) = ((w / 70).max(1), (h / 14).max(1));
            let mut art = String::new();
            for y in (0..h).step_by(ystep) {
                for x in (0..w).step_by(xstep) {
                    let i = (y * w + x) * 4;
                    let (r, g, b) = (px[i] as u32, px[i + 1] as u32, px[i + 2] as u32);
                    // Rec. 601 luma, bucketed dark-to-light.
                    art.push(match (r * 299 + g * 587 + b * 114) / 1000 {
                        0..=25 => '.',
                        26..=60 => ':',
                        61..=120 => '+',
                        121..=190 => '*',
                        _ => '#',
                    });
                }
                art.push('\n');
            }
            log::info!(
                "diagnose framebuffer {w}x{h}:
{art}"
            );
            std::process::exit(0);
        }
    }
}

/// `MainViewModel.LogoSource`, decoded into a texture once at startup.
fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    use chrono::Datelike;

    let bytes = theme::logo_bytes(chrono::Local::now().month());
    let image = image::load_from_memory(bytes)
        .map_err(|e| log::warn!("could not decode the logo: {e}"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], image.as_raw());
    Some(ctx.load_texture("qgo-logo", color_image, egui::TextureOptions::LINEAR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_axis_is_clamped_to_negative_coordinate_monitor() {
        assert_eq!(fit_axis(-2_200, 420, -1_920, 0), (-1_920, 420));
        assert_eq!(fit_axis(-100, 420, -1_920, 0), (-420, 420));
    }

    #[test]
    fn oversized_window_is_reduced_to_work_area() {
        assert_eq!(fit_axis(-50, 2_500, -1_920, 0), (-1_920, 1_920));
    }

    #[test]
    fn visible_window_can_span_monitor_edges() {
        assert_eq!(keep_axis_visible(-200, 420, 0, 1_920, 64), (-200, 420));
    }

    #[test]
    fn nearly_lost_window_is_brought_back() {
        assert_eq!(keep_axis_visible(-400, 420, 0, 1_920, 64), (0, 420));
        assert_eq!(keep_axis_visible(1_900, 420, 0, 1_920, 64), (1_500, 420));
    }

    #[test]
    fn launcher_width_is_clamped_to_the_supported_range() {
        assert_eq!(clamp_window_width(200.0), MIN_WIDTH);
        assert_eq!(clamp_window_width(420.0), 420.0);
        assert_eq!(clamp_window_width(1_000.0), MAX_WIDTH);
    }
}
