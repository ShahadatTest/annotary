//! Housekeeping (ticket 13) through the API and the real binary: the auto-purge setting
//! (owner-only, evented), one tick on demand (`POST /api/v1/housekeeping`) = auto-purge →
//! blob GC → derived-blob cleanup, the GC after an explicit purge, and the in-process ticker of
//! a real `serve` firing on its (shortened) interval.

mod common;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use common::serve::ServeGuard;
use serde_json::{Value, json};

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

fn put_json(ts: &TestServer, token: Option<&str>, path: &str, body: &Value) -> (u16, Value) {
    let mut req = ureq::put(format!("{}{}", ts.url, path));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    match req.config().http_status_as_error(false).build().send_json(body) {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
        Err(e) => (0, Value::String(e.to_string())),
    }
}

fn delete(ts: &TestServer, token: &str, path: &str) -> (u16, Value) {
    let mut r = ureq::delete(format!("{}{}", ts.url, path))
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

fn upload(ts: &TestServer, token: &str, bytes: &[u8]) -> String {
    let hash = sha256_hex(bytes);
    let (status, body) = put_bytes(ts, token, &format!("/api/v1/blobs/{hash}"), bytes);
    assert!(status == 201 || status == 200, "{status} {body}");
    hash
}

/// Create a Document whose v1 is `md` with `manifest` (entries may name uploaded Asset blobs).
fn create(ts: &TestServer, token: &str, name: &str, md: &[u8], manifest: Value) -> String {
    let hash = upload(ts, token, md);
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(token),
        &json!({ "vault": "notes", "name": name, "md_hash": hash, "manifest": manifest }),
    );
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn backdate_deleted(ts: &TestServer, id: &str, days: i64) {
    let conn = rusqlite::Connection::open(ts.data_dir.join("store/annotary.sqlite")).unwrap();
    let at = jiff::Timestamp::now().checked_sub(jiff::SignedDuration::from_secs(days * 86_400)).unwrap();
    let at = annotary_store::time::format(at);
    conn.execute("UPDATE documents SET deleted_at = ?1 WHERE id = ?2", rusqlite::params![at, id]).unwrap();
}

fn age_file(path: &std::path::Path, secs_ago: u64) {
    let t = std::time::SystemTime::now() - Duration::from_secs(secs_ago);
    std::fs::File::open(path).unwrap().set_modified(t).unwrap();
}

fn tick(ts: &TestServer, token: &str) -> Value {
    let (status, rep) = ts.post("/api/v1/housekeeping", Some(token), &json!({}));
    assert_eq!(status, 200, "{rep}");
    rep
}

// spec: S13-07
#[test]
fn auto_purge_setting_is_owner_only_validated_and_reported_by_settings() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, member) = ts.add_member("linter");
    // Off by default.
    let (status, s) = ts.get("/api/v1/settings", Some(&owner));
    assert_eq!(status, 200, "{s}");
    assert!(s["auto_purge_days"].is_null(), "{s}");
    // Owner-only: a Member gets 403, no credential 401.
    let (status, b) = put_json(&ts, Some(&member), "/api/v1/settings/auto-purge", &json!({ "days": 30 }));
    assert_eq!(status, 403, "{b}");
    assert!(b["message"].as_str().unwrap().contains("owner-only"));
    let (status, _) = put_json(&ts, None, "/api/v1/settings/auto-purge", &json!({ "days": 30 }));
    assert_eq!(status, 401);
    let (status, b) = ts.post("/api/v1/housekeeping", Some(&member), &json!({}));
    assert_eq!(status, 403, "{b}");
    let (status, _) = ts.post("/api/v1/housekeeping", None, &json!({}));
    assert_eq!(status, 401);
    // On, reported, idempotent, off again; invalid values are 400.
    let seq = ts.store.last_seq().unwrap();
    let (status, b) = put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": 30 }));
    assert_eq!(status, 200, "{b}");
    assert_eq!(b, json!({ "auto_purge_days": 30, "changed": true }));
    let (_, s) = ts.get("/api/v1/settings", Some(&owner));
    assert_eq!(s["auto_purge_days"], 30);
    let (_, b) = put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": 30 }));
    assert_eq!(b["changed"], false);
    let (status, b) = put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": 99999 }));
    assert_eq!(status, 400, "{b}");
    assert_eq!(b["error"], "invalid");
    let (status, b) = put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": "soon" }));
    assert_eq!(status, 400, "{b}");
    let (status, b) = put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": null }));
    assert_eq!(status, 200, "{b}");
    assert_eq!(b, json!({ "auto_purge_days": null, "changed": true }));
    let (status, b) = put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": 0 }));
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["changed"], false, "0 = off, already off");
    let ev = ts.store.events_since(seq).unwrap();
    assert_eq!(ev.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["setting.auto_purge"; 2]);
    assert_eq!(ev[0].by.as_deref(), Some(ts.owner_id.as_str()));
}

