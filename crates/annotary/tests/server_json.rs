//! `server.json` lifecycle: written on start, removed on clean exit, stale pid = not running.

use annotary::server::{ServerJson, pid_alive, running_server};
use annotary::testing::TestServer;

// spec: S01-03, S01-19
#[test]
fn written_on_start_removed_on_clean_exit() {
    let mut ts = TestServer::start();
    let path = ts.server_json.clone();
    assert_eq!(path, ts.data_dir.join("server.json"));
    let info = ServerJson::read(&path).expect("server.json written after bind");
    assert_eq!(info.url, ts.url);
    assert_eq!(info.pid, std::process::id());
    assert_eq!(info.data_dir, ts.data_dir);
    assert!(annotary_store::time::parse(&info.started_at).is_some(), "started_at is a timestamp");
    assert!(info.is_running());
    assert!(running_server(&path).is_some());
    // Data-dir layout.
    assert!(ts.data_dir.join("logs").is_dir());
    assert!(ts.data_dir.join("store").join("LOCK").is_file());
    assert!(ts.data_dir.join("store").join("meta.json").is_file());
    ts.shutdown();
    assert!(!path.exists(), "server.json removed on clean exit");
}

// spec: S01-03
#[test]
fn stale_pid_reads_as_not_running() {
    // A process that has exited: spawn `true`, wait, and reuse its pid.
    let child = std::process::Command::new("true").spawn().expect("spawn true");
    let dead_pid = child.id();
    let mut child = child;
    child.wait().unwrap();
    // The kernel does not reuse a pid immediately; still, guard the assertion.
    if pid_alive(dead_pid) {
        eprintln!("pid {dead_pid} was reused immediately; skipping the stale check");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.json");
    let stale = ServerJson {
        url: "http://127.0.0.1:7377".into(),
        pid: dead_pid,
        started_at: annotary_store::time::now(),
        data_dir: dir.path().to_path_buf(),
    };
    stale.write(&path).unwrap();
    let read = ServerJson::read(&path).unwrap();
    assert_eq!(read, stale);
    assert!(!read.is_running(), "stale pid must read as not running");
    assert!(running_server(&path).is_none());
    assert!(ServerJson::read(&dir.path().join("missing.json")).is_none());
    assert!(!pid_alive(0));
    assert!(pid_alive(std::process::id()));
}
