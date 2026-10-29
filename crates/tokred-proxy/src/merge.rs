//! Merges what the store knows (history) with what the proxy holds in memory (live state), so a
//! session shows both: turns are matched by id and the live version wins, and a session that
//! only exists in memory (its first request is still in flight) still appears.

use tokred_core::Timestamp;
use tokred_core::dashboard::{
    MAX_TURNS, RECENT_SESSIONS, SessionDetail, SessionList, SessionStatus, SessionView, TurnView,
    session_status,
};

use crate::live::LiveSession;

/// A session built only from its live turns that have ended.
fn live_only(live: &LiveSession) -> SessionView {
    let mut view = SessionView {
        id: live.id.clone(),
        started_at: live.started_at,
        last_activity_at: live.last_activity_at,
        status: SessionStatus::Active,
        requests: 0,
        models: Vec::new(),
        cost_usd: 0.0,
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        subagents_running: 0,
        content_recorded: false,
    };
    for turn in live.turns.iter().filter(|turn| !turn.in_flight) {
        add_turn(&mut view, turn);
    }
    view
}

/// Counts an ended turn in a session.
fn add_turn(view: &mut SessionView, turn: &TurnView) {
    view.requests += 1;
    view.input += turn.usage.input;
    view.output += turn.usage.output;
    view.cache_read += turn.usage.cache_read;
    view.cache_write += turn.usage.cache_write;
    view.cost_usd += turn.cost_usd.unwrap_or(0.0);
    if !turn.model_effective.is_empty() && !view.models.contains(&turn.model_effective) {
        view.models.push(turn.model_effective.clone());
    }
}

/// Applies what only the live state knows: activity, subagents running, the status.
fn overlay(view: &mut SessionView, live: &LiveSession, now: Timestamp) {
    view.started_at = view.started_at.min(live.started_at);
    view.last_activity_at = view.last_activity_at.max(live.last_activity_at);
    view.subagents_running = live.subagents_running;
    view.content_recorded |= live.turns.iter().any(|turn| turn.sent.is_some());
    view.status = session_status(now, Timestamp(view.last_activity_at), live.in_flight);
}

