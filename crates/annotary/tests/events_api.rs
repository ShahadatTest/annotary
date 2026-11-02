//! `GET /api/v1/events` over HTTP against the in-process server (ticket 08): the cursor
//! contract (`since`, `seq`), the long-poll waking on every kind, the filters (`doc`, `kinds`,
//! `action`, `mention`), rights, paging, the event shapes `wait` prints from, and the SSE form.

use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use annotary::testing::{TestServer, probe_root};
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(md)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

fn create(ts: &TestServer, token: &str, vault: &str, name: &str, md: &str) -> String {
    let hash = upload(ts, token, md.as_bytes());
    let (status, doc) = ts.post("/api/v1/docs", Some(token), &json!({ "vault": vault, "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, token: &str, id: &str, name: &str, md: &str, message: &str) -> u64 {
    let hash = upload(ts, token, md.as_bytes());
    let (status, v) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(token),
        &json!({ "name": name, "md_hash": hash, "message": message }),
    );
    assert_eq!(status, 201, "{v}");
    v["version"].as_u64().unwrap()
}

fn thread(ts: &TestServer, token: &str, id: &str, body: &str) -> String {
    let (status, t) =
        ts.post(&format!("/api/v1/docs/{id}/threads"), Some(token), &json!({ "subject": "document", "body": body }));
    assert_eq!(status, 201, "{t}");
    t["id"].as_str().unwrap().to_string()
}

fn reply(ts: &TestServer, token: &str, id: &str, tid: &str, body: &str) -> String {
    let (status, r) =
        ts.post(&format!("/api/v1/docs/{id}/threads/{tid}/replies"), Some(token), &json!({ "body": body }));
    assert_eq!(status, 201, "{r}");
    r["comment"]["id"].as_str().unwrap().to_string()
}

fn act(ts: &TestServer, token: &str, id: &str, action: &str) {
    let (status, a) = ts.post(&format!("/api/v1/docs/{id}/actions"), Some(token), &json!({ "action": action }));
    assert_eq!(status, 201, "{a}");
}

fn events(ts: &TestServer, token: &str, query: &str) -> (u16, Value) {
    ts.get(&format!("/api/v1/events?{query}"), Some(token))
}

fn kinds(body: &Value) -> Vec<String> {
    body["events"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap_or("-").to_string()).collect()
}

fn types(body: &Value) -> Vec<String> {
    body["events"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap().to_string()).collect()
}

// spec: S08-01, S08-03, S08-04
#[test]
fn cursor_contract_filters_and_shapes() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("scribe");
    let head0 = ts.store.last_seq().unwrap();

    // `since` absent = from the head: nothing yet, the cursor is the head.
    let (status, b) = events(&ts, &owner, "");
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["events"], json!([]));
    assert_eq!(b["seq"], head0);

    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n\nWe ship the CLI first.\n");
    let other = create(&ts, &owner, "notes", "other", "# Other\n");
    let v2 = push(&ts, &owner, &plan, "plan", "# Plan v2\n", "address review");
    assert_eq!(v2, 2);
    let t1 = thread(&ts, &owner, &plan, "Five external devs — do we have five lined up? @scribe");
    let t1_1 = reply(&ts, &agent, &plan, &t1, "Item 3 already lists it. @rowan");
    let t1_2 = reply(&ts, &owner, &plan, &t1, "Done.");
    let (_, r) = ts.post(&format!("/api/v1/docs/{plan}/threads/{t1}/resolve"), Some(&owner), &json!({}));
    assert_eq!(r["changed"], true);
    let (_, r) = ts.post(&format!("/api/v1/docs/{plan}/threads/{t1}/reopen"), Some(&agent), &json!({}));
    assert_eq!(r["changed"], true);
    act(&ts, &owner, &plan, "approve");
    act(&ts, &agent, &plan, "send-back");
    let head = ts.store.last_seq().unwrap();

    // Everything after head0, no narrowing: every readable event, raw types, in order.
    let (status, b) = events(&ts, &owner, &format!("since={head0}"));
    assert_eq!(status, 200);
    assert_eq!(b["seq"], head);
    assert_eq!(
        types(&b),
        [
            "document.created",
            "version.pushed",
            "document.created",
            "version.pushed",
            "version.pushed",
            "thread.created",
            "thread.replied",
            "thread.replied",
            "thread.resolved",
            "thread.reopened",
            "action.taken",
            "action.taken",
        ]
    );
    assert_eq!(
        kinds(&b),
        [
            "-", "version", "-", "version", "version", "thread", "reply", "reply", "resolve", "reopen", "action",
            "action"
        ]
    );
    let evs = b["events"].as_array().unwrap();
    // Shapes: handles, Document id/name/vault, raw payload, the excerpt and mentions as handles.
    let pushed = &evs[4];
    assert_eq!(pushed["by"], "rowan");
    assert_eq!(pushed["doc"], plan);
    assert_eq!(pushed["name"], "plan");
    assert_eq!(pushed["vault"], "notes");
    assert_eq!(pushed["payload"]["version"], 2);
    assert_eq!(pushed["payload"]["message"], "address review");
    assert_eq!(pushed["mention"], false);
    let created = &evs[5];
    assert_eq!(created["payload"]["thread"], t1);
    assert_eq!(created["payload"]["mentions"], json!([agent_id]));
    assert_eq!(created["mentions"], json!(["scribe"]));
    assert_eq!(created["excerpt"], "Five external devs — do we have five lined up? @scribe");
    assert_eq!(created["mention"], false, "the owner is not the one mentioned");
    let replied = &evs[6];
    assert_eq!(replied["payload"]["comment"], t1_1);
    assert_eq!(replied["by"], "scribe");
    assert_eq!(replied["mention"], true, "…but the reply mentions @rowan");
    assert_eq!(replied["excerpt"], "Item 3 already lists it. @rowan");
    let taken = &evs[10];
    assert_eq!(taken["payload"]["action"], "approve");
    assert_eq!(taken["payload"]["version"], 2);
    assert_eq!(taken["payload"]["label"], "Approve");
    assert!(evs.iter().all(|e| e["seq"].as_i64().unwrap() > head0 && e["at"].is_string()));
    let seqs: Vec<i64> = evs.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "strictly increasing: {seqs:?}");

    // `kinds=` = wait's words; `mention` is per caller; `doc=` narrows; `action=` narrows actions.
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=version,action,mention"));
    assert_eq!(kinds(&b), ["version", "version", "version", "mention", "action", "action"]);
    assert_eq!(b["events"][3]["payload"]["comment"], t1_1);
    assert_eq!(b["events"][3]["excerpt"], "Item 3 already lists it. @rowan");
    let (_, b) = events(&ts, &agent, &format!("since={head0}&kinds=mention"));
    assert_eq!(kinds(&b), ["mention"]);
    assert_eq!(b["events"][0]["payload"]["thread"], t1);
    assert_eq!(b["events"][0]["excerpt"], "Five external devs — do we have five lined up? @scribe");
    let (_, b) = events(&ts, &agent, &format!("since={head0}&kinds=reply"));
    assert_eq!(kinds(&b), ["reply", "reply"], "asked for replies: a mention is reported as the reply it is");
    assert_eq!(b["events"][0]["mention"], false);
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=reply,mention"));
    assert_eq!(kinds(&b), ["mention", "reply"], "asked for both: mention wins for the one that mentions me");
    assert_eq!(b["events"][1]["payload"]["comment"], t1_2);
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=thread,resolve,reopen"));
    assert_eq!(kinds(&b), ["thread", "resolve", "reopen"]);
    let (_, b) = events(&ts, &owner, &format!("since={head0}&doc={other}"));
    assert_eq!(types(&b), ["document.created", "version.pushed"]);
    assert_eq!(b["seq"], head, "the cursor still advances to the head");
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=action&action=send-back"));
    assert_eq!(kinds(&b), ["action"]);
    assert_eq!(b["events"][0]["by"], "scribe");
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=action&action=ship"));
    assert_eq!(b["events"], json!([]));
    assert_eq!(b["seq"], head);
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=version.rendered,action.taken"));
    assert_eq!(types(&b), ["action.taken", "action.taken"], "raw types are accepted too");
    // A future cursor heals to the head; `since=head` is empty.
    let (_, b) = events(&ts, &owner, &format!("since={}", head + 100));
    assert_eq!(b["events"], json!([]));
    assert_eq!(b["seq"], head);
    let (_, b) = events(&ts, &owner, &format!("since={head}"));
    assert_eq!(b["events"], json!([]));

    // Errors: unknown kind, unknown / unreadable Document.
    let (status, b) = events(&ts, &owner, "kinds=bogus");
    assert_eq!(status, 400, "{b}");
    assert!(b["message"].as_str().unwrap().contains("version,action,mention,thread,reply,resolve,reopen"));
    let (status, _) = events(&ts, &owner, "doc=zzzzzzzz");
    assert_eq!(status, 404);
    let (status, _) = events(&ts, &owner, "doc=nope");
    assert_eq!(status, 404);
    let (status, _) = ts.get("/api/v1/events", None);
    assert_eq!(status, 401);

    // A tombstoned Comment is quoted as such; a deleted Document's events stay readable.
    let (status, _) =
        ts.post(&format!("/api/v1/docs/{plan}/threads/{t1}/replies"), Some(&owner), &json!({ "body": "oops" }));
    assert_eq!(status, 201);
    let res = ureq::delete(format!("{}/api/v1/docs/{plan}/threads/{t1}.3", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .call()
        .unwrap();
    assert!(res.status().is_success());
    let (_, b) = events(&ts, &owner, &format!("since={head}&kinds=reply"));
    assert_eq!(b["events"][0]["excerpt"], "(deleted)");
}

// spec: S08-04
#[test]
fn paging_cuts_and_continues_without_loss() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let head0 = ts.store.last_seq().unwrap();
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    for i in 0..6 {
        thread(&ts, &owner, &plan, &format!("note {i}"));
    }
    let head = ts.store.last_seq().unwrap();
    // Raw rows are paged: with limit=2 and no match in the first page (document.created,
    // version.pushed), the scan moves on inside the same request until something matches.
    let (_, b) = events(&ts, &owner, &format!("since={head0}&kinds=thread&limit=2"));
    assert_eq!(kinds(&b), ["thread", "thread"]);
    let reached = b["seq"].as_i64().unwrap();
    assert!(reached < head, "a cut page reports the last scanned seq, not the head");
    assert_eq!(reached, b["events"][1]["seq"].as_i64().unwrap());
    let mut seen = vec![b["events"][0]["payload"]["thread"].clone(), b["events"][1]["payload"]["thread"].clone()];
    let mut cursor = reached;
    loop {
        let (_, b) = events(&ts, &owner, &format!("since={cursor}&kinds=thread&limit=2"));
        let evs = b["events"].as_array().unwrap();
        if evs.is_empty() {
            assert_eq!(b["seq"], head);
            break;
        }
        seen.extend(evs.iter().map(|e| e["payload"]["thread"].clone()));
        cursor = b["seq"].as_i64().unwrap();
    }
    assert_eq!(seen, (1..=6).map(|n| json!(format!("t{n}"))).collect::<Vec<_>>(), "every Thread once, in order");
}

