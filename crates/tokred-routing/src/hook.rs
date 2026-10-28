//! The routing hook: decides, records and (in `enforce`) rewrites the model of subagent and
//! auxiliary requests. The main loop's model is never changed: the rewrite goes through
//! [`rewrite_model`], which needs a ticket that only subagent and auxiliary requests get.

use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokred_core::Timestamp;
use tokred_core::config::{RoutingConfig, TierChoice, TrafficMode};
use tokred_core::pricing::{ModelPrice, PriceTable, Usage, cost};
use tokred_core::proxy::{
    HookFailure, Origin, ProxyHook, ProxyRecord, RecordSink, RequestCtx, rewrite_model,
    rewrite_ticket,
};
use tokred_core::routing::RoutingDecision;

use crate::RULES_VERSION;
use crate::breaker::{Breaker, BreakerStatus};
use crate::catalog::{Catalog, ModelInfo};
use crate::classify::{
    Classification, Layer, RouteClass, classify_auxiliary, classify_subagent, first_user_text,
};
use crate::compat::{first_unmet, needs};
use crate::lru::Lru;

/// Subagents remembered at once (sticky decisions).
const MAX_AGENTS: usize = 4096;
/// Sessions whose main-loop model is remembered.
const MAX_SESSIONS: usize = 1024;
/// Recent auxiliary decisions kept so a rejected rewrite can be recorded against them.
const MAX_AUXILIARY: usize = 256;
/// Output tokens assumed when estimating the cost of one request.
const ASSUMED_OUTPUT_TOKENS: u64 = 200;

/// What `tokred routing status` shows about a running proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutingStatus {
    /// `off`, `observe` or `enforce`.
    pub mode: String,
    /// Version of the rules and of the model catalog.
    pub rules_version: String,
    /// The circuit breaker.
    pub breaker: BreakerStatus,
    /// Subagents with a remembered decision.
    pub tracked_agents: usize,
    /// Sessions whose main-loop model is known.
    pub tracked_sessions: usize,
}

/// A subagent's sticky decision.
#[derive(Debug, Clone)]
struct Sticky {
    decision: RoutingDecision,
    /// The model every request of the agent is rewritten to; `None` when nothing is rewritten.
    rewrite_to: Option<String>,
}

#[derive(Debug)]
struct State {
    agents: Lru<(String, String), Sticky>,
    main_models: Lru<String, String>,
    auxiliary: Lru<String, RoutingDecision>,
    breaker: Breaker,
}

/// What the policy concluded for one decision.
enum Verdict {
    Route {
        target: String,
        tier: &'static str,
        est_to: f64,
    },
    Keep {
        guard: &'static str,
        why: String,
        candidate: Option<(String, f64)>,
    },
}

impl Verdict {
    fn keep(guard: &'static str, why: impl Into<String>) -> Self {
        Self::Keep {
            guard,
            why: why.into(),
            candidate: None,
        }
    }
}

/// The routing step of the proxy (spec section 5).
pub struct RoutingHook {
    config: RoutingConfig,
    prices: PriceTable,
    catalog: Catalog,
    sink: Arc<dyn RecordSink>,
    state: Mutex<State>,
    id_seed: u64,
    next_id: AtomicU64,
}

