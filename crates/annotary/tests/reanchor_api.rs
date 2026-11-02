//! Re-anchoring over HTTP against the in-process server (ticket 07): the whole fixture table
//! pushed through the API with edge-computed placements (the server's spot-verification must
//! agree with every one, and its own placements — nothing submitted — must be identical),
//! incremental mapping across a Version the pusher never saw, stale/wrong submissions, asset
//! Threads by content hash, and the `version=` time-slice / `anchor=` filter of the Thread list.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use annotary::testing::TestServer;
use annotary_sidecar::Anchor;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Reanchored, compute_placements};
use serde::Deserialize;
use serde_json::{Value, json};

fn upload(ts: &TestServer, token: &str, bytes: &[u8]) -> String {
    let hash = sha256_hex(bytes);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(bytes)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

fn create(ts: &TestServer, name: &str, md: &str, manifest: &Value) -> String {
    let hash = upload(ts, &ts.owner_token, md.as_bytes());
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&ts.owner_token),
        &json!({ "vault": "notes", "name": name, "md_hash": hash, "manifest": manifest }),
    );
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

/// Push `md` (+ manifest) with optional placements; returns `(status, body)`.
fn push(ts: &TestServer, id: &str, md: &str, manifest: &Value, placements: Option<(&[Value], u64)>) -> (u16, Value) {
    let hash = upload(ts, &ts.owner_token, md.as_bytes());
    let mut body = json!({ "name": "plan", "md_hash": hash, "manifest": manifest });
    if let Some((p, from)) = placements {
        body["placements"] = json!(p);
        body["placements_from"] = json!(from);
    }
    ts.post(&format!("/api/v1/docs/{id}/versions"), Some(&ts.owner_token), &body)
}

fn threads_all(ts: &TestServer, id: &str) -> Vec<(String, Option<Anchor>)> {
    let (status, body) = ts.get(&format!("/api/v1/docs/{id}/threads?status=all&hidden=1"), Some(&ts.owner_token));
    assert_eq!(status, 200, "{body}");
    let mut out: Vec<(String, Option<Anchor>)> = body["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let anchor = t.get("anchor").filter(|a| !a.is_null()).map(|a| serde_json::from_value(a.clone()).unwrap());
            (t["id"].as_str().unwrap().to_string(), anchor)
        })
        .collect();
    out.sort_by_key(|(id, _)| id[1..].parse::<u32>().unwrap());
    out
}

fn head_text(ts: &TestServer, id: &str) -> (u64, String) {
    let (_, doc) = ts.get(&format!("/api/v1/docs/{id}"), Some(&ts.owner_token));
    let md = ts.store.read_blob(doc["md_hash"].as_str().unwrap()).unwrap().unwrap();
    (doc["version"].as_u64().unwrap(), String::from_utf8(md).unwrap())
}

/// What the CLI does before a push: map every Thread from the head onto `new_md`.
fn edge_placements(ts: &TestServer, id: &str, new_md: &str, manifest: &Value) -> (Vec<Value>, u64) {
    let (head, text_n) = head_text(ts, id);
    let threads = threads_all(ts, id);
    let lines = annotary::assets::scan::reference_lines(new_md);
    let ps = compute_placements(&threads, &text_n, new_md, manifest, &lines).unwrap();
    (ps.iter().map(|p| serde_json::to_value(p).unwrap()).collect(), head)
}

// ---------------------------------------------------------------- the fixture table over HTTP

#[derive(Debug, Deserialize)]
struct ThreadSpec {
    id: String,
    subject: String,
    version: u32,
    #[serde(default)]
    range: Option<String>,
    #[serde(rename = "ref", default)]
    reference: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Expected {
    version: u32,
    status: String,
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    exact: Option<String>,
    #[serde(default)]
    line: Option<u32>,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/anchoring").canonicalize().unwrap()
}

/// Real blobs for a fixture's placeholder hashes (the manifest validator wants them stored).
fn real_manifest(ts: &TestServer, fixture: &Value) -> Value {
    let entries: Vec<Value> = fixture
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let placeholder = e["hash"].as_str().unwrap();
            let bytes = format!("asset bytes for {placeholder}");
            let hash = upload(ts, &ts.owner_token, bytes.as_bytes());
            json!({ "ref": e["ref"], "hash": hash, "size": bytes.len(), "type": "image/png" })
        })
        .collect();
    Value::Array(entries)
}

