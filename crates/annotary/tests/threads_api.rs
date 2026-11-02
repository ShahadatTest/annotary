//! `/api/v1/docs/{id}/threads…` and `/api/v1/mentions` over HTTP against the in-process server:
//! event stamping with the head Version, tombstone invariants, the body cap, mention resolution
//! (rename-safe), ordering/filters, rights, `open_threads`/`mentions_me` on docs/status/list, and
//! the Sidecar round-trip of the CLI-prototype scenario.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

const PLAN_V1: &str = include_str!("../../../fixtures/prototype-sidecar/versions/v1.md");

fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(md)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

fn create(ts: &TestServer, token: &str, name: &str, md: &str) -> String {
    let hash = upload(ts, token, md.as_bytes());
    let (status, doc) =
        ts.post("/api/v1/docs", Some(token), &json!({ "vault": "notes", "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, token: &str, id: &str, md: &str) -> u64 {
    let hash = upload(ts, token, md.as_bytes());
    let (status, v) =
        ts.post(&format!("/api/v1/docs/{id}/versions"), Some(token), &json!({ "name": "plan", "md_hash": hash }));
    assert_eq!(status, 201, "{v}");
    v["version"].as_u64().unwrap()
}

fn new_thread(ts: &TestServer, token: &str, id: &str, body: Value) -> (u16, Value) {
    ts.post(&format!("/api/v1/docs/{id}/threads"), Some(token), &body)
}

fn span(range: &str, body: &str) -> Value {
    json!({ "subject": "span", "range": range, "body": body })
}

fn document(body: &str) -> Value {
    json!({ "subject": "document", "body": body })
}

fn reply(ts: &TestServer, token: &str, id: &str, tid: &str, body: &str, resolve: bool) -> (u16, Value) {
    ts.post(
        &format!("/api/v1/docs/{id}/threads/{tid}/replies"),
        Some(token),
        &json!({ "body": body, "resolve": resolve }),
    )
}

fn send(method: &str, ts: &TestServer, token: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    let res = match method {
        "PUT" => ureq::put(format!("{}{}", ts.url, path))
            .header("Authorization", &auth)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(body.expect("PUT body")),
        "DELETE" => ureq::delete(format!("{}{}", ts.url, path))
            .header("Authorization", &auth)
            .config()
            .http_status_as_error(false)
            .build()
            .call(),
        _ => unreachable!(),
    };
    let mut r = res.unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn edit(ts: &TestServer, token: &str, id: &str, cid: &str, body: &str) -> (u16, Value) {
    send("PUT", ts, token, &format!("/api/v1/docs/{id}/threads/{cid}"), Some(&json!({ "body": body })))
}

fn delete(ts: &TestServer, token: &str, id: &str, cid: &str) -> (u16, Value) {
    send("DELETE", ts, token, &format!("/api/v1/docs/{id}/threads/{cid}"), None)
}

fn threads(ts: &TestServer, token: &str, id: &str, query: &str) -> Vec<Value> {
    let (status, body) = ts.get(&format!("/api/v1/docs/{id}/threads?{query}"), Some(token));
    assert_eq!(status, 200, "{body}");
    body["threads"].as_array().cloned().unwrap_or_default()
}

fn ids(list: &[Value]) -> Vec<String> {
    list.iter().map(|t| t["id"].as_str().unwrap().to_string()).collect()
}

fn sidecar_text(ts: &TestServer, token: &str, id: &str) -> String {
    let mut r = ureq::get(format!("{}/api/v1/docs/{id}/sidecar", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .unwrap();
    r.body_mut().read_to_string().unwrap()
}

// spec: S04-01, S04-02, S04-03, S04-04, S04-13
#[test]
fn thread_events_are_stamped_with_the_head_version_and_counts_are_live() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("scribe");
    let id = create(&ts, &owner, "plan", PLAN_V1);

    // A new span Thread on head: the quote is frozen from the head Markdown.
    let (status, t1) = new_thread(&ts, &owner, &id, span("16:0-16:33", "Agreed — @scribe check the order list"));
    assert_eq!(status, 201, "{t1}");
    assert_eq!(t1["id"], "t1");
    assert_eq!(t1["subject"], "span");
    assert_eq!(
        t1["anchor"]["original"],
        json!({ "version": 1, "range": "16:0-16:33", "exact": "We ship the CLI before the web UI" })
    );
    assert_eq!(t1["anchor"]["current"], json!({ "version": 1, "range": "16:0-16:33", "status": "anchored" }));
    assert_eq!(t1["anchor"]["history"], json!([{ "version": 1, "range": "16:0-16:33", "status": "anchored" }]));
    assert_eq!(t1["author"], "rowan");
    assert_eq!(t1["created"]["version"], 1);
    assert_eq!(t1["mentions"], json!(["scribe"]));
    assert_eq!(t1["resolved"], Value::Null);
    // Document-level Threads have no anchor. `version` guards the head.
    let (status, t2) = new_thread(&ts, &owner, &id, document("Whole-doc note"));
    assert_eq!(status, 201);
    assert_eq!((t2["id"].as_str(), t2["anchor"].is_null()), (Some("t2"), true));
    let (status, e) =
        new_thread(&ts, &owner, &id, json!({ "subject": "span", "range": "16:0-16:33", "body": "x", "version": 2 }));
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "not_head");
    assert_eq!(e["message"], "head is v1, not v2");
    let (status, e) = new_thread(&ts, &owner, &id, span("99:0-99:2", "x"));
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "range_not_on_head");
    let (status, e) = new_thread(&ts, &owner, &id, span("nope", "x"));
    assert_eq!(status, 400, "{e}");
    let (status, e) = new_thread(&ts, &owner, &id, span("16:0-16:33", "  "));
    assert_eq!(status, 400, "{e}");
    let (status, e) = new_thread(&ts, &owner, &id, json!({ "subject": "span", "body": "x" }));
    assert_eq!(status, 400, "{e}");

    // Counts on the Document / status / list are live and per caller.
    let (_, doc) = ts.get(&format!("/api/v1/docs/{id}"), Some(&agent));
    assert_eq!((doc["open_threads"].as_u64(), doc["mentions_me"].as_u64()), (Some(2), Some(1)));
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&owner));
    assert_eq!((st["open_threads"].as_u64(), st["mentions_me"].as_u64()), (Some(2), Some(0)));
    let (_, list) = ts.get("/api/v1/docs", Some(&agent));
    assert_eq!(list["docs"][0]["open_threads"], 2);
    assert_eq!(list["docs"][0]["mentions_me"], 1);
    let (_, list) = ts.get("/api/v1/docs?mentions_me=1", Some(&owner));
    assert_eq!(list["docs"].as_array().unwrap().len(), 0, "the owner is mentioned nowhere");
    let (_, list) = ts.get("/api/v1/docs?mentions_me=1", Some(&agent));
    assert_eq!(list["docs"].as_array().unwrap().len(), 1);

    // The head moves; a Reply and a resolve are stamped with the new head.
    let seq0 = ts.store.last_seq().unwrap();
    assert_eq!(push(&ts, &owner, &id, "# Plan v2\n"), 2);
    let (status, r) = reply(&ts, &agent, &id, "t1", "Item 3 says so. @rowan", false);
    assert_eq!(status, 201, "{r}");
    assert_eq!(r["comment"]["id"], "t1.1");
    assert_eq!(r["comment"]["created"]["version"], 2);
    assert_eq!(r["comment"]["mentions"], json!(["rowan"]));
    assert_eq!(r["resolved"], false);
    let (status, r) = reply(&ts, &agent, &id, "t1", "Done.", true);
    assert_eq!(status, 201);
    assert_eq!(r["comment"]["id"], "t1.2");
    assert_eq!(r["resolved"], true);
    assert_eq!(r["thread"]["resolved"], json!({ "by": "scribe", "at": r["thread"]["resolved"]["at"], "version": 2 }));
    // Resolve is idempotent; reopen is a bare event by anyone with `comment`.
    let (status, t) = ts.post(&format!("/api/v1/docs/{id}/threads/t1/resolve"), Some(&owner), &json!({}));
    assert_eq!((status, t["changed"].as_bool()), (200, Some(false)));
    let (status, t) = ts.post(&format!("/api/v1/docs/{id}/threads/t1/reopen"), Some(&owner), &json!({}));
    assert_eq!((status, t["changed"].as_bool(), t["resolved"].is_null()), (200, Some(true), true));
    let events = ts.store.events_since(seq0).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["version.pushed", "thread.replied", "thread.replied", "thread.resolved", "thread.reopened"]);
    assert_eq!(events[1].payload["comment"], "t1.1");
    assert_eq!(events[1].payload["version"], 2);
    assert_eq!(events[1].payload["mentions"], json!([ts.owner_id]));
    assert_eq!(events[1].by.as_deref(), Some(agent_id.as_str()));
    assert_eq!(events[3].payload["thread"], "t1");
    assert!(events.iter().all(|e| e.doc_id.as_deref() == Some(id.as_str())));
    // Unknown Threads name the Document.
    let (status, e) = reply(&ts, &owner, &id, "t99", "hi", false);
    assert_eq!(status, 404);
    assert_eq!(e["message"], format!("no Thread t99 in plan ({id})"));
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}/threads/t99"), Some(&owner));
    assert_eq!(status, 404);
    // Mutations on a deleted Document are refused; reads still work.
    send("DELETE", &ts, &owner, &format!("/api/v1/docs/{id}"), None);
    let (status, e) = reply(&ts, &owner, &id, "t1", "x", false);
    assert_eq!((status, e["error"].as_str()), (409, Some("deleted")));
    let (status, e) = new_thread(&ts, &owner, &id, document("x"));
    assert_eq!((status, e["error"].as_str()), (409, Some("deleted")));
    assert_eq!(threads(&ts, &owner, &id, "").len(), 2);
}

