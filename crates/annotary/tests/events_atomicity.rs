//! Failpoint between the mutation and the `events` insert ⇒ neither persists.
//! Lives in its own test binary because `fail` failpoints are process-global.

use annotary::testing::TestServer;

// spec: S01-05, S08-02
#[test]
fn mutation_and_event_are_one_transaction() {
    let ts = TestServer::start();
    let seq_before = ts.store.last_seq().unwrap();
    let members_before = ts.store.list_members().unwrap().len();

    fail::cfg("store.before_event_insert", "return").unwrap();
    let (status, body) = ts.post("/api/v1/members", Some(&ts.owner_token), &serde_json::json!({ "handle": "ghost" }));
    fail::remove("store.before_event_insert");

    assert_eq!(status, 500, "the request fails: {body}");
    assert_eq!(body["error"], "internal");
    assert_eq!(ts.store.last_seq().unwrap(), seq_before, "no event persisted");
    assert_eq!(ts.store.list_members().unwrap().len(), members_before, "no member persisted");
    assert!(ts.store.member_by_handle("ghost").unwrap().is_none());

    // With the failpoint gone the same mutation succeeds and appends its events.
    let (status, _) = ts.post("/api/v1/members", Some(&ts.owner_token), &serde_json::json!({ "handle": "ghost" }));
    assert_eq!(status, 201);
    assert_eq!(ts.store.last_seq().unwrap(), seq_before + 2);
    assert!(ts.store.member_by_handle("ghost").unwrap().is_some());
}
