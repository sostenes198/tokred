//! The dashboard's view of the store: `proxy_requests`, `captured_turns` and the routing
//! decisions, turned into the shapes of `tokred_core::dashboard`, plus the HTML report for
//! `/metrics`. The proxy calls it from blocking threads; every call opens the database anew,
//! so it never holds a connection the proxy's writer needs.

use std::path::PathBuf;

use tokred_core::Clock;
use tokred_core::Timestamp;
use tokred_core::dashboard::{
    DashboardSource, MAX_TURNS, RECENT_SESSIONS, ResponseView, RoutingView, SentMessage,
    SessionDetail, SessionList, SessionModelRow, SessionStatus, SessionView, StoredTurn, TurnView,
    UsageView, origin_label, session_status,
};
use tokred_core::pricing::{PriceTable, Usage, cost};
use tokred_store::Store;

use super::SystemClock;
use super::metrics::html_report;
use crate::env::Env;

/// Sessions read from the store per call (the newest ones).
const SESSIONS_SCANNED: usize = 100;
/// The period of the report served at `/metrics`.
const REPORT_PERIOD: &str = "30d";

/// Reads the dashboard's data from the tokred database.
pub struct StoreSource {
    env: Env,
    claude_dir: Option<PathBuf>,
    prices: PriceTable,
}

impl StoreSource {
    /// A source over the database of `env`; `claude_dir` is where the report reads transcripts.
    pub fn new(env: Env, claude_dir: Option<PathBuf>, prices: PriceTable) -> Self {
        Self {
            env,
            claude_dir,
            prices,
        }
    }

    fn open(&self) -> Result<Store, String> {
        Store::open(&self.env.data.db_path()).map_err(|error| error.to_string())
    }
}

/// Estimated cost of `usage` for `model` at `prices`.
fn cost_of(prices: &PriceTable, model: &str, usage: &Usage) -> Option<f64> {
    prices
        .resolve(model)
        .map(|resolved| cost(usage, &resolved.price))
}

/// One session per run of rows with the same id (the rows come newest session first).
fn fold_sessions(
    rows: &[SessionModelRow],
    prices: &PriceTable,
    now: Timestamp,
) -> Vec<SessionView> {
    let mut sessions: Vec<SessionView> = Vec::new();
    for row in rows {
        if sessions.last().is_none_or(|last| last.id != row.session_id) {
            sessions.push(SessionView {
                id: row.session_id.clone(),
                started_at: row.first_ts.0,
                last_activity_at: row.last_ts.0,
                status: session_status(now, row.last_ts, false),
                requests: 0,
                models: Vec::new(),
                cost_usd: 0.0,
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                subagents_running: 0,
                content_recorded: row.content_turns > 0,
            });
        }
        let Some(session) = sessions.last_mut() else {
            continue;
        };
        session.requests += row.requests;
        session.input += row.usage.input;
        session.output += row.usage.output;
        session.cache_read += row.usage.cache_read;
        session.cache_write += row.usage.cache_write;
        session.cost_usd += cost_of(prices, &row.model, &row.usage).unwrap_or(0.0);
        if !row.model.is_empty() && !session.models.contains(&row.model) {
            session.models.push(row.model.clone());
        }
    }
    sessions
}

/// A stored turn in the API's shape. Contents that cannot be read back read as absent.
fn turn_view(stored: StoredTurn, prices: &PriceTable) -> TurnView {
    let record = stored.record;
    let (stop_reason, tools, sent, response) = match stored.captured {
        Some(captured) => (
            captured.stop_reason,
            serde_json::from_str::<Vec<String>>(&captured.tools_json).unwrap_or_default(),
            captured
                .sent_json
                .and_then(|json| serde_json::from_str::<Vec<SentMessage>>(&json).ok()),
            captured
                .response_json
                .and_then(|json| serde_json::from_str::<ResponseView>(&json).ok()),
        ),
        None => (None, Vec::new(), None, None),
    };
    let model_effective = if record.model_effective.is_empty() {
        record.model_requested.clone()
    } else {
        record.model_effective
    };
    let succeeded = (200..300).contains(&record.status);
    TurnView {
        id: record.id,
        session_id: record.session_id,
        ts: record.ts.0,
        origin: origin_label(&record.origin).to_owned(),
        agent_id: record.agent_id,
        agent_type: record.agent_type,
        model_requested: record.model_requested,
        status: Some(record.status),
        stop_reason,
        usage: UsageView::from(&record.usage),
        cost_usd: succeeded
            .then(|| cost_of(prices, &model_effective, &record.usage))
            .flatten(),
        model_effective,
        duration_ms: Some(record.total_ms),
        ttfb_ms: Some(record.upstream_ttfb_ms),
        tools,
        routing: stored
            .routing
            .map(|(outcome, reason)| RoutingView { outcome, reason }),
        in_flight: false,
        sent,
        response,
    }
}

