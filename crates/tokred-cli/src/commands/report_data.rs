//! Maps the metrics results into the input of the HTML report.

use tokred_audit::AuditReport;
use tokred_core::Timestamp;
use tokred_core::hook_event::reason;
use tokred_core::pricing::{Family, PriceTable};
use tokred_metrics::{
    GroupBy, LatencyStats, Stats, StatsInput, TimeRange, compute_stats, guards_summary,
    latency_stats, routing_summary,
};
use tokred_pricing::PRICE_SYNCED_AT;
use tokred_report::model::{
    AuditItem, AuditStats, DayCache, DayCost, GuardReason, GuardStat, HookLatency,
    LatencyStats as ReportLatency, ModelFamily, ModelUsage, ProxyBreakdown, ProxyMiss,
    ProxyOverhead, ProxyReport, ProxyRoute, ProxyRouting, ReportData, ReportMeta, RoutingStats,
    Summary, TaskTypeCount,
};
use tokred_report::render_html;

use super::data::{Gathered, Repos, gather};
use crate::view::ymd;

const DAY: i64 = 86_400;

/// One entry per calendar day (UTC) of `range`, with the real cost and the "all on opus" cost.
fn daily_costs(data: &Gathered) -> Vec<DayCost> {
    let first = data.range.from.0.div_euclid(DAY);
    let last = (data.range.to.0 - 1).div_euclid(DAY);
    (first..=last)
        .map(|day| {
            let range = TimeRange::new(Timestamp(day * DAY), Timestamp((day + 1) * DAY));
            let slice: Vec<_> = data
                .usage
                .iter()
                .filter(|u| range.contains(u.ts))
                .cloned()
                .collect();
            let stats = compute_stats(&StatsInput {
                usage: &slice,
                prices: &data.prices,
                events: &[],
                decisions: &[],
                range,
                group_by: GroupBy::Model,
                now: data.now,
                first_configure: None,
            });
            let real = stats.savings.real_cost_usd;
            DayCost {
                date: ymd(day * DAY),
                real_usd: real,
                all_opus_usd: stats.savings.opus_cost_usd.unwrap_or(real),
            }
        })
        .collect()
}

fn model_family(model: &str) -> ModelFamily {
    match Family::from_model(model) {
        Some(Family::Opus) => ModelFamily::Opus,
        Some(Family::Sonnet) => ModelFamily::Sonnet,
        Some(Family::Haiku) => ModelFamily::Haiku,
        Some(Family::Fable) => ModelFamily::Fable,
        Some(Family::Mythos) => ModelFamily::Mythos,
        None => ModelFamily::Other,
    }
}

fn guard_reason(name: &str) -> Option<GuardReason> {
    match name {
        reason::REREAD => Some(GuardReason::Reread),
        reason::GENERATED_FILE => Some(GuardReason::GeneratedFile),
        reason::LARGE_FILE => Some(GuardReason::LargeFile),
        reason::BROAD_SEARCH => Some(GuardReason::BroadSearch),
        _ => None,
    }
}

/// Stats grouped by model for the period (the donut and the table).
pub fn model_stats(data: &Gathered, prices: &PriceTable) -> Stats {
    compute_stats(&StatsInput {
        usage: &data.usage,
        prices,
        events: &data.events,
        decisions: &data.decisions,
        range: data.range,
        group_by: GroupBy::Model,
        now: data.now,
        first_configure: data.first_configure,
    })
}

fn summary_of(stats: &Stats, latency: &LatencyStats) -> Option<Summary> {
    summary_with(stats, latency, false)
}

/// The summary cards of `stats`; the projection comes from the last 30 days when `from_30d`,
/// else from the last 7.
fn summary_with(stats: &Stats, latency: &LatencyStats, from_30d: bool) -> Option<Summary> {
    (stats.totals.messages > 0).then(|| Summary {
        real_cost_usd: stats.savings.real_cost_usd,
        model_choice_saving_usd: stats.savings.model_choice_saving_usd.unwrap_or(0.0),
        routing_saving_usd: stats.savings.routing_saving_usd,
        guard_saving_usd: stats.savings.guard.saving_usd,
        tokens_avoided: stats.savings.guard.tokens_avoided,
        monthly_projection_usd: stats.projection.as_ref().map_or(0.0, |p| {
            if from_30d {
                p.last_30d.projected_30d_usd
            } else {
                p.last_7d.projected_30d_usd
            }
        }),
        latency_added_ms: latency.total_ms,
        hook_runs: latency.runs,
        since: None,
    })
}

