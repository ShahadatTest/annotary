//! `thread list | show | new | reply | resolve | reopen | edit | delete` (ticket 04) and the row
//! formatting `show` and `mentions` share.
//!
//! - `thread list DOC [--all | --resolved] [--status anchored,refined,orphaned] [--mentions-me]
//!   [--since T] [--sort recent] [--version N]`: open Threads on head in document order —
//!   `ID AUTHOR ANCHOR BODY REPLIES AGE`; body = the root's first 120 characters, newlines
//!   collapsed, `(deleted)` for a tombstone; anchor column `vN:L "exact"` / `vN:L refined
//!   "exact"` / `vN:L orphaned "exact"` (vN:L = the last valid placement) / `document` /
//!   `asset REF`; orphaned Threads last. Non-TTY omits the REPLIES field when there are none
//!   (as the CLI prototype does); TTY shows a blank cell and footer hints about hidden resolved
//!   Threads and about orphaned ones. `--version N` is the read-only time-slice (ticket 07):
//!   Threads exactly as they stood on Version N, every line number stamped `vN:`.
//! - `thread show DOC ID...`: the full Thread — `vN:L:C-L:C "exact"`, created Version, mentions,
//!   every Reply, resolution, tombstones; a refined span adds `now: "<current text>"`.
//! - `thread new DOC "L:C-L:C" | --document | --asset REF -m MSG|- [--version N]`: the CLI reads
//!   the head Markdown, verifies the range exists there (and that head is `--version N`, exit 5
//!   otherwise) before posting; the server re-checks and freezes the quote. `--asset REF`
//!   anchors a whole-Asset Thread by the Asset's content hash (ticket 07).
//! - `thread reply DOC ID -m MSG|- [--resolve]`, `resolve | reopen | delete DOC ID`,
//!   `edit DOC ID -m MSG|-`. `-` reads the message from stdin (trailing newlines dropped).

use std::io::Read;

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use annotary_sidecar::identity::sha256_hex;
use annotary_sidecar::range::{Range, extract};

use crate::client::{Api, qs};
use crate::exit::{CliError, CliResult};
use crate::output::{Align, FieldSpec, JsonMode, Out, iso_seconds, relative_age};

use super::Ctx;
use super::docref::{DeletedMode, parse_since, resolve};

#[derive(Subcommand, Debug)]
pub enum ThreadCmd {
    /// Open Threads on head, document order, orphaned last (one per line)
    List(ListArgs),
    /// Everything on one or more Threads (t2 t7)
    Show(ShowArgs),
    /// Start a Thread: ANCHOR = "L:C-L:C" on head, --document, or --asset REF
    New(NewArgs),
    /// Reply; MSG may be '-' for stdin (add --resolve to close it in the same step)
    Reply(ReplyArgs),
    /// Resolve a Thread (a bare event: who/when is tracked, not controlled)
    Resolve(IdArgs),
    /// Reopen a resolved Thread
    Reopen(IdArgs),
    /// Edit your own Comment (owner: anyone's) — sets edited, no history
    Edit(EditArgs),
    /// Delete a Comment (tombstone: the Thread and ids stay)
    Delete(IdArgs),
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Include resolved Threads
    #[arg(long, conflicts_with = "resolved")]
    pub all: bool,
    /// Only resolved Threads
    #[arg(long)]
    pub resolved: bool,
    /// Placement status: anchored | refined | orphaned (comma list)
    #[arg(long, value_name = "S")]
    pub status: Option<String>,
    /// Only Threads that mention you (root or reply)
    #[arg(long)]
    pub mentions_me: bool,
    /// Activity since: 2h, 3d, 2026-08-17, 2026-08-17T10:00Z
    #[arg(long, value_name = "T")]
    pub since: Option<String>,
    /// `recent` = most recent activity first (default: document order)
    #[arg(long, value_name = "recent")]
    pub sort: Option<String>,
    /// Threads as they were on Version N (read-only time-slice)
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Thread ids (t2 t7)
    #[arg(required = true, value_name = "ID")]
    pub ids: Vec<String>,
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// "L:C-L:C" on head (lines from 1, columns from 0 in code points, half-open)
    #[arg(value_name = "ANCHOR", conflicts_with = "document")]
    pub range: Option<String>,
    /// A Thread about the whole Document (no anchor)
    #[arg(long)]
    pub document: bool,
    /// A Thread about a whole Asset (its path as referenced, e.g. img/x.png), anchored by content
    #[arg(long, value_name = "REF", conflicts_with_all = ["range", "document"])]
    pub asset: Option<String>,
    /// The Comment (Markdown); '-' reads stdin
    #[arg(short = 'm', long, value_name = "MSG", required = true)]
    pub message: String,
    /// Assert head is N (exit 5 if not) — never comment on content you haven't seen
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
}

