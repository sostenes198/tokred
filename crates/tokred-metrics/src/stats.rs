//! `tokred stats`: cost, savings, prompt-cache rate, projection and before/after (pure).
//!
//! Inputs are plain slices: nothing here reads the database or the clock. Pass **all** the usage
//! rows you have, not only those inside `range`: the projection looks at the last 7 and 30 days
//! and the before/after comparison at everything around the first `configure`, both regardless of
//! `range`. Totals, groups, savings and the cache rate only look at `range`.
//!
//! Money is rounded to 6 decimals in the output so JSON is stable. Every ratio is `None` rather
//! than NaN when its denominator is zero.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use tokred_core::Timestamp;
use tokred_core::hook_event::HookEventRecord;
use tokred_core::pricing::{
    Family, ModelPrice, PriceTable, Resolution, Resolved, Usage, cost as usage_cost,
};
use tokred_core::usage::UsageRecord;

use crate::report::{BYTES_PER_TOKEN, is_guard_block};
use crate::span::TimeRange;
use crate::util::{SECS_PER_DAY, day_string, len_u64, ratio, round6, to_f64};

/// How `groups` are formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    /// One group per model id.
    Model,
    /// One group per UTC day (`YYYY-MM-DD`).
    Day,
    /// One group per project directory.
    Project,
    /// One group per request origin (`main`, `subagent`, `auxiliary`, `compaction`). Only the
    /// proxy sees the last two; rows read from transcripts alone are `main` or `subagent`.
    Origin,
    /// One group per agent type of a subagent request (`Explore`, `Plan`, ...). Only the proxy
    /// knows it; every other row is [`UNKNOWN_AGENT_TYPE`].
    #[serde(rename = "agent_type")]
    AgentType,
}

/// The agent type of a row that has none.
pub const UNKNOWN_AGENT_TYPE: &str = "(unknown)";

/// Everything [`compute_stats`] needs.
#[derive(Debug, Clone, Copy)]
pub struct StatsInput<'a> {
    /// All known usage rows (see the module docs on why not only those in `range`).
    pub usage: &'a [UsageRecord],
    /// Prices.
    pub prices: &'a PriceTable,
    /// Hook events (used for the guard savings estimate).
    pub events: &'a [HookEventRecord],
    /// Period reported by totals, groups, savings and cache rate.
    pub range: TimeRange,
    /// Grouping of the `groups` list.
    pub group_by: GroupBy,
    /// The current time (anchors the projection windows).
    pub now: Timestamp,
    /// When tokred was first configured, if known (enables before/after).
    pub first_configure: Option<Timestamp>,
}

/// Token counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TokenCounts {
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Prompt-cache write tokens.
    pub cache_write: u64,
    /// Prompt-cache read tokens.
    pub cache_read: u64,
    /// Sum of the four.
    pub total: u64,
}

impl From<&Usage> for TokenCounts {
    fn from(usage: &Usage) -> Self {
        Self {
            input: usage.input,
            output: usage.output,
            cache_write: usage.cache_write,
            cache_read: usage.cache_read,
            total: usage.total(),
        }
    }
}

/// Totals over the range.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Totals {
    /// Assistant messages.
    pub messages: u64,
    /// Distinct sessions.
    pub sessions: u64,
    /// Tokens.
    pub tokens: TokenCounts,
    /// Real cost in USD.
    pub cost_usd: f64,
}

/// One group of the breakdown.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Group {
    /// Model id, day or project, depending on [`GroupBy`].
    pub key: String,
    /// Assistant messages.
    pub messages: u64,
    /// Tokens.
    pub tokens: TokenCounts,
    /// Real cost in USD.
    pub cost_usd: f64,
}

/// Estimated saving of the guards.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GuardSaving {
    /// Always `true`: derived from bytes, not measured.
    pub estimate: bool,
    /// Guard denials in the range.
    pub blocks: u64,
    /// Bytes of context avoided.
    pub bytes_saved: u64,
    /// Estimated tokens avoided (`bytes / 4`, integer division per session).
    pub tokens_avoided: u64,
    /// Estimated USD: tokens avoided at the input price of each session's most used model.
    pub saving_usd: f64,
}

