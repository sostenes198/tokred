//! The eight report sections, each rendered to an HTML fragment.
//!
//! Every function here is pure: it takes (part of) the model and returns a `String`.
//! A section without data renders a "no data yet" notice instead of disappearing.

use crate::escape::html;
use crate::format;
use crate::model::{
    AuditStats, DayCache, DayCost, GuardReason, GuardStat, LatencyStats, ModelFamily, ModelUsage,
    ReportData, RoutingStats, Summary,
};
use crate::svg::{
    HBarSpec, LineSpec, Reference, Series, Slice, VBarSpec, donut, hbar_chart, index_f64, legend,
    line_chart, vbar_chart,
};

/// Latency budget used when the model carries none (milliseconds).
const DEFAULT_BUDGET_MS: f64 = 50.0;

/// The period views: pane key and how many trailing days it shows.
const PERIODS: [(&str, Option<usize>); 3] = [("7", Some(7)), ("30", Some(30)), ("all", None)];

/// Colour class indices (see the CSS).
const OPUS: usize = 0;
const SONNET: usize = 1;
const HAIKU: usize = 2;
const OTHER: usize = 3;
const FABLE: usize = 4;
const MYTHOS: usize = 5;
const ACCENT: usize = 4;

/// The badge that marks a figure as an estimate.
const ESTIMATE: &str = "<span class=\"est\" title=\"Estimate: derived from sizes or projections, \
                        not measured\">estimate</span>";

/// Wraps content in a titled section; `estimate` adds the estimate badge to the heading.
fn section(id: &str, title: &str, estimate: bool, content: &str) -> String {
    let badge = if estimate {
        format!(" {ESTIMATE}")
    } else {
        String::new()
    };
    format!(
        "<section id=\"{id}\" aria-labelledby=\"{id}-heading\">\
         <h2 id=\"{id}-heading\">{}{badge}</h2>{content}</section>",
        html(title),
    )
}

/// The notice shown by a section that has nothing to display.
fn empty(hint: &str) -> String {
    format!("<p class=\"empty\">No data yet. {}</p>", html(hint))
}

/// Last `limit` items of a series (all of them when `limit` is `None`).
fn last<T>(items: &[T], limit: Option<usize>) -> &[T] {
    match limit {
        Some(limit) => &items[items.len().saturating_sub(limit)..],
        None => items,
    }
}

/// Renders `render` once per period view, wrapped in panes the toolbar can switch.
fn panes(render: &dyn Fn(Option<usize>, &str) -> String) -> String {
    PERIODS
        .iter()
        .map(|(key, limit)| {
            format!(
                "<div class=\"pane\" data-pane=\"{key}\">{}</div>",
                render(*limit, key)
            )
        })
        .collect()
}

/// Whether the model has any daily series (the period toolbar is only useful then).
#[must_use]
pub fn has_daily_series(data: &ReportData) -> bool {
    !data.cost_per_day.is_empty() || !data.cache_per_day.is_empty()
}

/// Section 1: summary cards.
#[must_use]
pub fn summary(summary: Option<&Summary>) -> String {
    let content = summary.map_or_else(
        || empty("The cards fill in once tokred has read your first transcripts."),
        |s| {
            let saving = s.routing_saving_usd + s.guard_saving_usd;
            let cards = [
                card(
                    "Real cost",
                    &format::usd(s.real_cost_usd),
                    "over the period",
                    false,
                ),
                card(
                    "Total saving",
                    &format::usd(saving),
                    &format!(
                        "routing {} + guards {}",
                        format::usd(s.routing_saving_usd),
                        format::usd(s.guard_saving_usd)
                    ),
                    true,
                ),
                card(
                    "Tokens avoided",
                    &format::compact(s.tokens_avoided),
                    &format!(
                        "{} tokens, by the guards",
                        format::integer(s.tokens_avoided)
                    ),
                    true,
                ),
                card(
                    "Monthly projection",
                    &format::usd(s.monthly_projection_usd),
                    "at the recent daily average",
                    true,
                ),
                card(
                    "Latency added",
                    &format::ms(s.latency_added_ms),
                    "by all hooks, in total",
                    false,
                ),
            ];
            format!("<div class=\"cards\">{}</div>", cards.concat())
        },
    );
    section("summary", "Summary", false, &content)
}

