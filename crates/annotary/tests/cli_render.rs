//! The real binary against the in-process server (ticket 09): `push` renders at the edge and
//! uploads the HTML blob with the commit (the server never renders on push); the served HTML is
//! the client's; `rerender DOC` / `--all` re-render in place with the server's crate and report
//! `name id versions changed`; exit codes.

use std::path::Path;
use std::process::{Command, Output};

use annotary::testing::TestServer;
use annotary_renderer::{Asset, RENDERER_VERSION, render_html};
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::Value;

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

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets").join(name)).unwrap()
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn get_html(ts: &TestServer, token: &str, id: &str, n: u32) -> (u16, String) {
    let mut r = ureq::get(format!("{}/api/v1/docs/{id}/versions/{n}/html", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap_or_default())
}

const MD: &str = "---\ntitle: Rendered\n---\n\n# Plan\n\n![a](img/a.png) ![bmp](img/x.bmp) ![gone](img/gone.png) [notes](notes.md)\n\n```mermaid\ngraph TD; A-->B;\n```\n\nMath $x^2$ and a table:\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";

// spec: S09-08, S09-09, S09-10, S09-06
#[test]
fn push_renders_at_the_edge_and_the_server_serves_that_html_unchanged() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "plan.md", MD.as_bytes());
    write(root, "img/a.png", &fixture("a.png"));
    write(root, "img/x.bmp", &fixture("a.bmp"));
    write(root, "notes.md", b"# Notes\n\nlinked\n");
    let seq0 = ts.store.last_seq().unwrap();
    let out = run(&ts, home.path(), root, &ts.owner_token, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let (so, se) = text(&out);
    assert!(so.starts_with("plan\t"), "{so}");
    assert!(se.contains("warning: missing asset img/gone.png"), "{se}");
    let id = so.split('\t').nth(1).unwrap().to_string();

    // The Version records the client's render + renderer version; the served HTML is that blob.
    let v1 = ts.store.version(&id, 1).unwrap().unwrap();
    assert_eq!(v1.renderer.as_deref(), Some(RENDERER_VERSION));
    let expected = render_html(MD, &Asset::from_manifest(&v1.manifest));
    let expected_hash = sha256_hex(expected.as_bytes());
    assert_eq!(v1.html_hash.as_deref(), Some(expected_hash.as_str()));
    assert!(ts.store.has_blob(&expected_hash), "the html blob was uploaded in the two-phase flow");
    let (status, html) = get_html(&ts, &ts.owner_token, &id, 1);
    assert_eq!(status, 200);
    assert_eq!(html, expected);
    // What the viewer relies on, from real manifest data.
    let a_hash = sha256_hex(&fixture("a.png"));
    let bmp_hash = sha256_hex(&fixture("a.bmp"));
    assert!(html.contains(&format!("<img src=\"/a/{a_hash}\" alt=\"a\" width=\"32\" height=\"20\">")), "{html}");
    assert!(html.contains(&format!("<img src=\"/a/{bmp_hash}?derivative=1\" alt=\"bmp\" width=")), "{html}");
    assert!(
        html.contains(
            "<span class=\"asset-missing\" data-ref=\"img/gone.png\">missing asset: img/gone.png (gone)</span>"
        ),
        "{html}"
    );
    assert!(html.contains(&format!("<a href=\"/a/{}\">notes</a>", sha256_hex(b"# Notes\n\nlinked\n"))), "{html}");
    assert!(html.contains("<pre data-l=\"9-11\" class=\"mermaid\">"), "{html}");
    assert!(html.contains("<math ") && html.contains("<table data-l="), "{html}");
    assert!(!html.contains("Rendered"), "front matter is not rendered");
    // The server rendered nothing: no version.rendered event on push or on read.
    let events = ts.store.events_since(seq0).unwrap();
    assert!(events.iter().all(|e| e.kind != "version.rendered"));

    // Re-push identical: `already`, nothing uploaded, still the same html.
    let out = run(&ts, home.path(), root, &ts.owner_token, &["push", "plan.md"]);
    assert!(text(&out).0.contains("already v1"), "{}", text(&out).0);
    // A new Version renders again; the html blob of v1 stays referenced.
    write(root, "plan.md", format!("{MD}\nMore.\n").as_bytes());
    let out = run(&ts, home.path(), root, &ts.owner_token, &["push", "plan.md"]);
    assert!(text(&out).0.contains("\tv2"), "{}", text(&out).0);
    let v2 = ts.store.version(&id, 2).unwrap().unwrap();
    assert_ne!(v2.html_hash, v1.html_hash);
    let removed = ts.store.gc_blobs(std::time::Duration::ZERO).unwrap();
    assert!(removed.is_empty(), "{removed:?}");
}