/// Real cost against the "everything on opus" counterfactual.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Savings {
    /// Real cost in USD.
    pub real_cost_usd: f64,
    /// Same tokens at the latest opus price (`None` if the table has no opus). Messages of
    /// models without a price are left out, so they cannot inflate the saving.
    pub opus_cost_usd: Option<f64>,
    /// `opus_cost_usd - real_cost_usd`.
    pub routing_saving_usd: Option<f64>,
    /// Guard saving estimate.
    pub guard: GuardSaving,
    /// Routing plus guard saving (`None` when the routing saving is unknown).
    pub total_saving_usd: Option<f64>,
}

/// Prompt-cache hit rate of one day.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayRate {
    /// UTC day.
    pub day: String,
    /// `cache_read / (cache_read + cache_write + input)`, `None` without input traffic.
    pub rate: Option<f64>,
}

/// Prompt-cache hit rates.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CacheRate {
    /// Over the whole range.
    pub overall: Option<f64>,
    /// Per UTC day, oldest first.
    pub per_day: Vec<DayRate>,
}

/// Projection from one trailing window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ProjectionBasis {
    /// Window length requested (7 or 30 days).
    pub window_days: u64,
    /// Days used as divisor: the window, or fewer if data starts later.
    pub days_of_data: u64,
    /// Cost per day over those days.
    pub daily_avg_usd: f64,
    /// `daily_avg_usd * 30`.
    pub projected_30d_usd: f64,
}

/// Monthly projection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Projection {
    /// From the last 7 days.
    pub last_7d: ProjectionBasis,
    /// From the last 30 days.
    pub last_30d: ProjectionBasis,
}

/// Sessions and cost on one side of the first `configure`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Period {
    /// Sessions that started on this side.
    pub sessions: u64,
    /// Their total cost in USD.
    pub cost_usd: f64,
    /// Cost per session (`None` without sessions).
    pub cost_per_session_usd: Option<f64>,
}

/// Cost per session before and after the first `configure`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct BeforeAfter {
    /// The first `configure` (unix seconds).
    pub first_configure: i64,
    /// Sessions that started before it.
    pub before: Period,
    /// Sessions that started at or after it.
    pub after: Period,
}

/// Why a model's price is not exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnpricedReason {
    /// Priced with the latest price of its family.
    FamilyFallback,
    /// No price at all: counted as zero cost.
    Unresolvable,
}

/// A model whose cost is approximate or missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnpricedModel {
    /// Model id.
    pub model: String,
    /// Why.
    pub reason: UnpricedReason,
    /// Family used for the fallback.
    pub family: Option<&'static str>,
    /// Messages of this model in the range.
    pub messages: u64,
    /// Tokens of this model in the range.
    pub tokens: u64,
}

/// Result of [`compute_stats`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    /// Period reported.
    pub range: TimeRange,
    /// Grouping used.
    pub group_by: GroupBy,
    /// Totals.
    pub totals: Totals,
    /// Breakdown, ordered by key.
    pub groups: Vec<Group>,
    /// Real cost, routing and guard savings.
    pub savings: Savings,
    /// Prompt-cache hit rates.
    pub cache_rate: CacheRate,
    /// Monthly projection (`None` without any usage).
    pub projection: Option<Projection>,
    /// Before/after the first `configure` (`None` unless older data exists).
    pub before_after: Option<BeforeAfter>,
    /// Models flagged as not exactly priced.
    pub unpriced_models: Vec<UnpricedModel>,
}

/// Prices resolved once per model.
struct PriceBook {
    by_model: BTreeMap<String, Option<Resolved>>,
    opus: Option<ModelPrice>,
}

impl PriceBook {
    fn new(table: &PriceTable, records: &[UsageRecord]) -> Self {
        let by_model = records
            .iter()
            .map(|record| (record.model.clone(), table.resolve(&record.model)))
            .collect();
        Self {
            by_model,
            opus: table.latest_of(Family::Opus).map(|entry| entry.price),
        }
    }

    fn resolved(&self, model: &str) -> Option<Resolved> {
        self.by_model.get(model).copied().flatten()
    }

    /// Real cost of a record; zero when the model has no price.
    fn cost(&self, record: &UsageRecord) -> f64 {
        self.resolved(&record.model)
            .map_or(0.0, |resolved| usage_cost(&record.usage, &resolved.price))
    }

    /// Cost of the same tokens at the latest opus price, for priced records only.
    fn opus_cost(&self, record: &UsageRecord) -> f64 {
        match (self.opus, self.resolved(&record.model)) {
            (Some(opus), Some(_)) => usage_cost(&record.usage, &opus),
            _ => 0.0,
        }
    }
}

