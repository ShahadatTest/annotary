//! The real binary against the in-process server: Vault selection order, pull's local-edit
//! guard, DOC-by-path without a Sidecar, TTY/non-TTY parity for push/pull. (The trycmd goldens
//! in `tests/e2e/` cover the rest; these need a second Vault or a profile `vault`, which only the
//! store / a config file can set up today.)

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

// spec: S02-08
#[test]
fn vault_selection_order_sidecar_flag_profile_then_error_when_more_than_one() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    std::fs::write(work.path().join("one.md"), "one\n").unwrap();
    // One Vault: never asked.
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "one.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.starts_with("one\t"));

    // Two Vaults, no Sidecar, no flag, no profile: typed error listing them (exit 1).
    ts.create_vault("drafts");
    std::fs::write(work.path().join("two.md"), "two\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "two.md"]);
    assert_eq!(out.status.code(), Some(1));
    let (so, se) = text(&out);
    assert!(so.is_empty());
    assert_eq!(
        se.trim(),
        "error: no sidecar beside two.md and 2 Vaults — pass --vault: notes, drafts — or set `vault` in your profile"
    );
    // --vault picks.
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "two.md", "--vault", "drafts"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let sc = std::fs::read_to_string(work.path().join("two.annotary")).unwrap();
    assert!(sc.contains("\"vault\": \"drafts\""));
    // Sidecar wins over --vault (a Version's Vault is fixed) — with a note.
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "two.md", "--vault", "notes"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out).1.contains("--vault notes ignored"));
    assert!(text(&out).0.contains("already v1"));
    // Profile `vault` (config.toml) picks for a new Document, and scopes `list`.
    let cd = home.path().join(".annotary");
    std::fs::create_dir_all(&cd).unwrap();
    std::fs::write(
        cd.join("config.toml"),
        format!("[profiles.d]\nserver = \"{}\"\ntoken = \"{tok}\"\nvault = \"drafts\"\n", ts.url),
    )
    .unwrap();
    std::fs::write(work.path().join("three.md"), "three\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["-p", "d", "push", "three.md"])
        .current_dir(work.path())
        .env("HOME", home.path())
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_HOME")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let sc = std::fs::read_to_string(work.path().join("three.annotary")).unwrap();
    assert!(sc.contains("\"vault\": \"drafts\""));
    let out = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["-p", "d", "list"])
        .current_dir(work.path())
        .env("HOME", home.path())
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_HOME")
        .output()
        .unwrap();
    let (so, _) = text(&out);
    assert!(
        so.contains("\ttwo\t") && so.contains("\tthree\t") && !so.contains("\tone\t"),
        "profile vault scopes list: {so}"
    );
    // Without a Vault in scope, `list` shows every readable Vault.
    let out = run(&ts, home.path(), work.path(), &tok, &["list"]);
    let (so, _) = text(&out);
    assert!(so.contains("\tone\t") && so.contains("\ttwo\t") && so.contains("\tthree\t"));
    let out = run(&ts, home.path(), work.path(), &tok, &["list", "--vault", "notes"]);
    let (so, _) = text(&out);
    assert!(so.contains("\tone\t") && !so.contains("\ttwo\t"));
    // A Name unique in the profile's Vault resolves; --vault on list to an unknown Vault is exit 2.
    let out = run(&ts, home.path(), work.path(), &tok, &["list", "--vault", "nope"]);
    assert_eq!(out.status.code(), Some(2));
}

