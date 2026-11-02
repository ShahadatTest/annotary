//! The real `annotary daemon` binary (ticket 13): `status` against `server.json` (running,
//! stale pid, missing), the off-macOS refusal, `start`/`restart` without a plist, `--json`, and
//! — only when `ANNOTARY_LAUNCHD_TESTS=1` — a live `launchctl` round trip
//! (install → status → restart → stop → start → uninstall) under a temp HOME, a temp data dir and
//! a private label, so the developer's real `com.annotary.server` is never touched.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use annotary::server::ServerJson;
use annotary::testing::TestServer;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

/// `daemon …` with HOME = `home` (no real LaunchAgents dir), a private label, and the OS forced
/// to macOS so `status` runs on any platform (it needs no launchctl).
fn daemon(home: &Path, data_dir: &Path) -> Command {
    let mut c = Command::new(bin());
    c.env("HOME", home)
        .env("ANNOTARY_HOME", data_dir)
        .env("ANNOTARY_DAEMON_OS", "macos")
        .env("ANNOTARY_LAUNCHD_LABEL", "com.annotary.cli-daemon-test")
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

// spec: S13-02
#[test]
fn status_reads_server_json_running_stale_and_missing() {
    let mut ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    // Running: the in-process server wrote server.json with our pid.
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let fields: Vec<&str> = so.trim_end().split('\t').collect();
    assert_eq!(fields[0], "running");
    assert_eq!(fields[1], format!("pid {}", std::process::id()));
    assert!(fields[2].starts_with("since 20"), "{so}");
    assert_eq!(fields[3], ts.url);
    assert_eq!(fields[4], format!("store {}", ts.data_dir.join("store").display()));
    assert_eq!(fields[5], format!("log {}", ts.data_dir.join("logs/server.log").display()));
    assert_eq!(fields[6], format!("(v{})", annotary::VERSION));
    assert!(se.contains("no LaunchAgent installed"), "{se}");
    // TTY parity: same fields, two spaces.
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status"]).env("ANNOTARY_TTY", "1").output().unwrap();
    let (so_tty, _) = text(&out);
    assert_eq!(so_tty.trim_end(), fields.join("  "));
    // --json
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["state"], "running");
    assert_eq!(v["pid"], std::process::id());
    assert_eq!(v["url"], ts.url);
    assert_eq!(v["port"], ts.url.rsplit(':').next().unwrap().parse::<u64>().unwrap());
    assert!(v.get("installed").is_none(), "false is omitted");
    let out = daemon(home.path(), &ts.data_dir)
        .args(["daemon", "status", "--json", "state,plist,version,label"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], annotary::VERSION);
    assert_eq!(v["label"], "com.annotary.cli-daemon-test");
    assert!(v["plist"].as_str().unwrap().ends_with("/Library/LaunchAgents/com.annotary.cli-daemon-test.plist"));
    // `--json ?` needs no server and lists the fields.
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status", "--json", "?"]).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "state pid since url port store log plist installed loaded label version"
    );

    // Stale pid: a server.json whose process is gone reads as not running.
    ts.shutdown();
    assert!(!ts.server_json.exists(), "clean exit removed server.json");
    let child = Command::new("true").spawn().unwrap();
    let dead = child.id();
    let mut child = child;
    child.wait().unwrap();
    ServerJson {
        url: "http://127.0.0.1:7377".into(),
        pid: dead,
        started_at: annotary_store::time::now(),
        data_dir: ts.data_dir.clone(),
    }
    .write(&ts.server_json)
    .unwrap();
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status"]).output().unwrap();
    let (so, _) = text(&out);
    if annotary::server::pid_alive(dead) {
        eprintln!("pid {dead} was reused immediately; skipping the stale assertion");
    } else {
        assert_eq!(out.status.code(), Some(0));
        assert!(so.starts_with("not running\t"), "{so}");
        assert!(!so.contains("pid "), "{so}");
    }
    // Missing server.json: not running, same shape.
    std::fs::remove_file(&ts.server_json).unwrap();
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status"]).output().unwrap();
    let (so, _) = text(&out);
    assert_eq!(
        so.trim_end(),
        format!(
            "not running\tstore {}\tlog {}",
            ts.data_dir.join("store").display(),
            ts.data_dir.join("logs/server.log").display()
        )
    );
    let out = daemon(home.path(), &ts.data_dir).args(["daemon", "status", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["state"], "not running");
    assert!(v.get("pid").is_none());
}