#[derive(Default)]
struct Acc {
    messages: u64,
    usage: Usage,
    cost: f64,
}

impl Acc {
    fn add(&mut self, record: &UsageRecord, cost: f64) {
        self.messages += 1;
        self.usage.add(&record.usage);
        self.cost += cost;
    }
}

fn group_key(record: &UsageRecord, group_by: GroupBy) -> String {
    match group_by {
        GroupBy::Model => record.model.clone(),
        GroupBy::Day => day_string(record.ts.0),
        GroupBy::Project => record.project.clone(),
        GroupBy::Origin => record.origin.clone().unwrap_or_else(|| {
            if record.is_subagent {
                "subagent"
            } else {
                "main"
            }
            .to_owned()
        }),
        GroupBy::AgentType => record
            .agent_type
            .clone()
            .unwrap_or_else(|| UNKNOWN_AGENT_TYPE.to_owned()),
    }
}

fn cache_rate_of(usage: &Usage) -> Option<f64> {
    ratio(
        usage.cache_read,
        usage.cache_read + usage.cache_write + usage.input,
    )
}

/// The model each session used most (by messages; ties go to the smaller model id).
fn session_models(records: &[UsageRecord]) -> BTreeMap<&str, &str> {
    let mut counts: BTreeMap<&str, BTreeMap<&str, u64>> = BTreeMap::new();
    for record in records {
        *counts
            .entry(&record.session_id)
            .or_default()
            .entry(&record.model)
            .or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(session, models)| {
            models
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(model, _)| (session, model))
        })
        .collect()
}

fn guard_saving(input: &StatsInput<'_>, book: &PriceBook) -> GuardSaving {
    let models = session_models(input.usage);
    let opus_input = book.opus.map_or(0.0, |price| price.input);
    let mut by_session: BTreeMap<&str, u64> = BTreeMap::new();
    let mut blocks = 0;
    for event in input
        .events
        .iter()
        .filter(|event| input.range.contains(event.ts) && is_guard_block(event))
    {
        blocks += 1;
        let bytes = by_session.entry(&event.session_id).or_default();
        *bytes = bytes.saturating_add(event.bytes_saved);
    }
    let mut saving = GuardSaving {
        estimate: true,
        blocks,
        bytes_saved: 0,
        tokens_avoided: 0,
        saving_usd: 0.0,
    };
    for (session, bytes) in by_session {
        let tokens = bytes / BYTES_PER_TOKEN;
        let input_price = models
            .get(session)
            .and_then(|model| book.resolved(model))
            .map_or(opus_input, |resolved| resolved.price.input);
        saving.bytes_saved = saving.bytes_saved.saturating_add(bytes);
        saving.tokens_avoided += tokens;
        saving.saving_usd += to_f64(tokens) * input_price / 1_000_000.0;
    }
    saving.saving_usd = round6(saving.saving_usd);
    saving
}

fn projection(records: &[UsageRecord], book: &PriceBook, now: Timestamp) -> Option<Projection> {
    let earliest = records.iter().map(|record| record.ts.0).min()?;
    let basis = |window_days: u64| {
        let window_secs = i64::try_from(window_days).unwrap_or(0) * SECS_PER_DAY;
        let from = now.0.saturating_sub(window_secs);
        let total: f64 = records
            .iter()
            .filter(|record| from <= record.ts.0 && record.ts.0 <= now.0)
            .map(|record| book.cost(record))
            .sum();
        let data_days = (now.0.saturating_sub(earliest).div_euclid(SECS_PER_DAY) + 1)
            .clamp(1, i64::try_from(window_days).unwrap_or(1));
        let days_of_data = u64::try_from(data_days).unwrap_or(1);
        let daily = total / to_f64(days_of_data);
        ProjectionBasis {
            window_days,
            days_of_data,
            daily_avg_usd: round6(daily),
            projected_30d_usd: round6(daily * 30.0),
        }
    };
    Some(Projection {
        last_7d: basis(7),
        last_30d: basis(30),
    })
}

