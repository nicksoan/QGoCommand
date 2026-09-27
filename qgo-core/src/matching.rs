//! The launcher's search behaviour: filtering, ranking, selection and
//! Tab-autocomplete. Ported from the `CollectionView` filter, the
//! `SortDescriptions` in `ApplyUsageBasedSorting`, `LaunchSelected`'s fallback
//! resolution, `ShowMostUsedShortcuts` and `AutoComplete` in `MainViewModel`.
//!
//! Everything here works on indices into the caller's `&[Shortcut]` and is pure,
//! which is what makes the ranking rules testable — they are not in the C#.

use crate::models::Shortcut;
use std::cmp::Ordering;

/// How many entries the most-used / alphabetical preview shows.
pub const PREVIEW_LIMIT: usize = 10;

/// Splits the input box into the shortcut key and its parameter, on the first
/// space — `LaunchSelected`'s first few lines.
pub fn split_input(input: &str) -> (&str, &str) {
    let input = input.trim();
    match input.find(' ') {
        Some(i) => (&input[..i], input[i + 1..].trim()),
        None => (input, ""),
    }
}

/// `StringComparison.OrdinalIgnoreCase` equality.
fn eq_ci(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn starts_with_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().starts_with(&needle.to_lowercase())
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// `SortDescription(nameof(Shortcut.Key), Ascending)` — case-insensitive first,
/// with a case-sensitive tiebreak so the order is total and stable.
fn cmp_key(a: &Shortcut, b: &Shortcut) -> Ordering {
    a.key
        .to_lowercase()
        .cmp(&b.key.to_lowercase())
        .then_with(|| a.key.cmp(&b.key))
}

/// Usage-aware ranking: `UsageCount` desc, `LastUsed` desc, `Key` asc.
fn cmp_usage_then_key(a: &Shortcut, b: &Shortcut) -> Ordering {
    b.usage_count
        .cmp(&a.usage_count)
        .then_with(|| b.last_used.cmp(&a.last_used))
        .then_with(|| cmp_key(a, b))
}

/// Applies usage-aware ordering to a set of indices in place.
pub fn sort_indices(all: &[Shortcut], indices: &mut [usize]) {
    indices.sort_by(|&a, &b| cmp_usage_then_key(&all[a], &all[b]));
}

/// The `CollectionView` filter predicate, applied and sorted.
///
/// * with `showing_most_used` and an empty query, the preview list is returned;
/// * an empty query otherwise matches nothing — the launcher shows no list until
///   the user types;
/// * otherwise a key matches if it starts with the whole query, or contains the
///   first token (so `"g maps"` still finds `google`).
pub fn filter_indices(all: &[Shortcut], search_text: &str, showing_most_used: bool) -> Vec<usize> {
    let query = search_text.trim();

    if showing_most_used && query.is_empty() {
        return preview_indices(all);
    }

    let mut matched: Vec<usize> = Vec::new();
    if query.is_empty() {
        return matched;
    }

    let token = match query.find(' ') {
        Some(i) => &query[..i],
        None => query,
    };

    for (i, s) in all.iter().enumerate() {
        if starts_with_ci(&s.key, query) || contains_ci(&s.key, token) {
            matched.push(i);
        }
    }

    sort_indices(all, &mut matched);
    matched
}

/// `ShowMostUsedShortcuts` — the list shown when the user presses an arrow key
/// on an empty input.
///
/// Returns the ten most-used entries, falling back to alphabetical ordering
/// when no usage has been recorded yet.
pub fn preview_indices(all: &[Shortcut]) -> Vec<usize> {
    let mut used: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, shortcut)| shortcut.usage_count > 0)
        .map(|(index, _)| index)
        .collect();

    if !used.is_empty() {
        used.sort_by(|&a, &b| {
            all[b]
                .usage_count
                .cmp(&all[a].usage_count)
                .then_with(|| cmp_key(&all[a], &all[b]))
        });
        used.truncate(PREVIEW_LIMIT);
        return used;
    }

    let mut visible: Vec<usize> = (0..all.len()).collect();
    visible.sort_by(|&a, &b| cmp_key(&all[a], &all[b]));
    visible.truncate(PREVIEW_LIMIT);
    visible
}

/// `LaunchSelected`'s fallback resolution when the user hits Enter without
/// picking anything from the list: exact key first, then a prefix match, ranked
/// by usage.
pub fn resolve_by_key(all: &[Shortcut], key: &str) -> Option<usize> {
    if key.is_empty() {
        return None;
    }

    let mut exact: Vec<usize> = (0..all.len())
        .filter(|&i| eq_ci(&all[i].key, key))
        .collect();
    let mut prefix: Vec<usize> = (0..all.len())
        .filter(|&i| starts_with_ci(&all[i].key, key))
        .collect();

    let by_usage = |a: &usize, b: &usize| {
        all[*b]
            .usage_count
            .cmp(&all[*a].usage_count)
            .then_with(|| all[*b].last_used.cmp(&all[*a].last_used))
    };
    exact.sort_by(by_usage);
    prefix.sort_by(by_usage);

    exact.first().copied().or_else(|| prefix.first().copied())
}

