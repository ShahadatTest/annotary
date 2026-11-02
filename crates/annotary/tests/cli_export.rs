//! The real `annotary export` binary (ticket 13): a tar of `store/` taken beside a running
//! server that restores — untar into a fresh data dir — to a working `serve`; the refusals
//! (existing file without `--force`, not a `.tar` name, no store).

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use common::serve::ServeGuard;
use serde_json::{Value, json};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

fn cli(data_dir: &Path) -> Command {
    let mut c = Command::new(bin());
    c.env("ANNOTARY_HOME", data_dir)
        .env_remove("ANNOTARY_TTY")
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .stdin(Stdio::null());
    c
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

fn upload(ts: &TestServer, token: &str, bytes: &[u8]) -> String {
    let hash = sha256_hex(bytes);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .send(bytes)
        .unwrap();
    assert!(res.status().is_success());
    hash
}

// spec: S13-06
#[test]
fn export_beside_a_running_server_restores_to_a_working_serve() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let png = upload(&ts, &owner, b"PNG bytes for export");
    let md = upload(&ts, &owner, b"# plan\n\n![a](a.png)\n");
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&owner),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md,
                 "manifest": [{ "ref": "a.png", "hash": png, "size": 20, "type": "image/png" }] }),
    );
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let (_, agent) = ts.add_member("linter");

    let out_dir = tempfile::tempdir().unwrap();
    let tar = out_dir.path().join("backup.tar");
    // Export while the server runs (its LOCK is held, WAL is live).
    let out = cli(&ts.data_dir).arg("export").arg(&tar).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(
        so.trim_end(),
        format!("{}\t2 blobs\t{}\tschema v{}", tar.display(), size_of(&tar), annotary_store::SCHEMA_VERSION)
    );
    assert!(se.contains(&format!("export: server running (pid {})", std::process::id())), "{se}");
    // TTY parity and --json.
    let out = cli(&ts.data_dir).arg("export").arg(&tar).arg("--force").env("ANNOTARY_TTY", "1").output().unwrap();
    let (so_tty, _) = text(&out);
    assert_eq!(so_tty.trim_end(), so.trim_end().replace('\t', "  "));
    let out = cli(&ts.data_dir).arg("export").arg(&tar).args(["--force", "--json"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["blobs"], 2);
    assert_eq!(v["schema_version"], annotary_store::SCHEMA_VERSION);
    assert_eq!(v["file"], tar.to_string_lossy().as_ref());
    let out =
        cli(&ts.data_dir).arg("export").arg(&tar).args(["--force", "--json", "live,tenant_id,store"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["live"], true);
    assert_eq!(v["tenant_id"], ts.store.tenant_id().unwrap().unwrap());
    assert_eq!(v["store"], ts.data_dir.join("store").to_string_lossy().as_ref());
    let out = cli(&ts.data_dir).args(["export", "x.tar", "--json", "?"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "file blobs bytes size schema_version tenant_id store live"
    );

    // Refusals: exists without --force (5), not a .tar (1), no store (5).
    let out = cli(&ts.data_dir).arg("export").arg(&tar).output().unwrap();
    let (_, se) = text(&out);
    assert_eq!(out.status.code(), Some(5), "{se}");
    assert!(se.contains("exists — --force overwrites it"), "{se}");
    let out = cli(&ts.data_dir).args(["export", "backup.tgz"]).output().unwrap();
    let (_, se) = text(&out);
    assert_eq!(out.status.code(), Some(1));
    assert!(se.contains("does not end in .tar"), "{se}");
    let empty = tempfile::tempdir().unwrap();
    let out = cli(empty.path()).arg("export").arg(out_dir.path().join("none.tar")).output().unwrap();
    let (_, se) = text(&out);
    assert_eq!(out.status.code(), Some(5), "{se}");
    assert!(se.contains("no store at"), "{se}");
    assert!(!out_dir.path().join("none.tar").exists());
    // An interrupted export never leaves a half tar: no temp file remains either.
    assert!(std::fs::read_dir(out_dir.path()).unwrap().flatten().all(|e| e.file_name() == "backup.tar"));

    // Restore: untar into a fresh data dir, run a real `serve` there, read the Document, the
    // Asset and authenticate with both Tokens.
    let restore = tempfile::tempdir().unwrap();
    let data = restore.path().join(".annotary");
    std::fs::create_dir_all(&data).unwrap();
    tar::Archive::new(std::fs::File::open(&tar).unwrap()).unpack(&data).unwrap();
    assert!(data.join("store/annotary.sqlite").is_file());
    assert!(data.join("store/meta.json").is_file());
    assert!(!data.join("store/LOCK").exists(), "LOCK does not travel");
    let child = Command::new(bin())
        .args(["serve", "--port", "0"])
        .env("HOME", restore.path())
        .env("ANNOTARY_HOME", &data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut serve = ServeGuard::new(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    let url = loop {
        if let Some(info) = annotary::server::ServerJson::read(&data.join("server.json")) {
            break info.url;
        }
        assert!(Instant::now() < deadline, "restored serve never wrote server.json");
        std::thread::sleep(Duration::from_millis(20));
    };
    let get = |path: &str, token: &str| -> (u16, Vec<u8>) {
        let mut r = ureq::get(format!("{url}{path}"))
            .header("Authorization", format!("Bearer {token}"))
            .config()
            .http_status_as_error(false)
            .build()
            .call()
            .unwrap();
        (r.status().as_u16(), r.body_mut().read_to_vec().unwrap())
    };
    let (status, body) = get(&format!("/api/v1/docs/{id}"), &owner);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let d: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(d["name"], "plan");
    let (status, body) = get(&format!("/api/v1/blobs/{png}"), &agent);
    assert_eq!(status, 200);
    assert_eq!(body, b"PNG bytes for export");
    let (status, body) = get(&format!("/api/v1/docs/{id}/versions/1"), &agent);
    assert_eq!(status, 200);
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["manifest"][0]["hash"], png);
    let (status, _) = get("/api/v1/members", &owner);
    assert_eq!(status, 200);
    serve.sigterm();
    assert!(serve.wait_timeout(Duration::from_secs(10)).success());
}

// spec: S21-02
#[test]
fn export_missing_a_referenced_blob_retries_then_exits_5_and_writes_nothing() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let md = upload(&ts, &owner, b"# lost\n");
    let (status, doc) =
        ts.post("/api/v1/docs", Some(&owner), &json!({ "vault": "notes", "name": "lost", "md_hash": md }));
    assert_eq!(status, 201, "{doc}");
    // The blob the database references is gone every time export looks (what a purge racing every
    // attempt would do): never a silently short archive.
    std::fs::remove_file(ts.store.blob_path(&md)).unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    let tar = out_dir.path().join("backup.tar");
    let out = cli(&ts.data_dir).arg("export").arg(&tar).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(5), "{se}");
    assert_eq!(so, "");
    assert_eq!(se.matches("removed during the export (a purge or GC ran) — retrying").count(), 2, "{se}");
    assert!(
        se.contains("error: export incomplete: 1 blob the database snapshot references is not in the store (after 3 attempts) — nothing was written"),
        "{se}"
    );
    assert!(se.contains(&format!("missing blob {md}")), "{se}");
    assert!(std::fs::read_dir(out_dir.path()).unwrap().next().is_none(), "no archive and no temp file is left");
}

fn size_of(p: &Path) -> String {
    annotary_store::caps::human_size(std::fs::metadata(p).unwrap().len())
}