// spec: S08-03
#[test]
fn long_poll_parks_and_wakes_on_each_kind() {
    // Locked: the parked poll answers with the head, which a loopback sign-in would move (CMD-19).
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    let t1 = thread(&ts, &owner, &plan, "root");
    let mut cursor = ts.store.last_seq().unwrap();

    // wait=1 with nothing happening: parks about a second, answers with the head. A port watcher
    // probing the server meanwhile (CMD-19) is not something happening.
    let t0 = Instant::now();
    let (status, b) = std::thread::scope(|s| {
        s.spawn(|| {
            std::thread::sleep(Duration::from_millis(300));
            probe_root(&ts.url);
        });
        events(&ts, &owner, &format!("since={cursor}&wait=1&kinds=version"))
    });
    assert_eq!(status, 200);
    assert_eq!(b["events"], json!([]));
    assert_eq!(b["seq"], cursor);
    let parked = t0.elapsed();
    assert!(parked >= Duration::from_millis(900) && parked < Duration::from_secs(5), "parked {parked:?}");

    // Each kind wakes a parked poll well before its `wait` elapses.
    type Mutation<'a> = Box<dyn Fn() + Send + 'a>;
    let cases: Vec<(&str, Mutation<'_>)> = vec![
        (
            "version",
            Box::new(|| {
                push(&ts, &owner, &plan, "plan", "# Plan v2\n", "");
            }),
        ),
        ("action", Box::new(|| act(&ts, &owner, &plan, "approve"))),
        (
            "mention",
            Box::new(|| {
                reply(&ts, &agent, &plan, &t1, "ping @rowan");
            }),
        ),
        (
            "thread",
            Box::new(|| {
                thread(&ts, &owner, &plan, "new");
            }),
        ),
        (
            "reply",
            Box::new(|| {
                reply(&ts, &owner, &plan, &t1, "r");
            }),
        ),
        (
            "resolve",
            Box::new(|| {
                ts.post(&format!("/api/v1/docs/{plan}/threads/{t1}/resolve"), Some(&owner), &json!({}));
            }),
        ),
        (
            "reopen",
            Box::new(|| {
                ts.post(&format!("/api/v1/docs/{plan}/threads/{t1}/reopen"), Some(&owner), &json!({}));
            }),
        ),
    ];
    for (kind, mutate) in cases {
        let (body, elapsed) = std::thread::scope(|s| {
            let poll = s.spawn(|| {
                let t0 = Instant::now();
                let (status, b) = events(&ts, &owner, &format!("since={cursor}&wait=20&kinds={kind}"));
                assert_eq!(status, 200, "{b}");
                (b, t0.elapsed())
            });
            std::thread::sleep(Duration::from_millis(150));
            mutate();
            poll.join().unwrap()
        });
        assert!(elapsed < Duration::from_secs(5), "{kind}: woke after {elapsed:?}");
        assert_eq!(kinds(&body), [kind], "{kind}: {body}");
        let seq = body["seq"].as_i64().unwrap();
        assert!(seq > cursor);
        assert_eq!(seq, body["events"][0]["seq"].as_i64().unwrap(), "the cursor is the event just returned");
        cursor = seq;
    }
    // `wait` is clamped to 30 s; a non-matching commit does not end the poll early.
    let (body, elapsed) = std::thread::scope(|s| {
        let poll = s.spawn(|| {
            let t0 = Instant::now();
            let (_, b) = events(&ts, &owner, &format!("since={cursor}&wait=2&kinds=action&action=ship"));
            (b, t0.elapsed())
        });
        std::thread::sleep(Duration::from_millis(150));
        act(&ts, &owner, &plan, "send-back");
        poll.join().unwrap()
    });
    assert_eq!(body["events"], json!([]));
    assert!(elapsed >= Duration::from_millis(1800), "kept parking past the non-matching commit: {elapsed:?}");
    assert_eq!(body["seq"], ts.store.last_seq().unwrap(), "…but the cursor moved past it");
}