/// One summary card.
fn card(label: &str, value: &str, note: &str, estimate: bool) -> String {
    let badge = if estimate {
        format!(" {ESTIMATE}")
    } else {
        String::new()
    };
    format!(
        "<div class=\"card\"><div class=\"k\">{}{badge}</div><div class=\"v\">{}</div>\
         <div class=\"n\">{}</div></div>",
        html(label),
        html(value),
        html(note),
    )
}

/// Section 2: cost per day, real versus "everything on opus".
#[must_use]
pub fn cost(days: &[DayCost]) -> String {
    let content = if days.is_empty() {
        empty("The daily cost chart fills in once there are priced sessions.")
    } else {
        panes(&|limit, key| cost_pane(last(days, limit), key))
    };
    section("cost", "Cost per day", false, &content)
}

/// One period view of the cost chart.
fn cost_pane(days: &[DayCost], key: &str) -> String {
    let labels: Vec<String> = days.iter().map(|d| d.date.clone()).collect();
    let real: Vec<f64> = days.iter().map(|d| d.real_usd).collect();
    let opus: Vec<f64> = days.iter().map(|d| d.all_opus_usd).collect();
    let series = [
        Series {
            name: "Real cost",
            values: &real,
            style: ACCENT,
            dashed: false,
        },
        Series {
            name: "Everything on opus",
            values: &opus,
            style: OTHER,
            dashed: true,
        },
    ];
    let chart = line_chart(&LineSpec {
        id: &format!("cost-{key}"),
        title: "Cost per day: real cost versus everything on opus",
        labels: &labels,
        series: &series,
        y_max: None,
        format: format::usd,
    });
    let real_total: f64 = real.iter().sum();
    let opus_total: f64 = opus.iter().sum();
    format!(
        "{chart}{}<p class=\"note\">Shown period: real {} versus {} if everything ran on opus \
         (difference {}).</p>",
        legend(&[
            ("Real cost", ACCENT, false),
            ("Everything on opus", OTHER, true)
        ]),
        format::usd(real_total),
        format::usd(opus_total),
        format::usd((opus_total - real_total).max(0.0)),
    )
}

/// Display name and colour of a model family.
const fn family(family: ModelFamily) -> (&'static str, usize) {
    match family {
        ModelFamily::Opus => ("Opus", OPUS),
        ModelFamily::Sonnet => ("Sonnet", SONNET),
        ModelFamily::Haiku => ("Haiku", HAIKU),
        ModelFamily::Fable => ("Fable", FABLE),
        ModelFamily::Mythos => ("Mythos", MYTHOS),
        ModelFamily::Other => ("Other", OTHER),
    }
}

/// Section 3: distribution by model.
#[must_use]
pub fn models(models: &[ModelUsage]) -> String {
    let content = if models.is_empty() {
        empty("The distribution fills in once tokred has priced some usage.")
    } else {
        models_content(models)
    };
    section("models", "Distribution by model", false, &content)
}