// spec: S13-03
#[test]
fn off_macos_every_daemon_verb_exits_1_with_the_service_manager_hint() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".annotary");
    for verb in ["install", "uninstall", "status", "start", "stop", "restart"] {
        let out =
            daemon(home.path(), &data).args(["daemon", verb]).env("ANNOTARY_DAEMON_OS", "linux").output().unwrap();
        let (so, se) = text(&out);
        assert_eq!(out.status.code(), Some(1), "{verb}: {se}");
        assert_eq!(so, "");
        assert_eq!(
            se.trim_end(),
            "error: daemon is unsupported here (linux) — run `annotary serve` under your service manager (systemd, a Windows service, …)",
            "{verb}"
        );
        assert!(!data.join("store").exists(), "{verb} touched the data dir");
    }
    // `--json` errors are the JSON object; `--json ?` still lists fields (no platform check).
    let out = daemon(home.path(), &data)
        .args(["daemon", "status", "--json"])
        .env("ANNOTARY_DAEMON_OS", "linux")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"], "usage");
    assert_eq!(v["message"], "daemon is unsupported here (linux)");
    let out = daemon(home.path(), &data)
        .args(["daemon", "uninstall", "--json", "?"])
        .env("ANNOTARY_DAEMON_OS", "linux")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "plist removed stopped label");
    // `serve` is unchanged there: `--setup-only` works.
    let out = Command::new(bin())
        .args(["serve", "--setup-only"])
        .env("HOME", home.path())
        .env("ANNOTARY_HOME", &data)
        .env("ANNOTARY_DAEMON_OS", "linux")
        .env("ANNOTARY_SETUP_HANDLE", "ana")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

