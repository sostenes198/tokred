//! The model catalog (spec 7.3): per model, its family, tier and the request features it
//! accepts. Data, not code: the embedded `data/models.json` can be replaced by an updated file
//! without touching the policy.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The catalog shipped with this build.
const EMBEDDED: &str = include_str!("../data/models.json");

/// What a model does with a request that has no `thinking` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DefaultThinking {
    /// It thinks adaptively.
    Adaptive,
    /// It does not think.
    Off,
}

impl DefaultThinking {
    /// Lowercase name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive",
            Self::Off => "off",
        }
    }
}

/// One model of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ModelInfo {
    /// Canonical model id (`claude-sonnet-5-5`).
    pub id: String,
    /// Other ids of the same model (dated snapshots).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Family (`opus`, `sonnet`, `haiku`, `fable`, `mythos`).
    pub family: String,
    /// Default tier (`cheap`, `mid`, `strong`).
    pub tier: String,
    /// Lifecycle (`current`, `legacy`, `deprecated`, `retired`).
    pub status: String,
    /// Behaviour without a `thinking` field.
    pub default_thinking: DefaultThinking,
    /// Largest accepted `max_tokens`.
    pub max_output: u64,
    /// Accepted `output_config.effort` values.
    #[serde(default)]
    pub effort: Vec<String>,
    /// Request features the model accepts (`adaptive_thinking`, `context_1m`, ...).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Models whose thinking blocks this model can read (empty: not documented).
    #[serde(default)]
    pub reads_thinking_from: Vec<String>,
}

impl ModelInfo {
    /// Whether the model accepts a request feature.
    #[must_use]
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }

    /// Whether the model accepts an effort level.
    #[must_use]
    pub fn accepts_effort(&self, level: &str) -> bool {
        self.effort.iter().any(|e| e == level)
    }
}

/// What an `anthropic-beta` value asks of the target model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BetaNeed<'a> {
    /// Nothing: it does not depend on the model.
    Neutral,
    /// The target must have this capability.
    Capability(&'a str),
    /// Not in the catalog: never route.
    Unknown,
}