/// The donuts and the per-model table.
fn models_content(models: &[ModelUsage]) -> String {
    let families = [
        ModelFamily::Opus,
        ModelFamily::Sonnet,
        ModelFamily::Haiku,
        ModelFamily::Fable,
        ModelFamily::Mythos,
        ModelFamily::Other,
    ];
    let total_tokens: u64 = models.iter().map(|m| m.tokens).sum();
    let total_cost: f64 = models.iter().map(|m| m.cost_usd).sum();
    let mut token_slices = Vec::new();
    let mut cost_slices = Vec::new();
    let mut entries = Vec::new();
    for kind in families {
        let (name, style) = family(kind);
        let of_kind = models.iter().filter(|m| m.family == kind);
        let tokens: u64 = of_kind.clone().map(|m| m.tokens).sum();
        let cost: f64 = of_kind.map(|m| m.cost_usd).sum();
        token_slices.push(Slice {
            label: name.to_owned(),
            value: format::to_f64(tokens),
            style,
            display: format!("{} tokens", format::integer(tokens)),
        });
        cost_slices.push(Slice {
            label: name.to_owned(),
            value: cost,
            style,
            display: format::usd(cost),
        });
        entries.push((name, style, false));
    }
    let rows: String = models
        .iter()
        .map(|m| {
            let (name, style) = family(m.family);
            format!(
                "<tr><td><span class=\"sw s{style}\" aria-hidden=\"true\"></span>{}</td>\
                 <td>{name}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                html(&m.name),
                format::integer(m.tokens),
                format::percent(share(
                    format::to_f64(m.tokens),
                    format::to_f64(total_tokens)
                )),
                format::usd(m.cost_usd),
                format::percent(share(m.cost_usd, total_cost)),
            )
        })
        .collect();
    format!(
        "<div class=\"donuts\"><figure><figcaption>Tokens</figcaption>{}</figure>\
         <figure><figcaption>Cost</figcaption>{}</figure></div>{}\
         <div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">Model</th>\
         <th scope=\"col\">Family</th><th scope=\"col\" class=\"num\">Tokens</th>\
         <th scope=\"col\" class=\"num\">Share</th><th scope=\"col\" class=\"num\">Cost</th>\
         <th scope=\"col\" class=\"num\">Share</th></tr></thead><tbody>{rows}</tbody></table></div>",
        donut(
            "models-tokens",
            "Tokens by model family",
            &token_slices,
            &format::compact(total_tokens),
        ),
        donut(
            "models-cost",
            "Cost by model family",
            &cost_slices,
            &format::usd(total_cost),
        ),
        legend(&entries),
    )
}

/// `part / total`, or `0.0` when the total is not positive.
fn share(part: f64, total: f64) -> f64 {
    if total > 0.0 { part / total } else { 0.0 }
}

/// Section 4: routing and delegation accuracy.
#[must_use]
pub fn routing(routing: Option<&RoutingStats>) -> String {
    let content = match routing {
        Some(r)
            if !r.classifications.is_empty()
                || r.delegations_correct + r.delegations_corrected > 0 =>
        {
            routing_content(r)
        }
        _ => empty("Routing statistics appear once the route and enforce hooks have run."),
    };
    section("routing", "Routing", false, &content)
}

/// Charts and summary text of the routing section.
fn routing_content(r: &RoutingStats) -> String {
    let groups: Vec<(String, Vec<f64>)> = r
        .classifications
        .iter()
        .map(|c| (c.task_type.clone(), vec![format::to_f64(c.count)]))
        .collect();
    let classifications = hbar_chart(&HBarSpec {
        id: "routing-types",
        title: "Classifications per task type",
        series: &[("prompts", SONNET)],
        groups: &groups,
        format: format::integer_f64,
        reference: None,
    });
    let delegations = hbar_chart(&HBarSpec {
        id: "routing-delegations",
        title: "Delegations: correct versus corrected by enforce",
        series: &[("delegations", HAIKU)],
        groups: &[
            (
                "Correct".to_owned(),
                vec![format::to_f64(r.delegations_correct)],
            ),
            (
                "Corrected by enforce".to_owned(),
                vec![format::to_f64(r.delegations_corrected)],
            ),
        ],
        format: format::integer_f64,
        reference: None,
    });
    let total = r.delegations_correct + r.delegations_corrected;
    let rate = share(format::to_f64(r.delegations_correct), format::to_f64(total));
    format!(
        "<h3>Classifications per task type</h3>{classifications}\
         <h3>Delegations</h3>{delegations}\
         <p class=\"note\">{} of {} delegations already used the right model.</p>",
        format::percent(rate),
        format::integer(total),
    )
}

/// Display name of a guard reason.
const fn reason_name(reason: GuardReason) -> &'static str {
    match reason {
        GuardReason::Reread => "Re-read",
        GuardReason::GeneratedFile => "Generated file",
        GuardReason::LargeFile => "Large file",
        GuardReason::BroadSearch => "Broad search",
    }
}

