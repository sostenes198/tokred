//! Token prices and cost arithmetic (pure).
//!
//! Prices are USD per **million** tokens. Models seen in transcripts but missing from the table
//! are mapped to the latest price of their family (opus, sonnet or haiku) and flagged, so the
//! numbers stay useful without pretending to be exact.

use crate::error::StoreError;
use crate::types::Timestamp;

/// A model family tokred knows how to route to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    /// Architecture and planning.
    Opus,
    /// Development and review.
    Sonnet,
    /// Search and summaries.
    Haiku,
    /// The Fable models.
    Fable,
    /// The Mythos models.
    Mythos,
}

impl Family {
    /// Every family.
    pub const ALL: [Self; 5] = [
        Self::Opus,
        Self::Sonnet,
        Self::Haiku,
        Self::Fable,
        Self::Mythos,
    ];

    /// Detects the family from a model id (`claude-opus-4-1-20250805`, `claude-3-haiku`, ...).
    #[must_use]
    pub fn from_model(model: &str) -> Option<Self> {
        let lower = model.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|family| lower.contains(family.name()))
    }

    /// Lowercase name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Fable => "fable",
            Self::Mythos => "mythos",
        }
    }
}

/// Prices of one model, in USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    /// Uncached input.
    pub input: f64,
    /// Output.
    pub output: f64,
    /// Prompt-cache writes with the default 5-minute lifetime.
    pub cache_write: f64,
    /// Prompt-cache writes with the 1-hour lifetime.
    pub cache_write_1h: f64,
    /// Prompt-cache reads.
    pub cache_read: f64,
}

/// One row of the `pricing` table.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceEntry {
    /// Model id as it appears in transcripts.
    pub model: String,
    /// Prices.
    pub price: ModelPrice,
    /// Where the row came from (`litellm`, `embedded`, ...).
    pub source: String,
    /// When it was last refreshed.
    pub updated_at: Timestamp,
}

/// Token counts of one or many messages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens written to the prompt cache (both lifetimes).
    pub cache_write: u64,
    /// The part of `cache_write` written with the 1-hour lifetime (priced higher).
    pub cache_write_1h: u64,
    /// Tokens read from the prompt cache.
    pub cache_read: u64,
}

impl Usage {
    /// Sum of all four counters.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.input + self.output + self.cache_write + self.cache_read
    }

    /// Adds another usage.
    pub fn add(&mut self, other: &Self) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_write_1h += other.cache_write_1h;
        self.cache_read += other.cache_read;
    }
}

/// How a model's price was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The table has this exact model.
    Exact,
    /// Not in the table: the latest price of the model's family was used.
    FamilyFallback(Family),
}

/// A price together with how it was found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    /// The price to use.
    pub price: ModelPrice,
    /// How it was found.
    pub resolution: Resolution,
}

/// The price table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceTable {
    entries: Vec<PriceEntry>,
}

impl PriceTable {
    /// Builds a table from rows.
    #[must_use]
    pub fn new(entries: Vec<PriceEntry>) -> Self {
        Self { entries }
    }

    /// The rows.
    #[must_use]
    pub fn entries(&self) -> &[PriceEntry] {
        &self.entries
    }

    /// Row for exactly this model id.
    #[must_use]
    pub fn exact(&self, model: &str) -> Option<&PriceEntry> {
        self.entries.iter().find(|entry| entry.model == model)
    }

    /// The "latest" row of a family: the highest version in its id (`claude-opus-5-5` is 5.5,
    /// `claude-opus-4-1-20250805` is 4.1), then the most recent date, then the id itself. Current
    /// ids carry no date, so the version must rank first or `claude-opus-4-7-20260416` would beat
    /// `claude-opus-5-5`.
    #[must_use]
    pub fn latest_of(&self, family: Family) -> Option<&PriceEntry> {
        let rank = |model: &str| (id_version(model), id_date(model));
        self.entries
            .iter()
            .filter(|entry| Family::from_model(&entry.model) == Some(family))
            .max_by(|a, b| (rank(&a.model), &a.model).cmp(&(rank(&b.model), &b.model)))
    }