impl DashboardSource for StoreSource {
    fn sessions(&self) -> Result<SessionList, String> {
        let rows = self
            .open()?
            .session_model_rows(None, SESSIONS_SCANNED)
            .map_err(|error| error.to_string())?;
        let (active, mut recent): (Vec<_>, Vec<_>) =
            fold_sessions(&rows, &self.prices, SystemClock.now())
                .into_iter()
                .partition(|session| session.status == SessionStatus::Active);
        recent.truncate(RECENT_SESSIONS);
        Ok(SessionList { active, recent })
    }

    fn session(&self, id: &str) -> Result<Option<SessionDetail>, String> {
        let store = self.open()?;
        let rows = store
            .session_model_rows(Some(id), 1)
            .map_err(|error| error.to_string())?;
        let Some(session) = fold_sessions(&rows, &self.prices, SystemClock.now()).pop() else {
            return Ok(None);
        };
        let (turns, truncated) = store
            .session_turns(id, MAX_TURNS)
            .map_err(|error| error.to_string())?;
        Ok(Some(SessionDetail {
            session,
            turns: turns
                .into_iter()
                .map(|turn| turn_view(turn, &self.prices))
                .collect(),
            truncated,
        }))
    }

    fn metrics_html(&self) -> Result<String, String> {
        let dir = self.claude_dir.clone().ok_or_else(|| {
            "the Claude Code folder is unknown; run `tokred configure` or set CLAUDE_CONFIG_DIR"
                .to_owned()
        })?;
        html_report(&self.env, dir, REPORT_PERIOD).map_err(|error| format!("{error:#}"))
    }

    fn cost_usd(&self, model: &str, usage: &Usage) -> Option<f64> {
        cost_of(&self.prices, model, usage)
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::dashboard::CapturedTurn;
    use tokred_core::proxy::{HookNotes, ProxyRequestRecord};
    use tokred_core::routing::{RoutingDecision, RoutingRepo};
    use tokred_pricing::load_table;

    use super::super::testing::Sandbox;
    use super::*;

    const MODEL: &str = "claude-opus-4-1-20250805";

    fn request(id: &str, session: &str, ts: i64, status: u16) -> ProxyRequestRecord {
        ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            session_id: session.to_owned(),
            origin: "compaction".to_owned(),
            model_requested: MODEL.to_owned(),
            model_effective: MODEL.to_owned(),
            usage: Usage {
                input: 1_000_000,
                ..Usage::default()
            },
            total_ms: 1500,
            upstream_ttfb_ms: 200,
            status,
            ..ProxyRequestRecord::default()
        }
    }

    fn source(sb: &Sandbox) -> StoreSource {
        let store = sb.store();
        let prices = load_table(&store, &SystemClock);
        StoreSource::new(sb.env.clone(), Some(sb.claude.clone()), prices)
    }

