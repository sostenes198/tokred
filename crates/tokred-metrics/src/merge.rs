//! One usage source out of two (spec 6.4 and 7.2): the rows the proxy measured and the rows read
//! from the transcripts.
//!
//! With the proxy running, every request is in both places, so adding the two would count it
//! twice. [`merge_usage`] pairs them instead:
//!
//! * a proxy request is paired with the transcript row that has the same `message_key` (the
//!   Anthropic message id plus the request id);
//! * a proxy request **without** a key (the response carried no message id, for example an
//!   interrupted stream) is paired with the transcript row of the same session whose tokens are
//!   equal and whose timestamp lies within [`MATCH_WINDOW_SECS`] of the request (the transcript
//!   line is written when the response ends, so the window runs from the request's arrival to the
//!   end of its response, plus the margin);
//! * a pair becomes one row that takes the proxy's numbers (it knows the model that really
//!   answered and the 1-hour cache split) and keeps the transcript's project;
//! * what has no partner stays: transcript-only rows (requests made while the proxy was off) and
//!   proxy-only rows.
//!
//! Pairing is one-to-one and greedy by distance in time, so two identical requests in the same
//! few seconds stay two rows. Requests that billed nothing (a non-2xx answer, or no tokens) are
//! not messages and are skipped. Without any proxy row the transcript rows come back unchanged.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use tokred_core::pricing::Usage;
use tokred_core::proxy::ProxyRequestRecord;
use tokred_core::usage::UsageRecord;

/// How far apart, in seconds, a keyless proxy request and its transcript row may be.
pub const MATCH_WINDOW_SECS: i64 = 5;

/// Project shown for a request only the proxy saw, in a session no transcript row mentions.
pub const PROXY_ONLY_PROJECT: &str = "(proxy)";

/// Model shown when neither the proxy nor a transcript row names one.
const UNKNOWN_MODEL: &str = "(unknown)";

/// How the rows of the two sources met.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MergeSummary {
    /// Proxy requests paired with a transcript row.
    pub matched: u64,
    /// Proxy requests no transcript row matched.
    pub proxy_only: u64,
    /// Transcript rows no proxy request matched.
    pub transcript_only: u64,
}

/// The rows of both sources, each request once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The usage rows, oldest first.
    pub records: Vec<UsageRecord>,
    /// How many came from where.
    pub summary: MergeSummary,
}

/// A request that cost tokens: answered with a 2xx status and with usage.
fn is_billable(request: &ProxyRequestRecord) -> bool {
    (200..300).contains(&request.status) && request.usage.total() > 0
}

/// The billable requests, each id and each message key once, in a stable order.
fn billable(proxy: &[ProxyRequestRecord]) -> Vec<&ProxyRequestRecord> {
    let mut ids: HashSet<&str> = HashSet::new();
    let mut keys: HashSet<&str> = HashSet::new();
    let mut requests: Vec<&ProxyRequestRecord> = proxy
        .iter()
        .filter(|request| is_billable(request))
        .filter(|request| ids.insert(&request.id))
        .filter(|request| {
            request
                .message_key
                .as_deref()
                .is_none_or(|key| keys.insert(key))
        })
        .collect();
    requests.sort_by(|a, b| (a.ts, &a.id).cmp(&(b.ts, &b.id)));
    requests
}

/// The four counters transcripts and the proxy both report (the 1-hour split can be missing on
/// one side).
fn same_tokens(a: &Usage, b: &Usage) -> bool {
    (a.input, a.output, a.cache_write, a.cache_read)
        == (b.input, b.output, b.cache_write, b.cache_read)
}

/// Seconds between a keyless request and a transcript row, when the row can be its message: the
/// row is written when the response ends, so it may come up to the response's length after the
/// request, and a few seconds of clock skew either side.
fn distance(request: &ProxyRequestRecord, row: &UsageRecord) -> Option<i64> {
    let response_secs = i64::try_from(request.total_ms.div_ceil(1_000)).unwrap_or(i64::MAX);
    let earliest = request.ts.0.saturating_sub(MATCH_WINDOW_SECS);
    let latest = request
        .ts
        .0
        .saturating_add(response_secs)
        .saturating_add(MATCH_WINDOW_SECS);
    (earliest..=latest)
        .contains(&row.ts.0)
        .then(|| (row.ts.0 - request.ts.0).abs())
}

