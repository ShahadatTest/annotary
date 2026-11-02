//! `/api/v1/docs/{id}/actions`, `/api/v1/vaults/{name}/actions` and the Action fields on docs /
//! status / list / Sidecar over HTTP against the in-process server (ticket 05): head-only guard,
//! undeclared ids, per-Version clearing, last-event-wins with two Members, per-Document
//! declarations (`[]`, redeclaration keeps snapshots, invalid ⇒ 400), Vault defaults
//! (owner-only, only future Actions of inheriting Documents), rights, events, purge, Sidecar
//! `actions` block round trip.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

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

fn create_with(ts: &TestServer, token: &str, name: &str, md: &str, extra: Value) -> String {
    let hash = upload(ts, token, md.as_bytes());
    let mut body = json!({ "vault": "notes", "name": name, "md_hash": hash });
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            body[k] = v.clone();
        }
    }
    let (status, doc) = ts.post("/api/v1/docs", Some(token), &body);
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn create(ts: &TestServer, token: &str, name: &str, md: &str) -> String {
    create_with(ts, token, name, md, json!({}))
}

fn push_with(ts: &TestServer, token: &str, id: &str, md: &str, extra: Value) -> (u16, Value) {
    let hash = upload(ts, token, md.as_bytes());
    let mut body = json!({ "name": "plan", "md_hash": hash });
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            body[k] = v.clone();
        }
    }
    ts.post(&format!("/api/v1/docs/{id}/versions"), Some(token), &body)
}

fn push(ts: &TestServer, token: &str, id: &str, md: &str) -> u64 {
    let (status, v) = push_with(ts, token, id, md, json!({}));
    assert_eq!(status, 201, "{v}");
    v["version"].as_u64().unwrap()
}

fn act(ts: &TestServer, token: &str, id: &str, action: &str, version: Option<u32>) -> (u16, Value) {
    ts.post(&format!("/api/v1/docs/{id}/actions"), Some(token), &json!({ "action": action, "version": version }))
}

