//! The sections about what only the proxy measures: model routing, overhead, prompt-cache
//! misses by cause, and cost per origin and agent type.
//!
//! They are rendered only when the proxy recorded something. Without it the report has exactly
//! the eight sections it always had.

use crate::escape::html;
use crate::format;
use crate::model::{ProxyBreakdown, ProxyMiss, ProxyOverhead, ProxyReport, ProxyRoute};
use crate::sections::{ACCENT, ESTIMATE, HAIKU, SONNET, section, share};
use crate::svg::{HBarSpec, Reference, hbar_chart};

/// Every proxy section, or nothing when the proxy recorded nothing.
#[must_use]
pub fn proxy(report: Option<&ProxyReport>) -> String {
    report.map_or_else(String::new, |report| {
        [
            routing(report),
            overhead(report.overhead.as_ref()),
            cache_misses(report),
            origins(report),
        ]
        .concat()
    })
}

/// Requested against effective model, what routing decided, and the suggestion waiting.
fn routing(report: &ProxyReport) -> String {
    let source = format!(
        "<p class=\"note\">{} messages were measured by the proxy; {} are known only from the \
         transcripts (made while the proxy was off).</p>",
        format::integer(report.measured_messages),
        format::integer(report.transcript_only_messages),
    );
    let routes = if report.routes.is_empty() {
        String::new()
    } else {
        routes_table(&report.routes)
    };
    let decisions = report.routing.as_ref().map_or_else(String::new, |routing| {
        format!(
            "<div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">Routing decisions</th>\
             <th scope=\"col\" class=\"num\">Count</th>\
             <th scope=\"col\" class=\"num\">Saving {ESTIMATE}</th></tr></thead><tbody>\
             <tr><td>Applied (model changed)</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\
             <tr><td>Observed (would have changed)</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\
             <tr><td>Kept as asked</td><td class=\"num\">{}</td><td class=\"num\">&ndash;</td></tr>\
             <tr><td>Fell back to the original</td><td class=\"num\">{}</td><td class=\"num\">&ndash;</td></tr>\
             </tbody></table></div>",
            format::integer(routing.applied),
            format::usd(routing.applied_saving_usd),
            format::integer(routing.observed),
            format::usd(routing.observed_saving_usd),
            format::integer(routing.kept),
            format::integer(routing.fallback),
        )
    });
    let suggestion = report.suggestion.as_ref().map_or_else(String::new, |text| {
        format!(
            "<p class=\"note\">Suggestion waiting: {} <code>tokred suggest</code> shows the evidence.</p>",
            html(text)
        )
    });
    section(
        "proxy-routing",
        "Model routing (proxy)",
        report.routing.is_some(),
        &format!("{source}{routes}{decisions}{suggestion}"),
    )
}

fn routes_table(routes: &[ProxyRoute]) -> String {
    let rows: String = routes
        .iter()
        .map(|route| {
            let changed = if route.requested == route.effective {
                ""
            } else {
                " <strong>rewritten</strong>"
            };
            format!(
                "<tr><td>{}</td><td>{}{changed}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td></tr>",
                html(&route.requested),
                html(&route.effective),
                format::integer(route.requests),
                format::usd(route.cost_usd),
            )
        })
        .collect();
    format!(
        "<div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">Asked for</th>\
         <th scope=\"col\">Answered by</th><th scope=\"col\" class=\"num\">Messages</th>\
         <th scope=\"col\" class=\"num\">Cost</th></tr></thead><tbody>{rows}</tbody></table></div>"
    )
}