/// Section 5: guard blocks and the bytes and tokens they avoided.
#[must_use]
pub fn guards(guards: &[GuardStat]) -> String {
    let content = if guards.is_empty() {
        empty("Guard statistics appear once the guard hooks have blocked a call.")
    } else {
        guards_content(guards)
    };
    section("guards", "Guards", true, &content)
}

/// Chart and table of the guards section.
fn guards_content(guards: &[GuardStat]) -> String {
    let groups: Vec<(String, Vec<f64>)> = guards
        .iter()
        .map(|g| {
            (
                reason_name(g.reason).to_owned(),
                vec![format::to_f64(g.blocks)],
            )
        })
        .collect();
    let chart = hbar_chart(&HBarSpec {
        id: "guards-blocks",
        title: "Blocked calls per guard reason",
        series: &[("blocks", ACCENT)],
        groups: &groups,
        format: format::integer_f64,
        reference: None,
    });
    let rows: String = guards
        .iter()
        .map(|g| {
            format!(
                "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td></tr>",
                reason_name(g.reason),
                format::integer(g.blocks),
                format::bytes(g.bytes_avoided),
                format::integer(g.tokens_avoided),
            )
        })
        .collect();
    let blocks: u64 = guards.iter().map(|g| g.blocks).sum();
    let bytes: u64 = guards.iter().map(|g| g.bytes_avoided).sum();
    let tokens: u64 = guards.iter().map(|g| g.tokens_avoided).sum();
    format!(
        "{chart}<div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">Reason</th>\
         <th scope=\"col\" class=\"num\">Blocks</th>\
         <th scope=\"col\" class=\"num\">Bytes avoided {ESTIMATE}</th>\
         <th scope=\"col\" class=\"num\">Tokens avoided {ESTIMATE}</th></tr></thead>\
         <tbody>{rows}</tbody><tfoot><tr><th scope=\"row\">Total</th>\
         <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr></tfoot>\
         </table></div>\
         <p class=\"note\">Bytes are what the blocked calls would have loaded; tokens are \
         bytes / 4. Both are estimates.</p>",
        format::integer(blocks),
        format::bytes(bytes),
        format::integer(tokens),
    )
}

/// Section 6: prompt-cache hit rate per day.
#[must_use]
pub fn cache(days: &[DayCache]) -> String {
    let content = if days.is_empty() {
        empty("The cache hit rate fills in once there are priced sessions.")
    } else {
        panes(&|limit, key| cache_pane(last(days, limit), key))
    };
    section("cache", "Prompt cache", false, &content)
}

/// One period view of the cache chart.
fn cache_pane(days: &[DayCache], key: &str) -> String {
    let labels: Vec<String> = days.iter().map(|d| d.date.clone()).collect();
    let rates: Vec<f64> = days.iter().map(|d| d.hit_rate).collect();
    let chart = vbar_chart(&VBarSpec {
        id: &format!("cache-{key}"),
        title: "Prompt-cache hit rate per day",
        labels: &labels,
        values: &rates,
        style: HAIKU,
        y_max: Some(1.0),
        format: format::percent,
    });
    let average = share(rates.iter().sum(), index_f64(rates.len()));
    format!(
        "{chart}<p class=\"note\">Average over the shown period: {}. Hit rate is cache reads \
         divided by all input tokens.</p>",
        format::percent(average),
    )
}

/// Section 7: latency per hook, against the budget.
#[must_use]
pub fn latency(latency: Option<&LatencyStats>) -> String {
    let content = match latency {
        Some(l) if !l.hooks.is_empty() => latency_content(l),
        _ => empty("Latency appears once hooks have run and recorded their duration."),
    };
    section("latency", "Latency", false, &content)
}