fn put_json(ts: &TestServer, token: &str, path: &str, body: &Value) -> (u16, Value) {
    let res = ureq::put(format!("{}{}", ts.url, path))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(body);
    let mut r = res.unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn sidecar_text(ts: &TestServer, token: &str, id: &str) -> String {
    let mut r = ureq::get(format!("{}/api/v1/docs/{id}/sidecar", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .unwrap();
    r.body_mut().read_to_string().unwrap()
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x["id"].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

// spec: S05-02, S05-03, S05-04, S05-05, S05-09, S05-10, S05-11
#[test]
fn act_is_head_only_last_wins_and_a_new_version_starts_clean() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let id = create(&ts, &owner, "plan", "# v1\n");

    // Nothing yet: status/doc/list say so, and none of them counts orphaned Threads.
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&owner));
    assert!(st["action"].is_null());
    assert!(st.get("orphaned").is_none() && st.get("orphaned_threads").is_none());
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&owner));
    assert!(d["action"].is_null());
    assert_eq!(ids(&d["available_actions"]), vec!["approve", "send-back"]);
    assert_eq!(d["actions_inherited"], true);
    assert!(d.get("orphaned").is_none());

    // --version guard: not head ⇒ 409 not_head, nothing recorded.
    let (status, e) = act(&ts, &owner, &id, "approve", Some(2));
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "not_head");
    assert!(e["message"].as_str().unwrap().contains("head is v1, not v2"));
    // Undeclared id ⇒ 409 undeclared_action naming the available ids.
    let (status, e) = act(&ts, &owner, &id, "ship", None);
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "undeclared_action");
    assert!(e["message"].as_str().unwrap().contains("no Action \"ship\" on plan"), "{e}");
    assert_eq!(e["hint"], "available: approve, send-back");
    assert_eq!(e["available"], json!(["approve", "send-back"]));
    let (status, e) = act(&ts, &owner, &id, "", None);
    assert_eq!(status, 400, "{e}");
    let (_, log) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    assert_eq!(log["actions"].as_array().unwrap().len(), 0);

    // Take one: the event is snapshotted `{id, by, at, version, label, intent}`.
    let seq0 = ts.store.last_seq().unwrap();
    let (status, a) = act(&ts, &owner, &id, "approve", Some(1));
    assert_eq!(status, 201, "{a}");
    assert_eq!(a["id"], "approve");
    assert_eq!(a["by"], "rowan");
    assert_eq!(a["version"], 1);
    assert_eq!(a["label"], "Approve");
    assert_eq!(a["intent"], "positive");
    assert_eq!(a["doc"], id);
    assert_eq!(a["name"], "plan");
    assert!(a["at"].as_str().unwrap().ends_with('Z'));
    let events = ts.store.events_since(seq0).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "action.taken");
    assert_eq!(events[0].doc_id.as_deref(), Some(id.as_str()));
    assert_eq!(events[0].by.as_deref(), Some(ts.owner_id.as_str()));
    assert_eq!(
        events[0].payload,
        json!({ "action": "approve", "version": 1, "label": "Approve", "intent": "positive", "name": "plan" })
    );

    // status / doc / list carry it; the list filter is live.
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&agent));
    assert_eq!(st["action"]["id"], "approve");
    assert_eq!(st["action"]["by"], "rowan");
    assert_eq!(st["action"]["version"], 1);
    assert_eq!(st["version"], 1);
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&agent));
    assert_eq!(d["action"]["id"], "approve");
    let (_, l) = ts.get("/api/v1/docs?action=approve", Some(&agent));
    assert_eq!(l["docs"].as_array().unwrap().len(), 1);
    assert_eq!(l["docs"][0]["action"]["id"], "approve");
    let (_, l) = ts.get("/api/v1/docs?action=none", Some(&agent));
    assert_eq!(l["docs"].as_array().unwrap().len(), 0);
    let (_, l) = ts.get("/api/v1/docs?action=send-back", Some(&agent));
    assert_eq!(l["docs"].as_array().unwrap().len(), 0);
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&agent));
    let after_act = d["updated_at"].as_str().unwrap().to_string();

    // Two Members, repeated Actions on one Version: all recorded, the last wins.
    let (status, b) = act(&ts, &agent, &id, "send-back", None);
    assert_eq!(status, 201, "{b}");
    assert_eq!(b["by"], "scribe");
    let (status, _) = act(&ts, &owner, &id, "approve", None);
    assert_eq!(status, 201);
    let (status, _) = act(&ts, &agent, &id, "send-back", None);
    assert_eq!(status, 201);
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&owner));
    assert_eq!((st["action"]["id"].as_str(), st["action"]["by"].as_str()), (Some("send-back"), Some("scribe")));
    let (_, log) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    let entries = log["actions"].as_array().unwrap();
    assert_eq!(entries.len(), 4, "append-only, oldest first");
    assert_eq!(ids(&log["actions"]), vec!["approve", "send-back", "approve", "send-back"]);
    assert_eq!(entries[1]["by"], "scribe");
    assert_eq!(log["action"]["id"], "send-back");
    assert_eq!(log["version"], 1);
    assert_eq!(ids(&log["available"]), vec!["approve", "send-back"]);
    assert_eq!(log["inherited"], true);
    let (_, l) = ts.get("/api/v1/docs?action=send-back", Some(&agent));
    assert_eq!(l["docs"].as_array().unwrap().len(), 1);
    let (_, l) = ts.get("/api/v1/docs?action=approve", Some(&agent));
    assert_eq!(l["docs"].as_array().unwrap().len(), 0, "the last event on the Version wins");
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&agent));
    assert!(d["updated_at"].as_str().unwrap() >= after_act.as_str(), "an Action bumps activity");

    // A new Version starts with no Action; the log keeps v1's four.
    assert_eq!(push(&ts, &owner, &id, "# v2\n"), 2);
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&owner));
    assert_eq!(st["version"], 2);
    assert!(st["action"].is_null(), "{st}");
    let (_, l) = ts.get("/api/v1/docs?action=none", Some(&owner));
    assert_eq!(l["docs"].as_array().unwrap().len(), 1);
    let (_, l) = ts.get("/api/v1/docs?action=send-back", Some(&owner));
    assert_eq!(l["docs"].as_array().unwrap().len(), 0);
    let (_, log) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    assert_eq!(log["actions"].as_array().unwrap().len(), 4);
    assert!(log["action"].is_null());
    // The old --version now fails; the new head works.
    let (status, e) = act(&ts, &owner, &id, "approve", Some(1));
    assert_eq!(status, 409, "{e}");
    let (status, a) = act(&ts, &owner, &id, "approve", Some(2));
    assert_eq!(status, 201, "{a}");
    assert_eq!(a["version"], 2);
    let (_, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&owner));
    assert_eq!(st["action"]["id"], "approve");
    assert_eq!(st["action"]["version"], 2);
}