// spec: S13-08, S13-09, S13-10
#[test]
fn a_tick_auto_purges_per_the_setting_and_gc_keeps_every_referenced_blob() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    // A shared Asset blob referenced by two Documents, an Asset only `old` references, and a
    // derivative.
    let shared = upload(&ts, &owner, b"PNG shared bytes");
    let only_old = upload(&ts, &owner, b"PNG only old");
    let deriv = upload(&ts, &owner, b"PNG derivative of old");
    let old = create(
        &ts,
        &owner,
        "old",
        b"# old\n\n![a](a.png) ![b](b.png)\n",
        json!([
            { "ref": "a.png", "hash": shared, "size": 16, "type": "image/png" },
            { "ref": "b.png", "hash": only_old, "size": 12, "type": "image/png", "derivative": deriv },
        ]),
    );
    let keeper = create(
        &ts,
        &owner,
        "keeper",
        b"# keeper\n\n![a](a.png)\n",
        json!([{ "ref": "a.png", "hash": shared, "size": 16, "type": "image/png" }]),
    );
    let young = create(&ts, &owner, "young", b"# young\n", json!([]));
    let live = create(&ts, &owner, "live", b"# live\n", json!([]));
    // An uploaded-but-never-committed blob: young (survives the grace) and old (goes).
    let stray_young = upload(&ts, &owner, b"stray young");
    let stray_old = upload(&ts, &owner, b"stray old");
    age_file(&ts.store.blob_path(&stray_old), 2 * 3600);
    let every_referenced = [
        sha256_hex(b"# old\n\n![a](a.png) ![b](b.png)\n"),
        shared.clone(),
        only_old.clone(),
        deriv.clone(),
        sha256_hex(b"# keeper\n\n![a](a.png)\n"),
        sha256_hex(b"# young\n"),
        sha256_hex(b"# live\n"),
    ];
    for h in &every_referenced {
        age_file(&ts.store.blob_path(h), 2 * 3600); // old enough to sweep — if they were unreferenced
    }
    for id in [&old, &young] {
        let (status, _) = delete(&ts, &owner, &format!("/api/v1/docs/{id}"));
        assert_eq!(status, 200);
    }
    backdate_deleted(&ts, &old, 40);

    // Setting off: the tick purges nothing; the GC still sweeps the old stray, keeps everything
    // referenced and the young stray.
    let rep = tick(&ts, &owner);
    assert_eq!(rep["purged"], json!([]));
    assert_eq!(rep["gc_removed"], 1, "{rep}");
    assert!(rep["gc_bytes"].as_u64().unwrap() > 0);
    assert!(!ts.store.has_blob(&stray_old));
    assert!(ts.store.has_blob(&stray_young));
    for h in &every_referenced {
        assert!(ts.store.has_blob(h), "referenced blob {h} must survive GC");
    }
    // N = 60 days: `old` (40 d) is not due.
    put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": 60 }));
    let rep = tick(&ts, &owner);
    assert_eq!(rep["purged"], json!([]));
    let (status, _) = ts.get(&format!("/api/v1/docs/{old}"), Some(&owner));
    assert_eq!(status, 200, "still there (deleted)");
    // N = 30 days: `old` goes; `young` (deleted just now) and the live Documents stay; the blob
    // shared with `keeper` stays; `old`'s own Asset, derivative and Markdown go.
    put_json(&ts, Some(&owner), "/api/v1/settings/auto-purge", &json!({ "days": 30 }));
    let seq = ts.store.last_seq().unwrap();
    let rep = tick(&ts, &owner);
    assert_eq!(rep["purged"].as_array().unwrap().len(), 1, "{rep}");
    assert_eq!(rep["purged"][0]["id"], old);
    assert_eq!(rep["purged"][0]["name"], "old");
    assert_eq!(rep["purged"][0]["blobs_removed"], 3);
    let (status, _) = ts.get(&format!("/api/v1/docs/{old}"), Some(&owner));
    assert_eq!(status, 404, "purged");
    assert!(ts.store.has_blob(&shared), "shared with keeper");
    assert!(!ts.store.has_blob(&only_old));
    assert!(!ts.store.has_blob(&deriv));
    assert!(!ts.store.has_blob(&sha256_hex(b"# old\n\n![a](a.png) ![b](b.png)\n")));
    for id in [&keeper, &young, &live] {
        let (status, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&owner));
        assert_eq!(status, 200, "{d}");
    }
    assert!(ts.store.has_blob(&sha256_hex(b"# young\n")));
    let ev = ts.store.events_since(seq).unwrap();
    let purged: Vec<_> = ev.iter().filter(|e| e.kind == "document.purged").collect();
    assert_eq!(purged.len(), 1);
    assert_eq!(purged[0].by, None, "auto-purge has no actor");
    assert_eq!(purged[0].payload["auto"], true);
    assert_eq!(purged[0].payload["name"], "old");
    // The events endpoint shows it to the owner like any purge.
    let (_, evs) = ts.get(&format!("/api/v1/events?since={seq}"), Some(&owner));
    assert!(evs["events"].as_array().unwrap().iter().any(|e| e["type"] == "document.purged" && e["by"].is_null()));
    // A second tick is a no-op.
    let rep = tick(&ts, &owner);
    assert_eq!(rep["purged"], json!([]));
    assert_eq!(rep["gc_removed"], 0);
}

