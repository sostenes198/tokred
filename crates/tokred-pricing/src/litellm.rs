//! Parser for `LiteLLM`'s `model_prices_and_context_window.json` and the embedded snapshot.

use std::collections::HashMap;

use serde_json::{Map, Value};
use tokred_core::pricing::{ModelPrice, PriceEntry};
use tokred_core::types::Timestamp;

use crate::error::PriceError;

/// `source` of rows parsed from `LiteLLM`'s file.
pub const LITELLM_SOURCE: &str = "litellm";
/// `source` of rows that come from the snapshot embedded in the binary.
pub const EMBEDDED_SOURCE: &str = "embedded";

const SNAPSHOT: &str = include_str!("../data/claude-prices.json");

/// Cache writes cost this multiple of the input price when the file has no explicit price.
const DERIVED_CACHE_WRITE: f64 = 1.25;
/// Cache reads cost this multiple of the input price when the file has no explicit price.
const DERIVED_CACHE_READ: f64 = 0.1;
/// 1-hour cache writes cost this multiple of the input price when the file has no explicit price
/// (the documented rate for the 1-hour lifetime).
const DERIVED_CACHE_WRITE_1H: f64 = 2.0;

/// Provider prefixes of keys that are accepted, with their rank. When several keys normalise to
/// the same model id, the lowest rank wins (bare Anthropic keys have rank 0).
const PREFIXES: [(&str, u8); 3] = [("anthropic/", 1), ("vertex_ai/", 2), ("bedrock/", 3)];
/// Rank of ids whose Bedrock version suffix (`-v1:0`) was stripped.
const VERSIONED_RANK: u8 = 3;
/// Rank of Bedrock-native ids (`anthropic.claude-...`): the least authoritative copy of a price.
const NATIVE_RANK: u8 = 4;

/// Parses `LiteLLM`'s price file.
///
/// Keeps Claude models only. Provider prefixes (`anthropic/`, `vertex_ai/`, `bedrock/`) and
/// Vertex/Bedrock version suffixes (`@20251101`, `@default`, `-v1:0`) are removed so the id matches
/// the one in Claude Code transcripts; regional (`us.anthropic.`), per-region
/// (`bedrock/us-east-1/...`) and reseller keys are ignored, because their prices differ. Prices are
/// converted from USD per token to USD per million tokens. Missing cache prices are derived from
/// the input price (write = 1.25x, read = 0.1x). Tiered pricing for long contexts is ignored.
///
/// Fails when the text is not a JSON object or when no Claude model survives, so an empty result
/// can never replace a good table.
pub fn parse_litellm(json: &str, now: Timestamp) -> Result<Vec<PriceEntry>, PriceError> {
    let root: Value = serde_json::from_str(json)?;
    let Value::Object(models) = root else {
        return Err(PriceError::NotAnObject);
    };
    let mut best: HashMap<String, (u8, ModelPrice)> = HashMap::new();
    for (key, value) in &models {
        let Some((rank, id)) = normalize_model_id(key) else {
            continue;
        };
        let Some(price) = value.as_object().and_then(row_price) else {
            continue;
        };
        let slot = best.entry(id).or_insert((rank, price));
        if rank < slot.0 {
            *slot = (rank, price);
        }
    }
    let mut entries: Vec<PriceEntry> = best
        .into_iter()
        .map(|(model, (_, price))| PriceEntry {
            model,
            price,
            source: LITELLM_SOURCE.to_owned(),
            updated_at: now,
        })
        .collect();
    if entries.is_empty() {
        return Err(PriceError::NoClaudeModels);
    }
    entries.sort_by(|a, b| a.model.cmp(&b.model));
    Ok(entries)
}

/// The prices embedded in the binary (no network needed), stamped with `now`.
///
/// Returns an empty list only if the embedded snapshot is corrupt, which a unit test rules out.
#[must_use]
pub fn fallback_entries(now: Timestamp) -> Vec<PriceEntry> {
    parse_litellm(SNAPSHOT, now)
        .unwrap_or_default()
        .into_iter()
        .map(|mut entry| {
            EMBEDDED_SOURCE.clone_into(&mut entry.source);
            entry
        })
        .collect()
}

