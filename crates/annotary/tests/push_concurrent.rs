//! Ticket 17 — concurrent edits: `push` checks the file contains the head's changes since its
//! base (ADR 0002). The commit API against the in-process server (`contains_head`, discard, the
//! server's recheck with `--verify-edge` on / off), then the real binary in two working dirs on
//! one Document — the git-worktree scenario without git: contained, refused with the diff, fixed
//! and re-pushed, `--json`, a stale unedited file, an Asset the head changed, a conflicted Sidecar
//! on push and pull. The real-git flow is `git_worktree_flow` at the end.

use std::path::Path;
use std::process::{Command, Output};

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

// ---------------------------------------------------------------- the commit API

fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let (status, body) = ts.post("/api/v1/blobs/check", Some(token), &json!({ "hashes": [hash] }));
    assert_eq!(status, 200, "{body}");
    if !body["lacking"].as_array().unwrap().is_empty() {
        let r = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
            .header("Authorization", format!("Bearer {token}"))
            .content_type("application/octet-stream")
            .send(md)
            .unwrap();
        assert!(r.status().is_success());
    }
    hash
}

fn commit(ts: &TestServer, token: &str, id: &str, md: &str, extra: Value) -> (u16, Value) {
    let hash = upload(ts, token, md.as_bytes());
    let mut body = json!({ "name": "plan", "md_hash": hash });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    ts.post(&format!("/api/v1/docs/{id}/versions"), Some(token), &body)
}

const V1: &str = "# Plan\n\n## Storage\nSQLite, one file.\n\n## API\nREST over HTTP.\n";
const V2: &str = "# Plan\n\n## Storage\nSQLite, one file, WAL mode.\n\n## API\nREST over HTTP.\n";

// spec: S17-08, S17-09
#[test]
fn the_commit_api_checks_the_claim() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let hash = upload(&ts, &tok, V1.as_bytes());
    let (_, doc) = ts.post("/api/v1/docs", Some(&tok), &json!({ "vault": "notes", "name": "plan", "md_hash": hash }));
    let id = doc["id"].as_str().unwrap().to_string();
    let (status, _) = commit(&ts, &agent, &id, V2, json!({ "base_version": 1 }));
    assert_eq!(status, 201);
    // A file with v2's change and one of its own, from base v1:
    let merged = V2.replace("REST over HTTP.", "REST over HTTP, JSON bodies.");
    // … no claim ⇒ refused; a claim on another head ⇒ "head moved again".
    let (status, out) = commit(&ts, &tok, &id, &merged, json!({ "base_version": 1 }));
    assert_eq!((status, out["error"].as_str()), (409, Some("head_moved")), "{out}");
    let (status, out) = commit(&ts, &tok, &id, &merged, json!({ "base_version": 1, "contains_head": 1 }));
    assert_eq!(status, 409);
    assert_eq!(out["message"], "head moved again (now v2) — your file was checked against v1");
    assert_eq!(out["hint"], "push again");
    // A client that lies (claims v2 but lacks its change) is refused while verify is on …
    let lie = V1.replace("REST over HTTP.", "gRPC.");
    let (status, out) = commit(&ts, &tok, &id, &lie, json!({ "base_version": 1, "contains_head": 2 }));
    assert_eq!(status, 409, "{out}");
    assert_eq!(out["message"], "v2 by @scribe changed plan since your v1 — those changes are not in your file");
    assert_eq!((out["base"].clone(), out["head"].clone(), out["by"].clone()), (json!(1), json!(2), json!(["scribe"])));
    // … the honest one goes through …
    let (status, out) = commit(&ts, &tok, &id, &merged, json!({ "base_version": 1, "contains_head": 2 }));
    assert_eq!((status, out["version"].clone()), (201, json!(3)), "{out}");
    assert_eq!(out["discarded"], json!([]));
    // … and with verify off the edge is trusted.
    ts.store.set_verify_edge(false);
    let (status, out) = commit(&ts, &tok, &id, &lie, json!({ "base_version": 2, "contains_head": 3 }));
    assert_eq!((status, out["version"].clone()), (201, json!(4)), "{out}");
    ts.store.set_verify_edge(true);
    // Discard: accepted over v3–v4, recorded on the event.
    let (status, out) = commit(&ts, &tok, &id, "mine\n", json!({ "base_version": 2, "discard_head_changes": true }));
    assert_eq!((status, out["discarded"].clone()), (201, json!([3, 4])), "{out}");
    let (_, events) = ts.get(&format!("/api/v1/events?since=0&doc={id}&kinds=version"), Some(&tok));
    let last = events["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((last["payload"]["version"].clone(), last["payload"]["discarded"].clone()), (json!(5), json!([3, 4])));
}