// spec: S13-10
#[test]
fn an_explicit_purge_runs_the_gc_too() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let doc = create(&ts, &owner, "gone", b"# gone\n", json!([]));
    let stray_old = upload(&ts, &owner, b"stray for purge");
    age_file(&ts.store.blob_path(&stray_old), 2 * 3600);
    let (status, _) = delete(&ts, &owner, &format!("/api/v1/docs/{doc}"));
    assert_eq!(status, 200);
    let (status, p) = ts.post(&format!("/api/v1/docs/{doc}/purge"), Some(&owner), &json!({}));
    assert_eq!(status, 200, "{p}");
    assert_eq!(p["state"], "purged");
    assert_eq!(p["blobs_removed"], 1, "the purge's own count: the Markdown");
    assert_eq!(p["gc_removed"], 1, "the stray swept by the GC that follows");
    assert!(!ts.store.has_blob(&stray_old));
    assert!(!ts.store.has_blob(&sha256_hex(b"# gone\n")));
}

// spec: S13-11
#[test]
fn a_tick_cleans_stale_renders_and_keeps_current_ones() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let html_old = upload(&ts, &owner, b"<h1 data-l=\"1-1\">old render</h1>\n");
    let html_new = upload(&ts, &owner, b"<h1 data-l=\"1-1\">new render</h1>\n");
    let html_shared = upload(&ts, &owner, b"<h1 data-l=\"1-1\">shared render</h1>\n");
    let md = upload(&ts, &owner, b"# r\n");
    let mk = |name: &str, html: &str, renderer: &str| -> String {
        let (status, d) = ts.post(
            "/api/v1/docs",
            Some(&owner),
            &json!({ "vault": "notes", "name": name, "md_hash": md, "html_hash": html, "renderer": renderer }),
        );
        assert_eq!(status, 201, "{d}");
        d["id"].as_str().unwrap().to_string()
    };
    let stale = mk("stale", &html_old, "0.0.1");
    let current = mk("current", &html_new, annotary_renderer::RENDERER_VERSION);
    let stale_shared = mk("stale-shared", &html_shared, "0.0.1");
    let current_shared = mk("current-shared", &html_shared, annotary_renderer::RENDERER_VERSION);
    // A leftover temp file from an interrupted write, old enough to sweep.
    let tmp = ts.store.blob_path(&html_old).parent().unwrap().join(".abc.9.tmp");
    std::fs::write(&tmp, b"partial").unwrap();
    age_file(&tmp, 2 * 3600);
    let seq = ts.store.last_seq().unwrap();
    let rep = tick(&ts, &owner);
    assert_eq!(rep["html_dropped"], 2, "{rep}");
    assert_eq!(rep["derived_removed"], 1, "only the unshared stale blob: {rep}");
    assert_eq!(rep["temp_removed"], 1);
    assert!(!tmp.exists());
    assert!(!ts.store.has_blob(&html_old));
    assert!(ts.store.has_blob(&html_new));
    assert!(ts.store.has_blob(&html_shared), "still referenced by the current render");
    let v = |id: &str| ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&owner)).1;
    assert!(v(&stale)["html_hash"].is_null());
    assert!(v(&stale)["renderer"].is_null());
    assert!(v(&stale_shared)["html_hash"].is_null());
    assert_eq!(v(&current)["html_hash"], html_new);
    assert_eq!(v(&current)["renderer"], annotary_renderer::RENDERER_VERSION);
    assert_eq!(v(&current_shared)["html_hash"], html_shared);
    // The events log carries one housekeeping.cleaned row; the read path re-renders on demand.
    let ev = ts.store.events_since(seq).unwrap();
    assert_eq!(ev.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["housekeeping.cleaned"]);
    assert_eq!(ev[0].payload["versions"], 2);
    let (status, _) = ts.get(&format!("/api/v1/docs/{stale}/versions/1/html"), Some(&owner));
    assert_eq!(status, 200);
    assert_eq!(v(&stale)["renderer"], annotary_renderer::RENDERER_VERSION, "re-rendered on read");
    // Nothing left to clean.
    let rep = tick(&ts, &owner);
    assert_eq!(rep["html_dropped"], 0);
    assert_eq!(rep["derived_removed"], 0);
}

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

