//! The real `annotary password` binary (CMD-22): clear or set a local password in a stopped
//! data dir's store — every Member's, or one — and the refusals (a running server's LOCK, no
//! store, an unknown Member, an empty password, neither flag).

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use annotary_store::Store;

/// The tests in this file run one at a time: each opens a store in-process (taking its `LOCK`)
/// between the CLI runs, and a child another test forks in that moment inherits the `flock` until
/// it execs, so a parallel test could see its own store as locked.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

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
        .env_remove("ANNOTARY_PASSWORD")
        .stdin(Stdio::null());
    c
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

/// A set-up data dir whose owner @rowan has the password `hunter2` and whose second Member
/// @ana has `secret`; @linter has none.
fn data_dir_with_passwords() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let out = cli(dir.path())
        .args(["serve", "--setup-only"])
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .env("ANNOTARY_SETUP_NAME", "Rowan")
        .env("ANNOTARY_SETUP_PASSWORD", "hunter2")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{:?}", text(&out));
    let store = Store::open(&dir.path().join("store")).unwrap();
    let owner = store.founding_owner().unwrap().unwrap();
    let (ana, _) = store.add_member(&owner.id, "ana", None).unwrap();
    store.add_member(&owner.id, "linter", None).unwrap();
    store.set_password(&owner.id, &ana.id, Some("secret")).unwrap();
    // @old had a password and was disabled: --clear leaves it alone, --member refuses it.
    let (old, _) = store.add_member(&owner.id, "old", None).unwrap();
    store.set_password(&owner.id, &old.id, Some("gone")).unwrap();
    store.disable_member(&owner.id, &old.id).unwrap();
    dir
}

fn passwords(dir: &Path) -> Vec<(String, bool)> {
    let store = Store::open(&dir.join("store")).unwrap();
    store.list_members().unwrap().into_iter().map(|m| (m.handle, m.has_password)).collect()
}

// spec: S30-01
#[test]
fn clear_removes_every_members_password_so_loopback_trusts_the_owner_again() {
    let _serial = serial();
    let dir = data_dir_with_passwords();
    let out = cli(dir.path()).args(["password", "--clear"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, "@rowan\tremoved\n@ana\tremoved\n");
    assert!(se.contains("no local password is set — a browser on this machine is trusted as the owner"), "{se}");
    let p = passwords(dir.path());
    assert!(p.iter().filter(|(h, _)| h != "old").all(|(_, has)| !has), "{p:?}");
    assert!(p.contains(&("old".into(), true)), "the disabled Member is skipped: {p:?}");
    let store = Store::open(&dir.path().join("store")).unwrap();
    assert!(!store.password_set().unwrap());
    drop(store);
    // Nothing left to clear: no record, a note, exit 0.
    let out = cli(dir.path()).args(["password", "--clear"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!((out.status.code(), so.as_str()), (Some(0), ""));
    assert!(se.contains("no local password to remove"), "{se}");
}

// spec: S30-01
#[test]
fn clear_with_member_removes_only_that_password_and_tty_json_follow_the_conventions() {
    let _serial = serial();
    let dir = data_dir_with_passwords();
    let out = cli(dir.path())
        .args(["password", "--clear", "--member", "@ana", "--no-color"])
        .env("ANNOTARY_TTY", "1")
        .output()
        .unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, "MEMBER  PASSWORD\n@ana    removed\n");
    // The owner still has one, so the browser is not trusted: no such note.
    assert!(!se.contains("trusted as the owner"), "{se}");
    let p = passwords(dir.path());
    assert!(p.contains(&("rowan".into(), true)) && p.contains(&("ana".into(), false)), "{p:?}");
    let out = cli(dir.path()).args(["password", "--clear", "--json", "member,password,id"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["member"], "rowan");
    assert_eq!(v[0]["password"], "removed");
    assert!(v[0]["id"].is_string());
}

// spec: S30-02
#[test]
fn set_takes_the_password_from_the_prompt_never_an_argument() {
    let _serial = serial();
    let dir = data_dir_with_passwords();
    cli(dir.path()).args(["password", "--clear"]).output().unwrap();
    // A stdin line answers the prompt on a non-TTY; the owner by default.
    let mut child = cli(dir.path())
        .args(["password", "--set"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(b"n3w-pass\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, "@rowan\tset\n");
    assert!(se.contains("new local password for @rowan: ********"), "{se}");
    assert!(!se.contains("n3w-pass") && !so.contains("n3w-pass"));
    let store = Store::open(&dir.path().join("store")).unwrap();
    assert_eq!(store.check_password("n3w-pass").unwrap().handle, "rowan");
    drop(store);
    // ANNOTARY_PASSWORD answers it; --member picks another Member.
    let out = cli(dir.path())
        .args(["password", "--set", "--member", "linter"])
        .env("ANNOTARY_PASSWORD", "c0dex")
        .output()
        .unwrap();
    assert_eq!(text(&out).0, "@linter\tset\n");
    let store = Store::open(&dir.path().join("store")).unwrap();
    assert_eq!(store.check_password("c0dex").unwrap().handle, "linter");
    drop(store);
    // An empty answer changes nothing: exit 1.
    let out = cli(dir.path()).args(["password", "--set"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!((out.status.code(), so.as_str()), (Some(1), ""));
    assert!(se.contains("error: empty password — --clear removes a password"), "{se}");
}

// spec: S30-03
#[test]
fn refused_while_a_server_holds_the_store_and_the_other_exit_paths() {
    let _serial = serial();
    let dir = data_dir_with_passwords();
    // A running server's LOCK (held here by an open Store, as `serve` holds it): exit 5, unchanged.
    let held = Store::open(&dir.path().join("store")).unwrap();
    let out = cli(dir.path()).args(["password", "--clear"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!((out.status.code(), so.as_str()), (Some(5), ""));
    assert!(se.contains("is locked by another annotary serve"), "{se}");
    drop(held);
    assert!(passwords(dir.path()).contains(&("rowan".into(), true)));
    // An unknown --json field is a usage error before anything changes.
    let out = cli(dir.path()).args(["password", "--clear", "--json", "bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(passwords(dir.path()).contains(&("rowan".into(), true)));
    // A disabled Member: exit 5, unchanged.
    let out = cli(dir.path()).args(["password", "--clear", "--member", "old"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert!(text(&out).1.contains("error: @old is disabled"));
    // A named Member without a password: the note names that Member, exit 0.
    let out = cli(dir.path()).args(["password", "--clear", "--member", "linter"]).output().unwrap();
    let (so, se) = text(&out);
    assert_eq!((out.status.code(), so.as_str()), (Some(0), ""));
    assert!(se.contains("password: @linter has no local password"), "{se}");
    // Unknown Member: exit 2.
    let out = cli(dir.path()).args(["password", "--clear", "--member", "nobody"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out).1.contains("error: no Member @nobody"));
    // Neither flag, or both: usage, exit 1.
    for args in [&["password"][..], &["password", "--clear", "--set"][..]] {
        let out = cli(dir.path()).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
    }
    // No store in the data dir: exit 5, and none is created.
    let empty = tempfile::tempdir().unwrap();
    let out = cli(empty.path()).args(["password", "--clear"]).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert!(text(&out).1.contains("error: no store at"));
    assert!(!empty.path().join("store").exists());
}