/// Replay one fixture directory: `submit` = send edge-computed placements (else let the server
/// place everything). Returns every Thread's final Anchor.
fn replay(ts: &TestServer, dir: &Path, submit: bool) -> Vec<(String, Option<Anchor>)> {
    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    let mut versions: Vec<String> = Vec::new();
    for n in 1.. {
        let p = dir.join(format!("v{n}.md"));
        if !p.exists() {
            break;
        }
        versions.push(std::fs::read_to_string(&p).unwrap());
    }
    let manifest_at = |n: usize| -> Value {
        let p = dir.join(format!("v{n}.manifest.json"));
        if p.exists() {
            real_manifest(ts, &serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap())
        } else {
            json!([])
        }
    };
    let threads: Vec<ThreadSpec> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("threads.json")).unwrap()).unwrap();
    let id = create(ts, &format!("{name}-{}", if submit { "edge" } else { "server" }), &versions[0], &manifest_at(1));
    for (i, md) in versions.iter().enumerate() {
        let n = i as u32 + 1;
        // Threads created on this Version, in file order (their ids must come out t1, t2, …).
        for t in threads.iter().filter(|t| t.version == n) {
            let body = match t.subject.as_str() {
                "span" => json!({ "subject": "span", "range": t.range, "body": format!("about {}", t.id) }),
                "document" => json!({ "subject": "document", "body": format!("about {}", t.id) }),
                "asset" => json!({ "subject": "asset", "ref": t.reference, "body": format!("about {}", t.id) }),
                other => panic!("{other}"),
            };
            let (status, v) = ts.post(&format!("/api/v1/docs/{id}/threads"), Some(&ts.owner_token), &body);
            assert_eq!(status, 201, "{name}/{}: {v}", t.id);
        }
        if let Some(next) = versions.get(i + 1) {
            let manifest = manifest_at(i + 2);
            let placements = submit.then(|| edge_placements(ts, &id, next, &manifest));
            let (status, v) =
                push(ts, &id, next, &manifest, placements.as_ref().map(|(p, from)| (p.as_slice(), *from)));
            if status == 200 {
                // Identical content (G13's rename, D12's reload): `already` — no Version, nothing
                // moves; the sparse rule answers for the missing number.
                assert_eq!(v["already"], true, "{name} v{}: {v}", n + 1);
                continue;
            }
            assert_eq!(status, 201, "{name} v{}: {v}", n + 1);
            let r: Reanchored = serde_json::from_value(v["reanchored"].clone()).unwrap();
            assert_eq!(r.total as usize, threads.iter().filter(|t| t.version <= n).count(), "{name} v{}", n + 1);
            if submit {
                assert_eq!(r.computed, 0, "{name} v{}: everything was submitted", n + 1);
                assert_eq!(
                    r.verified as usize,
                    threads.iter().filter(|t| t.version <= n && t.subject != "document").count()
                );
            } else {
                assert_eq!(r.verified, 0);
            }
        }
        let _ = md;
    }
    let mut out = threads_all(ts, &id);
    // Fixture ids are A, B, t1 …; the server allocates t1, t2, … in creation order.
    let mut ordered: Vec<&ThreadSpec> = threads.iter().collect();
    ordered.sort_by_key(|t| t.version);
    for ((server_id, _), spec) in out.iter_mut().zip(ordered.iter()) {
        *server_id = spec.id.clone();
    }
    out
}

// spec: S07-04, S07-07, S07-09, S07-16
#[test]
fn the_fixture_table_pushed_through_the_server_matches_edge_placements_and_expectations() {
    let ts = TestServer::start();
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(fixtures_dir()).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    assert!(dirs.len() >= 40);
    for dir in &dirs {
        check_against_expected(&ts, dir);
    }
}