    /// Price for `model`: exact, else the latest of its family, else `None`
    /// (for ids such as `<synthetic>` that belong to no family).
    #[must_use]
    pub fn resolve(&self, model: &str) -> Option<Resolved> {
        if let Some(entry) = self.exact(model) {
            return Some(Resolved {
                price: entry.price,
                resolution: Resolution::Exact,
            });
        }
        let family = Family::from_model(model)?;
        let entry = self.latest_of(family)?;
        Some(Resolved {
            price: entry.price,
            resolution: Resolution::FamilyFallback(family),
        })
    }
}

/// The `YYYYMMDD` suffix of a model id, if it has one.
fn id_date(model: &str) -> Option<u32> {
    model
        .rsplit('-')
        .find(|part| part.len() == 8 && part.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|part| part.parse().ok())
}

/// The version numbers in a model id, ignoring the date: `claude-opus-4-1-20250805` -> `[4, 1]`,
/// `claude-3-5-haiku` -> `[3, 5]`, `claude-mythos-preview` -> `[]`.
fn id_version(model: &str) -> Vec<u32> {
    model
        .split('-')
        .filter(|part| {
            !part.is_empty() && part.len() < 8 && part.bytes().all(|b| b.is_ascii_digit())
        })
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Cost in USD of `usage` at `price`.
#[must_use]
pub fn cost(usage: &Usage, price: &ModelPrice) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let per_million = |tokens: u64, usd: f64| tokens as f64 * usd / 1_000_000.0;
    let write_1h = usage.cache_write_1h.min(usage.cache_write);
    per_million(usage.input, price.input)
        + per_million(usage.output, price.output)
        + per_million(usage.cache_write - write_1h, price.cache_write)
        + per_million(write_1h, price.cache_write_1h)
        + per_million(usage.cache_read, price.cache_read)
}

/// Failure of an external data source (price list, release API).
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// The source could not be reached or timed out.
    #[error("source unavailable: {0}")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The source answered, but not with what was expected.
    #[error("unexpected response: {0}")]
    Invalid(String),
}

/// Remote source of prices (network adapter; never used inside a hook).
pub trait PriceSource {
    /// Fetches the current Claude prices.
    fn fetch(&self) -> Result<Vec<PriceEntry>, SourceError>;
}

/// Persistent price table (`pricing`).
pub trait PriceRepo {
    /// Replaces the whole table.
    fn replace_all(&self, entries: &[PriceEntry]) -> Result<(), StoreError>;
    /// Loads every row.
    fn load(&self) -> Result<Vec<PriceEntry>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(model: &str, input: f64) -> PriceEntry {
        PriceEntry {
            model: model.to_owned(),
            price: ModelPrice {
                input,
                output: input * 5.0,
                cache_write: input * 1.25,
                cache_write_1h: input * 2.0,
                cache_read: input * 0.1,
            },
            source: "test".to_owned(),
            updated_at: Timestamp(1),
        }
    }

    #[test]
    fn families_are_detected_from_model_ids() {
        assert_eq!(
            Family::from_model("claude-opus-4-1-20250805"),
            Some(Family::Opus)
        );
        assert_eq!(
            Family::from_model("claude-3-5-haiku-20241022"),
            Some(Family::Haiku)
        );
        assert_eq!(Family::from_model("Claude-Sonnet-4"), Some(Family::Sonnet));
        assert_eq!(Family::from_model("claude-fable-5-1"), Some(Family::Fable));
        assert_eq!(
            Family::from_model("claude-mythos-preview"),
            Some(Family::Mythos)
        );
        assert_eq!(Family::from_model("<synthetic>"), None);
        assert_eq!(Family::Opus.name(), "opus");
    }