// spec: S05-06, S05-07, S05-08, S05-15, S05-16
#[test]
fn per_document_declarations_replace_the_vault_set_and_snapshots_survive() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    // Declared at creation: only `ship` (label defaults to the id, intent to neutral).
    let id = create_with(&ts, &owner, "plan", "# v1\n", json!({ "actions_available": [{ "id": "ship" }] }));
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&owner));
    assert_eq!(d["available_actions"], json!([{ "id": "ship", "label": "ship", "intent": "neutral" }]));
    assert_eq!(d["actions_inherited"], false);
    let (status, e) = act(&ts, &owner, &id, "approve", None);
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["hint"], "available: ship");
    let (status, a) = act(&ts, &owner, &id, "ship", None);
    assert_eq!(status, 201, "{a}");
    assert_eq!((a["label"].as_str(), a["intent"].as_str()), (Some("ship"), Some("neutral")));

    // Redeclare on an `already` push (same content): the declaration still applies, and the
    // log keeps its snapshot.
    let (status, v) = push_with(
        &ts,
        &owner,
        &id,
        "# v1\n",
        json!({ "actions_available": [{ "id": "ship", "label": "Ship it", "intent": "positive" }, { "id": "hold", "label": "Hold", "intent": "negative" }] }),
    );
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["already"], true);
    let (_, log) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    assert_eq!(ids(&log["available"]), vec!["ship", "hold"]);
    assert_eq!(log["available"][0]["label"], "Ship it");
    assert_eq!(log["actions"][0]["label"], "ship", "snapshot survives the redeclaration");
    let (status, a) = act(&ts, &owner, &id, "ship", None);
    assert_eq!(status, 201, "{a}");
    assert_eq!((a["label"].as_str(), a["intent"].as_str()), (Some("Ship it"), Some("positive")));
    let (_, log) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    assert_eq!(log["actions"][0]["label"], "ship");
    assert_eq!(log["actions"][1]["label"], "Ship it");
    let declared: Vec<_> =
        ts.store.events_since(0).unwrap().into_iter().filter(|e| e.kind == "document.actions_declared").collect();
    assert_eq!(declared.len(), 2, "creation + redeclaration");
    assert_eq!(declared[1].payload["available"][1]["id"], "hold");

    // Nothing sent ⇒ the declaration stays (also across a real new Version).
    assert_eq!(push(&ts, &owner, &id, "# v2\n"), 2);
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&owner));
    assert_eq!(ids(&d["available_actions"]), vec!["ship", "hold"]);
    assert_eq!(d["actions_inherited"], false);
    assert!(d["action"].is_null(), "v2 starts clean");

    // `[]` declares none.
    let (status, _) = push_with(&ts, &owner, &id, "# v3\n", json!({ "actions_available": [] }));
    assert_eq!(status, 201);
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&owner));
    assert_eq!(d["available_actions"], json!([]));
    assert_eq!(d["actions_inherited"], false);
    let (status, e) = act(&ts, &owner, &id, "ship", None);
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "undeclared_action");
    assert!(e["hint"].as_str().unwrap().contains("declares no Actions"), "{e}");
    assert_eq!(e["available"], json!([]));

    // Invalid declarations are 400 invalid_actions and change nothing.
    for bad in [
        json!({ "id": "x" }),
        json!([{ "id": "Ship It" }]),
        json!([{ "id": "none" }]),
        json!([{ "id": "a" }, { "id": "a" }]),
        json!([{ "id": "a", "intent": "great" }]),
        json!(["approve"]),
    ] {
        let (status, e) = push_with(&ts, &owner, &id, "# v3\n", json!({ "actions_available": bad }));
        assert_eq!(status, 400, "{e}");
        assert_eq!(e["error"], "invalid_actions", "{e}");
    }
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&owner));
    assert_eq!(d["available_actions"], json!([]));
    assert_eq!(d["version"], 3);
    // Same at creation.
    let hash = upload(&ts, &owner, b"# other\n");
    let (status, e) = ts.post(
        "/api/v1/docs",
        Some(&owner),
        &json!({ "vault": "notes", "name": "other", "md_hash": hash, "actions_available": [{ "id": "-bad" }] }),
    );
    assert_eq!(status, 400, "{e}");
    assert_eq!(e["error"], "invalid_actions");
}

