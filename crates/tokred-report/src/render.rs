//! Assembly of the full HTML page from the embedded template and the sections.

use crate::escape::{self, html};
use crate::model::{ReportData, ReportMeta};
use crate::{proxy_sections, sections};

/// The page skeleton, with `{{CSS}}`, `{{BODY}}`, `{{DATA}}` and `{{JS}}` placeholders.
const TEMPLATE: &str = include_str!("../templates/report.html");
/// Inline stylesheet.
const CSS: &str = include_str!("../templates/report.css");
/// Inline script (period toggle and tooltips).
const JS: &str = include_str!("../templates/report.js");

/// Renders the complete, self-contained and offline HTML report.
///
/// The output has inline CSS and JS, SVG charts, and the model embedded as
/// `<script type="application/json" id="data">`. It never references an external
/// resource.
#[must_use]
pub fn render_html(data: &ReportData) -> String {
    let json = serde_json::to_string(data).unwrap_or_default();
    fill(
        TEMPLATE,
        &[
            ("CSS", CSS),
            ("JS", JS),
            ("DATA", &escape::json_for_script(&json)),
            ("BODY", &body(data)),
        ],
    )
}

/// Replaces `{{KEY}}` placeholders in a single left-to-right pass.
///
/// Substituted values are never scanned again, so a value that itself contains
/// `{{BODY}}` (for example a project name) stays literal. Unknown or unterminated
/// placeholders are copied unchanged.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            rest = &rest[start..];
            break;
        };
        match values.iter().find(|(key, _)| *key == &after[..end]) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[start..start + end + 4]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// The `<header>`, the eight sections, the proxy's when it recorded anything, and the `<footer>`.
fn body(data: &ReportData) -> String {
    let toolbar = if sections::has_daily_series(data) {
        "<div class=\"toolbar\" id=\"period-toolbar\" role=\"group\" \
         aria-label=\"Period of the summary and the daily charts\" hidden>Summary and daily charts:\
         <button type=\"button\" data-period=\"7\" aria-pressed=\"false\">7 days</button>\
         <button type=\"button\" data-period=\"30\" aria-pressed=\"false\">30 days</button>\
         <button type=\"button\" data-period=\"all\" aria-pressed=\"true\">All</button></div>"
    } else {
        ""
    };
    format!(
        "<header><div><h1>tokred metrics</h1><p class=\"sub\">Period: {}</p></div>{toolbar}</header>\
         <main>{}{}{}{}{}{}{}{}{}</main>{}",
        html(&period(&data.meta)),
        sections::summary(data),
        sections::cost(&data.cost_per_day),
        sections::models(&data.models),
        sections::routing(data.routing.as_ref()),
        sections::guards(&data.guards),
        sections::cache(&data.cache_per_day),
        sections::latency(data.latency.as_ref()),
        sections::audit(data.audit.as_ref()),
        proxy_sections::proxy(data.proxy.as_ref()),
        footer(&data.meta),
    )
}

/// Human-readable period covered.
fn period(meta: &ReportMeta) -> String {
    match (&meta.period_start, &meta.period_end) {
        (Some(start), Some(end)) => format!("{start} to {end}"),
        (Some(start), None) => format!("since {start}"),
        (None, Some(end)) => format!("until {end}"),
        (None, None) => "not recorded".to_owned(),
    }
}

/// The footer: generation date, version, period and last price sync.
fn footer(meta: &ReportMeta) -> String {
    let generated = non_empty(&meta.generated_at, "unknown");
    let version = non_empty(&meta.tokred_version, "unknown");
    let sync = meta.last_price_sync.as_deref().unwrap_or("never");
    format!(
        "<footer><p>Generated {} &middot; tokred v{} &middot; Period: {} &middot; \
         Last price sync: {}</p>\
         <p>Figures marked <span class=\"est\">estimate</span> are derived (sizes / 4, \
         projections), not measured. This file is self-contained and makes no network requests.</p>\
         </footer>",
        html(generated),
        html(version),
        html(&period(meta)),
        html(sync),
    )
}