// spec: S08-09
#[test]
fn rights_scope_what_a_member_sees() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("linter");
    ts.create_vault("drafts");
    let head0 = ts.store.last_seq().unwrap();
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    let draft = create(&ts, &owner, "drafts", "draft", "# Draft\n");
    // linter: `notes` only.
    ts.store
        .set_grants(&ts.owner_id, &agent_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Read] }])
        .unwrap();
    let (status, b) = events(&ts, &agent, &format!("since={head0}"));
    assert_eq!(status, 200, "{b}");
    let evs = b["events"].as_array().unwrap();
    assert!(!evs.is_empty());
    assert!(evs.iter().all(|e| e["doc"] == plan), "only notes' Document: {b}");
    assert_eq!(b["seq"], ts.store.last_seq().unwrap());
    let (status, b) = events(&ts, &agent, &format!("since={head0}&doc={draft}"));
    assert_eq!(status, 403, "{b}");
    // Tenant-level events (members, tokens, vaults) are the owner's.
    let (_, b) = events(&ts, &owner, "since=0");
    assert!(types(&b).iter().any(|t| t == "member.added"));
    assert!(b["events"].as_array().unwrap().iter().any(|e| e["doc"].is_null() && e["vault"].is_null()));
    let (_, b) = events(&ts, &agent, "since=0");
    assert!(b["events"].as_array().unwrap().iter().all(|e| e["doc"] == plan), "{b}");
    // A purged Document's events lose their Vault: readable by the owner only.
    let (status, _) = ts.get(&format!("/api/v1/docs/{plan}"), Some(&owner));
    assert_eq!(status, 200);
    let res = ureq::delete(format!("{}/api/v1/docs/{plan}", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .call()
        .unwrap();
    assert!(res.status().is_success());
    let (status, _) = ts.post(&format!("/api/v1/docs/{plan}/purge"), Some(&owner), &json!({}));
    assert_eq!(status, 200);
    let (_, b) = events(&ts, &agent, &format!("since={head0}"));
    assert_eq!(b["events"], json!([]), "nothing left that linter may read: {b}");
    let (status, b) = events(&ts, &owner, &format!("since={head0}&doc={plan}"));
    assert_eq!(status, 404, "the Document is gone: {b}");
    let (status, b) = events(&ts, &owner, &format!("since={head0}&kinds=version"));
    assert_eq!(status, 200);
    assert_eq!(b["events"].as_array().unwrap().len(), 2, "owner still sees both pushes (no name/vault): {b}");
    assert!(b["events"][0]["name"].is_null());
}

/// Complete SSE frames so far (each ends with a blank line).
fn frames(lines: &[String]) -> usize {
    lines.iter().filter(|l| l.is_empty()).count()
}

/// Open an SSE connection and collect raw lines until `until` says stop (or the read times out).
fn sse_lines(
    ts: &TestServer,
    token: &str,
    query: &str,
    headers: &[(&str, &str)],
    until: impl Fn(&[String]) -> bool,
) -> Vec<String> {
    let mut req = ureq::get(format!("{}/api/v1/events?{query}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "text/event-stream");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut res = req.config().timeout_global(Some(Duration::from_secs(10))).build().call().unwrap();
    assert_eq!(res.status(), 200);
    assert!(res.headers().get("content-type").unwrap().to_str().unwrap().starts_with("text/event-stream"));
    let mut lines = Vec::new();
    let mut reader = BufReader::new(res.body_mut().as_reader());
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => lines.push(line.trim_end().to_string()),
        }
        if until(&lines) {
            break;
        }
    }
    lines
}

// spec: S08-10
#[test]
fn sse_streams_the_same_events_and_resumes_from_last_event_id() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    let cursor = ts.store.last_seq().unwrap();
    let t1 = thread(&ts, &owner, &plan, "first");
    let after_t1 = ts.store.last_seq().unwrap();

    // Backlog first, then live: `ready` names the cursor, every event is `id`/`event`/`data`.
    let lines = std::thread::scope(|s| {
        let stream = s
            .spawn(|| sse_lines(&ts, &owner, &format!("since={cursor}&kinds=thread,reply"), &[], |ls| frames(ls) >= 3));
        std::thread::sleep(Duration::from_millis(300));
        reply(&ts, &owner, &plan, &t1, "live one");
        stream.join().unwrap()
    });
    let joined = lines.join("\n");
    assert!(joined.starts_with(&format!("event: ready\ndata: {{\"seq\":{cursor}}}")), "{joined}");
    assert!(joined.contains(&format!("id: {after_t1}\nevent: thread.created\ndata: {{\"seq\":{after_t1}")), "{joined}");
    let live = ts.store.last_seq().unwrap();
    assert!(joined.contains(&format!("id: {live}\nevent: thread.replied\ndata: ")), "{joined}");
    let data: Vec<Value> =
        lines.iter().filter_map(|l| l.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap()).collect();
    assert_eq!(data[1]["kind"], "thread");
    assert_eq!(data[1]["excerpt"], "first");
    assert_eq!(data[2]["kind"], "reply");
    assert_eq!(data[2]["payload"]["comment"], format!("{t1}.1"));

    // A reconnect with `Last-Event-ID` picks up right after it; `since` absent + no header = head.
    let lines =
        sse_lines(&ts, &owner, "kinds=thread,reply", &[("Last-Event-ID", &after_t1.to_string())], |ls| frames(ls) >= 2);
    let joined = lines.join("\n");
    assert!(joined.contains(&format!("event: ready\ndata: {{\"seq\":{after_t1}}}")), "{joined}");
    assert!(joined.contains(&format!("id: {live}\nevent: thread.replied")), "{joined}");
    assert!(!joined.contains("thread.created"), "nothing before the id is repeated: {joined}");
    let lines = std::thread::scope(|s| {
        let stream = s.spawn(|| sse_lines(&ts, &owner, "", &[], |ls| frames(ls) >= 2));
        std::thread::sleep(Duration::from_millis(300));
        // A port watcher probing the server is not an event (CMD-29); locked, it signs nobody in.
        probe_root(&ts.url);
        act(&ts, &owner, &plan, "approve");
        stream.join().unwrap()
    });
    let joined = lines.join("\n");
    assert!(joined.starts_with(&format!("event: ready\ndata: {{\"seq\":{live}}}")), "{joined}");
    assert!(joined.contains("event: action.taken"), "{joined}");
    assert!(!joined.contains("thread.replied"), "from the head only: {joined}");
}

