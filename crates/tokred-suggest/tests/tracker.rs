#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `PromptTracker` driven through the proxy hook interface with simulated sessions.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokred_core::Timestamp;
use tokred_core::config::{SuggestChannel, SuggestConfig};
use tokred_core::pricing::{ModelPrice, PriceEntry, PriceTable, Usage};
use tokred_core::prompts::{PromptSummary, SuggestionRecord};
use tokred_core::proxy::{
    HookNotes, Origin, ProxyHook, ProxyRecord, RecordSink, RequestCtx, RequestMeta, RequestShape,
    ResponseSummary,
};
use tokred_suggest::{PromptTracker, TrackerSettings};

const OPUS: &str = "claude-opus-5-5";
const SONNET: &str = "claude-sonnet-5-5";
const HAIKU: &str = "claude-haiku-5-5";

/// Round prices per million tokens: input, then output 5x, write 1.25x, 1-hour write 2x and
/// read 0.1x of the input price.
fn prices() -> PriceTable {
    let entry = |model: &str, input: f64| PriceEntry {
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
    };
    PriceTable::new(vec![
        entry(OPUS, 10.0),
        entry(SONNET, 2.0),
        entry(HAIKU, 1.0),
    ])
}

#[derive(Default)]
struct Sink(Mutex<Vec<ProxyRecord>>);

impl RecordSink for Sink {
    fn record(&self, record: ProxyRecord) {
        self.0.lock().unwrap().push(record);
    }
}

impl Sink {
    fn take(&self) -> Vec<ProxyRecord> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }

    fn prompts(&self) -> Vec<PromptSummary> {
        self.take()
            .into_iter()
            .filter_map(|r| match r {
                ProxyRecord::Prompt(p) => Some(*p),
                _ => None,
            })
            .collect()
    }
}

fn split(records: Vec<ProxyRecord>) -> (Vec<PromptSummary>, Vec<SuggestionRecord>) {
    let mut prompts = Vec::new();
    let mut suggestions = Vec::new();
    for record in records {
        match record {
            ProxyRecord::Prompt(p) => prompts.push(*p),
            ProxyRecord::Suggestion(s) => suggestions.push(*s),
            other => panic!("unexpected record {other:?}"),
        }
    }
    (prompts, suggestions)
}