/// The summary cards over the last `days` calendar days of the period (all of it when it is
/// shorter): the same days the daily charts show for that view.
fn summary_of_last(data: &Gathered, days: i64) -> Option<Summary> {
    let last_day = (data.range.to.0 - 1).div_euclid(DAY);
    let from = Timestamp(((last_day - days + 1) * DAY).max(data.range.from.0));
    let range = TimeRange::new(from, data.range.to);
    let stats = compute_stats(&StatsInput {
        usage: &data.usage,
        prices: &data.prices,
        events: &data.events,
        decisions: &data.decisions,
        range,
        group_by: GroupBy::Model,
        now: data.now,
        first_configure: data.first_configure,
    });
    let latency = latency_stats(&data.events, data.spawn_overhead_us, range);
    summary_with(&stats, &latency, days >= 30).map(|summary| Summary {
        since: Some(ymd(from.0)),
        ..summary
    })
}

fn routing_of(data: &Gathered) -> Option<RoutingStats> {
    let routing = routing_summary(&data.events, data.range);
    (routing.total_classified > 0 || routing.delegations.total > 0).then(|| RoutingStats {
        classifications: routing
            .classifications
            .iter()
            .map(|c| TaskTypeCount {
                task_type: c.class.clone(),
                count: c.count,
            })
            .collect(),
        delegations_correct: routing.delegations.correct,
        delegations_corrected: routing.delegations.corrected,
    })
}

fn guards_of(data: &Gathered) -> Vec<GuardStat> {
    guards_summary(&data.events, data.range)
        .reasons
        .iter()
        .filter_map(|r| {
            guard_reason(&r.reason).map(|reason| GuardStat {
                reason,
                blocks: r.blocks,
                bytes_avoided: r.bytes_saved,
                tokens_avoided: r.tokens_avoided,
            })
        })
        .collect()
}

fn latency_of(data: &Gathered, stats: &Stats, latency: &LatencyStats) -> Option<ReportLatency> {
    (latency.runs > 0).then(|| ReportLatency {
        budget_ms: latency.budget_ms,
        hooks: latency
            .hooks
            .iter()
            .map(|h| HookLatency {
                hook: h.hook.clone(),
                runs: h.runs,
                p50_ms: h.p50_ms,
                p95_ms: h.p95_ms,
                max_ms: h.max_ms,
            })
            .collect(),
        total_added_ms: latency.total_ms,
        saved_usd: stats.savings.total_saving_usd,
        saved_tokens: stats.savings.guard.tokens_avoided,
        spawn_overhead_ms: (data.spawn_overhead_us > 0)
            .then(|| u64_to_f64(data.spawn_overhead_us) / 1_000.0),
    })
}

fn audit_of(audit: &AuditReport) -> AuditStats {
    AuditStats {
        heaviest: audit
            .items
            .iter()
            .take(10)
            .map(|i| AuditItem {
                name: i.name.clone(),
                kind: format!("{:?}", i.kind).to_lowercase(),
                origin: format!("{:?}", i.scope).to_lowercase(),
                tokens: i.estimated_tokens,
            })
            .collect(),
        unused_mcp: audit
            .mcp_servers
            .iter()
            .filter(|s| s.unused && !s.disabled)
            .map(|s| s.name.clone())
            .collect(),
        unused_window_days: Some(audit.window_days),
    }
}