// spec: S09-11, S09-10
#[test]
fn rerender_reports_versions_and_changes_and_the_server_render_matches_the_clients() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "plan.md", MD.as_bytes());
    write(root, "img/a.png", &fixture("a.png"));
    write(root, "img/x.bmp", &fixture("a.bmp"));
    write(root, "notes.md", b"# Notes\n");
    let out = run(&ts, home.path(), root, &ts.owner_token, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let id = text(&out).0.split('\t').nth(1).unwrap().to_string();
    write(root, "plan.md", format!("{MD}\nv2\n").as_bytes());
    let out = run(&ts, home.path(), root, &ts.owner_token, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);

    // The server's crate renders exactly what the CLI pushed: nothing changes.
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "plan"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let (so, se) = text(&out);
    assert_eq!(so, format!("plan\t{id}\t2\t0\n"));
    assert_eq!(se.trim(), format!("rerender: 2 Versions, 0 changed (renderer {RENDERER_VERSION})"));
    // TTY: same fields, aligned, dim header.
    let out = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["rerender", "plan.md", "--no-color"])
        .current_dir(root)
        .env("HOME", home.path())
        .env("ANNOTARY_HOME", home.path().join(".annotary"))
        .env("ANNOTARY_TOKEN", &ts.owner_token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env("ANNOTARY_TTY", "1")
        .output()
        .unwrap();
    let (so, _) = text(&out);
    let lines: Vec<&str> = so.lines().collect();
    assert_eq!(lines[0].split_whitespace().collect::<Vec<_>>(), vec!["NAME", "ID", "VERSIONS", "CHANGED"]);
    assert_eq!(lines[1].split_whitespace().collect::<Vec<_>>(), vec!["plan", id.as_str(), "2", "0"]);
    // --json bare / narrowed / ?
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", &id, "--json"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v, serde_json::json!({ "name": "plan", "id": id, "versions": 2, "changed": 0 }));
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", &id, "--json", "renderer,vault"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v, serde_json::json!({ "renderer": RENDERER_VERSION, "vault": "notes" }));
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "--all", "--json", "?"]);
    assert_eq!(text(&out).0.trim(), "name id versions changed vault renderer");

    // Simulate an older render on v1 (a stale blob + old renderer): rerender replaces it.
    let (stale, _) = ts.store.put_blob(b"<p>stale</p>\n").unwrap();
    ts.store.set_version_html(&ts.owner_id, &id, 1, &stale, "0.0.1").unwrap();
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "plan"]);
    assert_eq!(text(&out).0, format!("plan\t{id}\t2\t1\n"));
    let v1 = ts.store.version(&id, 1).unwrap().unwrap();
    assert_eq!(v1.renderer.as_deref(), Some(RENDERER_VERSION));
    let expected = render_html(MD, &Asset::from_manifest(&v1.manifest));
    assert_eq!(v1.html_hash.as_deref(), Some(sha256_hex(expected.as_bytes()).as_str()));
    let (_, html) = get_html(&ts, &ts.owner_token, &id, 1);
    assert_eq!(html, expected);

    // --all (owner): every Document, deleted ones too; a Version pushed by an old client
    // without a render gets one.
    let (md, _) = ts.store.put_blob(b"# old client\n").unwrap();
    let vault = ts.store.vault_by_name("notes").unwrap().unwrap();
    let old = ts.store.create_document(&ts.owner_id, &vault.id, annotary_store::PushInput::new("old", md)).unwrap();
    ts.store.delete_document(&ts.owner_id, &old.id).unwrap();
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "--all"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let (so, se) = text(&out);
    let mut rows: Vec<&str> = so.lines().collect();
    rows.sort();
    assert_eq!(rows, vec![format!("old\t{}\t1\t1", old.id), format!("plan\t{id}\t2\t0")]);
    assert_eq!(se.trim(), format!("rerender: 2 Documents, 3 Versions, 1 changed (renderer {RENDERER_VERSION})"));
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "--all", "--json"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert!(ts.store.version(&old.id, 1).unwrap().unwrap().html_hash.is_some());

    // Exit codes: usage (neither / both), not found, missing right, owner-only.
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender"]);
    assert_eq!(out.status.code(), Some(1));
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "plan", "--all"]);
    assert_eq!(out.status.code(), Some(1));
    let out = run(&ts, home.path(), root, &ts.owner_token, &["rerender", "nope"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out).1);
    let (reader_id, reader_tok) = ts.add_member("reader");
    ts.store
        .set_grants(&ts.owner_id, &reader_id, vec![Grant { vault: "*".into(), rights: vec![Right::Read] }])
        .unwrap();
    let out = run(&ts, home.path(), root, &reader_tok, &["rerender", "plan"]);
    assert_eq!(out.status.code(), Some(4), "{}", text(&out).1);
    assert!(text(&out).1.contains("missing right `write`"), "{}", text(&out).1);
    let (_, agent_tok) = ts.add_member("agent");
    let out = run(&ts, home.path(), root, &agent_tok, &["rerender", "--all"]);
    assert_eq!(out.status.code(), Some(4), "{}", text(&out).1);
    assert!(text(&out).1.contains("owner-only"), "{}", text(&out).1);
    let out = run(&ts, home.path(), root, &agent_tok, &["rerender", "plan"]);
    assert_eq!(out.status.code(), Some(0), "an agent with write may rerender one Document: {}", text(&out).1);
}
