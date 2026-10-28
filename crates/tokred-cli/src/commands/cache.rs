//! `tokred cache`.

use anyhow::{Result, anyhow};
use serde_json::json;
use tokred_core::Clock;
use tokred_hooks::{cache_prune, cache_stats};

use super::{SystemClock, existing_store};
use crate::args::CacheCommand;
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::render_cache;

pub fn run(env: &Env, io: &mut Io, command: &CacheCommand) -> Result<()> {
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    let now = SystemClock.now();
    match command {
        CacheCommand::Stats { json } => {
            let stats = cache_stats(&store, now)?;
            if *json {
                let tiers: Vec<_> = stats
                    .tiers
                    .iter()
                    .map(|t| {
                        json!({
                            "tier": format!("{:?}", t.tier),
                            "entries": t.entries,
                            "expired": t.expired,
                            "hits": t.hits,
                            "bytes": t.bytes,
                        })
                    })
                    .collect();
                let text = serde_json::to_string_pretty(&json!({
                    "total_entries": stats.total_entries(),
                    "tiers": tiers,
                }))?;
                writeln!(io.out(), "{text}")?;
            } else {
                let shown = render_cache(Term::new(io.color()), &stats);
                write!(io.out(), "{shown}")?;
            }
        }
        CacheCommand::Prune => {
            let removed = cache_prune(&store, now)?;
            writeln!(io.out(), "Removed {removed} expired entries.")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokred_core::Timestamp;
    use tokred_core::cache::{CacheRepo, TierPolicy, cache_put};

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn go(sb: &Sandbox, command: &CacheCommand) -> (Result<()>, String) {
        let (mut io, out) = scripted(false, "");
        let result = run(&sb.env, &mut io, command);
        (result, out.text())
    }

    #[test]
    fn it_needs_a_database() {
        let sb = Sandbox::new();
        let (result, _) = go(&sb, &CacheCommand::Prune);
        assert!(result.unwrap_err().to_string().contains("tokred configure"));
    }

    #[test]
    fn an_empty_cache_says_so() {
        let sb = Sandbox::new();
        sb.configure();
        let (result, out) = go(&sb, &CacheCommand::Stats { json: false });
        result.unwrap();
        assert!(out.contains("The cache is empty."));
        let (_, json) = go(&sb, &CacheCommand::Stats { json: true });
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["total_entries"], 0);
        assert_eq!(value["tiers"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn stats_and_prune_work_on_real_entries() {
        let sb = Sandbox::new();
        sb.configure();
        let store = sb.store();
        let policy = TierPolicy::default();
        let now = Timestamp(tokred_core::Clock::now(&SystemClock).0);
        cache_put(&store, "fresh", "test", vec![0; 2_048], now, &policy).unwrap();
        // Long expired: written far in the past.
        cache_put(
            &store,
            "stale",
            "test",
            vec![1; 10],
            Timestamp(now.0 - 10 * 86_400),
            &policy,
        )
        .unwrap();
        assert_eq!(store.list_entries().unwrap().len(), 2);

        let (_, out) = go(&sb, &CacheCommand::Stats { json: false });
        for expected in [
            "tokred Cache",
            "Tier",
            "L1",
            "Entries",
            "Expired",
            "Impact",
            "tokred cache prune",
        ] {
            assert!(out.contains(expected), "missing {expected:?} in:\n{out}");
        }
        let (result, out) = go(&sb, &CacheCommand::Prune);
        result.unwrap();
        assert!(out.contains("Removed 1 expired entries."));
        let (_, out) = go(&sb, &CacheCommand::Prune);
        assert!(out.contains("Removed 0 expired entries."));
    }
}