#[derive(Args, Debug)]
pub struct ReplyArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Thread id (t7)
    pub id: String,
    /// The Reply (Markdown); '-' reads stdin
    #[arg(short = 'm', long, value_name = "MSG", required = true)]
    pub message: String,
    /// …and resolve the Thread in the same call
    #[arg(long)]
    pub resolve: bool,
}

#[derive(Args, Debug)]
pub struct EditArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Comment id (t7 = the root, t7.1 = a reply)
    pub id: String,
    /// The new body (Markdown); '-' reads stdin
    #[arg(short = 'm', long, value_name = "MSG", required = true)]
    pub message: String,
}

#[derive(Args, Debug)]
pub struct IdArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Thread id (t7) — for `delete`, a Comment id (t7 or t7.1)
    pub id: String,
}

/// `thread list` (and the rows of `show`).
pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &[
        "id", "author", "anchor", "body", "mentions", "replies", "resolved", "deleted", "created", "updated", "version",
    ],
    default: &[
        "id", "author", "anchor", "body", "mentions", "replies", "resolved", "deleted", "created", "updated", "version",
    ],
};

/// `thread show`: the full Thread (the Sidecar's shape, handles not ids).
pub const SHOW_FIELDS: FieldSpec = FieldSpec {
    all: &[
        "id",
        "subject",
        "anchor",
        "author",
        "created",
        "body",
        "mentions",
        "edited_at",
        "deleted",
        "resolved",
        "replies",
        "updated",
    ],
    default: &[
        "id",
        "subject",
        "anchor",
        "author",
        "created",
        "body",
        "mentions",
        "edited_at",
        "deleted",
        "resolved",
        "replies",
        "updated",
    ],
};

/// `thread new`.
pub const NEW_FIELDS: FieldSpec = FieldSpec {
    all: &["id", "subject", "anchor", "version", "created", "body", "mentions", "doc"],
    default: &["id", "subject", "anchor", "version"],
};

/// `thread reply | resolve | reopen | edit | delete`.
pub const EVENT_FIELDS: FieldSpec =
    FieldSpec { all: &["id", "thread", "state", "version", "doc"], default: &["id", "thread", "state", "version"] };

pub fn run(ctx: &Ctx, cmd: ThreadCmd) -> CliResult<()> {
    match cmd {
        ThreadCmd::List(a) => list(ctx, a),
        ThreadCmd::Show(a) => show(ctx, a),
        ThreadCmd::New(a) => new(ctx, a),
        ThreadCmd::Reply(a) => reply(ctx, a),
        ThreadCmd::Resolve(a) => set_resolved(ctx, a, true),
        ThreadCmd::Reopen(a) => set_resolved(ctx, a, false),
        ThreadCmd::Edit(a) => edit(ctx, a),
        ThreadCmd::Delete(a) => delete(ctx, a),
    }
}

// ---------------------------------------------------------------- shared helpers

/// The message text: `-` reads stdin (trailing newlines dropped); anything else is verbatim.
/// Empty (whitespace-only) messages are a usage error.
pub fn read_message(raw: &str) -> CliResult<String> {
    let text = if raw == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(|e| CliError::usage(format!("cannot read stdin: {e}")))?;
        s.trim_end_matches(['\n', '\r']).to_string()
    } else {
        raw.to_string()
    };
    if text.trim().is_empty() {
        return Err(CliError::usage("empty message").hint("pass -m MSG (Markdown), or -m - to read it from stdin"));
    }
    Ok(text)
}

/// A well-formed Thread id (`t7`) or Comment id (`t7.1`); composite forms are refused.
pub fn check_id(id: &str, comment_ok: bool) -> CliResult<String> {
    let id = id.trim();
    let ok = if comment_ok {
        annotary_store::parse_comment_id(id).is_some()
    } else {
        annotary_store::parse_thread_id(id).is_some()
    };
    if ok {
        return Ok(id.to_string());
    }
    let shape = if comment_ok { "t7 (a root) or t7.1 (a reply)" } else { "t7" };
    Err(CliError::usage(format!("{id:?} is not a {} id — use {shape}", if comment_ok { "Comment" } else { "Thread" }))
        .hint("thread verbs take DOC and ID separately: `annotary thread reply plan t7 -m …`"))
}

/// `plan/t7` is not how a Thread is named: DOC and ID are separate arguments (usage error).
pub fn reject_composite(doc: &str) -> CliResult<()> {
    if let Some((head, tail)) = doc.rsplit_once('/')
        && !doc.ends_with(".md")
        && annotary_store::parse_comment_id(tail).is_some()
    {
        return Err(CliError::usage(format!("{doc:?}: thread verbs take DOC and ID separately"))
            .hint(format!("e.g. `annotary thread show {head} {tail}`")));
    }
    Ok(())
}

