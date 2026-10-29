//! The state behind the dashboard: the live view of the sessions, the event bus, the browser
//! sessions, and the life of a turn (start, text arriving, end) that feeds all three and writes
//! the [`CapturedTurn`] record. Nothing here runs unless a dashboard source was given to the
//! proxy ([`crate::Proxy::with_dashboard`]).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;
use tokred_core::Timestamp;
use tokred_core::dashboard::{
    CapturedTurn, DashboardSource, ResponseView, RoutingView, SessionView, TurnView, UsageView,
    origin_label,
};
use tokred_core::proxy::{ProxyRecord, ProxyRequestRecord, RecordSink, ResponseSummary};

use crate::auth::DashAuth;
use crate::bus::EventBus;
use crate::capture::SentTracker;
use crate::live::Live;
use crate::merge::merge_detail;
use crate::pipeline::Prepared;
use crate::server::Shutdown;

/// Routing decisions remembered for the turns that carry them.
const DECISIONS_KEPT: usize = 1024;

#[derive(Debug, Default)]
struct Memo {
    by_id: HashMap<String, RoutingView>,
    order: VecDeque<String>,
}

/// A [`RecordSink`] that passes every record on and remembers the outcome and reason of the
/// routing decisions that go by, so the dashboard can show them while a turn is still running.
/// Give the routing hook this sink instead of the plain one and hand it to
/// [`crate::Proxy::with_dashboard`] too.
pub struct DecisionTap {
    inner: Arc<dyn RecordSink>,
    memo: Mutex<Memo>,
}

impl DecisionTap {
    /// A tap in front of `inner`.
    #[must_use]
    pub fn new(inner: Arc<dyn RecordSink>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            memo: Mutex::new(Memo::default()),
        })
    }

    fn lookup(&self, decision_id: &str) -> Option<RoutingView> {
        let memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
        memo.by_id.get(decision_id).cloned()
    }
}

impl std::fmt::Debug for DecisionTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionTap").finish_non_exhaustive()
    }
}

impl RecordSink for DecisionTap {
    fn record(&self, record: ProxyRecord) {
        if let ProxyRecord::Decision(decision) = &record {
            let mut memo = self.memo.lock().unwrap_or_else(PoisonError::into_inner);
            let view = RoutingView {
                outcome: decision.outcome.clone(),
                reason: decision.reason.clone(),
            };
            if memo.by_id.insert(decision.id.clone(), view).is_none() {
                memo.order.push_back(decision.id.clone());
                if memo.order.len() > DECISIONS_KEPT
                    && let Some(oldest) = memo.order.pop_front()
                {
                    memo.by_id.remove(&oldest);
                }
            }
        }
        self.inner.record(record);
    }
}

/// A turn the dashboard is following, kept by the exchange until its response ends.
#[derive(Debug)]
pub(crate) struct Tracked {
    pub session_id: String,
    base: TurnView,
}

impl Tracked {
    pub(crate) fn turn_id(&self) -> &str {
        &self.base.id
    }
}

/// Everything the dashboard routes and the request path share.
pub(crate) struct Dashboard {
    pub(crate) source: Arc<dyn DashboardSource>,
    /// Contents are captured (`[proxy] log_bodies`).
    pub(crate) capture: bool,
    pub(crate) bus: EventBus,
    pub(crate) live: Live,
    pub(crate) auth: DashAuth,
    pub(crate) shutdown: Shutdown,
    sent: Mutex<SentTracker>,
    decisions: Option<Arc<DecisionTap>>,
}

impl Dashboard {
    pub(crate) fn new(
        source: Arc<dyn DashboardSource>,
        capture: bool,
        decisions: Option<Arc<DecisionTap>>,
        shutdown: Shutdown,
    ) -> Self {
        Self {
            source,
            capture,
            bus: EventBus::new(),
            live: Live::default(),
            auth: DashAuth::default(),
            shutdown,
            sent: Mutex::new(SentTracker::default()),
            decisions,
        }
    }

