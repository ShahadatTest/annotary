//! `events.seq` is Tenant-wide, strictly monotonic, and every mutation appends exactly its events.

use annotary::testing::TestServer;

// spec: S01-05, S08-01
#[test]
fn seq_is_strictly_monotonic_across_mutations_over_http() {
    let ts = TestServer::start();
    let before = ts.store.last_seq().unwrap();
    assert_eq!(before, 4, "setup: tenant, owner, vault, owner token");
    for h in ["a1", "b2", "c3"] {
        let (status, _) = ts.post("/api/v1/members", Some(&ts.owner_token), &serde_json::json!({ "handle": h }));
        assert_eq!(status, 201);
    }
    let events = ts.store.events_since(0).unwrap();
    let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=10).collect::<Vec<i64>>(), "no gaps, strictly increasing");
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        &kinds[4..],
        &["member.added", "token.minted", "member.added", "token.minted", "member.added", "token.minted"]
    );
    // A refused mutation (duplicate handle) appends nothing.
    let (status, _) = ts.post("/api/v1/members", Some(&ts.owner_token), &serde_json::json!({ "handle": "a1" }));
    assert_eq!(status, 409);
    assert_eq!(ts.store.last_seq().unwrap(), 10);
    // Reads append nothing (last_used_at bookkeeping is not an event).
    let (status, _) = ts.get("/api/v1/members", Some(&ts.owner_token));
    assert_eq!(status, 200);
    assert_eq!(ts.store.last_seq().unwrap(), 10);
    assert_eq!(ts.store.events_since(8).unwrap().len(), 2);
}
