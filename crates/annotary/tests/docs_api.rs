//! `/api/v1/docs`, `/api/v1/blobs`, `/api/v1/vaults` over HTTP against the in-process server:
//! two-phase push, already / same_as / head-moved, rights, list filters, resolve, sidecar
//! projection, delete/restore/purge.

use annotary::testing::TestServer;
use annotary_sidecar::identity::{sha256_hex, version_hash};
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

/// Upload `md` through the two-phase blob API and return its hash.
fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let (status, body) = ts.post("/api/v1/blobs/check", Some(token), &json!({ "hashes": [hash] }));
    assert_eq!(status, 200, "{body}");
    let lacking = body["lacking"].as_array().unwrap();
    if !lacking.is_empty() {
        let (status, body) = put_bytes(ts, token, &format!("/api/v1/blobs/{hash}"), md);
        assert!(status == 201 || status == 200, "{status} {body}");
        assert_eq!(body["hash"], hash);
    }
    hash
}

fn put_bytes(ts: &TestServer, token: &str, path: &str, bytes: &[u8]) -> (u16, Value) {
    let res = ureq::put(format!("{}{}", ts.url, path))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(bytes);
    match res {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
        Err(e) => (0, Value::String(e.to_string())),
    }
}

fn get_bytes(ts: &TestServer, token: &str, path: &str) -> (u16, Vec<u8>) {
    let mut r = ureq::get(format!("{}{}", ts.url, path))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    let status = r.status().as_u16();
    (status, r.body_mut().read_to_vec().unwrap())
}

fn create(ts: &TestServer, token: &str, name: &str, md: &[u8], message: Option<&str>) -> Value {
    let hash = upload(ts, token, md);
    let mut body = json!({ "vault": "notes", "name": name, "md_hash": hash });
    if let Some(m) = message {
        body["message"] = json!(m);
    }
    let (status, doc) = ts.post("/api/v1/docs", Some(token), &body);
    assert_eq!(status, 201, "{doc}");
    doc
}

fn push(ts: &TestServer, token: &str, id: &str, name: &str, md: &[u8], extra: Value) -> (u16, Value) {
    let hash = upload(ts, token, md);
    let mut body = json!({ "name": name, "md_hash": hash });
    if let Value::Object(m) = extra {
        for (k, v) in m {
            body[k] = v;
        }
    }
    ts.post(&format!("/api/v1/docs/{id}/versions"), Some(token), &body)
}

fn delete(ts: &TestServer, token: &str, id: &str) -> (u16, Value) {
    let mut r = ureq::delete(format!("{}/api/v1/docs/{id}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

// spec: S02-01, S02-09, S02-10, S02-11, S02-32
#[test]
fn two_phase_push_creates_versions_with_already_and_same_as() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let doc = create(&ts, &tok, "plan", b"# v1\n", Some("first draft"));
    let id = doc["id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 8);
    assert_eq!(doc["name"], "plan");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["vault"], "notes");
    assert_eq!(doc["by"], "rowan");
    assert_eq!(doc["message"], "first draft");
    assert_eq!(doc["already"], false);
    let md1 = sha256_hex(b"# v1\n");
    assert_eq!(doc["md_hash"], md1);
    assert_eq!(doc["hash"], version_hash(&md1, []));

    // The check phase reports what the server lacks; a second check finds it present.
    let (_, body) = ts.post("/api/v1/blobs/check", Some(&tok), &json!({ "hashes": [md1, sha256_hex(b"new")] }));
    assert_eq!(body["lacking"], json!([sha256_hex(b"new")]));

    // identical → 200 already
    let (status, again) = push(&ts, &tok, &id, "plan", b"# v1\n", json!({}));
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["already"], true);
    assert_eq!(again["version"], 1);
    // new content → 201 v2
    let (status, v2) = push(&ts, &tok, &id, "plan", b"# v2\n", json!({ "message": "edit" }));
    assert_eq!(status, 201, "{v2}");
    assert_eq!(v2["version"], 2);
    assert!(v2["same_as"].is_null());
    // back to v1's bytes → v3 same as v1
    let (status, v3) = push(&ts, &tok, &id, "plan", b"# v1\n", json!({}));
    assert_eq!(status, 201);
    assert_eq!(v3["version"], 3);
    assert_eq!(v3["same_as"], 1);
    // versions listed ascending with same_as derived
    let (status, vs) = ts.get(&format!("/api/v1/docs/{id}/versions"), Some(&tok));
    assert_eq!(status, 200);
    let list = vs["versions"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[2]["same_as"], 1);
    assert_eq!(list[1]["message"], "edit");
    let (status, one) = ts.get(&format!("/api/v1/docs/{id}/versions/2"), Some(&tok));
    assert_eq!(status, 200);
    assert_eq!(one["number"], 2);
    assert_eq!(one["head"], 3);
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}/versions/9"), Some(&tok));
    assert_eq!(status, 404);
    // the Markdown comes back byte-identical
    let (status, bytes) = get_bytes(&ts, &tok, &format!("/api/v1/blobs/{}", sha256_hex(b"# v2\n")));
    assert_eq!(status, 200);
    assert_eq!(bytes, b"# v2\n");
    // a commit whose blob was never uploaded is refused; a mismatching upload too
    let (status, body) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": sha256_hex(b"never") }),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "blob_missing");
    let (status, body) = put_bytes(&ts, &tok, &format!("/api/v1/blobs/{}", sha256_hex(b"a")), b"b");
    assert_eq!(status, 400, "{body}");
    let (status, _) = put_bytes(&ts, &tok, "/api/v1/blobs/nothex", b"b");
    assert_eq!(status, 400);
}