// spec: S02-16, S02-14, S02-07
#[test]
fn pull_refuses_over_local_edits_unless_forced() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    std::fs::write(work.path().join("plan.md"), "# Plan\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    // Untouched: pull is fine (byte-identical rewrite).
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, "plan.md\tplan.annotary\tv1\n");
    assert_eq!(text(&out).1, "pull: 0 threads, 0 assets\n");
    // Edited: refused, exit 5, nothing overwritten.
    std::fs::write(work.path().join("plan.md"), "# Plan\n\nlocal edit\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan"]);
    assert_eq!(out.status.code(), Some(5));
    let (so, se) = text(&out);
    assert!(so.is_empty());
    assert!(se.starts_with("error: plan.md has local edits since v1"), "{se}");
    assert!(se.contains("`pull --force`"));
    assert_eq!(std::fs::read_to_string(work.path().join("plan.md")).unwrap(), "# Plan\n\nlocal edit\n");
    // --force overwrites.
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan", "--force"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(work.path().join("plan.md")).unwrap(), "# Plan\n");
    // No Sidecar + differing .md: refused; identical: fine.
    std::fs::remove_file(work.path().join("plan.annotary")).unwrap();
    std::fs::write(work.path().join("plan.md"), "something else\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan"]);
    assert_eq!(out.status.code(), Some(5));
    assert!(text(&out).1.contains("without a Sidecar and differs"));
    std::fs::write(work.path().join("plan.md"), "# Plan\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(work.path().join("plan.annotary").exists());
    // A path DOC without a Sidecar beside it is exit 2.
    std::fs::write(work.path().join("loose.md"), "x\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["show", "loose.md"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out).1.contains("no Sidecar beside loose.md"));
    // The id itself resolves everywhere.
    let sc = std::fs::read_to_string(work.path().join("plan.annotary")).unwrap();
    let id = sc.lines().find_map(|l| l.trim().strip_prefix("\"id\": \"")).unwrap().trim_end_matches("\",").to_string();
    assert_eq!(id.len(), 8);
    let out = run(&ts, home.path(), work.path(), &tok, &["status", &id]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.starts_with(&format!("plan\t{id}\tv1\t")));
    // A path DOC with a Sidecar resolves — even to a different Name than the file's.
    std::fs::rename(work.path().join("plan.md"), work.path().join("renamed.md")).unwrap();
    std::fs::rename(work.path().join("plan.annotary"), work.path().join("renamed.annotary")).unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["status", "./renamed.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.starts_with("plan\t"));
    // `--out DIR` creates the directory and writes both files there.
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan", "--out", "sub/dir"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(work.path().join("sub/dir/plan.md").exists() && work.path().join("sub/dir/plan.annotary").exists());
}

/// The Version cell (`v1 by @rowan 3s`) with its relative age replaced by `<age>`, so a parity
/// check does not depend on the clock (CMD-25); the age must still be well formed.
fn without_age(field: &str) -> String {
    match field.rsplit_once(' ') {
        Some((head, age)) if field.starts_with('v') && head.contains(" by @") => {
            let (n, unit) = age.split_at(age.len() - 1);
            assert!(!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && "smhd".contains(unit), "age {age:?}");
            format!("{head} <age>")
        }
        _ => field.to_string(),
    }
}

// spec: S02-30, S02-28
#[test]
fn tty_and_non_tty_show_the_same_fields_and_tty_appends_the_url() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    std::fs::write(work.path().join("plan.md"), "# Plan\n").unwrap();
    let plain = run(&ts, home.path(), work.path(), &tok, &["push", "plan.md"]);
    let (so, _) = text(&plain);
    let fields: Vec<&str> = so.trim().split('\t').collect();
    assert_eq!(fields.len(), 3);
    assert_eq!((fields[0], fields[2]), ("plan", "v1"));
    let id = fields[1].to_string();
    let tty = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["push", "plan.md", "--no-color"])
        .current_dir(work.path())
        .env("HOME", home.path())
        .env("ANNOTARY_TOKEN", &tok)
        .env("ANNOTARY_SERVER", &ts.url)
        .env("ANNOTARY_TTY", "1")
        .output()
        .unwrap();
    let (so, _) = text(&tty);
    assert!(so.starts_with(&format!("plan  {id}  already v1")), "{so}");
    assert!(so.trim_end().ends_with(&format!("{}/d/{id}", ts.url)), "TTY appends the web URL: {so}");
    let tty = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["show", "plan", "--no-color"])
        .current_dir(work.path())
        .env("HOME", home.path())
        .env("ANNOTARY_TOKEN", &tok)
        .env("ANNOTARY_SERVER", &ts.url)
        .env("ANNOTARY_TTY", "1")
        .output()
        .unwrap();
    let (so, _) = text(&tty);
    assert!(so.trim_end().ends_with(&format!("{}/d/{id}", ts.url)), "{so}");
    // The two calls straddle a second on purpose (CMD-25): the parity holds whatever the clock.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let plain = run(&ts, home.path(), work.path(), &tok, &["show", "plan"]);
    let (po, _) = text(&plain);
    let tty_fields: Vec<String> = so.trim().split("  ").filter(|f| !f.is_empty()).map(without_age).collect();
    let plain_fields: Vec<String> = po.trim().split('\t').map(without_age).collect();
    assert!(plain_fields.iter().any(|f| f.ends_with(" <age>")), "show prints the Version's age: {po}");
    assert_eq!(&tty_fields[..plain_fields.len()], &plain_fields[..], "same fields, same order");
    let tty = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["pull", "plan", "--no-color", "--out", "t"])
        .current_dir(work.path())
        .env("HOME", home.path())
        .env("ANNOTARY_TOKEN", &tok)
        .env("ANNOTARY_SERVER", &ts.url)
        .env("ANNOTARY_TTY", "1")
        .output()
        .unwrap();
    assert_eq!(text(&tty).0, "plan.md  plan.annotary  v1\n");
}