// spec: S05-01, S05-13
#[test]
fn vault_defaults_are_owner_only_and_only_touch_future_actions_of_inheriting_documents() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let inheriting = create(&ts, &owner, "a", "# a\n");
    let own = create_with(
        &ts,
        &owner,
        "b",
        "# b\n",
        json!({ "actions_available": [{ "id": "approve", "label": "Own approve", "intent": "positive" }] }),
    );
    let (status, a) = act(&ts, &owner, &inheriting, "approve", None);
    assert_eq!(status, 201, "{a}");
    assert_eq!(a["label"], "Approve");

    // GET shows the defaults; PUT is owner-only.
    let (status, v) = ts.get("/api/v1/vaults/notes/actions", Some(&agent));
    assert_eq!(status, 200, "{v}");
    assert_eq!(ids(&v["available"]), vec!["approve", "send-back"]);
    let new = json!({ "available": [{ "id": "lgtm", "label": "LGTM", "intent": "positive" }, { "id": "rework", "label": "Rework", "intent": "negative" }] });
    let (status, e) = put_json(&ts, &agent, "/api/v1/vaults/notes/actions", &new);
    assert_eq!(status, 403, "{e}");
    assert!(e["message"].as_str().unwrap().contains("owner-only"));
    let (status, e) = put_json(&ts, &owner, "/api/v1/vaults/nope/actions", &new);
    assert_eq!(status, 404, "{e}");
    let (status, e) =
        put_json(&ts, &owner, "/api/v1/vaults/notes/actions", &json!({ "available": [{ "id": "None" }] }));
    assert_eq!(status, 400, "{e}");
    assert_eq!(e["error"], "invalid_actions");
    let (status, v) = put_json(&ts, &owner, "/api/v1/vaults/notes/actions", &new);
    assert_eq!(status, 200, "{v}");
    assert_eq!(ids(&v["available"]), vec!["lgtm", "rework"]);
    assert_eq!(v["name"], "notes");
    let (_, v) = ts.get("/api/v1/vaults", Some(&owner));
    assert_eq!(ids(&v["vaults"][0]["default_actions"]), vec!["lgtm", "rework"]);
    assert!(ts.store.events_since(0).unwrap().iter().any(|e| e.kind == "vault.actions_set"));

    // Inheriting Document: new set, old Actions keep their snapshot; own Document untouched.
    let (_, d) = ts.get(&format!("/api/v1/docs/{inheriting}"), Some(&owner));
    assert_eq!(ids(&d["available_actions"]), vec!["lgtm", "rework"]);
    assert_eq!(d["actions_inherited"], true);
    assert_eq!(d["action"]["id"], "approve", "the Action already taken stays reported");
    let (status, e) = act(&ts, &owner, &inheriting, "approve", None);
    assert_eq!(status, 409, "{e}");
    let (status, a) = act(&ts, &agent, &inheriting, "lgtm", None);
    assert_eq!(status, 201, "{a}");
    let (_, log) = ts.get(&format!("/api/v1/docs/{inheriting}/actions"), Some(&owner));
    assert_eq!(log["actions"][0]["label"], "Approve");
    assert_eq!(log["actions"][1]["label"], "LGTM");
    let (_, d) = ts.get(&format!("/api/v1/docs/{own}"), Some(&owner));
    assert_eq!(d["available_actions"][0]["label"], "Own approve");
    assert_eq!(d["actions_inherited"], false);
    let (status, a) = act(&ts, &owner, &own, "approve", None);
    assert_eq!(status, 201, "{a}");
    assert_eq!(a["label"], "Own approve");
    // A new Vault starts with the built-in defaults, not the edited ones.
    ts.create_vault("drafts");
    let (_, v) = ts.get("/api/v1/vaults/drafts/actions", Some(&owner));
    assert_eq!(ids(&v["available"]), vec!["approve", "send-back"]);
}

