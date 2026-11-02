//! `deploy/verify_live.py` (`make verify-live`, CMD-22) on a fixture snapshot, never Rowan's data:
//! the copy is served with its password cleared, the CLI reaches it through the private `copy`
//! profile only, the copy is deleted after a run (also a failed one), and a crashed run's copy
//! is swept by the next.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// Stops a `serve` this test deliberately orphaned (not our child, so `ServeGuard` cannot own
/// it) if the test fails before the sweep has stopped it: on drop, a pid whose command line
/// still names `data` gets SIGKILL (`-ww`: BSD `ps` truncates it otherwise). A reused pid
/// serving something else is left alone.
struct OrphanServe {
    pid: i32,
    data: PathBuf,
}

impl Drop for OrphanServe {
    fn drop(&mut self) {
        let Ok(out) = Command::new("ps").args(["-ww", "-o", "command=", "-p", &self.pid.to_string()]).output() else {
            return;
        };
        let cmd = String::from_utf8_lossy(&out.stdout);
        if cmd.contains("annotary") && cmd.contains(&*self.data.to_string_lossy()) {
            // SAFETY: SIGKILL to a pid whose command line names this test's own temp data dir.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
    }
}

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A snapshot tar of a set-up store whose owner has a local password.
fn fixture_snapshot(dir: &Path) -> PathBuf {
    let data = dir.join("data");
    let run = |args: &[&str]| {
        let out = Command::new(bin())
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .env_remove("ANNOTARY_HOME")
            .env("ANNOTARY_SETUP_HANDLE", "rowan")
            .env("ANNOTARY_SETUP_NAME", "Rowan")
            .env("ANNOTARY_SETUP_PASSWORD", "hunter2")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    run(&["serve", "--setup-only"]);
    let tar = dir.join("annotary-20261030T000000Z-s1.tar");
    run(&["export", tar.to_str().unwrap()]);
    tar
}

fn verify_live(snapshot: &Path, tmp: &Path) -> (Option<i32>, Value, String) {
    let out = Command::new("python3")
        .arg(repo_root().join("deploy/verify_live.py"))
        .arg("--bin")
        .arg(bin())
        .arg("--snapshot")
        .arg(snapshot)
        .arg("--tmp")
        .arg(tmp)
        .arg("--no-scenarios")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    (out.status.code(), v, String::from_utf8_lossy(&out.stderr).to_string())
}

fn copies(tmp: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(tmp)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("cd-verify-live-"))
        .collect()
}

// spec: S30-04
#[test]
fn a_run_serves_the_copy_unlocked_on_loopback_and_deletes_it() {
    let dir = tempfile::tempdir().unwrap();
    let snap = fixture_snapshot(dir.path());
    let tmp = tempfile::tempdir().unwrap();
    let (code, v, se) = verify_live(&snap, tmp.path());
    assert_eq!(code, Some(0), "{v} {se}");
    assert_eq!(v["status"], "ok");
    assert_eq!(v["checks"]["port_not_live"], true);
    assert_eq!(v["checks"]["loopback_only"], true);
    assert_eq!(v["checks"]["cli_home_has_only_the_copy"], true);
    assert_eq!(v["documents"], 0);
    assert_eq!(v["deleted"], true);
    // A fixture run checks what it can of production (the copied file unchanged); the live-only
    // check (the live server refuses the copy's Token) is absent.
    assert_eq!(v["production"]["copied_snapshot_unchanged"], true);
    assert_eq!(v["production"]["unit_not_restarted"], true);
    assert!(v["production"].get("copy_token_rejected_by_live").is_none(), "{v}");
    assert!(copies(tmp.path()).is_empty());
    // The snapshot itself is untouched: the copy's password was cleared, not the original's.
    let restored = tempfile::tempdir().unwrap();
    let out = Command::new("tar").arg("-xf").arg(&snap).arg("-C").arg(restored.path()).output().unwrap();
    assert!(out.status.success());
    let store = annotary_store::Store::open(&restored.path().join("store")).unwrap();
    assert!(store.password_set().unwrap());
}

// spec: S30-05
#[test]
fn a_failed_run_still_deletes_its_copy_and_only_finished_runs_are_swept() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = |name: &str, pid: Option<&str>| {
        let d = tmp.path().join(name);
        std::fs::create_dir(&d).unwrap();
        if let Some(p) = pid {
            std::fs::write(d.join("pid"), p).unwrap();
        }
        std::fs::write(d.join("doc.md"), "private text").unwrap();
        d
    };
    // A crashed run (its pid is gone), a live run (this test's pid), and a run that has not
    // written its pid yet (young, no pid file): only the first is swept.
    let crashed = folder("cd-verify-live-crashed", Some("999999999\n"));
    let running = folder("cd-verify-live-running", Some(&format!("{}\n", std::process::id())));
    let starting = folder("cd-verify-live-starting", None);
    // A snapshot that is not a tar: the run fails after making its copy folder.
    let bad = tmp.path().join("not-a-snapshot.tar");
    std::fs::write(&bad, "nope").unwrap();
    let (code, v, _) = verify_live(&bad, tmp.path());
    assert_eq!(code, Some(1), "{v}");
    assert_eq!(v["status"], "fail");
    assert_eq!(v["deleted"], true);
    assert_eq!(v["swept"].as_array().unwrap().len(), 1, "{v}");
    assert!(!crashed.exists());
    assert!(running.exists() && starting.exists());
    assert_eq!(copies(tmp.path()).len(), 2);
}

