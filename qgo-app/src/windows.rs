//! The secondary windows, ported from `QGo.App/Views`.
//!
//! WPF gives each of these its own `Window` subclass with a XAML file. egui's
//! equivalent is an *immediate viewport*: a real OS window whose contents are
//! rebuilt inside the parent's frame, which keeps all the state in one struct
//! instead of spreading it across code-behind files.
//!
//! Each `show_*` returns `true` when it changed something on disk, so the
//! launcher knows to reload.

use crate::theme::Theme;
use egui::{Context, ViewportBuilder, ViewportId};
use qgo_core::models::{ChainedShortcut, ExecutionMode, Shortcut, ShortcutChainItem};
use qgo_core::{startup, storage, telemetry, LauncherSession};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    Settings,
    ManageShortcuts,
    ChainedShortcuts,
    UsageStats,
    Update,
    About,
}

/// Which windows are open, plus their scratch state.
#[derive(Default)]
pub struct SecondaryWindows {
    settings: Option<SettingsState>,
    manage: Option<ManageState>,
    chains: Option<ChainsState>,
    stats: bool,
    update: Option<UpdateState>,
    about: bool,
}

#[derive(Debug, Clone)]
struct UpdateState {
    previous_version: Option<String>,
}

impl SecondaryWindows {
    pub fn open(&mut self, kind: WindowKind) {
        match kind {
            WindowKind::Settings => {
                if self.settings.is_none() {
                    self.settings = Some(SettingsState::load());
                }
            }
            WindowKind::ManageShortcuts => {
                if self.manage.is_none() {
                    self.manage = Some(ManageState::load());
                }
            }
            WindowKind::ChainedShortcuts => {
                if self.chains.is_none() {
                    self.chains = Some(ChainsState::load());
                }
            }
            WindowKind::UsageStats => self.stats = true,
            WindowKind::Update => self.open_update(None),
            WindowKind::About => self.about = true,
        }
    }

    pub fn open_update(&mut self, previous_version: Option<String>) {
        self.update = Some(UpdateState { previous_version });
    }

    /// Draws every open window. Returns `true` when something was saved.
    pub fn render(
        &mut self,
        ctx: &Context,
        session: &Arc<Mutex<LauncherSession>>,
        theme: Theme,
    ) -> bool {
        let mut changed = false;

        if let Some(state) = &mut self.settings {
            let (keep, saved) = show_settings(ctx, state, theme);
            changed |= saved;
            if !keep {
                self.settings = None;
            }
        }

        if let Some(state) = &mut self.manage {
            let (keep, saved, open_chains) = show_manage(ctx, state, theme);
            changed |= saved;
            if open_chains {
                self.open(WindowKind::ChainedShortcuts);
            }
            if !keep {
                self.manage = None;
            }
        }

        if let Some(state) = &mut self.chains {
            let (keep, saved) = show_chains(ctx, state);
            changed |= saved;
            if !keep {
                self.chains = None;
            }
        }

        if self.stats && !show_stats(ctx, session) {
            self.stats = false;
        }

        if let Some(state) = &self.update {
            if !show_update(ctx, state, theme) {
                self.update = None;
            }
        }

        if self.about && !show_about(ctx, theme) {
            self.about = false;
        }

        changed
    }
}

fn window_header(ui: &mut egui::Ui, title: &str, subtitle: &str, theme: Theme) {
    ui.label(
        egui::RichText::new(title)
            .size(theme.font_size * 1.55)
            .strong()
            .color(theme.foreground),
    );
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(subtitle)
            .size(theme.font_size * 0.88)
            .color(theme.foreground.gamma_multiply(0.65)),
    );
    ui.add_space(16.0);
}

fn section_card<R>(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    theme: Theme,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(theme.surface)
        .stroke(egui::Stroke::new(1.0, crate::theme::BORDER))
        .corner_radius(10)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(theme.font_size * 1.05)
                    .strong(),
            );
            if !subtitle.is_empty() {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(subtitle)
                        .small()
                        .color(theme.foreground.gamma_multiply(0.6)),
                );
            }
            ui.add_space(12.0);
            contents(ui)
        })
        .inner
}

fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(label)
                .strong()
                .color(egui::Color32::WHITE),
        )
        .fill(crate::theme::ACCENT)
        .corner_radius(7)
        .min_size(egui::vec2(96.0, 34.0)),
    )
}

fn secondary_button(ui: &mut egui::Ui, label: &str, theme: Theme) -> egui::Response {
    ui.add(
        egui::Button::new(label)
            .fill(theme.shell)
            .stroke(egui::Stroke::new(1.0, crate::theme::BORDER))
            .corner_radius(7)
            .min_size(egui::vec2(96.0, 34.0)),
    )
}

fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE))
            .fill(egui::Color32::from_rgb(0xB9, 0x3B, 0x3B))
            .corner_radius(7)
            .min_size(egui::vec2(90.0, 34.0)),
    )
}

fn status_banner(ui: &mut egui::Ui, message: &str, theme: Theme) {
    if message.is_empty() {
        return;
    }
    let is_error = message.starts_with("Could not") || message.starts_with("Failed");
    let colour = if is_error {
        egui::Color32::from_rgb(0xF0, 0x7A, 0x7A)
    } else {
        crate::theme::GLOW
    };
    egui::Frame::new()
        .fill(colour.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, colour.gamma_multiply(0.55)))
        .corner_radius(7)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.colored_label(colour, message);
        });
    ui.add_space(theme.font_size * 0.2);
}

/// Wraps the boilerplate: build the viewport, run `contents`, report whether the
/// user closed it.
fn viewport<R>(
    ctx: &Context,
    id: &'static str,
    title: &str,
    size: [f32; 2],
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let mut keep_open = true;
    let mut result = None;
    // `show_viewport_immediate` wants an `FnMut`, so the one-shot body has to be
    // handed over through an Option that the first call takes.
    let mut contents = Some(contents);

    ctx.show_viewport_immediate(
        ViewportId::from_hash_of(id),
        ViewportBuilder::default()
            .with_title(title)
            .with_inner_size(size),
        |ctx, _class| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(contents) = contents.take() {
                        result = Some(contents(ui));
                    }
                });
            });
            if ctx.input(|i| i.viewport().close_requested()) {
                keep_open = false;
            }
        },
    );

    (keep_open, result.expect("viewport contents ran"))
}

// ---------------------------------------------------------------- Settings --

/// Port of `Views/SettingsWindow.xaml`.
pub struct SettingsState {
    settings: qgo_core::AppSettings,
    message: String,
}

impl SettingsState {
    fn load() -> Self {
        Self {
            settings: storage::load_settings(),
            message: String::new(),
        }
    }
}