/// The time the proxy added, against its budget.
fn overhead(overhead: Option<&ProxyOverhead>) -> String {
    let Some(o) = overhead else {
        return String::new();
    };
    let label = format!("{} budget", format::ms(o.budget_ms));
    let chart = hbar_chart(&HBarSpec {
        id: "proxy-overhead-chart",
        title: "Time added by the proxy: p50, p95 and maximum against the budget",
        series: &[("p50", HAIKU), ("p95", SONNET), ("max", ACCENT)],
        groups: &[("Proxy".to_owned(), vec![o.p50_ms, o.p95_ms, o.max_ms])],
        format: format::ms,
        reference: Some(Reference {
            value: o.budget_ms,
            label: &label,
        }),
    });
    let verdict = if o.p95_ms > o.budget_ms {
        format!(
            "<strong class=\"warn\">p95 above budget</strong>: {} against {}.",
            format::ms(o.p95_ms),
            format::ms(o.budget_ms)
        )
    } else {
        format!("p95 is within the {} budget.", format::ms(o.budget_ms))
    };
    let content = format!(
        "{chart}<div class=\"table-wrap\"><table><thead><tr><th scope=\"col\" class=\"num\">Requests</th>\
         <th scope=\"col\" class=\"num\">p50</th><th scope=\"col\" class=\"num\">p95</th>\
         <th scope=\"col\" class=\"num\">p99</th><th scope=\"col\" class=\"num\">Max</th></tr></thead>\
         <tbody><tr><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
         <td class=\"num\">{}</td><td class=\"num\">{}</td></tr></tbody></table></div>\
         <p class=\"note\">{verdict} This is the time the proxy spent before forwarding each request.</p>",
        format::integer(o.requests),
        format::ms(o.p50_ms),
        format::ms(o.p95_ms),
        format::ms(o.p99_ms),
        format::ms(o.max_ms),
    );
    section("proxy-overhead", "Proxy overhead", false, &content)
}

/// Prompt-cache misses of the main loop, by likely cause.
fn cache_misses(report: &ProxyReport) -> String {
    if report.cache_misses.is_empty() {
        return String::new();
    }
    let groups: Vec<(String, Vec<f64>)> = report
        .cache_misses
        .iter()
        .map(|miss| (miss.cause.clone(), vec![format::to_f64(miss.requests)]))
        .collect();
    let chart = hbar_chart(&HBarSpec {
        id: "proxy-cache-causes",
        title: "Main-loop prompt-cache misses per likely cause",
        series: &[("misses", ACCENT)],
        groups: &groups,
        format: format::integer_f64,
        reference: None,
    });
    let misses: u64 = report.cache_misses.iter().map(|miss| miss.requests).sum();
    let rows: String = report.cache_misses.iter().map(miss_row).collect();
    let rate = format::percent(share(
        format::to_f64(misses),
        format::to_f64(report.main_requests),
    ));
    let content = format!(
        "{chart}<div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">Cause</th>\
         <th scope=\"col\" class=\"num\">Misses</th>\
         <th scope=\"col\" class=\"num\">Tokens written again</th></tr></thead>\
         <tbody>{rows}</tbody></table></div>\
         <p class=\"note\">{} of {} main-loop messages ({rate}) missed the prompt cache. \
         The cause is the proxy's best reading of what changed. Each request is compared with the \
         previous one of its own series (same model and system prompt), because a session can \
         interleave side queries with the conversation and each series keeps a cache of its own; \
         the first request of a series counts as a miss.</p>",
        format::integer(misses),
        format::integer(report.main_requests),
    );
    section("proxy-cache", "Cache misses by cause", false, &content)
}

/// The cause with what it means, for the table ("compaction: the conversation was compacted").
fn cause_text(cause: &str) -> String {
    let meaning = match cause {
        "compaction" => "the conversation was compacted",
        "idle_ttl" => "the cache expired while idle",
        "model_changed" => "first request on this model (each model has its own cache)",
        "tools_changed" => "the tools changed",
        "system_changed" => "the system prompt changed",
        "thinking_changed" => "the thinking settings changed",
        "unknown" => "none of the above",
        _ => return cause.to_owned(),
    };
    format!("{cause}: {meaning}")
}

fn miss_row(miss: &ProxyMiss) -> String {
    format!(
        "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
        html(&cause_text(&miss.cause)),
        format::integer(miss.requests),
        format::compact(miss.cache_write_tokens),
    )
}

/// Cost per origin and per agent type.
fn origins(report: &ProxyReport) -> String {
    if report.by_origin.is_empty() && report.by_agent_type.is_empty() {
        return String::new();
    }
    let content = format!(
        "{}{}",
        breakdown("By origin", "Origin", &report.by_origin),
        breakdown("By agent type", "Agent type", &report.by_agent_type),
    );
    section(
        "proxy-origin",
        "Cost by origin and agent type",
        false,
        &content,
    )
}

