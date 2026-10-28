//! What a prompt would have cost on the next cheaper model family (spec 5.6).
//!
//! Only the **main loop** is re-priced: subagents and compaction keep their real cost on both
//! sides, so they cancel out of the saving. Switching the main model rebuilds the whole prompt
//! cache, so the counterfactual also pays for writing the prompt's context once on the target
//! model (at the 1-hour write price when the session writes 1-hour cache). Charging that rebuild
//! to every prompt is deliberately conservative.

use tokred_core::pricing::{Family, PriceTable, Usage, cost};

/// The family one step cheaper: opus, fable and mythos go to sonnet, sonnet to haiku, and haiku
/// has nothing cheaper (so tokred never suggests anything to a haiku main loop).
#[must_use]
pub const fn cheaper_family(family: Family) -> Option<Family> {
    match family {
        Family::Opus | Family::Fable | Family::Mythos => Some(Family::Sonnet),
        Family::Sonnet => Some(Family::Haiku),
        Family::Haiku => None,
    }
}

/// The cost of a prompt's main loop on the cheaper model.
#[derive(Debug, Clone, PartialEq)]
pub struct Counterfactual {
    /// The latest model of the cheaper family in the price table.
    pub target_model: String,
    /// The cheaper family.
    pub target_family: Family,
    /// The main loop's usage priced at the target model, USD.
    pub target_usd: f64,
    /// Writing the prompt's context to the target model's cache once, USD.
    pub rebuild_usd: f64,
}

impl Counterfactual {
    /// `real - target`, before paying for the cache rebuild.
    #[must_use]
    pub fn gross_saving(&self, main_real_usd: f64) -> f64 {
        main_real_usd - self.target_usd
    }

    /// `real - (target + rebuild)`: the saving net of rebuilding the cache.
    #[must_use]
    pub fn net_saving(&self, main_real_usd: f64) -> f64 {
        self.gross_saving(main_real_usd) - self.rebuild_usd
    }
}

/// The counterfactual of a main loop that ran on `model` with `main_usage` and started from a
/// context of `context_tokens`; `None` when the model has no cheaper family or the table has
/// no price for it.
#[must_use]
pub fn counterfactual(
    prices: &PriceTable,
    model: &str,
    main_usage: &Usage,
    context_tokens: u64,
    one_hour_cache: bool,
) -> Option<Counterfactual> {
    let target_family = cheaper_family(Family::from_model(model)?)?;
    let target = prices.latest_of(target_family)?;
    let rebuild = Usage {
        cache_write: context_tokens,
        cache_write_1h: if one_hour_cache { context_tokens } else { 0 },
        ..Usage::default()
    };
    Some(Counterfactual {
        target_model: target.model.clone(),
        target_family,
        target_usd: cost(main_usage, &target.price),
        rebuild_usd: cost(&rebuild, &target.price),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use tokred_core::Timestamp;
    use tokred_core::pricing::{ModelPrice, PriceEntry};

    use super::*;

    /// Round prices (USD per million tokens) so the expected costs are easy to check by hand.
    pub(crate) fn prices() -> PriceTable {
        let entry = |model: &str, input: f64| PriceEntry {
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
        };
        PriceTable::new(vec![
            entry("claude-opus-5-5", 10.0),
            entry("claude-sonnet-4-5", 3.0),
            entry("claude-sonnet-5-5", 2.0),
            entry("claude-haiku-5-5", 1.0),
            entry("claude-fable-5-1", 20.0),
        ])
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn each_family_steps_down_once() {
        assert_eq!(cheaper_family(Family::Opus), Some(Family::Sonnet));
        assert_eq!(cheaper_family(Family::Fable), Some(Family::Sonnet));
        assert_eq!(cheaper_family(Family::Mythos), Some(Family::Sonnet));
        assert_eq!(cheaper_family(Family::Sonnet), Some(Family::Haiku));
        assert_eq!(cheaper_family(Family::Haiku), None);
    }

    #[test]
    fn the_counterfactual_reprices_the_main_loop_and_adds_the_rebuild() {
        let usage = Usage {
            input: 1_000,
            output: 2_000,
            cache_write: 0,
            cache_write_1h: 0,
            cache_read: 100_000,
        };
        // Real on opus (10 / 50 / 12.5 / 20 / 1 per million):
        // 0.01 input + 0.1 output + 0.1 cache read = 0.21.
        let real = cost(&usage, &prices().exact("claude-opus-5-5").unwrap().price);
        assert!(close(real, 0.21), "{real}");

        let cf = counterfactual(&prices(), "claude-opus-5-5", &usage, 101_000, false).unwrap();
        // The latest sonnet (5.5 beats 4.5): 2 / 10 / 2.5 / 4 / 0.2 per million.
        assert_eq!(cf.target_model, "claude-sonnet-5-5");
        assert_eq!(cf.target_family, Family::Sonnet);
        // 0.002 input + 0.02 output + 0.02 cache read.
        assert!(close(cf.target_usd, 0.042), "{}", cf.target_usd);
        // 101,000 tokens written at 2.5 per million.
        assert!(close(cf.rebuild_usd, 0.2525), "{}", cf.rebuild_usd);
        assert!(close(cf.gross_saving(real), 0.168));
        assert!(close(cf.net_saving(real), 0.21 - (0.042 + 0.2525)));

        // A session that writes 1-hour cache rebuilds at the 1-hour price (4 per million).
        let hour = counterfactual(&prices(), "claude-opus-5-5", &usage, 101_000, true).unwrap();
        assert!(close(hour.rebuild_usd, 0.404), "{}", hour.rebuild_usd);
        assert!(close(hour.target_usd, cf.target_usd));
    }

    #[test]
    fn sonnet_steps_to_haiku_and_haiku_has_no_counterfactual() {
        let usage = Usage {
            output: 1_000_000,
            ..Usage::default()
        };
        let cf = counterfactual(&prices(), "claude-sonnet-5-5", &usage, 0, false).unwrap();
        assert_eq!(cf.target_model, "claude-haiku-5-5");
        assert!(close(cf.target_usd, 5.0) && close(cf.rebuild_usd, 0.0));
        let fable = counterfactual(&prices(), "claude-fable-5-1", &usage, 0, false).unwrap();
        assert_eq!(fable.target_model, "claude-sonnet-5-5");

        assert!(counterfactual(&prices(), "claude-haiku-5-5", &usage, 9, false).is_none());
        assert!(counterfactual(&prices(), "<synthetic>", &usage, 9, false).is_none());
        let no_haiku = PriceTable::new(
            prices()
                .entries()
                .iter()
                .filter(|e| !e.model.contains("haiku"))
                .cloned()
                .collect(),
        );
        assert!(counterfactual(&no_haiku, "claude-sonnet-5-5", &usage, 9, false).is_none());
    }
}