fn show_settings(ctx: &Context, state: &mut SettingsState, theme: Theme) -> (bool, bool) {
    let (keep, saved) = viewport(ctx, "qgo-settings", "QGo Settings", [820.0, 720.0], |ui| {
        let mut saved = false;

        egui::Frame::new()
            .fill(theme.shell)
            .inner_margin(20)
            .show(ui, |ui| {
                window_header(
                    ui,
                    "Settings",
                    "Personalize QGo and control how the launcher behaves.",
                    theme,
                );

                status_banner(ui, &state.message, theme);

                ui.columns(2, |columns| {
                    section_card(
                        &mut columns[0],
                        "Launcher",
                        "Keyboard activation and startup behavior",
                        theme,
                        |ui| {
                            ui.label("Global hotkey");
                            ui.add(
                                egui::TextEdit::singleline(&mut state.settings.hotkey_gesture)
                                    .desired_width(f32::INFINITY)
                                    .background_color(theme.shell),
                            );
                            ui.label(
                                egui::RichText::new("Examples: Alt+Q, Ctrl+Shift+Space, Win+Q")
                                    .small()
                                    .color(theme.foreground.gamma_multiply(0.55)),
                            );
                            ui.add_space(12.0);
                            ui.checkbox(
                                &mut state.settings.start_with_windows,
                                "Start QGo with Windows",
                            );
                            ui.checkbox(
                                &mut state.settings.minimize_to_tray,
                                "Hide to the system tray",
                            );
                            ui.checkbox(
                                &mut state.settings.start_minimized_to_tray,
                                "Start hidden in the tray",
                            );
                            ui.checkbox(
                                &mut state.settings.show_welcome_on_startup,
                                "Show welcome screen on startup",
                            );
                        },
                    );

                    columns[0].add_space(12.0);
                    section_card(
                        &mut columns[0],
                        "Appearance",
                        "Colors and typography used by the launcher",
                        theme,
                        |ui| {
                            colour_row(ui, "Shell", &mut state.settings.shell_colour, theme);
                            colour_row(
                                ui,
                                "Foreground",
                                &mut state.settings.foreground,
                                theme,
                            );
                            colour_row(
                                ui,
                                "Highlight",
                                &mut state.settings.highlight,
                                theme,
                            );
                            colour_row(ui, "Surface", &mut state.settings.surface, theme);
                            ui.add_space(8.0);
                            ui.label("Font family");
                            ui.add(
                                egui::TextEdit::singleline(&mut state.settings.font_family)
                                    .desired_width(f32::INFINITY)
                                    .background_color(theme.shell),
                            );
                            ui.add_space(6.0);
                            ui.add(
                                egui::Slider::new(&mut state.settings.font_size, 10.0..=32.0)
                                    .text("Font size")
                                    .step_by(1.0),
                            );
                        },
                    );

                    section_card(
                        &mut columns[1],
                        "Accessibility",
                        "Adjust QGo for comfort and assistive technology",
                        theme,
                        |ui| {
                            ui.add(
                                egui::Slider::new(
                                    &mut state.settings.accessibility_font_scale,
                                    1.0..=2.0,
                                )
                                .text("Interface scale"),
                            );
                            ui.checkbox(
                                &mut state.settings.enable_high_contrast,
                                "High contrast",
                            );
                            ui.checkbox(
                                &mut state.settings.reduce_animations,
                                "Reduce animations",
                            );
                            ui.checkbox(
                                &mut state.settings.enable_screen_reader_support,
                                "Screen reader support",
                            );
                        },
                    );

                    columns[1].add_space(12.0);
                    section_card(
                        &mut columns[1],
                        "Privacy",
                        "Choose what diagnostic information QGo may send",
                        theme,
                        |ui| {
                            ui.checkbox(
                                &mut state.settings.enable_telemetry,
                                "Send anonymous usage telemetry",
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Your shortcut history and local usage statistics stay on this device.",
                                )
                                .small()
                                .color(theme.foreground.gamma_multiply(0.55)),
                            );
                        },
                    );
                });

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, "Save changes").clicked() {
                        match storage::save_settings(&merge_owned_fields(&state.settings)) {
                            Ok(()) => {
                                startup::set_run_at_startup(state.settings.start_with_windows);
                                state.message = "Settings saved successfully.".into();
                                saved = true;
                            }
                            Err(e) => state.message = format!("Could not save settings: {e}"),
                        }
                    }
                    if secondary_button(ui, "Reset defaults", theme).clicked() {
                        state.settings = qgo_core::AppSettings::default();
                        state.message = "Defaults restored. Save to apply them.".into();
                    }
                });
            });

        saved
    });

    (keep, saved)
}

/// Folds the Settings window's edits onto a **fresh** read of `settings.json`.
///
/// `SettingsState` snapshots the file when the window opens, so writing that
/// snapshot back would revert anything changed in the meantime. Only the fields
/// this window actually presents are copied across.
fn merge_owned_fields(edited: &qgo_core::AppSettings) -> qgo_core::AppSettings {
    let mut disk = storage::load_settings();

    disk.hotkey_gesture = edited.hotkey_gesture.clone();
    disk.shell_colour = edited.shell_colour.clone();
    disk.foreground = edited.foreground.clone();
    disk.highlight = edited.highlight.clone();
    disk.surface = edited.surface.clone();
    disk.font_size = edited.font_size;
    disk.font_family = edited.font_family.clone();
    disk.start_with_windows = edited.start_with_windows;
    disk.show_welcome_on_startup = edited.show_welcome_on_startup;
    disk.show_glow_on_startup = edited.show_glow_on_startup;
    disk.enable_telemetry = edited.enable_telemetry;
    disk.enable_screen_reader_support = edited.enable_screen_reader_support;
    disk.enable_high_contrast = edited.enable_high_contrast;
    disk.enable_sound_feedback = edited.enable_sound_feedback;
    disk.reduce_animations = edited.reduce_animations;
    disk.enhanced_keyboard_navigation = edited.enhanced_keyboard_navigation;
    disk.accessibility_font_scale = edited.accessibility_font_scale;
    disk.accessibility_timeout = edited.accessibility_timeout;
    disk.minimize_to_tray = edited.minimize_to_tray;
    disk.start_minimized_to_tray = edited.start_minimized_to_tray;

    // Deliberately not copied because this window does not own them:
    // last_run_version, telemetry_consent_shown, has_shown_tray_notification,
    // and the window geometry.
    disk
}