// <!-- ---- ticket 16 / 17 ---- -->
// spec: S02-12, S03-05, S16-03, S17-03, S17-08
#[test]
fn a_stale_push_is_refused_before_re_anchoring_or_uploading_anything() {
    // Ticket 16: `push --strict` printed the `threads: … re-anchored` line (and uploaded the new
    // blobs) before the server refused a moved head. Ticket 17 made the refusal the default
    // whenever the file lacks the head's changes — and it still happens before anything is
    // uploaded, `--strict` or not.
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    std::fs::write(a.path().join("plan.md"), "# Plan\n\nfirst line\n").unwrap();
    let out = run(&ts, home.path(), a.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    // A Thread on v1 so a push re-anchors (the `threads:` line would print).
    let out = run(&ts, home.path(), a.path(), &tok, &["thread", "new", "plan", "3:0-3:10", "-m", "hmm"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    // A second clone pushes v2; the first clone (base v1) is now behind the head.
    let out = run(&ts, home.path(), b.path(), &tok, &["pull", "plan"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    std::fs::write(b.path().join("plan.md"), "# Plan\n\nfirst line\n\nsecond push\n").unwrap();
    let out = run(&ts, home.path(), b.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    // The stale clone, with or without --strict: exit 5, the missing hunk, no `threads:` line,
    // nothing uploaded.
    let stale = "# Plan\n\nfirst line\n\nstale edit\n";
    std::fs::write(a.path().join("plan.md"), stale).unwrap();
    for strict in [false, true] {
        let mut args = vec!["push", "plan.md"];
        if strict {
            args.push("--strict");
        }
        let out = run(&ts, home.path(), a.path(), &tok, &args);
        assert_eq!(out.status.code(), Some(5), "{}", text(&out).1);
        let (so, se) = text(&out);
        assert!(so.is_empty(), "{so}");
        assert_eq!(
            se,
            "error: v2 by @rowan changed plan.md since your v1 — those changes are not in your file — put them into plan.md and push again\n\
             @@ -1,3 +1,5 @@\n # Plan\n \n first line\n+\n+second push\n"
        );
    }
    let stale_hash = annotary_sidecar::identity::sha256_hex(stale.as_bytes());
    assert!(!ts.store.has_blob(&stale_hash), "nothing was uploaded");
    // `--json` names the same kind as the server's refusal would.
    let out = run(&ts, home.path(), a.path(), &tok, &["push", "plan.md", "--json"]);
    assert_eq!(out.status.code(), Some(5));
    assert!(text(&out).1.starts_with("{\"error\":\"head_moved\""), "{}", text(&out).1);
    // `--discard-head-changes` pushes over v2 as v3, warning first (the line still prints).
    let out = run(&ts, home.path(), a.path(), &tok, &["push", "plan.md", "--discard-head-changes"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let (so, se) = text(&out);
    assert!(so.starts_with("plan\t"), "{so}");
    assert!(so.trim_end().ends_with("\tv3"), "{so}");
    assert!(se.starts_with("warning: discarding v2 by @rowan since your v1\n"), "{se}");
    assert!(se.contains("threads: 1 re-anchored"), "{se}");
}
// <!-- ---- end ticket 16 ---- -->
