//! The terminal dashboards of `tokred stats`, `tokred stats latency` and `tokred metrics`.
//!
//! Pure functions from metrics data to text, in the style of `rtk gain`: a titled block of
//! totals, an efficiency meter and numbered tables with an "Impact" bar column.

use std::fmt::Write as _;

use tokred_audit::AuditReport;
use tokred_audit::report::{ItemKind, McpScope, Scope};
use tokred_core::cache::CacheStats;
use tokred_core::pricing::PriceEntry;
use tokred_metrics::stats::{BeforeAfter, Group, UnpricedReason};
use tokred_metrics::{GroupBy, LatencyStats, Stats, Tradeoff};
use tokred_report::format::{compact, integer, ms, percent, usd};

use crate::args::LatencyBy;
use crate::commands::BenchReport;
use crate::term::{Align, Column, Term, key_values, table, title, truncate};

/// Rows shown per table before the rest is summarized.
const MAX_ROWS: usize = 15;
/// Longest name shown in a table cell.
const MAX_NAME: usize = 38;
/// Cells in the "Impact" column.
const IMPACT_WIDTH: usize = 10;
/// Cells in the efficiency meter.
const METER_WIDTH: usize = 28;

fn fraction(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0 && part.is_finite()).then(|| (part / whole).clamp(0.0, 1.0))
}

fn money_or_na(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), usd)
}

/// The cost dashboard.
#[must_use]
pub fn render_stats(term: Term, stats: &Stats, label: &str) -> String {
    let totals = &stats.totals;
    let savings = &stats.savings;
    let mut out = title(term, &format!("tokred Token Savings ({label})"));
    out.push('\n');

    if totals.messages == 0 {
        out.push_str(&term.yellow("No Claude Code usage found for this period.\n"));
        out.push_str("Use Claude Code for a while, then run this again.\n");
        return out;
    }

    let guard = &savings.guard;
    let routing = savings.routing_saving_usd;
    let share = |saving: Option<f64>| match (saving, savings.opus_cost_usd) {
        (Some(saving), Some(opus)) if opus > 0.0 => format!(" ({})", percent(saving / opus)),
        _ => String::new(),
    };
    let mut rows: Vec<(&str, String)> = vec![
        (
            "Messages",
            format!(
                "{} ({} sessions)",
                integer(totals.messages),
                integer(totals.sessions)
            ),
        ),
        ("Input tokens", compact(totals.tokens.input)),
        ("Output tokens", compact(totals.tokens.output)),
        (
            "Cache read/write",
            format!(
                "{} / {}",
                compact(totals.tokens.cache_read),
                compact(totals.tokens.cache_write)
            ),
        ),
        ("Real cost", usd(savings.real_cost_usd)),
        ("If all on opus", money_or_na(savings.opus_cost_usd)),
        (
            "Routing saving",
            format!("{}{}", money_or_na(routing), share(routing)),
        ),
        (
            "Guard saving",
            format!(
                "~{} {}",
                usd(guard.saving_usd),
                term.dim(&format!(
                    "(estimate: {} blocks, ~{} tokens avoided)",
                    integer(guard.blocks),
                    compact(guard.tokens_avoided)
                ))
            ),
        ),
        (
            "Total saved",
            term.bold(&money_or_na(savings.total_saving_usd)),
        ),
        (
            "Prompt cache hit",
            stats
                .cache_rate
                .overall
                .map_or_else(|| "n/a".to_owned(), percent),
        ),
    ];
    if let Some(projection) = &stats.projection {
        rows.push((
            "Projection / month",
            format!(
                "{} at the 7-day average, {} at the 30-day average",
                usd(projection.last_7d.projected_30d_usd),
                usd(projection.last_30d.projected_30d_usd)
            ),
        ));
    }
    let meter = match savings.total_saving_usd.zip(savings.opus_cost_usd) {
        Some((saved, opus)) => {
            let f = fraction(saved, opus).unwrap_or(0.0);
            format!(
                "{} {}",
                term.meter(f, METER_WIDTH),
                term.heat(f, &percent(f))
            )
        }
        None => "n/a (no opus price)".to_owned(),
    };
    rows.push(("Efficiency meter", meter));
    out.push_str(&key_values(&rows));

    out.push_str(&notes(term, stats));
    if let Some(before_after) = &stats.before_after {
        out.push_str(&before_after_line(term, before_after));
    }

    out.push('\n');
    out.push_str(&groups_table(term, stats));
    out
}

fn notes(term: Term, stats: &Stats) -> String {
    let mut out = String::new();
    let (fallback, unknown): (Vec<_>, Vec<_>) = stats
        .unpriced_models
        .iter()
        .partition(|m| m.reason == UnpricedReason::FamilyFallback);
    if !fallback.is_empty() {
        let names = fallback
            .iter()
            .map(|m| m.model.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "{}",
            term.yellow(&format!(
                "▶ {} model(s) priced by family fallback (not in the price table): {names}",
                fallback.len()
            ))
        );
    }
    if !unknown.is_empty() {
        let names = unknown
            .iter()
            .map(|m| m.model.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "{}",
            term.yellow(&format!(
                "▶ {} model(s) could not be priced and count as $0: {names}",
                unknown.len()
            ))
        );
    }
    out
}

