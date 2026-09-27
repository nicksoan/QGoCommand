//! The non-visual half of `MainViewModel`: the loaded shortcut list, the chain
//! definitions behind it, and what happens when the user presses Enter.
//!
//! Splitting this out is the main structural change from the WPF app, where the
//! same file owns the collection view, the brushes, the dialogs and the launch
//! logic. Here the GUI crate owns rendering and this owns behaviour, so the
//! launch path is unit-testable — it is not in the C#.

use crate::chain;
use crate::command::determine_shortcut_type;
use crate::expand::expand;
use crate::launch::{execute_plan, plan_regular, user_profile_dir};
use crate::matching;
use crate::models::{AppSettings, ChainedShortcut, Shortcut};
use crate::storage;
use crate::telemetry;
use chrono::Utc;

/// What a launch attempt did, so the UI can decide whether to hide the window
/// or surface an error.
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchOutcome {
    /// Nothing matched what the user typed; the window stays open.
    NoMatch,
    /// Launched normally.
    Launched { key: String },
    /// A chain ran. `failed` is non-zero when some items could not start.
    ChainLaunched { key: String, failed: usize },
    /// The launch itself failed.
    Failed { key: String, error: String },
}

/// The result of [`LauncherSession::prepare_launch`].
pub enum Prepared {
    /// Blocking work remains; run it with the session lock released.
    Ready(PendingLaunch),
    /// Nothing to run because no shortcut matched.
    Done(LaunchOutcome),
}

/// A launch that has been resolved and accounted for, and is ready to block.
///
/// Carries everything by value precisely so it can outlive the session lock.
pub struct PendingLaunch {
    key: String,
    /// The expanded target, for a regular shortcut.
    target: String,
    param: String,
    /// `Some` for a chained shortcut.
    chain: Option<ChainedShortcut>,
    /// Snapshot of the shortcut list the chain's items resolve against.
    available: Vec<Shortcut>,
}

impl PendingLaunch {
    /// The blocking half of `LaunchSelected()`. Sleeps for chain delays and
    /// waits on `ShellExecuteW`, so it must not run on the render thread.
    pub fn run(mut self) -> LaunchOutcome {
        match self.chain.take() {
            Some(chain) => self.run_chain(chain),
            None => {
                let plan = plan_regular(&self.target, &self.param);
                match execute_plan(&plan, user_profile_dir().as_deref()) {
                    Ok(()) => LaunchOutcome::Launched { key: self.key },
                    Err(e) => LaunchOutcome::Failed {
                        key: self.key,
                        error: e.to_string(),
                    },
                }
            }
        }
    }

    fn run_chain(self, chain: ChainedShortcut) -> LaunchOutcome {
        match chain::execute(&chain, &self.available) {
            Ok(report) => {
                for outcome in &report.outcomes {
                    telemetry::instance().record_chain_item_execution(outcome.success);
                }
                LaunchOutcome::ChainLaunched {
                    key: chain.key,
                    failed: report.failed(),
                }
            }
            Err(e) => LaunchOutcome::Failed {
                key: chain.key,
                error: e.to_string(),
            },
        }
    }
}

/// Holds everything the launcher needs at runtime.
pub struct LauncherSession {
    /// Every entry shown in the list, including proxies for chained shortcuts.
    pub all: Vec<Shortcut>,
    pub settings: AppSettings,
    chains: Vec<ChainedShortcut>,
}

impl LauncherSession {
    /// Equivalent to the `MainViewModel` constructor: load settings, shortcuts
    /// and chains, then fold the chains in as proxy entries.
    pub fn load() -> Self {
        let settings = storage::load_settings();
        let chains = storage::load_chained_shortcuts();
        let all = storage::load_links_or_defaults();

        let mut session = Self {
            all,
            settings,
            chains,
        };
        session.sync_chain_proxies();
        session
    }

    pub fn chains(&self) -> &[ChainedShortcut] {
        &self.chains
    }