// spec: S04-07, S04-08, S04-17, S04-18
#[test]
fn edit_and_tombstone_over_http_with_rights() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("scribe");
    let (reader_id, reader) = ts.add_member("reader");
    ts.store
        .set_grants(&ts.owner_id, &reader_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Read] }])
        .unwrap();
    let id = create(&ts, &owner, "plan", PLAN_V1);
    let (_, t1) = new_thread(&ts, &agent, &id, document("root by agent"));
    assert_eq!(t1["id"], "t1");
    reply(&ts, &owner, &id, "t1", "reply by owner", false);

    // `comment` is needed to post/reply/resolve; `read` is enough to list.
    let (status, e) = new_thread(&ts, &reader, &id, document("nope"));
    assert_eq!(status, 403, "{e}");
    assert_eq!(e["message"], "missing right `comment` on Vault notes (you are @reader)");
    assert_eq!(reply(&ts, &reader, &id, "t1", "nope", false).0, 403);
    assert_eq!(ts.post(&format!("/api/v1/docs/{id}/threads/t1/resolve"), Some(&reader), &json!({})).0, 403);
    assert_eq!(edit(&ts, &reader, &id, "t1", "nope").0, 403);
    assert_eq!(delete(&ts, &reader, &id, "t1").0, 403);
    assert_eq!(threads(&ts, &reader, &id, "").len(), 1);
    assert_eq!(ts.get(&format!("/api/v1/docs/{id}/threads/t1"), Some(&reader)).0, 200);

    // Edit: the author or an owner; sets edited_at; a tombstoned Comment cannot be edited.
    let (status, e) = edit(&ts, &owner, &id, "t1.1", "still owner");
    assert_eq!(status, 200, "{e}");
    assert!(e["edited_at"].is_string());
    let (status, e) = edit(&ts, &agent, &id, "t1.1", "hijack");
    assert_eq!(status, 403, "{e}");
    assert_eq!(e["message"], "only the author (@rowan) or an owner may edit t1.1");
    assert_eq!(edit(&ts, &owner, &id, "t1", "owner edits anyone's").0, 200);
    let (status, e) = edit(&ts, &owner, &id, "t1.9", "x");
    assert_eq!(status, 404, "{e}");
    assert_eq!(e["message"], format!("no Comment t1.9 in plan ({id})"));
    assert_eq!(edit(&ts, &owner, &id, "t9", "x").0, 404);
    assert_eq!(edit(&ts, &owner, &id, "bogus", "x").0, 404);
    // Delete: tombstone keeps id, clears body + mentions, records {at, by}; root tombstone keeps
    // the Thread and its Replies; fully tombstoned ⇒ hidden from lists/counts, readable by id.
    let (status, e) = delete(&ts, &agent, &id, "t1.1");
    assert_eq!(status, 403, "{e}");
    let (status, c) = delete(&ts, &agent, &id, "t1");
    assert_eq!(status, 200, "{c}");
    assert_eq!(c["id"], "t1");
    assert_eq!(c["body"], "");
    assert_eq!(c["deleted"]["by"], "scribe");
    assert_eq!(c["changed"], true);
    assert_eq!(delete(&ts, &agent, &id, "t1").1["changed"], false, "idempotent");
    let (status, e) = edit(&ts, &agent, &id, "t1", "x");
    assert_eq!((status, e["error"].as_str()), (409, Some("deleted")));
    let list = threads(&ts, &owner, &id, "");
    assert_eq!(ids(&list), vec!["t1"]);
    assert!(list[0]["deleted"].is_object() && list[0]["reply_count"] == 1);
    let (status, c) = delete(&ts, &owner, &id, "t1.1");
    assert_eq!((status, c["changed"].as_bool()), (200, Some(true)));
    assert!(threads(&ts, &owner, &id, "").is_empty(), "fully tombstoned Threads are hidden");
    assert_eq!(threads(&ts, &owner, &id, "hidden=1").len(), 1);
    let (status, t) = ts.get(&format!("/api/v1/docs/{id}/threads/t1"), Some(&owner));
    assert_eq!((status, t["hidden"].as_bool()), (200, Some(true)));
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&owner));
    assert_eq!(st["open_threads"], 0);
    // A new Reply revives it in the lists.
    assert_eq!(reply(&ts, &owner, &id, "t1", "back", false).0, 201);
    assert_eq!(threads(&ts, &owner, &id, "").len(), 1);
    let _ = agent_id;
}

