//! CMD-1: the running server keeps consistent snapshots of its store under
//! `DATA_DIR/snapshots/` for a file-by-file backup to copy (`server/snapshots.rs`). Through the
//! real binary: one right after start, a new one after a change, none while idle, and the newest
//! restores into a fresh data dir with the change in it.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use annotary_store::{SnapshotFile, list_snapshots};
use common::serve::ServeGuard;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn snaps(dir: &Path) -> Vec<SnapshotFile> {
    list_snapshots(dir).unwrap()
}

#[test]
fn the_server_snapshots_at_start_after_each_change_and_not_while_idle() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".annotary");
    let dd = annotary::paths::DataDir::new(data.clone());
    dd.ensure().unwrap();
    let token = {
        let store = annotary_store::Store::open(&dd.store()).unwrap();
        let out = store
            .setup(annotary_store::SetupInput {
                owner_handle: "rowan".into(),
                owner_name: "Rowan".into(),
                // A password: a loopback `GET /` would otherwise sign in and change the store (CMD-19).
                password: Some(annotary::testing::TEST_PASSWORD.into()),
            })
            .unwrap();
        out.owner_token.plain
    };
    let child = Command::new(bin())
        .args(["serve", "--port", "0"])
        .env("HOME", home.path())
        .env("ANNOTARY_HOME", &data)
        .env("ANNOTARY_SNAPSHOT_INTERVAL", "1")
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let serve = ServeGuard::new(child);
    let dir = data.join("snapshots");

    // One right after start.
    wait_for("the first snapshot", 15, || snaps(&dir).len() == 1);
    let first = snaps(&dir).remove(0);

    // A change (a new Vault, through the API) gives a newer snapshot with a later seq.
    let mut url = None;
    wait_for("server.json", 15, || {
        url = annotary::server::ServerJson::read(&data.join("server.json")).map(|s| s.url);
        url.is_some()
    });
    let url = url.unwrap();
    let status = ureq::post(format!("{url}/api/v1/vaults"))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(serde_json::json!({ "name": "after-start" }))
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 201, "vault created");
    wait_for("a snapshot after the change", 15, || snaps(&dir).len() == 2);
    let all = snaps(&dir);
    assert_eq!(all[0], first);
    assert!(all[1].seq > first.seq, "{all:?}");

    // Idle: three more ticks write nothing new, even with a port watcher probing (CMD-19).
    std::thread::sleep(Duration::from_millis(500));
    assert!(annotary::testing::probe_root(&url) != 0, "the probe reached the server");
    std::thread::sleep(Duration::from_millis(3000));
    assert_eq!(snaps(&dir), all, "an unchanged store is not copied again");

    let out = serve.stop_with_output(Duration::from_secs(10));
    assert!(out.status.success());
    let log = String::from_utf8_lossy(&out.stderr);
    assert_eq!(log.matches("snapshot: wrote").count(), 2, "{log}");

    // Restore drill: the newest snapshot, untarred into a fresh data dir, has the new Vault.
    let restored = tempfile::tempdir().unwrap();
    tar::Archive::new(std::fs::File::open(&all[1].path).unwrap()).unpack(restored.path()).unwrap();
    let back = annotary_store::Store::open(&restored.path().join("store")).unwrap();
    assert!(back.vault_by_name("after-start").unwrap().is_some());
    let older = tempfile::tempdir().unwrap();
    tar::Archive::new(std::fs::File::open(&first.path).unwrap()).unpack(older.path()).unwrap();
    let back = annotary_store::Store::open(&older.path().join("store")).unwrap();
    assert!(back.vault_by_name("after-start").unwrap().is_none(), "the first snapshot predates the change");
}