/// Chart, table and trade-off text of the latency section.
fn latency_content(l: &LatencyStats) -> String {
    let budget = if l.budget_ms > 0.0 {
        l.budget_ms
    } else {
        DEFAULT_BUDGET_MS
    };
    let groups: Vec<(String, Vec<f64>)> = l
        .hooks
        .iter()
        .map(|h| (h.hook.clone(), vec![h.p50_ms, h.p95_ms, h.max_ms]))
        .collect();
    let budget_label = format!("{} budget", format::ms(budget));
    let chart = hbar_chart(&HBarSpec {
        id: "latency-hooks",
        title: "Hook latency: p50, p95 and maximum against the budget",
        series: &[("p50", HAIKU), ("p95", SONNET), ("max", ACCENT)],
        groups: &groups,
        format: format::ms,
        reference: Some(Reference {
            value: budget,
            label: &budget_label,
        }),
    });
    let rows: String = l
        .hooks
        .iter()
        .map(|h| {
            let over = if h.p95_ms > budget {
                " <strong class=\"warn\">p95 above budget</strong>"
            } else {
                ""
            };
            format!(
                "<tr><td>{}{over}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                html(&h.hook),
                format::integer(h.runs),
                format::ms(h.p50_ms),
                format::ms(h.p95_ms),
                format::ms(h.max_ms),
            )
        })
        .collect();
    let spawn = l.spawn_overhead_ms.map_or_else(String::new, |ms| {
        format!(
            " Process start-up adds about {} per hook call {ESTIMATE} (from <code>doctor --bench</code>).",
            format::ms(ms)
        )
    });
    format!(
        "{chart}<div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">Hook</th>\
         <th scope=\"col\" class=\"num\">Runs</th><th scope=\"col\" class=\"num\">p50</th>\
         <th scope=\"col\" class=\"num\">p95</th><th scope=\"col\" class=\"num\">Max</th></tr>\
         </thead><tbody>{rows}</tbody></table></div>\
         <p class=\"note\">Trade-off: hooks added {} in total, and in the same period saved \
         about {} and {} tokens {ESTIMATE}.{spawn}</p>",
        format::ms(l.total_added_ms),
        format::usd(l.saved_usd),
        format::integer(l.saved_tokens),
    )
}

/// Section 8: audit of the fixed context.
#[must_use]
pub fn audit(audit: Option<&AuditStats>) -> String {
    let content = match audit {
        Some(a) if !a.heaviest.is_empty() || !a.unused_mcp.is_empty() => audit_content(a),
        _ => empty(
            "Run tokred audit to see the heaviest fixed-context items and unused MCP servers.",
        ),
    };
    section("audit", "Audit", true, &content)
}

