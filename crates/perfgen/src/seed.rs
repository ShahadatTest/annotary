//! Seeding a running server with a [`Plan`] through the real path: the real `annotary`
//! binary for `member add` and every `push` (the edge computation — hashing, Asset scan, render,
//! re-anchoring — runs exactly as for a user), and the same `/api/v1` the CLI uses for Threads,
//! Replies, resolves, reopens, tombstones and Actions. Afterwards the server's own view of the
//! Document (Thread count, placement statuses, head) is read back into [`Seeded`].

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

use crate::plan::{Plan, Step, Subject, Who};

/// Where to seed: the binary under test, the server, the owner's Token, a HOME for the CLI and a
/// working directory for the Document's files.
#[derive(Clone, Debug)]
pub struct Target {
    pub bin: PathBuf,
    pub server: String,
    pub owner_token: String,
    /// `HOME` / `ANNOTARY_HOME` for the CLI (never the developer's).
    pub home: PathBuf,
    /// The directory the Markdown + images are written to and pushed from.
    pub work: PathBuf,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Statuses {
    pub anchored: usize,
    pub refined: usize,
    pub orphaned: usize,
    /// Document-level Threads (no placement).
    pub document: usize,
}

/// What the seed left on the server.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Seeded {
    pub server: String,
    pub doc_id: String,
    pub name: String,
    pub head: u32,
    pub threads: usize,
    /// `thread list --status …` counts over every Thread (resolved and hidden included).
    pub statuses: Statuses,
    pub resolved: usize,
    pub replies: usize,
    pub list_ids: Vec<String>,
    /// Handle → Token for every Member the plan posts as (the owner first).
    pub tokens: Vec<(String, String)>,
    pub pushes_ms: Vec<u128>,
    pub elapsed_ms: u128,
    pub stats: crate::plan::Stats,
}