// spec: S32-02
#[test]
fn the_corpus_histories_pushed_through_the_server_match_edge_placements_and_expectations() {
    let ts = TestServer::start();
    let corpus = fixtures_dir().join("../corpus").canonicalize().unwrap();
    for name in ["glossary-multilingual", "onboarding", "rfc-tile-cache"] {
        check_against_expected(&ts, &corpus.join(name));
    }
}

fn check_against_expected(ts: &TestServer, dir: &Path) {
    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    let expected: BTreeMap<String, Vec<Expected>> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    // The server verified every edge placement (201s above); its own placements are the same.
    let edge = replay(ts, dir, true);
    let server = replay(ts, dir, false);
    assert_eq!(edge, server, "{name}: server-computed placements equal the edge's");
    // …and both reproduce the declared history at every Version (assets: line included).
    for (id, anchor) in &edge {
        let want = &expected[id];
        match anchor {
            None => assert!(want.iter().all(|e| e.status == "anchored"), "{name}/{id}"),
            Some(a) => {
                for e in want {
                    let p = a.placement_at(e.version).unwrap_or_else(|| panic!("{name}/{id} v{}", e.version));
                    assert_eq!(p.status.as_deref(), Some(e.status.as_str()), "{name}/{id} v{}", e.version);
                    if e.status != "orphaned" {
                        if a.is_asset() {
                            assert_eq!(p.line, e.line, "{name}/{id} v{}: asset line", e.version);
                        } else {
                            assert_eq!(p.range.as_deref(), e.range.as_deref(), "{name}/{id} v{}", e.version);
                        }
                    }
                    // The last Version's exact text is what `current` carries when refined.
                    if e.version == want.last().unwrap().version && e.status == "refined" {
                        assert_eq!(a.current.exact, e.exact, "{name}/{id}: current.exact");
                    }
                }
            }
        }
    }
}

