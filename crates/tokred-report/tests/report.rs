#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Snapshot and offline checks of the rendered report through the public API.

use tokred_report::{
    AuditItem, AuditStats, DayCache, DayCost, GuardReason, GuardStat, HookLatency, LatencyStats,
    ModelFamily, ModelUsage, ReportData, ReportMeta, RoutingStats, Summary, TaskTypeCount,
    render_html,
};

/// 35 consecutive days ending on 2026-10-31.
fn dates() -> Vec<String> {
    (0..35_u32)
        .map(|offset| {
            if offset < 4 {
                format!("2026-09-{:02}", 27 + offset)
            } else {
                format!("2026-10-{:02}", offset - 3)
            }
        })
        .collect()
}

/// A rich, deterministic fixture that exercises every section.
fn rich() -> ReportData {
    let cost_per_day = dates()
        .into_iter()
        .zip(0_u32..)
        .map(|(date, i)| {
            let base = 2.0 + f64::from(i % 7);
            DayCost {
                date,
                real_usd: base,
                all_opus_usd: base * 2.4,
            }
        })
        .collect();
    let cache_per_day = dates()
        .into_iter()
        .zip(0_u32..)
        .map(|(date, i)| DayCache {
            date,
            hit_rate: 0.6 + f64::from(i % 5) / 20.0,
        })
        .collect();
    ReportData {
        meta: ReportMeta {
            generated_at: "2026-10-31 12:00 UTC".to_owned(),
            tokred_version: "0.1.0".to_owned(),
            period_start: Some("2026-09-27".to_owned()),
            period_end: Some("2026-10-31".to_owned()),
            last_price_sync: Some("2026-10-30".to_owned()),
        },
        summary: Some(Summary {
            real_cost_usd: 123.456,
            routing_saving_usd: 210.5,
            guard_saving_usd: 12.3456,
            tokens_avoided: 1_234_567,
            monthly_projection_usd: 135.0,
            latency_added_ms: 4_321.5,
        }),
        cost_per_day,
        models: models(),
        routing: Some(RoutingStats {
            classifications: vec![
                TaskTypeCount {
                    task_type: "explore".to_owned(),
                    count: 120,
                },
                TaskTypeCount {
                    task_type: "edit".to_owned(),
                    count: 75,
                },
                TaskTypeCount {
                    task_type: "plan".to_owned(),
                    count: 12,
                },
            ],
            delegations_correct: 90,
            delegations_corrected: 10,
        }),
        guards: guards(),
        cache_per_day,
        latency: Some(latency()),
        audit: Some(AuditStats {
            heaviest: vec![
                AuditItem {
                    name: "CLAUDE.md".to_owned(),
                    kind: "memory".to_owned(),
                    origin: "user".to_owned(),
                    tokens: 4_200,
                },
                AuditItem {
                    name: "skills/deploy".to_owned(),
                    kind: "skill".to_owned(),
                    origin: "project billing".to_owned(),
                    tokens: 1_900,
                },
            ],
            unused_mcp: vec!["figma".to_owned(), "postgres".to_owned()],
            unused_window_days: Some(30),
        }),
    }
}

fn models() -> Vec<ModelUsage> {
    let model = |name: &str, family, tokens, cost_usd| ModelUsage {
        name: name.to_owned(),
        family,
        tokens,
        cost_usd,
    };
    vec![
        model("claude-opus-4", ModelFamily::Opus, 2_000_000, 80.0),
        model("claude-sonnet-4", ModelFamily::Sonnet, 9_500_000, 40.0),
        model("claude-haiku-4", ModelFamily::Haiku, 4_000_000, 3.0),
        model("<unknown-model>", ModelFamily::Other, 10_000, 0.456),
    ]
}

fn guards() -> Vec<GuardStat> {
    let guard = |reason, blocks, bytes_avoided: u64| GuardStat {
        reason,
        blocks,
        bytes_avoided,
        tokens_avoided: bytes_avoided / 4,
    };
    vec![
        guard(GuardReason::Reread, 40, 2_000_000),
        guard(GuardReason::GeneratedFile, 12, 3_000_000),
        guard(GuardReason::LargeFile, 5, 900_000),
        guard(GuardReason::BroadSearch, 9, 270_000),
    ]
}

fn latency() -> LatencyStats {
    let hook = |hook: &str, runs, p50_ms, p95_ms, max_ms| HookLatency {
        hook: hook.to_owned(),
        runs,
        p50_ms,
        p95_ms,
        max_ms,
    };
    LatencyStats {
        budget_ms: 50.0,
        hooks: vec![
            hook("guard-read", 1_500, 3.2, 9.8, 41.0),
            hook("route", 300, 12.0, 58.5, 120.0),
        ],
        total_added_ms: 4_321.5,
        saved_usd: 222.85,
        saved_tokens: 1_234_567,
        spawn_overhead_ms: Some(6.5),
    }
}

#[test]
fn rich_report_snapshot() {
    insta::assert_snapshot!(render_html(&rich()));
}

#[test]
fn empty_report_snapshot() {
    insta::assert_snapshot!(render_html(&ReportData::default()));
}

#[test]
fn rich_report_is_offline_and_labels_estimates() {
    let page = render_html(&rich());
    for needle in ["http://", "https://", "@import", "src=", "href=", "url("] {
        assert!(!page.contains(needle), "found `{needle}`");
    }
    assert!(page.contains("class=\"est\""));
    assert!(page.contains("prefers-color-scheme: dark"));
    assert!(page.contains("p95 above budget"));
    assert!(page.contains("No tool call in the last 30 days"));
    assert!(!page.contains("No data yet."));
}

#[test]
fn rendering_is_deterministic() {
    assert_eq!(render_html(&rich()), render_html(&rich()));
}