    #[test]
    fn cost_is_per_million_tokens() {
        let price = ModelPrice {
            input: 3.0,
            output: 15.0,
            cache_write: 3.75,
            cache_write_1h: 6.0,
            cache_read: 0.3,
        };
        let usage = Usage {
            input: 1_000_000,
            output: 2_000_000,
            cache_write: 1_000_000,
            cache_write_1h: 0,
            cache_read: 10_000_000,
        };
        let got = cost(&usage, &price);
        assert!((got - (3.0 + 30.0 + 3.75 + 3.0)).abs() < 1e-9, "{got}");
        assert!(cost(&Usage::default(), &price).abs() < f64::EPSILON);

        // 1-hour writes are a part of the cache writes, priced at their own rate.
        let mixed = Usage {
            cache_write: 1_000_000,
            cache_write_1h: 400_000,
            ..Usage::default()
        };
        let got = cost(&mixed, &price);
        assert!((got - (0.6 * 3.75 + 0.4 * 6.0)).abs() < 1e-9, "{got}");
        // A 1-hour count larger than the total is clamped, never negative.
        let odd = Usage {
            cache_write: 100,
            cache_write_1h: 500,
            ..Usage::default()
        };
        assert!((cost(&odd, &price) - 100.0 * 6.0 / 1e6).abs() < 1e-12);
    }

    #[test]
    fn usage_adds_and_totals() {
        let mut a = Usage {
            input: 1,
            output: 2,
            cache_write: 3,
            cache_write_1h: 1,
            cache_read: 4,
        };
        a.add(&Usage {
            input: 10,
            output: 20,
            cache_write: 30,
            cache_write_1h: 5,
            cache_read: 40,
        });
        assert_eq!(
            a,
            Usage {
                input: 11,
                output: 22,
                cache_write: 33,
                cache_write_1h: 6,
                cache_read: 44
            }
        );
        assert_eq!(a.total(), 110);
    }

    #[test]
    fn resolve_prefers_exact_then_the_latest_of_the_family() {
        let table = PriceTable::new(vec![
            entry("claude-opus-4-20250514", 15.0),
            entry("claude-opus-4-1-20250805", 16.0),
            entry("claude-3-haiku-20240307", 0.25),
            entry("claude-sonnet-4-20250514", 3.0),
        ]);
        let exact = table.resolve("claude-opus-4-20250514").unwrap();
        assert_eq!(exact.resolution, Resolution::Exact);
        assert!((exact.price.input - 15.0).abs() < f64::EPSILON);

        let fallback = table.resolve("claude-opus-9-99999999").unwrap();
        assert_eq!(
            fallback.resolution,
            Resolution::FamilyFallback(Family::Opus)
        );
        assert!(
            (fallback.price.input - 16.0).abs() < f64::EPSILON,
            "the higher version (4.1 over 4) wins"
        );

        // Current ids carry no date: the version decides, so 5.5 beats a dated 4.7.
        let current = PriceTable::new(vec![
            entry("claude-opus-4-7-20260416", 5.0),
            entry("claude-opus-5", 5.5),
            entry("claude-opus-5-5", 4.0),
            entry("claude-sonnet-5", 2.0),
            entry("claude-sonnet-4-20250514", 3.0),
        ]);
        let opus = current.latest_of(Family::Opus).unwrap();
        assert_eq!(opus.model, "claude-opus-5-5");
        let got = current.resolve("claude-sonnet-x").unwrap();
        assert!((got.price.input - 2.0).abs() < f64::EPSILON, "5 beats 4");
        // Same version: the date breaks the tie; ids without numbers rank lowest.
        let dated = PriceTable::new(vec![
            entry("claude-opus-4-1-20250805", 16.0),
            entry("claude-opus-4-1-20250901", 17.0),
            entry("claude-opus-preview", 99.0),
        ]);
        assert_eq!(
            dated.latest_of(Family::Opus).unwrap().model,
            "claude-opus-4-1-20250901"
        );

        assert!(table.resolve("<synthetic>").is_none());
        assert!(PriceTable::default().resolve("claude-opus-4").is_none());
        assert_eq!(table.entries().len(), 4);
    }

    #[test]
    fn source_errors_are_readable() {
        let err = SourceError::Unavailable(Box::new(std::io::Error::other("timeout")));
        assert!(err.to_string().contains("timeout"));
        assert!(
            SourceError::Invalid("not json".into())
                .to_string()
                .contains("not json")
        );
    }
}