// spec: S07-06, S07-07, S07-16
#[test]
fn mapping_is_incremental_and_submissions_are_checked() {
    let ts = TestServer::start();
    let v1 = "# Plan\n\nWe ship the CLI before the web UI so agents can start.\n\nDogfood for two weeks, then invite three external developers.\n";
    let v2 =
        "# Plan\n\nWe ship the CLI before the web UI so agents can start.\n\nRewritten completely: something else.\n";
    let v3 = "# Plan\n\nIntro.\n\nWe ship the CLI before the web UI so agents can start.\n\nDogfood for two weeks, then invite three external developers.\n";
    let empty = json!([]);
    let id = create(&ts, "plan", v1, &empty);
    let (st, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "range": "5:28-5:60", "body": "three?" }),
    );
    assert_eq!(st, 201);
    let (st, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&ts.owner_token),
        &json!({ "subject": "span", "range": "3:0-3:33", "body": "cli first" }),
    );
    assert_eq!(st, 201);
    // Someone else pushes v2 (no placements: the server places) — t1 is orphaned there.
    let (st, v) = push(&ts, &id, v2, &empty, None);
    assert_eq!(st, 201, "{v}");
    assert_eq!(v["reanchored"]["orphaned"], 1);
    assert_eq!(v["reanchored"]["computed"], 2);
    // A pusher who pulled v1 pushes v3, which restores the quote: mapping from the head (v2)
    // keeps t1 orphaned (sticky) — mapping v1 → v3 directly would have revived it.
    let (ps, from) = edge_placements(&ts, &id, v3, &empty);
    assert_eq!(from, 2);
    let t1 = ps.iter().find(|p| p["thread"] == "t1").unwrap();
    assert_eq!(t1["status"], "orphaned");
    let direct = compute_placements(
        &[("t1".into(), Some(Anchor::new_span(1, "5:28-5:60", "invite three external developers")))],
        v1,
        v3,
        &empty,
        &[],
    )
    .unwrap();
    assert_eq!(direct[0].status, "anchored", "the non-incremental answer differs — which is why the head is the base");
    let (st, v) = push(&ts, &id, v3, &empty, Some((&ps, from)));
    assert_eq!(st, 201, "{v}");
    let r: Reanchored = serde_json::from_value(v["reanchored"].clone()).unwrap();
    assert_eq!((r.total, r.anchored, r.orphaned, r.verified, r.computed), (2, 1, 1, 2, 0));
    let anchors = threads_all(&ts, &id);
    assert!(anchors[0].1.as_ref().unwrap().is_orphaned(), "orphaned is sticky across the restore");
    assert_eq!(anchors[1].1.as_ref().unwrap().current.range.as_deref(), Some("5:0-5:33"));

    // A wrong placement is refused as 400 invalid_placements — nothing committed.
    let v4 = format!("{v3}\nMore.\n");
    let wrong = vec![json!({ "thread": "t2", "status": "orphaned" })];
    let (st, e) = push(&ts, &id, &v4, &empty, Some((&wrong, 3)));
    assert_eq!(st, 400, "{e}");
    assert_eq!(e["error"], "invalid_placements");
    assert!(e["message"].as_str().unwrap().contains("t2: the server computes"), "{e}");
    let (_, doc) = ts.get(&format!("/api/v1/docs/{id}"), Some(&ts.owner_token));
    assert_eq!(doc["version"], 3);
    // The same wrong placement from a stale head is ignored (the server places), not refused.
    let (st, v) = push(&ts, &id, &v4, &empty, Some((&wrong, 1)));
    assert_eq!(st, 201, "{v}");
    assert_eq!((v["reanchored"]["verified"].as_u64(), v["reanchored"]["computed"].as_u64()), (Some(0), Some(2)));
    assert!(!threads_all(&ts, &id)[1].1.as_ref().unwrap().is_orphaned());
    // Malformed shapes are 400 too; an unknown Thread as well.
    for bad in [
        json!({ "thread": "t2", "status": "sideways" }),
        json!({ "thread": "t9", "status": "anchored" }),
        json!({ "thread": "t2", "status": "anchored", "range": "1:0-1:2" }),
    ] {
        let (st, e) = push(&ts, &id, &format!("{v4}x\n"), &empty, Some((std::slice::from_ref(&bad), 4)));
        assert_eq!(st, 400, "{bad}: {e}");
        assert_eq!(e["error"], "invalid_placements");
    }
    // Verification off: a plausible submission is stored as sent.
    ts.store.set_verify_edge(false);
    let (st, v) = push(&ts, &id, &format!("{v4}x\n"), &empty, Some((&wrong, 4)));
    assert_eq!(st, 201, "{v}");
    assert!(threads_all(&ts, &id)[1].1.as_ref().unwrap().is_orphaned(), "trusted");
    ts.store.set_verify_edge(true);
    // Rights: placements ride on the commit, so `write` is what it takes (a comment-only Member
    // cannot push at all).
    let (_, limited) = ts.add_member("limited");
    ts.store
        .replace_grants(
            &ts.owner_id,
            &ts.store.list_members().unwrap().iter().find(|m| m.handle == "limited").unwrap().id,
            vec![annotary_store::Grant { vault: "notes".into(), rights: vec![annotary_store::Right::Comment] }],
        )
        .unwrap();
    let hash = upload(&ts, &ts.owner_token, b"# x\n");
    let (st, _) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&limited),
        &json!({ "name": "plan", "md_hash": hash, "placements": [] }),
    );
    assert_eq!(st, 403);
}

