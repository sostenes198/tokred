//! [`PromptTracker`]: follows each session's user prompts through the proxy and emits one
//! [`PromptSummary`] per prompt, plus a [`SuggestionRecord`] when a run of simple prompts would
//! have been cheaper on the next model family. It never touches a request: `before` does
//! nothing, and the main loop's model is never changed.
//!
//! **Prompt boundaries.** Claude Code 2.1.312 does not send `x-claude-code-prompt-id`, so a
//! prompt is delimited structurally: a successful main-loop request whose last message is a
//! typed user turn ([`RequestShape::starts_prompt`]) opens a new prompt, unless it repeats the
//! message count of the request that opened the current prompt (a retry). When the header is
//! sent, a new value opens a new prompt instead. The prompt is closed (and emitted) when the
//! session's next prompt starts, when it has been idle for `idle_secs` (see
//! [`PromptTracker::flush_idle`]), at shutdown ([`PromptTracker::flush_all`]) or when its
//! session is evicted to bound memory. A prompt closed for being idle is continued (and emitted
//! again under the same id) if its main loop resumes, e.g. after a long tool call.
//!
//! **What counts.** Main-loop requests add turns, tools, tokens, cost and the context size
//! (from the first one). Subagent requests add tools, tokens and cost, but no turns. Compaction
//! adds cost only. Auxiliary requests (titles and the like) and failed responses are ignored.
//! Requests that arrive before the session's first prompt start (the proxy started mid-prompt)
//! are ignored too.
//!
//! [`RequestShape::starts_prompt`]: tokred_core::proxy::RequestShape::starts_prompt

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::json;
use tokred_core::Timestamp;
use tokred_core::config::SuggestConfig;
use tokred_core::pricing::{PriceTable, Usage, cost};
use tokred_core::prompts::PromptSummary;
use tokred_core::proxy::{
    HookFailure, Origin, ProxyHook, ProxyRecord, RecordSink, RequestCtx, ResponseSummary,
};

use crate::counterfactual::counterfactual;
use crate::criteria::{PromptFacts, SimpleCriteria, is_edit_tool, is_subagent_tool};
use crate::rule::{PromptOutcome, Streak, SuggestRule};

/// Default `idle_secs`: under the 5-minute cache lifetime, so a prompt's summary (and any
/// suggestion) is stored before its cache goes cold.
pub const DEFAULT_IDLE_SECS: i64 = 240;

/// Default `max_sessions` followed at once.
pub const DEFAULT_MAX_SESSIONS: usize = 256;

/// Default `max_tools`: tool names kept per prompt (the rest are only counted).
pub const DEFAULT_MAX_TOOLS: usize = 200;

/// Idle prompts are looked for at most this often while requests flow.
const SWEEP_INTERVAL_SECS: i64 = 30;

/// How the tracker behaves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackerSettings {
    /// What makes a prompt simple.
    pub criteria: SimpleCriteria,
    /// When to suggest.
    pub rule: SuggestRule,
    /// A prompt with no request for longer than this is closed, seconds.
    pub idle_secs: i64,
    /// Sessions followed at once; the least recently active one is evicted first.
    pub max_sessions: usize,
    /// Tool names kept per prompt.
    pub max_tools: usize,
}

impl Default for TrackerSettings {
    fn default() -> Self {
        Self::from_config(&SuggestConfig::default())
    }
}

impl TrackerSettings {
    /// The settings for `[suggest]`, with the default criteria and limits.
    #[must_use]
    pub fn from_config(config: &SuggestConfig) -> Self {
        Self {
            criteria: SimpleCriteria::default(),
            rule: SuggestRule::from_config(config),
            idle_secs: DEFAULT_IDLE_SECS,
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_tools: DEFAULT_MAX_TOOLS,
        }
    }
}

/// Why a prompt was closed (`closed_by` in its detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Close {
    NextPrompt,
    Idle,
    Shutdown,
    Evicted,
}

impl Close {
    const fn name(self) -> &'static str {
        match self {
            Self::NextPrompt => "next_prompt",
            Self::Idle => "idle",
            Self::Shutdown => "shutdown",
            Self::Evicted => "evicted",
        }
    }
}

/// Who sent a request, among the kinds that count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Main,
    Subagent,
    Compaction,
}