fn detail(summary: &PromptSummary) -> Value {
    serde_json::from_str(&summary.detail).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

struct Harness {
    sink: Arc<Sink>,
    tracker: PromptTracker,
}

impl Harness {
    fn new(settings: TrackerSettings) -> Self {
        let sink = Arc::new(Sink::default());
        let tracker = PromptTracker::new(prices(), settings, sink.clone());
        Self { sink, tracker }
    }

    fn default() -> Self {
        Self::new(TrackerSettings::default())
    }
}

/// One request and its response.
#[derive(Clone)]
struct Req {
    session: Option<&'static str>,
    origin: Origin,
    prompt_header: Option<&'static str>,
    starts_prompt: bool,
    messages: usize,
    now: i64,
    status: u16,
    model: Option<&'static str>,
    usage: Usage,
    thinking: u64,
    tools: Vec<&'static str>,
    total_ms: u64,
}

impl Req {
    fn main(now: i64, messages: usize) -> Self {
        Self {
            session: Some("s1"),
            origin: Origin::Main,
            prompt_header: None,
            starts_prompt: false,
            messages,
            now,
            status: 200,
            model: Some(OPUS),
            usage: Usage {
                output: 100,
                ..Usage::default()
            },
            thinking: 0,
            tools: Vec::new(),
            total_ms: 0,
        }
    }

    /// A main-loop request that starts a prompt (its last message is a typed user turn).
    fn prompt(now: i64, messages: usize) -> Self {
        Self {
            starts_prompt: true,
            ..Self::main(now, messages)
        }
    }

    fn subagent(now: i64) -> Self {
        Self {
            origin: Origin::Subagent {
                agent_id: "a1".to_owned(),
            },
            model: Some(HAIKU),
            ..Self::main(now, 2)
        }
    }

    fn session(self, session: &'static str) -> Self {
        Self {
            session: Some(session),
            ..self
        }
    }

    fn tools(self, tools: &[&'static str]) -> Self {
        Self {
            tools: tools.to_vec(),
            ..self
        }
    }

    fn usage(self, usage: Usage) -> Self {
        Self { usage, ..self }
    }

    fn send(&self, tracker: &PromptTracker) {
        let meta = RequestMeta {
            session_id: self.session.map(str::to_owned),
            prompt_id: self.prompt_header.map(str::to_owned),
            ..RequestMeta::default()
        };
        let shape = RequestShape {
            model: self.model.map(str::to_owned),
            message_count: self.messages,
            starts_prompt: self.starts_prompt,
            tool_names: vec!["Read".to_owned()],
            ..RequestShape::default()
        };
        let mut ctx = RequestCtx {
            meta: &meta,
            shape: &shape,
            origin: &self.origin,
            now: Timestamp(self.now),
            body: json!({"model": self.model}),
            changed: false,
            notes: HookNotes::default(),
        };
        tracker.before(&mut ctx).unwrap();
        assert!(!ctx.changed, "the tracker never changes a request");
        assert_eq!(ctx.body, json!({"model": self.model}));
        // It never changes a request, so a fallback is not its concern (the default no-op).
        tracker.fallback(&ctx, 400);
        let response = ResponseSummary {
            status: self.status,
            model: self.model.map(str::to_owned),
            usage: self.usage,
            thinking_tokens: self.thinking,
            tool_uses: self.tools.iter().map(|t| (*t).to_owned()).collect(),
            total_ms: self.total_ms,
            ..ResponseSummary::default()
        };
        tracker.after(&ctx, &response);
    }
}

#[test]
fn prompt_boundaries_across_a_multi_turn_session_with_subagents() {
    let h = Harness::default();
    assert_eq!(h.tracker.name(), "prompts");
    let opus_turn = Usage {
        input: 1_000,
        output: 300,
        cache_write: 20_000,
        cache_write_1h: 0,
        cache_read: 0,
    };
    Req::prompt(100, 1)
        .usage(opus_turn)
        .tools(&["Read"])
        .send(&h.tracker);
    Req::main(110, 3).tools(&["Agent"]).send(&h.tracker);
    Req::subagent(120).tools(&["Grep", "Read"]).send(&h.tracker);
    Req::subagent(130).send(&h.tracker);
    Req::main(140, 5).tools(&["Edit"]).send(&h.tracker);
    Req::main(150, 7).send(&h.tracker);
    // The session title (auxiliary) and a failed retry count for nothing.
    Req {
        origin: Origin::Auxiliary,
        ..Req::main(155, 1)
    }
    .send(&h.tracker);
    Req {
        status: 429,
        ..Req::prompt(160, 9)
    }
    .send(&h.tracker);
    assert!(
        h.sink.take().is_empty(),
        "nothing closes while the prompt runs"
    );
    assert_eq!(h.tracker.open_prompts(), 1);

    // The next typed prompt closes the first one.
    Req::prompt(200, 9).send(&h.tracker);
    let first = h.sink.prompts();
    assert_eq!(first.len(), 1);
    let p = &first[0];
    assert_eq!(p.prompt_id, "s1:100:1");
    assert_eq!((p.session_id.as_str(), p.ts), ("s1", Timestamp(100)));
    assert_eq!(p.model, OPUS);
    assert_eq!(p.turns, 4, "main-loop requests only");
    assert_eq!(p.tools, ["Read", "Agent", "Grep", "Read", "Edit"]);
    assert_eq!(p.output_tokens, 300 + 100 * 3 + 100 * 2);
    assert_eq!(p.subagents, 1);
    assert!(p.edited);
    assert!(!p.simple, "it started a subagent");
    // Opus: 1,000 in (0.01) + 600 out (0.03) + 20,000 written (0.25); haiku subagents:
    // 200 out at 5 per million (0.001).
    assert!(close(p.cost_usd, 0.29 + 0.001), "{}", p.cost_usd);
    let d = detail(p);
    assert!(close(d["main_cost_usd"].as_f64().unwrap(), 0.29));
    assert!(close(d["subagent_cost_usd"].as_f64().unwrap(), 0.001));
    assert_eq!(d["context_tokens"], 21_000);
    assert_eq!(d["closed_by"], "next_prompt");
    assert_eq!(d["not_simple"], json!(["subagents"]));
    assert_eq!(d["edits"], 1);
    assert_eq!(d["target_model"], SONNET);

    // Shutdown closes the second one, a single-turn question: simple.
    h.tracker.flush_all(Timestamp(300));
    let second = h.sink.prompts();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].prompt_id, "s1:200:2");
    assert_eq!(second[0].turns, 1);
    assert!(second[0].simple);
    assert_eq!(detail(&second[0])["closed_by"], "shutdown");
    assert_eq!(h.tracker.open_prompts(), 0);
    h.tracker.flush_all(Timestamp(301));
    assert!(h.sink.take().is_empty(), "nothing is emitted twice");
}