impl RoutingHook {
    /// A hook using the embedded model catalog.
    #[must_use]
    pub fn new(config: RoutingConfig, prices: PriceTable, sink: Arc<dyn RecordSink>) -> Self {
        let breaker = Breaker::new(config.max_rewrite_failure_rate, &config.cooldown);
        Self {
            config,
            prices,
            catalog: Catalog::embedded(),
            sink,
            state: Mutex::new(State {
                agents: Lru::new(MAX_AGENTS),
                main_models: Lru::new(MAX_SESSIONS),
                auxiliary: Lru::new(MAX_AUXILIARY),
                breaker,
            }),
            id_seed: RandomState::new().hash_one(std::process::id()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Replaces the model catalog (an updated `models.json`).
    #[must_use]
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = catalog;
        self
    }

    /// Version of the rules and of the catalog, stored with every decision.
    #[must_use]
    pub fn rules_version(&self) -> String {
        format!("{RULES_VERSION}+catalog-{}", self.catalog.version)
    }

    /// The mode, the circuit breaker and what is remembered, at `now`.
    pub fn status(&self, now: Timestamp) -> RoutingStatus {
        let mut state = self.lock();
        RoutingStatus {
            mode: mode_name(self.config.mode).to_owned(),
            rules_version: self.rules_version(),
            breaker: state.breaker.status(now),
            tracked_agents: state.agents.len(),
            tracked_sessions: state.main_models.len(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn new_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("rd-{:016x}-{n}", self.id_seed)
    }

    fn record(&self, decision: RoutingDecision) {
        self.sink.record(ProxyRecord::Decision(Box::new(decision)));
    }

    fn subagent(&self, ctx: &mut RequestCtx<'_>, agent_id: &str) {
        let session = ctx.meta.session_id.clone().unwrap_or_default();
        let key = (session.clone(), agent_id.to_owned());
        let mut state = self.lock();
        if let Some(sticky) = state.agents.get_mut(&key) {
            ctx.notes.decision_id = Some(sticky.decision.id.clone());
            let Some(target) = sticky.rewrite_to.clone() else {
                return;
            };
            let from = sticky.decision.model_from.clone();
            let stop = self.why_stop(ctx, &from, &target);
            let stop = stop.or_else(|| {
                state
                    .breaker
                    .is_open(ctx.now)
                    .then(|| "the circuit breaker is open".to_owned())
            });
            match stop {
                None => {
                    state.breaker.record_rewrite();
                    rewrite(ctx, &target);
                }
                Some(why) => {
                    let stopped = self.stop_agent(&mut state, &key, &why);
                    drop(state);
                    self.record_opt(stopped);
                }
            }
            return;
        }

        let main_model = (!session.is_empty())
            .then(|| state.main_models.get_mut(&session).cloned())
            .flatten();
        let mut decision = self.decide_subagent(ctx, agent_id, &session, main_model.as_deref());
        let rewrite_to = apply(ctx, &mut state, &mut decision);
        state.agents.insert(
            key,
            Sticky {
                decision: decision.clone(),
                rewrite_to,
            },
        );
        drop(state);
        self.record(decision);
    }

    fn decide_subagent(
        &self,
        ctx: &RequestCtx<'_>,
        agent_id: &str,
        session: &str,
        main_model: Option<&str>,
    ) -> RoutingDecision {
        let usage = request_usage(&ctx.body);
        let mut base = self.base_decision(ctx, session, Some(agent_id), "agent", &usage);
        let first_turn = non_system_messages(&ctx.body) == 1;
        if !first_turn {
            let classification = Classification::new(
                RouteClass::Other,
                1.0,
                Layer::MidRun,
                "first seen mid-run (e.g. after a restart): switching now would drop thinking \
                 blocks and the agent's cache",
            );
            return finish(
                base,
                &classification,
                Verdict::keep("mid_run", "model kept"),
            );
        }
        let Some(source) = self.source(&mut base) else {
            return unknown_model(base);
        };
        let requested = source.id.clone();
        let explicit = match main_model {
            None => Some(
                "the main loop's model of this session is not known yet: cannot tell whether \
                 the subagent inherited it"
                    .to_owned(),
            ),
            Some(main) if self.catalog.model(main).map(|m| &m.id) != Some(&requested) => Some(
                format!("{requested} differs from the main loop's {main}: chosen on purpose"),
            ),
            Some(_) => None,
        };
        if let Some(reason) = explicit {
            let classification =
                Classification::new(RouteClass::Other, 1.0, Layer::Explicit, reason);
            return finish(
                base,
                &classification,
                Verdict::keep("explicit", "model kept"),
            );
        }
        let classification = classify_subagent(
            ctx.meta.agent_type.as_deref(),
            &ctx.shape.tool_names,
            &first_user_text(&ctx.body),
        );
        let verdict = self.evaluate(ctx, &classification, source, base.est_cost_from, &usage);
        finish(base, &classification, verdict)
    }

    fn auxiliary(&self, ctx: &mut RequestCtx<'_>) {
        let session = ctx.meta.session_id.clone().unwrap_or_default();
        let usage = request_usage(&ctx.body);
        let mut base = self.base_decision(ctx, &session, None, "request", &usage);
        let mut decision = match self.source(&mut base) {
            None => unknown_model(base),
            Some(source) => {
                let classification = classify_auxiliary(ctx.meta.request_class.as_deref());
                let verdict = if has_cache_control(&ctx.body) {
                    Verdict::keep(
                        "cache",
                        "cache_control markers: moving it would lose its prompt cache",
                    )
                } else if !ctx.shape.tool_names.is_empty() {
                    Verdict::keep(
                        "cache",
                        "it carries tools, so it may share the main loop's prompt cache",
                    )
                } else {
                    self.evaluate(ctx, &classification, source, base.est_cost_from, &usage)
                };
                finish(base, &classification, verdict)
            }
        };
        let mut state = self.lock();
        apply(ctx, &mut state, &mut decision);
        state
            .auxiliary
            .insert(decision.id.clone(), decision.clone());
        drop(state);
        self.record(decision);
    }

    /// The decision skeleton: who, when, which model and its estimated cost.
    fn base_decision(
        &self,
        ctx: &RequestCtx<'_>,
        session: &str,
        agent_id: Option<&str>,
        scope: &str,
        usage: &Usage,
    ) -> RoutingDecision {
        let model = ctx.shape.model.clone().unwrap_or_default();
        RoutingDecision {
            id: self.new_id(),
            ts: ctx.now,
            session_id: session.to_owned(),
            agent_id: agent_id.map(str::to_owned),
            scope: scope.to_owned(),
            model_from: model.clone(),
            model_to: model,
            mode: mode_name(self.config.mode).to_owned(),
            est_cost_from: self.estimate(usage, ctx.shape.model.as_deref().unwrap_or_default()),
            rules_version: self.rules_version(),
            outcome: "kept".to_owned(),
            detail: json!({"input_tokens_est": usage.input}).to_string(),
            ..RoutingDecision::default()
        }
    }

    /// The requested model's catalog entry; `None` when it is not in the catalog.
    fn source(&self, decision: &mut RoutingDecision) -> Option<&ModelInfo> {
        let found = self.catalog.model(&decision.model_from);
        if found.is_none() {
            decision.reason = format!(
                "{:?} is not in the model catalog: never routed",
                decision.model_from
            );
        }
        found
    }

    /// Confidence, tier, target and saving (spec 5.2 to 5.5).
    fn evaluate(
        &self,
        ctx: &RequestCtx<'_>,
        classification: &Classification,
        source: &ModelInfo,
        est_from: f64,
        usage: &Usage,
    ) -> Verdict {
        if classification.confidence < self.config.min_confidence {
            return Verdict::keep(
                "confidence",
                format!(
                    "confidence {:.2} below the minimum {:.2}",
                    classification.confidence, self.config.min_confidence
                ),
            );
        }
        let tier = match classification.class.tier(&self.config.tiers) {
            TierChoice::Keep => {
                return Verdict::keep(
                    "tier",
                    format!(
                        "{} is configured to keep the requested model",
                        classification.class.as_str()
                    ),
                );
            }
            TierChoice::Cheap => "cheap",
            TierChoice::Mid => "mid",
        };
        let request_needs = needs(&ctx.body, &ctx.meta.betas, source, &self.catalog);
        let mut first_refusal = None;
        for target in self.catalog.tier_targets(tier) {
            let refusal = self
                .refusal(source, target)
                .or_else(|| first_unmet(&request_needs, target).map(|n| n.refusal(&target.id)));
            if let Some(why) = refusal {
                first_refusal = first_refusal.or(Some(why));
                continue;
            }
            let est_to = self.estimate(usage, &target.id);
            let saving = est_from - est_to;
            if saving < self.config.min_saving_usd {
                return Verdict::Keep {
                    guard: "saving",
                    why: format!(
                        "estimated saving ${saving:.6} below the minimum ${:.6}",
                        self.config.min_saving_usd
                    ),
                    candidate: Some((target.id.clone(), est_to)),
                };
            }
            return Verdict::Route {
                target: target.id.clone(),
                tier,
                est_to,
            };
        }
        Verdict::keep(
            "target",
            first_refusal.unwrap_or_else(|| format!("no model in the {tier} tier")),
        )
    }

    /// Why `target` may not replace `source` regardless of the request: allowed list and
    /// downgrade only (every price no higher, input strictly lower).
    fn refusal(&self, source: &ModelInfo, target: &ModelInfo) -> Option<String> {
        let allowed = &self.config.allowed_models;
        if !allowed.is_empty()
            && !allowed
                .iter()
                .any(|a| *a == target.id || target.aliases.contains(a))
        {
            return Some(format!("{} is not in allowed_models", target.id));
        }
        let price = |id: &str| self.prices.resolve(id).map(|r| r.price);
        match (price(&source.id), price(&target.id)) {
            (Some(from), Some(to)) if is_cheaper(&to, &from) => None,
            (Some(_), Some(_)) => Some(format!("{} is not cheaper than {}", target.id, source.id)),
            _ => Some(format!("no price for {} or {}", source.id, target.id)),
        }
    }

    /// Estimated cost of a request with this usage on `model`, USD (0 when unpriced).
    fn estimate(&self, usage: &Usage, model: &str) -> f64 {
        self.prices
            .resolve(model)
            .map_or(0.0, |resolved| cost(usage, &resolved.price))
    }

    /// Why a sticky rewrite cannot go on for this request: the requested model changed, or
    /// the request needs something the target lacks.
    fn why_stop(&self, ctx: &RequestCtx<'_>, from: &str, target: &str) -> Option<String> {
        let requested = ctx.shape.model.as_deref().unwrap_or_default();
        if requested != from {
            return Some(format!(
                "the requested model changed mid-run to {requested:?}"
            ));
        }
        let (Some(source), Some(target)) = (self.catalog.model(from), self.catalog.model(target))
        else {
            return Some("the models left the catalog".to_owned());
        };
        let request_needs = needs(&ctx.body, &ctx.meta.betas, source, &self.catalog);
        first_unmet(&request_needs, target).map(|need| need.refusal(&target.id))
    }

    /// Stops rewriting an agent for good (going back and forth would drop thinking blocks
    /// each way) and returns its decision updated with outcome `fallback`.
    fn stop_agent(
        &self,
        state: &mut State,
        key: &(String, String),
        why: &str,
    ) -> Option<RoutingDecision> {
        let sticky = state.agents.get_mut(key)?;
        let target = sticky.rewrite_to.take()?;
        let decision = &mut sticky.decision;
        "fallback".clone_into(&mut decision.outcome);
        decision.reason = format!("{}; rewriting stopped: {why}", decision.reason);
        let mut detail: Value = serde_json::from_str(&decision.detail).unwrap_or_default();
        detail["stopped"] = json!(why);
        detail["original_reads_target_thinking"] =
            json!(self.catalog.reads_thinking(&decision.model_from, &target));
        decision.detail = detail.to_string();
        Some(decision.clone())
    }

    fn record_opt(&self, decision: Option<RoutingDecision>) {
        if let Some(decision) = decision {
            self.record(decision);
        }
    }

    fn note_main(&self, ctx: &RequestCtx<'_>) {
        if let (Some(session), Some(model)) = (&ctx.meta.session_id, &ctx.shape.model) {
            self.lock()
                .main_models
                .insert(session.clone(), model.clone());
        }
    }
}

impl ProxyHook for RoutingHook {
    fn name(&self) -> &'static str {
        "routing"
    }

    fn before(&self, ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure> {
        if self.config.mode == TrafficMode::Off {
            return Ok(());
        }
        match ctx.origin {
            Origin::Main => self.note_main(ctx),
            Origin::Subagent { agent_id } => self.subagent(ctx, &agent_id.clone()),
            Origin::Auxiliary => self.auxiliary(ctx),
            // Compaction gets no rewrite ticket: `compaction = "cheap"` cannot apply.
            Origin::Compaction => {}
        }
        Ok(())
    }

    fn fallback(&self, ctx: &RequestCtx<'_>, status: u16) {
        if ctx.notes.model_rewritten.is_none() {
            return;
        }
        let why = format!("the API rejected the rewritten request with status {status}");
        let mut state = self.lock();
        state.breaker.record_failure(ctx.now);
        let updated = match ctx.origin {
            Origin::Subagent { agent_id } => {
                let key = (
                    ctx.meta.session_id.clone().unwrap_or_default(),
                    agent_id.clone(),
                );
                self.stop_agent(&mut state, &key, &why)
            }
            _ => ctx
                .notes
                .decision_id
                .as_ref()
                .and_then(|id| state.auxiliary.get_mut(id))
                .map(|decision| {
                    "fallback".clone_into(&mut decision.outcome);
                    decision.reason = format!("{}; {why}", decision.reason);
                    decision.clone()
                }),
        };
        drop(state);
        self.record_opt(updated);
    }
}

/// Builds the final decision from the classification and the verdict.
fn finish(
    mut decision: RoutingDecision,
    classification: &Classification,
    verdict: Verdict,
) -> RoutingDecision {
    classification
        .class
        .as_str()
        .clone_into(&mut decision.task_class);
    decision.confidence = classification.confidence;
    classification
        .layer
        .as_str()
        .clone_into(&mut decision.layer);
    let mut detail: Value = serde_json::from_str(&decision.detail).unwrap_or_default();
    match verdict {
        Verdict::Route {
            target,
            tier,
            est_to,
        } => {
            decision.reason = format!(
                "{}; {tier} tier: {} -> {target}",
                classification.reason, decision.model_from
            );
            let outcome = if decision.mode == "enforce" {
                "applied"
            } else {
                "observed"
            };
            outcome.clone_into(&mut decision.outcome);
            decision.model_to = target;
            decision.est_cost_to = est_to;
            detail["tier"] = json!(tier);
        }
        Verdict::Keep {
            guard,
            why,
            candidate,
        } => {
            decision.reason = format!("{}; kept: {why}", classification.reason);
            decision.est_cost_to = decision.est_cost_from;
            detail["guard"] = json!(guard);
            if let Some((target, est)) = candidate {
                detail["candidate"] = json!(target);
                detail["candidate_est_cost"] = json!(est);
            }
        }
    }
    decision.detail = detail.to_string();
    decision
}

/// A decision for a model the catalog does not know (its reason is already set).
fn unknown_model(mut decision: RoutingDecision) -> RoutingDecision {
    RouteClass::Other
        .as_str()
        .clone_into(&mut decision.task_class);
    decision.confidence = 1.0;
    Layer::Default.as_str().clone_into(&mut decision.layer);
    decision.est_cost_to = decision.est_cost_from;
    decision
}

/// Applies a decision to the request: rewrites in `enforce` unless the circuit breaker is open
/// (then the decision becomes `kept`). Returns the model rewritten to.
fn apply(
    ctx: &mut RequestCtx<'_>,
    state: &mut State,
    decision: &mut RoutingDecision,
) -> Option<String> {
    ctx.notes.decision_id = Some(decision.id.clone());
    if decision.outcome != "applied" {
        return None;
    }
    if state.breaker.is_open(ctx.now) {
        "kept".clone_into(&mut decision.outcome);
        decision.reason = format!("{}; kept: the circuit breaker is open", decision.reason);
        decision.model_to.clone_from(&decision.model_from);
        decision.est_cost_to = decision.est_cost_from;
        let mut detail: Value = serde_json::from_str(&decision.detail).unwrap_or_default();
        detail["guard"] = json!("breaker");
        decision.detail = detail.to_string();
        return None;
    }
    state.breaker.record_rewrite();
    rewrite(ctx, &decision.model_to);
    Some(decision.model_to.clone())
}

fn rewrite(ctx: &mut RequestCtx<'_>, target: &str) {
    if let Some(ticket) = rewrite_ticket(ctx.origin) {
        rewrite_model(&mut ctx.body, &ticket, target);
        ctx.changed = true;
        ctx.notes.model_rewritten = Some(target.to_owned());
    }
}

const fn mode_name(mode: TrafficMode) -> &'static str {
    match mode {
        TrafficMode::Off => "off",
        TrafficMode::Observe => "observe",
        TrafficMode::Enforce => "enforce",
    }
}

/// Every price no higher and the input price strictly lower.
fn is_cheaper(to: &ModelPrice, from: &ModelPrice) -> bool {
    to.input < from.input
        && to.output <= from.output
        && to.cache_write <= from.cache_write
        && to.cache_write_1h <= from.cache_write_1h
        && to.cache_read <= from.cache_read
}

/// The usage a request is estimated at: its body as uncached input (bytes / 4) and a small
/// assumed output, capped by `max_tokens`. The same usage is priced on both models.
fn request_usage(body: &Value) -> Usage {
    let bytes = serde_json::to_vec(body).map_or(0, |bytes| bytes.len());
    let max_tokens = body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(ASSUMED_OUTPUT_TOKENS);
    Usage {
        input: u64::try_from(bytes / 4).unwrap_or(u64::MAX),
        output: max_tokens.min(ASSUMED_OUTPUT_TOKENS),
        ..Usage::default()
    }
}

/// Messages that are not mid-conversation system messages: 1 on a subagent's first request.
fn non_system_messages(body: &Value) -> usize {
    body.get("messages")
        .and_then(Value::as_array)
        .map_or(0, |messages| {
            messages
                .iter()
                .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
                .count()
        })
}

/// Whether any `cache_control` marker appears in the body (including top-level auto-caching).
fn has_cache_control(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| key == "cache_control" || has_cache_control(value)),
        Value::Array(items) => items.iter().any(has_cache_control),
        _ => false,
    }
}
