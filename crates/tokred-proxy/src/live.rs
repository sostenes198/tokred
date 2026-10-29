//! What the proxy knows about its sessions right now, in memory: the turns in flight, the turns
//! that just ended (the database may not have them yet) and which subagents are running. The
//! dashboard merges this with the history it reads from the store.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use tokred_core::dashboard::{
    ENDED_AFTER_SECS, MAX_TEXT_CHARS, SUBAGENT_LINGER_SECS, SubagentEvent, SubagentState, TurnView,
};

/// Sessions remembered at once; the one idle longest is forgotten first.
const MAX_SESSIONS: usize = 256;
/// Ended turns kept per session after their `turn_end` (the store has them from then on).
const ENDED_TURNS_KEPT: usize = 40;

/// What the dashboard needs to know of one live session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveSession {
    pub id: String,
    pub started_at: i64,
    pub last_activity_at: i64,
    pub in_flight: bool,
    pub subagents_running: u64,
    /// In flight and recently ended turns, oldest first.
    pub turns: Vec<TurnView>,
}

#[derive(Debug)]
struct Agent {
    kind: Option<String>,
    in_flight: u32,
    last_seen: i64,
    running: bool,
}

#[derive(Debug)]
struct Session {
    started_at: i64,
    last_activity_at: i64,
    turns: VecDeque<TurnView>,
    agents: HashMap<String, Agent>,
}

impl Session {
    fn snapshot(&self, id: &str) -> LiveSession {
        LiveSession {
            id: id.to_owned(),
            started_at: self.started_at,
            last_activity_at: self.last_activity_at,
            in_flight: self.turns.iter().any(|turn| turn.in_flight),
            subagents_running: self.agents.values().filter(|agent| agent.running).count() as u64,
            turns: self.turns.iter().cloned().collect(),
        }
    }

    fn drop_old_ended_turns(&mut self) {
        let ended = self.turns.iter().filter(|turn| !turn.in_flight).count();
        let mut excess = ended.saturating_sub(ENDED_TURNS_KEPT);
        self.turns.retain(|turn| {
            let drop = excess > 0 && !turn.in_flight;
            excess -= usize::from(drop);
            !drop
        });
    }
}

fn event(
    session_id: &str,
    agent_id: &str,
    agent: &Agent,
    state: SubagentState,
    ts: i64,
) -> SubagentEvent {
    SubagentEvent {
        session_id: session_id.to_owned(),
        agent_id: agent_id.to_owned(),
        agent_type: agent.kind.clone(),
        state,
        ts,
    }
}

/// The in-memory state. Every method takes a short lock; none blocks on anything else.
#[derive(Debug, Default)]
pub(crate) struct Live {
    sessions: Mutex<HashMap<String, Session>>,
}

impl Live {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Session>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A turn began. Returns the `subagent` event when it made its agent start running.
    pub(crate) fn start_turn(&self, turn: TurnView, now: i64) -> Option<SubagentEvent> {
        let mut sessions = self.lock();
        if !sessions.contains_key(&turn.session_id) && sessions.len() >= MAX_SESSIONS {
            let idle = sessions
                .iter()
                .filter(|(_, s)| s.turns.iter().all(|t| !t.in_flight))
                .min_by_key(|(_, s)| s.last_activity_at)
                .map(|(id, _)| id.clone());
            if let Some(id) = idle {
                sessions.remove(&id);
            }
        }
        let session = sessions
            .entry(turn.session_id.clone())
            .or_insert_with(|| Session {
                started_at: turn.ts,
                last_activity_at: turn.ts,
                turns: VecDeque::new(),
                agents: HashMap::new(),
            });
        session.last_activity_at = session.last_activity_at.max(now);
        let started = turn.agent_id.clone().map(|agent_id| {
            let agent = session.agents.entry(agent_id.clone()).or_insert(Agent {
                kind: None,
                in_flight: 0,
                last_seen: now,
                running: false,
            });
            agent.in_flight += 1;
            agent.last_seen = now;
            agent.kind.clone_from(&turn.agent_type);
            let was_running = std::mem::replace(&mut agent.running, true);
            (!was_running).then(|| {
                event(
                    &turn.session_id,
                    &agent_id,
                    agent,
                    SubagentState::Running,
                    now,
                )
            })
        });
        session.turns.push_back(turn);
        started.flatten()
    }

    /// Text of the response arrived. Only the first [`MAX_TEXT_CHARS`] characters are kept.
    pub(crate) fn append_text(&self, session_id: &str, turn_id: &str, text: &str) {
        let mut sessions = self.lock();
        let response = sessions
            .get_mut(session_id)
            .and_then(|session| session.turns.iter_mut().find(|turn| turn.id == turn_id))
            .and_then(|turn| turn.response.as_mut());
        if let Some(response) = response {
            let room = MAX_TEXT_CHARS.saturating_sub(response.text.chars().count());
            response.text.extend(text.chars().take(room));
        }
    }