/// Renders `entries` as the embedded snapshot: `LiteLLM`'s own shape, keyed by the normalised id, so
/// that [`parse_litellm`] reads it back unchanged. `generated` is a free-form provenance note.
#[must_use]
pub fn render_snapshot(entries: &[PriceEntry], generated: &str) -> String {
    let mut root = Map::new();
    root.insert("_generated".to_owned(), Value::from(generated));
    for entry in entries {
        let mut row = Map::new();
        row.insert("litellm_provider".to_owned(), Value::from("anthropic"));
        row.insert("mode".to_owned(), Value::from("chat"));
        let fields = [
            ("input_cost_per_token", entry.price.input),
            ("output_cost_per_token", entry.price.output),
            ("cache_creation_input_token_cost", entry.price.cache_write),
            (
                "cache_creation_input_token_cost_above_1hr",
                entry.price.cache_write_1h,
            ),
            ("cache_read_input_token_cost", entry.price.cache_read),
        ];
        for (name, per_million) in fields {
            row.insert(name.to_owned(), Value::from(per_million / 1_000_000.0));
        }
        root.insert(entry.model.clone(), Value::Object(row));
    }
    // One model per line keeps diffs of the snapshot readable.
    let mut text = Value::Object(root).to_string().replace("},\"", "},\n\"");
    text.push('\n');
    text
}

/// Maps a `LiteLLM` key to `(rank, bare model id)`, or `None` when the key is not a Claude model
/// whose price we trust.
fn normalize_model_id(key: &str) -> Option<(u8, String)> {
    let (mut rank, rest) = PREFIXES
        .iter()
        .find_map(|(prefix, rank)| key.strip_prefix(prefix).map(|rest| (*rank, rest)))
        .unwrap_or((0, key));
    let rest = match rest.strip_prefix("anthropic.") {
        Some(native) => {
            rank = rank.max(NATIVE_RANK);
            native
        }
        None => rest,
    };
    let rest = rest.strip_suffix("@default").unwrap_or(rest);
    let stem = strip_version_suffix(rest);
    if stem.len() != rest.len() {
        rank = rank.max(VERSIONED_RANK);
    }
    let id = stem.replace('@', "-");
    let valid = id.starts_with("claude-") && !id.contains(['/', ':']);
    valid.then_some((rank, id))
}

/// Removes a trailing Bedrock version (`-v1:0`, `-v2`) from an id.
fn strip_version_suffix(id: &str) -> &str {
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    match id.rsplit_once("-v") {
        Some((stem, version))
            if version.split_once(':').map_or_else(
                || all_digits(version),
                |(a, b)| all_digits(a) && all_digits(b),
            ) =>
        {
            stem
        }
        _ => id,
    }
}

/// The price of one row, or `None` when the row is not a chat model with input and output prices.
fn row_price(row: &Map<String, Value>) -> Option<ModelPrice> {
    if row.get("mode").and_then(Value::as_str) != Some("chat") {
        return None;
    }
    let round = |per_million: f64| (per_million * 1_000_000.0).round() / 1_000_000.0;
    let per_million = |field: &str| {
        row.get(field)
            .and_then(Value::as_f64)
            .filter(|cost| *cost >= 0.0)
            .map(|cost| round(cost * 1_000_000.0))
    };
    let input = per_million("input_cost_per_token")?;
    let output = per_million("output_cost_per_token")?;
    Some(ModelPrice {
        input,
        output,
        cache_write: per_million("cache_creation_input_token_cost")
            .unwrap_or_else(|| round(input * DERIVED_CACHE_WRITE)),
        cache_write_1h: per_million("cache_creation_input_token_cost_above_1hr")
            .unwrap_or_else(|| round(input * DERIVED_CACHE_WRITE_1H)),
        cache_read: per_million("cache_read_input_token_cost")
            .unwrap_or_else(|| round(input * DERIVED_CACHE_READ)),
    })
}

#[cfg(test)]
mod tests;