/// Tables of the audit section.
fn audit_content(a: &AuditStats) -> String {
    let rows: String = a
        .heaviest
        .iter()
        .map(|item| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td></tr>",
                html(&item.name),
                html(&item.kind),
                html(&item.origin),
                format::integer(item.tokens),
            )
        })
        .collect();
    let items = if a.heaviest.is_empty() {
        String::new()
    } else {
        format!(
            "<h3>Heaviest fixed-context items</h3><div class=\"table-wrap\"><table><thead><tr>\
             <th scope=\"col\">Item</th><th scope=\"col\">Kind</th><th scope=\"col\">Origin</th>\
             <th scope=\"col\" class=\"num\">Tokens {ESTIMATE}</th></tr></thead>\
             <tbody>{rows}</tbody></table></div>\
             <p class=\"note\">Weight is characters / 4, an estimate.</p>"
        )
    };
    let window = a.unused_window_days.map_or_else(
        || "in the analysed window".to_owned(),
        |days| format!("in the last {days} days"),
    );
    let unused = if a.unused_mcp.is_empty() {
        "<h3>Unused MCP servers</h3><p class=\"note\">Every configured MCP server was used.</p>"
            .to_owned()
    } else {
        let list: String = a
            .unused_mcp
            .iter()
            .map(|name| format!("<li><code>{}</code></li>", html(name)))
            .collect();
        format!(
            "<h3>Unused MCP servers</h3><p class=\"note\">No tool call {window}:</p><ul>{list}</ul>"
        )
    };
    format!("{items}{unused}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AuditItem, HookLatency, TaskTypeCount};
    use crate::testutil::assert_well_formed;

    #[test]
    fn audit_with_only_unused_servers_has_no_items_table() {
        let html = audit(Some(&AuditStats {
            unused_mcp: vec!["<figma>".to_owned()],
            ..AuditStats::default()
        }));
        assert_well_formed(&html).unwrap();
        assert!(html.contains("No tool call in the analysed window"));
        assert!(html.contains("&lt;figma&gt;"));
        assert!(!html.contains("Heaviest"));
    }

    #[test]
    fn audit_with_only_items_says_every_server_was_used() {
        let html = audit(Some(&AuditStats {
            heaviest: vec![AuditItem {
                name: "CLAUDE.md".to_owned(),
                tokens: 1_234,
                ..AuditItem::default()
            }],
            ..AuditStats::default()
        }));
        assert_well_formed(&html).unwrap();
        assert!(html.contains("1,234"));
        assert!(html.contains("Every configured MCP server was used"));
    }

    #[test]
    fn empty_audit_routing_and_latency_show_the_notice() {
        assert!(audit(Some(&AuditStats::default())).contains("No data yet."));
        assert!(routing(Some(&RoutingStats::default())).contains("No data yet."));
        assert!(latency(Some(&LatencyStats::default())).contains("No data yet."));
    }

    #[test]
    fn routing_with_only_delegations_renders() {
        let html = routing(Some(&RoutingStats {
            delegations_correct: 3,
            ..RoutingStats::default()
        }));
        assert_well_formed(&html).unwrap();
        assert!(html.contains("100.0% of 3 delegations"));
    }

    #[test]
    fn routing_with_zero_delegations_does_not_divide_by_zero() {
        let html = routing(Some(&RoutingStats {
            classifications: vec![TaskTypeCount {
                task_type: "edit".to_owned(),
                count: 2,
            }],
            ..RoutingStats::default()
        }));
        assert!(html.contains("0.0% of 0 delegations"));
    }

    #[test]
    fn latency_without_a_budget_uses_fifty_milliseconds() {
        let html = latency(Some(&LatencyStats {
            hooks: vec![HookLatency {
                hook: "route".to_owned(),
                p95_ms: 51.0,
                ..HookLatency::default()
            }],
            ..LatencyStats::default()
        }));
        assert_well_formed(&html).unwrap();
        assert!(html.contains("50.0 ms budget"));
        assert!(html.contains("p95 above budget"));
        assert!(!html.contains("doctor --bench"));
    }

    #[test]
    fn models_share_is_zero_when_totals_are_zero() {
        let html = models(&[ModelUsage::default()]);
        assert_well_formed(&html).unwrap();
        assert!(html.contains("0.0%"));
    }

    #[test]
    fn last_keeps_the_trailing_items() {
        let items = [1, 2, 3, 4];
        assert_eq!(last(&items, Some(2)), &[3, 4]);
        assert_eq!(last(&items, Some(10)), &items);
        assert_eq!(last(&items, None), &items);
    }

    #[test]
    fn period_views_show_the_last_days_only() {
        let days: Vec<DayCost> = (1..=10)
            .map(|d| DayCost {
                date: format!("2026-10-{d:02}"),
                real_usd: 1.0,
                all_opus_usd: 2.0,
            })
            .collect();
        let html = cost(&days);
        assert_well_formed(&html).unwrap();
        let pane = |key: &str| {
            let start = html.find(&format!("data-pane=\"{key}\"")).unwrap();
            let end = html[start..].find("</svg>").unwrap();
            &html[start..start + end]
        };
        assert!(!pane("7").contains("2026-10-03"));
        assert!(pane("7").contains("2026-10-04"));
        assert!(pane("all").contains("2026-10-01"));
        assert!(html.contains("Shown period: real $7.00 versus $14.00"));
    }

    #[test]
    fn daily_series_detection() {
        assert!(!has_daily_series(&ReportData::default()));
        let with_cache = ReportData {
            cache_per_day: vec![DayCache::default()],
            ..ReportData::default()
        };
        assert!(has_daily_series(&with_cache));
    }
}
