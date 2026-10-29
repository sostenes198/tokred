//! Cache-miss explanation: when a main-loop request reads much less from the prompt
//! cache than the previous one in its session left there, name the likely cause by comparing
//! the two requests. Only segment hashes and counts are kept, in memory, for a bounded number
//! of sessions.

use std::collections::HashMap;

use tokred_core::Timestamp;
use tokred_core::pricing::Usage;

/// Default cache lifetime in seconds.
const TTL_5M: i64 = 5 * 60;
/// Lifetime of a 1-hour cache write in seconds.
const TTL_1H: i64 = 60 * 60;
/// Below this many cached tokens there was no cache worth explaining.
const MIN_CACHED_TOKENS: u64 = 1024;

/// Likely reason a main-loop request missed the prompt cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissCause {
    /// More time than the cache lifetime passed since the previous request.
    IdleTtl,
    /// The conversation was compacted (headers, or fewer messages than before).
    Compaction,
    /// The model changed (each model has its own cache).
    ModelChanged,
    /// The `tools` block changed.
    ToolsChanged,
    /// The `system` block changed.
    SystemChanged,
    /// The thinking configuration or the effort changed.
    ThinkingChanged,
    /// None of the above.
    Unknown,
}

impl MissCause {
    /// Name stored in `proxy_requests.miss_cause`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdleTtl => "idle_ttl",
            Self::Compaction => "compaction",
            Self::ModelChanged => "model_changed",
            Self::ToolsChanged => "tools_changed",
            Self::SystemChanged => "system_changed",
            Self::ThinkingChanged => "thinking_changed",
            Self::Unknown => "unknown",
        }
    }
}

/// What the explanation compares between two main-loop requests of a session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MainTurn {
    /// When the request arrived.
    pub at: Timestamp,
    /// Model requested.
    pub model: String,
    /// Hash of `tools`.
    pub tools_hash: String,
    /// Hash of `system`.
    pub system_hash: String,
    /// Hash of the thinking configuration and effort.
    pub thinking_signature: String,
    /// Entries in `messages`.
    pub message_count: usize,
    /// The request carried a compaction header.
    pub compacted: bool,
    /// The session has written 1-hour cache entries (this request or an earlier one).
    pub uses_1h_cache: bool,
    /// Usage of the response.
    pub usage: Usage,
}

/// The likely cause of a cache miss on `current`, or `None` when its cache read did not
/// collapse (or there is no previous request to compare with).
///
/// A collapse is a cache read under half of what the previous request read and wrote, when
/// that was at least 1,024 tokens. Causes are checked in order: compaction, idle beyond the
/// cache lifetime (1 hour once the session wrote 1-hour entries, else 5 minutes), model,
/// tools, system, thinking/effort, unknown.
#[must_use]
pub fn explain_cache_miss(previous: Option<&MainTurn>, current: &MainTurn) -> Option<MissCause> {
    let previous = previous?;
    let expected = previous.usage.cache_read + previous.usage.cache_write;
    if expected < MIN_CACHED_TOKENS || current.usage.cache_read * 2 >= expected {
        return None;
    }
    let ttl = if previous.uses_1h_cache {
        TTL_1H
    } else {
        TTL_5M
    };
    // A model has a cache of its own, so a shorter conversation only says "compacted" when the
    // previous request was on the same model.
    let shorter = current.model == previous.model && current.message_count < previous.message_count;
    let cause = if current.compacted || shorter {
        MissCause::Compaction
    } else if current.at.0 - previous.at.0 > ttl {
        MissCause::IdleTtl
    } else if current.model != previous.model {
        MissCause::ModelChanged
    } else if current.tools_hash != previous.tools_hash {
        MissCause::ToolsChanged
    } else if current.system_hash != previous.system_hash {
        MissCause::SystemChanged
    } else if current.thinking_signature != previous.thinking_signature {
        MissCause::ThinkingChanged
    } else {
        MissCause::Unknown
    };
    Some(cause)
}

/// Lanes remembered per session (the least recently used one is forgotten first).
const LANES_PER_SESSION: usize = 8;

/// The recent main-loop requests of each session, one per lane, for at most `cap` sessions (the
/// least recently seen session is forgotten first).
///
/// A lane is a model with a system prompt. Claude Code can send several kinds of request that the
/// proxy sees as the main loop (the conversation and side queries that reuse its context), each
/// with its own cache. Comparing a request with the last request of *another* lane would call
/// every switch between them a miss, so a request is compared with the previous one of its own
/// lane; only the first request of a lane has to look elsewhere (see [`SessionTracker::observe`]).
#[derive(Debug)]
pub(crate) struct SessionTracker {
    cap: usize,
    /// Per session, the last request of each lane, the most recently used last.
    sessions: HashMap<String, Vec<MainTurn>>,
}