// ---- CMD-8: the SSE stream re-checks access ----

/// An SSE connection read one frame at a time, so a test can change access between frames.
struct SseConn {
    reader: BufReader<ureq::BodyReader<'static>>,
}

impl SseConn {
    /// Open `GET /api/v1/events?{query}` as an SSE stream with `credential` (a header pair).
    fn open(ts: &TestServer, credential: (&str, &str), query: &str) -> SseConn {
        let res = ureq::get(format!("{}/api/v1/events?{query}", ts.url))
            .header(credential.0, credential.1)
            .header("Accept", "text/event-stream")
            .config()
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .call()
            .unwrap();
        assert_eq!(res.status(), 200);
        SseConn { reader: BufReader::new(res.into_body().into_reader()) }
    }

    /// The next frame's lines (keep-alive comments skipped); `Ok(None)` when the server ended
    /// the stream, `Err` when nothing came before the timeout.
    fn frame(&mut self) -> std::io::Result<Option<Vec<String>>> {
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            let line = line.trim_end().to_string();
            if !line.is_empty() {
                if !line.starts_with(':') {
                    lines.push(line);
                }
            } else if !lines.is_empty() {
                return Ok(Some(lines));
            }
        }
    }

    /// The next frame as `(event type, data)`; panics when the stream ends or times out.
    fn event(&mut self) -> (String, Value) {
        let lines = self.frame().expect("a frame before the timeout").expect("the stream is still open");
        let kind = lines.iter().find_map(|l| l.strip_prefix("event: ")).unwrap_or("message").to_string();
        let data = lines.iter().find_map(|l| l.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap());
        (kind, data.unwrap_or(Value::Null))
    }
}

