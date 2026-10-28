//! Network adapter of the [`PriceSource`] port: downloads `LiteLLM`'s price file.

use std::time::Duration;

use tokred_core::pricing::{PriceEntry, PriceSource, SourceError};
use tokred_core::types::Timestamp;

use crate::litellm::parse_litellm;

/// Raw GitHub URL of `LiteLLM`'s price file.
pub const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";

/// Default time budget for the whole request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The file is a few MB; refuse anything absurdly larger.
const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;

/// Fetches prices from `LiteLLM` over HTTPS.
///
/// The returned rows carry `updated_at = 0`; [`crate::sync_prices`] stamps them with the sync time.
#[derive(Debug, Clone)]
pub struct LiteLlmSource {
    url: String,
    timeout: Duration,
}

impl Default for LiteLlmSource {
    fn default() -> Self {
        Self::new()
    }
}

impl LiteLlmSource {
    /// Source for the public `LiteLLM` file with the default timeout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            url: LITELLM_URL.to_owned(),
            timeout: DEFAULT_TIMEOUT,
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

    fn download(&self) -> Result<String, ureq::Error> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build()
            .into();
        let mut response = agent.get(&self.url).call()?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
    }
}

impl PriceSource for LiteLlmSource {
    fn fetch(&self) -> Result<Vec<PriceEntry>, SourceError> {
        let body = self
            .download()
            .map_err(|err| SourceError::Unavailable(Box::new(err)))?;
        parse_litellm(&body, Timestamp(0)).map_err(|err| SourceError::Invalid(err.to_string()))
    }
}