#[test]
fn requests_without_a_prompt_or_a_session_are_ignored() {
    let h = Harness::default();
    // The proxy started mid-prompt: continuations and subagents have nothing to join.
    Req::main(10, 5).send(&h.tracker);
    Req::subagent(11).send(&h.tracker);
    Req {
        origin: Origin::Compaction,
        ..Req::main(12, 5)
    }
    .send(&h.tracker);
    // No session id, or an empty one.
    Req {
        session: None,
        ..Req::prompt(13, 1)
    }
    .send(&h.tracker);
    Req::prompt(14, 1).session("").send(&h.tracker);
    assert_eq!(h.tracker.open_prompts(), 0);
    assert_eq!(
        h.tracker.tracked_sessions(),
        1,
        "s1 is known, with no prompt"
    );
    h.tracker.flush_all(Timestamp(100));
    assert!(h.sink.take().is_empty());
}

#[test]
fn a_retried_opening_request_does_not_start_a_new_prompt() {
    let h = Harness::default();
    Req::prompt(10, 3).send(&h.tracker);
    Req::prompt(12, 3).send(&h.tracker);
    Req::prompt(20, 5).send(&h.tracker);
    let closed = h.sink.prompts();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].turns, 2);
}

#[test]
fn the_prompt_id_header_wins_when_sent() {
    let h = Harness::default();
    let with = |now: i64, header: &'static str, starts: bool| Req {
        prompt_header: Some(header),
        starts_prompt: starts,
        ..Req::main(now, 3)
    };
    with(10, "pa", true).send(&h.tracker);
    // A typed turn under the same prompt id (e.g. hook feedback) continues it.
    with(11, "pa", true).send(&h.tracker);
    // A new id opens a new prompt even without a typed turn.
    with(12, "pb", false).send(&h.tracker);
    let closed = h.sink.prompts();
    assert_eq!(closed.len(), 1);
    assert_eq!((closed[0].prompt_id.as_str(), closed[0].turns), ("pa", 2));
    h.tracker.flush_all(Timestamp(20));
    assert_eq!(h.sink.prompts()[0].prompt_id, "pb");
}

#[test]
fn an_idle_prompt_is_flushed_and_continued_under_the_same_id() {
    let h = Harness::default();
    Req::prompt(1_000, 1).send(&h.tracker);
    Req {
        total_ms: 5_000,
        ..Req::main(1_010, 3)
    }
    .send(&h.tracker);
    // Still within idle_secs (240) of the last activity (1,015).
    h.tracker.flush_idle(Timestamp(1_255));
    assert!(h.sink.take().is_empty());
    h.tracker.flush_idle(Timestamp(1_256));
    let flushed = h.sink.prompts();
    assert_eq!(flushed.len(), 1);
    assert_eq!(flushed[0].turns, 2);
    let d = detail(&flushed[0]);
    assert_eq!(d["closed_by"], "idle");
    assert_eq!(d["last_activity"], 1_015);
    assert_eq!(h.tracker.open_prompts(), 0);

    // A long tool call ends and the main loop goes on: same prompt, written again.
    Req::main(1_400, 5).send(&h.tracker);
    Req::subagent(1_401).send(&h.tracker);
    assert_eq!(h.tracker.open_prompts(), 1);
    Req::prompt(1_500, 7).send(&h.tracker);
    let again = h.sink.prompts();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].prompt_id, flushed[0].prompt_id);
    assert_eq!(again[0].turns, 3);
    assert_eq!(detail(&again[0])["closed_by"], "next_prompt");

    // Once the next prompt started, the old one can no longer be continued.
    h.tracker.flush_idle(Timestamp(2_000));
    let third = h.sink.prompts();
    assert_eq!(third.len(), 1);
    assert_eq!(third[0].turns, 1);
}

#[test]
fn requests_of_other_sessions_sweep_idle_prompts() {
    let h = Harness::default();
    Req::prompt(1_000, 1).send(&h.tracker);
    // The first request swept at 1,000; the next sweep runs 30 s later at the earliest.
    Req::prompt(1_020, 1).session("s2").send(&h.tracker);
    Req::prompt(1_215, 1).session("s3").send(&h.tracker);
    assert!(h.sink.take().is_empty(), "s1 is not idle long enough yet");
    // Idle long enough at 1,241, but the last sweep (1,215) is too recent.
    Req::main(1_241, 3).session("s3").send(&h.tracker);
    assert!(h.sink.take().is_empty());
    Req::main(1_250, 5).session("s3").send(&h.tracker);
    let swept = h.sink.prompts();
    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].session_id, "s1");
    assert_eq!(h.tracker.open_prompts(), 2);
}

