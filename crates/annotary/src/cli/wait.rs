//! `wait [DOC] [--on KINDS] [--action ID] [--cursor SEQ] [--timeout DUR]` (ticket 08): block
//! until something happens, print it, exit 0 — one long-poll at a time, no polling loop.
//!
//! - `--on` defaults to `version,action,mention`; `thread`, `reply`, `resolve`, `reopen` are
//!   opt-in. `mention` = a Comment that names the calling Member. `--action ID` narrows `action`.
//! - The cursor: `--cursor SEQ` (from `status`'s `seq:` or a previous `wait`), else `status`'s
//!   `seq` when DOC is given, else the server's head — only what happens from now on. Every
//!   printed event carries its `seq`; the next `wait --cursor` from the last one resumes without
//!   a gap or a repeat, across the server's restarts or your own.
//! - Output: `SEQ KIND DOC DETAIL` per event (every event that arrived together), tab-separated
//!   (two spaces on a TTY); `--json` = one object per event line. DETAIL: `v5 by @rowan "msg"`,
//!   `approve v5 by @rowan`, `t2.2 by @rowan "…@you…"`, `t9 by @rowan "…"`, `t2 by @rowan`.
//! - Timeout (`--timeout`, default 30m, `0` = never): `wait: nothing after DUR (seq N)` on
//!   stderr, exit 7. Server unreachable at any point ⇒ exit 6 (the hint names the cursor).

use std::time::{Duration, Instant};

use clap::Args;
use serde_json::{Map, Value, json};

use crate::client::qs;
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds};

use super::Ctx;
use super::docref::{DeletedMode, resolve};

/// Longest the server parks one long-poll (mirrors `server::events::MAX_WAIT_SECS`).
pub const MAX_POLL_SECS: u64 = 30;
/// Slack over the parked time before the HTTP request itself is given up on (exit 6).
const POLL_SLACK_SECS: u64 = 15;

/// The kinds `--on` accepts, in `--help` order. The first three are the default.
pub const KINDS: [&str; 7] = ["version", "action", "mention", "thread", "reply", "resolve", "reopen"];
pub const DEFAULT_KINDS: &str = "version,action,mention";

// ticket 14: the verb's `--help` is `cli::help::WAIT` (the prototype's text).
#[derive(Args, Debug)]
pub struct WaitArgs {
    /// Document id, unique Name, or path to a pulled .md (default: every Document you can read)
    pub doc: Option<String>,
    /// Kinds that wake you (comma list): version,action,mention — also thread,reply,resolve,reopen
    #[arg(long, value_name = "KINDS", default_value = DEFAULT_KINDS)]
    pub on: String,
    /// With action: only this Action id
    #[arg(long, value_name = "ID")]
    pub action: Option<String>,
    /// Resume from an event cursor (the `seq:` of `status` or of a previous `wait`)
    #[arg(long, value_name = "SEQ")]
    pub cursor: Option<u64>,
    /// Give up after DUR (30s, 30m, 2h, 1d; 0 = never) — exit 7
    #[arg(long, value_name = "DUR", default_value = "30m")]
    pub timeout: String,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &[
        "seq",
        "kind",
        "doc",
        "name",
        "action",
        "version",
        "thread",
        "comment",
        "message",
        "excerpt",
        "by",
        "at",
        "type",
        "label",
        "intent",
        "same_as",
        "discarded",
        "mentions",
        "vault",
    ],
    default: &[
        "seq", "kind", "doc", "name", "action", "version", "thread", "comment", "message", "excerpt", "by", "at",
    ],
};

/// Parse `--on`: a comma list of known kinds, de-duplicated in the order given.
pub fn parse_kinds(text: &str) -> CliResult<Vec<&'static str>> {
    let mut out: Vec<&'static str> = Vec::new();
    for w in text.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        match KINDS.iter().find(|k| **k == w) {
            Some(k) if !out.contains(k) => out.push(k),
            Some(_) => {}
            None => {
                return Err(CliError::usage(format!("--on {w:?}: unknown kind"))
                    .hint(format!("use a comma list of {}", KINDS.join(","))));
            }
        }
    }
    if out.is_empty() {
        return Err(
            CliError::usage("--on needs at least one kind").hint(format!("use a comma list of {}", KINDS.join(",")))
        );
    }
    Ok(out)
}