/// Resolve DOC for a read-only Thread verb (deleted Documents included, like `versions`).
fn resolve_any(ctx: &Ctx, api: &Api, doc: &str) -> CliResult<Value> {
    reject_composite(doc)?;
    resolve(ctx, api, doc, None, DeletedMode::Any)
}

/// Resolve DOC for a Thread mutation: live Documents only (an id resolves anyway; a deleted one
/// is then refused by the server).
fn resolve_live(ctx: &Ctx, api: &Api, doc: &str) -> CliResult<Value> {
    reject_composite(doc)?;
    let d = resolve(ctx, api, doc, None, DeletedMode::Exclude)?;
    if !d["deleted_at"].is_null() {
        return Err(CliError::not_found(format!(
            "Document {} ({}) is deleted",
            d["name"].as_str().unwrap_or(""),
            d["id"].as_str().unwrap_or("")
        ))
        .hint("`annotary restore DOC` brings it back"));
    }
    Ok(d)
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// One line of text: newlines collapsed to spaces, whitespace trimmed at line joins.
pub fn one_line(text: &str) -> String {
    text.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

/// First `max` characters, with `…` when cut.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// The anchor column of a Thread JSON: `vN:L "exact"` (with `refined` / `orphaned` when the
/// current status says so — for an orphaned Thread `current` is its last valid placement, stamped
/// with the last Version it was valid on), `document`, or `asset REF`. `full` = the whole
/// `L:C-L:C` range.
pub fn anchor_cell(t: &Value, full: bool) -> String {
    let anchor = &t["anchor"];
    if anchor.is_null() {
        return "document".to_string();
    }
    let original = &anchor["original"];
    let current = &anchor["current"];
    if let Some(r) = original["ref"].as_str() {
        return format!("asset {r}");
    }
    let status = current["status"].as_str().unwrap_or("anchored");
    let place = current;
    let version = place["version"].as_u64().unwrap_or(0);
    let range = place["range"].as_str().unwrap_or("");
    let position = if full { range.to_string() } else { range.split(':').next().unwrap_or("").to_string() };
    let exact = one_line(original["exact"].as_str().unwrap_or(""));
    let mut out = format!("v{version}:{position}");
    if status != "anchored" {
        out.push(' ');
        out.push_str(status);
    }
    out.push_str(&format!(" \"{exact}\""));
    out
}

/// The flat `anchor` object of a `--json` list record: `{version, range, status, exact}` for a
/// span, `{asset: REF}` for an asset, absent for a Document-level Thread.
pub fn anchor_json(t: &Value) -> Value {
    let anchor = &t["anchor"];
    if anchor.is_null() {
        return Value::Null;
    }
    let original = &anchor["original"];
    let current = &anchor["current"];
    if let Some(r) = original["ref"].as_str() {
        return json!({ "asset": r });
    }
    let status = current["status"].as_str().unwrap_or("anchored");
    let mut m = Map::new();
    m.insert("version".into(), current["version"].clone());
    m.insert("range".into(), current["range"].clone());
    m.insert("status".into(), json!(status));
    m.insert("exact".into(), original["exact"].clone());
    if status == "refined" {
        m.insert("now".into(), current["exact"].clone());
    }
    Value::Object(m)
}

/// The body cell: the root's first 120 characters on one line, `(deleted)` for a tombstone.
pub fn body_cell(t: &Value) -> String {
    if !t["deleted"].is_null() {
        return "(deleted)".to_string();
    }
    truncate(&one_line(t["body"].as_str().unwrap_or("")), 120)
}

/// The `--json` list record of a Thread (`thread list`, `show`, `mentions` share it).
pub fn list_record(t: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    r.insert("id".into(), json!(s(t, "id")));
    r.insert("author".into(), json!(s(t, "author")));
    r.insert("anchor".into(), anchor_json(t));
    r.insert("body".into(), t["body"].clone());
    r.insert("mentions".into(), t["mentioned"].clone());
    r.insert("replies".into(), t["reply_count"].clone());
    r.insert(
        "resolved".into(),
        match t["resolved"].as_object() {
            Some(res) => json!({
                "by": res.get("by").cloned().unwrap_or(Value::Null),
                "at": iso_seconds(res.get("at").and_then(Value::as_str).unwrap_or("")),
                "version": res.get("version").cloned().unwrap_or(Value::Null),
            }),
            None => Value::Null,
        },
    );
    r.insert("deleted".into(), json!(!t["deleted"].is_null()));
    r.insert("created".into(), json!(iso_seconds(t["created"]["at"].as_str().unwrap_or(""))));
    r.insert("updated".into(), json!(iso_seconds(t["updated_at"].as_str().unwrap_or(""))));
    r.insert("version".into(), t["created"]["version"].clone());
    r
}

/// The text row of a Thread: `ID AUTHOR ANCHOR BODY REPLIES AGE`. TTY cells are cut to the
/// prototype's widths (anchor 56, body 44).
pub fn text_row(t: &Value, out: &Out, now: jiff::Timestamp) -> Vec<String> {
    let replies = t["reply_count"].as_u64().unwrap_or(0);
    let mut anchor = anchor_cell(t, false);
    let mut body = body_cell(t);
    if out.tty {
        anchor = truncate(&anchor, 56);
        body = truncate(&body, 44);
    }
    vec![
        s(t, "id"),
        format!("@{}", s(t, "author")),
        anchor,
        body,
        if replies > 0 { format!("+{replies}") } else { String::new() },
        relative_age(t["created"]["at"].as_str().unwrap_or(""), now),
    ]
}

/// Print Thread rows: TTY = aligned table with the dim header; non-TTY = tab-separated with the
/// empty REPLIES field omitted (the CLI prototype's shape). `header` may carry a leading column
/// (`DOC` for `mentions`) — `prefix` cells go first on each row.
pub fn print_rows(out: &Out, header: &[&str], rows: Vec<Vec<String>>) {
    if out.tty {
        if rows.is_empty() {
            return;
        }
        let mut align = vec![Align::Left; header.len()];
        if let Some(i) = header.iter().position(|h| *h == "REPLIES") {
            align[i] = Align::Left;
        }
        print!("{}", out.table(header, &rows, &align));
        return;
    }
    let replies_at = header.iter().position(|h| *h == "REPLIES");
    for r in rows {
        let cells: Vec<&str> = r
            .iter()
            .enumerate()
            .filter(|(i, c)| Some(*i) != replies_at || !c.is_empty())
            .map(|(_, c)| c.as_str())
            .collect();
        out.line(&cells.join("\t"));
    }
}

/// The footer hints after a `thread list` / `show` on a TTY: hidden resolved Threads
/// (`filtered_open`; `all_flag` = how to see them, e.g. `--all` or `thread list plan --all`)
/// and, unless a `--status` filter is on, orphaned Threads in the listing.
pub fn footer(ctx: &Ctx, body: &Value, filtered_open: bool, all_flag: &str, status_filtered: bool) {
    if !ctx.out.tty {
        return;
    }
    let threads = body["threads"].as_array().cloned().unwrap_or_default();
    if filtered_open {
        let hidden = body["resolved_hidden"].as_u64().unwrap_or(0);
        if hidden > 0 {
            ctx.out.note(&format!(
                "{hidden} resolved hidden — `{all_flag}` shows them{}",
                if threads.is_empty() { "; no open Threads" } else { "" }
            ));
        }
    }
    let orphaned = threads.iter().filter(|t| t["anchor"]["current"]["status"].as_str() == Some("orphaned")).count();
    if orphaned > 0 && !status_filtered {
        ctx.out.note(&format!(
            "{orphaned} orphaned — `thread show` prints their last placement; `--status anchored,refined` hides them"
        ));
    }
}

pub const LIST_HEADER: [&str; 6] = ["ID", "AUTHOR", "ANCHOR", "BODY", "REPLIES", "AGE"];

/// Build the `/threads` query for `thread list` flags.
pub fn list_query(
    all: bool,
    resolved: bool,
    mentions_me: bool,
    since: Option<&str>,
    sort: Option<&str>,
    status: Option<&str>,
    version: Option<u32>,
) -> CliResult<String> {
    let mut q = String::new();
    if all {
        q.push_str("&status=all");
    } else if resolved {
        q.push_str("&status=resolved");
    }
    if let Some(list) = status.map(str::trim).filter(|s| !s.is_empty()) {
        for s in list.split(',').map(str::trim) {
            if annotary_diff::Status::parse(s).is_none() {
                return Err(CliError::usage(format!("--status {s:?}: use anchored, refined, orphaned (comma list)"))
                    .hint("e.g. --status anchored,refined hides orphaned Threads"));
            }
        }
        q.push_str(&format!("&anchor={}", qs(list)));
    }
    if let Some(n) = version {
        q.push_str(&format!("&version={n}"));
    }
    if mentions_me {
        q.push_str("&mentions_me=1");
    }
    if let Some(t) = since {
        q.push_str(&format!("&since={}", qs(&parse_since(t, jiff::Timestamp::now())?)));
    }
    match sort.map(str::trim) {
        None | Some("") => {}
        Some("recent") => q.push_str("&sort=recent"),
        Some(other) => {
            return Err(CliError::usage(format!("--sort {other:?}: the only sort is `recent`"))
                .hint("default is document order (Document-level first, then by position)"));
        }
    }
    Ok(q)
}

// ---------------------------------------------------------------- list

fn list(ctx: &Ctx, a: ListArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let d = resolve_any(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let query = list_query(
        a.all,
        a.resolved,
        a.mentions_me,
        a.since.as_deref(),
        a.sort.as_deref(),
        a.status.as_deref(),
        a.version,
    )?;
    let body = api.get(&format!("/api/v1/docs/{id}/threads?{}", query.trim_start_matches('&')))?;
    let threads = body["threads"].as_array().cloned().unwrap_or_default();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, threads.iter().map(list_record).collect(), false);
    }
    let now = jiff::Timestamp::now();
    print_rows(&ctx.out, &LIST_HEADER, threads.iter().map(|t| text_row(t, &ctx.out, now)).collect());
    if let Some(n) = a.version
        && ctx.out.tty
    {
        let head = body["head"].as_u64().unwrap_or(0);
        if head != n as u64 {
            ctx.out.note(&format!("Threads as they were on v{n} (head is v{head}) — read-only"));
        }
    }
    footer(ctx, &body, !a.all && !a.resolved, "--all", a.status.is_some());
    Ok(())
}

// ---------------------------------------------------------------- show

fn show(ctx: &Ctx, a: ShowArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&SHOW_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let d = resolve_any(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let mut threads = Vec::new();
    for tid in &a.ids {
        let tid = check_id(tid, false)?;
        threads.push(api.get(&format!("/api/v1/docs/{id}/threads/{tid}"))?);
    }
    if ctx.out.is_json() {
        return ctx.out.json_records(&SHOW_FIELDS, threads.iter().map(show_record).collect(), false);
    }
    let now = jiff::Timestamp::now();
    for (i, t) in threads.iter().enumerate() {
        if i > 0 {
            ctx.out.line("");
        }
        print_thread(&ctx.out, t, now);
    }
    Ok(())
}

fn comment_record(c: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    r.insert("id".into(), json!(s(c, "id")));
    r.insert("author".into(), json!(s(c, "author")));
    r.insert(
        "created".into(),
        json!({ "at": iso_seconds(c["created"]["at"].as_str().unwrap_or("")), "version": c["created"]["version"] }),
    );
    r.insert("body".into(), c["body"].clone());
    r.insert("mentions".into(), c["mentions"].clone());
    r.insert("edited_at".into(), c["edited_at"].as_str().map(|t| json!(iso_seconds(t))).unwrap_or(Value::Null));
    r.insert(
        "deleted".into(),
        c["deleted"]
            .as_object()
            .map(|d| json!({ "at": iso_seconds(d.get("at").and_then(Value::as_str).unwrap_or("")), "by": d.get("by") }))
            .unwrap_or(Value::Null),
    );
    r
}

/// The `--json` record of `thread show`: the Sidecar's Thread shape (seconds, handles).
pub fn show_record(t: &Value) -> Map<String, Value> {
    let mut r = comment_record(t);
    r.insert("subject".into(), json!(s(t, "subject")));
    r.insert("anchor".into(), t["anchor"].clone());
    r.insert(
        "resolved".into(),
        t["resolved"]
            .as_object()
            .map(|res| {
                json!({ "by": res.get("by"), "at": iso_seconds(res.get("at").and_then(Value::as_str).unwrap_or("")), "version": res.get("version") })
            })
            .unwrap_or(Value::Null),
    );
    r.insert(
        "replies".into(),
        json!(
            t["replies"]
                .as_array()
                .map(|a| a.iter().map(|c| Value::Object(comment_record(c))).collect::<Vec<_>>())
                .unwrap_or_default()
        ),
    );
    r.insert("updated".into(), json!(iso_seconds(t["updated_at"].as_str().unwrap_or(""))));
    // Reorder to the Sidecar's key order.
    let order = [
        "id",
        "subject",
        "anchor",
        "author",
        "created",
        "body",
        "mentions",
        "edited_at",
        "deleted",
        "resolved",
        "replies",
        "updated",
    ];
    let mut ordered = Map::new();
    for k in order {
        if let Some(v) = r.remove(k) {
            ordered.insert(k.into(), v);
        }
    }
    ordered
}

/// Trailing optional segments of a Comment line: `mentions @a @b`, `edited AGE`, `deleted by @h AGE`.
fn comment_segments(c: &Value, now: jiff::Timestamp) -> Vec<String> {
    let mut segs = Vec::new();
    let mentions: Vec<String> = c["mentions"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(|h| format!("@{h}")).collect())
        .unwrap_or_default();
    if !mentions.is_empty() {
        segs.push(format!("mentions {}", mentions.join(" ")));
    }
    if let Some(e) = c["edited_at"].as_str() {
        segs.push(format!("edited {}", relative_age(e, now)));
    }
    if let Some(d) = c["deleted"].as_object() {
        segs.push(format!(
            "deleted by @{} {}",
            d.get("by").and_then(Value::as_str).unwrap_or("?"),
            relative_age(d.get("at").and_then(Value::as_str).unwrap_or(""), now)
        ));
    }
    segs
}

fn body_lines(out: &Out, c: &Value) {
    if !c["deleted"].is_null() {
        out.line("  (deleted)");
        return;
    }
    let body = c["body"].as_str().unwrap_or("");
    for l in body.split('\n') {
        out.line(&format!("  {}", l.trim_end_matches('\r')));
    }
}

/// Print one full Thread.
pub fn print_thread(out: &Out, t: &Value, now: jiff::Timestamp) {
    let sep = if out.tty { "  " } else { "\t" };
    let status = match t["resolved"].as_object() {
        Some(res) => format!(
            "resolved by @{} v{} {}",
            res.get("by").and_then(Value::as_str).unwrap_or("?"),
            res.get("version").and_then(Value::as_u64).unwrap_or(0),
            relative_age(res.get("at").and_then(Value::as_str).unwrap_or(""), now)
        ),
        None => "open".to_string(),
    };
    let mut fields = vec![
        s(t, "id"),
        status,
        format!("@{}", s(t, "author")),
        anchor_cell(t, true),
        format!(
            "created v{} {}",
            t["created"]["version"].as_u64().unwrap_or(0),
            relative_age(t["created"]["at"].as_str().unwrap_or(""), now)
        ),
    ];
    fields.extend(comment_segments(t, now));
    out.line(&fields.join(sep));
    body_lines(out, t);
    // A refined span (ticket 07) shows the text now under it.
    if t["anchor"]["current"]["status"].as_str() == Some("refined")
        && let Some(now_text) = t["anchor"]["current"]["exact"].as_str()
    {
        out.line(&format!("  now: \"{}\"", one_line(now_text)));
    }
    for c in t["replies"].as_array().into_iter().flatten() {
        let mut fields = vec![
            s(c, "id"),
            format!("@{}", s(c, "author")),
            format!(
                "v{} {}",
                c["created"]["version"].as_u64().unwrap_or(0),
                relative_age(c["created"]["at"].as_str().unwrap_or(""), now)
            ),
        ];
        fields.extend(comment_segments(c, now));
        out.line(&fields.join(sep));
        body_lines(out, c);
    }
}

// ---------------------------------------------------------------- new

fn new(ctx: &Ctx, a: NewArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&NEW_FIELDS, Vec::new(), true);
    }
    let asset = a.asset.as_deref().map(str::trim).map(|r| crate::assets::scan::normalize("", r));
    if asset.as_deref().is_some_and(str::is_empty) {
        return Err(CliError::usage("--asset needs the Asset's path as referenced (e.g. img/architecture.png)"));
    }
    let range_text = match (&a.range, a.document, &asset) {
        (Some(r), false, None) => Some(r.trim().to_string()),
        (None, true, None) | (None, false, Some(_)) => None,
        (None, false, None) => {
            return Err(CliError::usage(
                "say what the Thread is about: an ANCHOR \"L:C-L:C\" on head, --document, or --asset REF",
            )
            .hint("e.g. annotary thread new plan \"16:0-16:33\" -m \"…\"  ·  annotary thread new plan --document -m \"…\"  ·  annotary thread new plan --asset img/x.png -m \"…\""));
        }
        _ => unreachable!("clap conflicts_with"),
    };
    let range = match &range_text {
        Some(r) => Some(Range::parse(r).map_err(|e| CliError::usage(e.to_string()))?),
        None => None,
    };
    let message = read_message(&a.message)?;
    let api = ctx.api()?;
    let d = resolve_live(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let name = s(&d, "name");
    let head = d["version"].as_u64().unwrap_or(0) as u32;
    if let Some(n) = a.version
        && n != head
    {
        return Err(CliError::refused(format!("head is v{head}, not v{n}"))
            .hint(format!("run `annotary pull {name}` and re-read before commenting")));
    }
    // Verify the range on head before posting: the quote must exist and be non-empty.
    if let Some(r) = &range {
        let v = api.get(&format!("/api/v1/docs/{id}/versions/{head}"))?;
        let md_hash = s(&v, "md_hash");
        let bytes = api.get_bytes(&format!("/api/v1/blobs/{md_hash}"))?;
        if sha256_hex(&bytes) != md_hash {
            return Err(CliError::internal(format!("blob {md_hash} did not verify")));
        }
        let text = String::from_utf8_lossy(&bytes);
        let exact = extract(&text, r).map_err(|e| {
            CliError::refused(format!("range {r} is not on head v{head}: {e}"))
                .hint(format!("run `annotary pull {name}` and quote a range that exists in v{head}"))
        })?;
        if exact.trim().is_empty() {
            return Err(CliError::refused(format!(
                "range {r} on head v{head} is empty (zero-width or whitespace only)"
            ))
            .hint("quote at least one character, or use --document"));
        }
    }
    let body = json!({
        "subject": if range.is_some() { "span" } else if asset.is_some() { "asset" } else { "document" },
        "range": range.map(|r| r.to_string()),
        "ref": asset,
        "body": message,
        "version": head,
    });
    let t = api.post(&format!("/api/v1/docs/{id}/threads"), &body)?;
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("id".into(), json!(s(&t, "id")));
        rec.insert("subject".into(), json!(s(&t, "subject")));
        rec.insert("anchor".into(), anchor_json(&t));
        rec.insert("version".into(), t["created"]["version"].clone());
        rec.insert("created".into(), json!(iso_seconds(t["created"]["at"].as_str().unwrap_or(""))));
        rec.insert("body".into(), t["body"].clone());
        rec.insert("mentions".into(), t["mentions"].clone());
        rec.insert("doc".into(), json!(id));
        return ctx.out.json_records(&NEW_FIELDS, vec![rec], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&format!("{}{sep}{}", s(&t, "id"), anchor_cell(&t, true)));
    Ok(())
}

// ---------------------------------------------------------------- reply / resolve / reopen / edit / delete

fn event_out(ctx: &Ctx, id: &str, thread: &str, state: &str, version: Option<u64>, doc: &str) -> CliResult<()> {
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("id".into(), json!(id));
        rec.insert("thread".into(), json!(thread));
        rec.insert("state".into(), json!(state));
        rec.insert("version".into(), version.map(|v| json!(v)).unwrap_or(Value::Null));
        rec.insert("doc".into(), json!(doc));
        return ctx.out.json_records(&EVENT_FIELDS, vec![rec], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    if state.is_empty() {
        ctx.out.line(id);
    } else {
        ctx.out.line(&format!("{id}{sep}{state}"));
    }
    Ok(())
}

fn reply(ctx: &Ctx, a: ReplyArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&EVENT_FIELDS, Vec::new(), true);
    }
    let tid = check_id(&a.id, false)?;
    let message = read_message(&a.message)?;
    let api = ctx.api()?;
    let d = resolve_live(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let r = api
        .post(&format!("/api/v1/docs/{id}/threads/{tid}/replies"), &json!({ "body": message, "resolve": a.resolve }))?;
    let cid = s(&r["comment"], "id");
    let version = r["comment"]["created"]["version"].as_u64();
    let state = if a.resolve { "resolved" } else { "" };
    event_out(ctx, &cid, &tid, state, version, &id)
}

fn set_resolved(ctx: &Ctx, a: IdArgs, resolve_it: bool) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&EVENT_FIELDS, Vec::new(), true);
    }
    let tid = check_id(&a.id, false)?;
    let api = ctx.api()?;
    let d = resolve_live(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let verb = if resolve_it { "resolve" } else { "reopen" };
    let t = api.post(&format!("/api/v1/docs/{id}/threads/{tid}/{verb}"), &json!({}))?;
    if t["changed"].as_bool() == Some(false) {
        ctx.out.note(&format!("{tid} was already {}", if resolve_it { "resolved" } else { "open" }));
    }
    let version = if resolve_it { t["resolved"]["version"].as_u64() } else { d["version"].as_u64() };
    event_out(ctx, &tid, &tid, if resolve_it { "resolved" } else { "reopened" }, version, &id)
}