// spec: S05-13, S05-03
#[test]
fn rights_deleted_documents_and_purge() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (commenter_id, commenter) = ts.add_member("commenter");
    ts.store
        .set_grants(
            &ts.owner_id,
            &commenter_id,
            vec![Grant { vault: "notes".into(), rights: vec![Right::Read, Right::Comment, Right::Write] }],
        )
        .unwrap();
    let (actor_id, actor) = ts.add_member("actor");
    ts.store
        .set_grants(&ts.owner_id, &actor_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Action] }])
        .unwrap();
    let (nobody_id, nobody) = ts.add_member("nobody");
    ts.store.set_grants(&ts.owner_id, &nobody_id, vec![]).unwrap();
    let id = create(&ts, &owner, "plan", "# a\n");

    // `act` needs `action`, named in the refusal; `read` reads; none ⇒ 403 on everything.
    let (status, e) = act(&ts, &commenter, &id, "approve", None);
    assert_eq!(status, 403, "{e}");
    assert!(
        e["message"].as_str().unwrap().contains("missing right `action` on Vault notes (you are @commenter)"),
        "{e}"
    );
    let (status, a) = act(&ts, &actor, &id, "approve", None);
    assert_eq!(status, 201, "{a}");
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&commenter));
    assert_eq!(status, 200);
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&actor));
    assert_eq!(status, 200, "read is implied by action");
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&nobody));
    assert_eq!(status, 403);
    let (status, _) = act(&ts, &nobody, &id, "approve", None);
    assert_eq!(status, 403);
    let (status, _) = ts.get("/api/v1/vaults/notes/actions", Some(&nobody));
    assert_eq!(status, 403);
    // Declaring needs `write` (a push): the commenter has it, the actor does not.
    let (status, v) = push_with(&ts, &commenter, &id, "# b\n", json!({ "actions_available": [] }));
    assert_eq!(status, 201, "{v}");
    let hash = upload(&ts, &owner, b"# c\n");
    let (status, e) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&actor),
        &json!({ "name": "plan", "md_hash": hash, "actions_available": [{ "id": "x" }] }),
    );
    assert_eq!(status, 403, "{e}");
    assert!(e["message"].as_str().unwrap().contains("missing right `write`"), "{e}");
    let (status, e) = act(&ts, &actor, &id, "approve", None);
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "undeclared_action");
    let (status, e) = act(&ts, &nobody, &id, "approve", None);
    assert_eq!(status, 403, "{e}");
    let (status, e) = act(&ts, &owner, "zzzzzzzz", "approve", None);
    assert_eq!(status, 404, "{e}");

    // Deleted ⇒ 409 deleted for act; the log stays readable; purge drops it.
    let (status, _) = ureq::delete(format!("{}/api/v1/docs/{id}", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .call()
        .map(|r| (r.status().as_u16(), ()))
        .unwrap();
    assert_eq!(status, 200);
    let (status, e) = act(&ts, &owner, &id, "approve", None);
    assert_eq!(status, 409, "{e}");
    assert_eq!(e["error"], "deleted");
    let (status, log) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    assert_eq!(status, 200);
    assert_eq!(log["actions"].as_array().unwrap().len(), 1);
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/purge"), Some(&owner), &json!({}));
    assert_eq!(status, 200);
    assert!(ts.store.actions(&id).unwrap().is_empty());
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&owner));
    assert_eq!(status, 404);
}

