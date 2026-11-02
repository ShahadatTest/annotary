//! The real binary against the in-process server for what trycmd cannot drive: `-m -` (stdin),
//! `thread new` by path (id from the sibling Sidecar) and the pull → push round trip that ignores
//! hand-edited Threads in the Sidecar.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use annotary::testing::TestServer;

fn cmd(ts: &TestServer, home: &Path, cwd: &Path, token: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_annotary"));
    c.args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY");
    c
}

fn run(ts: &TestServer, home: &Path, cwd: &Path, token: &str, args: &[&str]) -> Output {
    cmd(ts, home, cwd, token, args).output().unwrap()
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

// spec: S04-03, S04-07, S04-14, S04-16
#[test]
fn stdin_messages_paths_and_the_sidecar_round_trip() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    std::fs::write(work.path().join("plan.md"), "# Plan\n\nWe ship the CLI first.\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);

    // DOC by path: the id comes from the sibling Sidecar (written by push).
    let out =
        run(&ts, home.path(), work.path(), &tok, &["thread", "new", "plan.md", "3:0-3:7", "-m", "Do we? @scribe"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, "t1\tv1:3:0-3:7 \"We ship\"\n");

    // `-m -` reads the body from stdin (verbatim inside; trailing newlines dropped).
    let mut child = cmd(&ts, home.path(), work.path(), &agent, &["thread", "reply", "plan.md", "t1", "-m", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("Yes.\n\nSee `@rowan` (not a mention) — @rowan (a mention)\n\n".as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, "t1.1\n");
    let out = run(&ts, home.path(), work.path(), &tok, &["thread", "show", "plan.md", "t1"]);
    let (so, _) = text(&out);
    assert!(so.contains("t1.1\t@scribe\tv1 "), "{so}");
    assert!(
        so.contains("\tmentions @rowan\n  Yes.\n  \n  See `@rowan` (not a mention) — @rowan (a mention)\n"),
        "{so}"
    );
    assert!(!so.ends_with("\n  \n"), "trailing newlines dropped: {so}");
    // An empty stdin is a usage error; a Comment edited via stdin too.
    let mut child = cmd(&ts, home.path(), work.path(), &agent, &["thread", "reply", "plan.md", "t1", "-m", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out).1.starts_with("error: empty message"));

    // TTY parity for `thread show`: same fields, two-space separated.
    let out = cmd(&ts, home.path(), work.path(), &tok, &["thread", "show", "plan.md", "t1", "--no-color"])
        .env("ANNOTARY_TTY", "1")
        .output()
        .unwrap();
    let (so, _) = text(&out);
    assert!(so.starts_with("t1  open  @rowan  v1:3:0-3:7 \"We ship\"  created v1 "), "{so}");

    // pull writes the Threads into the Sidecar; hand-editing them changes nothing push reads.
    let out = run(&ts, home.path(), work.path(), &tok, &["pull", "plan", "--out", "pulled"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).1, "pull: 1 thread, 0 assets\n");
    let sc_path = work.path().join("pulled/plan.annotary");
    let sc = std::fs::read_to_string(&sc_path).unwrap();
    assert!(sc.contains("\"threads\": [") && sc.contains("\"id\": \"t1.1\""), "{sc}");
    let parsed = annotary_sidecar::parse(&sc).unwrap();
    assert_eq!(annotary_sidecar::emit(&parsed), sc, "canonical on disk");
    std::fs::write(&sc_path, sc.replace("\"body\": \"Do we? @scribe\"", "\"body\": \"HAND EDITED\"")).unwrap();
    std::fs::write(work.path().join("pulled/plan.md"), "# Plan\n\nWe ship the CLI first.\n\nMore.\n").unwrap();
    let out = run(&ts, home.path(), work.path(), &tok, &["push", "pulled/plan.md", "-m", "v2"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.ends_with("\tv2\n"), "{}", text(&out).0);
    let sc = std::fs::read_to_string(&sc_path).unwrap();
    assert!(sc.contains("\"body\": \"Do we? @scribe\""), "regenerated whole from the server: {sc}");
    assert!(sc.contains("\"base_version\": 2"));
    // The push re-anchored the Thread onto v2 (ticket 07): same range, still anchored, stamped v2.
    let out =
        run(&ts, home.path(), work.path(), &tok, &["thread", "list", "pulled/plan.md", "--json", "id,anchor,version"]);
    assert_eq!(
        text(&out).0.trim(),
        "[{\"id\":\"t1\",\"anchor\":{\"version\":2,\"range\":\"3:0-3:7\",\"status\":\"anchored\",\"exact\":\"We ship\"},\"version\":1}]"
    );
}
