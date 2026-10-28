//! `ureq` adapters: the GitHub releases API ([`ReleaseSource`]) and release downloads
//! ([`Downloader`]).

use std::time::Duration;

use tokred_core::pricing::SourceError;
use tokred_core::release::{Release, ReleaseSource};

use crate::swap::Downloader;
use crate::version::parse;

/// GitHub API endpoint for the latest stable release.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/sostenes198/tokred/releases/latest";

/// Time budget of the daily version check (spec section 14).
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Time budget of a binary download.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// Release pages are small; asset downloads are binaries of a few MB.
const MAX_JSON_BYTES: u64 = 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

/// Reads the latest release from the GitHub API.
#[derive(Debug, Clone)]
pub struct GithubReleaseSource {
    url: String,
    timeout: Duration,
}

impl Default for GithubReleaseSource {
    fn default() -> Self {
        Self::new()
    }
}

impl GithubReleaseSource {
    /// Source for the real repository with the 2 s timeout of the spec.
    #[must_use]
    pub fn new() -> Self {
        Self {
            url: LATEST_RELEASE_URL.to_owned(),
            timeout: CHECK_TIMEOUT,
        }
    }

    /// Uses another URL (a mirror, or a local server in tests).
    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// Uses another overall timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn get_json(&self) -> Result<String, ureq::Error> {
        agent(self.timeout)
            .get(&self.url)
            .header("Accept", "application/vnd.github+json")
            .call()?
            .body_mut()
            .with_config()
            .limit(MAX_JSON_BYTES)
            .read_to_string()
    }
}

impl ReleaseSource for GithubReleaseSource {
    fn latest(&self) -> Result<Release, SourceError> {
        let body = self
            .get_json()
            .map_err(|err| SourceError::Unavailable(Box::new(err)))?;
        parse_release(&body)
    }
}

/// Reads `tag_name` and `html_url` from a GitHub release document.
pub(crate) fn parse_release(body: &str) -> Result<Release, SourceError> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|err| SourceError::Invalid(err.to_string()))?;
    let tag = json
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| SourceError::Invalid("the release has no tag_name".to_owned()))?;
    let version = parse(tag)
        .ok_or_else(|| SourceError::Invalid(format!("`{tag}` is not a version number")))?;
    let url = json
        .get("html_url")
        .and_then(serde_json::Value::as_str)
        .map_or_else(
            || format!("https://github.com/sostenes198/tokred/releases/tag/{tag}"),
            str::to_owned,
        );
    Ok(Release {
        version: version.to_string(),
        url,
    })
}

/// Downloads release assets over HTTPS.
#[derive(Debug, Clone)]
pub struct UreqDownloader {
    timeout: Duration,
}

impl Default for UreqDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqDownloader {
    /// Downloader with the default 2 minute budget per file.
    #[must_use]
    pub fn new() -> Self {
        Self {
            timeout: DOWNLOAD_TIMEOUT,
        }
    }

    /// Uses another overall timeout per file.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn get(&self, url: &str) -> Result<Vec<u8>, ureq::Error> {
        agent(self.timeout)
            .get(url)
            .call()?
            .body_mut()
            .with_config()
            .limit(MAX_DOWNLOAD_BYTES)
            .read_to_vec()
    }
}

impl Downloader for UreqDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>, SourceError> {
        self.get(url)
            .map_err(|err| SourceError::Unavailable(Box::new(err)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_release_document() {
        let release = parse_release(
            r#"{"tag_name":"v0.4.0","html_url":"https://github.com/x/y/releases/tag/v0.4.0","draft":false}"#,
        )
        .unwrap();
        assert_eq!(release.version, "0.4.0");
        assert_eq!(release.url, "https://github.com/x/y/releases/tag/v0.4.0");
    }

    #[test]
    fn a_missing_url_is_derived_from_the_tag() {
        let release = parse_release(r#"{"tag_name":"0.5.1"}"#).unwrap();
        assert_eq!(release.version, "0.5.1");
        assert_eq!(
            release.url,
            "https://github.com/sostenes198/tokred/releases/tag/0.5.1"
        );
    }

    #[test]
    fn malformed_documents_are_invalid() {
        for body in [
            "not json",
            "{}",
            r#"{"tag_name":7}"#,
            r#"{"tag_name":"nightly"}"#,
        ] {
            assert!(
                matches!(parse_release(body), Err(SourceError::Invalid(_))),
                "{body}"
            );
        }
    }
}