fn before_after_line(term: Term, ba: &BeforeAfter) -> String {
    match (
        ba.before.cost_per_session_usd,
        ba.after.cost_per_session_usd,
    ) {
        (Some(before), Some(after)) => {
            let change = if before > 0.0 {
                (after - before) / before
            } else {
                0.0
            };
            let text = format!(
                "▶ Cost per session: {} before tokred ({} sessions) -> {} after ({} sessions), {:+.0}%",
                usd(before),
                integer(ba.before.sessions),
                usd(after),
                integer(ba.after.sessions),
                change * 100.0
            );
            let styled = if change <= 0.0 {
                term.heat(1.0, &text)
            } else {
                term.yellow(&text)
            };
            format!("{styled}\n")
        }
        _ => String::new(),
    }
}

fn group_heading(by: GroupBy) -> (&'static str, &'static str) {
    match by {
        GroupBy::Model => ("By Model", "Model"),
        GroupBy::Day => ("By Day", "Day"),
        GroupBy::Project => ("By Project", "Project"),
    }
}

fn groups_table(term: Term, stats: &Stats) -> String {
    let (heading, first) = group_heading(stats.group_by);
    let mut groups: Vec<&Group> = stats.groups.iter().collect();
    match stats.group_by {
        GroupBy::Day => groups.sort_by(|a, b| a.key.cmp(&b.key)),
        _ => groups.sort_by(|a, b| {
            b.cost_usd
                .total_cmp(&a.cost_usd)
                .then_with(|| a.key.cmp(&b.key))
        }),
    }
    let max_cost = groups.iter().map(|g| g.cost_usd).fold(0.0_f64, f64::max);
    let total_cost = stats.totals.cost_usd;

    let mut rows = Vec::new();
    for (index, group) in groups.iter().take(MAX_ROWS).enumerate() {
        rows.push(vec![
            format!("{}.", index + 1),
            term.accent(&truncate(&group.key, MAX_NAME)),
            integer(group.messages),
            compact(group.tokens.total),
            usd(group.cost_usd),
            fraction(group.cost_usd, total_cost).map_or_else(|| "-".to_owned(), percent),
            term.bar(
                fraction(group.cost_usd, max_cost).unwrap_or(0.0),
                IMPACT_WIDTH,
            ),
        ]);
    }
    let columns = [
        Column {
            title: "#",
            align: Align::Right,
        },
        Column {
            title: first,
            align: Align::Left,
        },
        Column {
            title: "Msgs",
            align: Align::Right,
        },
        Column {
            title: "Tokens",
            align: Align::Right,
        },
        Column {
            title: "Cost",
            align: Align::Right,
        },
        Column {
            title: "Share",
            align: Align::Right,
        },
        Column {
            title: "Impact",
            align: Align::Left,
        },
    ];
    let mut out = format!("{}\n", term.accent(heading));
    out.push_str(&table(term, &columns, &rows));
    if groups.len() > MAX_ROWS {
        let _ = writeln!(
            out,
            "{}",
            term.dim(&format!("... and {} more", groups.len() - MAX_ROWS))
        );
    }
    out
}

/// The hook-latency dashboard.
#[must_use]
pub fn render_latency(
    term: Term,
    latency: &LatencyStats,
    tradeoff: &Tradeoff,
    by: Option<LatencyBy>,
    label: &str,
) -> String {
    let mut out = title(term, &format!("tokred Hook Latency ({label})"));
    out.push('\n');
    if latency.runs == 0 {
        out.push_str(&term.yellow("No hook runs recorded for this period.\n"));
        out.push_str("Run `tokred configure`, use Claude Code for a while, then try again.\n");
        return out;
    }

    let o = &latency.outcomes;
    let problems = o.error + o.panic + o.timeout_guard;
    let outcomes = format!(
        "ok {} · error {} · panic {} · timeout guard {}",
        integer(o.ok),
        integer(o.error),
        integer(o.panic),
        integer(o.timeout_guard)
    );
    let rows: Vec<(&str, String)> = vec![
        ("Hook runs", integer(latency.runs)),
        (
            "Time added",
            format!(
                "{} {}",
                ms(latency.total_ms),
                term.dim(&format!(
                    "(estimate: includes {} of process start-up per run)",
                    ms(to_ms(latency.spawn_overhead_us))
                ))
            ),
        ),
        (
            "Per user prompt",
            latency
                .avg_ms_per_prompt
                .map_or_else(|| "n/a".to_owned(), ms),
        ),
        (
            "Outcomes",
            if problems == 0 {
                term.heat(1.0, &outcomes)
            } else {
                term.yellow(&outcomes)
            },
        ),
        ("Budget", format!("p95 under {}", ms(latency.budget_ms))),
        (
            "Trade-off",
            format!(
                "{} added vs ~{} tokens avoided{}",
                ms(tradeoff.ms_added),
                compact(tradeoff.tokens_avoided),
                tradeoff
                    .saving_usd
                    .map_or_else(String::new, |s| format!(" and {} saved", usd(s)))
            ),
        ),
    ];
    out.push_str(&key_values(&rows));
    for alert in &latency.budget_alerts {
        let _ = writeln!(
            out,
            "{}",
            term.red(&format!(
                "▶ {} p95 is {} (budget {})",
                alert.hook,
                ms(alert.p95_ms),
                ms(latency.budget_ms)
            ))
        );
    }

    out.push('\n');
    match by.unwrap_or(LatencyBy::Hook) {
        LatencyBy::Hook => out.push_str(&hooks_table(term, latency)),
        LatencyBy::Day => out.push_str(&buckets_table(term, "By Day", "Day", &latency.per_day)),
        LatencyBy::Session => {
            out.push_str(&buckets_table(
                term,
                "By Session",
                "Session",
                &latency.per_session,
            ));
        }
    }
    out
}

fn to_ms(us: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let value = us as f64 / 1_000.0;
    value
}