// ---------------------------------------------------------------- the real binary

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

fn ok(out: Output) -> (String, String) {
    let (so, se) = (String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string());
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    (so, se)
}

fn refused(out: Output) -> String {
    let se = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(5), "stderr: {se}");
    assert!(out.stdout.is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
    se
}

fn head(ts: &TestServer, id: &str) -> (u64, String) {
    let (_, d) = ts.get(&format!("/api/v1/docs/{id}"), Some(&ts.owner_token));
    let md = d["md_hash"].as_str().unwrap().to_string();
    let text = String::from_utf8(ts.store.read_blob(&md).unwrap().unwrap()).unwrap();
    (d["version"].as_u64().unwrap(), text)
}

fn statuses(ts: &TestServer, id: &str) -> Vec<(String, String)> {
    let (_, t) = ts.get(&format!("/api/v1/docs/{id}/threads?status=all"), Some(&ts.owner_token));
    t["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let st = t["anchor"]["current"]["status"].as_str().unwrap_or("document").to_string();
            (t["id"].as_str().unwrap().to_string(), st)
        })
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

const BASE: &str = "\
# Plan

## Storage
SQLite, one file.

## API
REST over HTTP.

## Later
Nothing yet.
";

// spec: S17-02, S17-03, S17-04, S17-05
#[test]
fn two_working_dirs_on_one_document() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (a.path(), b.path());
    std::fs::write(a.join("plan.md"), BASE).unwrap();
    ok(run(&ts, home.path(), a, &tok, &["push", "plan.md"]));
    let id = ts.get("/api/v1/docs", Some(&tok)).1["docs"][0]["id"].as_str().unwrap().to_string();
    ok(run(&ts, home.path(), a, &tok, &["thread", "new", &id, "7:0-7:4", "-m", "REST?"]));
    ok(run(&ts, home.path(), b, &agent, &["pull", &id]));

    // The agent (dir A) pushes v2 — a new Storage line and a Thread on it.
    let v2 = BASE.replace("SQLite, one file.\n", "SQLite, one file, WAL mode.\nBlobs are content-addressed.\n");
    std::fs::write(a.join("plan.md"), &v2).unwrap();
    ok(run(&ts, home.path(), a, &agent, &["push", "plan.md", "-m", "storage"]));
    ok(run(&ts, home.path(), a, &tok, &["thread", "new", &id, "5:0-5:5", "-m", "which hash?"]));

    // Dir B edits Later without v2 ⇒ refused, the missing hunk on stderr, nothing uploaded.
    let mine = BASE.replace("Nothing yet.", "Search, later.");
    std::fs::write(b.join("plan.md"), &mine).unwrap();
    let se = refused(run(&ts, home.path(), b, &tok, &["push", "plan.md"]));
    assert_eq!(
        se,
        "error: v2 by @scribe changed plan.md since your v1 — those changes are not in your file — put them into plan.md and push again\n\
         @@ -1,7 +1,8 @@\n # Plan\n \n ## Storage\n-SQLite, one file.\n+SQLite, one file, WAL mode.\n\
         +Blobs are content-addressed.\n \n ## API\n REST over HTTP.\n"
    );
    assert!(!se.contains("threads:"), "refused before re-anchoring");
    assert!(!ts.store.has_blob(&sha256_hex(mine.as_bytes())), "nothing uploaded");
    // `--json`: the same refusal as data.
    let out = run(&ts, home.path(), b, &tok, &["push", "plan.md", "--json"]);
    let err: Value = serde_json::from_str(refused(out).trim()).unwrap();
    assert_eq!(err["error"], "head_moved");
    assert_eq!((err["base"].clone(), err["head"].clone(), err["by"].clone()), (json!(1), json!(2), json!(["scribe"])));
    assert_eq!(err["missing"][0]["hunk"], "@@ -1,7 +1,8 @@");
    assert!(err["missing"][0]["lines"].as_array().unwrap().contains(&json!("+Blobs are content-addressed.")));
    assert_eq!(err["assets"], json!([]));
    assert_eq!(err["hint"], "put them into plan.md and push again", "no diff lines folded into the hint");

    // B puts the change in (by hand) ⇒ a normal push from the head: no warning, v3, both
    // Threads re-anchored, Sidecar at v3.
    let merged = v2.replace("Nothing yet.", "Search, later.");
    std::fs::write(b.join("plan.md"), &merged).unwrap();
    let (so, se) = ok(run(&ts, home.path(), b, &tok, &["push", "plan.md"]));
    assert!(so.trim_end().ends_with("\tv3"), "{so}");
    assert!(!se.contains("warning"), "{se}");
    assert!(se.contains("threads: 2 re-anchored — 2 anchored"), "{se}");
    assert_eq!(head(&ts, &id), (3, merged.clone()));
    assert!(std::fs::read_to_string(b.join("plan.annotary")).unwrap().contains("\"base_version\": 3,"));
    assert_eq!(statuses(&ts, &id), vec![("t1".into(), "anchored".into()), ("t2".into(), "anchored".into())]);

    // Dir A never pulled v3 and did not edit: a stale file is refused, never reverts the head.
    let se = refused(run(&ts, home.path(), a, &agent, &["push", "plan.md"]));
    assert!(se.starts_with("error: v3 by @rowan changed plan.md since your v2 — "), "{se}");
    assert!(se.contains("\n-Nothing yet.\n+Search, later.\n"), "{se}");
    assert_eq!(head(&ts, &id).0, 3);
    // Several Versions since the base name them all.
    ok(run(&ts, home.path(), b, &agent, &["push", "plan.md", "-m", "same"])); // already v3
    std::fs::write(b.join("plan.md"), merged.replace("REST", "HTTP/JSON")).unwrap();
    ok(run(&ts, home.path(), b, &tok, &["push", "plan.md"]));
    let se = refused(run(&ts, home.path(), a, &agent, &["push", "plan.md"]));
    assert!(se.starts_with("error: v3–v4 by @rowan changed plan.md since your v2"), "{se}");
}

