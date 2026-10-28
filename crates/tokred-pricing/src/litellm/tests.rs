use super::*;
use tokred_core::pricing::Family;

fn row(provider: &str, input: f64, output: f64, extra: &str) -> String {
    format!(
        r#"{{"litellm_provider":"{provider}","mode":"chat","input_cost_per_token":{input},"output_cost_per_token":{output}{extra}}}"#
    )
}

fn parse(models: &[(&str, String)]) -> Vec<PriceEntry> {
    let body: Vec<String> = models
        .iter()
        .map(|(key, value)| format!("\"{key}\":{value}"))
        .collect();
    parse_litellm(&format!("{{{}}}", body.join(",")), Timestamp(7)).unwrap()
}

fn input_of(entries: &[PriceEntry], model: &str) -> f64 {
    entries
        .iter()
        .find(|e| e.model == model)
        .unwrap_or_else(|| panic!("{model} missing in {entries:?}"))
        .price
        .input
}

fn ids(entries: &[PriceEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.model.as_str()).collect()
}

#[test]
fn converts_per_token_to_per_million_with_explicit_cache_prices() {
    let cache =
        r#","cache_creation_input_token_cost":0.00000375,"cache_read_input_token_cost":3e-7"#;
    let entries = parse(&[(
        "claude-sonnet-4-5",
        row("anthropic", 0.000_003, 0.000_015, cache),
    )]);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.model, "claude-sonnet-4-5");
    assert_eq!(e.source, LITELLM_SOURCE);
    assert_eq!(e.updated_at, Timestamp(7));
    // Exact values: rounding removes floating point noise such as 2.9999999999999996.
    assert_eq!(
        e.price,
        ModelPrice {
            input: 3.0,
            output: 15.0,
            cache_write: 3.75,
            cache_write_1h: 6.0,
            cache_read: 0.3
        }
    );
}

#[test]
fn derives_missing_cache_prices_from_the_input_price() {
    let entries = parse(&[(
        "claude-3-opus-20240229",
        row("anthropic", 0.000_015, 0.000_075, ""),
    )]);
    let price = entries[0].price;
    assert_eq!(price.cache_write, 18.75);
    assert_eq!(price.cache_write_1h, 30.0);
    assert_eq!(price.cache_read, 1.5);
}

#[test]
fn reads_the_one_hour_cache_write_price_when_present() {
    let entries = parse(&[(
        "claude-sonnet-5-5",
        row(
            "anthropic",
            0.000_002,
            0.000_01,
            r#","cache_creation_input_token_cost":0.0000025,"cache_creation_input_token_cost_above_1hr":0.000004,"cache_read_input_token_cost":0.0000001"#,
        ),
    )]);
    let price = entries[0].price;
    assert_eq!(price.cache_write, 2.5);
    assert_eq!(price.cache_write_1h, 4.0);
    assert_eq!(price.cache_read, 0.1);
}

#[test]
fn normalises_provider_prefixes_and_version_suffixes() {
    let entries = parse(&[
        (
            "anthropic/claude-a-1",
            row("anthropic", 0.000_001, 0.000_002, ""),
        ),
        (
            "vertex_ai/claude-b-1@20250101",
            row("vertex_ai", 0.000_002, 0.000_003, ""),
        ),
        (
            "vertex_ai/claude-c-1@default",
            row("vertex_ai", 0.000_003, 0.000_004, ""),
        ),
        (
            "bedrock/claude-d-1-v1:0",
            row("bedrock", 0.000_004, 0.000_005, ""),
        ),
        (
            "anthropic.claude-e-1-20250101-v2:0",
            row("bedrock", 0.000_005, 0.000_006, ""),
        ),
        (
            "anthropic.claude-f-1",
            row("bedrock", 0.000_006, 0.000_007, ""),
        ),
        ("claude-g-1-v2", row("bedrock", 0.000_007, 0.000_008, "")),
    ]);
    assert_eq!(
        ids(&entries),
        [
            "claude-a-1",
            "claude-b-1-20250101",
            "claude-c-1",
            "claude-d-1",
            "claude-e-1-20250101",
            "claude-f-1",
            "claude-g-1"
        ],
        "sorted and normalised"
    );
}