// spec: S05-07, S05-12
#[test]
fn sidecar_actions_block_round_trips() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let id = create(&ts, &owner, "plan", "# v1\n");
    // Inherited, no log: `available` + `inherited: true`, no `log`.
    let text = sidecar_text(&ts, &owner, &id);
    let sc = annotary_sidecar::parse(&text).unwrap();
    assert_eq!(annotary_sidecar::emit(&sc), text);
    assert!(sc.actions.inherited);
    assert_eq!(sc.actions.available.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["approve", "send-back"]);
    assert!(sc.actions.log.is_empty());
    assert!(text.contains("  \"actions\": {\n    \"available\": [\n      {\"id\": \"approve\", \"label\": \"Approve\", \"intent\": \"positive\"},\n      {\"id\": \"send-back\", \"label\": \"Send back\", \"intent\": \"neutral\"}\n    ],\n    \"inherited\": true\n  }"), "{text}");
    // `inherited` present ⇒ push reads nothing.
    let b = annotary_sidecar::Binding::from_text(&text).unwrap();
    assert_eq!(b.actions_available, None);

    // With Actions taken (two Members), an older base_version and a redeclaration: the log is
    // the whole Document's, oldest first, handles + seconds, snapshotted label/intent.
    act(&ts, &owner, &id, "send-back", None);
    push(&ts, &owner, &id, "# v2\n");
    act(&ts, &agent, &id, "approve", None);
    let (status, _) = push_with(
        &ts,
        &owner,
        &id,
        "# v2\n",
        json!({ "actions_available": [{ "id": "approve", "label": "Approve!", "intent": "positive" }] }),
    );
    assert_eq!(status, 200);
    let text = sidecar_text(&ts, &owner, &id);
    let sc = annotary_sidecar::parse(&text).unwrap();
    assert_eq!(annotary_sidecar::emit(&sc), text);
    assert!(!sc.actions.inherited);
    assert_eq!(sc.actions.available.len(), 1);
    assert_eq!(sc.actions.available[0].label, "Approve!");
    assert_eq!(sc.actions.log.len(), 2);
    let l0 = &sc.actions.log[0];
    assert_eq!(
        (l0.id.as_str(), l0.by.as_str(), l0.version, l0.label.as_str(), l0.intent.as_str()),
        ("send-back", "rowan", 1, "Send back", "neutral")
    );
    let l1 = &sc.actions.log[1];
    assert_eq!((l1.id.as_str(), l1.by.as_str(), l1.version, l1.label.as_str()), ("approve", "scribe", 2, "Approve"));
    assert_eq!(l1.at.len(), 20, "seconds: {}", l1.at);
    assert!(text.contains("\"log\": [\n      {\"id\": \"send-back\", \"by\": \"rowan\", \"at\": \""), "{text}");
    assert!(sc.members.get("scribe").is_some(), "Action takers are in the members table");
    // Dropping nothing: the declaration comes back to push as-is; `inherited` absent ⇒ sent.
    let b = annotary_sidecar::Binding::from_text(&text).unwrap();
    assert_eq!(b.actions_available.as_ref().map(|d| d.len()), Some(1));
    // A `[]` declaration: `available` is omitted (empty), the log stays, no `inherited`.
    let (status, _) = push_with(&ts, &owner, &id, "# v2\n", json!({ "actions_available": [] }));
    assert_eq!(status, 200);
    let text = sidecar_text(&ts, &owner, &id);
    let sc = annotary_sidecar::parse(&text).unwrap();
    assert_eq!(annotary_sidecar::emit(&sc), text);
    assert!(sc.actions.available.is_empty() && !sc.actions.inherited && sc.actions.log.len() == 2);
    assert!(text.contains("  \"actions\": {\n    \"log\": [\n"), "{text}");
    // Editing an inherited Sidecar's `available` and dropping the flag = declaring.
    let other = create(&ts, &owner, "other", "# o\n");
    let text = sidecar_text(&ts, &owner, &other);
    let edited = text.replace("    ],\n    \"inherited\": true\n", "    ]\n").replace(
        "{\"id\": \"send-back\", \"label\": \"Send back\", \"intent\": \"neutral\"}",
        "{\"id\": \"ship\", \"label\": \"Ship\", \"intent\": \"positive\"}",
    );
    assert_ne!(edited, text);
    let b = annotary_sidecar::Binding::from_text(&edited).unwrap();
    assert_eq!(
        b.actions_available.as_ref().unwrap().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        vec!["approve", "ship"]
    );
    // Garbage in the read field is a typed error.
    assert!(annotary_sidecar::Binding::from_text(&edited.replace("{\"id\": \"ship\", ", "{")).is_err());
}