    /// A turn ended: `turn` replaces the one in flight.
    pub(crate) fn end_turn(&self, turn: TurnView, now: i64) {
        let mut sessions = self.lock();
        let Some(session) = sessions.get_mut(&turn.session_id) else {
            return;
        };
        session.last_activity_at = session.last_activity_at.max(now);
        if let Some(agent) = turn
            .agent_id
            .as_ref()
            .and_then(|agent_id| session.agents.get_mut(agent_id))
        {
            agent.in_flight = agent.in_flight.saturating_sub(1);
            agent.last_seen = now;
        }
        if let Some(slot) = session.turns.iter_mut().find(|t| t.id == turn.id) {
            *slot = turn;
        }
        session.drop_old_ended_turns();
    }

    /// Ends the `running` state of agents quiet for [`SUBAGENT_LINGER_SECS`] and forgets
    /// sessions that ended long ago (their history is in the store).
    pub(crate) fn sweep(&self, now: i64) -> Vec<SubagentEvent> {
        let mut sessions = self.lock();
        let mut finished = Vec::new();
        for (session_id, session) in sessions.iter_mut() {
            for (agent_id, agent) in &mut session.agents {
                if agent.running
                    && agent.in_flight == 0
                    && now - agent.last_seen > SUBAGENT_LINGER_SECS
                {
                    agent.running = false;
                    finished.push(event(
                        session_id,
                        agent_id,
                        agent,
                        SubagentState::Finished,
                        now,
                    ));
                }
            }
        }
        sessions.retain(|_, session| {
            session.turns.iter().any(|turn| turn.in_flight)
                || now - session.last_activity_at < ENDED_AFTER_SECS
        });
        finished
    }

    /// Every live session.
    pub(crate) fn sessions(&self) -> Vec<LiveSession> {
        self.lock()
            .iter()
            .map(|(id, session)| session.snapshot(id))
            .collect()
    }

    /// One live session.
    pub(crate) fn session(&self, id: &str) -> Option<LiveSession> {
        self.lock().get(id).map(|session| session.snapshot(id))
    }

    /// The response text so far of a turn in flight (tests).
    #[cfg(test)]
    fn response_of(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Option<tokred_core::dashboard::ResponseView> {
        self.session(session_id)?
            .turns
            .into_iter()
            .find(|turn| turn.id == turn_id)?
            .response
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::dashboard::{ResponseView, UsageView};

    use super::*;

    pub(crate) fn turn(id: &str, session: &str, ts: i64, agent: Option<&str>) -> TurnView {
        TurnView {
            id: id.to_owned(),
            session_id: session.to_owned(),
            ts,
            origin: if agent.is_some() { "subagent" } else { "main" }.to_owned(),
            agent_id: agent.map(str::to_owned),
            agent_type: agent.map(|_| "Explore".to_owned()),
            model_requested: "m".to_owned(),
            model_effective: "m".to_owned(),
            status: None,
            stop_reason: None,
            usage: UsageView::default(),
            cost_usd: None,
            duration_ms: None,
            ttfb_ms: None,
            tools: vec![],
            routing: None,
            in_flight: true,
            sent: None,
            response: Some(ResponseView::default()),
        }
    }

    fn at(base: i64, n: usize) -> i64 {
        base + i64::try_from(n).unwrap()
    }

    fn ended(mut turn: TurnView) -> TurnView {
        turn.in_flight = false;
        turn.status = Some(200);
        turn
    }

    #[test]
    fn a_session_exists_while_a_turn_is_in_flight_and_after_it_ended() {
        let live = Live::default();
        assert!(live.session("s").is_none());
        assert!(live.start_turn(turn("t1", "s", 100, None), 100).is_none());
        let during = live.session("s").unwrap();
        assert!(during.in_flight);
        assert_eq!((during.started_at, during.last_activity_at), (100, 100));
        assert_eq!(during.subagents_running, 0);

        live.end_turn(ended(turn("t1", "s", 100, None)), 105);
        let after = live.session("s").unwrap();
        assert!(!after.in_flight);
        assert_eq!(after.last_activity_at, 105);
        assert_eq!(after.turns.len(), 1);
        assert_eq!(after.turns[0].status, Some(200));
        assert_eq!(live.sessions().len(), 1);
        // Ending an unknown turn or session changes nothing.
        live.end_turn(ended(turn("zz", "s", 1, None)), 106);
        live.end_turn(ended(turn("zz", "other", 1, None)), 106);
        assert_eq!(live.session("s").unwrap().turns.len(), 1);
        assert!(live.session("other").is_none());
    }

    #[test]
    fn text_is_appended_to_the_turn_in_flight_up_to_the_limit() {
        let live = Live::default();
        live.start_turn(turn("t1", "s", 100, None), 100);
        live.append_text("s", "t1", "Hel");
        live.append_text("s", "t1", "lo");
        assert_eq!(live.response_of("s", "t1").unwrap().text, "Hello");
        live.append_text("s", "t1", &"é".repeat(MAX_TEXT_CHARS));
        let text = live.response_of("s", "t1").unwrap().text;
        assert_eq!(text.chars().count(), MAX_TEXT_CHARS);
        live.append_text("s", "t1", "more");
        assert_eq!(live.response_of("s", "t1").unwrap().text, text);
        // Unknown turns and turns without content recording ignore text.
        live.append_text("s", "nope", "x");
        live.append_text("nope", "t1", "x");
        let mut bare = turn("t2", "s", 101, None);
        bare.response = None;
        live.start_turn(bare, 101);
        live.append_text("s", "t2", "x");
        assert!(live.response_of("s", "t2").is_none());
    }

    #[test]
    fn subagents_run_while_in_flight_and_for_a_minute_after() {
        let live = Live::default();
        let start = live
            .start_turn(turn("t1", "s", 100, Some("a1")), 100)
            .unwrap();
        assert_eq!(
            (
                start.agent_id.as_str(),
                start.state,
                start.ts,
                start.agent_type.as_deref()
            ),
            ("a1", SubagentState::Running, 100, Some("Explore"))
        );
        // A second request of the same agent is not a new start.
        assert!(
            live.start_turn(turn("t2", "s", 101, Some("a1")), 101)
                .is_none()
        );
        assert_eq!(live.session("s").unwrap().subagents_running, 1);

        live.end_turn(ended(turn("t1", "s", 100, Some("a1"))), 110);
        assert!(live.sweep(200).is_empty(), "t2 is still in flight");
        live.end_turn(ended(turn("t2", "s", 101, Some("a1"))), 120);
        assert!(
            live.sweep(120 + SUBAGENT_LINGER_SECS).is_empty(),
            "still within the minute"
        );
        assert_eq!(live.session("s").unwrap().subagents_running, 1);
        let done = live.sweep(121 + SUBAGENT_LINGER_SECS);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].state, SubagentState::Finished);
        assert_eq!(live.session("s").unwrap().subagents_running, 0);
        assert!(live.sweep(1000).is_empty(), "finished once");

        // The agent comes back: running again.
        let again = live
            .start_turn(turn("t3", "s", 300, Some("a1")), 300)
            .unwrap();
        assert_eq!(again.state, SubagentState::Running);
    }