/// A prompt being followed.
#[derive(Debug, Clone)]
struct OpenPrompt {
    id: String,
    header_id: Option<String>,
    start: Timestamp,
    last_activity: Timestamp,
    opening_messages: usize,
    model: String,
    turns: u32,
    tools: Vec<String>,
    tools_dropped: u64,
    output_tokens: u64,
    thinking_tokens: u64,
    main_usage: Usage,
    main_cost: f64,
    subagent_cost: f64,
    compaction_cost: f64,
    compactions: u32,
    edits: u32,
    subagents: u32,
    context_tokens: u64,
    unpriced: u32,
}

impl OpenPrompt {
    fn new(id: String, header_id: Option<String>, start: Timestamp, messages: usize) -> Self {
        Self {
            id,
            header_id,
            start,
            last_activity: start,
            opening_messages: messages,
            model: String::new(),
            turns: 0,
            tools: Vec::new(),
            tools_dropped: 0,
            output_tokens: 0,
            thinking_tokens: 0,
            main_usage: Usage::default(),
            main_cost: 0.0,
            subagent_cost: 0.0,
            compaction_cost: 0.0,
            compactions: 0,
            edits: 0,
            subagents: 0,
            context_tokens: 0,
            unpriced: 0,
        }
    }

    fn add(&mut self, seen: &Seen<'_>, max_tools: usize) {
        self.last_activity = self.last_activity.max(seen.end);
        let usage = &seen.response.usage;
        if seen.cost.is_none() {
            self.unpriced = self.unpriced.saturating_add(1);
        }
        let cost = seen.cost.unwrap_or(0.0);
        match seen.kind {
            Kind::Compaction => {
                self.compactions = self.compactions.saturating_add(1);
                self.compaction_cost += cost;
                return;
            }
            Kind::Main => {
                if self.turns == 0 {
                    self.context_tokens = usage
                        .input
                        .saturating_add(usage.cache_read)
                        .saturating_add(usage.cache_write);
                }
                self.turns = self.turns.saturating_add(1);
                self.model.clone_from(&seen.model);
                self.main_usage.add(usage);
                self.main_cost += cost;
            }
            Kind::Subagent => self.subagent_cost += cost,
        }
        self.output_tokens = self.output_tokens.saturating_add(usage.output);
        self.thinking_tokens = self
            .thinking_tokens
            .saturating_add(seen.response.thinking_tokens);
        for tool in &seen.response.tool_uses {
            self.edits = self.edits.saturating_add(u32::from(is_edit_tool(tool)));
            self.subagents = self
                .subagents
                .saturating_add(u32::from(is_subagent_tool(tool)));
            if self.tools.len() < max_tools {
                self.tools.push(tool.clone());
            } else {
                self.tools_dropped = self.tools_dropped.saturating_add(1);
            }
        }
    }
}

/// One counted request, as the tracker sees it.
struct Seen<'a> {
    kind: Kind,
    model: String,
    cost: Option<f64>,
    end: Timestamp,
    response: &'a ResponseSummary,
}

/// What the tracker keeps per session.
#[derive(Debug, Default)]
struct SessionState {
    seq: u64,
    open: Option<OpenPrompt>,
    idle_closed: Option<OpenPrompt>,
    one_hour_cache: bool,
    streak: Streak,
    last_seen: Timestamp,
}

#[derive(Debug, Default)]
struct State {
    prices: PriceTable,
    sessions: HashMap<String, SessionState>,
    last_sweep: Timestamp,
}

/// The proxy hook that summarizes the main loop's prompts and makes model suggestions.
pub struct PromptTracker {
    settings: TrackerSettings,
    sink: Arc<dyn RecordSink>,
    state: Mutex<State>,
}