// spec: S02-12, S17-09
#[test]
fn a_moved_head_is_refused_unless_contained_or_discarded() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let doc = create(&ts, &tok, "plan", b"v1\n", None);
    let id = doc["id"].as_str().unwrap().to_string();
    push(&ts, &agent, &id, "plan", b"v2\n", json!({}));
    push(&ts, &agent, &id, "plan", b"v3\n", json!({}));
    // Ticket 17: no more warn-and-overwrite — a commit over a moved head that claims nothing is
    // refused (an older CLI, a raw API call), naming who changed what.
    let (status, out) = push(&ts, &tok, &id, "plan", b"mine\n", json!({ "base_version": 1 }));
    assert_eq!(status, 409, "{out}");
    assert_eq!(out["error"], "head_moved");
    assert_eq!((out["base"].clone(), out["head"].clone(), out["by"].clone()), (json!(1), json!(3), json!(["scribe"])));
    assert!(out["message"].as_str().unwrap().starts_with("v2–v3 by @scribe changed plan since your v1"), "{out}");
    // `strict` is no longer read; discarding is explicit and reported.
    let (status, out) =
        push(&ts, &tok, &id, "plan", b"mine\n", json!({ "base_version": 1, "discard_head_changes": true }));
    assert_eq!(status, 201, "{out}");
    assert_eq!((out["version"].clone(), out["discarded"].clone()), (json!(4), json!([2, 3])));
    assert_eq!(out["head_moved"], json!({ "base": 1, "head": 3, "by": ["scribe"] }));
    let (status, out) = push(&ts, &tok, &id, "plan", b"fine\n", json!({ "base_version": 4 }));
    assert_eq!(status, 201, "{out}");
    assert!(out["head_moved"].is_null());
    assert_eq!(out["discarded"], json!([]));
}