fn colour_row(ui: &mut egui::Ui, label: &str, value: &mut String, theme: Theme) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(130.0)
                .background_color(theme.shell),
        );
        if let Some(colour) = crate::theme::parse_color(value) {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 20.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 5.0, colour);
            ui.painter().rect_stroke(
                rect,
                5.0,
                egui::Stroke::new(1.0, crate::theme::BORDER),
                egui::StrokeKind::Inside,
            );
        }
    });
}

// -------------------------------------------------------- Manage shortcuts --

/// Port of `Views/ManageShortcutsWindow.xaml`.
pub struct ManageState {
    shortcuts: Vec<Shortcut>,
    selected: Option<usize>,
    message: String,
}

impl ManageState {
    fn load() -> Self {
        let shortcuts: Vec<_> = storage::load_links_or_defaults()
            .into_iter()
            .filter(|s| !s.is_chained)
            .collect();
        Self {
            // Chain proxies live in `chained-shortcuts.json` and are rebuilt on
            // load, so they are not editable here — the same split the WPF app has.
            selected: (!shortcuts.is_empty()).then_some(0),
            shortcuts,
            message: String::new(),
        }
    }
}

fn show_manage(ctx: &Context, state: &mut ManageState, theme: Theme) -> (bool, bool, bool) {
    let (keep, (saved, open_chains)) = viewport(
        ctx,
        "qgo-manage",
        "QGo - Manage Shortcuts",
        [900.0, 700.0],
        |ui| {
            let mut saved = false;
            let mut open_chains = false;

            egui::Frame::new()
                .fill(theme.shell)
                .inner_margin(20)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Manage Shortcuts")
                                    .size(theme.font_size * 1.55)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Create memorable keywords for websites, files, folders, and commands.",
                                )
                                .size(theme.font_size * 0.88)
                                .color(theme.foreground.gamma_multiply(0.65)),
                            );
                        });
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if secondary_button(ui, "Chains...", theme).clicked() {
                                    open_chains = true;
                                }
                                if primary_button(ui, "+ Add shortcut").clicked() {
                                    state.shortcuts.push(Shortcut::default());
                                    state.selected = Some(state.shortcuts.len() - 1);
                                }
                            },
                        );
                    });

                    ui.add_space(14.0);
                    status_banner(ui, &state.message, theme);

                    let mut remove = None;
                    ui.columns(2, |columns| {
                        columns[0].set_width(250.0);
                        section_card(
                            &mut columns[0],
                            "Shortcuts",
                            &format!("{} configured", state.shortcuts.len()),
                            theme,
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("shortcut-list")
                                    .max_height(470.0)
                                    .show(ui, |ui| {
                                        for (index, shortcut) in state.shortcuts.iter().enumerate() {
                                            let selected = state.selected == Some(index);
                                            let response = ui.add_sized(
                                                [ui.available_width(), 38.0],
                                                egui::Button::new(
                                                    egui::RichText::new(if shortcut.key.trim().is_empty() {
                                                        "Untitled shortcut"
                                                    } else {
                                                        &shortcut.key
                                                    })
                                                    .color(theme.foreground),
                                                )
                                                .selected(selected)
                                                .fill(if selected {
                                                    crate::theme::ACCENT.gamma_multiply(0.32)
                                                } else {
                                                    theme.shell
                                                })
                                                .stroke(egui::Stroke::new(
                                                    1.0,
                                                    if selected {
                                                        crate::theme::ACCENT
                                                    } else {
                                                        crate::theme::BORDER
                                                    },
                                                ))
                                                .corner_radius(7),
                                            );
                                            if response.clicked() {
                                                state.selected = Some(index);
                                            }
                                            ui.add_space(4.0);
                                        }
                                    });
                            },
                        );

                        section_card(
                            &mut columns[1],
                            "Shortcut details",
                            "The key is what you type; the target is what QGo opens.",
                            theme,
                            |ui| {
                                let Some(index) = state.selected else {
                                    ui.centered_and_justified(|ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                "Select a shortcut or create a new one.",
                                            )
                                            .color(theme.foreground.gamma_multiply(0.6)),
                                        );
                                    });
                                    return;
                                };
                                let Some(shortcut) = state.shortcuts.get_mut(index) else {
                                    state.selected = None;
                                    return;
                                };

                                ui.label("Keyword");
                                ui.add(
                                    egui::TextEdit::singleline(&mut shortcut.key)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("e.g. google")
                                        .background_color(theme.shell),
                                );
                                ui.add_space(12.0);
                                ui.label("Target");
                                ui.add(
                                    egui::TextEdit::multiline(&mut shortcut.template)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(4)
                                        .hint_text("URL, file path, folder, cmd:, or ps:")
                                        .background_color(theme.shell),
                                );
                                ui.add_space(8.0);
                                egui::Frame::new()
                                    .fill(crate::theme::ACCENT.gamma_multiply(0.1))
                                    .corner_radius(7)
                                    .inner_margin(10)
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                "Tip: add {param} to insert text typed after the keyword.",
                                            )
                                            .small()
                                            .color(theme.foreground.gamma_multiply(0.75)),
                                        );
                                    });
                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    if danger_button(ui, "Delete shortcut").clicked() {
                                        remove = Some(index);
                                    }
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Used {} time{}",
                                            shortcut.usage_count,
                                            if shortcut.usage_count == 1 { "" } else { "s" }
                                        ))
                                        .small()
                                        .color(theme.foreground.gamma_multiply(0.55)),
                                    );
                                });
                            },
                        );
                    });

                    if let Some(index) = remove {
                        state.shortcuts.remove(index);
                        state.selected = if state.shortcuts.is_empty() {
                            None
                        } else {
                            Some(index.min(state.shortcuts.len() - 1))
                        };
                    }

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, "Save changes").clicked() {
                            match storage::save_links(&state.shortcuts) {
                                Ok(()) => {
                                    state.message =
                                        format!("Saved {} shortcuts.", state.shortcuts.len());
                                    saved = true;
                                }
                                Err(e) => state.message = format!("Could not save shortcuts: {e}"),
                            }
                        }
                        if secondary_button(ui, "Reload from disk", theme).clicked() {
                            *state = ManageState::load();
                        }
                    });
                });

            (saved, open_chains)
        },
    );

    (keep, saved, open_chains)
}