    /// `RefreshChainedShortcutsInMainList()` — drop proxies whose chain is gone,
    /// then add or refresh a proxy for every chain that exists.
    pub fn sync_chain_proxies(&mut self) {
        let keys: Vec<String> = self.chains.iter().map(|c| c.key.to_lowercase()).collect();

        self.all.retain(|s| {
            if !s.is_chained {
                return true;
            }
            match &s.chained_shortcut_key {
                Some(key) => keys.contains(&key.to_lowercase()),
                None => false,
            }
        });

        for chain in &self.chains {
            let template = chain.proxy_template();
            let existing = self.all.iter_mut().find(|s| {
                s.is_chained
                    && s.chained_shortcut_key
                        .as_deref()
                        .map(|k| k.eq_ignore_ascii_case(&chain.key))
                        .unwrap_or(false)
            });

            match existing {
                Some(proxy) => {
                    proxy.template = template;
                    proxy.usage_count = chain.usage_count;
                    proxy.last_used = chain.last_used;
                }
                None => self.all.push(Shortcut {
                    key: chain.key.clone(),
                    template,
                    is_chained: true,
                    chained_shortcut_key: Some(chain.key.clone()),
                    usage_count: chain.usage_count,
                    last_used: chain.last_used,
                    ..Default::default()
                }),
            }
        }
    }

    /// `ReloadFrom(items)` — re-reads chains from disk and re-sorts by usage.
    pub fn reload_from(&mut self, items: Vec<Shortcut>) {
        self.all = items;
        let mut indices: Vec<usize> = (0..self.all.len()).collect();
        matching::sort_indices(&self.all, &mut indices);
        self.all = indices.into_iter().map(|i| self.all[i].clone()).collect();

        self.chains = storage::load_chained_shortcuts();
        self.sync_chain_proxies();
    }

    pub fn reload_chains(&mut self) {
        self.chains = storage::load_chained_shortcuts();
        self.sync_chain_proxies();
    }

    /// The filtered, ranked list the UI renders.
    pub fn filter(&self, search_text: &str, showing_most_used: bool) -> Vec<usize> {
        matching::filter_indices(&self.all, search_text, showing_most_used)
    }

    /// First half of `LaunchSelected()`: resolve what to run and update the
    /// counters, all of which is fast.
    ///
    /// Deliberately split from [`PendingLaunch::run`], which is the blocking
    /// half. A sequential chain sleeps for the sum of its delays — up to a
    /// minute per item — and holding the session lock across that would freeze
    /// every frame, because the render loop locks it too. So the caller does:
    ///
    /// ```ignore
    /// let prepared = session.lock().prepare_launch(&input, selected);
    /// drop(session_guard);              // <- before anything blocks
    /// let outcome = prepared.run();
    /// ```
    ///
    /// `selected` is whatever the user highlighted in the list; when it is
    /// `None` the typed key is resolved the same way the C# does.
    pub fn prepare_launch(&mut self, input: &str, selected: Option<usize>) -> Prepared {
        let (key, param) = matching::split_input(input);
        let param = param.to_string();

        let Some(index) = selected.or_else(|| matching::resolve_by_key(&self.all, key)) else {
            return Prepared::Done(LaunchOutcome::NoMatch);
        };
        if index >= self.all.len() {
            return Prepared::Done(LaunchOutcome::NoMatch);
        }

        self.all[index].usage_count += 1;
        self.all[index].last_used = Utc::now();
        let _ = storage::save_links(&self.all);
        telemetry::instance().update_shortcut_usage(&self.all);

        let shortcut = self.all[index].clone();
        let shortcut_type = if shortcut.is_chained {
            "chained".to_string()
        } else {
            determine_shortcut_type(&shortcut.template).to_string()
        };
        telemetry::instance().record_shortcut_execution(&shortcut_type);

        if !shortcut.is_chained {
            let target = expand(&shortcut.template, &param);
            return Prepared::Ready(PendingLaunch {
                key: shortcut.key,
                target,
                param,
                chain: None,
                available: Vec::new(),
            });
        }

        let Some(chain_index) = self.chains.iter().position(|c| {
            shortcut
                .chained_shortcut_key
                .as_deref()
                .map(|k| k.eq_ignore_ascii_case(&c.key))
                .unwrap_or(false)
        }) else {
            return Prepared::Done(LaunchOutcome::NoMatch);
        };

        self.chains[chain_index].usage_count += 1;
        self.chains[chain_index].last_used = Utc::now();
        let _ = storage::save_chained_shortcuts(&self.chains);

        let chain = self.chains[chain_index].clone();
        let enabled = chain.enabled_items().count() as i32;
        telemetry::instance()
            .record_chained_shortcut_execution(chain.execution_mode.name(), enabled);

        Prepared::Ready(PendingLaunch {
            key: chain.key.clone(),
            target: String::new(),
            param,
            chain: Some(chain),
            // The chain executor resolves each item's key against this snapshot.
            available: self.all.clone(),
        })
    }

