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

/// A version as written in a release tag (`0.2.0-rc.1`): `text` without whitespace or a leading
/// `v`, when it is a semantic version. Anything else (it ends up in a download address) is `None`.
#[must_use]
pub fn normalize(text: &str) -> Option<String> {
    parse(text).map(|version| version.to_string())
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
    fn a_version_is_written_the_way_a_tag_names_it() {
        assert_eq!(normalize("v0.2.0-rc.1").as_deref(), Some("0.2.0-rc.1"));
        assert_eq!(normalize(" 1.0.0\n").as_deref(), Some("1.0.0"));
        for bad in ["", "latest", "1.0", "../x", "1.0.0/../../y", "v"] {
            assert_eq!(normalize(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn garbage_is_never_newer() {
        assert!(!is_newer("0.3.2", "latest"));
        assert!(!is_newer("dev", "0.4.0"));
        assert!(!is_newer("", ""));
    }
}
