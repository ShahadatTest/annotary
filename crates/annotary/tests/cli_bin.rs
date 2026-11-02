//! The real binary, spawned: first-run setup non-interactively, exit 6 never hangs, `-V`.

use std::process::{Command, Stdio};
use std::time::Instant;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
}

// spec: S01-16
#[test]
fn stopped_server_is_exit_6_fast_never_a_hang() {
    let home = tempfile::tempdir().unwrap();
    // A closed port: connection refused.
    let started = Instant::now();
    let out = bin()
        .args(["member", "list"])
        .env("ANNOTARY_HOME", home.path())
        .env("ANNOTARY_TOKEN", "ant_whatever")
        .env("ANNOTARY_SERVER", "http://127.0.0.1:1")
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.trim(),
        "error: server at http://127.0.0.1:1 not reachable — run `annotary serve` or `annotary daemon install`"
    );
    assert!(out.stdout.is_empty());
    assert!(elapsed.as_secs() < 5, "took {elapsed:?}");
    // JSON form.
    let out = bin()
        .args(["member", "list", "--json"])
        .env("ANNOTARY_HOME", home.path())
        .env("ANNOTARY_TOKEN", "ant_whatever")
        .env("ANNOTARY_SERVER", "http://127.0.0.1:1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"], "unreachable");
    assert_eq!(v["message"], "server at http://127.0.0.1:1 not reachable");
    assert!(v["hint"].as_str().unwrap().contains("annotary serve"));
}

// spec: S01-18
#[test]
fn version_and_usage_exit_codes() {
    let out = bin().arg("-V").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), format!("annotary {}", annotary::VERSION));
    let out = bin().arg("--help").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Exit codes: 0 ok · 1 usage"));
    let out = bin().arg("bogus-verb").output().unwrap();
    assert_eq!(out.status.code(), Some(1), "usage errors exit 1, not clap's 2");
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}

// spec: S01-10, S01-19, S01-21
#[test]
fn first_run_setup_is_scriptable_via_stdin_and_env() {
    let home = tempfile::tempdir().unwrap();
    // Answers piped on stdin (blank = default): handle from $USER, name default, no password.
    let mut child = bin()
        .args(["serve", "--setup-only"])
        .env("ANNOTARY_HOME", home.path())
        .env("USER", "ana")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"\n\nsecret-pw\n").unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("owner handle [ana]: ana"), "{stdout}");
    assert!(stdout.contains("display name ["), "{stdout}");
    assert!(stdout.contains("local password (blank = loopback trust, browser is you): ********"), "{stdout}");
    assert!(stdout.contains("Vault \"notes\" created."), "{stdout}");
    assert!(stdout.contains("ANNOTARY_TOKEN=ant_"), "{stdout}");
    assert!(stdout.contains("No CLI credential is saved"), "{stdout}");
    assert!(stdout.contains("annotary login"), "{stdout}");
    assert!(stdout.contains("annotary member add"), "{stdout}");
    // Layout: store/ with LOCK + meta.json (tenant id set), logs/, no config.toml, no server.json.
    let store = home.path().join("store");
    assert!(store.join("annotary.sqlite").is_file());
    assert!(store.join("LOCK").is_file());
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(store.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["schema_version"], annotary_store::SCHEMA_VERSION);
    assert!(meta["tenant_id"].as_str().is_some());
    assert!(home.path().join("logs").is_dir());
    assert!(!home.path().join("config.toml").exists(), "server never writes a profile");
    assert!(!home.path().join("server.json").exists(), "--setup-only does not serve");
    // The owner has an argon2id password.
    let s = annotary_store::Store::open(&store).unwrap();
    let owner = s.member_by_handle("ana").unwrap().expect("owner ana");
    assert!(owner.has_password);
    assert_eq!(owner.role, annotary_store::Role::Owner);
    assert_eq!(owner.grants, vec![annotary_store::Grant::all()]);
    assert_eq!(s.list_vaults().unwrap()[0].name, "notes");
    drop(s);

    // Second run: already set up, no prompts, exit 0.
    let out =
        bin().args(["serve", "--setup-only"]).env("ANNOTARY_HOME", home.path()).stdin(Stdio::null()).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("already set up (owner @ana)"));

    // Env answers, no stdin at all, and --data-dir instead of ANNOTARY_HOME.
    let home2 = tempfile::tempdir().unwrap();
    let out = bin()
        .args(["--data-dir", home2.path().to_str().unwrap(), "serve", "--setup-only"])
        .env_remove("ANNOTARY_HOME")
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .env("ANNOTARY_SETUP_NAME", "Rowan Vale")
        .env("ANNOTARY_SETUP_PASSWORD", "")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("owner handle [") && stdout.contains("]: rowan"), "{stdout}");
    assert!(stdout.contains("display name [") && stdout.contains("]: Rowan Vale"), "{stdout}");
    assert!(home2.path().join("store").join("meta.json").is_file());
    let s = annotary_store::Store::open(&home2.path().join("store")).unwrap();
    assert!(!s.member_by_handle("rowan").unwrap().unwrap().has_password);

    // An invalid env handle is a usage error (exit 1), nothing created.
    let home3 = tempfile::tempdir().unwrap();
    let out = bin()
        .args(["serve", "--setup-only"])
        .env("ANNOTARY_HOME", home3.path())
        .env("ANNOTARY_SETUP_HANDLE", "Not Valid")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let s = annotary_store::Store::open(&home3.path().join("store")).unwrap();
    assert!(!s.is_set_up().unwrap());
}