// spec: S07-09, S07-10
#[test]
fn asset_threads_follow_the_content_hash() {
    let ts = TestServer::start();
    let png = upload(&ts, &ts.owner_token, b"png bytes v1");
    let png2 = upload(&ts, &ts.owner_token, b"png bytes v2");
    let m1 = json!([{ "ref": "img/a.png", "hash": png, "size": 12, "type": "image/png" }, { "ref": "img/gone.png", "type": "image/png", "missing": true }]);
    let v1 = "# Doc\n\n![a](img/a.png)\n\n![gone](img/gone.png)\n\ntext\n";
    let id = create(&ts, "assets", v1, &m1);
    let post = |body: Value| ts.post(&format!("/api/v1/docs/{id}/threads"), Some(&ts.owner_token), &body);
    let (st, t) = post(json!({ "subject": "asset", "ref": "img/a.png", "body": "diagram" }));
    assert_eq!(st, 201, "{t}");
    assert_eq!(t["anchor"]["original"]["hash"], png);
    assert_eq!(t["anchor"]["original"]["ref"], "img/a.png");
    assert_eq!(t["anchor"]["original"]["line"], 3);
    assert_eq!(t["anchor"]["current"]["status"], "anchored");
    let (st, e) = post(json!({ "subject": "asset", "ref": "img/nope.png", "body": "x" }));
    assert_eq!(st, 409, "{e}");
    assert_eq!(e["error"], "asset_not_on_head");
    assert!(e["message"].as_str().unwrap().contains("img/a.png"), "lists the refs: {e}");
    let (st, e) = post(json!({ "subject": "asset", "ref": "img/gone.png", "body": "x" }));
    assert_eq!(st, 409, "{e}");
    assert_eq!(e["error"], "asset_missing");
    let (st, e) = post(json!({ "subject": "asset", "body": "x" }));
    assert_eq!(st, 400, "{e}");
    // v2: same bytes referenced from another line under another name ⇒ anchored, line updated.
    let m2 = json!([{ "ref": "pics/arch.png", "hash": png, "size": 12, "type": "image/png" }]);
    let v2 = "# Doc\n\ntext\n\nmore\n\n![arch](pics/arch.png)\n";
    let (ps, from) = edge_placements(&ts, &id, v2, &m2);
    assert_eq!(ps, vec![json!({ "thread": "t1", "status": "anchored", "line": 7 })]);
    let (st, v) = push(&ts, &id, v2, &m2, Some((&ps, from)));
    assert_eq!(st, 201, "{v}");
    let a = threads_all(&ts, &id)[0].1.clone().unwrap();
    assert_eq!(a.current.line, Some(7));
    assert_eq!(a.current.status.as_deref(), Some("anchored"));
    assert_eq!(a.original.reference.as_deref(), Some("img/a.png"), "the label keeps the original ref");
    // v3: different bytes at the old ref ⇒ orphaned; v4 with the old bytes back ⇒ still orphaned.
    let m3 = json!([{ "ref": "img/a.png", "hash": png2, "size": 12, "type": "image/png" }]);
    let (st, v) = push(&ts, &id, v1, &m3, None);
    assert_eq!(st, 201, "{v}");
    assert_eq!(v["reanchored"]["orphaned"], 1);
    let a = threads_all(&ts, &id)[0].1.clone().unwrap();
    assert!(a.is_orphaned());
    assert_eq!(a.current.line, Some(7), "last valid line, stamped v2");
    assert_eq!(a.current.version, 2);
    let (st, _) = push(&ts, &id, format!("{v1}\n").as_str(), &m1, None);
    assert_eq!(st, 201);
    assert!(threads_all(&ts, &id)[0].1.as_ref().unwrap().is_orphaned(), "sticky");
    // Orphaned ⊥ resolved: it is still open, replyable, resolvable.
    let (st, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads/t1/replies"),
        Some(&ts.owner_token),
        &json!({ "body": "still here", "resolve": true }),
    );
    assert_eq!(st, 201);
    let (_, list) = ts.get(&format!("/api/v1/docs/{id}/threads?status=resolved"), Some(&ts.owner_token));
    assert_eq!(list["threads"][0]["anchor"]["current"]["status"], "orphaned");
    assert_eq!(list["threads"][0]["resolved"]["version"], 4);
}