impl SessionTracker {
    /// An empty tracker keeping at most `cap` sessions (at least one).
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            sessions: HashMap::new(),
        }
    }

    /// Explains `turn` against the previous request of its own lane (same model and system
    /// prompt), else the last one on the same model (the system prompt changed), else the last one
    /// of the session (the model changed), and remembers it.
    pub(crate) fn observe(&mut self, session: &str, mut turn: MainTurn) -> Option<MissCause> {
        if !self.sessions.contains_key(session) && self.sessions.len() >= self.cap {
            let oldest = self
                .sessions
                .iter()
                .filter_map(|(id, lanes)| lanes.last().map(|last| (id, last.at)))
                .min_by_key(|(_, at)| *at)
                .map(|(id, _)| id.clone());
            oldest.map(|id| self.sessions.remove(&id));
        }
        let lanes = self.sessions.entry(session.to_owned()).or_default();
        turn.uses_1h_cache |= lanes.iter().any(|lane| lane.uses_1h_cache);
        let own = lanes
            .iter()
            .rposition(|lane| lane.model == turn.model && lane.system_hash == turn.system_hash);
        let previous = own
            .and_then(|at| lanes.get(at))
            .or_else(|| lanes.iter().rev().find(|lane| lane.model == turn.model))
            .or_else(|| lanes.last());
        let cause = explain_cache_miss(previous, &turn);
        if let Some(at) = own {
            lanes.remove(at);
        }
        lanes.push(turn);
        if lanes.len() > LANES_PER_SESSION {
            lanes.remove(0);
        }
        cause
    }

    /// Sessions currently remembered.
    pub(crate) fn len(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(at: i64, cache_read: u64, cache_write: u64) -> MainTurn {
        MainTurn {
            at: Timestamp(at),
            model: "opus".to_owned(),
            tools_hash: "t".to_owned(),
            system_hash: "s".to_owned(),
            thinking_signature: "k".to_owned(),
            message_count: 10,
            usage: Usage {
                cache_read,
                cache_write,
                ..Usage::default()
            },
            ..MainTurn::default()
        }
    }

    #[test]
    fn no_previous_request_or_no_collapse_means_no_cause() {
        let previous = turn(0, 40_000, 2_000);
        assert_eq!(explain_cache_miss(None, &turn(10, 0, 9_000)), None);
        assert_eq!(
            explain_cache_miss(Some(&previous), &turn(10, 42_000, 500)),
            None
        );
        assert_eq!(
            explain_cache_miss(Some(&previous), &turn(10, 21_000, 0)),
            None,
            "half is not a collapse"
        );
        assert_eq!(
            explain_cache_miss(Some(&turn(0, 500, 500)), &turn(10, 0, 0)),
            None,
            "too little was cached to explain"
        );
    }

    #[test]
    fn causes_are_checked_in_order() {
        let previous = turn(0, 40_000, 2_000);
        let missed = |change: &dyn Fn(&mut MainTurn)| {
            let mut current = turn(60, 0, 42_000);
            change(&mut current);
            explain_cache_miss(Some(&previous), &current)
        };
        assert_eq!(
            missed(&|t| {
                t.compacted = true;
                t.at = Timestamp(10_000);
            }),
            Some(MissCause::Compaction)
        );
        assert_eq!(
            missed(&|t| t.message_count = 3),
            Some(MissCause::Compaction)
        );
        assert_eq!(
            missed(&|t| {
                t.at = Timestamp(301);
                t.model = "sonnet".to_owned();
            }),
            Some(MissCause::IdleTtl)
        );
        assert_eq!(
            missed(&|t| {
                t.model = "sonnet".to_owned();
                t.tools_hash = "x".to_owned();
            }),
            Some(MissCause::ModelChanged)
        );
        assert_eq!(
            missed(&|t| {
                t.tools_hash = "x".to_owned();
                t.system_hash = "x".to_owned();
            }),
            Some(MissCause::ToolsChanged)
        );
        assert_eq!(
            missed(&|t| {
                t.system_hash = "x".to_owned();
                t.thinking_signature = "x".to_owned();
            }),
            Some(MissCause::SystemChanged)
        );
        assert_eq!(
            missed(&|t| t.thinking_signature = "x".to_owned()),
            Some(MissCause::ThinkingChanged)
        );
        assert_eq!(missed(&|_| {}), Some(MissCause::Unknown));
    }

    #[test]
    fn the_idle_limit_is_one_hour_once_the_session_wrote_1h_entries() {
        let mut previous = turn(0, 40_000, 2_000);
        previous.uses_1h_cache = true;
        assert_eq!(
            explain_cache_miss(Some(&previous), &turn(1_800, 0, 42_000)),
            Some(MissCause::Unknown)
        );
        assert_eq!(
            explain_cache_miss(Some(&previous), &turn(3_601, 0, 42_000)),
            Some(MissCause::IdleTtl)
        );
    }

    #[test]
    fn cause_names() {
        let names: Vec<&str> = [
            MissCause::IdleTtl,
            MissCause::Compaction,
            MissCause::ModelChanged,
            MissCause::ToolsChanged,
            MissCause::SystemChanged,
            MissCause::ThinkingChanged,
            MissCause::Unknown,
        ]
        .into_iter()
        .map(MissCause::as_str)
        .collect();
        assert_eq!(
            names,
            [
                "idle_ttl",
                "compaction",
                "model_changed",
                "tools_changed",
                "system_changed",
                "thinking_changed",
                "unknown"
            ]
        );
    }

    #[test]
    fn the_tracker_compares_with_the_previous_turn_and_keeps_1h_sticky() {
        let mut tracker = SessionTracker::new(10);
        let mut first = turn(0, 0, 40_000);
        first.uses_1h_cache = true;
        assert_eq!(tracker.observe("s", first), None);
        assert_eq!(tracker.observe("s", turn(60, 40_000, 100)), None);
        assert_eq!(
            tracker.observe("s", turn(2_000, 0, 40_100)),
            Some(MissCause::Unknown),
            "1-hour lifetime still applies although the last turn wrote nothing"
        );
        assert_eq!(tracker.observe("other", turn(2_000, 0, 9)), None);
        assert_eq!(tracker.len(), 2);
    }

    #[test]
    fn the_tracker_forgets_the_oldest_session_at_its_cap() {
        let mut tracker = SessionTracker::new(2);
        tracker.observe("a", turn(1, 0, 40_000));
        tracker.observe("b", turn(2, 0, 40_000));
        tracker.observe("a", turn(3, 40_000, 0));
        tracker.observe("c", turn(4, 0, 40_000));
        assert_eq!(tracker.len(), 2);
        assert_eq!(
            tracker.observe("b", turn(5, 0, 0)),
            None,
            "b was forgotten, so there is nothing to compare with"
        );
        assert_eq!(SessionTracker::new(0).cap, 1);
    }

    fn lane(at: i64, model: &str, system: &str, cache_read: u64, cache_write: u64) -> MainTurn {
        MainTurn {
            model: model.to_owned(),
            system_hash: system.to_owned(),
            ..turn(at, cache_read, cache_write)
        }
    }

    #[test]
    fn requests_of_two_lanes_in_turn_are_not_misses() {
        // The conversation (large context) and a side query (smaller context, other model and
        // system prompt) alternate, each reading its own warm cache.
        let mut tracker = SessionTracker::new(10);
        assert_eq!(
            tracker.observe("s", lane(0, "big", "chat", 0, 500_000)),
            None
        );
        // The first side query is a miss that is real: that model had no cache yet.
        assert_eq!(
            tracker.observe("s", lane(1, "small", "side", 0, 100_000)),
            Some(MissCause::ModelChanged)
        );
        for step in 0..6 {
            let at = 10 + step * 10;
            assert_eq!(
                tracker.observe("s", lane(at, "big", "chat", 500_000, 1_000)),
                None,
                "the conversation after the side query"
            );
            assert_eq!(
                tracker.observe("s", lane(at + 1, "small", "side", 100_000, 500)),
                None,
                "the side query after the conversation"
            );
        }
    }

    #[test]
    fn a_miss_inside_a_lane_is_still_explained() {
        let mut tracker = SessionTracker::new(10);
        tracker.observe("s", lane(0, "big", "chat", 0, 500_000));
        tracker.observe("s", lane(10, "small", "side", 0, 100_000));
        // The side query came back after more than five minutes: its own cache expired.
        assert_eq!(
            tracker.observe("s", lane(400, "small", "side", 0, 100_000)),
            Some(MissCause::IdleTtl)
        );
        // The conversation shrank without a header: compacted.
        let mut shorter = lane(410, "big", "chat", 0, 90_000);
        shorter.message_count = 2;
        assert_eq!(tracker.observe("s", shorter), Some(MissCause::Compaction));
    }

    #[test]
    fn the_first_request_of_a_lane_looks_at_the_same_model_then_at_the_session() {
        let mut tracker = SessionTracker::new(10);
        tracker.observe("s", lane(0, "big", "chat", 40_000, 2_000));
        // Same model, other system prompt.
        assert_eq!(
            tracker.observe("s", lane(10, "big", "other", 0, 42_000)),
            Some(MissCause::SystemChanged)
        );
        // A model the session had not used: its cache is empty.
        assert_eq!(
            tracker.observe("s", lane(20, "new", "chat", 0, 42_000)),
            Some(MissCause::ModelChanged)
        );
    }

    #[test]
    fn a_shorter_conversation_on_another_model_is_not_a_compaction() {
        let mut other_model = turn(10, 0, 42_000);
        other_model.model = "sonnet".to_owned();
        other_model.message_count = 1;
        assert_eq!(
            explain_cache_miss(Some(&turn(0, 40_000, 2_000)), &other_model),
            Some(MissCause::ModelChanged)
        );
    }

    #[test]
    fn a_session_remembers_a_few_lanes_and_forgets_the_least_recently_used() {
        let mut tracker = SessionTracker::new(10);
        for index in 0..=LANES_PER_SESSION {
            tracker.observe(
                "s",
                lane(
                    i64::try_from(index).unwrap(),
                    &format!("m{index}"),
                    "x",
                    0,
                    40_000,
                ),
            );
        }
        assert_eq!(tracker.sessions["s"].len(), LANES_PER_SESSION);
        assert!(tracker.sessions["s"].iter().all(|lane| lane.model != "m0"));
    }
}