// spec: S17-06
#[test]
fn an_asset_the_head_changed_is_a_head_change() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (a.path(), b.path());
    let png = |n: u8| -> Vec<u8> {
        let mut v = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/assets/a.png")).unwrap();
        v.push(n); // trailing bytes keep it a valid PNG with a new hash
        v
    };
    let md = "# Pic\n\n![arch](img/arch.png)\n\nText.\n";
    std::fs::create_dir_all(a.join("img")).unwrap();
    std::fs::write(a.join("img/arch.png"), png(1)).unwrap();
    std::fs::write(a.join("pic.md"), md).unwrap();
    ok(run(&ts, home.path(), a, &tok, &["push", "pic.md"]));
    let id = ts.get("/api/v1/docs", Some(&tok)).1["docs"][0]["id"].as_str().unwrap().to_string();
    ok(run(&ts, home.path(), b, &tok, &["pull", &id, "--assets"]));
    // A replaces the image (same ref, new bytes) ⇒ v2 with the same Markdown.
    std::fs::write(a.join("img/arch.png"), png(2)).unwrap();
    ok(run(&ts, home.path(), a, &tok, &["push", "pic.md"]));
    // B edits only the text, keeping the old image ⇒ refused, naming the Asset.
    std::fs::write(b.join("pic.md"), md.replace("Text.", "More text.")).unwrap();
    let se = refused(run(&ts, home.path(), b, &tok, &["push", "pic.md"]));
    assert_eq!(
        se,
        "error: v2 by @rowan changed pic.md since your v1 — those changes are not in your file — put them into pic.md and push again\nasset img/arch.png changed in v2\n"
    );
    let err: Value =
        serde_json::from_str(refused(run(&ts, home.path(), b, &tok, &["push", "pic.md", "--json"])).trim()).unwrap();
    assert_eq!(
        (err["missing"].clone(), err["assets"].clone()),
        (json!([]), json!([{"ref": "img/arch.png", "change": "changed"}]))
    );
    // With the head's image (and a new one the head never touched) ⇒ contained.
    std::fs::write(b.join("img/arch.png"), png(2)).unwrap();
    std::fs::write(b.join("img/new.png"), png(3)).unwrap();
    std::fs::write(b.join("pic.md"), md.replace("Text.", "More text.\n\n![new](img/new.png)")).unwrap();
    let (so, _) = ok(run(&ts, home.path(), b, &tok, &["push", "pic.md"]));
    assert!(so.trim_end().ends_with("\tv3"), "{so}");
}