// spec: S13-09
#[test]
fn the_ticker_of_a_real_serve_fires_on_its_interval() {
    // Prepare a store with auto-purge on and a Document deleted long ago, then start a real
    // `serve` with a 1 s housekeeping interval and watch the Document disappear.
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".annotary");
    let dd = annotary::paths::DataDir::new(data.clone());
    dd.ensure().unwrap();
    let (token, doc_id) = {
        let store = annotary_store::Store::open(&dd.store()).unwrap();
        let out = store
            .setup(annotary_store::SetupInput {
                owner_handle: "rowan".into(),
                owner_name: "Rowan".into(),
                password: None,
            })
            .unwrap();
        let (h, _) = store.put_blob(b"# doomed\n").unwrap();
        let doc =
            store.create_document(&out.owner.id, &out.vault.id, annotary_store::PushInput::new("doomed", h)).unwrap();
        store.delete_document(&out.owner.id, &doc.id).unwrap();
        store.set_auto_purge_days(&out.owner.id, Some(7)).unwrap();
        (out.owner_token.plain, doc.id)
    };
    {
        let conn = rusqlite::Connection::open(dd.store().join("annotary.sqlite")).unwrap();
        let at = jiff::Timestamp::now().checked_sub(jiff::SignedDuration::from_secs(30 * 86_400)).unwrap();
        conn.execute(
            "UPDATE documents SET deleted_at = ?1 WHERE id = ?2",
            rusqlite::params![annotary_store::time::format(at), doc_id],
        )
        .unwrap();
    }
    let child = Command::new(bin())
        .args(["serve", "--port", "0"])
        .env("HOME", home.path())
        .env("ANNOTARY_HOME", &data)
        .env("ANNOTARY_HOUSEKEEPING_INTERVAL", "1")
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let serve = ServeGuard::new(child);
    let server_json = data.join("server.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    let url = loop {
        if let Some(info) = annotary::server::ServerJson::read(&server_json) {
            break info.url;
        }
        assert!(Instant::now() < deadline, "server.json never appeared");
        std::thread::sleep(Duration::from_millis(20));
    };
    let get = |path: &str| -> u16 {
        ureq::get(format!("{url}{path}"))
            .header("Authorization", format!("Bearer {token}"))
            .config()
            .http_status_as_error(false)
            .build()
            .call()
            .map(|r| r.status().as_u16())
            .unwrap_or(0)
    };
    // Deleted but present right after start (the first tick is one interval away)…
    assert_eq!(get(&format!("/api/v1/docs/{doc_id}")), 200);
    // …and gone once the ticker has run.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if get(&format!("/api/v1/docs/{doc_id}")) == 404 {
            break;
        }
        assert!(Instant::now() < deadline, "the ticker never purged the Document");
        std::thread::sleep(Duration::from_millis(100));
    }
    let out = serve.stop_with_output(Duration::from_secs(10));
    assert!(out.status.success());
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(log.contains("housekeeping: purged 1 Document"), "{log}");
}