/// Failure to read a catalog file.
#[derive(Debug, thiserror::Error)]
#[error("invalid model catalog: {0}")]
pub struct CatalogError(#[from] serde_json::Error);

/// The model catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Catalog {
    /// Version of the data (a date).
    #[serde(default)]
    pub version: String,
    /// `anthropic-beta` prefixes that do not depend on the model.
    #[serde(default)]
    pub neutral_betas: Vec<String>,
    /// `anthropic-beta` prefix -> the capability the target must have.
    #[serde(default)]
    pub capability_betas: BTreeMap<String, String>,
    /// Top-level body fields that do not depend on the model.
    #[serde(default)]
    pub neutral_fields: Vec<String>,
    /// Tier -> routing targets, in order of preference.
    #[serde(default)]
    pub tiers: BTreeMap<String, Vec<String>>,
    /// The models.
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

impl Catalog {
    /// Parses a catalog file.
    pub fn from_json(text: &str) -> Result<Self, CatalogError> {
        Ok(serde_json::from_str(text)?)
    }

    /// The catalog embedded in this build. An unreadable file (impossible for the tested
    /// embedded one) gives an empty catalog, which routes nothing.
    #[must_use]
    pub fn embedded() -> Self {
        Self::from_json(EMBEDDED).unwrap_or_default()
    }

    /// The model with this id or alias. Unknown models are never routed.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<&ModelInfo> {
        self.models
            .iter()
            .find(|m| m.id == id || m.aliases.iter().any(|a| a == id))
    }

    /// The routing targets of a tier, in order of preference (unknown ids skipped).
    #[must_use]
    pub fn tier_targets(&self, tier: &str) -> Vec<&ModelInfo> {
        self.tiers
            .get(tier)
            .map(|ids| ids.iter().filter_map(|id| self.model(id)).collect())
            .unwrap_or_default()
    }

    /// Whether `reader` can read thinking blocks written by `writer` (a model always reads its
    /// own). `false` when the documentation does not say.
    #[must_use]
    pub fn reads_thinking(&self, reader: &str, writer: &str) -> bool {
        let canonical = |id: &str| {
            self.model(id)
                .map_or_else(|| id.to_owned(), |m| m.id.clone())
        };
        let (reader, writer) = (canonical(reader), canonical(writer));
        reader == writer
            || self
                .model(&reader)
                .is_some_and(|m| m.reads_thinking_from.contains(&writer))
    }

    /// What an `anthropic-beta` value asks of the target model.
    pub(crate) fn beta_need(&self, beta: &str) -> BetaNeed<'_> {
        if self
            .neutral_betas
            .iter()
            .any(|p| beta.starts_with(p.as_str()))
        {
            return BetaNeed::Neutral;
        }
        self.capability_betas
            .iter()
            .find(|(prefix, _)| beta.starts_with(prefix.as_str()))
            .map_or(BetaNeed::Unknown, |(_, capability)| {
                BetaNeed::Capability(capability)
            })
    }

    /// Whether a top-level body field is model-independent.
    pub(crate) fn is_neutral_field(&self, field: &str) -> bool {
        self.neutral_fields.iter().any(|f| f == field)
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::pricing::Family;

    use super::*;

    #[test]
    fn the_embedded_catalog_parses_and_covers_the_current_models() {
        let catalog = Catalog::embedded();
        assert!(!catalog.version.is_empty());
        for id in [
            "claude-haiku-5-5",
            "claude-sonnet-5-5",
            "claude-sonnet-5",
            "claude-opus-5-5",
            "claude-opus-5",
            "claude-fable-5-1",
            "claude-haiku-4-5-20251001",
            "claude-opus-4-1-20250805",
            "claude-sonnet-4-20250514",
        ] {
            assert!(catalog.model(id).is_some(), "{id}");
        }
        assert!(catalog.model("claude-3-haiku").is_none());
        // Every tier target exists and is never retired.
        for (tier, ids) in &catalog.tiers {
            for id in ids {
                let model = catalog.model(id).unwrap();
                assert_ne!(model.status, "retired", "{tier}: {id}");
            }
        }
        // Families agree with the pricing crate's detection.
        for model in &catalog.models {
            assert_eq!(
                Family::from_model(&model.id).map(Family::name),
                Some(model.family.as_str()),
                "{}",
                model.id
            );
        }
    }

    #[test]
    fn the_documented_capabilities_are_recorded() {
        let c = Catalog::embedded();
        let m = |id| c.model(id).unwrap();
        // Adaptive thinking from the 4.6 generation on; manual budgets up to 4.6.
        assert!(m("claude-haiku-5-5").has("adaptive_thinking"));
        assert!(!m("claude-haiku-4-5").has("adaptive_thinking"));
        assert!(m("claude-haiku-4-5").has("budget_thinking"));
        assert!(!m("claude-sonnet-5-5").has("budget_thinking"));
        assert!(m("claude-sonnet-4-6").has("budget_thinking"));
        // Effort levels.
        assert!(m("claude-haiku-5-5").accepts_effort("xhigh"));
        assert!(!m("claude-sonnet-4-6").accepts_effort("xhigh"));
        assert!(!m("claude-haiku-4-5").accepts_effort("low"));
        // 1M context and the window of older models.
        assert!(m("claude-haiku-5-5").has("context_1m"));
        assert!(!m("claude-haiku-4-5").has("context_1m"));
        assert!(!m("claude-sonnet-4-5").has("context_1m"));
        // Interleaved thinking is not supported on Haiku 4.5.
        assert!(!m("claude-haiku-4-5").has("interleaved_thinking"));
        assert_eq!(m("claude-opus-4-8").default_thinking, DefaultThinking::Off);
        assert_eq!(DefaultThinking::Adaptive.as_str(), "adaptive");
        assert_eq!(DefaultThinking::Off.as_str(), "off");
    }

    #[test]
    fn thinking_readability_follows_the_preserved_thinking_page() {
        let c = Catalog::embedded();
        assert!(c.reads_thinking("claude-sonnet-5-5", "claude-haiku-5-5"));
        assert!(c.reads_thinking("claude-opus-5-5", "claude-sonnet-5-5"));
        assert!(!c.reads_thinking("claude-haiku-5-5", "claude-sonnet-5-5"));
        assert!(!c.reads_thinking("claude-sonnet-5-5", "claude-opus-5-5"));
        assert!(c.reads_thinking("claude-haiku-4-5-20251001", "claude-haiku-4-5"));
        assert!(c.reads_thinking("x", "x"), "a model reads its own blocks");
        assert!(!c.reads_thinking("unknown", "claude-haiku-5-5"));
    }

    #[test]
    fn tiers_resolve_in_order_and_skip_unknown_ids() {
        let mut c = Catalog::embedded();
        let ids = |c: &Catalog, tier| {
            c.tier_targets(tier)
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&c, "cheap"), ["claude-haiku-5-5", "claude-haiku-4-5"]);
        assert!(ids(&c, "keep").is_empty());
        c.tiers.insert(
            "cheap".to_owned(),
            vec!["nope".to_owned(), "claude-haiku-5-5".to_owned()],
        );
        assert_eq!(ids(&c, "cheap"), ["claude-haiku-5-5"]);
    }

    #[test]
    fn betas_and_fields_are_classified() {
        let c = Catalog::embedded();
        assert_eq!(c.beta_need("oauth-2025-04-20"), BetaNeed::Neutral);
        assert_eq!(
            c.beta_need("context-1m-2025-08-07"),
            BetaNeed::Capability("context_1m")
        );
        assert_eq!(c.beta_need("brand-new-2027-01-01"), BetaNeed::Unknown);
        assert!(c.is_neutral_field("messages"));
        assert!(!c.is_neutral_field("speed"));
    }

    #[test]
    fn a_bad_file_is_an_error_and_the_default_catalog_is_empty() {
        let err = Catalog::from_json("{").unwrap_err();
        assert!(err.to_string().contains("invalid model catalog"));
        let empty = Catalog::from_json("{}").unwrap();
        assert_eq!(empty, Catalog::default());
        assert!(empty.model("claude-haiku-5-5").is_none());
    }
}