// spec: S04-09
#[test]
fn body_cap_names_the_cap_and_the_current_count() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let id = create(&ts, &owner, "plan", "text\n");
    let (status, _) = new_thread(&ts, &owner, &id, document(&"ü".repeat(60_000)));
    assert_eq!(status, 201);
    let (status, e) = new_thread(&ts, &owner, &id, document(&"x".repeat(40_001)));
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "body_cap");
    assert_eq!(
        e["message"],
        "Comment bodies in this Document total 60000 of 100000 characters — 40001 more would exceed the cap"
    );
    assert_eq!(new_thread(&ts, &owner, &id, document(&"x".repeat(40_000))).0, 201);
    let (status, e) = reply(&ts, &owner, &id, "t1", "1", false);
    assert_eq!((status, e["error"].as_str()), (409, Some("body_cap")));
    // Editing counts the old body out; tombstoning frees it.
    assert_eq!(edit(&ts, &owner, &id, "t2", &"y".repeat(40_000)).0, 200);
    assert_eq!(edit(&ts, &owner, &id, "t2", &"y".repeat(40_001)).0, 409);
    assert_eq!(delete(&ts, &owner, &id, "t2").0, 200);
    assert_eq!(reply(&ts, &owner, &id, "t1", &"z".repeat(40_000), false).0, 201);
}