    #[test]
    fn old_sessions_are_forgotten_but_not_while_a_turn_is_in_flight() {
        let live = Live::default();
        live.start_turn(turn("t1", "quiet", 100, None), 100);
        live.end_turn(ended(turn("t1", "quiet", 100, None)), 100);
        live.start_turn(turn("t2", "busy", 100, None), 100);
        live.sweep(100 + ENDED_AFTER_SECS - 1);
        assert_eq!(live.sessions().len(), 2);
        live.sweep(100 + ENDED_AFTER_SECS);
        let left: Vec<String> = live.sessions().into_iter().map(|s| s.id).collect();
        assert_eq!(left, ["busy"]);
    }

    #[test]
    fn ended_turns_are_kept_in_a_bounded_number() {
        let live = Live::default();
        for n in 0..ENDED_TURNS_KEPT + 5 {
            let t = turn(&format!("t{n}"), "s", at(100, n), None);
            live.start_turn(t.clone(), 100);
            live.end_turn(ended(t), 100);
        }
        live.start_turn(turn("flying", "s", 500, None), 500);
        let turns = live.session("s").unwrap().turns;
        assert_eq!(turns.len(), ENDED_TURNS_KEPT + 1);
        assert_eq!(turns[0].id, "t5", "the oldest ended turns went first");
        assert!(turns.last().unwrap().in_flight);
    }

    #[test]
    fn the_number_of_sessions_is_bounded_and_the_longest_idle_goes_first() {
        let live = Live::default();
        for n in 0..MAX_SESSIONS {
            let id = format!("s{n}");
            let t = turn("t", &id, at(1000, n), None);
            live.start_turn(t.clone(), at(1000, n));
            live.end_turn(ended(t), at(1000, n));
        }
        live.start_turn(turn("t", "new", 5000, None), 5000);
        assert_eq!(live.sessions().len(), MAX_SESSIONS);
        assert!(live.session("s0").is_none());
        assert!(live.session("s1").is_some() && live.session("new").is_some());
        // With every session busy nothing is evicted: the new one is simply added.
        let busy = Live::default();
        for n in 0..=MAX_SESSIONS {
            busy.start_turn(turn("t", &format!("b{n}"), 1, None), 1);
        }
        assert_eq!(busy.sessions().len(), MAX_SESSIONS + 1);
    }
}