// ------------------------------------------------------- Chained shortcuts --

/// Port of `Views/ChainedShortcutsManagerWindow.xaml` and `ChainedShortcutEditor.xaml`.
pub struct ChainsState {
    chains: Vec<ChainedShortcut>,
    selected: usize,
    message: String,
}

impl ChainsState {
    fn load() -> Self {
        Self {
            chains: storage::load_chained_shortcuts(),
            selected: 0,
            message: String::new(),
        }
    }
}

fn show_chains(ctx: &Context, state: &mut ChainsState) -> (bool, bool) {
    let (keep, saved) = viewport(
        ctx,
        "qgo-chains",
        "QGo - Chained Shortcuts",
        [780.0, 640.0],
        |ui| {
            let mut saved = false;

            ui.horizontal(|ui| {
                if ui.button("New chain").clicked() {
                    state.chains.push(ChainedShortcut {
                        key: format!("chain{}", state.chains.len() + 1),
                        ..Default::default()
                    });
                    state.selected = state.chains.len() - 1;
                }
                if ui.button("Delete chain").clicked() && state.selected < state.chains.len() {
                    state.chains.remove(state.selected);
                    state.selected = state.selected.saturating_sub(1);
                }
            });
            ui.separator();

            if state.chains.is_empty() {
                ui.label("No chains yet.");
                return saved;
            }

            ui.horizontal(|ui| {
                ui.label("Chain:");
                let selected_label = state
                    .chains
                    .get(state.selected)
                    .map(|c| c.key.clone())
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt("chain-picker")
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        for (i, chain) in state.chains.iter().enumerate() {
                            ui.selectable_value(&mut state.selected, i, chain.key.clone());
                        }
                    });
            });

            let index = state.selected.min(state.chains.len() - 1);
            let chain = &mut state.chains[index];

            ui.horizontal(|ui| {
                ui.label("Key:");
                ui.text_edit_singleline(&mut chain.key);
            });
            ui.horizontal(|ui| {
                ui.label("Description:");
                ui.text_edit_singleline(&mut chain.description);
            });
            ui.horizontal(|ui| {
                ui.label("Mode:");
                ui.radio_value(
                    &mut chain.execution_mode,
                    ExecutionMode::Sequential,
                    "Sequential",
                );
                ui.radio_value(
                    &mut chain.execution_mode,
                    ExecutionMode::Simultaneous,
                    "Simultaneous",
                );
            });
            ui.add(
                egui::Slider::new(&mut chain.global_delay_ms, 0..=5000)
                    .text("Delay between items (ms)"),
            );
            ui.checkbox(&mut chain.continue_on_error, "Continue if an item fails");

            ui.separator();
            ui.strong("Items");

            let mut remove: Option<usize> = None;
            egui::Grid::new("chain-items")
                .num_columns(5)
                .striped(true)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.strong("Shortcut");
                    ui.strong("Parameters");
                    ui.strong("Delay (ms)");
                    ui.strong("Enabled");
                    ui.strong("");
                    ui.end_row();

                    for (i, item) in chain.chain_items.iter_mut().enumerate() {
                        ui.add(
                            egui::TextEdit::singleline(&mut item.shortcut_key).desired_width(160.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut item.parameters).desired_width(200.0),
                        );
                        ui.add(egui::DragValue::new(&mut item.delay_ms).range(0..=60_000));
                        ui.checkbox(&mut item.enabled, "");
                        if ui.button("Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });

            if let Some(i) = remove {
                chain.chain_items.remove(i);
            }
            if ui.button("Add item").clicked() {
                chain.chain_items.push(ShortcutChainItem::default());
            }

            ui.separator();
            if ui.button("Save").clicked() {
                match storage::save_chained_shortcuts(&state.chains) {
                    Ok(()) => {
                        state.message = format!("Saved {} chains.", state.chains.len());
                        saved = true;
                    }
                    Err(e) => state.message = format!("Could not save: {e}"),
                }
            }
            if !state.message.is_empty() {
                ui.label(&state.message);
            }

            saved
        },
    );

    (keep, saved)
}