// spec: S20-01
#[test]
fn sse_stops_a_vaults_events_once_its_grant_is_revoked() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("linter");
    ts.create_vault("drafts");
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    let draft = create(&ts, &owner, "drafts", "draft", "# Draft\n");
    let read = |vault: &str| Grant { vault: vault.into(), rights: vec![Right::Read] };
    ts.store.set_grants(&ts.owner_id, &agent_id, vec![read("notes"), read("drafts")]).unwrap();

    let bearer = format!("Bearer {agent}");
    let mut sse = SseConn::open(&ts, ("Authorization", &bearer), "kinds=thread");
    assert_eq!(sse.event().0, "ready");
    thread(&ts, &owner, &plan, "before the revoke");
    let (kind, data) = sse.event();
    assert_eq!(kind, "thread.created");
    assert_eq!(data["doc"], plan);
    assert_eq!(data["excerpt"], "before the revoke");

    // The owner takes `notes` away while the stream is open: `notes` goes quiet, `drafts` flows.
    ts.store.set_grants(&ts.owner_id, &agent_id, vec![read("drafts")]).unwrap();
    thread(&ts, &owner, &plan, "after the revoke");
    thread(&ts, &owner, &draft, "drafts still readable");
    let (kind, data) = sse.event();
    assert_eq!(kind, "thread.created");
    assert_eq!(data["excerpt"], "drafts still readable", "no event of the revoked Vault: {data}");
    assert_eq!(data["doc"], draft);

    // Granting it back resumes it (from now on).
    ts.store.set_grants(&ts.owner_id, &agent_id, vec![read("notes"), read("drafts")]).unwrap();
    thread(&ts, &owner, &plan, "granted again");
    let (_, data) = sse.event();
    assert_eq!(data["excerpt"], "granted again", "{data}");
}