fn breakdown(heading: &str, first: &str, rows: &[ProxyBreakdown]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let body: String = rows
        .iter()
        .map(|row| {
            format!(
                "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td></tr>",
                html(&row.name),
                format::integer(row.messages),
                format::compact(row.tokens),
                format::usd(row.cost_usd),
            )
        })
        .collect();
    format!(
        "<h3>{}</h3><div class=\"table-wrap\"><table><thead><tr><th scope=\"col\">{}</th>\
         <th scope=\"col\" class=\"num\">Messages</th><th scope=\"col\" class=\"num\">Tokens</th>\
         <th scope=\"col\" class=\"num\">Cost</th></tr></thead><tbody>{body}</tbody></table></div>",
        html(heading),
        html(first),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProxyRouting;
    use crate::testutil::assert_well_formed;

    const HOSTILE: &str = "<script>alert(1)</script>";

    fn full() -> ProxyReport {
        ProxyReport {
            measured_messages: 120,
            transcript_only_messages: 14,
            routes: vec![
                ProxyRoute {
                    requested: "claude-sonnet-5-5".to_owned(),
                    effective: "claude-haiku-5-5".to_owned(),
                    requests: 40,
                    cost_usd: 1.5,
                },
                ProxyRoute {
                    requested: HOSTILE.to_owned(),
                    effective: HOSTILE.to_owned(),
                    requests: 80,
                    cost_usd: 9.0,
                },
            ],
            routing: Some(ProxyRouting {
                applied: 30,
                observed: 10,
                kept: 5,
                fallback: 1,
                applied_saving_usd: 4.25,
                observed_saving_usd: 1.5,
            }),
            overhead: Some(ProxyOverhead {
                requests: 150,
                p50_ms: 0.4,
                p95_ms: 1.2,
                p99_ms: 2.0,
                max_ms: 3.5,
                budget_ms: 10.0,
            }),
            main_requests: 80,
            cache_misses: vec![
                ProxyMiss {
                    cause: "ttl_expired".to_owned(),
                    requests: 6,
                    cache_write_tokens: 120_000,
                },
                ProxyMiss {
                    cause: HOSTILE.to_owned(),
                    requests: 2,
                    cache_write_tokens: 900,
                },
            ],
            by_origin: vec![ProxyBreakdown {
                name: "main".to_owned(),
                messages: 80,
                tokens: 3_000_000,
                cost_usd: 12.0,
            }],
            by_agent_type: vec![ProxyBreakdown {
                name: HOSTILE.to_owned(),
                messages: 40,
                tokens: 500_000,
                cost_usd: 1.5,
            }],
            suggestion: Some(HOSTILE.to_owned()),
        }
    }

    #[test]
    fn without_the_proxy_nothing_is_rendered() {
        assert_eq!(proxy(None), "");
    }

    #[test]
    fn every_section_is_rendered_and_user_text_is_escaped() {
        let page = proxy(Some(&full()));
        for id in [
            "proxy-routing",
            "proxy-overhead",
            "proxy-cache",
            "proxy-origin",
        ] {
            assert!(page.contains(&format!("id=\"{id}\"")), "{id}");
        }
        for needle in [
            "120 messages were measured by the proxy; 14 are known only",
            "<strong>rewritten</strong>",
            "Applied (model changed)",
            "p95 is within the 10.0 ms budget.",
            "8 of 80 main-loop messages (10.0%)",
            "Suggestion waiting",
        ] {
            assert!(page.contains(needle), "missing {needle:?}");
        }
        assert!(!page.contains(HOSTILE), "hostile text must be escaped");
        assert_well_formed(&page).unwrap();
    }

    #[test]
    fn an_overhead_over_budget_is_called_out() {
        let mut report = full();
        if let Some(o) = report.overhead.as_mut() {
            o.p95_ms = 10.5;
        }
        let page = proxy(Some(&report));
        assert!(page.contains("p95 above budget"));
        assert!(!page.contains("p95 is within"));
    }

    #[test]
    fn a_report_with_little_data_drops_the_empty_sections() {
        let page = proxy(Some(&ProxyReport::default()));
        assert!(
            page.contains("id=\"proxy-routing\""),
            "the source note is always there"
        );
        for id in ["proxy-overhead", "proxy-cache", "proxy-origin"] {
            assert!(!page.contains(id), "{id}");
        }
        assert!(!page.contains("<table>"), "no routes, no decisions");
        assert!(
            !page.contains("est\""),
            "no routing decisions, nothing to flag as an estimate"
        );

        let only_agents = ProxyReport {
            by_agent_type: full().by_agent_type,
            ..ProxyReport::default()
        };
        let page = proxy(Some(&only_agents));
        assert!(page.contains("By agent type") && !page.contains("By origin"));
        let only_origin = ProxyReport {
            by_origin: full().by_origin,
            ..ProxyReport::default()
        };
        let page = proxy(Some(&only_origin));
        assert!(page.contains("By origin") && !page.contains("By agent type"));
    }
}