    /// A request is about to be sent upstream. Returns what to carry until its response ends;
    /// `None` for a request without a session id (it is not followed).
    pub(crate) fn begin(
        self: &Arc<Self>,
        id: &str,
        ts: Timestamp,
        prepared: &Prepared,
    ) -> Option<Tracked> {
        let session_id = prepared
            .meta
            .session_id
            .as_deref()
            .filter(|id| !id.is_empty())?;
        let agent_id = prepared.meta.agent_id.as_deref();
        let sent = if self.capture {
            let mut tracker = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
            prepared
                .body
                .as_ref()
                .map(|body| tracker.new_messages(session_id, agent_id, body))
        } else {
            None
        };
        let requested = prepared.shape.model.clone().unwrap_or_default();
        let model_sent = prepared
            .body
            .as_ref()
            .and_then(|body| body.get("model"))
            .and_then(Value::as_str)
            .map_or_else(|| requested.clone(), str::to_owned);
        let routing = prepared
            .notes
            .decision_id
            .as_deref()
            .zip(self.decisions.as_ref())
            .and_then(|(decision, tap)| tap.lookup(decision));
        let turn = TurnView {
            id: id.to_owned(),
            session_id: session_id.to_owned(),
            ts: ts.0,
            origin: origin_label(prepared.origin.name()).to_owned(),
            agent_id: agent_id.map(str::to_owned),
            agent_type: prepared.meta.agent_type.clone(),
            model_requested: requested,
            model_effective: model_sent,
            status: None,
            stop_reason: None,
            usage: UsageView::default(),
            cost_usd: None,
            duration_ms: None,
            ttfb_ms: None,
            tools: Vec::new(),
            routing,
            in_flight: true,
            sent,
            response: self.capture.then(ResponseView::default),
        };
        if let Some(started) = self.live.start_turn(turn.clone(), ts.0) {
            self.bus.publish("subagent", session_id, &started);
        }
        if self.bus.has_subscribers() {
            let announced = TurnView {
                response: None,
                ..turn.clone()
            };
            self.bus.publish("turn_start", session_id, &announced);
            self.announce_session(session_id, ts);
        }
        Some(Tracked {
            session_id: session_id.to_owned(),
            base: turn,
        })
    }

    /// Text of the response arrived.
    pub(crate) fn delta(&self, tracked: &Tracked, text: String) {
        self.live
            .append_text(&tracked.session_id, tracked.turn_id(), &text);
        self.bus.publish(
            "turn_delta",
            &tracked.session_id,
            &tokred_core::dashboard::TurnDelta {
                session_id: tracked.session_id.clone(),
                turn_id: tracked.turn_id().to_owned(),
                text,
            },
        );
    }

    /// The response ended (or the request failed): the turn is final.
    pub(crate) fn end(
        self: &Arc<Self>,
        tracked: Tracked,
        record: &ProxyRequestRecord,
        summary: &ResponseSummary,
        content: Option<ResponseView>,
        sink: &dyn RecordSink,
    ) {
        let mut turn = tracked.base;
        let ok = (200..300).contains(&summary.status);
        if let Some(model) = summary.model.as_ref().filter(|model| !model.is_empty()) {
            model.clone_into(&mut turn.model_effective);
        }
        turn.status = Some(summary.status);
        turn.stop_reason.clone_from(&summary.stop_reason);
        turn.usage = UsageView::from(&summary.usage);
        turn.cost_usd = ok
            .then(|| self.source.cost_usd(&turn.model_effective, &summary.usage))
            .flatten();
        turn.duration_ms = Some(summary.total_ms);
        turn.ttfb_ms = Some(summary.ttfb_ms);
        turn.tools.clone_from(&summary.tool_uses);
        turn.in_flight = false;
        turn.response = content;
        sink.record(ProxyRecord::Captured(Box::new(CapturedTurn {
            request_id: turn.id.clone(),
            session_id: turn.session_id.clone(),
            agent_id: turn.agent_id.clone(),
            ts: record.ts,
            stop_reason: turn.stop_reason.clone(),
            tools_json: to_json(&turn.tools).unwrap_or_default(),
            sent_json: turn.sent.as_ref().and_then(to_json),
            response_json: turn.response.as_ref().and_then(to_json),
        })));
        let session_id = turn.session_id.clone();
        let now = Timestamp(record.ts.0 + i64::try_from(summary.total_ms / 1000).unwrap_or(0));
        self.live.end_turn(turn.clone(), now.0);
        if self.bus.has_subscribers() {
            self.bus.publish("turn_end", &session_id, &turn);
            self.announce_session(&session_id, now);
        }
    }