// spec: S20-02
#[test]
fn sse_ends_when_its_credential_dies() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    let read_notes = vec![Grant { vault: "notes".into(), rights: vec![Right::Read] }];

    // After the owner kills the Credential, a readable event lands: the stream ends instead.
    let assert_ends = |sse: &mut SseConn, what: &str| {
        thread(&ts, &owner, &plan, &format!("after {what}"));
        let started = Instant::now();
        match sse.frame() {
            Ok(None) => {}
            other => panic!("{what}: the stream should have ended, got {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(3), "{what}: ended at once, not by the timeout");
    };

    // A revoked Token.
    let (linter_id, linter) = ts.add_member("linter");
    ts.store.set_grants(&ts.owner_id, &linter_id, read_notes.clone()).unwrap();
    let bearer = format!("Bearer {linter}");
    let mut sse = SseConn::open(&ts, ("Authorization", &bearer), "");
    assert_eq!(sse.event().0, "ready");
    let token_id = ts.store.list_tokens(Some(&linter_id)).unwrap()[0].info.id.clone();
    assert!(ts.store.revoke_token(&ts.owner_id, &token_id).unwrap());
    assert_ends(&mut sse, "the Token was revoked");

    // An ended browser session.
    let (gem_id, _) = ts.add_member("gemini");
    ts.store.set_grants(&ts.owner_id, &gem_id, read_notes.clone()).unwrap();
    let session = ts.store.open_session(&gem_id, "login").unwrap();
    let cookie = format!("cd_session={}", session.plain);
    let mut sse = SseConn::open(&ts, ("Cookie", &cookie), "");
    assert_eq!(sse.event().0, "ready");
    assert!(ts.store.end_session(&session.id).unwrap());
    assert_ends(&mut sse, "the session ended");

    // A disabled Member.
    let (aider_id, aider) = ts.add_member("aider");
    ts.store.set_grants(&ts.owner_id, &aider_id, read_notes).unwrap();
    let bearer = format!("Bearer {aider}");
    let mut sse = SseConn::open(&ts, ("Authorization", &bearer), "");
    assert_eq!(sse.event().0, "ready");
    assert!(ts.store.disable_member(&ts.owner_id, &aider_id).unwrap());
    assert_ends(&mut sse, "the Member was disabled");
}

// spec: S20-03
#[test]
fn a_parked_long_poll_re_checks_access_when_it_wakes() {
    let ts = TestServer::start_locked();
    let owner = ts.owner_token.clone();
    let (agent_id, agent) = ts.add_member("linter");
    ts.create_vault("drafts");
    let plan = create(&ts, &owner, "notes", "plan", "# Plan\n");
    let draft = create(&ts, &owner, "drafts", "draft", "# Draft\n");
    let read = |vault: &str| Grant { vault: vault.into(), rights: vec![Right::Read] };
    ts.store.set_grants(&ts.owner_id, &agent_id, vec![read("notes"), read("drafts")]).unwrap();

    // Parked as a reader of both; `notes` is revoked mid-park, then each Vault gets an event.
    let since = ts.store.last_seq().unwrap();
    let (status, b) = std::thread::scope(|s| {
        let poll = s.spawn(|| events(&ts, &agent, &format!("since={since}&wait=5&kinds=thread")));
        std::thread::sleep(Duration::from_millis(500));
        ts.store.set_grants(&ts.owner_id, &agent_id, vec![read("drafts")]).unwrap();
        thread(&ts, &owner, &plan, "after the revoke");
        thread(&ts, &owner, &draft, "drafts still readable");
        poll.join().unwrap()
    });
    assert_eq!(status, 200, "{b}");
    let evs = b["events"].as_array().unwrap();
    assert!(!evs.is_empty(), "{b}");
    assert!(evs.iter().all(|e| e["doc"] == draft), "only the Vault still readable: {b}");
    assert_eq!(evs[0]["excerpt"], "drafts still readable");

    // A Credential that dies mid-park answers 401.
    let since = ts.store.last_seq().unwrap();
    let token_id = ts.store.list_tokens(Some(&agent_id)).unwrap()[0].info.id.clone();
    let (status, b) = std::thread::scope(|s| {
        let poll = s.spawn(|| events(&ts, &agent, &format!("since={since}&wait=5")));
        std::thread::sleep(Duration::from_millis(500));
        assert!(ts.store.revoke_token(&ts.owner_id, &token_id).unwrap());
        thread(&ts, &owner, &draft, "after the Token was revoked");
        poll.join().unwrap()
    });
    assert_eq!(status, 401, "{b}");
}
// ---- end CMD-8 ----