/// Builds the report from everything that was loaded and computed.
pub fn build(
    data: &Gathered,
    stats: &Stats,
    latency: &LatencyStats,
    audit: Option<&AuditReport>,
    last_price_sync: Option<i64>,
) -> ReportData {
    let has_usage = stats.totals.messages > 0;
    ReportData {
        meta: ReportMeta {
            generated_at: ymd(data.now.0),
            tokred_version: env!("CARGO_PKG_VERSION").to_owned(),
            period_start: Some(ymd(data.range.from.0)),
            period_end: Some(ymd(data.range.to.0 - 1)),
            last_price_sync: last_price_sync.map(ymd),
        },
        summary: summary_of(stats, latency).map(|summary| Summary {
            since: Some(ymd(data.range.from.0)),
            ..summary
        }),
        summary_7d: summary_of_last(data, 7),
        summary_30d: summary_of_last(data, 30),
        cost_per_day: if has_usage {
            daily_costs(data)
        } else {
            Vec::new()
        },
        models: stats
            .groups
            .iter()
            .map(|g| ModelUsage {
                name: g.key.clone(),
                family: model_family(&g.key),
                tokens: g.tokens.total,
                cost_usd: g.cost_usd,
            })
            .collect(),
        routing: routing_of(data),
        guards: guards_of(data),
        cache_per_day: stats
            .cache_rate
            .per_day
            .iter()
            .filter_map(|d| {
                d.rate.map(|hit_rate| DayCache {
                    date: d.day.clone(),
                    hit_rate,
                })
            })
            .collect(),
        latency: latency_of(data, stats, latency),
        audit: audit.map(audit_of),
        proxy: proxy_of(data),
    }
}

/// Cost per origin or agent type, the most expensive first.
fn breakdown(data: &Gathered, by: GroupBy) -> Vec<ProxyBreakdown> {
    let mut rows: Vec<ProxyBreakdown> = super::stats::compute(data, by)
        .groups
        .into_iter()
        .map(|group| ProxyBreakdown {
            name: group.key,
            messages: group.messages,
            tokens: group.tokens.total,
            cost_usd: group.cost_usd,
        })
        .collect();
    rows.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd).then(a.name.cmp(&b.name)));
    rows
}

/// What the proxy measured (`None` when it recorded nothing in the period).
fn proxy_of(data: &Gathered) -> Option<ProxyReport> {
    let proxy = data.proxy()?;
    Some(ProxyReport {
        measured_messages: proxy.coverage.measured,
        transcript_only_messages: proxy.coverage.transcript_only,
        routes: proxy
            .models
            .iter()
            .map(|route| ProxyRoute {
                requested: route.requested.clone(),
                effective: route.effective.clone(),
                requests: route.requests,
                cost_usd: route.cost_usd,
            })
            .collect(),
        routing: proxy.routing.map(|routing| ProxyRouting {
            applied: routing.applied,
            observed: routing.observed,
            kept: routing.kept,
            fallback: routing.fallback,
            applied_saving_usd: routing.applied_saving_usd,
            observed_saving_usd: routing.observed_saving_usd,
        }),
        overhead: proxy.overhead.map(|overhead| ProxyOverhead {
            requests: overhead.requests,
            p50_ms: overhead.p50_ms,
            p95_ms: overhead.p95_ms,
            p99_ms: overhead.p99_ms,
            max_ms: overhead.max_ms,
            budget_ms: overhead.budget_ms,
        }),
        main_requests: proxy
            .cache_misses
            .as_ref()
            .map_or(0, |misses| misses.main_requests),
        cache_misses: proxy.cache_misses.map_or_else(Vec::new, |misses| {
            misses
                .causes
                .into_iter()
                .map(|cause| ProxyMiss {
                    cause: cause.cause,
                    requests: cause.requests,
                    cache_write_tokens: cause.cache_write_tokens,
                })
                .collect()
        }),
        by_origin: breakdown(data, GroupBy::Origin),
        by_agent_type: breakdown(data, GroupBy::AgentType),
        suggestion: proxy.suggestion.map(|suggestion| suggestion.message),
    })
}

/// The HTML report of a period, from the repositories alone: nothing is read from the transcripts
/// or the disk, nothing is written or opened. This is what `tokred metrics` writes to its file and
/// what the proxy serves at `/metrics`.
pub fn report_html(
    repos: &Repos<'_>,
    prices: PriceTable,
    now: Timestamp,
    since: &str,
    audit: Option<&AuditReport>,
) -> anyhow::Result<String> {
    let data = gather(repos, prices, now, since)?;
    let last_sync = repos
        .meta
        .meta_get(PRICE_SYNCED_AT)?
        .and_then(|value| value.parse::<i64>().ok());
    Ok(html_of(&data, audit, last_sync))
}