/// Parse `--timeout`: `0` = never (`None`); else `Ns|Nm|Nh|Nd|Nw`.
pub fn parse_timeout(text: &str) -> CliResult<Option<Duration>> {
    let t = text.trim();
    let bad = || CliError::usage(format!("--timeout {t:?}: use a duration (30s, 30m, 2h, 1d) or 0 for never"));
    if t == "0" {
        return Ok(None);
    }
    let (num, unit) = t.split_at_checked(t.len().saturating_sub(1)).ok_or_else(bad)?;
    let n: u64 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60).ok_or_else(bad)?,
        "h" => n.checked_mul(3600).ok_or_else(bad)?,
        "d" => n.checked_mul(86_400).ok_or_else(bad)?,
        "w" => n.checked_mul(7 * 86_400).ok_or_else(bad)?,
        _ => return Err(bad()),
    };
    Ok(if secs == 0 { None } else { Some(Duration::from_secs(secs)) })
}

/// The DETAIL cell of one event as the API returns it.
pub fn detail(e: &Value) -> String {
    let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
    let p = &e["payload"];
    let ps = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("");
    let by = if s("by").is_empty() { String::new() } else { format!(" by @{}", s("by")) };
    let version = p.get("version").and_then(Value::as_u64).map(|v| format!("v{v}")).unwrap_or_default();
    let quote = |text: &str| if text.is_empty() { String::new() } else { format!(" \"{text}\"") };
    match s("kind") {
        "version" => {
            // Ticket 17: a push over the head's changes says so (`v6 by @x discarding v5`).
            let discarded: Vec<String> = p["discarded"]
                .as_array()
                .map(|d| d.iter().filter_map(Value::as_u64).map(|n| format!("v{n}")).collect())
                .unwrap_or_default();
            let discarding =
                if discarded.is_empty() { String::new() } else { format!(" discarding {}", discarded.join(",")) };
            format!("{version}{by}{discarding}{}", quote(ps("message")))
        }
        "action" => format!("{} {version}{by}", ps("action")),
        "mention" | "thread" | "reply" => {
            let id = if ps("comment").is_empty() { ps("thread") } else { ps("comment") };
            format!("{id}{by}{}", quote(s("excerpt")))
        }
        "resolve" | "reopen" => format!("{}{by}", ps("thread")),
        _ => {
            let id = if !ps("comment").is_empty() {
                ps("comment").to_string()
            } else if !ps("thread").is_empty() {
                ps("thread").to_string()
            } else {
                String::new()
            };
            format!("{}{}{by}", s("type"), if id.is_empty() { String::new() } else { format!(" {id}") })
                .trim()
                .to_string()
        }
    }
}

/// The `--json` record of one event (flat: the DETAIL's parts as fields).
pub fn record(e: &Value) -> Map<String, Value> {
    let p = &e["payload"];
    let mut r = Map::new();
    r.insert("seq".into(), e["seq"].clone());
    r.insert("kind".into(), e["kind"].clone());
    r.insert("doc".into(), e["doc"].clone());
    r.insert("name".into(), e["name"].clone());
    r.insert("action".into(), p["action"].clone());
    r.insert("version".into(), p["version"].clone());
    r.insert("thread".into(), p["thread"].clone());
    r.insert("comment".into(), p["comment"].clone());
    r.insert("message".into(), p["message"].clone());
    r.insert("excerpt".into(), e["excerpt"].clone());
    r.insert("by".into(), e["by"].clone());
    r.insert("at".into(), json!(iso_seconds(e["at"].as_str().unwrap_or(""))));
    r.insert("type".into(), e["type"].clone());
    r.insert("label".into(), p["label"].clone());
    r.insert("intent".into(), p["intent"].clone());
    r.insert("same_as".into(), p["same_as"].clone());
    r.insert("discarded".into(), p["discarded"].clone()); // ticket 17
    r.insert("mentions".into(), e["mentions"].clone());
    r.insert("vault".into(), e["vault"].clone());
    r
}

