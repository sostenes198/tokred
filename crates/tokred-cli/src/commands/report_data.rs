//! Maps the metrics results into the input of the HTML report.

use tokred_audit::AuditReport;
use tokred_core::Timestamp;
use tokred_core::hook_event::reason;
use tokred_core::pricing::{Family, PriceTable};
use tokred_metrics::{
    GroupBy, LatencyStats, Stats, StatsInput, TimeRange, compute_stats, guards_summary,
    routing_summary,
};
use tokred_report::model::{
    AuditItem, AuditStats, DayCache, DayCost, GuardReason, GuardStat, HookLatency,
    LatencyStats as ReportLatency, ModelFamily, ModelUsage, ReportData, ReportMeta, RoutingStats,
    Summary, TaskTypeCount,
};

use super::data::Loaded;
use crate::view::ymd;

const DAY: i64 = 86_400;

/// One entry per calendar day (UTC) of `range`, with the real cost and the "all on opus" cost.
fn daily_costs(loaded: &Loaded) -> Vec<DayCost> {
    let first = loaded.range.from.0.div_euclid(DAY);
    let last = (loaded.range.to.0 - 1).div_euclid(DAY);
    (first..=last)
        .map(|day| {
            let range = TimeRange::new(Timestamp(day * DAY), Timestamp((day + 1) * DAY));
            let slice: Vec<_> = loaded
                .usage
                .iter()
                .filter(|u| range.contains(u.ts))
                .cloned()
                .collect();
            let stats = compute_stats(&StatsInput {
                usage: &slice,
                prices: &loaded.prices,
                events: &[],
                range,
                group_by: GroupBy::Model,
                now: loaded.now,
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
pub fn model_stats(loaded: &Loaded, prices: &PriceTable) -> Stats {
    compute_stats(&StatsInput {
        usage: &loaded.usage,
        prices,
        events: &loaded.events,
        range: loaded.range,
        group_by: GroupBy::Model,
        now: loaded.now,
        first_configure: loaded.first_configure,
    })
}

fn summary_of(stats: &Stats, latency: &LatencyStats) -> Option<Summary> {
    (stats.totals.messages > 0).then(|| Summary {
        real_cost_usd: stats.savings.real_cost_usd,
        routing_saving_usd: stats.savings.routing_saving_usd.unwrap_or(0.0),
        guard_saving_usd: stats.savings.guard.saving_usd,
        tokens_avoided: stats.savings.guard.tokens_avoided,
        monthly_projection_usd: stats
            .projection
            .as_ref()
            .map_or(0.0, |p| p.last_7d.projected_30d_usd),
        latency_added_ms: latency.total_ms,
    })
}

fn routing_of(loaded: &Loaded) -> Option<RoutingStats> {
    let routing = routing_summary(&loaded.events, loaded.range);
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

fn guards_of(loaded: &Loaded) -> Vec<GuardStat> {
    guards_summary(&loaded.events, loaded.range)
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

fn latency_of(loaded: &Loaded, stats: &Stats, latency: &LatencyStats) -> Option<ReportLatency> {
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
        saved_usd: stats.savings.total_saving_usd.unwrap_or(0.0),
        saved_tokens: stats.savings.guard.tokens_avoided,
        spawn_overhead_ms: (loaded.spawn_overhead_us > 0)
            .then(|| u64_to_f64(loaded.spawn_overhead_us) / 1_000.0),
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
    loaded: &Loaded,
    stats: &Stats,
    latency: &LatencyStats,
    audit: Option<&AuditReport>,
    last_price_sync: Option<i64>,
) -> ReportData {
    let has_usage = stats.totals.messages > 0;
    ReportData {
        meta: ReportMeta {
            generated_at: ymd(loaded.now.0),
            tokred_version: env!("CARGO_PKG_VERSION").to_owned(),
            period_start: Some(ymd(loaded.range.from.0)),
            period_end: Some(ymd(loaded.range.to.0 - 1)),
            last_price_sync: last_price_sync.map(ymd),
        },
        summary: summary_of(stats, latency),
        cost_per_day: if has_usage {
            daily_costs(loaded)
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
        routing: routing_of(loaded),
        guards: guards_of(loaded),
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
        latency: latency_of(loaded, stats, latency),
        audit: audit.map(audit_of),
    }
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

    fn loaded(sb: &Sandbox) -> Loaded {
        let (mut io, _) = scripted(false, "");
        load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap()
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
}