    /// `Storage.SaveLinks(All)`, minus the chain proxies — those are rebuilt
    /// from `chained-shortcuts.json` on load, and the WPF app writes them into
    /// `links.json` too, so they are kept for round-trip compatibility.
    pub fn save(&self) -> std::io::Result<()> {
        storage::save_links(&self.all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionMode, ShortcutChainItem};

    fn session_with(all: Vec<Shortcut>, chains: Vec<ChainedShortcut>) -> LauncherSession {
        let mut s = LauncherSession {
            all,
            settings: AppSettings::default(),
            chains,
        };
        s.sync_chain_proxies();
        s
    }

    fn chain(key: &str, description: &str) -> ChainedShortcut {
        ChainedShortcut {
            key: key.into(),
            description: description.into(),
            execution_mode: ExecutionMode::Sequential,
            chain_items: vec![ShortcutChainItem {
                shortcut_key: "github".into(),
                ..Default::default()
            }],
            usage_count: 7,
            ..Default::default()
        }
    }

    #[test]
    fn a_chain_gains_a_proxy_entry_in_the_shortcut_list() {
        let s = session_with(
            vec![Shortcut::new("github", "https://github.com/")],
            vec![chain("work", "Apps useful for work")],
        );
        let proxy = s.all.iter().find(|x| x.key == "work").unwrap();
        assert!(proxy.is_chained);
        assert_eq!(proxy.template, "[Chain: Apps useful for work]");
        assert_eq!(proxy.usage_count, 7);
        assert_eq!(proxy.chained_shortcut_key.as_deref(), Some("work"));
    }

    #[test]
    fn an_existing_proxy_is_refreshed_rather_than_duplicated() {
        let stale = Shortcut {
            key: "work".into(),
            template: "[Chain: old description]".into(),
            is_chained: true,
            chained_shortcut_key: Some("work".into()),
            usage_count: 1,
            ..Default::default()
        };
        let s = session_with(vec![stale], vec![chain("work", "new description")]);
        assert_eq!(s.all.iter().filter(|x| x.key == "work").count(), 1);
        assert_eq!(s.all[0].template, "[Chain: new description]");
        assert_eq!(s.all[0].usage_count, 7);
    }

    #[test]
    fn a_proxy_whose_chain_was_deleted_is_removed() {
        let orphan = Shortcut {
            key: "gone".into(),
            template: "[Chain: ]".into(),
            is_chained: true,
            chained_shortcut_key: Some("gone".into()),
            ..Default::default()
        };
        let s = session_with(
            vec![orphan, Shortcut::new("qgo", "https://qgocommand.com/")],
            vec![],
        );
        assert_eq!(s.all.len(), 1);
        assert_eq!(s.all[0].key, "qgo");
    }

    #[test]
    fn filtering_goes_through_the_shared_matcher() {
        let s = session_with(
            vec![
                Shortcut::new("github", "https://github.com/"),
                Shortcut::new("google", "https://google.com/"),
            ],
            vec![],
        );
        let hits = s.filter("git", false);
        assert_eq!(hits.len(), 1);
        assert_eq!(s.all[hits[0]].key, "github");
    }

    #[test]
    fn an_unmatched_input_launches_nothing() {
        let mut s = session_with(vec![Shortcut::new("github", "https://github.com/")], vec![]);
        match s.prepare_launch("definitely-not-a-key", None) {
            Prepared::Done(LaunchOutcome::NoMatch) => {}
            _ => panic!("expected NoMatch without any blocking work"),
        }
    }

    #[test]
    fn preparing_a_launch_captures_everything_by_value() {
        // The whole point of the split: nothing in a PendingLaunch borrows the
        // session, so the caller can drop the lock before the blocking half.
        let mut s = session_with(vec![Shortcut::new("github", "https://github.com/")], vec![]);
        match s.prepare_launch("github", None) {
            Prepared::Ready(pending) => {
                assert_eq!(pending.key, "github");
                assert_eq!(pending.target, "https://github.com/");
                assert!(pending.chain.is_none());
            }
            Prepared::Done(other) => panic!("expected a runnable launch, got {other:?}"),
        }
    }

    #[test]
    fn a_parameter_is_expanded_before_the_lock_is_released() {
        let mut s = session_with(
            vec![Shortcut::new("google", "https://google.com/search?q={q}")],
            vec![],
        );
        match s.prepare_launch("google rust lang", None) {
            Prepared::Ready(pending) => {
                assert_eq!(pending.target, "https://google.com/search?q=rust%20lang");
            }
            Prepared::Done(other) => panic!("expected a runnable launch, got {other:?}"),
        }
    }
}