// spec: S07-12, S07-13
#[test]
fn time_slice_and_anchor_filter_over_http() {
    let ts = TestServer::start();
    let empty = json!([]);
    let v1 = "# Plan\n\nalpha beta gamma\n\ndelta epsilon zeta\n";
    let v2 = "# Plan\n\nIntro line.\n\nalpha beta gamma\n\ndelta EPSILON zeta\n";
    let v3 = "# Plan\n\nIntro line.\n\nalpha beta gamma\n\nrewritten entirely\n";
    let id = create(&ts, "slice", v1, &empty);
    let post = |body: Value| ts.post(&format!("/api/v1/docs/{id}/threads"), Some(&ts.owner_token), &body);
    assert_eq!(post(json!({ "subject": "span", "range": "5:0-5:13", "body": "delta epsilon" })).0, 201); // t1
    assert_eq!(post(json!({ "subject": "document", "body": "whole" })).0, 201); // t2
    assert_eq!(post(json!({ "subject": "span", "range": "3:0-3:5", "body": "alpha" })).0, 201); // t3
    assert_eq!(push(&ts, &id, v2, &empty, None).0, 201);
    let (st, _) = ts.post(&format!("/api/v1/docs/{id}/threads/t1/resolve"), Some(&ts.owner_token), &json!({}));
    assert_eq!(st, 200);
    assert_eq!(post(json!({ "subject": "span", "range": "3:0-3:5", "body": "intro" })).0, 201); // t4 on v2
    assert_eq!(push(&ts, &id, v3, &empty, None).0, 201);
    let (st, _) = ts.post(&format!("/api/v1/docs/{id}/threads/t1/reopen"), Some(&ts.owner_token), &json!({}));
    assert_eq!(st, 200);
    let list = |q: &str| {
        let (st, b) = ts.get(&format!("/api/v1/docs/{id}/threads?{q}"), Some(&ts.owner_token));
        (st, b)
    };
    let ids = |b: &Value| {
        b["threads"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    // Live: t1 orphaned (last), t2 first, then t3/t4 by position (t4 line 3, t3 line 5).
    let (st, b) = list("");
    assert_eq!(st, 200);
    assert_eq!(ids(&b), vec!["t2", "t4", "t3", "t1"]);
    assert_eq!(b["threads"][3]["anchor"]["current"]["status"], "orphaned");
    assert_eq!(b["threads"][3]["anchor"]["current"]["version"], 2, "last valid Version");
    assert_eq!(b["version"], 3);
    // As of v1: t1 anchored + open, t3; t4 does not exist yet.
    let (st, b) = list("version=1&status=all");
    assert_eq!(st, 200, "{b}");
    assert_eq!(ids(&b), vec!["t2", "t3", "t1"]);
    assert_eq!(b["threads"][2]["anchor"]["current"]["range"], "5:0-5:13");
    assert_eq!(b["threads"][2]["anchor"]["current"]["version"], 1);
    assert!(b["threads"][2]["resolved"].is_null(), "resolved only at v2");
    assert_eq!(b["version"], 1);
    assert_eq!(b["head"], 3);
    // As of v2: t1 refined ("EPSILON"), resolved; t4 present.
    let (st, b) = list("version=2&status=all");
    assert_eq!(st, 200, "{b}");
    assert_eq!(ids(&b), vec!["t2", "t4", "t3", "t1"]);
    let t1 = &b["threads"][3];
    assert_eq!(t1["anchor"]["current"]["status"], "refined");
    // "epsilon" was retyped at the span's end: the span shrinks to what survived (edge
    // replacements fall outside, like D4/D7).
    assert_eq!(t1["anchor"]["current"]["exact"], "delta");
    assert_eq!(t1["anchor"]["current"]["range"], "7:0-7:5");
    assert_eq!(t1["resolved"]["version"], 2);
    let (st, b) = list("version=2");
    assert_eq!(st, 200);
    assert_eq!(ids(&b), vec!["t2", "t4", "t3"], "open only at v2");
    // Placement-status filter, live and sliced; Document-level counts as anchored.
    let (_, b) = list("anchor=orphaned");
    assert_eq!(ids(&b), vec!["t1"]);
    let (_, b) = list("anchor=anchored,refined");
    assert_eq!(ids(&b), vec!["t2", "t4", "t3"]);
    let (_, b) = list("anchor=refined&version=2&status=all");
    assert_eq!(ids(&b), vec!["t1"]);
    // Errors: unknown Version ⇒ 404, bad status name ⇒ 400.
    let (st, e) = list("version=9");
    assert_eq!(st, 404, "{e}");
    let (st, e) = list("anchor=moved");
    assert_eq!(st, 400, "{e}");
}