// spec: S02-31, S02-05
#[test]
fn rights_write_read_owner_and_name_rules() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (reader_id, reader) = ts.add_member("reader");
    // reader: read only on notes; nobody: no grants at all
    ts.store
        .set_grants(&ts.owner_id, &reader_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Read] }])
        .unwrap();
    let (_, writer) = ts.add_member("writer");
    let (nobody_id, nobody) = ts.add_member("nobody");
    ts.store.set_grants(&ts.owner_id, &nobody_id, vec![]).unwrap();

    let doc = create(&ts, &writer, "plan", b"v1\n", None);
    let id = doc["id"].as_str().unwrap().to_string();

    // reader may read but not push/delete; nobody may not even read
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}"), Some(&reader));
    assert_eq!(status, 200);
    let (status, body) = ts.get(&format!("/api/v1/docs/{id}"), Some(&nobody));
    assert_eq!(status, 403, "{body}");
    assert!(body["message"].as_str().unwrap().contains("`read`"));
    let (status, body) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&reader),
        &json!({ "name": "plan", "md_hash": sha256_hex(b"v1\n") }),
    );
    assert_eq!(status, 403, "{body}");
    assert!(body["message"].as_str().unwrap().contains("`write`"), "{body}");
    let (status, body) = ts.post("/api/v1/blobs/check", Some(&reader), &json!({ "hashes": [] }));
    assert_eq!(status, 403, "{body}");
    let (status, body) = ts.post(
        "/api/v1/docs",
        Some(&reader),
        &json!({ "vault": "notes", "name": "x", "md_hash": sha256_hex(b"v1\n") }),
    );
    assert_eq!(status, 403, "{body}");
    let (status, _) = delete(&ts, &reader, &id);
    assert_eq!(status, 403);
    // list for nobody is empty (no readable Vault), for reader it has the doc
    let (status, body) = ts.get("/api/v1/docs", Some(&nobody));
    assert_eq!(status, 200);
    assert_eq!(body["total"], 0);
    let (_, body) = ts.get("/api/v1/docs", Some(&reader));
    assert_eq!(body["total"], 1);
    let (_, body) = ts.get("/api/v1/vaults", Some(&nobody));
    assert_eq!(body["vaults"].as_array().unwrap().len(), 0);
    let (_, body) = ts.get("/api/v1/vaults", Some(&reader));
    assert_eq!(body["vaults"][0]["name"], "notes");
    // purge is owner-only even for a writer
    let (status, _) = delete(&ts, &writer, &id);
    assert_eq!(status, 200);
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/purge"), Some(&writer), &json!({}));
    assert_eq!(status, 403, "{body}");
    assert!(body["message"].as_str().unwrap().contains("owner-only"));
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/purge"), Some(&tok), &json!({}));
    assert_eq!(status, 200);
    // name rules are typed 400s
    for bad in ["", ".env", "a/b", "tab\tx"] {
        let (status, body) = ts.post(
            "/api/v1/docs",
            Some(&tok),
            &json!({ "vault": "notes", "name": bad, "md_hash": sha256_hex(b"v1\n") }),
        );
        assert_eq!(status, 400, "{bad:?}: {body}");
        assert_eq!(body["error"], "invalid_name");
    }
    // verbatim Unicode / case is fine
    let d = create(&ts, &tok, "Übersicht 2026", b"u\n", None);
    assert_eq!(d["name"], "Übersicht 2026");
    // unknown vault → 404
    let (status, _) =
        ts.post("/api/v1/docs", Some(&tok), &json!({ "vault": "nope", "name": "x", "md_hash": sha256_hex(b"v1\n") }));
    assert_eq!(status, 404);
}