pub fn run(ctx: &Ctx, a: WaitArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let kinds = parse_kinds(&a.on)?;
    let timeout = parse_timeout(&a.timeout)?;
    if let Some(act) = a.action.as_deref().map(str::trim)
        && act.is_empty()
    {
        return Err(CliError::usage("--action needs an Action id"));
    }
    let api = ctx.api()?;

    // DOC → id, and the first cursor from `status` unless given.
    let mut doc_id: Option<String> = None;
    let mut doc_name: Option<String> = None;
    let mut cursor: Option<i64> = a.cursor.map(|c| c as i64);
    if let Some(doc) = &a.doc {
        let d = resolve(ctx, &api, doc, None, DeletedMode::Exclude)?;
        if !d["deleted_at"].is_null() {
            return Err(CliError::not_found(format!(
                "Document {} ({}) is deleted",
                d["name"].as_str().unwrap_or(""),
                d["id"].as_str().unwrap_or("")
            ))
            .hint("`annotary restore DOC` brings it back"));
        }
        let id = d["id"].as_str().unwrap_or("").to_string();
        if cursor.is_none() {
            let st = api.get(&format!("/api/v1/docs/{id}/status"))?;
            cursor = st["seq"].as_i64();
        }
        doc_name = d["name"].as_str().map(str::to_string);
        doc_id = Some(id);
    }
    if ctx.out.tty {
        ctx.out.note(&format!(
            "wait: watching {} for {} from seq {} (timeout {})",
            doc_name.as_deref().unwrap_or("every Document you can read"),
            kinds.join(", "),
            cursor.map(|c| c.to_string()).unwrap_or_else(|| "now".into()),
            if timeout.is_none() { "never" } else { a.timeout.trim() }
        ));
    }

    let deadline = timeout.map(|t| Instant::now() + t);
    loop {
        let poll_secs = match deadline {
            None => MAX_POLL_SECS,
            Some(d) => {
                let remaining = d.saturating_duration_since(Instant::now());
                (remaining.as_secs_f64().ceil() as u64).min(MAX_POLL_SECS)
            }
        };
        let mut path = format!("/api/v1/events?wait={poll_secs}&kinds={}", kinds.join(","));
        if let Some(c) = cursor {
            path.push_str(&format!("&since={c}"));
        }
        if let Some(id) = &doc_id {
            path.push_str(&format!("&doc={id}"));
        }
        if let Some(act) = a.action.as_deref().map(str::trim) {
            path.push_str(&format!("&action={}", qs(act)));
        }
        let body = api.get_timeout(&path, Duration::from_secs(poll_secs + POLL_SLACK_SECS)).map_err(|mut e| {
            if e.exit == crate::exit::Exit::Unreachable
                && let Some(c) = cursor
            {
                e = e.detail(format!("resume with --cursor {c}"));
            }
            e
        })?;
        if let Some(s) = body["seq"].as_i64() {
            cursor = Some(s);
        }
        let events = body["events"].as_array().cloned().unwrap_or_default();
        if !events.is_empty() {
            for e in &events {
                if ctx.out.is_json() {
                    ctx.out.json_records(&FIELDS, vec![record(e)], true)?;
                } else {
                    let fields = [
                        e["seq"].as_i64().unwrap_or(0).to_string(),
                        e["kind"].as_str().unwrap_or("").to_string(),
                        e["name"].as_str().or(e["doc"].as_str()).unwrap_or("-").to_string(),
                        detail(e),
                    ];
                    let sep = if ctx.out.tty { "  " } else { "\t" };
                    ctx.out.line(&fields.join(sep));
                }
            }
            return Ok(());
        }
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            return Err(CliError::wait_timeout(a.timeout.trim(), cursor));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S08-05
    #[test]
    fn kinds_parse_and_reject() {
        assert_eq!(parse_kinds("version,action,mention").unwrap(), vec!["version", "action", "mention"]);
        assert_eq!(parse_kinds(" reply, thread ,reply").unwrap(), vec!["reply", "thread"]);
        let e = parse_kinds("version,foo").unwrap_err();
        assert_eq!(e.exit, crate::exit::Exit::Usage);
        assert!(e.text_line().contains("--on \"foo\": unknown kind"), "{e}");
        assert!(parse_kinds(" , ").is_err());
    }

    // spec: S08-08
    #[test]
    fn timeouts_parse() {
        assert_eq!(parse_timeout("0").unwrap(), None);
        assert_eq!(parse_timeout("0s").unwrap(), None);
        assert_eq!(parse_timeout("10s").unwrap(), Some(Duration::from_secs(10)));
        assert_eq!(parse_timeout("30m").unwrap(), Some(Duration::from_secs(1800)));
        assert_eq!(parse_timeout("2h").unwrap(), Some(Duration::from_secs(7200)));
        assert_eq!(parse_timeout("1d").unwrap(), Some(Duration::from_secs(86_400)));
        for bad in ["", "30", "m", "5x", "-1s", "1.5h"] {
            assert!(parse_timeout(bad).is_err(), "{bad:?} should be refused");
        }
    }

    // spec: S08-06, S17-08
    #[test]
    fn detail_forms_match_the_prototype() {
        let ev = |kind: &str, ty: &str, payload: Value, extra: Value| {
            let mut e = json!({"seq": 814, "kind": kind, "type": ty, "by": "rowan", "doc": "k7q2m9xd", "name": "plan", "payload": payload});
            if let Some(o) = extra.as_object() {
                for (k, v) in o {
                    e[k] = v.clone();
                }
            }
            e
        };
        assert_eq!(
            detail(&ev("version", "version.pushed", json!({"version": 5, "message": "address review"}), json!({}))),
            "v5 by @rowan \"address review\""
        );
        assert_eq!(detail(&ev("version", "version.pushed", json!({"version": 5}), json!({}))), "v5 by @rowan");
        // Ticket 17: a push over the head's changes names what it discarded.
        assert_eq!(
            detail(&ev(
                "version",
                "version.pushed",
                json!({"version": 7, "discarded": [5, 6], "message": "mine"}),
                json!({})
            )),
            "v7 by @rowan discarding v5,v6 \"mine\""
        );
        assert_eq!(
            detail(&ev("action", "action.taken", json!({"action": "approve", "version": 5}), json!({}))),
            "approve v5 by @rowan"
        );
        assert_eq!(
            detail(&ev(
                "mention",
                "thread.replied",
                json!({"thread": "t2", "comment": "t2.2", "version": 5}),
                json!({"excerpt": "…@scribe…"})
            )),
            "t2.2 by @rowan \"…@scribe…\""
        );
        assert_eq!(
            detail(&ev("thread", "thread.created", json!({"thread": "t9", "version": 5}), json!({"excerpt": "Five?"}))),
            "t9 by @rowan \"Five?\""
        );
        assert_eq!(
            detail(&ev("resolve", "thread.resolved", json!({"thread": "t2", "version": 5}), json!({}))),
            "t2 by @rowan"
        );
        assert_eq!(detail(&ev("reopen", "thread.reopened", json!({"thread": "t2"}), json!({}))), "t2 by @rowan");
        let r = record(&ev(
            "action",
            "action.taken",
            json!({"action": "approve", "version": 5}),
            json!({"at": "2026-08-18T09:41:02.123Z"}),
        ));
        assert_eq!(r["at"], "2026-08-18T09:41:02Z");
        assert_eq!(r["action"], "approve");
        assert!(r["thread"].is_null());
    }
}