/// For each billable request, the transcript row it is paired with.
fn pair(requests: &[&ProxyRequestRecord], rows: &[UsageRecord]) -> Vec<Option<usize>> {
    let mut by_key: HashMap<&str, usize> = HashMap::new();
    let mut by_session: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        by_key.entry(&row.message_key).or_insert(index);
        by_session.entry(&row.session_id).or_default().push(index);
    }

    let mut taken = vec![false; rows.len()];
    let mut paired: Vec<Option<usize>> = requests
        .iter()
        .map(|request| {
            let index = *by_key.get(request.message_key.as_deref()?)?;
            // Keys are unique among the requests, so the row is free; the guard only protects
            // against a transcript source that repeats a key.
            (!std::mem::replace(&mut taken[index], true)).then_some(index)
        })
        .collect();

    let mut candidates: Vec<(i64, usize, usize)> = Vec::new();
    for (request_index, request) in requests.iter().enumerate() {
        if request.message_key.is_some() {
            continue;
        }
        for &row_index in by_session
            .get(request.session_id.as_str())
            .into_iter()
            .flatten()
        {
            let row = &rows[row_index];
            if !taken[row_index] && same_tokens(&request.usage, &row.usage) {
                candidates
                    .extend(distance(request, row).map(|gap| (gap, request_index, row_index)));
            }
        }
    }
    candidates.sort_unstable();
    for (_, request_index, row_index) in candidates {
        if paired[request_index].is_none() && !taken[row_index] {
            taken[row_index] = true;
            paired[request_index] = Some(row_index);
        }
    }
    paired
}