    fn seed(sb: &Sandbox) -> i64 {
        let now = SystemClock.now().0;
        let store = sb.store();
        store
            .insert_proxy_request(&request("a1", "live", now - 30, 200))
            .unwrap();
        let mut routed = request("a2", "live", now - 10, 200);
        routed.notes = HookNotes {
            decision_id: Some("d1".to_owned()),
            ..HookNotes::default()
        };
        store.insert_proxy_request(&routed).unwrap();
        store
            .insert_proxy_request(&request("a3", "live", now - 5, 429))
            .unwrap();
        store
            .insert_proxy_request(&request("o1", "old", now - 7_200, 200))
            .unwrap();
        store
            .insert_decision(&RoutingDecision {
                id: "d1".to_owned(),
                outcome: "applied".to_owned(),
                reason: "read-only tools".to_owned(),
                ..RoutingDecision::default()
            })
            .unwrap();
        store
            .insert_captured_turn(&CapturedTurn {
                request_id: "a2".to_owned(),
                session_id: "live".to_owned(),
                ts: Timestamp(now - 10),
                stop_reason: Some("end_turn".to_owned()),
                tools_json: r#"["Read","Grep"]"#.to_owned(),
                sent_json: Some(r#"[{"role":"user","text":"hi"}]"#.to_owned()),
                response_json: Some(
                    r#"{"text":"hello","thinking":null,"tool_uses":[]}"#.to_owned(),
                ),
                ..CapturedTurn::default()
            })
            .unwrap();
        store
            .insert_captured_turn(&CapturedTurn {
                request_id: "a1".to_owned(),
                session_id: "live".to_owned(),
                ts: Timestamp(now - 30),
                tools_json: "not json".to_owned(),
                sent_json: Some("not json either".to_owned()),
                ..CapturedTurn::default()
            })
            .unwrap();
        now
    }

    #[test]
    fn sessions_are_summed_priced_and_split_into_active_and_recent() {
        let sb = Sandbox::new();
        sb.configure();
        seed(&sb);
        let list = source(&sb).sessions().unwrap();
        assert_eq!(list.active.len(), 1);
        let live = &list.active[0];
        assert_eq!(live.id, "live");
        assert_eq!(live.status, SessionStatus::Active);
        assert_eq!(live.requests, 3);
        assert_eq!(live.input, 3_000_000);
        assert_eq!(live.models, [MODEL]);
        assert!(live.cost_usd > 0.0, "{}", live.cost_usd);
        assert!(live.content_recorded);
        assert_eq!(live.subagents_running, 0);

        assert_eq!(list.recent.len(), 1);
        assert_eq!(list.recent[0].id, "old");
        assert_eq!(list.recent[0].status, SessionStatus::Ended);
        assert!(!list.recent[0].content_recorded);
    }

    #[test]
    fn a_session_comes_with_its_turns_in_the_api_shape() {
        let sb = Sandbox::new();
        sb.configure();
        let now = seed(&sb);
        let detail = source(&sb).session("live").unwrap().unwrap();
        assert!(!detail.truncated);
        assert_eq!(detail.session.requests, 3);
        let ids: Vec<&str> = detail.turns.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a1", "a2", "a3"]);

        let bare = &detail.turns[0];
        assert_eq!(
            bare.origin, "auxiliary",
            "a compaction request is shown as auxiliary"
        );
        assert!(
            bare.tools.is_empty() && bare.sent.is_none(),
            "unreadable contents read as absent"
        );
        let routed = &detail.turns[1];
        assert_eq!(routed.ts, now - 10);
        assert_eq!(routed.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(routed.tools, ["Read", "Grep"]);
        assert_eq!(routed.sent.as_ref().unwrap()[0].text, "hi");
        assert_eq!(routed.response.as_ref().unwrap().text, "hello");
        assert_eq!(routed.routing.as_ref().unwrap().outcome, "applied");
        assert!(routed.cost_usd.unwrap() > 0.0);
        assert_eq!(
            (routed.duration_ms, routed.ttfb_ms),
            (Some(1500), Some(200))
        );
        assert!(!routed.in_flight);
        let failed = &detail.turns[2];
        assert_eq!((failed.status, failed.cost_usd), (Some(429), None));
        assert!(failed.stop_reason.is_none() && failed.response.is_none());

        assert!(source(&sb).session("nobody").unwrap().is_none());
    }

    #[test]
    fn a_turn_without_an_answering_model_shows_the_requested_one() {
        let mut record = request("r", "s", 1, 200);
        record.model_effective = String::new();
        let view = turn_view(
            StoredTurn {
                record,
                ..StoredTurn::default()
            },
            &PriceTable::default(),
        );
        assert_eq!(view.model_effective, MODEL);
        assert_eq!(view.cost_usd, None, "no price table, no cost");
    }

    #[test]
    fn prices_come_from_the_table() {
        let sb = Sandbox::new();
        sb.configure();
        let source = source(&sb);
        let usage = Usage {
            input: 1_000_000,
            ..Usage::default()
        };
        assert!(source.cost_usd(MODEL, &usage).unwrap() > 0.0);
        assert_eq!(source.cost_usd("<synthetic>", &usage), None);
    }

    #[test]
    fn the_report_is_the_one_tokred_metrics_writes() {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        let html = source(&sb).metrics_html().unwrap();
        assert!(
            html.contains("<html") && html.contains("claude-opus-4-1-20250805"),
            "{html:.200}"
        );
        assert!(!html.contains("http://") && !html.contains("https://"));

        let no_folder = StoreSource::new(sb.env.clone(), None, PriceTable::default());
        assert!(
            no_folder
                .metrics_html()
                .unwrap_err()
                .contains("Claude Code folder")
        );
        // No database yet (`configure` never ran): the reason is passed on.
        let fresh = Sandbox::new();
        let no_database = StoreSource::new(
            fresh.env.clone(),
            Some(fresh.claude.clone()),
            PriceTable::default(),
        );
        assert!(
            no_database
                .metrics_html()
                .unwrap_err()
                .contains("tokred configure")
        );
    }

    #[test]
    fn a_database_that_cannot_be_opened_is_an_error() {
        let sb = Sandbox::new();
        let mut env = sb.env.clone();
        env.data = tokred_core::DataLayout::new(sb.path("no/such/dir"));
        let source = StoreSource::new(env, None, PriceTable::default());
        assert!(source.sessions().is_err());
        assert!(source.session("x").is_err());
    }

    #[test]
    fn a_missing_table_is_reported() {
        let sb = Sandbox::new();
        sb.configure();
        let source = source(&sb);
        let conn = rusqlite::Connection::open(sb.env.data.db_path()).unwrap();
        conn.execute_batch("DROP TABLE proxy_requests;").unwrap();
        assert!(source.sessions().is_err());
        assert!(source.session("x").is_err());
    }

    #[test]
    fn a_session_with_turns_beyond_the_limit_is_flagged() {
        let sb = Sandbox::new();
        sb.configure();
        let store = sb.store();
        for n in 0..=MAX_TURNS {
            store
                .insert_proxy_request(&request(
                    &format!("t{n:04}"),
                    "big",
                    1_000 + i64::try_from(n).unwrap(),
                    200,
                ))
                .unwrap();
        }
        let detail = source(&sb).session("big").unwrap().unwrap();
        assert!(detail.truncated);
        assert_eq!(detail.turns.len(), MAX_TURNS);
        assert_eq!(
            detail.session.requests,
            u64::try_from(MAX_TURNS + 1).unwrap()
        );
    }
}
