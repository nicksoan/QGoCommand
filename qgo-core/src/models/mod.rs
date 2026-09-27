//! Ports of the types under `QGo.App/Models`.
//!
//! Every struct serialises to the same JSON the WPF app writes: PascalCase
//! member names (no `PropertyNamingPolicy` is configured in `Storage.Opt`) and
//! `#[serde(default)]` on each field so a file written by an older build still
//! loads, which is what the C# property initialisers achieve.

mod chained_shortcut;
mod settings;
mod shortcut;
mod usage;

pub use chained_shortcut::{ChainedShortcut, ExecutionMode, ShortcutChainItem};
pub use settings::AppSettings;
pub use shortcut::Shortcut;
pub use usage::{ChainedShortcutStats, DailyUsage, ShortcutUsageInfo, UsageStatistics};
