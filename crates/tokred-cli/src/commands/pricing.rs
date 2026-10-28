//! `tokred pricing`.

use anyhow::{Result, anyhow, bail};
use serde_json::json;
use tokred_pricing::{SyncOutcome, load_table, sync_prices};

use super::housekeeping::Network;
use super::{SystemClock, existing_store};
use crate::args::PricingCommand;
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::render_prices;

/// Appended to a failed sync when the embedded snapshot is what remains in use.
const SNAPSHOT_NOTE: &str = " (the embedded price snapshot is in use)";

pub fn run(env: &Env, io: &mut Io, command: &PricingCommand, net: &Network<'_>) -> Result<()> {
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    match command {
        PricingCommand::Sync => match sync_prices(net.prices, &store, &store, &SystemClock) {
            SyncOutcome::Synced { models } => {
                writeln!(io.out(), "Synced the prices of {models} models.")?;
                Ok(())
            }
            SyncOutcome::Failed {
                reason,
                seeded_fallback,
            } => {
                let note = if seeded_fallback { SNAPSHOT_NOTE } else { "" };
                bail!("could not refresh the prices: {reason}{note}")
            }
        },
        PricingCommand::Show { json } => {
            let table = load_table(&store, &SystemClock);
            if *json {
                let rows: Vec<_> = table
                    .entries()
                    .iter()
                    .map(|e| {
                        json!({
                            "model": e.model,
                            "input": e.price.input,
                            "output": e.price.output,
                            "cache_write": e.price.cache_write,
                            "cache_read": e.price.cache_read,
                            "source": e.source,
                            "updated_at": e.updated_at.0,
                        })
                    })
                    .collect();
                writeln!(io.out(), "{}", serde_json::to_string_pretty(&rows)?)?;
            } else {
                let shown = render_prices(Term::new(io.color()), table.entries());
                write!(io.out(), "{shown}")?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::Timestamp;
    use tokred_core::pricing::{ModelPrice, PriceEntry, PriceSource, SourceError};

    use super::super::testing::{FakeReleases, Sandbox};
    use super::*;
    use crate::io::testing::scripted;

    struct Source(Result<Vec<PriceEntry>, ()>);

    impl PriceSource for Source {
        fn fetch(&self) -> Result<Vec<PriceEntry>, SourceError> {
            self.0
                .clone()
                .map_err(|()| SourceError::Invalid("no network".to_owned()))
        }
    }

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        super::super::configure::run(&sb.env, &mut io, Some(sb.claude.clone()), true, false)
            .unwrap();
        sb
    }

    fn entry() -> PriceEntry {
        PriceEntry {
            model: "claude-opus-4-1-20250805".to_owned(),
            price: ModelPrice {
                input: 15.0,
                output: 75.0,
                cache_write: 18.75,
                cache_read: 1.5,
            },
            source: "fake".to_owned(),
            updated_at: Timestamp(0),
        }
    }

    fn go(sb: &Sandbox, command: &PricingCommand, source: &Source) -> (Result<()>, String) {
        let (mut io, out) = scripted(false, "");
        let net = Network {
            prices: source,
            releases: &FakeReleases {
                version: "0.0.0",
                calls: std::cell::Cell::new(0),
            },
        };
        let result = run(&sb.env, &mut io, command, &net);
        (result, out.text())
    }

    #[test]
    fn without_a_database_it_asks_for_configure() {
        let sb = Sandbox::new();
        let (result, _) = go(
            &sb,
            &PricingCommand::Show { json: false },
            &Source(Ok(vec![])),
        );
        assert!(result.unwrap_err().to_string().contains("tokred configure"));
    }

    #[test]
    fn sync_reports_how_many_models_were_refreshed() {
        let sb = configured();
        let (result, out) = go(&sb, &PricingCommand::Sync, &Source(Ok(vec![entry()])));
        result.unwrap();
        assert!(out.contains("Synced the prices of 1 models."));
    }

    #[test]
    fn a_failed_sync_is_an_error_that_mentions_the_fallback() {
        let sb = configured();
        let (result, _) = go(&sb, &PricingCommand::Sync, &Source(Err(())));
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("could not refresh the prices: "),
            "{message}"
        );
        assert!(message.contains("no network"));
    }

    #[test]
    fn show_prints_a_table_or_json() {
        let sb = configured();
        let (result, out) = go(
            &sb,
            &PricingCommand::Show { json: false },
            &Source(Ok(vec![])),
        );
        result.unwrap();
        assert!(out.contains("tokred Prices (USD per million tokens)"));
        assert!(
            out.contains("Cache write") && out.contains("claude-"),
            "{out}"
        );

        let (result, out) = go(
            &sb,
            &PricingCommand::Show { json: true },
            &Source(Ok(vec![])),
        );
        result.unwrap();
        let rows: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(rows.as_array().is_some_and(|rows| !rows.is_empty()));
        assert!(rows[0]["input"].is_number() && rows[0]["model"].is_string());
    }
}