// ------------------------------------------------------------ Usage stats --

/// Port of `Views/UsageStatsWindow.xaml`.
fn show_stats(ctx: &Context, session: &Arc<Mutex<LauncherSession>>) -> bool {
    let (keep, ()) = viewport(
        ctx,
        "qgo-stats",
        "QGo - Usage Statistics",
        [520.0, 560.0],
        |ui| {
            let stats = telemetry::instance();
            ui.heading("Your QGo usage");
            ui.monospace(stats.usage_summary());
            let snapshot = stats.stats().clone();
            drop(stats);

            ui.add_space(12.0);
            ui.strong("By shortcut type");
            for (kind, count) in &snapshot.shortcut_type_usage {
                ui.label(format!("{kind}: {count}"));
            }

            ui.add_space(12.0);
            ui.strong("Top shortcuts");
            for entry in &snapshot.top_shortcuts {
                ui.label(format!(
                    "{} - {} runs ({})",
                    entry.key, entry.execution_count, entry.shortcut_type
                ));
            }

            ui.add_space(12.0);
            if let Ok(session) = session.lock() {
                ui.small(format!("{} shortcuts loaded", session.all.len()));
            }
            ui.small(format!(
                "Stored at {}",
                qgo_core::paths::usage_stats_path().display()
            ));
        },
    );
    keep
}