// spec: S04-06, S04-10, S04-14
#[test]
fn mentions_resolve_at_post_time_and_survive_renames() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("scribe");
    let (_linter_id, linter) = ts.add_member("linter");
    let id = create(&ts, &owner, "plan", PLAN_V1);
    let (_, t) = new_thread(&ts, &owner, &id, document("@scribe and @nobody, not `@linter`\n```\n@linter\n```\n"));
    assert_eq!(t["mentions"], json!(["scribe"]), "unknown handles stay text; code is skipped");
    assert_eq!(t["body"], "@scribe and @nobody, not `@linter`\n```\n@linter\n```\n", "verbatim");
    let (_, t) = new_thread(&ts, &owner, &id, span("20:41-20:73", "Five? @linter"));
    assert_eq!(t["mentions"], json!(["linter"]));
    // Stored as Member ids: a display-name change leaves handles and mentions intact.
    let conn = rusqlite::Connection::open(ts.data_dir.join("store/annotary.sqlite")).unwrap();
    conn.execute("UPDATE members SET display_name = 'Scribe (renamed)' WHERE id = ?1", [&agent_id]).unwrap();
    drop(conn);
    let (_, t1) = ts.get(&format!("/api/v1/docs/{id}/threads/t1"), Some(&owner));
    assert_eq!(t1["mentions"], json!(["scribe"]));
    let sc = annotary_sidecar::parse(&sidecar_text(&ts, &owner, &id)).unwrap();
    assert_eq!(sc.members.get("scribe").unwrap().name, "Scribe (renamed)");
    assert_eq!(sc.members.get("scribe").unwrap().id, agent_id);
    assert_eq!(sc.threads[0].mentions, vec!["scribe"]);
    assert!(sc.members.get("linter").is_some(), "every mentioned handle is in the members table");

    // `/api/v1/mentions`: open Threads with a live mentioning Comment, per caller, across Documents.
    let id2 = create(&ts, &owner, "other", "other\n");
    std::thread::sleep(std::time::Duration::from_millis(3));
    new_thread(&ts, &agent, &id2, document("ping @linter"));
    let (status, m) = ts.get("/api/v1/mentions", Some(&linter));
    assert_eq!(status, 200, "{m}");
    let hits = m["mentions"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0]["doc"]["id"], id2, "most recent activity first");
    assert_eq!(hits[0]["thread"]["id"], "t1");
    assert_eq!(hits[1]["thread"]["id"], "t2");
    let (_, m) = ts.get("/api/v1/mentions", Some(&owner));
    assert!(m["mentions"].as_array().unwrap().is_empty());
    let (_, m) = ts.get("/api/v1/mentions?vault=notes", Some(&agent));
    assert_eq!(m["mentions"].as_array().unwrap().len(), 1);
    let since = hits[0]["thread"]["updated_at"].as_str().unwrap();
    let (_, m) = ts.get(&format!("/api/v1/mentions?since={since}"), Some(&linter));
    assert_eq!(m["mentions"].as_array().unwrap().len(), 1);
    assert_eq!(ts.get("/api/v1/mentions?since=junk", Some(&linter)).0, 400);
    // Resolving drops the hit; tombstoning the mentioning Comment drops it too.
    ts.post(&format!("/api/v1/docs/{id2}/threads/t1/resolve"), Some(&linter), &json!({}));
    let (_, m) = ts.get("/api/v1/mentions", Some(&linter));
    assert_eq!(m["mentions"].as_array().unwrap().len(), 1);
    delete(&ts, &owner, &id, "t2");
    let (_, m) = ts.get("/api/v1/mentions", Some(&linter));
    assert!(m["mentions"].as_array().unwrap().is_empty());
    // A Reply's mention counts too (root or reply).
    reply(&ts, &agent, &id, "t1", "@linter have a look", false);
    let (_, m) = ts.get("/api/v1/mentions", Some(&linter));
    assert_eq!(m["mentions"].as_array().unwrap().len(), 1);
    let (_, list) = ts.get("/api/v1/docs?mentions_me=1", Some(&linter));
    assert_eq!(list["docs"].as_array().unwrap().len(), 1);
    assert_eq!(list["docs"][0]["mentions_me"], 1);
}