// spec: S13-04
#[test]
fn start_and_restart_without_a_plist_exit_5_and_uninstall_is_idempotent() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".annotary");
    for verb in ["start", "restart"] {
        let out = daemon(home.path(), &data).args(["daemon", verb]).output().unwrap();
        let (_, se) = text(&out);
        assert_eq!(out.status.code(), Some(5), "{verb}: {se}");
        assert!(se.contains("no LaunchAgent installed"), "{se}");
        assert!(se.contains("annotary daemon install"), "{se}");
    }
    // These two need launchctl only to look; without a plist (and our private label never
    // loaded) they are no-ops on a Mac. Elsewhere `launchctl` is missing and `is_loaded` is false.
    let out = daemon(home.path(), &data).args(["daemon", "uninstall"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert!(so.contains("\tnot installed\t"), "{so}");
    assert!(se.contains("nothing to do"), "{se}");
    let out = daemon(home.path(), &data).args(["daemon", "uninstall", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["plist"].as_str().unwrap().ends_with("com.annotary.cli-daemon-test.plist"));
    assert!(v.get("removed").is_none() && v.get("stopped").is_none(), "false omitted: {v}");
}

/// Boots the private label out and removes its plist, whatever happened.
struct Cleanup {
    label: String,
    plist: PathBuf,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let uid = unsafe { libc::getuid() };
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/{}", self.label)]).output();
        let _ = std::fs::remove_file(&self.plist);
    }
}

// spec: S13-01, S13-04, S13-05, S13-12 (Deferred: the live launchctl round trip runs only by hand — `make launchd-test`)
#[test]
fn launchd_round_trip_when_enabled() {
    if std::env::var_os("ANNOTARY_LAUNCHD_TESTS").is_none() || !cfg!(target_os = "macos") {
        eprintln!("skipped: set ANNOTARY_LAUNCHD_TESTS=1 on a Mac to drive the real launchctl");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join(".annotary");
    let label = format!("com.annotary.test-{}", std::process::id());
    let plist = home.path().join("Library/LaunchAgents").join(format!("{label}.plist"));
    let _cleanup = Cleanup { label: label.clone(), plist: plist.clone() };
    // A free port for the daemon.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let d = |args: &[&str]| {
        let mut c = Command::new(bin());
        c.env("HOME", home.path())
            .env("ANNOTARY_HOME", &data)
            .env("ANNOTARY_LAUNCHD_LABEL", &label)
            .env("ANNOTARY_SETUP_HANDLE", "rowan")
            .env("ANNOTARY_SETUP_NAME", "Rowan")
            .env_remove("ANNOTARY_DAEMON_OS")
            .stdin(Stdio::null())
            .arg("daemon")
            .args(args);
        c.output().unwrap()
    };
    // install: first-run setup inline, plist written, server up.
    let out = d(&["install", "--port", &port.to_string()]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "install: {se}\n{so}");
    assert!(so.contains("Tenant created at"), "{so}");
    assert!(so.contains("owner handle [") && so.contains("]: rowan\n"), "{so}");
    assert!(so.contains("Vault \"notes\" created."), "{so}");
    let installed_line = so.lines().find(|l| l.starts_with("Installed ")).expect("Installed line");
    assert!(installed_line.contains("→ running, pid "), "{installed_line}");
    assert!(installed_line.ends_with(&format!("http://127.0.0.1:{port}")), "{installed_line}");
    assert!(so.contains("ANNOTARY_TOKEN=ant_"), "{so}");
    assert!(so.contains("No CLI credential is saved"), "{so}");
    let token = so.lines().find_map(|l| l.trim().strip_prefix("ANNOTARY_TOKEN=")).unwrap().to_string();
    assert!(plist.is_file());
    let p = annotary::cli::daemon::Plist::read(&plist).expect("plist parses");
    assert_eq!(p.label, label);
    assert_eq!(p.port, port);
    assert_eq!(p.data_dir, data);
    assert_eq!(p.log, data.join("logs/server.log"));
    assert_eq!(p.binary, bin().canonicalize().unwrap());
    let info = annotary::server::running_server(&data.join("server.json")).expect("server.json with a live pid");
    let pid1 = info.pid;
    assert_eq!(info.url, format!("http://127.0.0.1:{port}"));
    // The daemon's serve answers the CLI with the owner token.
    let me = Command::new(bin())
        .args(["member", "list"])
        .env("ANNOTARY_TOKEN", &token)
        .env("ANNOTARY_SERVER", &info.url)
        .output()
        .unwrap();
    assert!(me.status.success(), "{}", String::from_utf8_lossy(&me.stderr));
    // status: running, LaunchAgent installed.
    let out = d(&["status"]);
    let (so, se) = text(&out);
    assert!(so.starts_with(&format!("running\tpid {pid1}\t")), "{so}");
    assert!(se.contains(&format!("LaunchAgent {label} installed")), "{se}");
    // restart: a new pid.
    let out = d(&["restart"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "restart: {se}");
    let pid2 = annotary::server::running_server(&data.join("server.json")).unwrap().pid;
    assert_ne!(pid1, pid2, "{so}");
    assert!(so.starts_with(&format!("running\tpid {pid2}\t")), "{so}");
    // stop: not running, plist stays.
    let out = d(&["stop"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "stop: {se}");
    assert!(so.starts_with("not running\t"), "{so}");
    assert!(se.contains("stopped LaunchAgent"), "{se}");
    assert!(plist.is_file());
    assert!(annotary::server::running_server(&data.join("server.json")).is_none());
    // stop again: idempotent.
    let out = d(&["stop"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out).1.contains("not loaded"));
    // start: running again.
    let out = d(&["start"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "start: {se}");
    assert!(so.starts_with("running\tpid "), "{so}");
    // install again (reinstall): stops, rewrites, starts; no setup transcript this time.
    let out = d(&["install", "--port", &port.to_string()]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "reinstall: {se}\n{so}");
    assert!(!so.contains("Tenant created"), "{so}");
    assert!(so.lines().any(|l| l.starts_with("Installed ") && l.contains("→ running, pid ")), "{so}");
    // uninstall: booted out, plist gone, data dir kept.
    let out = d(&["uninstall"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "uninstall: {se}");
    assert!(so.contains("\tstopped and removed\t"), "{so}");
    assert!(!plist.exists());
    assert!(data.join("store/annotary.sqlite").is_file(), "data dir kept");
    let deadline = Instant::now() + Duration::from_secs(10);
    while annotary::server::running_server(&data.join("server.json")).is_some() {
        assert!(Instant::now() < deadline, "server still running after uninstall");
        std::thread::sleep(Duration::from_millis(100));
    }
}