/// `UpdatePlaceholder` — the greyed-out parameter hint, shown only when the
/// typed text is an exact key of a parameterised shortcut.
pub fn placeholder_for(all: &[Shortcut], search_text: &str) -> String {
    if search_text.is_empty() {
        return String::new();
    }
    let trimmed = search_text.trim();
    all.iter()
        .find(|s| eq_ci(&s.key, trimmed))
        .filter(|s| s.is_parameterized())
        .map(|s| s.placeholder_text())
        .unwrap_or_default()
}

/// Tab-completion state, matching the `_lastAutocompleteInput` /
/// `_autocompleteIndex` / `_autocompleteMatches` trio in `MainViewModel`.
#[derive(Debug, Default)]
pub struct Autocomplete {
    last_input: String,
    index: usize,
    matches: Vec<usize>,
}

/// What the caller should do with the input box after a Tab press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocompleteResult {
    /// Index into `all` of the chosen shortcut.
    pub chosen: usize,
    /// The new contents of the input box — a trailing space is appended for
    /// parameterised shortcuts so the user can type the parameter straight away.
    pub search_text: String,
    /// The new parameter hint.
    pub placeholder: String,
}

impl Autocomplete {
    pub fn reset(&mut self) {
        self.last_input.clear();
        self.index = 0;
        self.matches.clear();
    }

    /// `AutoComplete()`. Returns `None` when there is nothing to complete —
    /// an empty input, an input that already contains a space (the user is
    /// typing a parameter), or no matches at all.
    ///
    /// `visible` is the currently filtered list, whose entries are appended
    /// after the exact and prefix matches so a substring hit like `g` -> `qgo`
    /// is still reachable by cycling.
    pub fn advance(
        &mut self,
        all: &[Shortcut],
        search_text: &str,
        selected: Option<usize>,
        visible: &[usize],
    ) -> Option<AutocompleteResult> {
        let input = search_text.trim().to_string();
        if input.is_empty() || input.contains(' ') {
            return None;
        }

        let continuing = eq_ci(&input, &self.last_input);

        if !continuing {
            self.last_input = input.clone();
            self.matches = build_matches(all, &input, visible);
            if self.matches.is_empty() {
                return None;
            }
            self.index = selected
                .and_then(|s| self.matches.iter().position(|&m| m == s))
                .unwrap_or(0);
        } else {
            if self.matches.is_empty() {
                return None;
            }
            // If the user moved the selection with the arrow keys, resume cycling
            // from wherever they landed.
            if let Some(pos) = selected.and_then(|s| self.matches.iter().position(|&m| m == s)) {
                if pos != self.index {
                    self.index = pos;
                }
            }
            self.index = (self.index + 1) % self.matches.len();
        }

        let chosen = self.matches[self.index];
        let shortcut = &all[chosen];

        if shortcut.is_parameterized() {
            Some(AutocompleteResult {
                chosen,
                search_text: format!("{} ", shortcut.key),
                placeholder: shortcut.placeholder_text(),
            })
        } else {
            Some(AutocompleteResult {
                chosen,
                search_text: shortcut.key.clone(),
                placeholder: String::new(),
            })
        }
    }
}