// spec: S04-11, S04-05
#[test]
fn list_ordering_and_filters() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let id = create(&ts, &owner, "plan", "alpha beta\r\ngamma delta\r\nepsilon\r\n");
    // CRLF tolerated: columns are code points on the line without the `\r`.
    let (status, t1) = new_thread(&ts, &owner, &id, span("2:6-2:11", "on delta"));
    assert_eq!(status, 201, "{t1}");
    assert_eq!(t1["anchor"]["original"]["exact"], "delta");
    new_thread(&ts, &owner, &id, document("whole doc"));
    new_thread(&ts, &owner, &id, span("1:0-1:5", "on alpha @scribe"));
    new_thread(&ts, &owner, &id, span("2:0-2:5", "on gamma"));
    let (status, e) = new_thread(&ts, &owner, &id, span("2:0-2:12", "past the CR"));
    assert_eq!(status, 409, "{e}");
    std::thread::sleep(std::time::Duration::from_millis(3));
    reply(&ts, &agent, &id, "t1", "reply", false);
    // Document-level first, then by current position; `sort=recent`; status filters.
    assert_eq!(ids(&threads(&ts, &owner, &id, "")), vec!["t2", "t3", "t4", "t1"]);
    assert_eq!(ids(&threads(&ts, &owner, &id, "sort=recent"))[0], "t1");
    ts.post(&format!("/api/v1/docs/{id}/threads/t4/resolve"), Some(&owner), &json!({}));
    assert_eq!(ids(&threads(&ts, &owner, &id, "")), vec!["t2", "t3", "t1"]);
    assert_eq!(ids(&threads(&ts, &owner, &id, "status=resolved")), vec!["t4"]);
    assert_eq!(ids(&threads(&ts, &owner, &id, "status=all")), vec!["t2", "t3", "t4", "t1"]);
    assert_eq!(ids(&threads(&ts, &agent, &id, "mentions_me=1")), vec!["t3"]);
    assert!(threads(&ts, &owner, &id, "mentions_me=1").is_empty());
    let (_, body) = ts.get(&format!("/api/v1/docs/{id}/threads"), Some(&owner));
    assert_eq!(body["resolved_hidden"], 1);
    let recent = threads(&ts, &owner, &id, "sort=recent");
    let since = recent[0]["updated_at"].as_str().unwrap().to_string();
    assert_eq!(ids(&threads(&ts, &owner, &id, &format!("since={since}"))), vec!["t1"]);
    assert_eq!(ts.get(&format!("/api/v1/docs/{id}/threads?status=weird"), Some(&owner)).0, 400);
    assert_eq!(ts.get(&format!("/api/v1/docs/{id}/threads?sort=age"), Some(&owner)).0, 400);
    // Replies chronological, with the Thread's `reply_count` and `updated_at`.
    reply(&ts, &owner, &id, "t1", "second", false);
    let (_, t1) = ts.get(&format!("/api/v1/docs/{id}/threads/t1"), Some(&owner));
    assert_eq!(
        t1["replies"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["t1.1", "t1.2"]
    );
    assert_eq!(t1["reply_count"], 2);
    assert_eq!(t1["updated_at"], t1["replies"][1]["created"]["at"]);
}