/// `text`, or `fallback` when it is blank.
fn non_empty<'a>(text: &'a str, fallback: &'a str) -> &'a str {
    if text.trim().is_empty() {
        fallback
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AuditItem, AuditStats, DayCache, DayCost, GuardStat, HookLatency, LatencyStats,
        ModelFamily, ModelUsage, RoutingStats, Summary, TaskTypeCount,
    };
    use crate::testutil::assert_well_formed;

    const HOSTILE: &str = "<script>alert(1)</script>";

    fn hostile_data() -> ReportData {
        ReportData {
            meta: ReportMeta {
                generated_at: HOSTILE.to_owned(),
                tokred_version: HOSTILE.to_owned(),
                ..ReportMeta::default()
            },
            summary: Some(Summary::default()),
            summary_7d: None,
            summary_30d: None,
            cost_per_day: vec![DayCost {
                date: HOSTILE.to_owned(),
                real_usd: 1.0,
                all_opus_usd: 2.0,
            }],
            models: vec![ModelUsage {
                name: HOSTILE.to_owned(),
                family: ModelFamily::Other,
                tokens: 10,
                cost_usd: 1.0,
            }],
            routing: Some(RoutingStats {
                classifications: vec![TaskTypeCount {
                    task_type: HOSTILE.to_owned(),
                    count: 3,
                }],
                ..RoutingStats::default()
            }),
            guards: vec![GuardStat::default()],
            cache_per_day: vec![DayCache {
                date: HOSTILE.to_owned(),
                hit_rate: 0.5,
            }],
            latency: Some(LatencyStats {
                hooks: vec![HookLatency {
                    hook: HOSTILE.to_owned(),
                    ..HookLatency::default()
                }],
                ..LatencyStats::default()
            }),
            audit: Some(AuditStats {
                heaviest: vec![AuditItem {
                    name: HOSTILE.to_owned(),
                    kind: HOSTILE.to_owned(),
                    origin: format!("project {HOSTILE}"),
                    tokens: 5,
                }],
                unused_mcp: vec![HOSTILE.to_owned()],
                unused_window_days: Some(30),
            }),
            proxy: None,
        }
    }

    #[test]
    fn fill_substitutes_in_one_pass() {
        let out = fill("a {{X}} b {{Y}} c", &[("X", "{{Y}}"), ("Y", "y")]);
        assert_eq!(out, "a {{Y}} b y c");
    }

    #[test]
    fn fill_keeps_unknown_and_unterminated_placeholders() {
        assert_eq!(fill("{{NOPE}} x", &[]), "{{NOPE}} x");
        assert_eq!(fill("a {{OPEN", &[("OPEN", "z")]), "a {{OPEN");
        assert_eq!(fill("plain", &[]), "plain");
    }

    #[test]
    fn user_text_that_looks_like_a_placeholder_stays_literal() {
        let data = ReportData {
            models: vec![ModelUsage {
                name: "{{JS}}{{BODY}}".to_owned(),
                ..ModelUsage::default()
            }],
            ..ReportData::default()
        };
        let page = render_html(&data);
        assert!(page.contains("{{JS}}{{BODY}}"));
        assert_eq!(page.matches("<body").count(), 1);
    }

    #[test]
    fn untrusted_text_is_escaped_everywhere() {
        let page = render_html(&hostile_data());
        assert_well_formed(&page).unwrap();
        // The only `<script>` elements are the two the page owns.
        assert_eq!(page.matches("<script").count(), 2);
        assert_eq!(page.matches("</script>").count(), 2);
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!page.contains(HOSTILE));
    }

    #[test]
    fn the_embedded_json_block_is_safe_and_parses_back() {
        let data = hostile_data();
        let page = render_html(&data);
        let marker = "<script type=\"application/json\" id=\"data\">";
        let start = page.find(marker).unwrap() + marker.len();
        let end = start + page[start..].find("</script>").unwrap();
        let block = &page[start..end];
        assert!(!block.contains('<'));
        let parsed: ReportData = serde_json::from_str(block).unwrap();
        assert_eq!(parsed, data);
    }

    #[test]
    fn the_page_is_offline() {
        let page = render_html(&hostile_data());
        for needle in [
            "http://", "https://", "//cdn", "@import", "url(", "src=", "href=", "<link", "<img",
            "<iframe",
        ] {
            assert!(!page.contains(needle), "found `{needle}`");
        }
        assert!(!page.contains("fetch("));
        assert!(!page.contains("XMLHttpRequest"));
    }

    #[test]
    fn the_empty_report_has_eight_no_data_sections() {
        let page = render_html(&ReportData::default());
        assert_well_formed(&page).unwrap();
        for id in [
            "summary", "cost", "models", "routing", "guards", "cache", "latency", "audit",
        ] {
            assert!(
                page.contains(&format!("<section id=\"{id}\"")),
                "missing {id}"
            );
        }
        assert_eq!(page.matches("No data yet.").count(), 8);
        assert!(!page.contains("id=\"period-toolbar\""));
    }

    #[test]
    fn the_footer_has_fallbacks_and_real_values() {
        let empty = footer(&ReportMeta::default());
        assert!(empty.contains("Generated unknown"));
        assert!(empty.contains("tokred vunknown"));
        assert!(empty.contains("Last price sync: never"));
        assert!(empty.contains("Period: not recorded"));
        let meta = ReportMeta {
            generated_at: "2026-10-28 14:03 UTC".to_owned(),
            tokred_version: "0.1.0".to_owned(),
            period_start: Some("2026-10-01".to_owned()),
            period_end: Some("2026-10-28".to_owned()),
            last_price_sync: Some("2026-10-27".to_owned()),
        };
        let full = footer(&meta);
        assert!(full.contains("Generated 2026-10-28 14:03 UTC"));
        assert!(full.contains("tokred v0.1.0"));
        assert!(full.contains("Period: 2026-10-01 to 2026-10-28"));
        assert!(full.contains("Last price sync: 2026-10-27"));
    }

    #[test]
    fn period_describes_partial_ranges() {
        let only = |start: Option<&str>, end: Option<&str>| {
            period(&ReportMeta {
                period_start: start.map(str::to_owned),
                period_end: end.map(str::to_owned),
                ..ReportMeta::default()
            })
        };
        assert_eq!(only(Some("a"), None), "since a");
        assert_eq!(only(None, Some("b")), "until b");
    }

    #[test]
    fn daily_series_enable_the_period_toolbar() {
        let page = render_html(&hostile_data());
        assert!(page.contains("id=\"period-toolbar\""));
        assert_eq!(page.matches("<div class=\"pane\" data-pane=").count(), 6);
        // Ids must stay unique across the period panes.
        assert_eq!(page.matches("id=\"cost-7-title\"").count(), 1);
        assert_eq!(page.matches("id=\"cost-all-title\"").count(), 1);
    }
}
