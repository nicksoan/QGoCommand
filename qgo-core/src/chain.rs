//! Port of `QGo.App.Services.ChainedShortcutExecutor`.
//!
//! Executing a chain is blocking — it sleeps between items — so callers run it
//! on a worker thread, the way the C# hands it to the task pool. `Simultaneous`
//! mode fans out to one thread per item and joins them all, matching
//! `Task.WhenAll`, which waits for every task even when one has faulted.

use crate::expand::expand;
use crate::launch::{execute_plan, plan_chain_item, user_profile_dir, LaunchError};
use crate::models::{ChainedShortcut, ExecutionMode, Shortcut, ShortcutChainItem};
use std::thread;
use std::time::Duration;

/// Reported per item so the caller can update usage statistics without this
/// module depending on the telemetry layer.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainItemOutcome {
    pub shortcut_key: String,
    /// The `DetermineShortcutType` bucket, or `"unknown"` when the referenced
    /// shortcut no longer exists.
    pub shortcut_type: String,
    pub had_parameters: bool,
    pub delay_ms: i32,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("shortcut '{0}' not found in available shortcuts")]
    ShortcutNotFound(String),
    #[error("chained shortcuts cannot reference other chained shortcuts ('{0}')")]
    NestedChain(String),
    #[error(transparent)]
    Launch(#[from] LaunchError),
}

/// The result of running a whole chain.
#[derive(Debug, Clone, Default)]
pub struct ChainRunReport {
    pub outcomes: Vec<ChainItemOutcome>,
}

impl ChainRunReport {
    pub fn succeeded(&self) -> usize {
        self.outcomes.iter().filter(|o| o.success).count()
    }

    pub fn failed(&self) -> usize {
        self.outcomes.len() - self.succeeded()
    }
}

/// `ChainedShortcutExecutor.ExecuteAsync(chainedShortcut)`.
///
/// Blocks for the full duration of the chain, including every delay.
pub fn execute(
    chain: &ChainedShortcut,
    available: &[Shortcut],
) -> Result<ChainRunReport, ChainError> {
    let enabled: Vec<&ShortcutChainItem> = chain.enabled_items().collect();
    if enabled.is_empty() {
        log::debug!("chain '{}' has no enabled items", chain.key);
        return Ok(ChainRunReport::default());
    }

    log::debug!(
        "executing chain '{}' with {} items in {} mode",
        chain.key,
        enabled.len(),
        chain.execution_mode
    );

    match chain.execution_mode {
        ExecutionMode::Simultaneous => Ok(execute_simultaneous(
            &enabled,
            available,
            chain.continue_on_error,
        )),
        ExecutionMode::Sequential => execute_sequential(
            &enabled,
            available,
            chain.global_delay_ms,
            chain.continue_on_error,
        ),
    }
}

/// `ExecuteSimultaneousAsync` — every item starts at once and all are awaited,
/// so `continue_on_error` only affects what gets reported, never the waiting.
fn execute_simultaneous(
    items: &[&ShortcutChainItem],
    available: &[Shortcut],
    _continue_on_error: bool,
) -> ChainRunReport {
    let handles: Vec<_> = items
        .iter()
        .map(|item| {
            let item = (*item).clone();
            let available = available.to_vec();
            thread::spawn(move || run_item(&item, &available))
        })
        .collect();

    let mut report = ChainRunReport::default();
    for handle in handles {
        match handle.join() {
            Ok(outcome) => report.outcomes.push(outcome),
            Err(_) => report.outcomes.push(ChainItemOutcome {
                shortcut_key: String::new(),
                shortcut_type: "unknown".into(),
                had_parameters: false,
                delay_ms: 0,
                success: false,
                error: Some("chain item thread panicked".into()),
            }),
        }
    }
    report
}