// ----------------------------------------------------------------- About --

/// Port of `Views/AboutWindow.xaml`.
fn show_about(ctx: &Context, _theme: Theme) -> bool {
    let (keep, ()) = viewport(ctx, "qgo-about", "About QGo", [420.0, 320.0], |ui| {
        ui.heading("QGo");
        ui.label("Keyboard-driven shortcut launcher");
        ui.add_space(8.0);
        ui.label(format!("Version {}", qgo_core::version::display_version()));
        ui.add_space(8.0);
        if ui.link("qgocommand.com").clicked() {
            let _ = qgo_core::launch::shell_execute("https://qgocommand.com/", None, None);
        }
        ui.add_space(12.0);
        ui.small(format!(
            "Data folder: {}",
            qgo_core::paths::data_dir().display()
        ));
        ui.small("Rust port of the WPF application.");
    });
    keep
}

fn show_update(ctx: &Context, state: &UpdateState, theme: Theme) -> bool {
    let mut keep_open = true;
    let current = qgo_core::version::VERSION;
    let changelog_url = qgo_core::version::changelog_url(current);

    let (viewport_open, ()) = viewport(ctx, "qgo-update", "QGo Updated!", [550.0, 360.0], |ui| {
        egui::Frame::new()
            .fill(theme.shell)
            .corner_radius(10)
            .inner_margin(24)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(0x28, 0xA7, 0x45), "●");
                    ui.vertical(|ui| {
                        ui.heading("QGo Updated!");
                        ui.label(
                            egui::RichText::new(format!("Updated to {current}"))
                                .color(theme.foreground.gamma_multiply(0.7)),
                        );
                    });
                });

                ui.add_space(20.0);
                if state.previous_version.is_some() {
                    ui.label("QGo has been updated!");
                    ui.label("Check out what's new in this release.");
                } else {
                    ui.label(format!("Welcome to QGo {current}!"));
                    ui.label("Check out the latest features and improvements.");
                }

                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    if ui.button("View Changelog").clicked() {
                        match qgo_core::launch::shell_execute(&changelog_url, None, None) {
                            Ok(()) => {
                                telemetry::instance().record_feature_usage("changelog_viewed");
                                keep_open = false;
                            }
                            Err(error) => {
                                log::warn!("could not open the QGo changelog: {error}");
                            }
                        }
                    }
                    if ui.button("Continue").clicked() {
                        telemetry::instance().record_feature_usage("update_notification_dismissed");
                        keep_open = false;
                    }
                });
            });
    });

    viewport_open && keep_open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manage_hides_chain_proxies_because_they_are_not_editable_there() {
        let mut proxy = Shortcut::new("work", "[Chain: ]");
        proxy.is_chained = true;
        let mixed = vec![Shortcut::new("qgo", "https://qgocommand.com/"), proxy];
        let editable: Vec<Shortcut> = mixed.into_iter().filter(|s| !s.is_chained).collect();
        assert_eq!(editable.len(), 1);
        assert_eq!(editable[0].key, "qgo");
    }

    #[test]
    fn opening_a_window_twice_does_not_reset_its_state() {
        let mut windows = SecondaryWindows::default();
        windows.open(WindowKind::ManageShortcuts);
        windows.manage.as_mut().unwrap().message = "edited".into();
        windows.open(WindowKind::ManageShortcuts);
        assert_eq!(windows.manage.as_ref().unwrap().message, "edited");
    }

    #[test]
    fn the_stateless_windows_toggle_cleanly() {
        let mut windows = SecondaryWindows::default();
        assert!(!windows.about);
        windows.open(WindowKind::About);
        assert!(windows.about);
        windows.open(WindowKind::UsageStats);
        assert!(windows.stats);
        windows.open(WindowKind::Update);
        assert!(windows.update.is_some());
    }
}