/// Wrap `from` (a line of `text`) in git conflict markers with `to` on the other side.
fn conflict(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from}");
    text.replacen(from, &format!("<<<<<<< HEAD\n{from}\n=======\n{to}\n>>>>>>> other"), 1)
}

// spec: S17-07
#[test]
fn a_conflicted_sidecar_needs_no_resolving() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let home = tempfile::tempdir().unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (a.path(), b.path());
    std::fs::write(a.join("plan.md"), BASE).unwrap();
    ok(run(&ts, home.path(), a, &tok, &["push", "plan.md"]));
    let id = ts.get("/api/v1/docs", Some(&tok)).1["docs"][0]["id"].as_str().unwrap().to_string();
    ok(run(&ts, home.path(), b, &tok, &["pull", &id]));
    let v2 = BASE.replace("REST over HTTP.", "REST over HTTP, JSON.");
    std::fs::write(a.join("plan.md"), &v2).unwrap();
    ok(run(&ts, home.path(), a, &tok, &["push", "plan.md"]));
    // B's Sidecar as git leaves it after merging A's branch: base 1 vs 2 in conflict, and the
    // two sides declare different Actions. B's Markdown has A's change and one of its own.
    let sc_a = std::fs::read_to_string(a.join("plan.annotary")).unwrap();
    // (One side drops `inherited`: its `available` becomes the Document's own declaration.)
    let sc = conflict(&sc_a, "  \"base_version\": 2,", "  \"base_version\": 1,");
    let sc = conflict(&sc, "    \"inherited\": true", "    \"inherited\": false");
    std::fs::write(b.join("plan.annotary"), &sc).unwrap();
    std::fs::write(b.join("plan.md"), v2.replace("Nothing yet.", "Search.")).unwrap();
    let (so, se) = ok(run(&ts, home.path(), b, &tok, &["push", "plan.md"]));
    assert!(so.trim_end().ends_with("\tv3"), "{so}");
    assert_eq!(
        se.lines().take(2).collect::<Vec<_>>(),
        vec![
            "note: plan.annotary has git conflict markers — using v1 (the older side) as your base; the push rewrites it",
            "note: the two sides of plan.annotary declare different actions.available — none sent, the Document keeps its set",
        ]
    );
    let clean = std::fs::read_to_string(b.join("plan.annotary")).unwrap();
    assert!(!clean.contains("<<<<<<<") && clean.contains("\"base_version\": 3,"), "{clean}");
    assert!(annotary_sidecar::parse(&clean).is_ok());
    let (_, acts) = ts.get(&format!("/api/v1/docs/{id}/actions"), Some(&tok));
    assert_eq!(acts["inherited"], true, "nothing declared: {acts}");

    // pull reads it too: the lower side's base hash decides "local edits".
    let sc = conflict(&clean, "  \"base_version\": 3,", "  \"base_version\": 2,");
    std::fs::write(b.join("plan.annotary"), &sc).unwrap();
    let se = refused(run(&ts, home.path(), b, &tok, &["pull", &id]));
    assert!(se.starts_with("error: plan.md has local edits since v2"), "{se}");
    std::fs::write(b.join("plan.md"), &v2).unwrap(); // = v2: no local edits against the lower base
    ok(run(&ts, home.path(), b, &tok, &["pull", &id]));
    let clean = std::fs::read_to_string(b.join("plan.annotary")).unwrap();
    assert!(!clean.contains("<<<<<<<") && clean.contains("\"base_version\": 3,"), "{clean}");

    // Two different Documents in conflict: keep one side (exit 5).
    let sc = conflict(&clean, &format!("  \"id\": \"{id}\","), "  \"id\": \"p3x8w2ka\",");
    std::fs::write(b.join("plan.annotary"), &sc).unwrap();
    let se = refused(run(&ts, home.path(), b, &tok, &["push", "plan.md"]));
    assert_eq!(se, "error: plan.annotary has conflicting Document ids — keep one side\n");
}