/// One simple opus prompt: 2,000 output tokens over a 40,000-token cached context.
///
/// Real (opus): 2,000 x 50 + 40,000 x 1 per million = 0.10 + 0.04 = 0.14.
/// Sonnet: 2,000 x 10 + 40,000 x 0.2 per million = 0.02 + 0.008 = 0.028.
/// Rebuild on sonnet: 40,000 x 2.5 per million = 0.10.
fn simple_prompt(session: &'static str, now: i64, messages: usize) -> Req {
    Req::prompt(now, messages).session(session).usage(Usage {
        output: 2_000,
        cache_read: 40_000,
        ..Usage::default()
    })
}

#[test]
fn simple_prompts_lead_to_one_suggestion_then_the_cooldown_holds() {
    let h = Harness::default();
    for (i, now) in [100, 200, 300].into_iter().enumerate() {
        simple_prompt("s1", now, 1 + 2 * i).send(&h.tracker);
    }
    let (prompts, suggestions) = split(h.sink.take());
    assert_eq!(prompts.len(), 2);
    assert!(suggestions.is_empty());
    let p = &prompts[0];
    assert!(p.simple);
    assert!(close(p.cost_usd, 0.14), "{}", p.cost_usd);
    assert!(
        close(p.est_saving_usd, 0.14 - (0.028 + 0.10)),
        "net of the rebuild: {}",
        p.est_saving_usd
    );
    let d = detail(p);
    assert!(close(d["target_cost_usd"].as_f64().unwrap(), 0.028));
    assert!(close(d["rebuild_usd"].as_f64().unwrap(), 0.10));
    assert!(close(d["gross_saving_usd"].as_f64().unwrap(), 0.112));
    assert_eq!(d["one_hour_cache"], false);

    // The third prompt closes when the fourth starts: 3 x 0.112 - 0.10 = 0.236.
    simple_prompt("s1", 400, 7).send(&h.tracker);
    let (prompts, suggestions) = split(h.sink.take());
    assert_eq!(prompts.len(), 1);
    assert_eq!(suggestions.len(), 1);
    let s = &suggestions[0];
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.ts, Timestamp(400));
    assert_eq!(s.id, "s1:300:3:suggestion");
    assert_eq!(
        s.message,
        "tokred: your last 3 prompts were simple; claude-sonnet-5-5 would have cost ~$0.24 less \
         (switch with /model sonnet)."
    );
    let evidence: Value = serde_json::from_str(&s.detail).unwrap();
    assert_eq!(
        evidence["prompt_ids"],
        json!(["s1:100:1", "s1:200:2", "s1:300:3"])
    );
    assert!(close(evidence["saving_usd"].as_f64().unwrap(), 0.236));

    // Three more simple prompts within the 6-hour cooldown: no new suggestion.
    for (i, now) in [500, 600, 700].into_iter().enumerate() {
        simple_prompt("s1", now, 9 + 2 * i).send(&h.tracker);
    }
    let (_, suggestions) = split(h.sink.take());
    assert!(suggestions.is_empty());
}

#[test]
fn a_haiku_main_loop_never_gets_a_suggestion() {
    let h = Harness::new(TrackerSettings {
        rule: tokred_suggest::SuggestRule {
            min_simple_prompts: 1,
            min_saving_usd: 0.0,
            ..TrackerSettings::default().rule
        },
        ..TrackerSettings::default()
    });
    for (i, now) in [100, 200, 300, 400].into_iter().enumerate() {
        Req {
            model: Some(HAIKU),
            ..simple_prompt("s1", now, 1 + 2 * i)
        }
        .send(&h.tracker);
    }
    let (prompts, suggestions) = split(h.sink.take());
    assert_eq!(prompts.len(), 3);
    assert!(suggestions.is_empty());
    assert!(prompts.iter().all(|p| p.simple && p.est_saving_usd == 0.0));
    assert_eq!(detail(&prompts[0])["target_model"], Value::Null);
}