#[test]
fn ignores_regional_reseller_and_unusable_rows() {
    let ok = row("anthropic", 0.000_001, 0.000_002, "");
    let entries = parse(&[
        ("claude-ok-1", ok.clone()),
        ("us.anthropic.claude-regional-1", ok.clone()),
        ("bedrock/us-east-1/anthropic.claude-region-1", ok.clone()),
        ("databricks/databricks-claude-reseller-1", ok.clone()),
        ("gpt-4", ok.clone()),
        ("claude-v2:1", ok.clone()),
        ("claude-embedding-1", ok.replace("chat", "embedding")),
        (
            "claude-no-input",
            r#"{"mode":"chat","output_cost_per_token":1e-6}"#.to_owned(),
        ),
        (
            "claude-no-output",
            r#"{"mode":"chat","input_cost_per_token":1e-6}"#.to_owned(),
        ),
        (
            "claude-negative",
            row("anthropic", -0.000_001, 0.000_002, ""),
        ),
        ("claude-not-an-object", "\"text\"".to_owned()),
        ("claude-bad-version-1:", ok.clone()),
        ("sample_spec", "{}".to_owned()),
    ]);
    assert_eq!(ids(&entries), ["claude-ok-1"]);
}

#[test]
fn the_most_authoritative_copy_of_a_model_wins() {
    let cheap = row("x", 0.000_001, 0.000_002, "");
    let mid = row("x", 0.000_002, 0.000_003, "");
    let dear = row("x", 0.000_004, 0.000_005, "");
    let native = row("x", 0.000_005, 0.000_006, "");
    // Worst copies first: a better one later in the file must still win.
    let entries = parse(&[
        ("anthropic.claude-m-1-v1:0", native),
        ("claude-m-1-v1:0", dear.clone()),
        ("bedrock/claude-m-1", dear),
        ("vertex_ai/claude-m-1", mid.clone()),
        ("anthropic/claude-m-1", mid),
        ("claude-m-1", cheap),
        // A later copy of the same rank does not replace the first one.
        ("anthropic/claude-m-1", row("x", 0.000_009, 0.000_009, "")),
    ]);
    assert_eq!(entries.len(), 1);
    assert_eq!(input_of(&entries, "claude-m-1"), 1.0);
    let swapped = parse(&[
        ("vertex_ai/claude-n-1", row("x", 0.000_002, 0.000_003, "")),
        ("anthropic/claude-n-1", row("x", 0.000_001, 0.000_002, "")),
    ]);
    assert_eq!(input_of(&swapped, "claude-n-1"), 1.0);
}

#[test]
fn rejects_unusable_files() {
    assert!(matches!(
        parse_litellm("not json", Timestamp(0)),
        Err(PriceError::Json(_))
    ));
    assert!(matches!(
        parse_litellm("[]", Timestamp(0)),
        Err(PriceError::NotAnObject)
    ));
    assert!(matches!(
        parse_litellm(r#"{"gpt-4":{"mode":"chat"}}"#, Timestamp(0)),
        Err(PriceError::NoClaudeModels)
    ));
    for err in [
        PriceError::NotAnObject,
        PriceError::NoClaudeModels,
        PriceError::from(serde_json::from_str::<Value>("x").unwrap_err()),
    ] {
        assert!(!err.to_string().is_empty());
    }
}

#[test]
fn the_embedded_snapshot_is_valid_and_covers_every_family() {
    let entries = fallback_entries(Timestamp(42));
    assert!(entries.len() > 10, "{} models", entries.len());
    assert!(entries.iter().all(|e| e.source == EMBEDDED_SOURCE));
    assert!(entries.iter().all(|e| e.updated_at == Timestamp(42)));
    for family in Family::ALL {
        assert!(
            entries
                .iter()
                .any(|e| Family::from_model(&e.model) == Some(family)),
            "{family:?} missing"
        );
    }
    for entry in &entries {
        let p = entry.price;
        assert!(p.input > 0.0 && p.output >= p.input, "{entry:?}");
        assert!(
            p.cache_write >= p.input && p.cache_read <= p.input,
            "{entry:?}"
        );
    }
}

#[test]
fn snapshots_round_trip_through_the_parser() {
    let entries = fallback_entries(Timestamp(1));
    let text = render_snapshot(&entries, "test note");
    assert!(text.contains("test note") && text.ends_with("}\n"));
    // One model per line; the first shares its line with the `_generated` note.
    assert_eq!(text.lines().count(), entries.len());
    let again = parse_litellm(&text, Timestamp(1)).unwrap();
    assert_eq!(again.len(), entries.len());
    for (a, b) in again.iter().zip(&entries) {
        assert_eq!((a.model.as_str(), a.price), (b.model.as_str(), b.price));
    }
}