fn hooks_table(term: Term, latency: &LatencyStats) -> String {
    let max_total = latency
        .hooks
        .iter()
        .map(|h| h.total_ms)
        .fold(0.0_f64, f64::max);
    let rows: Vec<Vec<String>> = latency
        .hooks
        .iter()
        .enumerate()
        .map(|(index, hook)| {
            let over = hook.p95_ms > latency.budget_ms;
            let p95 = ms(hook.p95_ms);
            vec![
                format!("{}.", index + 1),
                term.accent(&hook.hook),
                integer(hook.runs),
                ms(hook.p50_ms),
                if over { term.red(&p95) } else { p95 },
                ms(hook.p99_ms),
                ms(hook.max_ms),
                ms(hook.total_ms),
                term.bar(
                    fraction(hook.total_ms, max_total).unwrap_or(0.0),
                    IMPACT_WIDTH,
                ),
            ]
        })
        .collect();
    let columns = [
        Column {
            title: "#",
            align: Align::Right,
        },
        Column {
            title: "Hook",
            align: Align::Left,
        },
        Column {
            title: "Runs",
            align: Align::Right,
        },
        Column {
            title: "p50",
            align: Align::Right,
        },
        Column {
            title: "p95",
            align: Align::Right,
        },
        Column {
            title: "p99",
            align: Align::Right,
        },
        Column {
            title: "Max",
            align: Align::Right,
        },
        Column {
            title: "Total",
            align: Align::Right,
        },
        Column {
            title: "Impact",
            align: Align::Left,
        },
    ];
    format!(
        "{}\n{}",
        term.accent("By Hook"),
        table(term, &columns, &rows)
    )
}

fn buckets_table(
    term: Term,
    heading: &str,
    first: &'static str,
    buckets: &[tokred_metrics::latency::Bucket],
) -> String {
    let max_total = buckets.iter().map(|b| b.total_ms).fold(0.0_f64, f64::max);
    let rows: Vec<Vec<String>> = buckets
        .iter()
        .take(MAX_ROWS)
        .enumerate()
        .map(|(index, bucket)| {
            vec![
                format!("{}.", index + 1),
                term.accent(&truncate(&bucket.key, MAX_NAME)),
                integer(bucket.runs),
                ms(bucket.total_ms),
                term.bar(
                    fraction(bucket.total_ms, max_total).unwrap_or(0.0),
                    IMPACT_WIDTH,
                ),
            ]
        })
        .collect();
    let columns = [
        Column {
            title: "#",
            align: Align::Right,
        },
        Column {
            title: first,
            align: Align::Left,
        },
        Column {
            title: "Runs",
            align: Align::Right,
        },
        Column {
            title: "Added",
            align: Align::Right,
        },
        Column {
            title: "Impact",
            align: Align::Left,
        },
    ];
    let mut out = format!("{}\n{}", term.accent(heading), table(term, &columns, &rows));
    if buckets.len() > MAX_ROWS {
        let _ = writeln!(
            out,
            "{}",
            term.dim(&format!("... and {} more", buckets.len() - MAX_ROWS))
        );
    }
    out
}

/// The fixed-context audit.
#[must_use]
pub fn render_audit(term: Term, report: &AuditReport) -> String {
    let summary = &report.summary;
    let mut out = title(term, "tokred Context Audit");
    out.push('\n');

    let count = |kind: ItemKind| summary.by_kind.get(&kind).map_or(0, |total| total.count);
    let items_total: usize = ItemKind::ALL.iter().map(|kind| count(*kind)).sum();
    let rows: Vec<(&str, String)> = vec![
        (
            "Fixed context",
            format!(
                "~{} tokens {}",
                compact(summary.total_estimated_tokens),
                term.dim(&format!("({})", report.token_estimate))
            ),
        ),
        (
            "Loaded at start-up",
            format!("~{} tokens", compact(summary.total_startup_tokens)),
        ),
        (
            "Items",
            format!(
                "{} (CLAUDE.md {}, rules {}, skills {}, agents {})",
                integer(items_total as u64),
                count(ItemKind::ClaudeMd),
                count(ItemKind::Rule),
                count(ItemKind::Skill),
                count(ItemKind::Agent)
            ),
        ),
        (
            "MCP servers",
            format!(
                "{} configured, {} unused in the last {} days",
                summary.mcp_servers, summary.mcp_servers_unused, report.window_days
            ),
        ),
    ];
    out.push_str(&key_values(&rows));
    for warning in &report.warnings {
        let _ = writeln!(out, "{}", term.yellow(&format!("▶ {warning}")));
    }
    for note in &report.notes {
        let _ = writeln!(out, "{}", term.dim(note));
    }

    out.push('\n');
    out.push_str(&items_table(term, report));
    out.push('\n');
    out.push_str(&mcp_table(term, report));
    out
}

fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::ClaudeMd => "CLAUDE.md",
        ItemKind::Rule => "rule",
        ItemKind::Skill => "skill",
        ItemKind::Agent => "agent",
    }
}