#[test]
fn channel_off_still_summarizes_but_never_suggests() {
    let config = SuggestConfig {
        channel: SuggestChannel::Off,
        min_simple_prompts: 1,
        ..SuggestConfig::default()
    };
    let h = Harness::new(TrackerSettings::from_config(&config));
    for (i, now) in [100, 200, 300].into_iter().enumerate() {
        simple_prompt("s1", now, 1 + 2 * i).send(&h.tracker);
    }
    let (prompts, suggestions) = split(h.sink.take());
    assert_eq!(prompts.len(), 2);
    assert!(suggestions.is_empty());
}

#[test]
fn a_session_writing_one_hour_cache_rebuilds_at_the_one_hour_price() {
    let h = Harness::default();
    Req::prompt(100, 1)
        .usage(Usage {
            cache_write: 40_000,
            cache_write_1h: 40_000,
            ..Usage::default()
        })
        .send(&h.tracker);
    h.tracker.flush_all(Timestamp(200));
    let p = &h.sink.prompts()[0];
    let d = detail(p);
    assert_eq!(d["one_hour_cache"], true);
    assert_eq!(d["context_tokens"], 40_000);
    // 40,000 tokens at sonnet's 1-hour write price of 4 per million.
    assert!(close(d["rebuild_usd"].as_f64().unwrap(), 0.16));
}

#[test]
fn compaction_adds_cost_but_no_turns() {
    let h = Harness::default();
    Req::prompt(100, 1).send(&h.tracker);
    Req {
        origin: Origin::Compaction,
        tools: vec!["Edit"],
        ..Req::main(110, 3)
    }
    .send(&h.tracker);
    h.tracker.flush_all(Timestamp(200));
    let p = &h.sink.prompts()[0];
    assert_eq!(p.turns, 1);
    assert!(p.tools.is_empty() && !p.edited);
    assert_eq!(
        p.output_tokens, 100,
        "compaction output is not the prompt's work"
    );
    let d = detail(p);
    assert_eq!(d["compactions"], 1);
    // 100 output tokens on opus at 50 per million.
    assert!(close(d["compaction_cost_usd"].as_f64().unwrap(), 0.005));
    assert!(close(p.cost_usd, 0.01));
}

#[test]
fn unknown_models_are_counted_as_unpriced() {
    let h = Harness::default();
    Req {
        model: Some("<synthetic>"),
        ..Req::prompt(100, 1)
    }
    .send(&h.tracker);
    Req {
        model: None,
        ..Req::main(101, 3)
    }
    .send(&h.tracker);
    h.tracker.flush_all(Timestamp(200));
    let p = &h.sink.prompts()[0];
    assert_eq!(p.model, "");
    assert!(close(p.cost_usd, 0.0));
    assert_eq!(detail(p)["unpriced_requests"], 2);
    assert!(!p.est_saving_usd.is_nan());
}

#[test]
fn prices_can_be_replaced() {
    let h = Harness::default();
    h.tracker.set_prices(PriceTable::default());
    Req::prompt(100, 1).send(&h.tracker);
    h.tracker.flush_all(Timestamp(200));
    let p = &h.sink.prompts()[0];
    assert_eq!(detail(p)["unpriced_requests"], 1);
    assert!(format!("{:?}", h.tracker).starts_with("PromptTracker"));
}

#[test]
fn memory_is_bounded() {
    let h = Harness::new(TrackerSettings {
        max_sessions: 2,
        max_tools: 2,
        ..TrackerSettings::default()
    });
    Req::prompt(1, 1)
        .session("a")
        .tools(&["Read", "Grep", "Glob"])
        .send(&h.tracker);
    Req::prompt(2, 1).session("b").send(&h.tracker);
    assert_eq!(h.tracker.tracked_sessions(), 2);
    Req::prompt(3, 1).session("b").send(&h.tracker);
    assert!(h.sink.take().is_empty(), "b was retried, not replaced");
    // A third session evicts the least recently active one (a), emitting its prompt.
    Req::prompt(4, 1).session("c").send(&h.tracker);
    assert_eq!(h.tracker.tracked_sessions(), 2);
    let evicted = h.sink.prompts();
    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].session_id, "a");
    assert_eq!(evicted[0].tools, ["Read", "Grep"]);
    let d = detail(&evicted[0]);
    assert_eq!(d["closed_by"], "evicted");
    assert_eq!(d["tools_dropped"], 1);

    // An evicted session with no open prompt emits nothing.
    h.tracker.flush_all(Timestamp(10));
    h.sink.take();
    Req::main(20, 3).session("d").send(&h.tracker);
    assert!(h.sink.take().is_empty());
}