fn before_after(
    records: &[UsageRecord],
    book: &PriceBook,
    first_configure: Option<Timestamp>,
) -> Option<BeforeAfter> {
    let cut = first_configure?;
    if !records.iter().any(|record| record.ts < cut) {
        return None;
    }
    let mut started: BTreeMap<&str, Timestamp> = BTreeMap::new();
    for record in records {
        let first = started.entry(&record.session_id).or_insert(record.ts);
        *first = (*first).min(record.ts);
    }
    let mut costs = [0.0_f64; 2];
    let mut sessions = [BTreeSet::new(), BTreeSet::new()];
    for record in records {
        let side = usize::from(started[record.session_id.as_str()] >= cut);
        costs[side] += book.cost(record);
        sessions[side].insert(record.session_id.as_str());
    }
    let period = |side: usize| {
        let count = len_u64(sessions[side].len());
        Period {
            sessions: count,
            cost_usd: round6(costs[side]),
            cost_per_session_usd: (count != 0).then(|| round6(costs[side] / to_f64(count))),
        }
    };
    Some(BeforeAfter {
        first_configure: cut.0,
        before: period(0),
        after: period(1),
    })
}

fn unpriced(in_range: &[&UsageRecord], book: &PriceBook) -> Vec<UnpricedModel> {
    let mut by_model: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for record in in_range {
        let entry = by_model.entry(&record.model).or_default();
        entry.0 += 1;
        entry.1 += record.usage.total();
    }
    by_model
        .into_iter()
        .filter_map(|(model, (messages, tokens))| {
            let (reason, family) = match book.resolved(model).map(|resolved| resolved.resolution) {
                Some(Resolution::Exact) => return None,
                Some(Resolution::FamilyFallback(family)) => {
                    (UnpricedReason::FamilyFallback, Some(family.name()))
                }
                None => (UnpricedReason::Unresolvable, None),
            };
            Some(UnpricedModel {
                model: model.to_owned(),
                reason,
                family,
                messages,
                tokens,
            })
        })
        .collect()
}

