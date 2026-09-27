//! # qgo-core
//!
//! The non-visual half of QGo, ported from `QGo.App` (WPF, C#).
//!
//! The WPF app keeps its data models, JSON persistence, template expansion,
//! launch routing and usage statistics inside the same project
//! as the windows. This crate holds all of that with no GUI dependency, so the
//! behaviour can be exercised with `cargo test` — the C# side has no automated
//! tests at all.
//!
//! ## Compatibility
//!
//! Everything reads and writes the same files as the WPF build, under
//! `%LOCALAPPDATA%\QGo`. The two can be run against one profile in either order.
//! The awkward parts of that contract live in [`serde_cs`]: PascalCase members,
//! `"NaN"` as a JSON string, integer enums, and C#'s `DateTime` formatting.
//!
//! ## Layout
//!
//! | Module | Ported from |
//! |---|---|
//! | [`models`] | `Models/*.cs` |
//! | [`storage`] | `Storage.cs` |
//! | [`expand`] | `MainViewModel.Expand` |
//! | [`command`] | `Services/CommandExecutionService.cs` |
//! | [`launch`] | the `Process.Start` calls in `MainViewModel` / `ChainedShortcutExecutor` |
//! | [`chain`] | `Services/ChainedShortcutExecutor.cs` |
//! | [`matching`] | the collection-view filter and `AutoComplete` in `MainViewModel` |
//! | [`session`] | the rest of `MainViewModel` |
//! | [`telemetry`] | `Services/LocalUsageStatsService.cs` |
//! | [`startup`] | `StartupManager.cs` |

pub mod chain;
pub mod command;
pub mod expand;
pub mod launch;
pub mod matching;
pub mod models;
pub mod paths;
pub mod serde_cs;
pub mod session;
pub mod startup;
pub mod storage;
pub mod telemetry;
pub mod version;

pub use models::{AppSettings, ChainedShortcut, ExecutionMode, Shortcut, ShortcutChainItem};
pub use session::{LaunchOutcome, LauncherSession, PendingLaunch, Prepared};