fn edit(ctx: &Ctx, a: EditArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&EVENT_FIELDS, Vec::new(), true);
    }
    let cid = check_id(&a.id, true)?;
    let message = read_message(&a.message)?;
    let api = ctx.api()?;
    let d = resolve_live(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let c = api.put_json(&format!("/api/v1/docs/{id}/threads/{cid}"), &json!({ "body": message }))?;
    let thread = cid.split('.').next().unwrap_or(&cid).to_string();
    event_out(ctx, &s(&c, "id"), &thread, "edited", c["created"]["version"].as_u64(), &id)
}

fn delete(ctx: &Ctx, a: IdArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&EVENT_FIELDS, Vec::new(), true);
    }
    let cid = check_id(&a.id, true)?;
    let api = ctx.api()?;
    let d = resolve_live(ctx, &api, &a.doc)?;
    let id = s(&d, "id");
    let c = api.delete(&format!("/api/v1/docs/{id}/threads/{cid}"))?;
    if c["changed"].as_bool() == Some(false) {
        ctx.out.note(&format!("{cid} was already deleted"));
    }
    let thread = cid.split('.').next().unwrap_or(&cid).to_string();
    event_out(ctx, &s(&c, "id"), &thread, "deleted", c["created"]["version"].as_u64(), &id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exit::Exit;

    fn span(exact: &str, range: &str, status: &str) -> Value {
        json!({
            "id": "t2", "author": "rowan", "body": "Agreed — but\nthe skill file", "reply_count": 1,
            "created": {"at": "2026-08-14T09:16:30.000Z", "version": 1}, "updated_at": "2026-08-15T16:42:00.000Z",
            "mentioned": ["scribe"], "deleted": null, "resolved": null,
            "anchor": {
                "original": {"version": 1, "range": range, "exact": exact},
                "current": {"version": 4, "range": "16:0-16:33", "status": status},
                "history": []
            }
        })
    }

    // spec: S04-11, S04-12
    #[test]
    fn anchor_and_body_cells_follow_the_prototype() {
        let t = span("We ship the CLI before the web UI", "16:0-16:33", "anchored");
        assert_eq!(anchor_cell(&t, false), "v4:16 \"We ship the CLI before the web UI\"");
        assert_eq!(anchor_cell(&t, true), "v4:16:0-16:33 \"We ship the CLI before the web UI\"");
        assert_eq!(
            anchor_json(&t),
            json!({"version":4,"range":"16:0-16:33","status":"anchored","exact":"We ship the CLI before the web UI"})
        );
        let o = span("agents should", "21:29-21:79", "orphaned");
        assert_eq!(
            anchor_cell(&o, false),
            "v4:16 orphaned \"agents should\"",
            "orphaned = the last valid placement (`current`, stamped with its Version)"
        );
        assert_eq!(anchor_json(&o)["version"], 4);
        let mut r = span("agents should", "21:29-21:79", "refined");
        r["anchor"]["current"]["exact"] = json!("agents must");
        assert_eq!(anchor_cell(&r, true), "v4:16:0-16:33 refined \"agents should\"");
        assert_eq!(anchor_json(&r)["now"], "agents must");
        let asset = json!({"anchor": {"original": {"version": 1, "hash": "ab", "ref": "img/a.png", "line": 3}, "current": {"version": 2, "line": 5, "status": "anchored"}}});
        assert_eq!(anchor_cell(&asset, false), "asset img/a.png");
        assert_eq!(anchor_json(&asset), json!({"asset": "img/a.png"}));
        assert_eq!(anchor_cell(&json!({"anchor": null}), false), "document");
        assert_eq!(anchor_json(&json!({"anchor": null})), Value::Null);
        assert_eq!(body_cell(&t), "Agreed — but the skill file");
        assert_eq!(body_cell(&json!({"body": "", "deleted": {"at": "x", "by": "linter"}})), "(deleted)");
        let long = json!({"body": "x".repeat(200), "deleted": null});
        assert_eq!(body_cell(&long).chars().count(), 120);
        assert!(body_cell(&long).ends_with('…'));
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(one_line("a\n\n  b \r\nc"), "a b c");
        // Rows: non-TTY drops the empty REPLIES field, TTY keeps a blank cell.
        let now = annotary_store::time::parse("2026-08-18T09:16:30.000Z").unwrap();
        let row = text_row(&t, &Out::plain(), now);
        assert_eq!(
            row,
            vec![
                "t2",
                "@rowan",
                "v4:16 \"We ship the CLI before the web UI\"",
                "Agreed — but the skill file",
                "+1",
                "4d"
            ]
        );
        let mut none = t.clone();
        none["reply_count"] = json!(0);
        assert_eq!(text_row(&none, &Out::plain(), now)[4], "");
        assert_eq!(check_id("t7", false).unwrap(), "t7");
        assert_eq!(check_id("t7.2", true).unwrap(), "t7.2");
        assert_eq!(check_id("t7.2", false).unwrap_err().exit, Exit::Usage);
        assert_eq!(check_id("plan/t7", false).unwrap_err().exit, Exit::Usage);
        assert_eq!(reject_composite("plan/t7").unwrap_err().exit, Exit::Usage);
        assert_eq!(reject_composite("plan/t7.1").unwrap_err().exit, Exit::Usage);
        assert!(
            reject_composite("docs/plan.md").is_ok()
                && reject_composite("plan").is_ok()
                && reject_composite("a/b").is_ok()
        );
        assert_eq!(read_message("hi").unwrap(), "hi");
        assert_eq!(read_message("  ").unwrap_err().exit, Exit::Usage);
        assert!(
            list_query(false, false, true, None, Some("recent"), None, None)
                .unwrap()
                .contains("mentions_me=1&sort=recent")
        );
        assert_eq!(list_query(false, false, false, None, Some("age"), None, None).unwrap_err().exit, Exit::Usage);
        assert_eq!(
            list_query(false, false, false, None, None, Some("anchored,refined"), Some(2)).unwrap(),
            "&anchor=anchored%2Crefined&version=2"
        );
        assert_eq!(list_query(false, false, false, None, None, Some("moved"), None).unwrap_err().exit, Exit::Usage);
    }
}