// ---------------------------------------------------------------- real git worktrees

fn git(cwd: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

fn git_ok(cwd: &Path, args: &[&str]) {
    let out = git(cwd, args);
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

// spec: S17-10
#[test]
fn git_worktree_flow() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("git not on PATH — skipping the worktree flow");
        return;
    }
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let main = root.path().join("main");
    std::fs::create_dir(&main).unwrap();
    git_ok(&main, &["init", "-q", "-b", "main"]);
    std::fs::write(main.join("plan.md"), BASE).unwrap();
    ok(run(&ts, home.path(), &main, &tok, &["push", "plan.md"]));
    git_ok(&main, &["add", "."]);
    git_ok(&main, &["commit", "-qm", "v1"]);
    let id = ts.get("/api/v1/docs", Some(&tok)).1["docs"][0]["id"].as_str().unwrap().to_string();
    let (wa, wb) = (root.path().join("wa"), root.path().join("wb"));
    git_ok(&main, &["worktree", "add", "-q", "-b", "a", wa.to_str().unwrap()]);
    git_ok(&main, &["worktree", "add", "-q", "-b", "b", wb.to_str().unwrap()]);

    // B (a human) comments on v1's API line and pulls (its Sidecar now lists that Thread).
    ok(run(&ts, home.path(), &wb, &tok, &["thread", "new", &id, "7:0-7:4", "-m", "REST?"]));
    ok(run(&ts, home.path(), &wb, &tok, &["pull", &id]));

    // A (an agent) edits Storage, pushes v2, comments on its new line, pulls, commits.
    std::fs::write(wa.join("plan.md"), BASE.replace("SQLite, one file.", "SQLite, one file, WAL mode.")).unwrap();
    ok(run(&ts, home.path(), &wa, &agent, &["push", "plan.md", "-m", "wal"]));
    ok(run(&ts, home.path(), &wa, &agent, &["thread", "new", &id, "4:0-4:26", "-m", "why WAL?"]));
    ok(run(&ts, home.path(), &wa, &agent, &["pull", &id]));
    git_ok(&wa, &["commit", "-qam", "storage: WAL"]);

    // B edits Later, commits; its push is refused (it lacks v2).
    std::fs::write(wb.join("plan.md"), BASE.replace("Nothing yet.", "Search, later.")).unwrap();
    git_ok(&wb, &["commit", "-qam", "later: search"]);
    refused(run(&ts, home.path(), &wb, &tok, &["push", "plan.md"]));

    // B merges A's branch: git merges the Markdown; the Sidecar conflicts and is committed as is.
    let out = git(&wb, &["merge", "-q", "a", "-m", "merge a"]);
    assert!(!out.status.success(), "the Sidecars should conflict: {}", String::from_utf8_lossy(&out.stdout));
    let md = std::fs::read_to_string(wb.join("plan.md")).unwrap();
    assert!(md.contains("WAL mode") && md.contains("Search, later.") && !md.contains("<<<<<<<"), "{md}");
    let sc = std::fs::read_to_string(wb.join("plan.annotary")).unwrap();
    assert!(sc.contains("<<<<<<<") && sc.contains(">>>>>>>"), "{sc}");
    git_ok(&wb, &["commit", "-qam", "merge a (Sidecar left in conflict)"]);

    // B pushes: the head (v2) is contained — v3 holds both edits, every Thread stays anchored,
    // and the Sidecar comes back clean.
    let (so, se) = ok(run(&ts, home.path(), &wb, &tok, &["push", "plan.md"]));
    assert!(so.trim_end().ends_with("\tv3"), "{so}");
    assert!(!se.contains("warning"), "{se}");
    let (n, text) = head(&ts, &id);
    assert_eq!(n, 3);
    assert!(text.contains("SQLite, one file, WAL mode.") && text.contains("Search, later."), "{text}");
    assert_eq!(statuses(&ts, &id), vec![("t1".into(), "anchored".into()), ("t2".into(), "anchored".into())]);
    let sc = std::fs::read_to_string(wb.join("plan.annotary")).unwrap();
    assert!(!sc.contains("<<<<<<<") && sc.contains("\"base_version\": 3,"), "{sc}");
}