    /// Publishes the session (merged with the store) as a `session` event, off the request
    /// path. Needs a tokio runtime; does nothing without one.
    fn announce_session(self: &Arc<Self>, session_id: &str, now: Timestamp) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let this = self.clone();
        let id = session_id.to_owned();
        runtime.spawn(async move {
            if let Some(view) = this.session_view(&id, now).await {
                this.bus.publish("session", &id, &view);
            }
        });
    }

    async fn session_view(&self, id: &str, now: Timestamp) -> Option<SessionView> {
        let source = self.source.clone();
        let owned = id.to_owned();
        let stored = tokio::task::spawn_blocking(move || source.session(&owned))
            .await
            .ok()?
            .ok()
            .flatten();
        merge_detail(stored, self.live.session(id).as_ref(), now).map(|detail| detail.session)
    }

    /// Periodic upkeep: subagents that went quiet finish, spent tokens and old browser
    /// sessions are forgotten.
    pub(crate) fn sweep(&self, now: Timestamp) {
        for finished in self.live.sweep(now.0) {
            self.bus
                .publish("subagent", &finished.session_id, &finished);
        }
        self.auth.sweep(Instant::now());
    }
}

/// A value as JSON text; a failure (which a plain value cannot have) reads as absent.
fn to_json<T: Serialize + ?Sized>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}