impl fmt::Debug for PromptTracker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromptTracker")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl PromptTracker {
    /// A tracker that prices with `prices` and sends its records to `sink`.
    #[must_use]
    pub fn new(prices: PriceTable, settings: TrackerSettings, sink: Arc<dyn RecordSink>) -> Self {
        Self {
            settings,
            sink,
            state: Mutex::new(State {
                prices,
                ..State::default()
            }),
        }
    }

    /// Replaces the price table (after a price sync).
    pub fn set_prices(&self, prices: PriceTable) {
        self.lock().prices = prices;
    }

    /// Closes and emits every prompt idle for longer than `idle_secs` at `now`. The proxy
    /// should call this on a timer (every 30 to 60 seconds): an idle session sends no request
    /// that would trigger it.
    pub fn flush_idle(&self, now: Timestamp) {
        let records = {
            let mut state = self.lock();
            state.last_sweep = now;
            self.sweep(&mut state, now)
        };
        self.emit(records);
    }

    /// Closes and emits every open prompt (proxy shutdown).
    pub fn flush_all(&self, now: Timestamp) {
        let records = {
            let mut state = self.lock();
            let State {
                prices, sessions, ..
            } = &mut *state;
            let mut out = Vec::new();
            for (id, session) in sessions.iter_mut() {
                if let Some(prompt) = session.open.take() {
                    self.close(prompt, id, session, prices, now, Close::Shutdown, &mut out);
                }
                session.idle_closed = None;
            }
            out
        };
        self.emit(records);
    }

    /// Prompts currently open.
    #[must_use]
    pub fn open_prompts(&self) -> usize {
        self.lock()
            .sessions
            .values()
            .filter(|session| session.open.is_some())
            .count()
    }

    /// Sessions currently followed.
    #[must_use]
    pub fn tracked_sessions(&self) -> usize {
        self.lock().sessions.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, records: Vec<ProxyRecord>) {
        for record in records {
            self.sink.record(record);
        }
    }

    fn sweep(&self, state: &mut State, now: Timestamp) -> Vec<ProxyRecord> {
        let State {
            prices, sessions, ..
        } = state;
        let idle = self.settings.idle_secs;
        let mut out = Vec::new();
        for (id, session) in sessions.iter_mut() {
            let is_idle = |p: &mut OpenPrompt| now.0.saturating_sub(p.last_activity.0) > idle;
            if let Some(prompt) = session.open.take_if(is_idle) {
                self.close(prompt, id, session, prices, now, Close::Idle, &mut out);
            }
        }
        out
    }

    /// Makes room for a new `session_id` by evicting the least recently active session (its
    /// open prompt is closed and emitted first) when `max_sessions` are followed.
    fn make_room(
        &self,
        state: &mut State,
        session_id: &str,
        now: Timestamp,
        out: &mut Vec<ProxyRecord>,
    ) {
        let State {
            prices, sessions, ..
        } = state;
        let full = sessions.len() >= self.settings.max_sessions.max(1);
        if !full || sessions.contains_key(session_id) {
            return;
        }
        let oldest = sessions
            .iter()
            .min_by_key(|(_, session)| session.last_seen)
            .map(|(id, _)| id.clone());
        if let Some((id, mut gone)) = oldest.and_then(|id| sessions.remove_entry(&id))
            && let Some(prompt) = gone.open.take()
        {
            self.close(prompt, &id, &mut gone, prices, now, Close::Evicted, out);
        }
    }

    fn observe(
        &self,
        state: &mut State,
        ctx: &RequestCtx<'_>,
        response: &ResponseSummary,
    ) -> Vec<ProxyRecord> {
        let mut out = Vec::new();
        if !(200..300).contains(&response.status) {
            return out;
        }
        let Some(session_id) = ctx.meta.session_id.as_deref().filter(|id| !id.is_empty()) else {
            return out;
        };
        let kind = match ctx.origin {
            Origin::Main => Kind::Main,
            Origin::Subagent { .. } => Kind::Subagent,
            Origin::Compaction => Kind::Compaction,
            Origin::Auxiliary => return out,
        };
        let model = response
            .model
            .clone()
            .or_else(|| ctx.shape.model.clone())
            .unwrap_or_default();
        let seen = Seen {
            kind,
            cost: state
                .prices
                .resolve(&model)
                .map(|resolved| cost(&response.usage, &resolved.price)),
            model,
            end: Timestamp(
                ctx.now
                    .0
                    .saturating_add(i64::try_from(response.total_ms / 1000).unwrap_or(0)),
            ),
            response,
        };

        self.make_room(state, session_id, ctx.now, &mut out);
        let State {
            prices, sessions, ..
        } = state;
        let session = sessions.entry(session_id.to_owned()).or_default();
        session.last_seen = session.last_seen.max(seen.end);
        if kind == Kind::Main {
            session.one_hour_cache |= response.usage.cache_write_1h > 0;
            if starts_new_prompt(session, ctx) {
                if let Some(previous) = session.open.take() {
                    self.close(
                        previous,
                        session_id,
                        session,
                        prices,
                        ctx.now,
                        Close::NextPrompt,
                        &mut out,
                    );
                }
                session.idle_closed = None;
                session.seq = session.seq.saturating_add(1);
                let id = ctx
                    .meta
                    .prompt_id
                    .clone()
                    .unwrap_or_else(|| format!("{session_id}:{}:{}", ctx.now.0, session.seq));
                session.open = Some(OpenPrompt::new(
                    id,
                    ctx.meta.prompt_id.clone(),
                    ctx.now,
                    ctx.shape.message_count,
                ));
            }
        }
        if session.open.is_none() {
            session.open = session.idle_closed.take();
        }
        if let Some(prompt) = session.open.as_mut() {
            prompt.add(&seen, self.settings.max_tools);
        }
        out
    }

    /// Emits the summary of a finished prompt and, when the rule fires, a suggestion.
    #[allow(clippy::too_many_arguments)]
    fn close(
        &self,
        prompt: OpenPrompt,
        session_id: &str,
        session: &mut SessionState,
        prices: &PriceTable,
        now: Timestamp,
        why: Close,
        out: &mut Vec<ProxyRecord>,
    ) {
        let facts = PromptFacts {
            turns: prompt.turns,
            output_tokens: prompt.output_tokens,
            thinking_tokens: prompt.thinking_tokens,
            edits: prompt.edits,
            subagents: prompt.subagents,
        };
        let exceeded = self.settings.criteria.exceeded(&facts);
        let cf = counterfactual(
            prices,
            &prompt.model,
            &prompt.main_usage,
            prompt.context_tokens,
            session.one_hour_cache,
        );
        let est_saving_usd = cf
            .as_ref()
            .map_or(0.0, |cf| cf.net_saving(prompt.main_cost));
        let detail = json!({
            "main_cost_usd": prompt.main_cost,
            "subagent_cost_usd": prompt.subagent_cost,
            "compaction_cost_usd": prompt.compaction_cost,
            "compactions": prompt.compactions,
            "edits": prompt.edits,
            "context_tokens": prompt.context_tokens,
            "one_hour_cache": session.one_hour_cache,
            "last_activity": prompt.last_activity.0,
            "closed_by": why.name(),
            "not_simple": exceeded,
            "tools_dropped": prompt.tools_dropped,
            "unpriced_requests": prompt.unpriced,
            "target_model": cf.as_ref().map(|cf| cf.target_model.clone()),
            "target_cost_usd": cf.as_ref().map(|cf| cf.target_usd),
            "rebuild_usd": cf.as_ref().map(|cf| cf.rebuild_usd),
            "gross_saving_usd": cf.as_ref().map(|cf| cf.gross_saving(prompt.main_cost)),
        });
        let simple = exceeded.is_empty();
        out.push(ProxyRecord::Prompt(Box::new(PromptSummary {
            prompt_id: prompt.id.clone(),
            session_id: session_id.to_owned(),
            ts: prompt.start,
            model: prompt.model.clone(),
            turns: prompt.turns,
            tools: prompt.tools.clone(),
            output_tokens: prompt.output_tokens,
            thinking_tokens: prompt.thinking_tokens,
            cost_usd: prompt.main_cost + prompt.subagent_cost + prompt.compaction_cost,
            edited: prompt.edits > 0,
            subagents: prompt.subagents,
            simple,
            est_saving_usd,
            detail: detail.to_string(),
        })));
        let outcome = PromptOutcome {
            prompt_id: prompt.id.clone(),
            model: prompt.model.clone(),
            simple,
            main_cost_usd: prompt.main_cost,
            counterfactual: cf,
        };
        if let Some(suggestion) = session.streak.record(&outcome, &self.settings.rule, now) {
            out.push(ProxyRecord::Suggestion(Box::new(
                suggestion.to_record(session_id, now),
            )));
        }
        if why == Close::Idle {
            session.idle_closed = Some(prompt);
        }
    }
}

/// The main-loop request opens a new prompt (see the module documentation).
fn starts_new_prompt(session: &SessionState, ctx: &RequestCtx<'_>) -> bool {
    let current = session.open.as_ref().or(session.idle_closed.as_ref());
    if let Some(header) = ctx.meta.prompt_id.as_deref() {
        return current.and_then(|p| p.header_id.as_deref()) != Some(header);
    }
    ctx.shape.starts_prompt
        && !session
            .open
            .as_ref()
            .is_some_and(|p| p.header_id.is_none() && p.opening_messages == ctx.shape.message_count)
}

impl ProxyHook for PromptTracker {
    fn name(&self) -> &'static str {
        "prompts"
    }

    fn before(&self, _ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure> {
        Ok(())
    }

    fn after(&self, ctx: &RequestCtx<'_>, response: &ResponseSummary) {
        let records = {
            let mut state = self.lock();
            let mut out = self.observe(&mut state, ctx, response);
            if ctx.now.0.saturating_sub(state.last_sweep.0) >= SWEEP_INTERVAL_SECS {
                state.last_sweep = ctx.now;
                out.extend(self.sweep(&mut state, ctx.now));
            }
            out
        };
        self.emit(records);
    }
}