/// Exact matches, then prefix matches, then whatever else is currently visible —
/// each group ranked by usage.
fn build_matches(all: &[Shortcut], input: &str, visible: &[usize]) -> Vec<usize> {
    let rank = |group: &mut [usize]| {
        group.sort_by(|&a, &b| {
            all[b]
                .usage_count
                .cmp(&all[a].usage_count)
                .then_with(|| all[b].last_used.cmp(&all[a].last_used))
        });
    };

    let mut exact: Vec<usize> = (0..all.len())
        .filter(|&i| eq_ci(&all[i].key, input))
        .collect();
    let mut prefix: Vec<usize> = (0..all.len())
        .filter(|&i| starts_with_ci(&all[i].key, input) && !eq_ci(&all[i].key, input))
        .collect();

    rank(&mut exact);
    rank(&mut prefix);

    let mut matches = exact;
    matches.extend(prefix);

    let mut extra: Vec<usize> = visible
        .iter()
        .copied()
        .filter(|i| !matches.contains(i))
        .collect();
    rank(&mut extra);
    matches.extend(extra);

    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn shortcut(key: &str, template: &str, usage: i32, day: u32) -> Shortcut {
        Shortcut {
            key: key.into(),
            template: template.into(),
            usage_count: usage,
            last_used: Utc.with_ymd_and_hms(2026, 1, day.max(1), 0, 0, 0).unwrap(),
            ..Default::default()
        }
    }

    fn sample() -> Vec<Shortcut> {
        vec![
            shortcut("google", "https://google.com/search?q={Query}", 5, 3),
            shortcut("github", "https://github.com/", 20, 2),
            shortcut("qgo", "https://qgocommand.com/", 1, 1),
            {
                let mut c = shortcut("work", "[Chain: ]", 50, 4);
                c.is_chained = true;
                c.chained_shortcut_key = Some("work".into());
                c
            },
        ]
    }

    #[test]
    fn input_splits_on_the_first_space_only() {
        assert_eq!(split_input("google rust lang"), ("google", "rust lang"));
        assert_eq!(split_input("  github  "), ("github", ""));
        assert_eq!(split_input(""), ("", ""));
    }

    #[test]
    fn chained_shortcuts_are_searchable() {
        let all = sample();
        assert_eq!(filter_indices(&all, "wor", false), vec![3]);
    }

    #[test]
    fn an_empty_query_shows_nothing_until_the_preview_is_requested() {
        let all = sample();
        assert!(filter_indices(&all, "", false).is_empty());
        assert!(!filter_indices(&all, "", true).is_empty());
    }

    #[test]
    fn matches_are_ranked_by_usage() {
        let all = sample();
        let matches = filter_indices(&all, "g", false);
        assert_eq!(all[matches[0]].key, "github");
        assert_eq!(all[matches[1]].key, "google");
    }

    #[test]
    fn a_substring_match_on_the_first_token_still_counts() {
        let all = sample();
        // "go" is not a prefix of "qgo", but it is contained in it.
        let hits = filter_indices(&all, "go", false);
        let keys: Vec<&str> = hits.iter().map(|&i| all[i].key.as_str()).collect();
        assert!(keys.contains(&"qgo"));
        assert!(keys.contains(&"google"));
    }

    #[test]
    fn the_preview_prefers_used_shortcuts() {
        let all = sample();
        let preview = preview_indices(&all);
        assert_eq!(all[preview[0]].key, "work");
    }

    #[test]
    fn the_preview_falls_back_to_alphabetical_when_nothing_has_been_used() {
        let all = vec![shortcut("zebra", "z", 0, 1), shortcut("alpha", "a", 0, 1)];
        let preview = preview_indices(&all);
        assert_eq!(all[preview[0]].key, "alpha");
    }

    #[test]
    fn exact_matches_beat_prefix_matches_when_resolving_a_typed_key() {
        let all = vec![
            shortcut("git", "https://git-scm.com/", 1, 1),
            shortcut("github", "https://github.com/", 99, 1),
        ];
        // "git" is exact even though "github" is used far more.
        let hit = resolve_by_key(&all, "git").unwrap();
        assert_eq!(all[hit].key, "git");
        // A prefix-only query falls through to ranking.
        let hit = resolve_by_key(&all, "gi").unwrap();
        assert_eq!(all[hit].key, "github");
    }

    #[test]
    fn key_resolution_is_case_insensitive() {
        let all = sample();
        let hit = resolve_by_key(&all, "GOOGLE").unwrap();
        assert_eq!(all[hit].key, "google");
        assert!(resolve_by_key(&all, "").is_none());
    }

    #[test]
    fn the_placeholder_only_appears_on_an_exact_parameterised_key() {
        let all = sample();
        assert_eq!(placeholder_for(&all, "google"), " query");
        assert_eq!(placeholder_for(&all, "goog"), "");
        assert_eq!(placeholder_for(&all, "qgo"), "");
    }

    #[test]
    fn tab_cycles_through_the_matches_and_wraps() {
        let all = sample();
        let visible = filter_indices(&all, "g", false);
        let mut ac = Autocomplete::default();

        let first = ac.advance(&all, "g", None, &visible).unwrap();
        let second = ac.advance(&all, "g", None, &visible).unwrap();
        assert_ne!(first.chosen, second.chosen);

        // One full lap returns to where it started.
        let count = visible.len();
        assert!(count >= 2);
        let mut last = second;
        for _ in 0..count - 1 {
            last = ac.advance(&all, "g", None, &visible).unwrap();
        }
        assert_eq!(last.chosen, first.chosen);
    }

    #[test]
    fn tab_appends_a_space_for_parameterised_shortcuts() {
        let all = sample();
        let visible = filter_indices(&all, "google", false);
        let mut ac = Autocomplete::default();
        let r = ac.advance(&all, "google", None, &visible).unwrap();
        assert_eq!(r.search_text, "google ");
        assert_eq!(r.placeholder, " query");
    }

    #[test]
    fn tab_does_nothing_once_the_user_is_typing_a_parameter() {
        let all = sample();
        let mut ac = Autocomplete::default();
        assert!(ac.advance(&all, "google rust", None, &[]).is_none());
        assert!(ac.advance(&all, "   ", None, &[]).is_none());
    }
}