fn items_table(term: Term, report: &AuditReport) -> String {
    let max = report
        .items
        .iter()
        .map(|i| i.estimated_tokens)
        .max()
        .unwrap_or(0);
    let rows: Vec<Vec<String>> = report
        .items
        .iter()
        .take(MAX_ROWS)
        .enumerate()
        .map(|(index, item)| {
            #[allow(clippy::cast_precision_loss)]
            let weight = fraction(item.estimated_tokens as f64, max as f64).unwrap_or(0.0);
            vec![
                format!("{}.", index + 1),
                kind_label(item.kind).to_owned(),
                term.accent(&truncate(&item.name, MAX_NAME)),
                match item.scope {
                    Scope::User => "user",
                    Scope::Project => "project",
                }
                .to_owned(),
                compact(item.estimated_tokens),
                term.bar(weight, IMPACT_WIDTH),
                item.action
                    .as_deref()
                    .map_or_else(String::new, |a| term.yellow(&truncate(a, 48))),
            ]
        })
        .collect();
    let columns = [
        Column {
            title: "#",
            align: Align::Right,
        },
        Column {
            title: "Kind",
            align: Align::Left,
        },
        Column {
            title: "Name",
            align: Align::Left,
        },
        Column {
            title: "Scope",
            align: Align::Left,
        },
        Column {
            title: "Tokens",
            align: Align::Right,
        },
        Column {
            title: "Impact",
            align: Align::Left,
        },
        Column {
            title: "Suggestion",
            align: Align::Left,
        },
    ];
    let mut out = format!("{}\n", term.accent("Heaviest Items"));
    if rows.is_empty() {
        out.push_str(&term.dim("No CLAUDE.md, rules, skills or agents found.\n"));
        return out;
    }
    out.push_str(&table(term, &columns, &rows));
    if report.items.len() > MAX_ROWS {
        let _ = writeln!(
            out,
            "{}",
            term.dim(&format!("... and {} more", report.items.len() - MAX_ROWS))
        );
    }
    out
}

fn mcp_table(term: Term, report: &AuditReport) -> String {
    let mut out = format!("{}\n", term.accent("MCP Servers"));
    if report.mcp_servers.is_empty() {
        out.push_str(&term.dim("No MCP servers configured.\n"));
    } else {
        let rows: Vec<Vec<String>> = report
            .mcp_servers
            .iter()
            .enumerate()
            .map(|(index, server)| {
                let status = if server.disabled {
                    term.dim("disabled")
                } else if server.unused {
                    term.yellow("unused")
                } else {
                    term.heat(1.0, "in use")
                };
                vec![
                    format!("{}.", index + 1),
                    term.accent(&truncate(&server.name, MAX_NAME)),
                    match server.scope {
                        McpScope::Local => "local",
                        McpScope::Project => "project",
                        McpScope::User => "user",
                    }
                    .to_owned(),
                    server.transport.clone(),
                    integer(server.calls),
                    server
                        .last_seen_unix
                        .map_or_else(|| "never".to_owned(), ymd),
                    status,
                ]
            })
            .collect();
        let columns = [
            Column {
                title: "#",
                align: Align::Right,
            },
            Column {
                title: "Server",
                align: Align::Left,
            },
            Column {
                title: "Scope",
                align: Align::Left,
            },
            Column {
                title: "Transport",
                align: Align::Left,
            },
            Column {
                title: "Calls",
                align: Align::Right,
            },
            Column {
                title: "Last used",
                align: Align::Left,
            },
            Column {
                title: "Status",
                align: Align::Left,
            },
        ];
        out.push_str(&table(term, &columns, &rows));
    }
    for usage in &report.mcp_usage_without_local_config {
        let _ = writeln!(
            out,
            "{}",
            term.dim(&format!(
                "used but not configured here: {} ({} calls; plugin or connector)",
                usage.server,
                integer(usage.calls)
            ))
        );
    }
    out
}

/// `YYYY-MM-DD` of a unix timestamp (UTC).
#[must_use]
pub fn ymd(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The price table, USD per million tokens.
#[must_use]
pub fn render_prices(term: Term, entries: &[PriceEntry]) -> String {
    let mut out = title(term, "tokred Prices (USD per million tokens)");
    out.push('\n');
    let mut sorted: Vec<&PriceEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        let family = |e: &PriceEntry| tokred_core::pricing::Family::from_model(&e.model);
        family(a)
            .cmp(&family(b))
            .then_with(|| b.model.cmp(&a.model))
    });
    let rows: Vec<Vec<String>> = sorted
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            vec![
                format!("{}.", index + 1),
                term.accent(&truncate(&entry.model, MAX_NAME)),
                usd(entry.price.input),
                usd(entry.price.output),
                usd(entry.price.cache_write),
                usd(entry.price.cache_read),
                entry.source.clone(),
            ]
        })
        .collect();
    let columns = [
        Column {
            title: "#",
            align: Align::Right,
        },
        Column {
            title: "Model",
            align: Align::Left,
        },
        Column {
            title: "Input",
            align: Align::Right,
        },
        Column {
            title: "Output",
            align: Align::Right,
        },
        Column {
            title: "Cache write",
            align: Align::Right,
        },
        Column {
            title: "Cache read",
            align: Align::Right,
        },
        Column {
            title: "Source",
            align: Align::Left,
        },
    ];
    out.push_str(&table(term, &columns, &rows));
    out
}