// spec: S02-06, S02-07, S02-20, S02-21
#[test]
fn list_and_resolve() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let a = create(&ts, &tok, "plan", b"a\n", None);
    std::thread::sleep(std::time::Duration::from_millis(3));
    let b = create(&ts, &tok, "plan", b"b\n", None);
    std::thread::sleep(std::time::Duration::from_millis(3));
    let c = create(&ts, &tok, "roadmap", b"c\n", None);
    let (aid, bid, cid) = (a["id"].as_str().unwrap(), b["id"].as_str().unwrap(), c["id"].as_str().unwrap());

    let (status, body) = ts.get("/api/v1/docs", Some(&tok));
    assert_eq!(status, 200);
    let ids: Vec<&str> = body["docs"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![cid, bid, aid], "newest activity first");
    assert_eq!(body["total"], 3);
    assert_eq!(body["truncated"], false);
    assert_eq!(body["docs"][0]["open_threads"], 0);
    assert!(body["docs"][0]["action"].is_null());
    // limit + truncated + total
    let (_, body) = ts.get("/api/v1/docs?limit=2", Some(&tok));
    assert_eq!(body["docs"].as_array().unwrap().len(), 2);
    assert_eq!(body["truncated"], true);
    assert_eq!(body["total"], 3);
    // substring, exact, vault, since, deleted, action/mentions stubs
    let (_, body) = ts.get("/api/v1/docs?name=ROAD", Some(&tok));
    assert_eq!(body["total"], 1);
    let (_, body) = ts.get("/api/v1/docs?exact=plan", Some(&tok));
    assert_eq!(body["total"], 2);
    let (_, body) = ts.get("/api/v1/docs?vault=notes", Some(&tok));
    assert_eq!(body["total"], 3);
    let (status, _) = ts.get("/api/v1/docs?vault=nope", Some(&tok));
    assert_eq!(status, 404);
    let since = c["updated_at"].as_str().unwrap();
    let (_, body) = ts.get(&format!("/api/v1/docs?since={since}"), Some(&tok));
    assert_eq!(body["total"], 1);
    let (status, _) = ts.get("/api/v1/docs?since=yesterday", Some(&tok));
    assert_eq!(status, 400);
    let (_, body) = ts.get("/api/v1/docs?action=none", Some(&tok));
    assert_eq!(body["total"], 3);
    let (_, body) = ts.get("/api/v1/docs?action=approve", Some(&tok));
    assert_eq!(body["total"], 0);
    let (_, body) = ts.get("/api/v1/docs?mentions_me=1", Some(&tok));
    assert_eq!(body["total"], 0);
    // deleted filter
    delete(&ts, &tok, aid);
    let (_, body) = ts.get("/api/v1/docs", Some(&tok));
    assert_eq!(body["total"], 2);
    let (_, body) = ts.get("/api/v1/docs?deleted=only", Some(&tok));
    assert_eq!(body["total"], 1);
    assert_eq!(body["docs"][0]["id"], aid);
    let (_, body) = ts.get("/api/v1/docs?deleted=any", Some(&tok));
    assert_eq!(body["total"], 3);

    // resolve: id (even deleted), unique name, ambiguous name with candidates, unknown
    let (status, body) = ts.get(&format!("/api/v1/docs/resolve?doc={aid}"), Some(&tok));
    assert_eq!(status, 200);
    assert!(!body["deleted_at"].is_null());
    let (status, body) = ts.get("/api/v1/docs/resolve?doc=roadmap", Some(&tok));
    assert_eq!(status, 200);
    assert_eq!(body["id"], cid);
    // "plan": a is deleted so only b is live → unique
    let (status, body) = ts.get("/api/v1/docs/resolve?doc=plan", Some(&tok));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], bid);
    let (status, body) = ts.get("/api/v1/docs/resolve?doc=plan&deleted=any", Some(&tok));
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "ambiguous");
    assert!(body["message"].as_str().unwrap().starts_with("2 Documents named \"plan\""));
    let cands = body["candidates"].as_array().unwrap();
    assert_eq!(cands.len(), 2);
    assert!(cands[0]["id"].is_string() && cands[0]["version"].is_number() && cands[0]["updated_at"].is_string());
    let (status, body) = ts.get("/api/v1/docs/resolve?doc=plan&deleted=only", Some(&tok));
    assert_eq!(status, 200);
    assert_eq!(body["id"], aid);
    let (status, body) = ts.get("/api/v1/docs/resolve?doc=nothing&vault=notes", Some(&tok));
    assert_eq!(status, 404);
    assert_eq!(body["error"], "not_found");
    assert!(body["message"].as_str().unwrap().contains("in Vault notes"));
    let (status, _) = ts.get("/api/v1/docs/zzzzzzzz", Some(&tok));
    assert_eq!(status, 404);
    let (status, _) = ts.get("/api/v1/docs/resolve?doc=", Some(&tok));
    assert_eq!(status, 400);
}

