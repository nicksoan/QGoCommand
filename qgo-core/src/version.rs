//! Application version handling.
//!
//! The version comes from the standalone Cargo workspace. Debug builds append
//! a `-dev` marker to the displayed version.

/// The version declared in the workspace `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// True for debug builds, matching the csproj's `<VersionSuffix>dev</VersionSuffix>`.
pub const fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

/// `VersionService.GetDisplayVersion()` — `2.1.1` or `2.1.1-dev`.
pub fn display_version() -> String {
    if is_dev_build() {
        format!("{VERSION}-dev")
    } else {
        VERSION.to_string()
    }
}

/// The version-specific changelog URL used by the update notification.
pub fn changelog_url(version: &str) -> String {
    format!(
        "https://qgocommand.com/changelog#{}",
        version.replace('.', "")
    )
}

/// Compares two dotted version strings numerically, so `2.10.0` sorts above
/// `2.9.0`. Non-numeric or missing components count as zero.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['+', '-'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(a), parse(b));
    let len = a.len().max(b.len());
    for i in 0..len {
        let ordering = a
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&b.get(i).copied().unwrap_or(0));
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    std::cmp::Ordering::Equal
}

/// True when `latest` is newer than the running build — the check behind
/// `UpdateDetectionService`.
pub fn is_update_available(latest: &str) -> bool {
    compare_versions(latest, VERSION) == std::cmp::Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn the_crate_version_comes_from_the_workspace() {
        assert_eq!(VERSION, "2.1.1");
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        assert_eq!(compare_versions("2.10.0", "2.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("2.1.1", "2.1.1"), Ordering::Equal);
        assert_eq!(compare_versions("2.1", "2.1.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0", "2.0.0"), Ordering::Less);
    }

    #[test]
    fn build_metadata_and_prefixes_are_ignored() {
        assert_eq!(
            compare_versions("v2.1.1", "2.1.1+build.20260805"),
            Ordering::Equal
        );
    }

    #[test]
    fn updates_are_only_offered_for_newer_versions() {
        assert!(is_update_available("9.0.0"));
        assert!(!is_update_available(VERSION));
        assert!(!is_update_available("0.1.0"));
    }

    #[test]
    fn changelog_url_matches_the_original_anchor_format() {
        assert_eq!(
            changelog_url("2.1.1"),
            "https://qgocommand.com/changelog#211"
        );
    }
}
