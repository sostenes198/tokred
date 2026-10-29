//! `tokred routing`, `tokred suggest`, `tokred statusline` and the internal `tokred mcp`.
//!
//! These read what the proxy stored; the main loop's model is never changed by any of them.

use std::fmt::Write as _;
use std::io::Read;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;
use tokred_core::config::TrafficMode;
use tokred_core::prompts::{PromptRepo, SuggestionRepo};
use tokred_core::routing::RoutingRepo;
use tokred_core::savings::{PERIOD_DAYS, SavingsFigures, SavingsRepo};
use tokred_core::{Clock, Timestamp};
use tokred_metrics::range_from_span;
use tokred_routing::{explain, summarize_decisions, summarize_period, verdict};
use tokred_suggest::{SessionFigures, StatuslineInput, statusline_text};

use super::data::span_label;
use super::{SystemClock, existing_store, housekeeping};
use crate::args::{CompressionCommand, RoutingCommand};
use crate::env::Env;
use crate::io::Io;
use crate::term::{Align, Column, Term, key_values, table, title, truncate};
use tokred_report::format::{integer, usd};

/// Decisions shown by `routing status`.
const RECENT_DECISIONS: usize = 15;

/// Sets `[<section>] <key> = <value>` in `tokred.toml` text, leaving other keys as they are.
/// When the key already has that value the text is returned untouched (comments survive);
/// otherwise the file is rewritten by the TOML serializer, so comments are not kept.
pub fn set_key(existing: &str, section: &str, key: &str, value: toml::Value) -> Result<String> {
    let mut table: toml::Table =
        toml::from_str(existing).context("tokred.toml is not valid TOML")?;
    let entry = table
        .entry(section)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let section_table = entry
        .as_table_mut()
        .ok_or_else(|| anyhow!("`[{section}]` in tokred.toml must be a table"))?;
    if section_table.get(key) == Some(&value) {
        return Ok(existing.to_owned());
    }
    section_table.insert(key.to_owned(), value);
    Ok(toml::to_string_pretty(&table)?)
}

/// Sets `[<section>] mode = "<value>"` in `tokred.toml` text.
pub fn set_mode(existing: &str, section: &str, value: &str) -> Result<String> {
    set_key(
        existing,
        section,
        "mode",
        toml::Value::String(value.to_owned()),
    )
}

fn mode_name(mode: TrafficMode) -> &'static str {
    match mode {
        TrafficMode::Off => "off",
        TrafficMode::Observe => "observe",
        TrafficMode::Enforce => "enforce",
    }
}

/// What the last [`PERIOD_DAYS`] days say about turning routing and compression on; `None` without
/// a database or when it cannot be read.
pub(super) fn recent_savings(env: &Env) -> Option<SavingsFigures> {
    let store = existing_store(env).ok().flatten()?;
    let now = SystemClock.now().0;
    // The range is open at its end: one second more counts what happened in this very second.
    store
        .savings_figures(Timestamp(now - PERIOD_DAYS * 86_400), Timestamp(now + 1))
        .ok()
}

pub fn routing(env: &Env, io: &mut Io, command: &RoutingCommand) -> Result<()> {
    match command {
        RoutingCommand::On | RoutingCommand::Off => {
            routing_set_mode(env, io, matches!(command, RoutingCommand::On))
        }
        RoutingCommand::Status { since, json } => routing_status(env, io, since, *json),
        RoutingCommand::Explain { id } => routing_explain(env, io, id),
    }
}

/// `routing on` / `routing off`: writes `[routing] mode` to `tokred.toml`.
fn routing_set_mode(env: &Env, io: &mut Io, on: bool) -> Result<()> {
    let (mode, note) = if on {
        (
            "enforce",
            "Routing now changes the model of eligible subagent and auxiliary requests. The main loop is never changed.",
        )
    } else {
        (
            "observe",
            "Routing keeps recording what it would do and changes nothing.",
        )
    };
    let path = env.data.config_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, set_mode(&existing, "routing", mode)?)
        .with_context(|| format!("could not write {}", path.display()))?;
    write!(io.out(), "[routing] mode = \"{mode}\"\n{note}\n")?;
    Ok(())
}