/// Runs the periodic upkeep until the proxy stops.
pub(crate) async fn upkeep(
    dashboard: Arc<Dashboard>,
    clock: Arc<dyn tokred_core::ports::Clock + Send + Sync>,
) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        tokio::select! {
            () = dashboard.shutdown.wait() => return,
            _ = tick.tick() => dashboard.sweep(clock.now()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use bytes::Bytes;
    use hyper::HeaderMap;
    use hyper::header::HeaderValue;
    use tokred_core::dashboard::{SessionDetail, SessionList};
    use tokred_core::pricing::Usage;
    use tokred_core::routing::RoutingDecision;

    use super::*;
    use crate::config::SystemClock;
    use crate::pipeline::prepare;

    struct Source;

    impl DashboardSource for Source {
        fn sessions(&self) -> Result<SessionList, String> {
            Ok(SessionList::default())
        }

        fn session(&self, _id: &str) -> Result<Option<SessionDetail>, String> {
            Ok(None)
        }

        fn metrics_html(&self) -> Result<String, String> {
            Err("none".to_owned())
        }

        fn cost_usd(&self, model: &str, _usage: &Usage) -> Option<f64> {
            (model != "unpriced").then_some(0.25)
        }
    }

    #[derive(Default)]
    struct Collect(Mutex<Vec<ProxyRecord>>);

    impl Collect {
        fn captured(&self) -> Vec<CapturedTurn> {
            let mut out = Vec::new();
            for record in self.0.lock().unwrap().iter() {
                if let ProxyRecord::Captured(turn) = record {
                    out.push((**turn).clone());
                }
            }
            out
        }
    }

    impl RecordSink for Collect {
        fn record(&self, record: ProxyRecord) {
            self.0.lock().unwrap().push(record);
        }
    }

    fn dashboard(capture: bool, tap: Option<Arc<DecisionTap>>) -> Arc<Dashboard> {
        Arc::new(Dashboard::new(
            Arc::new(Source),
            capture,
            tap,
            Shutdown::new(),
        ))
    }

    fn prepared(
        session: Option<&str>,
        agent: Option<&str>,
        body: &str,
        decision: Option<&str>,
    ) -> Prepared {
        let mut headers = HeaderMap::new();
        if let Some(session) = session {
            headers.insert(
                "x-claude-code-session-id",
                HeaderValue::from_str(session).unwrap(),
            );
        }
        if let Some(agent) = agent {
            headers.insert(
                "x-claude-code-agent-id",
                HeaderValue::from_str(agent).unwrap(),
            );
        }
        let mut prepared = prepare(&headers, &Bytes::from(body.to_owned()), &[], Timestamp(100));
        prepared.notes.decision_id = decision.map(str::to_owned);
        prepared
    }

    const BODY: &str = r#"{"model":"claude-opus-5-5","tools":[{"name":"Read"}],"messages":[{"role":"user","content":"hi"}]}"#;

    fn summary(status: u16, model: Option<&str>) -> ResponseSummary {
        ResponseSummary {
            status,
            model: model.map(str::to_owned),
            usage: Usage {
                input: 1,
                output: 2,
                ..Usage::default()
            },
            total_ms: 2500,
            ttfb_ms: 100,
            tool_uses: vec!["Read".to_owned()],
            stop_reason: Some("end_turn".to_owned()),
            ..ResponseSummary::default()
        }
    }

    fn record(id: &str) -> ProxyRequestRecord {
        ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(100),
            ..ProxyRequestRecord::default()
        }
    }

    #[test]
    fn the_test_source_answers() {
        assert_eq!(Source.sessions().unwrap(), SessionList::default());
        assert!(Source.session("x").unwrap().is_none());
        assert!(Source.metrics_html().is_err());
    }

    #[test]
    fn a_request_without_a_session_is_not_followed() {
        let dash = dashboard(false, None);
        assert!(
            dash.begin("r1", Timestamp(100), &prepared(None, None, BODY, None))
                .is_none()
        );
        assert!(
            dash.begin("r1", Timestamp(100), &prepared(Some(""), None, BODY, None))
                .is_none()
        );
        assert!(dash.live.sessions().is_empty());
    }

    #[test]
    fn a_body_that_is_not_json_is_still_a_turn_without_messages() {
        let dash = dashboard(true, None);
        let tracked = dash
            .begin(
                "r1",
                Timestamp(100),
                &prepared(Some("s"), None, "not json", None),
            )
            .unwrap();
        let turn = &dash.live.session("s").unwrap().turns[0];
        assert_eq!(turn.id, tracked.turn_id());
        assert_eq!(turn.sent, None);
        assert_eq!(turn.model_requested, "");
        assert_eq!(turn.response, Some(ResponseView::default()));
    }

    #[test]
    fn a_turn_goes_from_in_flight_to_final_and_leaves_a_record() {
        let dash = dashboard(true, None);
        let sink = Collect::default();
        let tracked = dash
            .begin("r1", Timestamp(100), &prepared(Some("s"), None, BODY, None))
            .unwrap();
        let started = dash.live.session("s").unwrap().turns[0].clone();
        assert!(started.in_flight && started.status.is_none());
        assert_eq!(
            started.model_effective, "claude-opus-5-5",
            "the model sent, until one answers"
        );
        assert_eq!(started.sent.as_ref().unwrap().len(), 1);

        dash.delta(&tracked, "Hel".to_owned());
        dash.delta(&tracked, "lo".to_owned());
        assert_eq!(
            dash.live.session("s").unwrap().turns[0]
                .response
                .as_ref()
                .unwrap()
                .text,
            "Hello"
        );

        let content = ResponseView {
            text: "Hello".to_owned(),
            ..ResponseView::default()
        };
        dash.end(
            tracked,
            &record("r1"),
            &summary(200, Some("claude-haiku-5-5")),
            Some(content),
            &sink,
        );
        let ended = dash.live.session("s").unwrap();
        let turn = &ended.turns[0];
        assert!(!turn.in_flight);
        assert_eq!(turn.model_effective, "claude-haiku-5-5");
        assert_eq!(turn.cost_usd, Some(0.25));
        assert_eq!(
            (turn.duration_ms, turn.ttfb_ms, turn.status),
            (Some(2500), Some(100), Some(200))
        );
        assert_eq!(turn.tools, ["Read"]);
        assert_eq!(
            ended.last_activity_at, 102,
            "the request time plus the whole seconds it took"
        );

        let captured = sink.captured().remove(0);
        assert_eq!(
            (captured.request_id.as_str(), captured.session_id.as_str()),
            ("r1", "s")
        );
        assert_eq!(captured.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(captured.tools_json, r#"["Read"]"#);
        assert!(captured.sent_json.as_ref().unwrap().contains("hi"));
        assert!(captured.response_json.as_ref().unwrap().contains("Hello"));
    }

    #[test]
    fn failed_or_unpriced_turns_have_no_cost_and_unrecorded_content_stays_out() {
        let dash = dashboard(false, None);
        let sink = Collect::default();
        for (id, status, model) in [
            ("a", 429, Some("claude-opus-5-5")),
            ("b", 200, Some("unpriced")),
        ] {
            let tracked = dash
                .begin(id, Timestamp(100), &prepared(Some("s"), None, BODY, None))
                .unwrap();
            dash.end(tracked, &record(id), &summary(status, model), None, &sink);
        }
        let turns = dash.live.session("s").unwrap().turns;
        assert!(
            turns
                .iter()
                .all(|t| t.cost_usd.is_none() && t.sent.is_none() && t.response.is_none())
        );
        let all = sink.captured();
        assert_eq!(all.len(), 2);
        assert!(
            all.iter()
                .all(|c| c.sent_json.is_none() && c.response_json.is_none())
        );
    }

    #[test]
    fn subscribers_hear_about_turns_and_subagents_even_without_a_runtime() {
        let dash = dashboard(true, None);
        let mut events = dash.bus.subscribe();
        let sink = Collect::default();
        let tracked = dash
            .begin(
                "r1",
                Timestamp(100),
                &prepared(Some("s"), Some("agent-1"), BODY, None),
            )
            .unwrap();
        dash.delta(&tracked, "x".to_owned());
        dash.end(tracked, &record("r1"), &summary(200, None), None, &sink);
        let mut names = Vec::new();
        while let Ok(event) = events.try_recv() {
            let text = String::from_utf8(event.frame.to_vec()).unwrap();
            names.push(text.lines().nth(1).unwrap().to_owned());
        }
        assert_eq!(
            names,
            [
                "event: subagent",
                "event: turn_start",
                "event: turn_delta",
                "event: turn_end"
            ],
            "the session events need a runtime"
        );
    }

    #[test]
    fn quiet_subagents_finish_on_the_sweep() {
        let dash = dashboard(false, None);
        let mut events = dash.bus.subscribe();
        let sink = Collect::default();
        let tracked = dash
            .begin(
                "r1",
                Timestamp(100),
                &prepared(Some("s"), Some("agent-1"), BODY, None),
            )
            .unwrap();
        dash.end(tracked, &record("r1"), &summary(200, None), None, &sink);
        while events.try_recv().is_ok() {}
        dash.sweep(Timestamp(110));
        assert!(events.try_recv().is_err(), "still within the minute");
        dash.sweep(Timestamp(200));
        let text = String::from_utf8(events.try_recv().unwrap().frame.to_vec()).unwrap();
        assert!(
            text.contains("event: subagent") && text.contains("\"state\":\"finished\""),
            "{text}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_upkeep_sweeps_every_few_seconds_and_stops_with_the_proxy() {
        let dash = dashboard(false, None);
        let mut events = dash.bus.subscribe();
        let sink = Collect::default();
        // A subagent that was last seen long before now.
        let tracked = dash
            .begin(
                "r1",
                Timestamp(100),
                &prepared(Some("s"), Some("agent-1"), BODY, None),
            )
            .unwrap();
        dash.end(tracked, &record("r1"), &summary(200, None), None, &sink);
        while events.try_recv().is_ok() {}
        let task = tokio::spawn(upkeep(dash.clone(), Arc::new(SystemClock)));
        let finished = events.recv().await.unwrap();
        assert!(String::from_utf8_lossy(&finished.frame).contains("finished"));
        dash.shutdown.trigger();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn the_routing_decision_of_a_turn_comes_from_the_tap() {
        let collect = Arc::new(Collect::default());
        let tap = DecisionTap::new(collect.clone());
        let decision = |id: &str, outcome: &str| {
            ProxyRecord::Decision(Box::new(RoutingDecision {
                id: id.to_owned(),
                outcome: outcome.to_owned(),
                reason: format!("because {outcome}"),
                ..RoutingDecision::default()
            }))
        };
        tap.record(decision("d1", "observed"));
        tap.record(ProxyRecord::Request(Box::default()));
        assert_eq!(
            collect.0.lock().unwrap().len(),
            2,
            "every record is passed on"
        );
        tap.record(decision("d1", "fallback"));
        assert_eq!(
            tap.lookup("d1"),
            Some(RoutingView {
                outcome: "fallback".to_owned(),
                reason: "because fallback".to_owned()
            }),
            "the last version wins"
        );
        assert_eq!(tap.lookup("nope"), None);
        assert!(format!("{tap:?}").contains("DecisionTap"));

        let dash = dashboard(false, Some(tap.clone()));
        dash.begin(
            "r1",
            Timestamp(100),
            &prepared(Some("s"), None, BODY, Some("d1")),
        )
        .unwrap();
        dash.begin(
            "r2",
            Timestamp(100),
            &prepared(Some("s"), None, BODY, Some("unknown")),
        )
        .unwrap();
        dash.begin("r3", Timestamp(100), &prepared(Some("s"), None, BODY, None))
            .unwrap();
        let turns = dash.live.session("s").unwrap().turns;
        assert_eq!(turns[0].routing.as_ref().unwrap().outcome, "fallback");
        assert!(turns[1].routing.is_none() && turns[2].routing.is_none());
    }

    #[test]
    fn the_tap_remembers_a_bounded_number_of_decisions() {
        let tap = DecisionTap::new(Arc::new(Collect::default()));
        for n in 0..=DECISIONS_KEPT {
            tap.record(ProxyRecord::Decision(Box::new(RoutingDecision {
                id: format!("d{n}"),
                ..RoutingDecision::default()
            })));
        }
        assert!(tap.lookup("d0").is_none());
        assert!(tap.lookup("d1").is_some() && tap.lookup(&format!("d{DECISIONS_KEPT}")).is_some());
    }
}
