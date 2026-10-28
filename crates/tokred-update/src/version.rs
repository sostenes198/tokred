//! Version comparison.

use semver::Version;

/// Parses a version, tolerating surrounding whitespace and a leading `v` (`v0.4.0`).
pub(crate) fn parse(text: &str) -> Option<Version> {
    let trimmed = text.trim();
    let bare = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    Version::parse(bare).ok()
}

/// Whether `latest` is a newer version than `current`. A leading `v` is tolerated; text that is
/// not a semantic version never counts as newer.
#[must_use]
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse(current), parse(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semantic_versions() {
        assert!(is_newer("0.3.2", "0.4.0"));
        assert!(is_newer("0.3.2", "0.3.10"), "numeric, not lexical");
        assert!(is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.5.0", "0.4.0"));
        assert!(!is_newer("1.0.0", "1.0.0-rc.1"));
    }

    #[test]
    fn tolerates_a_leading_v_and_whitespace() {
        assert!(is_newer("v0.3.2", "v0.4.0"));
        assert!(is_newer("0.3.2", " V0.4.0\n"));
        assert_eq!(parse("v1.2.3"), Version::parse("1.2.3").ok());
    }

    #[test]
    fn garbage_is_never_newer() {
        assert!(!is_newer("0.3.2", "latest"));
        assert!(!is_newer("dev", "0.4.0"));
        assert!(!is_newer("", ""));
    }
}
