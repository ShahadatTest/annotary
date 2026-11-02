//! The real `annotary wait` binary (ticket 08): it blocks and wakes when a matching event
//! lands, times out with exit 7 naming the cursor, and — against a real spawned `serve` — exits 6
//! when the server is SIGKILLed mid-wait, after which the same `--cursor` on the restarted server
//! resumes with no gap and no repeat.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use annotary::testing::{TEST_PASSWORD, TestServer, probe_root};
use common::serve::ServeGuard;
use serde_json::json;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_annotary"))
}

fn cli(home: &Path, server: &str, token: &str) -> Command {
    let mut c = Command::new(bin());
    c.env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", server)
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY")
        .stdin(Stdio::null());
    c
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

/// Wait for a child to exit, at most `max`; panics if it does not.
fn finish(mut child: Child, max: Duration) -> Output {
    let deadline = Instant::now() + max;
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let out = child.wait_with_output().unwrap();
            panic!("child did not exit within {max:?}; stdout={:?} stderr={:?}", text(&out).0, text(&out).1);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A spawned `wait` whose stderr is read live: on a TTY (`ANNOTARY_TTY=1`) the verb prints
/// `wait: watching …` right before its first long-poll, which is how a test knows the child is
/// parked before it mutates anything (the first request of a fresh server can take most of a
/// second — a fixed sleep is not enough).
struct Waiting {
    child: Child,
    stderr: std::sync::mpsc::Receiver<String>,
    seen: Vec<String>,
}

fn spawn_wait(cmd: &mut Command) -> Waiting {
    let mut child = cmd.env("ANNOTARY_TTY", "1").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let err = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(err);
        loop {
            let mut line = String::new();
            match std::io::BufRead::read_line(&mut reader, &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Waiting { child, stderr: rx, seen: Vec::new() }
}

impl Waiting {
    /// Block until the child announced its first long-poll (then a beat, for the request to
    /// reach the server).
    fn parked(&mut self) {
        let line = self.stderr.recv_timeout(Duration::from_secs(10)).expect("the `wait: watching` note");
        assert!(line.starts_with("wait: watching "), "{line:?}");
        self.seen.push(line);
        std::thread::sleep(Duration::from_millis(200));
    }

    fn alive(&self) -> bool {
        child_alive(&self.child)
    }

    /// Wait for the exit (≤ `max`): (code, stdout, stderr after the note).
    fn finish(mut self, max: Duration) -> (Option<i32>, String, String) {
        let out = finish(self.child, max);
        while let Ok(l) = self.stderr.recv_timeout(Duration::from_millis(200)) {
            self.seen.push(l);
        }
        let rest: String = self.seen.iter().skip(1).cloned().collect();
        (out.status.code(), text(&out).0, rest)
    }
}

fn upload(url: &str, token: &str, md: &[u8]) -> String {
    let hash = annotary_sidecar::identity::sha256_hex(md);
    let res = ureq::put(format!("{url}/api/v1/blobs/{hash}"))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .send(md)
        .unwrap();
    assert!(res.status().is_success());
    hash
}

// spec: S08-05, S08-06, S08-07, S08-08
#[test]
fn wait_blocks_then_prints_what_woke_it_and_times_out_with_the_cursor() {
    // Locked: the timeout names the head, which a loopback browser sign-in would move (CMD-19).
    let ts = TestServer::start_locked();
    let home = tempfile::tempdir().unwrap();
    let tok = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let hash = upload(&ts.url, &tok, b"# Plan\n\nWe ship the CLI first.\n");
    let (status, doc) =
        ts.post("/api/v1/docs", Some(&tok), &json!({ "vault": "notes", "name": "plan", "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let (status, t) =
        ts.post(&format!("/api/v1/docs/{id}/threads"), Some(&tok), &json!({ "subject": "document", "body": "root" }));
    assert_eq!(status, 201, "{t}");

    // A parked `wait --on action,mention` ignores a reply, wakes on the Action, prints it
    // (TTY shape: two-space fields), exits 0.
    let mut w = spawn_wait(cli(home.path(), &ts.url, &agent).args([
        "wait",
        "plan",
        "--on",
        "action,mention",
        "--timeout",
        "20s",
    ]));
    w.parked();
    // A port watcher probing the server meanwhile (CMD-19) changes nothing in the store.
    probe_root(&ts.url);
    let (status, _) =
        ts.post(&format!("/api/v1/docs/{id}/threads/t1/replies"), Some(&tok), &json!({ "body": "no wake" }));
    assert_eq!(status, 201);
    std::thread::sleep(Duration::from_millis(300));
    assert!(w.alive(), "a reply must not wake --on action,mention");
    let t0 = Instant::now();
    let (status, a) = ts.post(&format!("/api/v1/docs/{id}/actions"), Some(&tok), &json!({ "action": "approve" }));
    assert_eq!(status, 201, "{a}");
    let (code, stdout, stderr) = w.finish(Duration::from_secs(10));
    let woke = t0.elapsed();
    assert!(woke < Duration::from_secs(5), "woke after {woke:?}");
    assert_eq!(code, Some(0), "{stderr}");
    let seq = ts.store.last_seq().unwrap();
    assert_eq!(stdout, format!("{seq}  action  plan  approve v1 by @rowan\n"));
    assert_eq!(stderr, "");

    // A mention wakes the mentioned Member only; `--json` prints one object per line.
    let mut w = spawn_wait(cli(home.path(), &ts.url, &agent).args(["wait", "plan", "--json", "--timeout", "20s"]));
    let mut other =
        spawn_wait(cli(home.path(), &ts.url, &tok).args(["wait", "plan", "--on", "mention", "--timeout", "2s"]));
    w.parked();
    other.parked();
    probe_root(&ts.url);
    let (status, r) = ts.post(
        &format!("/api/v1/docs/{id}/threads/t1/replies"),
        Some(&tok),
        &json!({ "body": "Over to you @scribe — the order list needs the skill file." }),
    );
    assert_eq!(status, 201, "{r}");
    let (code, stdout, stderr) = w.finish(Duration::from_secs(10));
    assert_eq!(code, Some(0), "{stderr}");
    let seq = ts.store.last_seq().unwrap();
    let line: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(line["seq"], seq);
    assert_eq!(line["kind"], "mention");
    assert_eq!(line["doc"], id);
    assert_eq!(line["name"], "plan");
    assert_eq!(line["comment"], "t1.2");
    assert_eq!(line["by"], "rowan");
    assert_eq!(line["excerpt"], "Over to you @scribe — the order list needs the skill file.");
    assert!(line["at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(stdout.lines().count(), 1);
    probe_root(&ts.url);
    // The owner was not mentioned: its wait times out, naming the cursor it reached.
    let (code, stdout, stderr) = other.finish(Duration::from_secs(10));
    assert_eq!(code, Some(7), "{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, format!("wait: nothing after 2s (seq {seq})\n"));

    // Resuming from an old cursor prints everything that arrived since, in one go.
    let out = cli(home.path(), &ts.url, &agent)
        .args(["wait", "plan", "--cursor", "0", "--on", "version,action,mention,thread,reply,resolve,reopen"])
        .output()
        .unwrap();
    let (stdout, stderr) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let kinds: Vec<&str> = stdout.lines().map(|l| l.split('\t').nth(1).unwrap()).collect();
    assert_eq!(kinds, ["version", "thread", "reply", "action", "mention"]);
    assert!(stdout.lines().all(|l| l.split('\t').nth(2) == Some("plan")), "{stdout}");
    // `--json` timeout carries `seq` as a number.
    let out = cli(home.path(), &ts.url, &agent).args(["wait", "plan", "--timeout", "1s", "--json"]).output().unwrap();
    assert_eq!(out.status.code(), Some(7));
    let err: serde_json::Value = serde_json::from_str(text(&out).1.trim()).unwrap();
    assert_eq!(
        err,
        json!({"error": "wait_timeout", "message": format!("wait: nothing after 1s (seq {seq})"), "seq": seq})
    );
}

fn child_alive(child: &Child) -> bool {
    // SAFETY: signal 0 only checks for existence.
    unsafe { libc::kill(child.id() as libc::pid_t, 0) == 0 }
}

fn token_from(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("ANNOTARY_TOKEN=").map(|t| t.trim().to_string()))
        .expect("a ANNOTARY_TOKEN= line")
}

/// Spawn a real `serve` on `port` (0 = ephemeral) for `home`; returns its guard and its URL.
fn spawn_serve(home: &Path, port: u16) -> (ServeGuard, String) {
    let child = Command::new(bin())
        .args(["serve", "--port", &port.to_string()])
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let guard = ServeGuard::new(child);
    let sj = home.join(".annotary/server.json");
    let deadline = Instant::now() + Duration::from_secs(15);
    let url = loop {
        if let Some(info) = annotary::server::ServerJson::read(&sj)
            && info.pid == pid
            && ureq::get(format!("{}/api/v1/health", info.url)).call().is_ok()
        {
            break info.url;
        }
        assert!(Instant::now() < deadline, "server.json never named pid {pid}");
        std::thread::sleep(Duration::from_millis(20));
    };
    (guard, url)
}

// spec: S08-07
#[test]
fn sigkill_serve_mid_wait_then_restart_resumes_from_the_same_cursor() {
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    let out = Command::new(bin())
        .args(["serve", "--setup-only"])
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .env("ANNOTARY_SETUP_NAME", "Rowan Vale")
        .env("ANNOTARY_SETUP_PASSWORD", TEST_PASSWORD)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out).1);
    let tok = token_from(&text(&out).0);
    let (mut serve, url) = spawn_serve(home, 0);
    let port: u16 = url.rsplit(':').next().unwrap().parse().unwrap();

    // A Document, a Thread, and the cursor an agent would hold.
    let hash = upload(&url, &tok, b"# Plan\n");
    let res = ureq::post(format!("{url}/api/v1/docs"))
        .header("Authorization", format!("Bearer {tok}"))
        .send_json(json!({ "vault": "notes", "name": "plan", "md_hash": hash }))
        .unwrap();
    assert_eq!(res.status(), 201);
    let out = cli(home, &url, &tok).args(["thread", "new", "plan", "--document", "-m", "root"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let out = cli(home, &url, &tok).args(["status", "plan", "--json", "seq"]).output().unwrap();
    let cursor = serde_json::from_str::<serde_json::Value>(text(&out).0.trim()).unwrap()["seq"].as_i64().unwrap();

    // The agent waits for an Action; a reply lands meanwhile (not a wake); then the server dies.
    let mut w = spawn_wait(cli(home, &url, &tok).args([
        "wait",
        "plan",
        "--cursor",
        &cursor.to_string(),
        "--on",
        "action",
        "--timeout",
        "30s",
    ]));
    w.parked();
    probe_root(&url);
    let out = cli(home, &url, &tok).args(["thread", "reply", "plan", "t1", "-m", "before the crash"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    std::thread::sleep(Duration::from_millis(200));
    assert!(w.alive());
    serve.kill(); // SIGKILL
    let (code, stdout, stderr) = w.finish(Duration::from_secs(10));
    assert_eq!(code, Some(6), "stdout={stdout} stderr={stderr}");
    assert_eq!(stdout, "");
    assert!(stderr.starts_with(&format!("error: server at {url} not reachable — run `annotary serve`")), "{stderr}");
    assert!(stderr.contains(&format!("resume with --cursor {cursor}")), "{stderr}");

    // Restart on the same port (the store LOCK died with the process); more happens.
    let (mut serve2, url2) = spawn_serve(home, port);
    assert_eq!(url2, url);
    probe_root(&url);
    let out = cli(home, &url, &tok).args(["act", "plan", "approve"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let out =
        cli(home, &url, &tok).args(["thread", "reply", "plan", "t1", "-m", "after the restart"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);

    // The same cursor resumes: the reply from before the crash, the Action, the reply after —
    // once each, in order, and nothing before the cursor.
    let out = cli(home, &url, &tok)
        .args(["wait", "plan", "--cursor", &cursor.to_string(), "--on", "reply,action", "--timeout", "5s"])
        .output()
        .unwrap();
    let (stdout, stderr) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let rows: Vec<Vec<&str>> = stdout.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), 3, "{stdout}");
    let seqs: Vec<i64> = rows.iter().map(|r| r[0].parse().unwrap()).collect();
    assert_eq!(seqs, [cursor + 1, cursor + 2, cursor + 3], "{stdout}");
    assert_eq!(rows[0][1..], ["reply", "plan", "t1.1 by @rowan \"before the crash\""]);
    assert_eq!(rows[1][1..], ["action", "plan", "approve v1 by @rowan"]);
    assert_eq!(rows[2][1..], ["reply", "plan", "t1.2 by @rowan \"after the restart\""]);
    // Continuing from the last printed seq repeats nothing.
    let out = cli(home, &url, &tok)
        .args(["wait", "plan", "--cursor", &(cursor + 3).to_string(), "--on", "reply,action", "--timeout", "1s"])
        .output()
        .unwrap();
    let (stdout, stderr) = text(&out);
    assert_eq!(out.status.code(), Some(7), "{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, format!("wait: nothing after 1s (seq {})\n", cursor + 3));
    // And `status` agrees on the head.
    let out = cli(home, &url, &tok).args(["status", "plan"]).output().unwrap();
    assert!(text(&out).0.trim_end().ends_with(&format!("seq:{}", cursor + 3)), "{}", text(&out).0);

    serve2.kill();
}

// spec: S08-11
#[test]
fn sigterm_with_sse_and_long_poll_attached_exits_promptly_and_removes_server_json() {
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path();
    let out = Command::new(bin())
        .args(["serve", "--setup-only"])
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .env("ANNOTARY_SETUP_PASSWORD", TEST_PASSWORD)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out).1);
    let tok = token_from(&text(&out).0);
    let (mut serve, url) = spawn_serve(home, 0);
    let server_json = home.join(".annotary/server.json");
    assert!(server_json.exists());
    let head: i64 = {
        let mut r =
            ureq::get(format!("{url}/api/v1/events")).header("Authorization", format!("Bearer {tok}")).call().unwrap();
        r.body_mut().read_json::<serde_json::Value>().unwrap()["seq"].as_i64().unwrap()
    };

    // One SSE stream and one parked long-poll, both held open by clients that will not hang up.
    let (sse_tx, sse_rx) = std::sync::mpsc::channel::<(bool, Vec<String>)>();
    let (lp_tx, lp_rx) = std::sync::mpsc::channel::<(u16, serde_json::Value)>();
    let (url_s, tok_s) = (url.clone(), tok.clone());
    let sse = std::thread::spawn(move || {
        let mut res = ureq::get(format!("{url_s}/api/v1/events"))
            .header("Authorization", format!("Bearer {tok_s}"))
            .header("Accept", "text/event-stream")
            .config()
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .call()
            .unwrap();
        let mut lines = Vec::new();
        let mut reader = std::io::BufReader::new(res.body_mut().as_reader());
        let ended_cleanly = loop {
            let mut line = String::new();
            match std::io::BufRead::read_line(&mut reader, &mut line) {
                Ok(0) => break true,
                Ok(_) => lines.push(line.trim_end().to_string()),
                Err(_) => break false,
            }
        };
        let _ = sse_tx.send((ended_cleanly, lines));
    });
    let (url_l, tok_l) = (url.clone(), tok.clone());
    let long_poll = std::thread::spawn(move || {
        let res = ureq::get(format!("{url_l}/api/v1/events?wait=30&since={head}"))
            .header("Authorization", format!("Bearer {tok_l}"))
            .config()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .call();
        let out = match res {
            Ok(mut r) => (r.status().as_u16(), r.body_mut().read_json::<serde_json::Value>().unwrap_or_default()),
            Err(e) => (0, serde_json::Value::String(e.to_string())),
        };
        let _ = lp_tx.send(out);
    });
    // Give both requests time to be parked server-side; a port watcher probes meanwhile.
    std::thread::sleep(Duration::from_millis(350));
    probe_root(&url);
    std::thread::sleep(Duration::from_millis(350));
    assert!(sse_rx.try_recv().is_err(), "the SSE stream must still be open before SIGTERM");
    assert!(lp_rx.try_recv().is_err(), "the long-poll must still be parked before SIGTERM");

    // SIGTERM: the listener closes, every parked request lets go, the process exits, server.json goes.
    let t0 = Instant::now();
    serve.sigterm();
    let out = serve.wait_timeout(Duration::from_secs(5));
    let took = t0.elapsed();
    assert!(out.success(), "serve exits 0 on SIGTERM: {out}");
    assert!(took < Duration::from_secs(5), "exited after {took:?}");
    assert!(!server_json.exists(), "server.json removed on clean exit");
    // The long-poll answered with an empty page and its cursor (the client resumes from it);
    // the SSE stream ended cleanly after its `ready` frame.
    let (status, body) = lp_rx.recv_timeout(Duration::from_secs(5)).expect("long-poll answered");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "events": [], "seq": head }));
    let (clean, lines) = sse_rx.recv_timeout(Duration::from_secs(5)).expect("SSE ended");
    assert!(clean, "SSE ended with EOF, not an error: {lines:?}");
    assert!(lines.first().is_some_and(|l| l == "event: ready"), "{lines:?}");
    long_poll.join().unwrap();
    sse.join().unwrap();
    // The store is free again: another serve on the same dir starts.
    let (mut again, _) = spawn_serve(home, 0);
    again.kill();
}