/// `ExecuteSequentialAsync` — each item's own delay first, then the item, then
/// the chain's global delay (skipped after the last item).
fn execute_sequential(
    items: &[&ShortcutChainItem],
    available: &[Shortcut],
    global_delay_ms: i32,
    continue_on_error: bool,
) -> Result<ChainRunReport, ChainError> {
    let mut report = ChainRunReport::default();

    for (i, item) in items.iter().enumerate() {
        if item.delay_ms > 0 {
            thread::sleep(Duration::from_millis(item.delay_ms as u64));
        }

        let outcome = run_item(item, available);
        let failed = !outcome.success;
        let error = outcome.error.clone();
        report.outcomes.push(outcome);

        if failed && !continue_on_error {
            log::debug!("stopping chain: ContinueOnError is false");
            return Err(ChainError::Launch(LaunchError::Failed(
                error.unwrap_or_else(|| "chain item failed".into()),
            )));
        }

        if i < items.len() - 1 && global_delay_ms > 0 {
            thread::sleep(Duration::from_millis(global_delay_ms as u64));
        }
    }

    Ok(report)
}

/// `ExecuteChainItemAsync` — resolve, expand, launch.
fn run_item(item: &ShortcutChainItem, available: &[Shortcut]) -> ChainItemOutcome {
    let mut outcome = ChainItemOutcome {
        shortcut_key: item.shortcut_key.clone(),
        shortcut_type: "unknown".into(),
        had_parameters: !item.parameters.is_empty(),
        delay_ms: item.delay_ms,
        success: false,
        error: None,
    };

    let Some(shortcut) = available
        .iter()
        .find(|s| s.key.to_lowercase() == item.shortcut_key.to_lowercase())
    else {
        outcome.error = Some(ChainError::ShortcutNotFound(item.shortcut_key.clone()).to_string());
        return outcome;
    };

    outcome.shortcut_type = crate::command::determine_shortcut_type(&shortcut.template).to_string();

    // Guards against a chain calling itself into infinite recursion.
    if shortcut.is_chained {
        outcome.error = Some(ChainError::NestedChain(item.shortcut_key.clone()).to_string());
        return outcome;
    }

    let target = expand(&shortcut.template, &item.parameters);
    let plan = plan_chain_item(&target, &item.parameters);

    match execute_plan(&plan, user_profile_dir().as_deref()) {
        Ok(()) => outcome.success = true,
        Err(e) => outcome.error = Some(e.to_string()),
    }

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain_with(mode: ExecutionMode, items: Vec<ShortcutChainItem>) -> ChainedShortcut {
        ChainedShortcut {
            key: "test".into(),
            execution_mode: mode,
            chain_items: items,
            ..Default::default()
        }
    }

    fn item(key: &str, enabled: bool) -> ShortcutChainItem {
        ShortcutChainItem {
            shortcut_key: key.into(),
            enabled,
            ..Default::default()
        }
    }

    #[test]
    fn disabled_items_are_skipped_when_collecting_the_run_list() {
        let chain = chain_with(
            ExecutionMode::Sequential,
            vec![item("a", true), item("b", false), item("c", true)],
        );
        let keys: Vec<&str> = chain
            .enabled_items()
            .map(|i| i.shortcut_key.as_str())
            .collect();
        assert_eq!(keys, ["a", "c"]);
    }

    #[test]
    fn a_missing_shortcut_fails_the_item_without_panicking() {
        let outcome = run_item(&item("nope", true), &[]);
        assert!(!outcome.success);
        assert!(outcome.error.unwrap().contains("not found"));
        assert_eq!(outcome.shortcut_type, "unknown");
    }

    #[test]
    fn a_nested_chain_is_refused() {
        let mut proxy = Shortcut::new("inner", "[Chain: ]");
        proxy.is_chained = true;
        let outcome = run_item(&item("inner", true), &[proxy]);
        assert!(!outcome.success);
        assert!(outcome.error.unwrap().contains("cannot reference"));
    }

    #[test]
    fn the_report_counts_successes_and_failures() {
        let report = ChainRunReport {
            outcomes: vec![
                ChainItemOutcome {
                    shortcut_key: "a".into(),
                    shortcut_type: "web_url".into(),
                    had_parameters: false,
                    delay_ms: 0,
                    success: true,
                    error: None,
                },
                ChainItemOutcome {
                    shortcut_key: "b".into(),
                    shortcut_type: "unknown".into(),
                    had_parameters: false,
                    delay_ms: 0,
                    success: false,
                    error: Some("boom".into()),
                },
            ],
        };
        assert_eq!(report.succeeded(), 1);
        assert_eq!(report.failed(), 1);
    }
}
