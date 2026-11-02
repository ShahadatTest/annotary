//! CMD-28: a `annotary serve` a test spawns never outlives the test, even when the test
//! panics while the server runs (`tests/common/serve.rs`).

mod common;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::serve::{GRACE, ServeGuard};

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks for existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[test]
fn a_test_that_panics_while_its_server_runs_leaves_no_server_behind() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".annotary");
    let (tx, rx) = std::sync::mpsc::channel();
    let data_t = data.clone();
    let home_t = home.path().to_path_buf();
    // The "test": starts a real server, waits until it answers, then fails an assertion.
    let failed = std::thread::spawn(move || {
        let child = Command::new(env!("CARGO_BIN_EXE_annotary"))
            .args(["serve", "--port", "0"])
            .env("HOME", &home_t)
            .env("ANNOTARY_HOME", &data_t)
            .env("ANNOTARY_SETUP_HANDLE", "rowan")
            .env_remove("ANNOTARY_TOKEN")
            .env_remove("ANNOTARY_SERVER")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let _serve = ServeGuard::new(child);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(info) = annotary::server::ServerJson::read(&data_t.join("server.json"))
                && info.pid == pid
                && ureq::get(format!("{}/api/v1/health", info.url)).call().is_ok()
            {
                break;
            }
            assert!(Instant::now() < deadline, "serve never answered");
            std::thread::sleep(Duration::from_millis(20));
        }
        tx.send(pid).unwrap();
        panic!("an assertion fails while the server is running");
    })
    .join();
    assert!(failed.is_err(), "the inner test panicked");
    let pid = rx.recv().expect("the server started before the panic");
    assert!(!alive(pid), "serve (pid {pid}) outlived the test that panicked");
    // It got SIGTERM, a clean stop: server.json is gone, the store is unlocked.
    assert!(!data.join("server.json").exists(), "a clean stop removes server.json");
}

#[test]
fn a_server_that_ignores_sigterm_is_killed_after_the_grace_period() {
    // `exec` keeps the ignored SIGTERM disposition in `sleep`, so only SIGKILL stops it. The
    // shell touches `ready` once the trap is set, so SIGTERM never lands before it.
    let tmp = tempfile::tempdir().unwrap();
    let ready = tmp.path().join("ready");
    let child =
        Command::new("sh").args(["-c", "trap '' TERM; touch \"$1\"; exec sleep 60", "sh"]).arg(&ready).spawn().unwrap();
    let pid = child.id();
    let guard = ServeGuard::new(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "the shell never set its trap");
        std::thread::sleep(Duration::from_millis(10));
    }
    let t0 = Instant::now();
    drop(guard);
    let took = t0.elapsed();
    assert!(!alive(pid), "pid {pid} survived the guard");
    assert!(took >= GRACE, "SIGKILL only after the grace period ({took:?})");
    assert!(took < GRACE + Duration::from_secs(3), "{took:?}");
}
