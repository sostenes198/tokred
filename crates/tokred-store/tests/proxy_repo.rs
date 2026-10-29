//! The read port over `proxy_requests` that the reports use.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::proxy::{ProxyRequestRecord, ProxyRequestRepo};
use tokred_store::Store;

#[test]
fn the_port_reads_what_the_store_holds() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    for (id, ts) in [("a", 100), ("b", 200), ("c", 300)] {
        let record = ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            ..ProxyRequestRecord::default()
        };
        assert!(store.insert_proxy_request(&record).unwrap());
    }

    let repo: &dyn ProxyRequestRepo = &store;
    let rows = repo
        .requests_between(Timestamp(150), Timestamp(300))
        .unwrap();
    let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["b"], "the range is half-open");
    assert_eq!(
        repo.requests_between(Timestamp(0), Timestamp(1_000))
            .unwrap(),
        store
            .proxy_requests_between(Timestamp(0), Timestamp(1_000))
            .unwrap()
    );
}