// spec: S30-05
#[test]
fn the_sweep_stops_a_server_a_hard_killed_run_left_serving_its_copy() {
    let dir = tempfile::tempdir().unwrap();
    let snap = fixture_snapshot(dir.path());
    let tmp = tempfile::tempdir().unwrap();
    // A run that was SIGKILLed: its copy, unlocked and still served, and a dead pid.
    let stale = tmp.path().join("cd-verify-live-killed");
    let data = stale.join("data");
    std::fs::create_dir_all(&data).unwrap();
    assert!(Command::new("tar").arg("-xf").arg(&snap).arg("-C").arg(&data).status().unwrap().success());
    std::fs::write(stale.join("pid"), "999999999\n").unwrap();
    // Started as the script starts it (its own session), then orphaned the way a SIGKILLed
    // run leaves it: the init process, not this test, reaps it.
    let launcher = Command::new("python3")
        .args([
            "-c",
            "import subprocess,sys; p=subprocess.Popen(sys.argv[1:], start_new_session=True, \
             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); print(p.pid)",
        ])
        .arg(bin())
        .args(["serve", "--port", "0", "--bind", "127.0.0.1", "--link-base", "none", "--data-dir"])
        .arg(&data)
        .env_remove("ANNOTARY_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let pid: i32 = String::from_utf8_lossy(&launcher.stdout).trim().parse().unwrap();
    let _orphan = OrphanServe { pid, data: data.clone() }; // a failed assertion below still stops it
    std::fs::write(stale.join("server.pid"), format!("{pid}\n")).unwrap();
    let running = |pid: i32| {
        let out = Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
        let st = String::from_utf8_lossy(&out.stdout).trim().to_string();
        !st.is_empty() && !st.starts_with('Z')
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !running(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(running(pid), "the leftover server should be running");
    let bad = tmp.path().join("not-a-snapshot.tar");
    std::fs::write(&bad, "nope").unwrap();
    let (_, v, _) = verify_live(&bad, tmp.path());
    assert_eq!(v["swept"].as_array().unwrap().len(), 1, "{v}");
    // Stopped (its group killed) before its folder went.
    assert!(!running(pid), "the sweep left the copy being served");
    assert!(!stale.exists());
}

// spec: S30-06
#[test]
fn the_scenario_config_refuses_anything_but_a_loopback_copy() {
    // The scenarios change data, so the Playwright config itself refuses the live port, another
    // host, or a CLI home outside the copy folder — even when run by hand. Needs `npm ci`: the
    // cargo-only CI jobs skip it, the e2e job (`make e2e`, after `npm ci`) runs it.
    let pw = repo_root().join("node_modules/.bin/playwright");
    if !pw.exists() {
        eprintln!("skipped: no node_modules (npm ci); `make e2e` runs this test after `npm ci` in CI");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let copy = tmp.path().join("cd-verify-live-x");
    let home = copy.join("cli");
    std::fs::create_dir_all(&home).unwrap();
    let outside = tmp.path().join("elsewhere");
    let cases: [(&str, &Path, &Path, &str); 4] = [
        ("http://127.0.0.1:7377", &copy, &home, "is not a loopback copy"),
        ("http://localhost:4001", &copy, &home, "is not a loopback copy"),
        ("http://127.0.0.1:4001", &copy, &outside, "must be a cd-verify-live-* copy folder"),
        ("http://127.0.0.1:4001", tmp.path(), &home, "must be a cd-verify-live-* copy folder"),
    ];
    for (url, verify_tmp, cli_home, why) in cases {
        let out = Command::new(&pw)
            .args(["test", "-c", "verify/playwright.config.ts", "--list"])
            .current_dir(repo_root())
            .env("VERIFY_URL", url)
            .env("VERIFY_TMP", verify_tmp)
            .env("VERIFY_CLI_HOME", cli_home)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(!out.status.success(), "{url}: {all}");
        assert!(all.contains(why), "{url}: {all}");
    }
}

/// A stub `ssh` that runs the remote command here, a stub `sudo` and `systemctl`, and a stub `curl`
/// that answers as the live server does to a Token it does not know (and shows it was given none on
/// its command line).
#[cfg(unix)]
fn stub_host(bin_dir: &Path, snapshots: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let stubs: [(&str, String); 4] = [
        ("ssh", "#!/bin/bash\nwhile [ \"$1\" = -o ]; do shift 2; done\nshift\neval \"$*\"\n".to_string()),
        ("sudo", "#!/bin/bash\n[ \"$1\" = -n ] && shift\nexec \"$@\"\n".to_string()),
        ("systemctl", "#!/bin/sh\necho 'Mon 2026-11-02 05:09:11 UTC'\n".to_string()),
        (
            "curl",
            format!(
                "#!/bin/bash\ncat > {0}/curl-stdin\necho \"$*\" > {0}/curl-args\nprintf 401\n",
                snapshots.display()
            ),
        ),
    ];
    for (name, body) in stubs {
        let path = bin_dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The host's GNU `stat -c %Y`, which a Mac spells differently.
    let stat = bin_dir.join("stat");
    let body = "#!/bin/sh\nshift 2\n[ \"$(uname)\" = Darwin ] && exec /usr/bin/stat -f %m \"$@\"\nexec /usr/bin/stat -c %Y \"$@\"\n";
    std::fs::write(&stat, body).unwrap();
    std::fs::set_permissions(&stat, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn verify_settings(tmp: &Path) -> Command {
    let mut c = Command::new("python3");
    c.arg(repo_root().join("deploy/verify_live.py"))
        .arg("--bin")
        .arg(bin())
        .arg("--tmp")
        .arg(tmp)
        .arg("--no-scenarios")
        .env_remove("VERIFY_SNAPSHOT_GLOB")
        .env_remove("VERIFY_UNIT")
        .env_remove("SERVER_SSH")
        .stdin(Stdio::null());
    c
}

// spec: S33-07
#[test]
fn a_live_run_without_its_settings_says_which_and_copies_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let out = verify_settings(tmp.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let problems = v["problems"].to_string();
    assert!(problems.contains("VERIFY_SNAPSHOT_GLOB and VERIFY_UNIT are not set"), "{problems}");
    assert!(copies(tmp.path()).is_empty());
}

// spec: S33-08
#[cfg(unix)]
#[test]
fn over_ssh_a_run_copies_the_hosts_newest_snapshot_and_checks_production_there() {
    let dir = tempfile::tempdir().unwrap();
    let snap = fixture_snapshot(dir.path());
    let host = tempfile::tempdir().unwrap();
    let older = host.path().join("annotary-20261029T000000Z-s1.tar");
    std::fs::write(&older, "not the newest").unwrap();
    std::fs::copy(&snap, host.path().join("annotary-20261030T000000Z-s1.tar")).unwrap();
    let bin_dir = tempfile::tempdir().unwrap();
    stub_host(bin_dir.path(), host.path());
    let tmp = tempfile::tempdir().unwrap();
    let path = format!("{}:{}", bin_dir.path().display(), std::env::var("PATH").unwrap());
    let out = verify_settings(tmp.path())
        .env("PATH", path)
        .env("SERVER_SSH", "the-host")
        .env("VERIFY_SNAPSHOT_GLOB", host.path().join("annotary-*.tar"))
        .env("VERIFY_UNIT", "annotary.service")
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{v} {}", String::from_utf8_lossy(&out.stderr));
    assert!(v["snapshot"].as_str().unwrap().ends_with("annotary-20261030T000000Z-s1.tar"), "{v}");
    assert_eq!(v["production"]["unit_not_restarted"], true);
    assert_eq!(v["production"]["copied_snapshot_unchanged"], true);
    assert_eq!(v["production"]["copy_token_rejected_by_live"], true);
    assert!(copies(tmp.path()).is_empty(), "the copy, and the tar it came from, are deleted");
    // The copy's Token reached the host over stdin, never on a command line.
    let args = std::fs::read_to_string(host.path().join("curl-args")).unwrap();
    let stdin = std::fs::read_to_string(host.path().join("curl-stdin")).unwrap();
    assert!(!args.contains("ant_"), "{args}");
    assert!(stdin.contains("Authorization: Bearer ant_"), "{stdin}");
}