/// The HTML report of data that was already gathered.
fn html_of(data: &Gathered, audit: Option<&AuditReport>, last_sync: Option<i64>) -> String {
    let stats = model_stats(data, &data.prices);
    let latency = latency_stats(&data.events, data.spawn_overhead_us, data.range);
    render_html(&build(data, &stats, &latency, audit, last_sync))
}

fn u64_to_f64(value: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let out = value as f64;
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tokred_core::meta::MetaRepo;
    use tokred_metrics::latency_stats;

    use super::super::data::load;
    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn loaded(sb: &Sandbox) -> Gathered {
        let (mut io, _) = scripted(false, "");
        load(&sb.env, &mut io, Some(sb.claude.clone()), "7d")
            .unwrap()
            .data
    }

    /// A transcript with one message of `input` tokens on each of the given days ago.
    fn seed_days(sb: &Sandbox, days_ago: &[(i64, u64)]) {
        let now = tokred_core::Clock::now(&super::super::SystemClock).0;
        let text: Vec<String> = days_ago
            .iter()
            .enumerate()
            .map(|(i, (ago, input))| {
                let day = crate::view::ymd(now - ago * 86_400);
                format!(
                    r#"{{"type":"assistant","sessionId":"s{i}","timestamp":"{day}T12:00:00.000Z","requestId":"r{i}","cwd":"/p","isSidechain":false,"message":{{"id":"m{i}","model":"claude-opus-4-1-20250805","usage":{{"input_tokens":{input},"output_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#
                )
            })
            .collect();
        let dir = sb.claude.join("projects/-tmp-proj");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s.jsonl"), format!("{}\n", text.join("\n"))).unwrap();
    }

    #[test]
    fn the_summary_is_also_kept_for_the_last_7_and_30_days() {
        let sb = Sandbox::new();
        sb.configure();
        // 50 days ago (only in "all"), 20 days ago (30 days and "all"), 2 days ago (every view).
        seed_days(&sb, &[(50, 1_000_000), (20, 2_000_000), (2, 4_000_000)]);
        let (mut io, _) = scripted(false, "");
        let l = load(&sb.env, &mut io, Some(sb.claude.clone()), "90d")
            .unwrap()
            .data;
        let stats = model_stats(&l, &l.prices);
        let latency = latency_stats(&l.events, l.spawn_overhead_us, l.range);
        let data = build(&l, &stats, &latency, None, None);

        let cost = |s: &Option<Summary>| s.as_ref().unwrap().real_cost_usd;
        let (all, month, week) = (
            cost(&data.summary),
            cost(&data.summary_30d),
            cost(&data.summary_7d),
        );
        assert!(
            week > 0.0 && week < month && month < all,
            "{week} < {month} < {all}"
        );
        // Opus input costs the same per token: the three messages weigh 4, 6 and 7 million.
        assert!((month / week - 6.0 / 4.0).abs() < 0.01, "{month} / {week}");
        assert!((all / week - 7.0 / 4.0).abs() < 0.01, "{all} / {week}");
        // Each view says which days it covers; the charts' days are the same ones.
        let since = |s: &Option<Summary>| s.as_ref().unwrap().since.clone().unwrap();
        assert_eq!(
            since(&data.summary),
            data.meta.period_start.clone().unwrap()
        );
        let n = data.cost_per_day.len();
        assert_eq!(since(&data.summary_7d), data.cost_per_day[n - 7].date);
        assert_eq!(since(&data.summary_30d), data.cost_per_day[n - 30].date);
    }

    #[test]
    fn a_period_shorter_than_a_view_makes_that_view_the_whole_period() {
        let sb = Sandbox::new();
        sb.configure();
        seed_days(&sb, &[(2, 1_000_000), (1, 3_000_000)]);
        let (mut io, _) = scripted(false, "");
        let l = load(&sb.env, &mut io, Some(sb.claude.clone()), "3d")
            .unwrap()
            .data;
        let stats = model_stats(&l, &l.prices);
        let latency = latency_stats(&l.events, l.spawn_overhead_us, l.range);
        let data = build(&l, &stats, &latency, None, None);
        let (all, week, month) = (
            data.summary.unwrap(),
            data.summary_7d.unwrap(),
            data.summary_30d.unwrap(),
        );
        assert_eq!(week.real_cost_usd, all.real_cost_usd);
        assert_eq!(month.real_cost_usd, all.real_cost_usd);
        assert_eq!(
            week.since, all.since,
            "the views start where the period does"
        );
        // Without usage there is nothing in any view.
        let empty = Sandbox::new();
        empty.configure();
        let l = loaded(&empty);
        let stats = model_stats(&l, &l.prices);
        let latency = latency_stats(&l.events, l.spawn_overhead_us, l.range);
        let data = build(&l, &stats, &latency, None, None);
        assert!(data.summary.is_none() && data.summary_7d.is_none() && data.summary_30d.is_none());
    }

    #[test]
    fn a_full_report_maps_every_section() {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        sb.seed_events();
        sb.store()
            .meta_set("bench_spawn_overhead_us", "6000")
            .unwrap();
        let l = loaded(&sb);
        let stats = model_stats(&l, &l.prices);
        let latency = latency_stats(&l.events, l.spawn_overhead_us, l.range);
        let audit_report = super::super::audit::build_report(&sb.env, &sb.claude, sb.path("proj"));
        let data = build(&l, &stats, &latency, Some(&audit_report), Some(l.now.0));

        assert_eq!(data.meta.tokred_version, env!("CARGO_PKG_VERSION"));
        assert!(data.meta.last_price_sync.is_some() && data.meta.period_start.is_some());
        assert!(
            data.summary
                .as_ref()
                .is_some_and(|s| s.real_cost_usd > 0.0 && s.tokens_avoided > 0)
        );
        assert_eq!(
            data.cost_per_day.len(),
            8,
            "one entry per day of the period, gap-filled"
        );
        assert!(
            data.cost_per_day
                .iter()
                .any(|d| d.real_usd > 0.0 && d.all_opus_usd > 0.0)
        );
        assert!(data.models.iter().any(|m| m.family == ModelFamily::Opus));
        assert!(data.models.iter().any(|m| m.family == ModelFamily::Sonnet));
        let routing = data.routing.unwrap();
        assert_eq!(
            (routing.delegations_correct, routing.delegations_corrected),
            (1, 1)
        );
        assert_eq!(routing.classifications[0].task_type, "implementation");
        assert!(
            data.guards
                .iter()
                .any(|g| matches!(g.reason, GuardReason::Reread) && g.blocks == 1)
        );
        assert!(
            data.guards
                .iter()
                .any(|g| matches!(g.reason, GuardReason::BroadSearch))
        );
        assert!(!data.cache_per_day.is_empty());
        let lat = data.latency.unwrap();
        assert_eq!(lat.spawn_overhead_ms, Some(6.0));
        assert!(lat.hooks.iter().any(|h| h.hook == "guard-read"));
        assert!(data.audit.is_some());
    }

    #[test]
    fn an_empty_database_yields_empty_sections() {
        let sb = Sandbox::new();
        sb.configure();
        let l = loaded(&sb);
        let stats = model_stats(&l, &l.prices);
        let latency = latency_stats(&l.events, 0, l.range);
        let data = build(&l, &stats, &latency, None, None);
        assert!(
            data.summary.is_none()
                && data.routing.is_none()
                && data.latency.is_none()
                && data.audit.is_none()
        );
        assert!(data.cost_per_day.is_empty() && data.guards.is_empty() && data.models.is_empty());
        assert!(data.meta.last_price_sync.is_none());
    }

    #[test]
    fn the_helpers_classify_models_and_guard_reasons() {
        assert_eq!(model_family("claude-haiku-4"), ModelFamily::Haiku);
        assert_eq!(model_family("claude-fable-5-1"), ModelFamily::Fable);
        assert_eq!(model_family("claude-mythos-5"), ModelFamily::Mythos);
        assert_eq!(model_family("mystery"), ModelFamily::Other);
        assert!(guard_reason("generated_file").is_some() && guard_reason("large_file").is_some());
        assert!(guard_reason("nope").is_none());
    }

    #[test]
    fn the_audit_section_lists_heavy_items_and_unused_servers() {
        use tokred_audit::report::{AuditReport, McpScope, McpServerReport, Summary};
        let server = |name: &str, unused: bool, disabled: bool| McpServerReport {
            name: name.to_owned(),
            scope: McpScope::User,
            also_in: vec![],
            source: PathBuf::from("/x"),
            transport: "stdio".to_owned(),
            disabled,
            calls: 0,
            last_seen_unix: None,
            unused,
            action: None,
        };
        let report = AuditReport {
            token_estimate: "chars/4".to_owned(),
            window_days: 30,
            items: vec![],
            mcp_servers: vec![
                server("idle", true, false),
                server("off", true, true),
                server("busy", false, false),
            ],
            mcp_usage_without_local_config: vec![],
            summary: Summary {
                by_kind: std::collections::BTreeMap::default(),
                total_estimated_tokens: 0,
                total_startup_tokens: 0,
                mcp_servers: 3,
                mcp_servers_unused: 2,
            },
            notes: vec![],
            warnings: vec![],
        };
        let stats = audit_of(&report);
        assert_eq!(
            stats.unused_mcp,
            vec!["idle".to_owned()],
            "disabled servers are not 'unused'"
        );
        assert_eq!(stats.unused_window_days, Some(30));
    }
    #[test]
    fn the_proxy_maps_into_the_report_and_is_left_out_without_it() {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        let l = loaded(&sb);
        let stats = model_stats(&l, &l.prices);
        let latency = latency_stats(&l.events, 0, l.range);
        assert!(build(&l, &stats, &latency, None, None).proxy.is_none());

        sb.seed_proxy();
        let l = loaded(&sb);
        let stats = model_stats(&l, &l.prices);
        let proxy = build(&l, &stats, &latency, None, None).proxy.unwrap();
        assert_eq!(
            (proxy.measured_messages, proxy.transcript_only_messages),
            (3, 0)
        );
        assert_eq!(proxy.routes[0].effective, "claude-haiku-4-5");
        let routing = proxy.routing.unwrap();
        assert_eq!((routing.applied, routing.observed), (1, 1));
        assert!((routing.applied_saving_usd - 0.6).abs() < 1e-9);
        assert!((proxy.overhead.unwrap().p95_ms - 0.8).abs() < 1e-9);
        assert_eq!(proxy.main_requests, 1);
        assert_eq!(proxy.cache_misses[0].cause, "ttl_expired");
        assert_eq!(proxy.cache_misses[0].cache_write_tokens, 100);
        let origins: Vec<_> = proxy.by_origin.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(origins.len(), 3);
        assert!(origins.contains(&"main") && origins.contains(&"subagent"));
        assert!(
            proxy
                .by_origin
                .windows(2)
                .all(|w| w[0].cost_usd >= w[1].cost_usd),
            "most expensive first"
        );
        assert!(proxy.by_agent_type.iter().any(|a| a.name == "Explore"));
        assert!(proxy.suggestion.unwrap().contains("simple"));

        // Only a pending suggestion: no requests, but something waits to be read.
        let quiet = Sandbox::new();
        quiet.configure();
        quiet.seed_proxy();
        quiet.store().prune_proxy_requests(l.now, 0).unwrap();
        let l = loaded(&quiet);
        let proxy = super::proxy_of(&l).unwrap();
        assert_eq!(proxy.main_requests, 0);
        assert!(proxy.routes.is_empty() && proxy.overhead.is_none());
        assert!(proxy.cache_misses.is_empty() && proxy.suggestion.is_some());
    }

    #[test]
    fn the_report_is_built_from_the_repositories_alone() {
        use super::super::data::Repos;

        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        sb.seed_proxy();
        sb.store()
            .meta_set("price_synced_at", "1700000000")
            .unwrap();
        let store = sb.store();
        let (mut io, _) = scripted(false, "");
        let l = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();

        let html = report_html(
            &Repos::of(&store),
            l.data.prices.clone(),
            l.data.now,
            "7d",
            None,
        )
        .unwrap();
        assert!(html.contains("proxy-routing") && html.contains("claude-haiku-4-5"));
        assert!(
            html.contains("2023-11-14"),
            "the last price sync is read from the repository"
        );

        let err = report_html(
            &Repos::of(&store),
            l.data.prices.clone(),
            l.data.now,
            "soon",
            None,
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("invalid --since"), "{err}");
    }
}
