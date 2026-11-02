//! `show DOC [--deleted]` (the agent's read path) and `status DOC [--deleted]` (the poll).
//!
//! `show` prints the header line `name id vault vN by @h AGE "message" action:ID@vN by @h|-
//! open:N actions:a,b` followed by the open Threads as `thread list` rows (ticket 04; the
//! ≈245-token read path). `status` prints `name id vN action:ID@vN by @h|- open:N mentions-me:N
//! seq:N` (`action` = the last Action on head, ticket 05; `--json` `action {id, by, at} | null`).
//! Deleted Documents are hidden from both unless `--deleted`. On a TTY `show` appends the web URL
//! (ticket 18: the server's public link — `annotary link` — when a public URL is set).

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::act::{action_cell, action_record};
use super::docref::{DeletedMode, resolve};
use super::{Ctx, thread};

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Also show a deleted Document
    #[arg(long)]
    pub deleted: bool,
}

pub const SHOW_FIELDS: FieldSpec = FieldSpec {
    all: &[
        "name", "id", "vault", "version", "by", "at", "message", "action", "open", "actions", "threads", "same_as",
        "deleted", "url",
    ],
    default: &["name", "id", "vault", "version", "by", "at", "message", "action", "open", "actions", "threads"],
};

pub const STATUS_FIELDS: FieldSpec = FieldSpec {
    all: &["name", "id", "version", "action", "open_threads", "mentions_me", "seq", "vault", "deleted"],
    default: &["name", "id", "version", "action", "open_threads", "mentions_me", "seq"],
};

/// Resolve DOC for `show`/`status`: deleted Documents are hidden unless asked for.
fn resolve_visible(ctx: &Ctx, api: &crate::client::Api, doc: &str, deleted: bool) -> CliResult<Value> {
    let mode = if deleted { DeletedMode::Any } else { DeletedMode::Exclude };
    let d = resolve(ctx, api, doc, None, mode)?;
    if !deleted && !d["deleted_at"].is_null() {
        return Err(CliError::not_found(format!(
            "Document {} ({}) is deleted",
            d["name"].as_str().unwrap_or(""),
            d["id"].as_str().unwrap_or("")
        ))
        .hint("`annotary restore DOC` brings it back; `--deleted` shows it anyway"));
    }
    Ok(d)
}

pub fn show(ctx: &Ctx, a: ShowArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&SHOW_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let d = resolve_visible(ctx, &api, &a.doc, a.deleted)?;
    let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let id = s("id");
    let version = d["version"].as_u64().unwrap_or(0);
    let open = d["open_threads"].as_u64().unwrap_or(0);
    let actions: Vec<String> = d["available_actions"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["id"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    // The open Threads on head (ticket 04) — the same rows as `thread list DOC`.
    let threads_body = api.get(&format!("/api/v1/docs/{id}/threads"))?;
    let threads = threads_body["threads"].as_array().cloned().unwrap_or_default();
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(s("name")));
        rec.insert("id".into(), json!(id));
        rec.insert("vault".into(), json!(s("vault")));
        rec.insert("version".into(), json!(version));
        rec.insert("by".into(), json!(s("by")));
        rec.insert("at".into(), json!(iso_seconds(&s("at"))));
        rec.insert("message".into(), d["message"].clone());
        rec.insert("action".into(), action_record(&d["action"]));
        rec.insert("open".into(), json!(open));
        rec.insert("actions".into(), json!(actions));
        rec.insert(
            "threads".into(),
            json!(threads.iter().map(|t| Value::Object(thread::list_record(t))).collect::<Vec<_>>()),
        );
        rec.insert("same_as".into(), d["same_as"].clone());
        rec.insert("deleted".into(), json!(!d["deleted_at"].is_null()));
        // ticket 18: the public link when the server has one; `url` is in `all` only, so `health`
        // is asked only when `--json …,url` names it.
        if matches!(&ctx.out.json, Some(crate::output::JsonMode::Fields(f)) if f.iter().any(|x| x == "url")) {
            rec.insert("url".into(), json!(api.display_link(&id)));
        }
        return ctx.out.json_records(&SHOW_FIELDS, vec![rec], true);
    }
    let mut version_cell = format!("v{version} by @{} {}", s("by"), relative_age(&s("at"), jiff::Timestamp::now()));
    if let Some(m) = d["message"].as_str().filter(|m| !m.is_empty()) {
        version_cell.push_str(&format!(" \"{m}\""));
    }
    let mut fields = vec![
        s("name"),
        id.clone(),
        s("vault"),
        version_cell,
        action_cell(&d["action"]),
        format!("open:{open}"),
        format!("actions:{}", if actions.is_empty() { "-".to_string() } else { actions.join(",") }),
    ];
    if !d["deleted_at"].is_null() {
        fields.push("deleted".into());
    }
    if ctx.out.tty {
        ctx.out.line(&format!("{}  {}", fields.join("  "), api.display_link(&id))); // ticket 18: public link
    } else {
        ctx.out.line(&fields.join("\t"));
    }
    let now = jiff::Timestamp::now();
    thread::print_rows(
        &ctx.out,
        &thread::LIST_HEADER,
        threads.iter().map(|t| thread::text_row(t, &ctx.out, now)).collect(),
    );
    thread::footer(ctx, &threads_body, true, &format!("thread list {} --all", s("name")), false);
    Ok(())
}

pub fn status(ctx: &Ctx, a: ShowArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&STATUS_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let d = resolve_visible(ctx, &api, &a.doc, a.deleted)?;
    let id = d["id"].as_str().unwrap_or("").to_string();
    let st = api.get(&format!("/api/v1/docs/{id}/status"))?;
    let s = |k: &str| st.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let version = st["version"].as_u64().unwrap_or(0);
    let open = st["open_threads"].as_u64().unwrap_or(0);
    let mentions = st["mentions_me"].as_u64().unwrap_or(0);
    let seq = st["seq"].as_u64().unwrap_or(0);
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(s("name")));
        rec.insert("id".into(), json!(id));
        rec.insert("version".into(), json!(version));
        rec.insert("action".into(), action_record(&st["action"]));
        rec.insert("open_threads".into(), json!(open));
        rec.insert("mentions_me".into(), json!(mentions));
        rec.insert("seq".into(), json!(seq));
        rec.insert("vault".into(), json!(s("vault")));
        rec.insert("deleted".into(), json!(!st["deleted_at"].is_null()));
        return ctx.out.json_records(&STATUS_FIELDS, vec![rec], true);
    }
    let fields = [
        s("name"),
        id,
        format!("v{version}"),
        action_cell(&st["action"]),
        format!("open:{open}"),
        format!("mentions-me:{mentions}"),
        format!("seq:{seq}"),
    ];
    if ctx.out.tty {
        ctx.out.line(&fields.join("  "));
    } else {
        ctx.out.line(&fields.join("\t"));
    }
    Ok(())
}
