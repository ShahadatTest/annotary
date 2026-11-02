//! The real binary against the in-process server for the Sidecar-driven declaration flow trycmd
//! cannot drive (files edited between steps): pull → edit `actions.available` and drop
//! `inherited` → push declares (and `show` lists the new set) → `[]` declares none → an invalid
//! declaration is refused before anything is uploaded; `inherited` present ⇒ push sends nothing.

use std::path::Path;
use std::process::{Command, Output};

use annotary::testing::TestServer;

fn run(ts: &TestServer, home: &Path, cwd: &Path, token: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY")
        .output()
        .unwrap()
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

// spec: S05-06, S05-07, S05-10, S05-12, S05-15
#[test]
fn sidecar_declarations_round_trip_through_pull_edit_push() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    let md = work.path().join("plan.md");
    let sc = work.path().join("plan.annotary");
    std::fs::write(&md, "# Plan\n\nWe ship the CLI first.\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let id = text(&out).0.split('\t').nth(1).unwrap().to_string();

    // The Sidecar written by push carries the Vault defaults with `inherited: true`.
    let before = std::fs::read_to_string(&sc).unwrap();
    assert!(before.contains("    \"inherited\": true\n"), "{before}");
    let out = run(&ts, home.path(), work.path(), &tok, &["show", "plan"]);
    assert!(text(&out).0.ends_with("\tactions:approve,send-back\n"), "{}", text(&out).0);

    // A hand edit of `available` that keeps `inherited: true` is ignored on push (nothing sent).
    let ignored = before.replace("\"id\": \"send-back\"", "\"id\": \"ignored\"");
    std::fs::write(&sc, &ignored).unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, format!("plan\t{id}\talready v1\n"));
    let out = run(&ts, home.path(), work.path(), &tok, &["show", "plan"]);
    assert!(text(&out).0.ends_with("\tactions:approve,send-back\n"), "{}", text(&out).0);
    let after = std::fs::read_to_string(&sc).unwrap();
    assert_eq!(after, before, "push rewrote the Sidecar from the server (the hand edit is gone)");

    // Editing `available` and dropping the flag declares per-Document; the next pull/push shows
    // the Document's own set (no `inherited`), and `act` honours it.
    let declared = before.replace("    ],\n    \"inherited\": true\n", "    ]\n").replace(
        "{\"id\": \"send-back\", \"label\": \"Send back\", \"intent\": \"neutral\"}",
        "{\"id\": \"ship\", \"label\": \"Ship it\", \"intent\": \"positive\"}",
    );
    assert_ne!(declared, before);
    std::fs::write(&sc, &declared).unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, format!("plan\t{id}\talready v1\n"), "declaring changes no content");
    let out = run(&ts, home.path(), work.path(), &tok, &["show", "plan"]);
    assert!(text(&out).0.ends_with("\tactions:approve,ship\n"), "{}", text(&out).0);
    let written = std::fs::read_to_string(&sc).unwrap();
    assert!(!written.contains("inherited"), "{written}");
    assert!(written.contains("{\"id\": \"ship\", \"label\": \"Ship it\", \"intent\": \"positive\"}"), "{written}");
    let out = run(&ts, home.path(), work.path(), &tok, &["act", "plan.md", "send-back"]);
    assert_eq!(out.status.code(), Some(5), "{}", text(&out).1);
    assert!(
        text(&out).1.contains("no Action \"send-back\" on plan") && text(&out).1.contains("available: approve, ship"),
        "{}",
        text(&out).1
    );
    let out = run(&ts, home.path(), work.path(), &tok, &["act", "plan.md", "ship"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, "plan\tv1\tship\tby @rowan\n");
    // A fresh pull carries the log with the snapshotted label and round-trips byte-identically.
    let out_dir = work.path().join("pulled");
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan", "--out", "pulled"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let pulled = std::fs::read_to_string(out_dir.join("plan.annotary")).unwrap();
    assert!(pulled.contains("\"log\": [\n      {\"id\": \"ship\", \"by\": \"rowan\", \"at\": \""), "{pulled}");
    assert!(pulled.contains("\"version\": 1, \"label\": \"Ship it\", \"intent\": \"positive\"}"), "{pulled}");
    let parsed = annotary_sidecar::parse(&pulled).unwrap();
    assert_eq!(annotary_sidecar::emit(&parsed), pulled);
    assert!(!parsed.actions.inherited && parsed.actions.available.len() == 2 && parsed.actions.log.len() == 1);
    // Pushing the pulled tree back declares the same set again: nothing changes.
    let out = run(&ts, home.path(), &out_dir, &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, format!("plan\t{id}\talready v1\n"));
    let declared_events =
        ts.store.events_since(0).unwrap().into_iter().filter(|e| e.kind == "document.actions_declared").count();
    assert_eq!(declared_events, 1, "re-sending the same declaration is not a change");

    // `[]` declares none; the log stays.
    let none = pulled.replace(
        "    \"available\": [\n      {\"id\": \"approve\", \"label\": \"Approve\", \"intent\": \"positive\"},\n      {\"id\": \"ship\", \"label\": \"Ship it\", \"intent\": \"positive\"}\n    ],\n",
        "    \"available\": [],\n",
    );
    assert_ne!(none, pulled);
    std::fs::write(out_dir.join("plan.annotary"), &none).unwrap();
    let out = run(&ts, home.path(), &out_dir, &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let out = run(&ts, home.path(), &out_dir, &tok, &["show", "plan"]);
    assert!(text(&out).0.contains("\taction:ship@v1 by @rowan\topen:0\tactions:-\n"), "{}", text(&out).0);
    let out = run(&ts, home.path(), &out_dir, &tok, &["act", "plan", "ship"]);
    assert_eq!(out.status.code(), Some(5), "{}", text(&out).1);
    assert!(text(&out).1.contains("declares no Actions"), "{}", text(&out).1);
    let out = run(&ts, home.path(), &out_dir, &tok, &["actions", "plan"]);
    assert_eq!(text(&out).0.lines().count(), 1, "{}", text(&out).0);
    let written = std::fs::read_to_string(out_dir.join("plan.annotary")).unwrap();
    assert!(written.contains("  \"actions\": {\n    \"log\": [\n"), "{written}");

    // An invalid declaration is refused by the CLI before anything is uploaded (exit 1).
    let bad = written.replace("  \"actions\": {\n", "  \"actions\": {\n    \"available\": [{\"id\": \"Ship It\"}],\n");
    assert_ne!(bad, written);
    std::fs::write(out_dir.join("plan.annotary"), &bad).unwrap();
    std::fs::write(out_dir.join("plan.md"), "# Plan v2\n").unwrap();
    let out = run(&ts, home.path(), &out_dir, &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out).1);
    assert!(
        text(&out).1.starts_with(
            "error: plan.annotary: invalid Action declaration: entry 0: id \"Ship It\" must be kebab-case"
        ),
        "{}",
        text(&out).1
    );
    let out = run(&ts, home.path(), work.path(), &tok, &["status", "plan"]);
    assert!(text(&out).0.contains("\tv1\t"), "nothing was pushed: {}", text(&out).0);
    let bad = written.replace("  \"actions\": {\n", "  \"actions\": {\n    \"available\": [{\"label\": \"no id\"}],\n");
    std::fs::write(out_dir.join("plan.annotary"), &bad).unwrap();
    let out = run(&ts, home.path(), &out_dir, &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out).1);
    assert!(text(&out).1.contains("actions.available[0] has no `id`"), "{}", text(&out).1);
}