/// Computes the `stats` numbers. See the module docs for what is limited to `range`.
#[must_use]
pub fn compute_stats(input: &StatsInput<'_>) -> Stats {
    let book = PriceBook::new(input.prices, input.usage);
    let in_range: Vec<&UsageRecord> = input
        .usage
        .iter()
        .filter(|record| input.range.contains(record.ts))
        .collect();

    let mut total = Acc::default();
    let mut groups: BTreeMap<String, Acc> = BTreeMap::new();
    let mut days: BTreeMap<String, Usage> = BTreeMap::new();
    let mut sessions: BTreeSet<&str> = BTreeSet::new();
    let mut opus_cost = 0.0;
    for record in &in_range {
        let cost = book.cost(record);
        total.add(record, cost);
        groups
            .entry(group_key(record, input.group_by))
            .or_default()
            .add(record, cost);
        days.entry(day_string(record.ts.0))
            .or_default()
            .add(&record.usage);
        sessions.insert(&record.session_id);
        opus_cost += book.opus_cost(record);
    }

    let real_cost = round6(total.cost);
    let guard = guard_saving(input, &book);
    let opus_cost_usd = book.opus.map(|_| round6(opus_cost));
    let routing_saving_usd = opus_cost_usd.map(|opus| round6(opus - real_cost));
    Stats {
        range: input.range,
        group_by: input.group_by,
        totals: Totals {
            messages: total.messages,
            sessions: len_u64(sessions.len()),
            tokens: TokenCounts::from(&total.usage),
            cost_usd: real_cost,
        },
        groups: groups
            .into_iter()
            .map(|(key, acc)| Group {
                key,
                messages: acc.messages,
                tokens: TokenCounts::from(&acc.usage),
                cost_usd: round6(acc.cost),
            })
            .collect(),
        savings: Savings {
            real_cost_usd: real_cost,
            opus_cost_usd,
            routing_saving_usd,
            guard,
            total_saving_usd: routing_saving_usd.map(|routing| round6(routing + guard.saving_usd)),
        },
        cache_rate: CacheRate {
            overall: cache_rate_of(&total.usage),
            per_day: days
                .into_iter()
                .map(|(day, usage)| DayRate {
                    day,
                    rate: cache_rate_of(&usage),
                })
                .collect(),
        },
        projection: projection(input.usage, &book, input.now),
        before_after: before_after(input.usage, &book, input.first_configure),
        unpriced_models: unpriced(&in_range, &book),
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::hook_event::Outcome;
    use tokred_core::pricing::PriceEntry;

    use super::*;

    const DAY: i64 = 86_400;
    /// 2026-03-01T00:00:00Z.
    const T0: i64 = 1_772_323_200;
    const OPUS: &str = "claude-opus-4-1-20250805";
    const SONNET: &str = "claude-sonnet-4-20250514";

    fn price_entry(model: &str, input: f64, output: f64, write: f64, read: f64) -> PriceEntry {
        PriceEntry {
            model: model.to_owned(),
            price: ModelPrice {
                input,
                output,
                cache_write: write,
                cache_write_1h: input * 2.0,
                cache_read: read,
            },
            source: "test".to_owned(),
            updated_at: Timestamp(0),
        }
    }

    fn table() -> PriceTable {
        PriceTable::new(vec![
            price_entry(OPUS, 15.0, 75.0, 18.75, 1.5),
            price_entry(SONNET, 3.0, 15.0, 3.75, 0.3),
            price_entry("claude-3-5-haiku-20241022", 0.8, 4.0, 1.0, 0.08),
        ])
    }

    fn rec(
        key: &str,
        session: &str,
        project: &str,
        ts: i64,
        model: &str,
        tokens: [u64; 4],
    ) -> UsageRecord {
        UsageRecord {
            message_key: key.to_owned(),
            session_id: session.to_owned(),
            project: project.to_owned(),
            ts: Timestamp(ts),
            model: model.to_owned(),
            usage: Usage {
                input: tokens[0],
                output: tokens[1],
                cache_write: tokens[2],
                cache_write_1h: 0,
                cache_read: tokens[3],
            },
            is_subagent: false,
            origin: None,
            agent_type: None,
        }
    }

    /// Hand-computed dataset; see the assertions for the arithmetic.
    fn dataset() -> Vec<UsageRecord> {
        vec![
            rec("r1", "s1", "A", T0 + 10, OPUS, [1_000, 500, 2_000, 10_000]),
            rec("r2", "s1", "A", T0 + 20, SONNET, [2_000, 1_000, 0, 8_000]),
            rec(
                "r3",
                "s2",
                "B",
                T0 + DAY + 5,
                SONNET,
                [1_000_000, 100_000, 0, 0],
            ),
            rec(
                "r4",
                "s2",
                "B",
                T0 + DAY + 6,
                "claude-haiku-9-20990101",
                [500_000, 100_000, 0, 0],
            ),
            rec("r5", "s3", "A", T0 + 30, "gpt-foo", [100, 0, 0, 0]),
        ]
    }

    fn guard(session: &str, decision: &str, bytes: u64) -> HookEventRecord {
        HookEventRecord {
            ts: Timestamp(T0 + 50),
            session_id: session.to_owned(),
            hook: "guard-read".to_owned(),
            decision: decision.to_owned(),
            reason: "reread".to_owned(),
            bytes_saved: bytes,
            duration_us: 100,
            outcome: Outcome::Ok,
        }
    }

    fn events() -> Vec<HookEventRecord> {
        vec![
            guard("s1", "deny", 40_000),
            guard("s1", "warn", 99_999),
            guard("s2", "deny", 4_003),
            guard("nobody", "deny", 8_000),
        ]
    }

    fn stats_for(
        usage: &[UsageRecord],
        range: TimeRange,
        group_by: GroupBy,
        first_configure: Option<Timestamp>,
    ) -> Stats {
        compute_stats(&StatsInput {
            usage,
            prices: &table(),
            events: &events(),
            range,
            group_by,
            now: Timestamp(T0 + DAY + 100),
            first_configure,
        })
    }

    fn whole() -> TimeRange {
        TimeRange::new(Timestamp(T0), Timestamp(T0 + 3 * DAY))
    }

    #[test]
    fn totals_match_a_manual_sum() {
        let stats = stats_for(&dataset(), whole(), GroupBy::Model, None);
        // r1 0.015+0.0375+0.0375+0.015 = 0.105; r2 0.006+0.015+0.0024 = 0.0234;
        // r3 3+1.5 = 4.5; r4 (haiku fallback) 0.4+0.4 = 0.8; r5 has no price: 0.
        assert_eq!(stats.totals.cost_usd, 5.4284);
        assert_eq!(stats.totals.messages, 5);
        assert_eq!(stats.totals.sessions, 3);
        assert_eq!(
            stats.totals.tokens,
            TokenCounts {
                input: 1_503_100,
                output: 201_500,
                cache_write: 2_000,
                cache_read: 18_000,
                total: 1_724_600
            }
        );
        assert_eq!(stats.group_by, GroupBy::Model);
    }

    #[test]
    fn groups_by_model_day_and_project() {
        let usage = dataset();
        let by_model = stats_for(&usage, whole(), GroupBy::Model, None).groups;
        let got: Vec<_> = by_model
            .iter()
            .map(|g| (g.key.as_str(), g.messages, g.cost_usd))
            .collect();
        assert_eq!(
            got,
            [
                ("claude-haiku-9-20990101", 1, 0.8),
                (OPUS, 1, 0.105),
                (SONNET, 2, 4.5234),
                ("gpt-foo", 1, 0.0),
            ]
        );

        let by_day = stats_for(&usage, whole(), GroupBy::Day, None).groups;
        let got: Vec<_> = by_day
            .iter()
            .map(|g| (g.key.as_str(), g.messages, g.cost_usd))
            .collect();
        assert_eq!(got, [("2026-03-01", 3, 0.1284), ("2026-03-02", 2, 5.3)]);

        let by_project = stats_for(&usage, whole(), GroupBy::Project, None).groups;
        let got: Vec<_> = by_project
            .iter()
            .map(|g| (g.key.as_str(), g.tokens.total))
            .collect();
        assert_eq!(got, [("A", 24_600), ("B", 1_700_000)]);
    }

    #[test]
    fn groups_by_origin_and_agent_type_use_what_the_proxy_knows() {
        let mut usage = dataset();
        usage[0].origin = Some("main".to_owned());
        usage[1].origin = Some("auxiliary".to_owned());
        usage[2].origin = Some("subagent".to_owned());
        usage[2].agent_type = Some("Explore".to_owned());
        usage[3].is_subagent = true;

        let by_origin = stats_for(&usage, whole(), GroupBy::Origin, None);
        let got: Vec<_> = by_origin
            .groups
            .iter()
            .map(|g| (g.key.as_str(), g.messages))
            .collect();
        assert_eq!(
            got,
            [("auxiliary", 1), ("main", 2), ("subagent", 2)],
            "rows without the proxy are main, or subagent when the transcript says so"
        );

        let by_agent = stats_for(&usage, whole(), GroupBy::AgentType, None);
        let got: Vec<_> = by_agent
            .groups
            .iter()
            .map(|g| (g.key.as_str(), g.messages))
            .collect();
        assert_eq!(got, [("(unknown)", 4), ("Explore", 1)]);
        let json = serde_json::to_value(&by_agent).unwrap();
        assert_eq!(json["group_by"], "agent_type");
        assert_eq!(
            serde_json::to_value(&by_origin).unwrap()["group_by"],
            "origin"
        );
    }

    #[test]
    fn the_range_limits_totals_but_not_the_projection() {
        let usage = dataset();
        let day_one = TimeRange::new(Timestamp(T0), Timestamp(T0 + DAY));
        let stats = stats_for(&usage, day_one, GroupBy::Day, None);
        assert_eq!(stats.totals.messages, 3);
        assert_eq!(stats.totals.cost_usd, 0.1284);
        let projection = stats.projection.unwrap();
        // Everything is inside the trailing windows; data spans two days.
        assert_eq!(projection.last_7d.days_of_data, 2);
        assert_eq!(projection.last_7d.daily_avg_usd, 2.7142);
        assert_eq!(projection.last_7d.projected_30d_usd, 81.426);
        assert_eq!(projection.last_30d.days_of_data, 2);
        assert_eq!(projection.last_30d.window_days, 30);
    }

    #[test]
    fn routing_saving_compares_with_everything_on_opus() {
        let savings = stats_for(&dataset(), whole(), GroupBy::Model, None).savings;
        // Priced messages only (r5 excluded): 0.105 + 0.117 + 22.5 + 15 = 37.722.
        assert_eq!(savings.real_cost_usd, 5.4284);
        assert_eq!(savings.opus_cost_usd, Some(37.722));
        assert_eq!(savings.routing_saving_usd, Some(32.2936));
        assert_eq!(savings.total_saving_usd, Some(32.4744));
    }

    #[test]
    fn guard_saving_is_an_estimate_at_the_session_models_input_price() {
        let guard = stats_for(&dataset(), whole(), GroupBy::Model, None)
            .savings
            .guard;
        // s1 ties opus/sonnet 1:1 -> smaller id (opus): 10_000 tokens x 15 = 0.15.
        // s2 ties sonnet/haiku 1:1 -> haiku (fallback price 0.8): 1_000 x 0.8 = 0.0008. Unknown session -> opus: 2_000 x 15 = 0.03.
        assert!(guard.estimate);
        assert_eq!(guard.blocks, 3);
        assert_eq!(guard.bytes_saved, 52_003);
        assert_eq!(guard.tokens_avoided, 13_000);
        assert_eq!(guard.saving_usd, 0.1808);
    }

    #[test]
    fn guard_events_outside_the_range_or_without_an_opus_price_cost_nothing() {
        let late = TimeRange::new(Timestamp(T0 + 100), Timestamp(T0 + DAY));
        let guard = stats_for(&dataset(), late, GroupBy::Model, None)
            .savings
            .guard;
        assert_eq!((guard.blocks, guard.tokens_avoided), (0, 0));

        let no_opus = PriceTable::new(vec![price_entry(SONNET, 3.0, 15.0, 3.75, 0.3)]);
        let stats = compute_stats(&StatsInput {
            usage: &[],
            prices: &no_opus,
            events: &events(),
            range: whole(),
            group_by: GroupBy::Model,
            now: Timestamp(T0),
            first_configure: None,
        });
        assert_eq!(stats.savings.guard.tokens_avoided, 13_000);
        assert_eq!(stats.savings.guard.saving_usd, 0.0, "no price known");
    }

    #[test]
    fn without_an_opus_price_the_counterfactual_is_unknown() {
        let no_opus = PriceTable::new(vec![price_entry(SONNET, 3.0, 15.0, 3.75, 0.3)]);
        let stats = compute_stats(&StatsInput {
            usage: &dataset(),
            prices: &no_opus,
            events: &[],
            range: whole(),
            group_by: GroupBy::Model,
            now: Timestamp(T0 + DAY),
            first_configure: None,
        });
        assert_eq!(stats.savings.opus_cost_usd, None);
        assert_eq!(stats.savings.routing_saving_usd, None);
        assert_eq!(stats.savings.total_saving_usd, None);
        let reasons: Vec<_> = stats
            .unpriced_models
            .iter()
            .map(|m| (m.model.as_str(), m.reason))
            .collect();
        assert_eq!(
            reasons,
            [
                ("claude-haiku-9-20990101", UnpricedReason::Unresolvable),
                (OPUS, UnpricedReason::Unresolvable),
                ("gpt-foo", UnpricedReason::Unresolvable),
            ]
        );
    }

    #[test]
    fn cache_rate_overall_and_per_day() {
        let cache = stats_for(&dataset(), whole(), GroupBy::Model, None).cache_rate;
        // 18_000 / (18_000 + 2_000 + 1_503_100)
        assert_eq!(cache.overall, Some(0.011_818));
        let days: Vec<_> = cache
            .per_day
            .iter()
            .map(|d| (d.day.as_str(), d.rate))
            .collect();
        // Day 1: 18_000 / (18_000 + 2_000 + 3_100); day 2: nothing was read from the cache.
        assert_eq!(
            days,
            [("2026-03-01", Some(0.779_221)), ("2026-03-02", Some(0.0))]
        );
    }

    #[test]
    fn unpriced_models_are_flagged() {
        let stats = stats_for(&dataset(), whole(), GroupBy::Model, None);
        assert_eq!(
            stats.unpriced_models,
            [
                UnpricedModel {
                    model: "claude-haiku-9-20990101".to_owned(),
                    reason: UnpricedReason::FamilyFallback,
                    family: Some("haiku"),
                    messages: 1,
                    tokens: 600_000,
                },
                UnpricedModel {
                    model: "gpt-foo".to_owned(),
                    reason: UnpricedReason::Unresolvable,
                    family: None,
                    messages: 1,
                    tokens: 100,
                },
            ]
        );
    }

    fn projection_of(usage: &[UsageRecord], now: Timestamp) -> Projection {
        projection(usage, &PriceBook::new(&table(), usage), now).unwrap()
    }

    #[test]
    fn projection_uses_trailing_windows_and_clamps_to_available_data() {
        let now = Timestamp(T0 + 100 * DAY);
        let one_m = [1_000_000, 0, 0, 0];
        let usage = vec![
            rec("old", "a", "p", now.0 - 40 * DAY, SONNET, one_m),
            rec("ten", "b", "p", now.0 - 10 * DAY, SONNET, one_m),
            rec("three", "c", "p", now.0 - 3 * DAY, SONNET, one_m),
            rec("future", "d", "p", now.0 + 5, SONNET, one_m),
        ];
        let got = projection_of(&usage, now);
        // 7d: only "three" (3.0) over 7 days; 30d: "ten" + "three" (6.0) over 30 days.
        assert_eq!(got.last_7d.days_of_data, 7);
        assert_eq!(got.last_7d.daily_avg_usd, 0.428_571);
        assert_eq!(got.last_7d.projected_30d_usd, 12.857_143);
        assert_eq!(got.last_30d.daily_avg_usd, 0.2);
        assert_eq!(got.last_30d.projected_30d_usd, 6.0);

        // Data that started one hour ago counts as one day, not seven.
        let fresh = vec![rec("x", "a", "p", now.0 - 3_600, SONNET, one_m)];
        let got = projection_of(&fresh, now);
        assert_eq!(got.last_7d.days_of_data, 1);
        assert_eq!(got.last_7d.daily_avg_usd, 3.0);
        assert_eq!(got.last_30d.projected_30d_usd, 90.0);

        // All records in the future: still one day, no panic.
        let ahead = vec![rec("x", "a", "p", now.0 + DAY, SONNET, [1, 0, 0, 0])];
        assert_eq!(projection_of(&ahead, now).last_7d.days_of_data, 1);
    }

    #[test]
    fn before_after_compares_cost_per_session() {
        let usage = dataset();
        let cut = Timestamp(T0 + DAY);
        let stats = stats_for(&usage, whole(), GroupBy::Model, Some(cut));
        let ba = stats.before_after.unwrap();
        assert_eq!(ba.first_configure, T0 + DAY);
        // Before: s1 (0.105 + 0.0234) and s3 (unpriced, 0) -> 0.1284 over 2 sessions.
        assert_eq!(
            ba.before,
            Period {
                sessions: 2,
                cost_usd: 0.1284,
                cost_per_session_usd: Some(0.0642)
            }
        );
        // After: s2 (4.5 + 0.8).
        assert_eq!(
            ba.after,
            Period {
                sessions: 1,
                cost_usd: 5.3,
                cost_per_session_usd: Some(5.3)
            }
        );
    }

    #[test]
    fn before_after_needs_older_data_and_a_configure_time() {
        let usage = dataset();
        assert!(
            stats_for(&usage, whole(), GroupBy::Model, None)
                .before_after
                .is_none()
        );
        // Configured before any usage: nothing to compare against.
        let early = Some(Timestamp(T0));
        assert!(
            stats_for(&usage, whole(), GroupBy::Model, early)
                .before_after
                .is_none()
        );
        // Configured after everything: no session after.
        let late = Some(Timestamp(T0 + 10 * DAY));
        let ba = stats_for(&usage, whole(), GroupBy::Model, late)
            .before_after
            .unwrap();
        assert_eq!(ba.after.sessions, 0);
        assert_eq!(ba.after.cost_per_session_usd, None);
        assert_eq!(ba.before.sessions, 3);
    }

    #[test]
    fn empty_periods_do_not_divide_by_zero() {
        let stats = stats_for(&[], whole(), GroupBy::Day, Some(Timestamp(T0)));
        assert_eq!(stats.totals.messages, 0);
        assert_eq!(stats.totals.cost_usd, 0.0);
        assert!(stats.groups.is_empty());
        assert_eq!(stats.cache_rate.overall, None);
        assert!(stats.cache_rate.per_day.is_empty());
        assert!(stats.projection.is_none());
        assert!(stats.before_after.is_none());
        assert!(stats.unpriced_models.is_empty());
        assert_eq!(stats.savings.opus_cost_usd, Some(0.0));
        assert_eq!(stats.savings.routing_saving_usd, Some(0.0));
        let json = serde_json::to_string(&stats).unwrap();
        assert!(!json.contains("NaN") && !json.contains("inf"));

        // Records exist, but none in the range.
        let elsewhere = TimeRange::new(Timestamp(0), Timestamp(10));
        let stats = stats_for(&dataset(), elsewhere, GroupBy::Model, None);
        assert_eq!(stats.cache_rate.overall, None);
        assert!(stats.projection.is_some());
    }

    #[test]
    fn a_session_of_zero_tokens_has_no_cache_rate() {
        let usage = vec![rec("z", "s", "p", T0, OPUS, [0, 0, 0, 0])];
        let stats = stats_for(&usage, whole(), GroupBy::Model, None);
        assert_eq!(stats.cache_rate.overall, None);
        assert_eq!(stats.cache_rate.per_day[0].rate, None);
    }
}