/// The sessions of the store plus the live ones, bucketed by status: every session whose
/// status is `active` is in `active`, the others are the newest [`RECENT_SESSIONS`] of `recent`.
pub(crate) fn merge_list(db: SessionList, live: &[LiveSession], now: Timestamp) -> SessionList {
    let mut all: Vec<SessionView> = db.active.into_iter().chain(db.recent).collect();
    for session in live {
        if let Some(view) = all.iter_mut().find(|view| view.id == session.id) {
            overlay(view, session, now);
        } else {
            let mut view = live_only(session);
            overlay(&mut view, session, now);
            all.push(view);
        }
    }
    all.sort_by(|a, b| {
        b.last_activity_at
            .cmp(&a.last_activity_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let (active, mut recent): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|view| view.status == SessionStatus::Active);
    recent.truncate(RECENT_SESSIONS);
    SessionList { active, recent }
}

/// One session from the store (when it knows it) and the live state (when there is one).
pub(crate) fn merge_detail(
    db: Option<SessionDetail>,
    live: Option<&LiveSession>,
    now: Timestamp,
) -> Option<SessionDetail> {
    let (mut detail, live) = match (db, live) {
        (None, None) => return None,
        (Some(detail), None) => return Some(detail),
        (Some(detail), Some(live)) => (detail, live),
        (None, Some(live)) => {
            let session = live_only(live);
            let turns = live.turns.clone();
            return Some(finish(
                SessionDetail {
                    session,
                    turns,
                    truncated: false,
                },
                live,
                now,
            ));
        }
    };
    for turn in &live.turns {
        if let Some(at) = detail.turns.iter().position(|known| known.id == turn.id) {
            detail.turns[at] = turn.clone();
        } else {
            if !turn.in_flight {
                add_turn(&mut detail.session, turn);
            }
            detail.turns.push(turn.clone());
        }
    }
    Some(finish(detail, live, now))
}

fn finish(mut detail: SessionDetail, live: &LiveSession, now: Timestamp) -> SessionDetail {
    detail.turns.sort_by_key(|turn| turn.ts);
    if detail.turns.len() > MAX_TURNS {
        let excess = detail.turns.len() - MAX_TURNS;
        detail.turns.drain(..excess);
        detail.truncated = true;
    }
    overlay(&mut detail.session, live, now);
    detail
}

#[cfg(test)]
mod tests {
    use tokred_core::dashboard::{ResponseView, UsageView};

    use super::*;

    const NOW: Timestamp = Timestamp(10_000);

    fn view(id: &str, last: i64, status: SessionStatus) -> SessionView {
        SessionView {
            id: id.to_owned(),
            started_at: last - 50,
            last_activity_at: last,
            status,
            requests: 3,
            models: vec!["opus".to_owned()],
            cost_usd: 1.0,
            input: 10,
            output: 20,
            cache_read: 30,
            cache_write: 40,
            subagents_running: 0,
            content_recorded: false,
        }
    }

    fn turn(id: &str, ts: i64, in_flight: bool) -> TurnView {
        TurnView {
            id: id.to_owned(),
            session_id: "s".to_owned(),
            ts,
            origin: "main".to_owned(),
            agent_id: None,
            agent_type: None,
            model_requested: "opus".to_owned(),
            model_effective: "haiku".to_owned(),
            status: (!in_flight).then_some(200),
            stop_reason: None,
            usage: UsageView {
                input: 1,
                output: 2,
                cache_write: 3,
                cache_read: 4,
            },
            cost_usd: Some(0.5),
            duration_ms: None,
            ttfb_ms: None,
            tools: vec![],
            routing: None,
            in_flight,
            sent: None,
            response: None,
        }
    }

    fn live(id: &str, turns: Vec<TurnView>, subagents: u64) -> LiveSession {
        LiveSession {
            id: id.to_owned(),
            started_at: turns.first().map_or(0, |t| t.ts),
            last_activity_at: turns.last().map_or(0, |t| t.ts),
            in_flight: turns.iter().any(|t| t.in_flight),
            subagents_running: subagents,
            turns,
        }
    }

    fn ids(views: &[SessionView]) -> Vec<&str> {
        views.iter().map(|v| v.id.as_str()).collect()
    }

    #[test]
    fn a_live_session_missing_from_the_store_appears_as_active() {
        let list = merge_list(
            SessionList::default(),
            &[live("new", vec![turn("t1", 9_990, true)], 0)],
            NOW,
        );
        assert_eq!(ids(&list.active), ["new"]);
        assert!(list.recent.is_empty());
        assert_eq!(
            list.active[0].requests, 0,
            "a turn in flight is not counted yet"
        );
        assert_eq!(list.active[0].status, SessionStatus::Active);
    }

    #[test]
    fn a_live_session_in_the_store_keeps_its_totals_and_gains_live_facts() {
        let db = SessionList {
            active: vec![view("s", 9_900, SessionStatus::Active)],
            recent: vec![view("quiet", 5_000, SessionStatus::Ended)],
        };
        let mut running = live("s", vec![turn("t1", 9_990, true)], 2);
        running.turns[0].sent = Some(vec![]);
        let list = merge_list(db, &[running], NOW);
        let s = &list.active[0];
        assert_eq!((s.requests, s.cost_usd), (3, 1.0));
        assert_eq!(s.last_activity_at, 9_990);
        assert_eq!(s.subagents_running, 2);
        assert!(s.content_recorded);
        assert_eq!(ids(&list.recent), ["quiet"]);
    }

    #[test]
    fn the_buckets_follow_each_sessions_status_and_recent_is_capped() {
        let mut recent: Vec<SessionView> = (0..12)
            .map(|n| view(&format!("r{n:02}"), 8_000 - n * 10, SessionStatus::Ended))
            .collect();
        // The store thinks this one is idle; a request in flight makes it active.
        recent.push(view("woke", 1_000, SessionStatus::Ended));
        let db = SessionList {
            active: vec![],
            recent,
        };
        let list = merge_list(db, &[live("woke", vec![turn("t", 9_999, true)], 0)], NOW);
        assert_eq!(ids(&list.active), ["woke"]);
        assert_eq!(list.recent.len(), RECENT_SESSIONS);
        assert_eq!(list.recent[0].id, "r00", "newest first");
        assert!(
            list.recent
                .iter()
                .all(|s| s.status != SessionStatus::Active)
        );
    }

    #[test]
    fn with_nothing_live_the_lists_are_bucketed_by_the_stores_status() {
        let db = SessionList {
            active: vec![view("a", 9_900, SessionStatus::Active)],
            recent: vec![view("b", 5_000, SessionStatus::Idle)],
        };
        let list = merge_list(db.clone(), &[], NOW);
        assert_eq!(list, db);
    }

    #[test]
    fn a_detail_needs_the_store_or_the_live_state() {
        assert!(merge_detail(None, None, NOW).is_none());
        let stored = SessionDetail {
            session: view("s", 9_000, SessionStatus::Idle),
            turns: vec![turn("a", 1, false)],
            truncated: false,
        };
        assert_eq!(merge_detail(Some(stored.clone()), None, NOW), Some(stored));
    }

    #[test]
    fn a_live_only_detail_counts_the_turns_that_ended() {
        let l = live(
            "s",
            vec![turn("a", 9_900, false), turn("b", 9_950, true)],
            0,
        );
        let detail = merge_detail(None, Some(&l), NOW).unwrap();
        assert_eq!(detail.session.requests, 1);
        assert_eq!(detail.session.models, ["haiku"]);
        assert!((detail.session.cost_usd - 0.5).abs() < 1e-9);
        assert_eq!(detail.turns.len(), 2);
        assert_eq!(detail.session.status, SessionStatus::Active);
        assert_eq!(detail.session.started_at, 9_900);
    }

    #[test]
    fn live_turns_override_stored_ones_and_new_ones_are_counted_once() {
        let mut stale = turn("b", 9_950, false);
        stale.status = Some(500);
        let stored = SessionDetail {
            session: view("s", 9_950, SessionStatus::Active),
            turns: vec![turn("a", 9_900, false), stale],
            truncated: false,
        };
        let mut finished = turn("b", 9_950, false);
        finished.response = Some(ResponseView {
            text: "done".to_owned(),
            ..ResponseView::default()
        });
        // `b` is in the store already (its counters too); `c` is not; `d` is still in flight.
        let l = live(
            "s",
            vec![finished, turn("c", 9_960, false), turn("d", 9_970, true)],
            1,
        );
        let detail = merge_detail(Some(stored), Some(&l), NOW).unwrap();
        let shape: Vec<(&str, Option<u16>)> = detail
            .turns
            .iter()
            .map(|t| (t.id.as_str(), t.status))
            .collect();
        assert_eq!(
            shape,
            [
                ("a", Some(200)),
                ("b", Some(200)),
                ("c", Some(200)),
                ("d", None)
            ]
        );
        assert_eq!(detail.turns[1].response.as_ref().unwrap().text, "done");
        let s = &detail.session;
        assert_eq!(s.requests, 4, "three stored plus `c`");
        assert_eq!(s.models, ["opus", "haiku"]);
        assert_eq!(s.input, 11);
        assert_eq!(s.subagents_running, 1);
        assert_eq!(s.last_activity_at, 9_970);
    }

    #[test]
    fn a_detail_keeps_the_newest_turns_and_says_when_it_cut() {
        let many: Vec<TurnView> = (0..MAX_TURNS + 3)
            .map(|n| {
                turn(
                    &format!("t{n:04}"),
                    1_000 + i64::try_from(n).unwrap(),
                    false,
                )
            })
            .collect();
        let stored = SessionDetail {
            session: view("s", 9_000, SessionStatus::Idle),
            turns: many,
            truncated: false,
        };
        let l = live("s", vec![turn("live", 9_000, true)], 0);
        let detail = merge_detail(Some(stored), Some(&l), NOW).unwrap();
        assert_eq!(detail.turns.len(), MAX_TURNS);
        assert!(detail.truncated);
        assert_eq!(detail.turns.last().unwrap().id, "live");
        assert_eq!(detail.turns[0].id, "t0004");
    }
}