/// The tiered-cache state.
#[must_use]
pub fn render_cache(term: Term, stats: &CacheStats) -> String {
    let mut out = title(term, "tokred Cache");
    out.push('\n');
    if stats.total_entries() == 0 {
        out.push_str(&term.dim("The cache is empty.\n"));
        return out;
    }
    let max = stats.tiers.iter().map(|t| t.entries).max().unwrap_or(0);
    let rows: Vec<Vec<String>> = stats
        .tiers
        .iter()
        .map(|tier| {
            #[allow(clippy::cast_precision_loss)]
            let weight = fraction(tier.entries as f64, max as f64).unwrap_or(0.0);
            vec![
                term.accent(&format!("{:?}", tier.tier)),
                integer(tier.entries),
                if tier.expired > 0 {
                    term.yellow(&integer(tier.expired))
                } else {
                    integer(tier.expired)
                },
                integer(tier.hits),
                crate::view::bytes(tier.bytes),
                term.bar(weight, IMPACT_WIDTH),
            ]
        })
        .collect();
    let columns = [
        Column {
            title: "Tier",
            align: Align::Left,
        },
        Column {
            title: "Entries",
            align: Align::Right,
        },
        Column {
            title: "Expired",
            align: Align::Right,
        },
        Column {
            title: "Hits",
            align: Align::Right,
        },
        Column {
            title: "Size",
            align: Align::Right,
        },
        Column {
            title: "Impact",
            align: Align::Left,
        },
    ];
    out.push_str(&table(term, &columns, &rows));
    let _ = writeln!(
        out,
        "{}",
        term.dim("Run `tokred cache prune` to remove expired entries.")
    );
    out
}

fn bytes(n: u64) -> String {
    tokred_report::format::bytes(n)
}

/// The hook-latency benchmark of `doctor --bench`.
#[must_use]
pub fn render_bench(term: Term, report: &BenchReport, budget_ms: f64) -> String {
    let mut out = title(
        term,
        &format!(
            "tokred Hook Benchmark ({} runs per payload)",
            report.runs_per_payload
        ),
    );
    out.push('\n');
    let rows: Vec<Vec<String>> = report
        .hooks
        .iter()
        .enumerate()
        .map(|(index, hook)| {
            let verdict = if hook.p95_ms <= budget_ms {
                term.heat(1.0, "within budget")
            } else {
                term.red("over budget")
            };
            vec![
                format!("{}.", index + 1),
                term.accent(&hook.hook),
                integer(hook.runs as u64),
                ms(hook.mean_ms),
                ms(hook.p50_ms),
                ms(hook.p95_ms),
                ms(hook.max_ms),
                verdict,
            ]
        })
        .collect();
    let columns = [
        Column {
            title: "#",
            align: Align::Right,
        },
        Column {
            title: "Hook",
            align: Align::Left,
        },
        Column {
            title: "Runs",
            align: Align::Right,
        },
        Column {
            title: "Mean",
            align: Align::Right,
        },
        Column {
            title: "p50",
            align: Align::Right,
        },
        Column {
            title: "p95",
            align: Align::Right,
        },
        Column {
            title: "Max",
            align: Align::Right,
        },
        Column {
            title: "Budget",
            align: Align::Left,
        },
    ];
    out.push_str(&table(term, &columns, &rows));
    let _ = writeln!(
        out,
        "Process start-up overhead: {} per run {}",
        ms(to_ms(report.spawn_overhead_us)),
        term.dim(&format!(
            "(saved for `tokred stats latency`; budget: p95 under {})",
            ms(budget_ms)
        ))
    );
    out
}

#[cfg(test)]
mod tests {
    use tokred_core::Timestamp;
    use tokred_core::hook_event::{HookEventRecord, Outcome, decision, reason};
    use tokred_core::pricing::{ModelPrice, PriceEntry, PriceTable, Usage};
    use tokred_core::usage::UsageRecord;
    use tokred_metrics::{StatsInput, TimeRange, compute_stats, latency_stats, tradeoff};

    use super::*;
    use crate::term::visible_width;

    const OFF: Term = Term::new(false);
    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;