// spec: S04-14
#[test]
fn sidecar_projects_the_prototype_scenario_threads_and_round_trips() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    // In handle order: `list_members` sorts by creation time, then handle, and both may land in one millisecond.
    let (_, linter) = ts.add_member("linter");
    let (_, agent) = ts.add_member("scribe");
    let id = create(&ts, &owner, "plan", PLAN_V1);
    // t1: document-level, resolved via reply --resolve, one mention.
    new_thread(&ts, &owner, &id, document("Needs a **Risks** section before I can approve. @scribe can you add one?"));
    reply(&ts, &agent, &id, "t1", "Added §Risks in v2.", true);
    // t2: anchored span, open, mentions both ways.
    new_thread(
        &ts,
        &owner,
        &id,
        span(
            "16:0-16:33",
            "Agreed — but the skill file must land *with* the CLI, not after. @scribe make sure the order list says so.",
        ),
    );
    reply(
        &ts,
        &agent,
        &id,
        "t2",
        "Item 3 already lists the skill file with the CLI. @rowan do you want it as its own item?",
        false,
    );
    // t3: edited root with two mentions, resolved by a third Member.
    new_thread(&ts, &agent, &id, span("11:48-11:69", "Should the anchor engine be its own package? @rowan @linter"));
    edit(
        &ts,
        &agent,
        &id,
        "t3",
        "Should the anchor engine be its own package so the CLI and server share one implementation? @rowan @linter",
    );
    reply(&ts, &linter, &id, "t3", "Yes — one package, both import it.", false);
    ts.post(&format!("/api/v1/docs/{id}/threads/t3/resolve"), Some(&linter), &json!({}));
    // t4: a tombstoned reply.
    new_thread(&ts, &owner, &id, span("20:41-20:73", "Three feels low for useful signal. Five?"));
    reply(&ts, &agent, &id, "t4", "oops", false);
    delete(&ts, &agent, &id, "t4.1");
    // t5: tombstoned root with a live reply.
    new_thread(&ts, &linter, &id, span("3:0-3:14", "Diagram is out of date."));
    reply(&ts, &owner, &id, "t5", "(Replying to the deleted question: yes.)", false);
    delete(&ts, &linter, &id, "t5");

    let text = sidecar_text(&ts, &owner, &id);
    let sc = annotary_sidecar::parse(&text).unwrap();
    // The writer is canonical: emit(parse(text)) == text.
    assert_eq!(annotary_sidecar::emit(&sc), text);
    assert_eq!(sc.members.0.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(), vec!["rowan", "linter", "scribe"]);
    let t: Vec<&annotary_sidecar::Thread> = sc.threads.iter().collect();
    assert_eq!(t.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t1", "t2", "t3", "t4", "t5"]);
    // t1
    assert_eq!((t[0].subject.as_str(), t[0].anchor.is_none()), ("document", true));
    assert_eq!(t[0].mentions, vec!["scribe"]);
    let res = t[0].resolved.as_ref().unwrap();
    assert_eq!((res.by.as_str(), res.version), ("scribe", 1));
    assert_eq!(t[0].replies[0].id, "t1.1");
    assert_eq!(t[0].replies[0].author, "scribe");
    // t2
    let a = t[1].anchor.as_ref().unwrap();
    assert_eq!(a.original.range.as_deref(), Some("16:0-16:33"));
    assert_eq!(a.original.exact.as_deref(), Some("We ship the CLI before the web UI"));
    assert_eq!(a.current.status.as_deref(), Some("anchored"));
    assert_eq!(a.current.exact, None, "current.exact omitted when equal to the original");
    assert_eq!(a.history.len(), 1);
    assert_eq!(t[1].replies[0].mentions, vec!["rowan"]);
    assert!(t[1].resolved.is_none());
    // t3
    assert!(t[2].edited_at.is_some());
    assert_eq!(t[2].mentions, vec!["rowan", "linter"]);
    assert_eq!(t[2].resolved.as_ref().unwrap().by, "linter");
    // t4: tombstoned reply keeps its id, empty body, `deleted {at, by}`.
    let r = &t[3].replies[0];
    assert_eq!((r.id.as_str(), r.body.as_str(), r.deleted.as_ref().unwrap().by.as_str()), ("t4.1", "", "scribe"));
    // t5: tombstoned root, live reply.
    assert_eq!(t[4].body, "");
    assert_eq!(t[4].deleted.as_ref().unwrap().by, "linter");
    assert_eq!(t[4].replies[0].author, "rowan");
    // The text is the leaf-inline layout with seconds precision and the decided key order.
    assert!(text.contains("\"created\": {\"at\": \"20"));
    assert!(!text.contains(".000Z") && !text.contains("\"null\""));
    let t2_pos = text.find("\"id\": \"t2\"").unwrap();
    let t2_block = &text[t2_pos..text.find("\"id\": \"t3\"").unwrap()];
    for (a, b) in [
        ("\"subject\"", "\"anchor\""),
        ("\"anchor\"", "\"author\""),
        ("\"author\"", "\"created\""),
        ("\"created\"", "\"body\""),
        ("\"body\"", "\"mentions\""),
        ("\"mentions\"", "\"replies\""),
    ] {
        assert!(t2_block.find(a).unwrap() < t2_block.find(b).unwrap(), "{a} before {b}");
    }
    // Hand edits to threads change nothing push reads.
    let edited = text.replace("Three feels low", "HAND EDITED");
    assert_eq!(
        annotary_sidecar::Binding::from_text(&edited).unwrap(),
        annotary_sidecar::Binding::from_text(&text).unwrap()
    );
}