#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    #[error("{0}")]
    Cli(String),
    #[error("HTTP {status} on {what}: {body}")]
    Http { what: String, status: u16, body: String },
    #[error("{0}")]
    Other(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The `/api/v1` client. Every request gets its own connection (no idle pool, `Connection:
/// close`): a kept-alive socket the server or kernel dropped between two seed requests failed
/// the next one (macOS: `io: Invalid argument (os error 22)`, CMD-10). A transport failure is
/// retried, so a request is tried at most [`Http::ATTEMPTS`] times; an HTTP status never is.
struct Http {
    agent: ureq::Agent,
    server: String,
}

impl Http {
    const ATTEMPTS: u32 = 3;

    fn new(server: &str) -> Http {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            .timeout_connect(Some(std::time::Duration::from_secs(5)))
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .build();
        Http { agent: config.new_agent(), server: server.trim_end_matches('/').to_string() }
    }

    fn post(&self, token: &str, path: &str, body: &Value) -> Result<Value, SeedError> {
        self.send(path, || {
            self.agent
                .post(format!("{}{path}", self.server))
                .header("Authorization", format!("Bearer {token}"))
                .header("Connection", "close")
                .send_json(body)
        })
    }

    fn get(&self, token: &str, path: &str) -> Result<Value, SeedError> {
        self.send(path, || {
            self.agent
                .get(format!("{}{path}", self.server))
                .header("Authorization", format!("Bearer {token}"))
                .header("Connection", "close")
                .call()
        })
    }

    fn delete(&self, token: &str, path: &str) -> Result<Value, SeedError> {
        self.send(path, || {
            self.agent
                .delete(format!("{}{path}", self.server))
                .header("Authorization", format!("Bearer {token}"))
                .header("Connection", "close")
                .call()
        })
    }

    fn send(
        &self,
        what: &str,
        request: impl Fn() -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Value, SeedError> {
        let mut attempt = 1;
        let (status, text) = loop {
            let res = request().and_then(|mut resp| {
                let status = resp.status().as_u16();
                Ok((status, resp.body_mut().read_to_string()?))
            });
            match res {
                Err(ureq::Error::Io(_)) if attempt < Self::ATTEMPTS => {
                    std::thread::sleep(std::time::Duration::from_millis(50 * u64::from(attempt)));
                    attempt += 1;
                }
                Err(e) => return Err(SeedError::Other(format!("{what}: {e}"))),
                Ok(ok) => break ok,
            }
        };
        if !(200..300).contains(&status) {
            return Err(SeedError::Http { what: what.to_string(), status, body: text });
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

fn cli(t: &Target, token: &str, cwd: &Path, args: &[&str]) -> Result<String, SeedError> {
    let out = Command::new(&t.bin)
        .args(args)
        .current_dir(cwd)
        .env("HOME", &t.home)
        .env("ANNOTARY_HOME", t.home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &t.server)
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY")
        .env_remove("ANNOTARY_PASSWORD")
        .output()?;
    if !out.status.success() {
        return Err(SeedError::Cli(format!(
            "annotary {} exited {:?}: {}{}",
            args.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn token_in(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        l.trim().strip_prefix("ANNOTARY_TOKEN=").map(|t| t.split_whitespace().next().unwrap_or("").to_string())
    })
}

/// `name\tid\tvN…` → (id, N).
fn parse_push(out: &str, name: &str) -> Option<(String, u32)> {
    out.lines().find_map(|l| {
        let mut f = l.split('\t');
        if f.next()? != name {
            return None;
        }
        let id = f.next()?.to_string();
        let v = f.next()?.trim_start_matches('v').parse::<u32>().ok()?;
        Some((id, v))
    })
}

/// Seed `plan` into the server described by `target`. `log` receives one line per phase.
pub fn seed(plan: &Plan, target: &Target, log: &mut dyn FnMut(&str)) -> Result<Seeded, SeedError> {
    let t0 = Instant::now();
    // The CLI runs with `current_dir` set, so the binary path must be absolute.
    let bin = target
        .bin
        .canonicalize()
        .map_err(|e| SeedError::Other(format!("annotary binary {}: {e}", target.bin.display())))?;
    let target = &Target { bin, ..target.clone() };
    std::fs::create_dir_all(&target.work)?;
    std::fs::create_dir_all(target.home.join(".annotary"))?;
    let http = Http::new(&target.server);
    let owner_handle = plan.handles.owner.clone();

    // ---- Members: the two agents ----
    let mut tokens: Vec<(String, String)> = vec![(owner_handle.clone(), target.owner_token.clone())];
    for (i, h) in plan.handles.agents.iter().enumerate() {
        let name = if i == 0 { "Scribe" } else { "Linter" };
        let out = cli(target, &target.owner_token, &target.work, &["member", "add", h, "--name", name])?;
        let tok = token_in(&out).ok_or_else(|| SeedError::Other(format!("member add {h} printed no token: {out}")))?;
        tokens.push((h.clone(), tok));
    }
    let tok = |who: Who| -> &str {
        let h = who.handle(&plan.handles);
        tokens.iter().find(|(k, _)| k == h).map(|(_, t)| t.as_str()).unwrap_or("")
    };
    log(&format!("members: {} (+{} agents)", owner_handle, plan.handles.agents.len()));

    // ---- the files ----
    let md_path = target.work.join(format!("{}.md", plan.name));
    for im in &plan.images {
        let p = target.work.join(&im.reference);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&p, &im.png)?;
    }

    // ---- the steps ----
    let mut doc_id = String::new();
    let mut head = 0u32;
    let mut thread_ids: Vec<String> = Vec::new();
    let mut pushes_ms: Vec<u128> = Vec::new();
    let mut replies = 0usize;
    let md_file = format!("{}.md", plan.name);
    for step in &plan.steps {
        match step {
            Step::Push { version, message } => {
                std::fs::write(&md_path, &plan.versions[(*version - 1) as usize])?;
                let t = Instant::now();
                let out = cli(target, &target.owner_token, &target.work, &["push", &md_file, "-m", message])?;
                pushes_ms.push(t.elapsed().as_millis());
                let (id, v) = parse_push(&out, &plan.name)
                    .ok_or_else(|| SeedError::Other(format!("push printed no id/version: {out}")))?;
                if *version == 1 {
                    doc_id = id;
                } else if id != doc_id {
                    return Err(SeedError::Other(format!("push v{version} went to {id}, not {doc_id}")));
                }
                head = v;
                log(&format!("pushed v{v} ({} ms)", pushes_ms.last().copied().unwrap_or(0)));
            }
            Step::Thread { by, subject, body } => {
                let payload = match subject {
                    Subject::Span { range, .. } => {
                        json!({ "subject": "span", "range": range, "body": body, "version": head })
                    }
                    Subject::Document => json!({ "subject": "document", "body": body, "version": head }),
                    Subject::Asset { reference } => {
                        json!({ "subject": "asset", "ref": reference, "body": body, "version": head })
                    }
                };
                let r = http.post(tok(*by), &format!("/api/v1/docs/{doc_id}/threads"), &payload)?;
                let id = r["id"].as_str().ok_or_else(|| SeedError::Other(format!("thread: no id in {r}")))?.to_string();
                thread_ids.push(id);
            }
            Step::Reply { thread, by, body, resolve } => {
                let tid = &thread_ids[*thread];
                http.post(
                    tok(*by),
                    &format!("/api/v1/docs/{doc_id}/threads/{tid}/replies"),
                    &json!({ "body": body, "resolve": resolve }),
                )?;
                replies += 1;
            }
            Step::Resolve { thread, by } => {
                let tid = &thread_ids[*thread];
                http.post(tok(*by), &format!("/api/v1/docs/{doc_id}/threads/{tid}/resolve"), &json!({}))?;
            }
            Step::Reopen { thread, by } => {
                let tid = &thread_ids[*thread];
                http.post(tok(*by), &format!("/api/v1/docs/{doc_id}/threads/{tid}/reopen"), &json!({}))?;
            }
            Step::Delete { thread, reply, by } => {
                let tid = &thread_ids[*thread];
                let cid = match reply {
                    Some(n) => format!("{tid}.{n}"),
                    None => tid.clone(),
                };
                http.delete(tok(*by), &format!("/api/v1/docs/{doc_id}/threads/{cid}"))?;
            }
            Step::Act { by, action, version } => {
                http.post(
                    tok(*by),
                    &format!("/api/v1/docs/{doc_id}/actions"),
                    &json!({ "action": action, "version": version }),
                )?;
            }
        }
    }
    log(&format!("threads: {} created, {} replies", thread_ids.len(), replies));

    // ---- the list Documents ----
    let mut list_ids = Vec::new();
    let list_dir = target.work.join("list");
    std::fs::create_dir_all(&list_dir)?;
    for (i, (name, md)) in plan.list_docs.iter().enumerate() {
        std::fs::write(list_dir.join(format!("{name}.md")), md)?;
        let out = cli(target, &target.owner_token, &list_dir, &["push", &format!("{name}.md")])?;
        let (id, _) = parse_push(&out, name).ok_or_else(|| SeedError::Other(format!("push printed no id: {out}")))?;
        if i % 3 == 0 {
            http.post(tok(Who::Agent1), &format!("/api/v1/docs/{id}/threads"), &json!({ "subject": "document", "body": format!("@{owner_handle} please look at this one"), "version": 1 }))?;
        }
        if i % 5 == 0 {
            http.post(
                tok(Who::Owner),
                &format!("/api/v1/docs/{id}/actions"),
                &json!({ "action": "approve", "version": 1 }),
            )?;
        }
        list_ids.push(id);
    }
    if !list_ids.is_empty() {
        log(&format!("list: {} Documents", list_ids.len()));
    }

    // ---- read back ----
    let all = http.get(&target.owner_token, &format!("/api/v1/docs/{doc_id}/threads?status=all&hidden=1"))?;
    let threads = all["threads"].as_array().cloned().unwrap_or_default();
    let mut statuses = Statuses::default();
    let mut resolved = 0usize;
    for t in &threads {
        if !t["resolved"].is_null() {
            resolved += 1;
        }
        match t["anchor"]["current"]["status"].as_str() {
            Some("anchored") => statuses.anchored += 1,
            Some("refined") => statuses.refined += 1,
            Some("orphaned") => statuses.orphaned += 1,
            _ => statuses.document += 1,
        }
    }
    log(&format!(
        "placements on v{head}: {} anchored, {} refined, {} orphaned, {} document-level; {} resolved",
        statuses.anchored, statuses.refined, statuses.orphaned, statuses.document, resolved
    ));
    Ok(Seeded {
        server: target.server.clone(),
        doc_id,
        name: plan.name.clone(),
        head,
        threads: threads.len(),
        statuses,
        resolved,
        replies,
        list_ids,
        tokens,
        pushes_ms,
        elapsed_ms: t0.elapsed().as_millis(),
        stats: plan.stats.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Reads one HTTP/1.1 request (head + `Content-Length` body) off `r`; false at EOF.
    fn read_request(r: &mut BufReader<TcpStream>) -> bool {
        let mut len = 0usize;
        let mut first = true;
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                return false;
            }
            if line == "\r\n" && !first {
                break;
            }
            first = false;
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                len = v.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; len];
        r.read_exact(&mut body).is_ok()
    }

    /// Waits for the next request's bytes on `s`, then drops `s` with them unread, so the kernel
    /// answers with a reset: a connection dropped under the client mid-seed.
    fn reset_on_next_request(s: TcpStream) {
        let mut b = [0u8; 1];
        let _ = s.peek(&mut b);
        drop(s);
    }

    /// A loopback server whose connections each answer one request (`status`, body `{}`,
    /// keep-alive) and reset on the next one; the first `reset_first` connections reset before
    /// they answer anything. Returns its URL, the connections accepted and the requests answered.
    fn server(reset_first: usize, status: &'static str) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let (accepted, answered) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (a, n) = (accepted.clone(), answered.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(s) = s else { continue };
                if a.fetch_add(1, Ordering::SeqCst) < reset_first {
                    reset_on_next_request(s);
                    continue;
                }
                let n = n.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    if !read_request(&mut r) {
                        return;
                    }
                    let head =
                        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}");
                    s.try_clone().unwrap().write_all(head.as_bytes()).unwrap();
                    n.fetch_add(1, Ordering::SeqCst);
                    reset_on_next_request(s);
                });
            }
        });
        (url, accepted, answered)
    }

    // CMD-10: under load a kept-alive socket went stale between seed requests and the next one
    // on it failed with `io: Invalid argument (os error 22)` on macOS (a reset elsewhere).
    #[test]
    fn a_connection_dropped_between_requests_does_not_fail_the_seed() {
        let (url, _, answered) = server(0, "200 OK");
        let http = Http::new(&url);
        for i in 0..20 {
            let path = format!("/api/v1/docs/d/threads/t{i}/resolve");
            http.post("tok", &path, &json!({})).unwrap_or_else(|e| panic!("request {i}: {e}"));
        }
        http.get("tok", "/api/v1/docs/d/threads").unwrap();
        http.delete("tok", "/api/v1/docs/d/threads/t1").unwrap();
        assert_eq!(answered.load(Ordering::SeqCst), 22);
    }

    #[test]
    fn a_transport_failure_is_retried_and_gives_up_after_three_attempts() {
        let (url, accepted, answered) = server(2, "200 OK");
        Http::new(&url).post("tok", "/api/v1/docs/d/threads/t1/resolve", &json!({})).unwrap();
        assert_eq!((accepted.load(Ordering::SeqCst), answered.load(Ordering::SeqCst)), (3, 1));

        let (url, accepted, answered) = server(usize::MAX, "200 OK");
        let err = Http::new(&url).post("tok", "/api/v1/docs/d/threads/t1/resolve", &json!({})).unwrap_err();
        assert!(err.to_string().starts_with("/api/v1/docs/d/threads/t1/resolve: io: "), "{err}");
        assert_eq!((accepted.load(Ordering::SeqCst), answered.load(Ordering::SeqCst)), (3, 0));
    }

    #[test]
    fn an_http_error_status_is_not_retried() {
        let (url, accepted, _) = server(0, "409 Conflict");
        let err = Http::new(&url).post("tok", "/api/v1/docs/d/actions", &json!({})).unwrap_err();
        assert!(matches!(err, SeedError::Http { status: 409, .. }), "{err}");
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    }
}