    fn price(model: &str, input: f64) -> PriceEntry {
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

    fn prices() -> PriceTable {
        PriceTable::new(vec![
            price("claude-opus-4-1-20250805", 15.0),
            price("claude-sonnet-4-20250514", 3.0),
        ])
    }

    fn record(key: &str, session: &str, model: &str, day: i64, input: u64) -> UsageRecord {
        UsageRecord {
            message_key: key.to_owned(),
            session_id: session.to_owned(),
            project: "proj".to_owned(),
            ts: Timestamp(NOW - day * DAY),
            model: model.to_owned(),
            usage: Usage {
                input,
                output: input / 2,
                cache_write: 100,
                cache_write_1h: 0,
                cache_read: 1_000,
            },
            is_subagent: false,
            origin: None,
            agent_type: None,
        }
    }

    fn event(
        hook: &str,
        decision: &str,
        reason: &str,
        bytes: u64,
        us: u64,
        day: i64,
    ) -> HookEventRecord {
        HookEventRecord {
            ts: Timestamp(NOW - day * DAY),
            session_id: "s1".to_owned(),
            hook: hook.to_owned(),
            decision: decision.to_owned(),
            reason: reason.to_owned(),
            bytes_saved: bytes,
            duration_us: us,
            outcome: Outcome::Ok,
        }
    }

    fn stats_for(usage: &[UsageRecord], events: &[HookEventRecord], by: GroupBy) -> Stats {
        let table = prices();
        compute_stats(&StatsInput {
            usage,
            prices: &table,
            events,
            range: TimeRange::new(Timestamp(NOW - 40 * DAY), Timestamp(NOW + 1)),
            group_by: by,
            now: Timestamp(NOW),
            first_configure: None,
        })
    }

    fn rich() -> Stats {
        let usage = vec![
            record("a", "s1", "claude-opus-4-1-20250805", 1, 2_000_000),
            record("b", "s1", "claude-sonnet-4-20250514", 1, 3_000_000),
            record("c", "s2", "claude-sonnet-4-20250514", 2, 1_000_000),
        ];
        let events = vec![event(
            "guard-read",
            decision::DENY,
            reason::REREAD,
            40_000,
            1_000,
            1,
        )];
        stats_for(&usage, &events, GroupBy::Model)
    }

    #[test]
    fn the_cost_dashboard_has_the_sections_of_the_example() {
        let text = render_stats(OFF, &rich(), "last 7 days");
        assert!(
            text.starts_with("tokred Token Savings (last 7 days)\n═"),
            "{text}"
        );
        for expected in [
            "Messages:",
            "(2 sessions)",
            "Input tokens:",
            "Output tokens:",
            "Cache read/write:",
            "Real cost:",
            "If all on opus:",
            "Routing saving:",
            "Guard saving:",
            "estimate: 1 blocks",
            "Total saved:",
            "Prompt cache hit:",
            "Efficiency meter:",
            "By Model",
            "claude-sonnet-4-20250514",
            "claude-opus-4-1-20250805",
            "Impact",
            "Share",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(!text.contains('\x1b'), "no color codes when color is off");
        // The meter and the Impact bars are drawn with block characters.
        assert!(text.contains('█') && text.contains('░'));
        // Rows are numbered and the biggest spender (opus here) comes first.
        let first = text
            .lines()
            .find(|l| l.trim_start().starts_with("1."))
            .unwrap();
        assert!(first.contains("claude-opus-4-1-20250805"), "{first}");
    }

    #[test]
    fn color_mode_paints_without_changing_the_layout() {
        let plain = render_stats(OFF, &rich(), "last 7 days");
        let colored = render_stats(Term::new(true), &rich(), "last 7 days");
        assert!(colored.contains('\x1b'));
        let strip = |s: &str| s.lines().map(visible_width).collect::<Vec<_>>();
        assert_eq!(strip(&plain), strip(&colored));
    }

    #[test]
    fn an_empty_period_says_so() {
        let stats = stats_for(&[], &[], GroupBy::Model);
        let text = render_stats(OFF, &stats, "last 7 days");
        assert!(text.contains("No Claude Code usage found for this period."));
        assert!(!text.contains("By Model"));
    }

    #[test]
    fn unpriced_models_are_flagged() {
        let usage = vec![
            record("a", "s1", "claude-opus-9-20990101", 1, 1_000),
            record("b", "s1", "mystery-model", 1, 1_000),
            record("c", "s1", "claude-sonnet-4-20250514", 1, 1_000),
        ];
        let text = render_stats(OFF, &stats_for(&usage, &[], GroupBy::Model), "x");
        assert!(
            text.contains("▶ 1 model(s) priced by family fallback"),
            "{text}"
        );
        assert!(text.contains("claude-opus-9-20990101"));
        assert!(text.contains("▶ 1 model(s) could not be priced"), "{text}");
        assert!(text.contains("mystery-model"));
    }

    #[test]
    fn without_an_opus_price_the_meter_says_so() {
        let usage = vec![record("c", "s1", "claude-sonnet-4-20250514", 1, 1_000)];
        let table = PriceTable::new(vec![price("claude-sonnet-4-20250514", 3.0)]);
        let stats = compute_stats(&StatsInput {
            usage: &usage,
            prices: &table,
            events: &[],
            range: TimeRange::new(Timestamp(NOW - 10 * DAY), Timestamp(NOW + 1)),
            group_by: GroupBy::Model,
            now: Timestamp(NOW),
            first_configure: None,
        });
        let text = render_stats(OFF, &stats, "x");
        assert!(text.contains("n/a (no opus price)"), "{text}");
        assert!(
            text.contains("If all on opus:    n/a")
                || text.contains("If all on opus: n/a")
                || text.contains("n/a")
        );
    }

    #[test]
    fn groups_by_day_are_chronological_and_long_tables_are_cut() {
        let usage: Vec<_> = (0..20)
            .map(|day| {
                record(
                    &format!("m{day}"),
                    "s",
                    "claude-sonnet-4-20250514",
                    day,
                    1_000,
                )
            })
            .collect();
        let text = render_stats(OFF, &stats_for(&usage, &[], GroupBy::Day), "x");
        assert!(text.contains("By Day"));
        assert!(text.contains("... and 5 more"), "{text}");
        let days: Vec<&str> = text
            .lines()
            .filter(|l| {
                l.contains("20")
                    && l.trim_start()
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit())
            })
            .collect();
        let mut sorted = days.clone();
        sorted.sort_by_key(|l| l.split_whitespace().nth(1).map(str::to_owned));
        assert_eq!(days, sorted, "days ascend");

        let by_project = render_stats(OFF, &stats_for(&usage, &[], GroupBy::Project), "x");
        assert!(by_project.contains("By Project") && by_project.contains("proj"));
    }

    #[test]
    fn before_and_after_configuring_is_summarized() {
        let usage = vec![
            record("a", "old", "claude-opus-4-1-20250805", 30, 4_000_000),
            record("b", "new", "claude-sonnet-4-20250514", 1, 1_000_000),
        ];
        let table = prices();
        let range = TimeRange::new(Timestamp(NOW - 40 * DAY), Timestamp(NOW + 1));
        let build = |first: Option<i64>| {
            compute_stats(&StatsInput {
                usage: &usage,
                prices: &table,
                events: &[],
                range,
                group_by: GroupBy::Model,
                now: Timestamp(NOW),
                first_configure: first.map(Timestamp),
            })
        };
        let better = render_stats(OFF, &build(Some(NOW - 10 * DAY)), "x");
        assert!(better.contains("▶ Cost per session:"), "{better}");
        assert!(better.contains("before tokred (1 sessions)"));
        assert!(better.contains('-'), "a saving shows a negative change");

        // Flip it: expensive after configuring.
        let worse_usage = vec![
            record("a", "old", "claude-sonnet-4-20250514", 30, 100_000),
            record("b", "new", "claude-opus-4-1-20250805", 1, 4_000_000),
        ];
        let worse = compute_stats(&StatsInput {
            usage: &worse_usage,
            prices: &table,
            events: &[],
            range,
            group_by: GroupBy::Model,
            now: Timestamp(NOW),
            first_configure: Some(Timestamp(NOW - 10 * DAY)),
        });
        assert!(render_stats(OFF, &worse, "x").contains('+'));
        // Nothing before configuring: no comparison line.
        assert!(!render_stats(OFF, &build(Some(NOW - 60 * DAY)), "x").contains("Cost per session"));
    }

    fn latency(events: &[HookEventRecord], spawn_us: u64) -> (LatencyStats, Tradeoff) {
        let range = TimeRange::new(Timestamp(NOW - 40 * DAY), Timestamp(NOW + 1));
        let lat = latency_stats(events, spawn_us, range);
        let stats = rich();
        let trade = tradeoff(&lat, &stats);
        (lat, trade)
    }

    #[test]
    fn the_latency_dashboard_lists_hooks_with_percentiles() {
        let events = vec![
            event("guard-read", decision::ALLOW, "", 0, 2_000, 1),
            event("guard-read", decision::DENY, reason::REREAD, 10, 4_000, 1),
            event("route", "implementation", reason::SUGGESTED, 0, 90_000, 2),
        ];
        let (lat, trade) = latency(&events, 5_000);
        let text = render_latency(OFF, &lat, &trade, None, "last 7 days");
        for expected in [
            "tokred Hook Latency (last 7 days)",
            "Hook runs:",
            "Time added:",
            "estimate: includes",
            "Per user prompt:",
            "Outcomes:",
            "ok 3",
            "Budget:",
            "Trade-off:",
            "By Hook",
            "guard-read",
            "route",
            "p95",
            "Impact",
            "▶ route p95 is",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }

    #[test]
    fn latency_can_be_grouped_by_day_or_session_and_problems_are_highlighted() {
        let mut events = vec![event("guard-read", decision::ALLOW, "", 0, 2_000, 1)];
        let mut failing = event("guard-read", decision::NONE, "", 0, 1_000, 2);
        failing.outcome = Outcome::Error;
        events.push(failing);
        let many: Vec<_> = (0..20)
            .map(|d| event("route", "other", reason::UNCHANGED, 0, 1_000, d))
            .collect();
        events.extend(many);
        let (lat, trade) = latency(&events, 0);

        let by_day = render_latency(Term::new(true), &lat, &trade, Some(LatencyBy::Day), "x");
        assert!(
            by_day.contains("By Day") && by_day.contains("Added") && by_day.contains("... and "),
            "{by_day}"
        );
        assert!(by_day.contains("\x1b[33m"), "errors are yellow");
        let by_session = render_latency(OFF, &lat, &trade, Some(LatencyBy::Session), "x");
        assert!(by_session.contains("By Session") && by_session.contains("s1"));
        let by_hook = render_latency(OFF, &lat, &trade, Some(LatencyBy::Hook), "x");
        assert!(by_hook.contains("By Hook"));
    }

    #[test]
    fn latency_without_runs_explains_what_to_do() {
        let (lat, trade) = latency(&[], 0);
        let text = render_latency(OFF, &lat, &trade, None, "last 7 days");
        assert!(text.contains("No hook runs recorded for this period."));
        assert!(text.contains("tokred configure"));
    }

    #[test]
    fn the_tradeoff_without_a_price_table_omits_the_dollar_figure() {
        let events = vec![event("guard-read", decision::ALLOW, "", 0, 2_000, 1)];
        let (lat, mut trade) = latency(&events, 0);
        trade.saving_usd = None;
        let text = render_latency(OFF, &lat, &trade, None, "x");
        assert!(text.contains("tokens avoided"));
        assert!(!text.contains("saved"));
    }

    #[test]
    fn helpers_handle_edge_cases() {
        assert_eq!(fraction(1.0, 0.0), None);
        assert_eq!(fraction(f64::NAN, 1.0), None);
        assert_eq!(fraction(5.0, 2.0), Some(1.0));
        assert_eq!(money_or_na(None), "n/a");
        assert!((to_ms(2_500) - 2.5).abs() < f64::EPSILON);
    }

    // ---- audit and benchmark screens ----

    use tokred_audit::report::{AuditItem, KindTotal, McpServerReport, Summary, UnconfiguredUsage};

    fn item(
        kind: ItemKind,
        scope: Scope,
        name: &str,
        tokens: u64,
        action: Option<&str>,
    ) -> AuditItem {
        AuditItem {
            kind,
            scope,
            name: name.to_owned(),
            path: std::path::PathBuf::from("/x"),
            imported_by: None,
            estimated_tokens: tokens,
            startup_tokens: None,
            action: action.map(str::to_owned),
        }
    }

    fn server(
        name: &str,
        scope: McpScope,
        calls: u64,
        last: Option<i64>,
        unused: bool,
        disabled: bool,
    ) -> McpServerReport {
        McpServerReport {
            name: name.to_owned(),
            scope,
            also_in: vec![],
            source: std::path::PathBuf::from("/x"),
            transport: "stdio".to_owned(),
            disabled,
            calls,
            last_seen_unix: last,
            unused,
            action: None,
        }
    }

    fn audit_report(items: Vec<AuditItem>, servers: Vec<McpServerReport>) -> AuditReport {
        let mut by_kind = std::collections::BTreeMap::new();
        by_kind.insert(
            ItemKind::ClaudeMd,
            KindTotal {
                count: 1,
                estimated_tokens: 900,
            },
        );
        AuditReport {
            token_estimate: "estimate: characters / 4".to_owned(),
            window_days: 30,
            items,
            mcp_servers: servers,
            mcp_usage_without_local_config: vec![UnconfiguredUsage {
                server: "plugin-x".to_owned(),
                calls: 7,
                last_seen_unix: None,
            }],
            summary: Summary {
                by_kind,
                total_estimated_tokens: 1_200,
                total_startup_tokens: 900,
                mcp_servers: 3,
                mcp_servers_unused: 1,
            },
            notes: vec!["MCP schema weight is not computable locally".to_owned()],
            warnings: vec!["a skill listing is long".to_owned()],
        }
    }

    #[test]
    fn the_audit_screen_shows_items_servers_notes_and_warnings() {
        let items = vec![
            item(
                ItemKind::ClaudeMd,
                Scope::User,
                "CLAUDE.md",
                900,
                Some("move to path-scoped rules"),
            ),
            item(ItemKind::Rule, Scope::Project, "style", 200, None),
            item(ItemKind::Skill, Scope::User, "deploy", 100, None),
            item(ItemKind::Agent, Scope::Project, "helper", 50, None),
        ];
        let servers = vec![
            server(
                "busy",
                McpScope::Local,
                12,
                Some(1_700_000_000),
                false,
                false,
            ),
            server("idle", McpScope::Project, 0, None, true, false),
            server("off", McpScope::User, 0, None, true, true),
        ];
        let text = render_audit(OFF, &audit_report(items, servers));
        for expected in [
            "tokred Context Audit",
            "Fixed context:",
            "~1.2k tokens",
            "estimate: characters / 4",
            "Loaded at start-up:",
            "Items:",
            "1 (CLAUDE.md 1, rules 0, skills 0, agents 0)",
            "1 unused",
            "▶ a skill listing is long",
            "MCP schema weight is not computable locally",
            "Heaviest Items",
            "CLAUDE.md",
            "rule",
            "skill",
            "agent",
            "user",
            "project",
            "move to path-scoped rules",
            "MCP Servers",
            "local",
            "stdio",
            "2023-11-14",
            "never",
            "in use",
            "unused",
            "disabled",
            "used but not configured here: plugin-x (7 calls",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }

    #[test]
    fn an_empty_audit_and_a_long_one() {
        let empty = render_audit(OFF, &audit_report(vec![], vec![]));
        assert!(empty.contains("No CLAUDE.md, rules, skills or agents found."));
        assert!(empty.contains("No MCP servers configured."));

        let many: Vec<_> = (0..20)
            .map(|i| {
                item(
                    ItemKind::Skill,
                    Scope::User,
                    &format!("skill-{i}"),
                    10 + i,
                    None,
                )
            })
            .collect();
        let long = render_audit(Term::new(true), &audit_report(many, vec![]));
        assert!(long.contains("... and 5 more"), "{long}");
    }

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(951_782_400), "2000-02-29");
        assert_eq!(ymd(1_700_000_000), "2023-11-14");
        assert_eq!(ymd(-86_400), "1969-12-31");
    }

    #[test]
    fn prices_are_listed_by_family_newest_first() {
        let text = render_prices(OFF, prices().entries());
        assert!(text.contains("tokred Prices (USD per million tokens)"));
        let opus = text.find("claude-opus").unwrap();
        let sonnet = text.find("claude-sonnet").unwrap();
        assert!(opus < sonnet, "opus before sonnet");
        assert!(text.contains("Cache write") && text.contains("$15.00"));
    }

    #[test]
    fn the_benchmark_screen_marks_hooks_over_budget() {
        use crate::commands::BenchReport;
        let hook = |name: &str, p95: f64| crate::commands::HookBench {
            hook: name.to_owned(),
            runs: 50,
            mean_ms: 8.0,
            p50_ms: 7.5,
            p95_ms: p95,
            max_ms: p95 + 5.0,
        };
        let report = BenchReport {
            runs_per_payload: 50,
            hooks: vec![hook("guard-read", 12.0), hook("route", 80.0)],
            spawn_overhead_us: 7_000,
        };
        let text = render_bench(OFF, &report, 50.0);
        for expected in [
            "tokred Hook Benchmark (50 runs per payload)",
            "guard-read",
            "within budget",
            "over budget",
            "Process start-up overhead: 7.00 ms",
            "budget: p95 under 50.0 ms",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(render_bench(Term::new(true), &report, 50.0).contains("\x1b[31mover budget"));
    }

    #[test]
    fn the_cost_screen_covers_zero_cost_baselines_and_missing_cache_data() {
        // A "before" period priced at $0 (unpriced model): no division by zero, no change shown as inf.
        let usage = vec![
            record("a", "old", "mystery-model", 30, 4_000_000),
            record("b", "new", "claude-sonnet-4-20250514", 1, 1_000_000),
        ];
        let table = prices();
        let stats = compute_stats(&StatsInput {
            usage: &usage,
            prices: &table,
            events: &[],
            range: TimeRange::new(Timestamp(NOW - 40 * DAY), Timestamp(NOW + 1)),
            group_by: GroupBy::Model,
            now: Timestamp(NOW),
            first_configure: Some(Timestamp(NOW - 10 * DAY)),
        });
        let text = render_stats(OFF, &stats, "x");
        assert!(text.contains("Cost per session"), "{text}");
        assert!(text.contains("+0%") || text.contains("0%"));
    }
}