/// The first non-empty name, else the placeholder.
fn model_of(request: &ProxyRequestRecord, row: Option<&UsageRecord>) -> String {
    [
        Some(request.model_effective.as_str()),
        Some(request.model_requested.as_str()),
        row.map(|row| row.model.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|model| !model.is_empty())
    .unwrap_or(UNKNOWN_MODEL)
    .to_owned()
}

/// The row of a request: the proxy's numbers, the transcript's identity when there is one.
fn row_of(
    request: &ProxyRequestRecord,
    row: Option<&UsageRecord>,
    projects: &HashMap<&str, &str>,
) -> UsageRecord {
    let subagent = request.origin == "subagent";
    UsageRecord {
        message_key: row
            .map(|row| row.message_key.clone())
            .or_else(|| request.message_key.clone())
            .unwrap_or_else(|| format!("proxy:{}", request.id)),
        session_id: row.map_or_else(|| request.session_id.clone(), |row| row.session_id.clone()),
        project: row.map_or_else(
            || {
                projects
                    .get(request.session_id.as_str())
                    .copied()
                    .unwrap_or(PROXY_ONLY_PROJECT)
                    .to_owned()
            },
            |row| row.project.clone(),
        ),
        ts: request.ts,
        model: model_of(request, row),
        usage: request.usage,
        is_subagent: subagent || row.is_some_and(|row| row.is_subagent),
        origin: Some(request.origin.clone()),
        agent_type: request.agent_type.clone(),
    }
}

/// Merges the transcript rows (unique by `message_key`, as the `usage` table keeps them) with the
/// proxy requests. See the module docs for the rules.
#[must_use]
pub fn merge_usage(transcripts: &[UsageRecord], proxy: &[ProxyRequestRecord]) -> Merged {
    let requests = billable(proxy);
    if requests.is_empty() {
        return Merged {
            records: transcripts.to_vec(),
            summary: MergeSummary {
                transcript_only: u64::try_from(transcripts.len()).unwrap_or(u64::MAX),
                ..MergeSummary::default()
            },
        };
    }

    let paired = pair(&requests, transcripts);
    let mut projects: HashMap<&str, &str> = HashMap::new();
    for row in transcripts {
        projects.entry(&row.session_id).or_insert(&row.project);
    }

    let mut partner: HashMap<usize, &ProxyRequestRecord> = HashMap::new();
    let mut summary = MergeSummary::default();
    let mut records: Vec<UsageRecord> = Vec::with_capacity(transcripts.len() + requests.len());
    for (request, index) in requests.iter().zip(&paired) {
        if let Some(index) = index {
            partner.insert(*index, request);
            summary.matched += 1;
        } else {
            records.push(row_of(request, None, &projects));
            summary.proxy_only += 1;
        }
    }
    for (index, row) in transcripts.iter().enumerate() {
        if let Some(request) = partner.get(&index) {
            records.push(row_of(request, Some(row), &projects));
        } else {
            records.push(row.clone());
            summary.transcript_only += 1;
        }
    }
    records.sort_by(|a, b| (a.ts, &a.message_key).cmp(&(b.ts, &b.message_key)));
    Merged { records, summary }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use tokred_core::Timestamp;

    use super::*;

    fn tokens(input: u64, output: u64) -> Usage {
        Usage {
            input,
            output,
            cache_write: 10,
            cache_write_1h: 0,
            cache_read: 100,
        }
    }

    fn row(key: &str, session: &str, ts: i64, usage: Usage) -> UsageRecord {
        UsageRecord {
            message_key: key.to_owned(),
            session_id: session.to_owned(),
            project: "-proj".to_owned(),
            ts: Timestamp(ts),
            model: "claude-sonnet-4-20250514".to_owned(),
            usage,
            is_subagent: false,
            origin: None,
            agent_type: None,
        }
    }

    fn request(
        id: &str,
        key: Option<&str>,
        session: &str,
        ts: i64,
        usage: Usage,
    ) -> ProxyRequestRecord {
        ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            session_id: session.to_owned(),
            origin: "main".to_owned(),
            model_requested: "claude-opus-4-1-20250805".to_owned(),
            model_effective: "claude-haiku-4-5".to_owned(),
            usage,
            message_key: key.map(str::to_owned),
            status: 200,
            ..ProxyRequestRecord::default()
        }
    }

    fn keys(merged: &Merged) -> Vec<&str> {
        merged
            .records
            .iter()
            .map(|r| r.message_key.as_str())
            .collect()
    }

    #[test]
    fn without_proxy_rows_the_transcripts_come_back_unchanged() {
        let rows = vec![
            row("b", "s", 20, tokens(1, 1)),
            row("a", "s", 10, tokens(2, 2)),
        ];
        let merged = merge_usage(&rows, &[]);
        assert_eq!(merged.records, rows, "same rows, same order");
        assert_eq!(
            merged.summary,
            MergeSummary {
                matched: 0,
                proxy_only: 0,
                transcript_only: 2
            }
        );

        let nothing = merge_usage(&[], &[]);
        assert!(nothing.records.is_empty());
        assert_eq!(nothing.summary, MergeSummary::default());
    }

    #[test]
    fn requests_that_billed_nothing_are_not_messages() {
        let rows = vec![row("m1:r1", "s", 100, tokens(5, 5))];
        let mut failed = request("p1", Some("m9:r9"), "s", 100, tokens(5, 5));
        failed.status = 529;
        let empty = request("p2", Some("m8:r8"), "s", 100, Usage::default());
        let merged = merge_usage(&rows, &[failed, empty]);
        assert_eq!(merged.records, rows);
        assert_eq!(merged.summary.transcript_only, 1);
        assert_eq!(merged.summary.proxy_only, 0);
    }

    #[test]
    fn a_shared_key_makes_one_row_that_takes_the_proxy_numbers() {
        let mut transcript = row("m1:r1", "s", 130, tokens(1, 1));
        transcript.model = "claude-opus-4-1-20250805".to_owned();
        transcript.is_subagent = true;
        let mut proxied = request("p1", Some("m1:r1"), "s", 100, tokens(7, 3));
        proxied.agent_type = Some("Explore".to_owned());
        proxied.usage.cache_write_1h = 4;
        let merged = merge_usage(&[transcript], &[proxied]);

        assert_eq!(merged.records.len(), 1);
        let merged_row = &merged.records[0];
        assert_eq!(merged_row.message_key, "m1:r1");
        assert_eq!(
            merged_row.model, "claude-haiku-4-5",
            "the model that answered"
        );
        assert_eq!(merged_row.usage.input, 7);
        assert_eq!(
            merged_row.usage.cache_write_1h, 4,
            "the 1-hour split is the proxy's"
        );
        assert_eq!(merged_row.ts, Timestamp(100), "when the request arrived");
        assert_eq!(
            merged_row.project, "-proj",
            "the transcript knows the project"
        );
        assert!(merged_row.is_subagent, "either source can say subagent");
        assert_eq!(merged_row.origin.as_deref(), Some("main"));
        assert_eq!(merged_row.agent_type.as_deref(), Some("Explore"));
        assert_eq!(
            merged.summary,
            MergeSummary {
                matched: 1,
                proxy_only: 0,
                transcript_only: 0
            }
        );
    }

    #[test]
    fn rows_without_a_partner_stay_and_proxy_only_rows_get_an_identity() {
        let rows = vec![
            row("m1:r1", "s1", 100, tokens(1, 1)),
            row("m2:r2", "s2", 300, tokens(2, 2)),
        ];
        let proxied = vec![
            request("p1", Some("m1:r1"), "s1", 100, tokens(1, 1)),
            request("p2", Some("m3:r3"), "s1", 200, tokens(3, 3)),
            request("p3", None, "s9", 250, tokens(4, 4)),
        ];
        let merged = merge_usage(&rows, &proxied);
        assert_eq!(keys(&merged), ["m1:r1", "m3:r3", "proxy:p3", "m2:r2"]);
        assert_eq!(
            merged.summary,
            MergeSummary {
                matched: 1,
                proxy_only: 2,
                transcript_only: 1
            }
        );
        assert_eq!(
            merged.records[1].project, "-proj",
            "the session's project is known"
        );
        assert_eq!(merged.records[2].project, PROXY_ONLY_PROJECT);
        assert_eq!(
            merged.records[3].origin, None,
            "transcript-only rows stay as they were"
        );
    }

    #[test]
    fn a_model_is_always_named() {
        let mut nameless = request("p1", None, "s", 10, tokens(1, 1));
        nameless.model_effective = String::new();
        let merged = merge_usage(&[], &[nameless.clone()]);
        assert_eq!(
            merged.records[0].model, "claude-opus-4-1-20250805",
            "falls back to the requested"
        );

        nameless.model_requested = String::new();
        assert_eq!(
            merge_usage(&[], &[nameless.clone()]).records[0].model,
            UNKNOWN_MODEL
        );

        let transcript = row("m:r", "s", 10, tokens(1, 1));
        nameless.message_key = Some("m:r".to_owned());
        let merged = merge_usage(&[transcript], &[nameless]);
        assert_eq!(
            merged.records[0].model, "claude-sonnet-4-20250514",
            "falls back to the transcript"
        );
    }

    #[test]
    fn a_keyless_request_matches_on_session_time_and_tokens() {
        let rows = vec![
            row("a:1", "s", 103, tokens(5, 5)),
            row("b:2", "other", 103, tokens(5, 5)),
            row("c:3", "s", 103, tokens(6, 5)),
            row("d:4", "s", 140, tokens(5, 5)),
        ];
        let mut keyless = request("p1", None, "s", 100, tokens(5, 5));
        keyless.total_ms = 2_000;
        let merged = merge_usage(&rows, &[keyless]);
        assert_eq!(merged.summary.matched, 1);
        assert_eq!(merged.summary.proxy_only, 0);
        assert_eq!(
            merged.records.len(),
            4,
            "only the session's equal row was consumed"
        );
        assert!(keys(&merged).contains(&"a:1"));
        assert_eq!(
            merged
                .records
                .iter()
                .find(|r| r.message_key == "a:1")
                .and_then(|r| r.origin.clone()),
            Some("main".to_owned())
        );
    }

    #[test]
    fn the_time_window_covers_the_length_of_the_response() {
        let matched = |row_ts: i64, total_ms: u64| {
            let transcript = row("a:1", "s", row_ts, tokens(5, 5));
            let mut keyless = request("p1", None, "s", 100, tokens(5, 5));
            keyless.total_ms = total_ms;
            merge_usage(&[transcript], &[keyless]).summary.matched
        };

        assert_eq!(matched(112, 7_000), 1, "7 s of response and the 5 s margin");
        assert_eq!(matched(113, 7_000), 0);
        assert_eq!(
            matched(113, 7_001),
            1,
            "a started second counts as a whole one"
        );
        assert_eq!(matched(100 - MATCH_WINDOW_SECS, 0), 1);
        assert_eq!(matched(100 - MATCH_WINDOW_SECS - 1, 0), 0);
    }

    #[test]
    fn a_present_but_different_key_is_a_different_message() {
        let rows = vec![row("a:1", "s", 100, tokens(5, 5))];
        let proxied = vec![request("p1", Some("b:2"), "s", 100, tokens(5, 5))];
        let merged = merge_usage(&rows, &proxied);
        assert_eq!(
            merged.records.len(),
            2,
            "never merged by time when both have keys"
        );
        assert_eq!(merged.summary.matched, 0);
    }

    #[test]
    fn identical_requests_in_the_same_seconds_stay_separate() {
        let rows = vec![
            row("a:1", "s", 101, tokens(5, 5)),
            row("b:2", "s", 103, tokens(5, 5)),
        ];
        let proxied = vec![
            request("p1", None, "s", 100, tokens(5, 5)),
            request("p2", None, "s", 102, tokens(5, 5)),
            request("p3", None, "s", 102, tokens(5, 5)),
        ];
        let merged = merge_usage(&rows, &proxied);
        assert_eq!(
            merged.summary,
            MergeSummary {
                matched: 2,
                proxy_only: 1,
                transcript_only: 0
            }
        );
        assert_eq!(merged.records.len(), 3, "three requests, three rows");
    }

    #[test]
    fn the_nearest_pair_wins_and_ties_go_to_the_earlier_request() {
        let rows = vec![
            row("a:1", "s", 104, tokens(5, 5)),
            row("b:2", "s", 100, tokens(5, 5)),
        ];
        let proxied = vec![
            request("p1", None, "s", 100, tokens(5, 5)),
            request("p2", None, "s", 104, tokens(5, 5)),
        ];
        let merged = merge_usage(&rows, &proxied);
        let ts_of = |key: &str| {
            merged
                .records
                .iter()
                .find(|r| r.message_key == key)
                .map(|r| r.ts.0)
        };
        assert_eq!(ts_of("b:2"), Some(100), "p1 took the row at its own second");
        assert_eq!(ts_of("a:1"), Some(104));
    }

    #[test]
    fn repeated_requests_and_keys_count_once() {
        let proxied = vec![
            request("p1", Some("m:r"), "s", 10, tokens(1, 1)),
            request("p1", Some("other:key"), "s", 10, tokens(1, 1)),
            request("p2", Some("m:r"), "s", 11, tokens(1, 1)),
            request("p3", None, "s", 12, tokens(2, 2)),
        ];
        let merged = merge_usage(&[], &proxied);
        assert_eq!(keys(&merged), ["m:r", "proxy:p3"]);
        assert_eq!(merged.summary.proxy_only, 2);
    }

    #[test]
    fn a_transcript_source_that_repeats_a_key_still_pairs_once() {
        let rows = vec![
            row("m:r", "s", 10, tokens(1, 1)),
            row("m:r", "s", 10, tokens(1, 1)),
        ];
        let proxied = vec![request("p1", Some("m:r"), "s", 10, tokens(1, 1))];
        let merged = merge_usage(&rows, &proxied);
        assert_eq!(merged.summary.matched, 1);
        assert_eq!(merged.records.len(), 2);
    }

    // ---- properties ----------------------------------------------------------------------

    /// Small pools so that keys, sessions and times collide all the time.
    fn arb_usage() -> impl Strategy<Value = Usage> {
        (1_u64..3, 1_u64..3).prop_map(|(input, output)| tokens(input, output))
    }

    fn arb_rows() -> impl Strategy<Value = Vec<UsageRecord>> {
        prop::collection::vec(("[st][0-9]", 0_i64..14, arb_usage()), 0..8).prop_map(|mut rows| {
            // The `usage` table hands rows over oldest first.
            rows.sort_by_key(|(_, ts, _)| *ts);
            rows.into_iter()
                .enumerate()
                .map(|(n, (session, ts, usage))| {
                    // Unique keys, like the `usage` table; some share the proxy key pool.
                    let key = if n % 2 == 0 {
                        format!("k{n}:r")
                    } else {
                        format!("m{n}:r")
                    };
                    row(&key, &session, ts, usage)
                })
                .collect()
        })
    }

    fn arb_requests() -> impl Strategy<Value = Vec<ProxyRequestRecord>> {
        prop::collection::vec(
            (
                prop::option::of(0_usize..8),
                "[st][0-9]",
                0_i64..14,
                arb_usage(),
                0_u64..4_000,
                prop::bool::weighted(0.85),
            ),
            0..8,
        )
        .prop_map(|requests| {
            requests
                .into_iter()
                .enumerate()
                .map(|(n, (key, session, ts, usage, total_ms, ok))| {
                    let key = key.map(|k| {
                        if k % 2 == 0 {
                            format!("k{k}:r")
                        } else {
                            format!("m{k}:r")
                        }
                    });
                    let mut request =
                        request(&format!("p{n}"), key.as_deref(), &session, ts, usage);
                    request.total_ms = total_ms;
                    request.status = if ok { 200 } else { 500 };
                    request
                })
                .collect()
        })
    }

    fn sorted(mut records: Vec<UsageRecord>) -> Vec<UsageRecord> {
        records.sort_by(|a, b| a.message_key.cmp(&b.message_key));
        records
    }

    proptest! {
        #[test]
        fn merging_never_loses_or_repeats_a_request(rows in arb_rows(), requests in arb_requests()) {
            let merged = merge_usage(&rows, &requests);
            let summary = merged.summary;
            let billable_requests = billable(&requests).len() as u64;

            // Every request is either paired or kept on its own, every row likewise.
            prop_assert_eq!(summary.matched + summary.proxy_only, billable_requests);
            prop_assert_eq!(summary.matched + summary.transcript_only, rows.len() as u64);
            prop_assert_eq!(
                merged.records.len() as u64,
                summary.matched + summary.proxy_only + summary.transcript_only
            );

            // Nothing is counted twice: every key appears once, and every transcript key survives.
            let mut seen = HashSet::new();
            for record in &merged.records {
                prop_assert!(seen.insert(record.message_key.clone()), "{} twice", record.message_key);
            }
            for transcript in &rows {
                prop_assert!(seen.contains(&transcript.message_key));
            }

            // Every billable request is there with its tokens, and every unpaired row too.
            let kept_alone = |key: &str| {
                merged.records.iter().any(|m| m.message_key == key && m.origin.is_none())
            };
            let total_in: u64 = merged.records.iter().map(|r| r.usage.input).sum();
            let expected_in: u64 = billable(&requests).iter().map(|r| r.usage.input).sum::<u64>()
                + rows.iter().filter(|r| kept_alone(&r.message_key)).map(|r| r.usage.input).sum::<u64>();
            prop_assert_eq!(total_in, expected_in);

            // Sorted by time.
            prop_assert!(merged.records.windows(2).all(|w| w[0].ts <= w[1].ts));
        }

        #[test]
        fn merging_is_idempotent(rows in arb_rows(), requests in arb_requests()) {
            let once = merge_usage(&rows, &requests);

            // Repeating the proxy rows changes nothing.
            let doubled: Vec<_> = requests.iter().chain(requests.iter()).cloned().collect();
            prop_assert_eq!(&merge_usage(&rows, &doubled), &once);

            // Merging the result with the same proxy rows again changes nothing either.
            let again = merge_usage(&once.records, &requests);
            prop_assert_eq!(sorted(again.records), sorted(once.records.clone()));
            prop_assert_eq!(again.summary.proxy_only, 0);
        }

        #[test]
        fn without_requests_the_rows_are_untouched(rows in arb_rows()) {
            prop_assert_eq!(merge_usage(&rows, &[]).records, rows);
        }
    }
}