/// `routing status`: the mode, what routing did over `since`, and the recent decisions.
fn routing_status(env: &Env, io: &mut Io, since: &str, json: bool) -> Result<()> {
    let config = housekeeping::load_config(env);
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    let now = SystemClock.now();
    let range =
        range_from_span(now, since).map_err(|err| anyhow!("invalid --since `{since}`: {err}"))?;
    let label = format!("last {}", span_label(since));
    let days = f64::from(i32::try_from(range.to.0 - range.from.0).unwrap_or(i32::MAX)) / 86_400.0;
    let period = store.decisions_between(range.from, range.to)?;
    let summary = summarize_period(&period);
    let mode = mode_name(config.routing.mode);
    let verdict = verdict(&summary, mode, &label, days);
    let recent = store.recent_decisions(RECENT_DECISIONS)?;
    if json {
        let by_task: Vec<_> = summary
                    .by_task
                    .iter()
                    .map(|(task, requests, saving)| {
                        json!({ "task": task, "requests": requests, "saving_usd": saving })
                    })
                    .collect();
        let value = json!({
            "mode": config.routing.mode,
            "period": span_label(since),
            "decisions": summary.decisions,
            "applied": summary.applied,
            "observed": summary.observed,
            "kept": summary.kept,
            "fallback": summary.fallback,
            "saving_usd": summary.saving_usd,
            "by_task": by_task,
            "verdict": verdict,
            "recent": recent,
        });
        writeln!(io.out(), "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(());
    }
    let term = Term::new(io.color());
    let mut out = title(term, "tokred Model Routing");
    out.push('\n');
    out.push_str(&key_values(&[
        ("Mode", mode.to_owned()),
        ("Main loop", term.heat(1.0, "never changed")),
        ("Summary", summarize_decisions(&period)),
    ]));
    let _ = write!(out, "\n{verdict}\n");
    if recent.is_empty() {
        out.push_str(
            "\nNo routing decisions recorded yet. Use `tokred claude` and run some subagents.\n",
        );
    } else {
        let rows: Vec<Vec<String>> = recent
            .iter()
            .map(|d| {
                vec![
                    truncate(&d.id, 12),
                    d.task_class.clone(),
                    format!(
                        "{} -> {}",
                        truncate(&d.model_from, 22),
                        truncate(&d.model_to, 22)
                    ),
                    d.outcome.clone(),
                    usd(d.est_cost_from - d.est_cost_to),
                ]
            })
            .collect();
        let columns = [
            Column {
                title: "Id",
                align: Align::Left,
            },
            Column {
                title: "Task",
                align: Align::Left,
            },
            Column {
                title: "Model",
                align: Align::Left,
            },
            Column {
                title: "Outcome",
                align: Align::Left,
            },
            Column {
                title: "Est. saving",
                align: Align::Right,
            },
        ];
        out.push('\n');
        out.push_str(&table(term, &columns, &rows));
    }
    write!(io.out(), "{out}")?;
    Ok(())
}

/// `routing explain`: why one decision was taken.
fn routing_explain(env: &Env, io: &mut Io, id: &str) -> Result<()> {
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    let Some(decision) = store.decision(id)? else {
        bail!("no routing decision with id `{id}`");
    };
    write!(io.out(), "{}", explain(&decision))?;
    Ok(())
}

pub fn suggest(env: &Env, io: &mut Io, since: &str, json: bool) -> Result<()> {
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    let now = SystemClock.now();
    let range =
        range_from_span(now, since).map_err(|err| anyhow!("invalid --since `{since}`: {err}"))?;
    let suggestions = store.suggestions_between(range.from, range.to)?;
    let prompts = store.prompts_between(range.from, range.to)?;
    let simple = prompts.iter().filter(|p| p.simple).count();
    if json {
        let value = json!({ "period": span_label(since), "prompts": prompts.len(), "simple": simple, "suggestions": suggestions });
        writeln!(io.out(), "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(());
    }
    let term = Term::new(io.color());
    let mut out = title(
        term,
        &format!("tokred Model Suggestions ({})", span_label(since)),
    );
    out.push('\n');
    out.push_str(&key_values(&[
        ("Prompts tracked", integer(prompts.len() as u64)),
        ("Simple prompts", integer(simple as u64)),
        ("Suggestions", integer(suggestions.len() as u64)),
    ]));
    out.push_str(
        &term
            .dim("tokred never changes your main model; switching is always your call (/model).\n"),
    );
    if suggestions.is_empty() {
        out.push_str("\nNo suggestions: either the work was not simple enough, or the proxy has not seen enough prompts (use `tokred claude`).\n");
    } else {
        out.push('\n');
        for suggestion in suggestions.iter().rev().take(10) {
            let _ = writeln!(out, "  {} {}", term.cyan("▶"), suggestion.message);
        }
    }
    write!(io.out(), "{out}")?;
    Ok(())
}

pub fn statusline(env: &Env, io: &mut Io) -> Result<()> {
    let mut payload = Vec::new();
    Read::take(io.streams().0, 64 * 1024).read_to_end(&mut payload)?;
    let input = StatuslineInput::parse(&payload);
    let (suggestion, prompts) = match (existing_store(env), input.session_id.as_deref()) {
        (Ok(Some(store)), Some(session)) => (
            store.latest_unshown_suggestion(session).ok().flatten(),
            store
                .session_prompts(session, Timestamp(0), Timestamp(i64::MAX / 2))
                .unwrap_or_default(),
        ),
        _ => (None, Vec::new()),
    };
    let figures = SessionFigures::new(&prompts, &input);
    let line = statusline_text(suggestion.as_ref(), &figures, io.color());
    writeln!(io.out(), "{line}")?;
    Ok(())
}

/// The internal MCP server: `tokred_retrieve` over stdio, backed by the stored originals.
pub fn mcp(env: &Env, io: &mut Io, version: &str) -> Result<()> {
    let store = existing_store(env)?;
    let lookup = |id: &str| {
        store
            .as_ref()
            .and_then(|store| store.stored_result(id, SystemClock.now()).ok().flatten())
    };
    let (stdin, stdout) = io.streams();
    tokred_compress::serve(stdin, stdout, &lookup, &|| SystemClock.now(), version)?;
    Ok(())
}

/// Meta key: when the compression circuit breaker tripped (unix seconds). Cleared by
/// `tokred compression on`. While it is set the proxy starts with compression in `observe`.
pub const COMPRESSION_TRIPPED_AT: &str = "compression_tripped_at";

/// The Claude Code user config to register the MCP server in: the first existing candidate,
/// else `<home>/.claude.json`.
fn mcp_config_path(env: &Env, claude_dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let candidates = tokred_audit::global_config_candidates(claude_dir, env.home_dir.as_deref());
    candidates
        .iter()
        .find(|path| path.exists())
        .or_else(|| candidates.last())
        .cloned()
        .context("could not determine where Claude Code keeps its user configuration")
}

/// Whether the `tokred_retrieve` MCP server is registered in Claude Code's user config.
pub fn compression_registered(env: &Env, claude_dir: &std::path::Path) -> bool {
    mcp_config_path(env, claude_dir).is_ok_and(|path| super::serve::mcp_registered(&[path]))
}

/// Writes `value` to `path` after saving the previous content next to it (`.tokred.bak`).
fn write_json_with_backup(path: &std::path::Path, value: &serde_json::Value) -> Result<()> {
    use tokred_core::FileSystem;
    let fs = tokred_install::StdFileSystem;
    if let Some(previous) = fs.read_to_string(path)? {
        let mut backup = path.as_os_str().to_owned();
        backup.push(".tokred.bak");
        let backup = std::path::PathBuf::from(backup);
        fs.write_atomic(&backup, previous.as_bytes())?;
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fs.write_atomic(path, text.as_bytes())?;
    Ok(())
}

fn read_json_or_empty(path: &std::path::Path) -> Result<serde_json::Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON; nothing was changed", path.display())),
        Err(_) => Ok(json!({})),
    }
}

pub fn compression(
    env: &Env,
    io: &mut Io,
    command: &CompressionCommand,
    claude_dir: Option<std::path::PathBuf>,
    binary: &str,
) -> Result<()> {
    use tokred_core::meta::MetaRepo;
    let config = housekeeping::load_config(env);
    match command {
        CompressionCommand::Status => {
            let registered = claude_dir
                .as_deref()
                .is_some_and(|dir| compression_registered(env, dir));
            let tripped = existing_store(env)?
                .and_then(|store| store.meta_get(COMPRESSION_TRIPPED_AT).ok().flatten())
                // `compression on` clears the mark by storing an empty value.
                .filter(|at| !at.is_empty());
            let mode = mode_name(config.compression.mode);
            let retrieve = if registered {
                "registered in Claude Code"
            } else {
                "not registered"
            };
            let breaker = tripped.map_or_else(
                || "not tripped".to_owned(),
                |at| format!("tripped at {at}; the proxy starts in observe mode until `tokred compression on`"),
            );
            // Only while it is measuring: once it is on, the question is answered.
            let potential = recent_savings(env)
                .filter(|_| config.compression.mode == TrafficMode::Observe)
                .map_or_else(String::new, |figures| {
                    format!("potential:          {}\n", figures.compression_line())
                });
            let text = format!(
                "compression mode:   {mode}\ntokred_retrieve:    {retrieve}\ncircuit breaker:    {breaker}\n{potential}"
            );
            write!(io.out(), "{text}")?;
            Ok(())
        }
        CompressionCommand::On { .. } => {
            let dir =
                claude_dir.context("could not find the Claude Code folder; pass --claude-dir")?;
            let path = mcp_config_path(env, &dir)?;
            let updated = tokred_install::mcp::register(read_json_or_empty(&path)?, binary)
                .map_err(|err| {
                    anyhow!(
                        "{} cannot take the server: {err}; nothing was changed",
                        path.display()
                    )
                })?;
            write_json_with_backup(&path, &updated)?;
            let config_path = env.data.config_path();
            let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
            std::fs::write(&config_path, set_mode(&existing, "compression", "enforce")?)?;
            existing_store(env)?
                .map(|store| store.meta_set(COMPRESSION_TRIPPED_AT, ""))
                .transpose()?;
            let text = format!(
                "Registered the `tokred_retrieve` MCP server in {} (backup: .tokred.bak).\n\
                 [compression] mode = \"enforce\": large tool results are replaced by a preview; nothing is lost, the model reads the rest back with tokred_retrieve.\n\
                 Restart the proxy and Claude Code to apply it.\n",
                path.display()
            );
            write!(io.out(), "{text}")?;
            Ok(())
        }
        CompressionCommand::Off { .. } => {
            if let Some(dir) = claude_dir
                && let Ok(path) = mcp_config_path(env, &dir)
                && path.exists()
            {
                let updated = tokred_install::mcp::unregister(read_json_or_empty(&path)?);
                write_json_with_backup(&path, &updated)?;
            }
            let config_path = env.data.config_path();
            let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
            std::fs::write(&config_path, set_mode(&existing, "compression", "observe")?)?;
            let text = "[compression] mode = \"observe\": nothing is replaced any more; the MCP server is unregistered.\n";
            write!(io.out(), "{text}")?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::meta::MetaRepo;
    use tokred_core::prompts::{PromptSummary, SuggestionRecord};
    use tokred_core::routing::RoutingDecision;

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn now() -> Timestamp {
        SystemClock.now()
    }

    fn decision(id: &str, outcome: &str) -> RoutingDecision {
        RoutingDecision {
            id: id.to_owned(),
            ts: now(),
            session_id: "s1".to_owned(),
            agent_id: Some("a1".to_owned()),
            scope: "agent".to_owned(),
            task_class: "search".to_owned(),
            confidence: 0.9,
            layer: "keywords".to_owned(),
            reason: "read-only tools".to_owned(),
            model_from: "claude-sonnet-5-5".to_owned(),
            model_to: "claude-haiku-5-5".to_owned(),
            mode: "observe".to_owned(),
            est_cost_from: 0.02,
            est_cost_to: 0.005,
            rules_version: "1".to_owned(),
            outcome: outcome.to_owned(),
            detail: "{}".to_owned(),
        }
    }

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        sb.configure();
        sb
    }

    #[test]
    fn set_key_leaves_the_text_alone_when_the_value_is_already_there() {
        let text = "# mine\n[proxy]\nlog_bodies = true\n";
        let same = set_key(text, "proxy", "log_bodies", toml::Value::Boolean(true)).unwrap();
        assert_eq!(same, text);
        let changed = set_key(text, "proxy", "log_bodies", toml::Value::Boolean(false)).unwrap();
        assert!(changed.contains("log_bodies = false"), "{changed}");
        let added = set_key("", "proxy", "bind", toml::Value::String("h:1".to_owned())).unwrap();
        assert!(added.contains("bind = \"h:1\""), "{added}");
    }

    #[test]
    fn set_mode_changes_one_key_and_rejects_bad_files() {
        let text = set_mode(
            "[routing]\nmin_confidence = 0.9\n\n[cache]\nl2_hits = 7\n",
            "routing",
            "enforce",
        )
        .unwrap();
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(parsed["routing"]["mode"].as_str(), Some("enforce"));
        assert_eq!(parsed["routing"]["min_confidence"].as_float(), Some(0.9));
        assert_eq!(parsed["cache"]["l2_hits"].as_integer(), Some(7));
        assert!(
            set_mode("", "compression", "observe")
                .unwrap()
                .contains("observe")
        );
        assert!(
            set_mode("routing = 3", "routing", "x")
                .unwrap_err()
                .to_string()
                .contains("must be a table")
        );
        assert!(
            set_mode("[[[", "routing", "x")
                .unwrap_err()
                .to_string()
                .contains("not valid TOML")
        );
    }

    #[test]
    fn routing_on_and_off_edit_the_config() {
        let sb = configured();
        let (mut io, out) = scripted(false, "");
        routing(&sb.env, &mut io, &RoutingCommand::On).unwrap();
        assert!(
            out.text().contains("enforce") && out.text().contains("The main loop is never changed")
        );
        assert_eq!(
            housekeeping::load_config(&sb.env).routing.mode,
            TrafficMode::Enforce
        );
        let (mut io, out) = scripted(false, "");
        routing(&sb.env, &mut io, &RoutingCommand::Off).unwrap();
        assert!(out.text().contains("observe"));
        assert_eq!(
            housekeeping::load_config(&sb.env).routing.mode,
            TrafficMode::Observe
        );
        assert_eq!(mode_name(TrafficMode::Off), "off");
    }

    #[test]
    fn routing_status_and_explain_read_the_stored_decisions() {
        let sb = configured();
        let (mut io, out) = scripted(false, "");
        routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Status {
                since: "7d".to_owned(),
                json: false,
            },
        )
        .unwrap();
        assert!(out.text().contains("No routing decisions recorded yet"));

        let store = sb.store();
        store
            .insert_decision(&decision("rd-1", "observed"))
            .unwrap();
        store.insert_decision(&decision("rd-2", "kept")).unwrap();
        let (mut io, out) = scripted(false, "");
        routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Status {
                since: "7d".to_owned(),
                json: false,
            },
        )
        .unwrap();
        for expected in [
            "tokred Model Routing",
            "Mode:",
            "observe",
            "never changed",
            "rd-1",
            "claude-haiku-5-5",
            "Outcome",
        ] {
            let text = out.text();
            assert!(text.contains(expected), "missing {expected:?}:\n{text}");
        }
        let (mut io, out) = scripted(false, "");
        routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Status {
                since: "7d".to_owned(),
                json: true,
            },
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&out.text()).unwrap();
        assert_eq!(value["mode"], "observe");
        assert_eq!(value["recent"].as_array().unwrap().len(), 2);
        assert_eq!(
            (value["decisions"].as_u64(), value["observed"].as_u64()),
            (Some(2), Some(1))
        );
        assert!((value["saving_usd"].as_f64().unwrap() - 0.015).abs() < 1e-9);
        assert_eq!(value["by_task"][0]["task"], "search");
        let verdict = value["verdict"].as_str().unwrap();
        assert!(
            verdict.contains("would have changed 1 of 2 requests"),
            "{verdict}"
        );
    }

    #[test]
    fn routing_status_says_it_in_text_and_refuses_a_period_that_is_not_a_span() {
        let sb = configured();
        let store = sb.store();
        store
            .insert_decision(&decision("rd-1", "observed"))
            .unwrap();
        store.insert_decision(&decision("rd-2", "kept")).unwrap();
        // The text says it too, and a period that is not a span is refused.
        let (mut io, out) = scripted(false, "");
        routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Status {
                since: "7d".to_owned(),
                json: false,
            },
        )
        .unwrap();
        let text = out.text();
        assert!(
            text.contains("Turning routing on would have changed"),
            "{text}"
        );
        let (mut io, _) = scripted(false, "");
        let err = routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Status {
                since: "soon".to_owned(),
                json: false,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid --since"), "{err}");

        let (mut io, out) = scripted(false, "");
        routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Explain {
                id: "rd-1".to_owned(),
            },
        )
        .unwrap();
        assert!(out.text().contains("decision rd-1"), "{}", out.text());
        let (mut io, _) = scripted(false, "");
        let err = routing(
            &sb.env,
            &mut io,
            &RoutingCommand::Explain {
                id: "nope".to_owned(),
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("no routing decision"));
    }

    #[test]
    fn routing_needs_a_database_for_status_and_explain() {
        let sb = Sandbox::new();
        for command in [
            RoutingCommand::Status {
                since: "7d".to_owned(),
                json: false,
            },
            RoutingCommand::Explain { id: "x".to_owned() },
        ] {
            let (mut io, _) = scripted(false, "");
            assert!(
                routing(&sb.env, &mut io, &command)
                    .unwrap_err()
                    .to_string()
                    .contains("tokred configure")
            );
        }
        let (mut io, _) = scripted(false, "");
        assert!(
            suggest(&sb.env, &mut io, "7d", false)
                .unwrap_err()
                .to_string()
                .contains("tokred configure")
        );
    }

    #[test]
    fn suggest_lists_suggestions_and_counts_prompts() {
        let sb = configured();
        let (mut io, out) = scripted(false, "");
        suggest(&sb.env, &mut io, "7d", false).unwrap();
        assert!(
            out.text().contains("No suggestions")
                && out.text().contains("never changes your main model")
        );

        let store = sb.store();
        store
            .upsert_prompt(&PromptSummary {
                prompt_id: "p1".into(),
                session_id: "s1".into(),
                ts: now(),
                simple: true,
                ..PromptSummary::default()
            })
            .unwrap();
        store
            .upsert_prompt(&PromptSummary {
                prompt_id: "p2".into(),
                session_id: "s1".into(),
                ts: now(),
                simple: false,
                ..PromptSummary::default()
            })
            .unwrap();
        store
            .insert_suggestion(&SuggestionRecord {
                id: "g1".into(),
                session_id: "s1".into(),
                ts: now(),
                kind: "cheaper_model".into(),
                message: "tokred: use sonnet".into(),
                ..SuggestionRecord::default()
            })
            .unwrap();
        let (mut io, out) = scripted(false, "");
        suggest(&sb.env, &mut io, "7d", false).unwrap();
        let text = out.text();
        assert!(
            text.contains("Prompts tracked:")
                && text.contains("Simple prompts:")
                && text.contains("tokred: use sonnet"),
            "{text}"
        );

        let (mut io, out) = scripted(false, "");
        suggest(&sb.env, &mut io, "7d", true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out.text()).unwrap();
        assert_eq!(
            (value["prompts"].as_u64(), value["simple"].as_u64()),
            (Some(2), Some(1))
        );
        let (mut io, _) = scripted(false, "");
        assert!(
            suggest(&sb.env, &mut io, "soon", false)
                .unwrap_err()
                .to_string()
                .contains("invalid --since")
        );
    }

    #[test]
    fn the_status_line_shows_a_pending_suggestion_or_a_neutral_line() {
        let sb = configured();
        let payload = br#"{"session_id":"s1","model":{"id":"claude-opus-5-5"},"cost":{"total_cost_usd":1.5}}"#;
        let (mut io, out) = scripted(false, std::str::from_utf8(payload).unwrap());
        statusline(&sb.env, &mut io).unwrap();
        let neutral = out.text();
        assert_eq!(neutral.lines().count(), 1, "{neutral}");

        sb.store()
            .insert_suggestion(&SuggestionRecord {
                id: "g1".into(),
                session_id: "s1".into(),
                ts: now(),
                kind: "cheaper_model".into(),
                message: "tokred: sonnet would have cost less".into(),
                ..SuggestionRecord::default()
            })
            .unwrap();
        let (mut io, out) = scripted(false, std::str::from_utf8(payload).unwrap());
        statusline(&sb.env, &mut io).unwrap();
        assert!(out.text().contains("sonnet"), "{}", out.text());

        // Garbage input, and no database at all, still print one line.
        let bare = Sandbox::new();
        let (mut io, out) = scripted(false, "not json");
        statusline(&bare.env, &mut io).unwrap();
        assert_eq!(out.text().lines().count(), 1);
    }

    #[test]
    fn the_mcp_server_serves_stored_originals() {
        use tokred_core::compression::StoredResult;
        let sb = configured();
        let created = now();
        sb.store()
            .put_stored_result(&StoredResult {
                id: "tr-1".into(),
                session_id: "s1".into(),
                tool: "Bash".into(),
                content: "line one\nline two\nline three\n".into(),
                lines: 3,
                bytes: 29,
                created_at: created,
                expires_at: Timestamp(created.0 + 3600),
                ..StoredResult::default()
            })
            .unwrap();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tokred_retrieve","arguments":{"id":"tr-1","offset":1,"limit":1}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tokred_retrieve","arguments":{"id":"missing"}}}"#,
            "\n",
        );
        let (mut io, out) = scripted(false, input);
        mcp(&sb.env, &mut io, "9.9.9").unwrap();
        let text = out.text();
        assert!(text.contains("9.9.9"), "server version reported");
        assert!(text.contains("line two"), "{text}");
        assert!(!text.contains("line three"), "only the requested range");
        assert!(text.contains(r#""isError":true"#), "unknown id: {text}");

        // Without a database every lookup fails cleanly.
        let bare = Sandbox::new();
        let (mut io, out) = scripted(
            false,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tokred_retrieve","arguments":{"id":"x"}}}"#,
        );
        mcp(&bare.env, &mut io, "1").unwrap();
        assert!(out.text().contains("isError"));
    }

    #[test]
    fn compression_on_registers_the_server_backs_up_and_off_restores() {
        let sb = configured();
        let claude_json = sb.env.home_dir.clone().unwrap().join(".claude.json");
        std::fs::write(
            &claude_json,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#,
        )
        .unwrap();
        let on = CompressionCommand::On {
            dir: crate::args::ClaudeDirArg { claude_dir: None },
        };
        let (mut io, out) = scripted(false, "");
        compression(
            &sb.env,
            &mut io,
            &on,
            Some(sb.claude.clone()),
            "/opt/bin/tokred",
        )
        .unwrap();
        let text = out.text();
        assert!(
            text.contains("Registered the `tokred_retrieve` MCP server"),
            "{text}"
        );

        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert!(tokred_install::mcp::is_registered(&after));
        assert_eq!(after["theme"], "dark");
        assert!(
            after["mcpServers"]["other"].is_object(),
            "other servers untouched"
        );
        let backup = std::fs::read_to_string(
            sb.env
                .home_dir
                .clone()
                .unwrap()
                .join(".claude.json.tokred.bak"),
        )
        .unwrap();
        assert!(
            !backup.contains("tokred"),
            "the backup is the previous content"
        );
        assert_eq!(
            housekeeping::load_config(&sb.env).compression.mode,
            TrafficMode::Enforce
        );

        let (mut io, out) = scripted(false, "");
        compression(
            &sb.env,
            &mut io,
            &CompressionCommand::Status,
            Some(sb.claude.clone()),
            "x",
        )
        .unwrap();
        let text = out.text();
        assert!(
            text.contains("compression mode:   enforce")
                && text.contains("registered in Claude Code"),
            "{text}"
        );
        assert!(text.contains("not tripped"));

        let off = CompressionCommand::Off {
            dir: crate::args::ClaudeDirArg { claude_dir: None },
        };
        let (mut io, out) = scripted(false, "");
        compression(&sb.env, &mut io, &off, Some(sb.claude.clone()), "x").unwrap();
        assert!(out.text().contains("observe"));
        let restored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&claude_json).unwrap()).unwrap();
        assert!(!tokred_install::mcp::is_registered(&restored));
        assert!(restored["mcpServers"]["other"].is_object());
        assert_eq!(
            housekeeping::load_config(&sb.env).compression.mode,
            TrafficMode::Observe
        );
    }

    #[test]
    fn compression_creates_the_config_when_missing_and_refuses_broken_ones() {
        let sb = configured();
        let claude_json = sb.env.home_dir.clone().unwrap().join(".claude.json");
        let on = CompressionCommand::On {
            dir: crate::args::ClaudeDirArg { claude_dir: None },
        };
        let (mut io, _) = scripted(false, "");
        compression(&sb.env, &mut io, &on, Some(sb.claude.clone()), "/b/tokred").unwrap();
        assert!(claude_json.exists(), "created when missing");
        assert!(
            !sb.env
                .home_dir
                .clone()
                .unwrap()
                .join(".claude.json.tokred.bak")
                .exists(),
            "nothing to back up"
        );

        std::fs::write(&claude_json, "{ broken").unwrap();
        let (mut io, _) = scripted(false, "");
        let err =
            compression(&sb.env, &mut io, &on, Some(sb.claude.clone()), "/b/tokred").unwrap_err();
        assert!(err.to_string().contains("not valid JSON"), "{err}");
        assert_eq!(std::fs::read_to_string(&claude_json).unwrap(), "{ broken");

        std::fs::write(&claude_json, r#"{"mcpServers": 3}"#).unwrap();
        let (mut io, _) = scripted(false, "");
        let err =
            compression(&sb.env, &mut io, &on, Some(sb.claude.clone()), "/b/tokred").unwrap_err();
        assert!(err.to_string().contains("cannot take the server"), "{err}");

        let (mut io, _) = scripted(false, "");
        let err = compression(&sb.env, &mut io, &on, None, "/b/tokred").unwrap_err();
        assert!(err.to_string().contains("--claude-dir"));
        // Off with no folder or no file just resets the mode.
        let off = CompressionCommand::Off {
            dir: crate::args::ClaudeDirArg { claude_dir: None },
        };
        let (mut io, _) = scripted(false, "");
        compression(&sb.env, &mut io, &off, None, "x").unwrap();
        // Status mentions a tripped breaker.
        sb.store()
            .meta_set(COMPRESSION_TRIPPED_AT, "1800000000")
            .unwrap();
        let (mut io, out) = scripted(false, "");
        compression(&sb.env, &mut io, &CompressionCommand::Status, None, "x").unwrap();
        let text = out.text();
        assert!(text.contains("tripped at 1800000000"), "{text}");
    }

    #[test]
    fn compression_status_says_what_turning_it_on_would_save_only_while_measuring() {
        let sb = configured();
        let (mut io, out) = scripted(false, "");
        compression(&sb.env, &mut io, &CompressionCommand::Status, None, "x").unwrap();
        let text = out.text();
        assert!(text.contains("compression mode:   observe"), "{text}");
        assert!(
            text.contains("potential:          nothing measured"),
            "{text}"
        );

        // With real measurements: a small share is "not worth it yet".
        let store = sb.store();
        store
            .insert_shadow_records(&[tokred_core::compression::ShadowRecord {
                ts: now(),
                session_id: "s".to_owned(),
                kind: "compression".to_owned(),
                key: "Read".to_owned(),
                bytes_saved: 4_000,
                est_tokens_saved: 1_000,
                rules_version: "r".to_owned(),
            }])
            .unwrap();
        store
            .insert_proxy_request(&tokred_core::proxy::ProxyRequestRecord {
                id: "r1".to_owned(),
                ts: now(),
                usage: tokred_core::pricing::Usage {
                    input: 1_000_000,
                    ..tokred_core::pricing::Usage::default()
                },
                ..tokred_core::proxy::ProxyRequestRecord::default()
            })
            .unwrap();
        let (mut io, out) = scripted(false, "");
        compression(&sb.env, &mut io, &CompressionCommand::Status, None, "x").unwrap();
        let text = out.text();
        assert!(text.contains("not worth turning on yet"), "{text}");

        // Turned on, the question is answered and the line goes away.
        std::fs::write(
            sb.env.data.config_path(),
            "[compression]\nmode = \"enforce\"\n",
        )
        .unwrap();
        let (mut io, out) = scripted(false, "");
        compression(&sb.env, &mut io, &CompressionCommand::Status, None, "x").unwrap();
        assert!(!out.text().contains("potential:"), "{}", out.text());
    }
}