// spec: S02-22, S02-23, S02-15, S02-28
#[test]
fn sidecar_projection_and_status() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let doc = create(&ts, &tok, "plan", b"v1\n", Some("first draft"));
    let id = doc["id"].as_str().unwrap().to_string();
    push(&ts, &agent, &id, "phase1-plan", b"v2\n", json!({}));

    let (status, text) = get_bytes(&ts, &tok, &format!("/api/v1/docs/{id}/sidecar"));
    assert_eq!(status, 200);
    let text = String::from_utf8(text).unwrap();
    let sc = annotary_sidecar::parse(&text).unwrap();
    assert_eq!(annotary_sidecar::emit(&sc), text, "the server serves the canonical text");
    assert_eq!(sc.id, id);
    assert_eq!(sc.vault, "notes");
    assert_eq!(sc.base_version, 2);
    assert_eq!(sc.versions.len(), 2);
    assert_eq!(sc.versions[0].by, "rowan");
    assert_eq!(sc.versions[0].message.as_deref(), Some("first draft"));
    assert_eq!(sc.versions[1].by, "scribe");
    assert_eq!(sc.versions[1].name, "phase1-plan");
    assert!(
        sc.versions[1].at.ends_with('Z') && sc.versions[1].at.len() == 20,
        "seconds precision: {}",
        sc.versions[1].at
    );
    let handles: Vec<&str> = sc.members.0.iter().map(|(h, _)| h.as_str()).collect();
    assert_eq!(handles, vec!["rowan", "scribe"], "members table in creation order, handles not ids");
    assert_eq!(sc.members.get("rowan").unwrap().name, "Rowan Vale");
    assert_eq!(sc.members.get("rowan").unwrap().id, ts.owner_id);
    assert!(sc.actions.inherited);
    assert_eq!(sc.actions.available.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["approve", "send-back"]);
    assert!(sc.assets.is_empty() && sc.threads.is_empty());
    // an older base_version
    let (_, text) = get_bytes(&ts, &tok, &format!("/api/v1/docs/{id}/sidecar?version=1"));
    let sc1 = annotary_sidecar::parse(std::str::from_utf8(&text).unwrap()).unwrap();
    assert_eq!(sc1.base_version, 1);
    assert_eq!(sc1.versions.len(), 2, "still the full projection");
    let (status, _) = get_bytes(&ts, &tok, &format!("/api/v1/docs/{id}/sidecar?version=7"));
    assert_eq!(status, 404);
    // status
    let (status, st) = ts.get(&format!("/api/v1/docs/{id}/status"), Some(&tok));
    assert_eq!(status, 200);
    assert_eq!(st["name"], "phase1-plan");
    assert_eq!(st["version"], 2);
    assert!(st["action"].is_null());
    assert_eq!(st["open_threads"], 0);
    assert_eq!(st["mentions_me"], 0);
    assert_eq!(st["seq"], ts.store.last_seq().unwrap());
}

// spec: S02-17, S02-18, S02-27
#[test]
fn delete_restore_purge_over_http_with_events() {
    // Locked: a loopback sign-in would add an event to the exact list below (CMD-29).
    let ts = TestServer::start_locked();
    let tok = ts.owner_token.clone();
    let doc = create(&ts, &tok, "plan", b"shared\n", None);
    let id = doc["id"].as_str().unwrap().to_string();
    let other = create(&ts, &tok, "other", b"shared\n", None);
    let seq = ts.store.last_seq().unwrap();

    let (status, body) = delete(&ts, &tok, &id);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["state"], "deleted");
    assert!(!body["deleted_at"].is_null());
    let (status, body) = delete(&ts, &tok, &id);
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "deleted");
    // pushing to a deleted Document is refused
    let (status, body) = push(&ts, &tok, &id, "plan", b"x\n", json!({}));
    assert_eq!(status, 409, "{body}");
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/restore"), Some(&tok), &json!({}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["state"], "restored");
    assert!(body["deleted_at"].is_null());
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/restore"), Some(&tok), &json!({}));
    assert_eq!(status, 409, "{body}");
    // purge needs deleted
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/purge"), Some(&tok), &json!({}));
    assert_eq!(status, 409, "{body}");
    delete(&ts, &tok, &id);
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/purge"), Some(&tok), &json!({}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["state"], "purged");
    assert_eq!(body["blobs_removed"], 0, "the Markdown blob is shared with `other`");
    assert!(ts.store.has_blob(&sha256_hex(b"shared\n")));
    let (status, _) = ts.get(&format!("/api/v1/docs/{id}"), Some(&tok));
    assert_eq!(status, 404);
    let (status, _) = ts.get(&format!("/api/v1/docs/{}", other["id"].as_str().unwrap()), Some(&tok));
    assert_eq!(status, 200);
    let kinds: Vec<String> = ts.store.events_since(seq).unwrap().into_iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec!["document.deleted", "document.restored", "document.deleted", "document.purged"]);
}
