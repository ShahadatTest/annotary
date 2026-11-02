//! `act DOC ACTION [--version N]` and `actions DOC` (ticket 05).
//!
//! - `act`: take an Action on the head Version (`action` right). `--version N` asserts head is
//!   N — the CLI checks first (exit 5), the server re-checks; an id not in the Document's
//!   effective declaration is refused by the server (exit 5, the hint lists the available ids).
//!   Prints one line `NAME vN ACTION by @handle` (the human prototype's shape); `--json` →
//!   `{name,id,version,action,by,at}` (+ `label`, `intent`).
//! - `actions`: the append-only log, newest first — `ACTION BY AT VER LABEL` (label/intent as
//!   they were when the Action was taken, even after the declaration changed); `--json` →
//!   `[{id,by,at,version,label,intent}]`.
//!
//! The formatting `status`/`show` share for the head Action (`action:ID@vN by @h` | `action:-`)
//! lives here too ([`action_cell`], [`action_record`]).

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;
use super::docref::{DeletedMode, resolve};

#[derive(Args, Debug)]
pub struct ActArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// An Action id from the Document's available set (`show DOC` lists them)
    pub action: String,
    /// Assert head is N (exit 5 if not) — never act on content you haven't seen
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
}

#[derive(Args, Debug)]
pub struct ActionsArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Also show a deleted Document's log
    #[arg(long)]
    pub deleted: bool,
}

pub const ACT_FIELDS: FieldSpec = FieldSpec {
    all: &["name", "id", "version", "action", "by", "at", "label", "intent"],
    default: &["name", "id", "version", "action", "by", "at"],
};

pub const ACTIONS_FIELDS: FieldSpec = FieldSpec {
    all: &["id", "by", "at", "version", "label", "intent"],
    default: &["id", "by", "at", "version", "label"],
};

/// The head-Action cell of `status` / `show`: `action:ID@vN by @h`, or `action:-`.
pub fn action_cell(action: &Value) -> String {
    match action.as_object() {
        Some(a) => format!(
            "action:{}@v{} by @{}",
            a.get("id").and_then(Value::as_str).unwrap_or("?"),
            a.get("version").and_then(Value::as_u64).unwrap_or(0),
            a.get("by").and_then(Value::as_str).unwrap_or("?")
        ),
        None => "action:-".to_string(),
    }
}

/// The `--json` `action` field of `status` / `show`: `{id, by, at}` (seconds), or null.
pub fn action_record(action: &Value) -> Value {
    match action.as_object() {
        Some(a) => json!({
            "id": a.get("id").cloned().unwrap_or(Value::Null),
            "by": a.get("by").cloned().unwrap_or(Value::Null),
            "at": iso_seconds(a.get("at").and_then(Value::as_str).unwrap_or("")),
        }),
        None => Value::Null,
    }
}

fn resolve_live(ctx: &Ctx, api: &crate::client::Api, doc: &str) -> CliResult<Value> {
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

pub fn act(ctx: &Ctx, a: ActArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&ACT_FIELDS, Vec::new(), true);
    }
    let action = a.action.trim();
    if action.is_empty() {
        return Err(CliError::usage("ACTION must not be empty").hint("`annotary show DOC` lists the available Actions"));
    }
    let api = ctx.api()?;
    let d = resolve_live(ctx, &api, &a.doc)?;
    let id = d["id"].as_str().unwrap_or("").to_string();
    let name = d["name"].as_str().unwrap_or("").to_string();
    let head = d["version"].as_u64().unwrap_or(0) as u32;
    if let Some(n) = a.version
        && n != head
    {
        return Err(CliError::refused(format!("head is v{head}, not v{n}"))
            .hint(format!("run `annotary show {name}` and act on v{head} once you have read it")));
    }
    let r = api.post(&format!("/api/v1/docs/{id}/actions"), &json!({ "action": action, "version": head }))?;
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let version = r["version"].as_u64().unwrap_or(u64::from(head));
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(name));
        rec.insert("id".into(), json!(id));
        rec.insert("version".into(), json!(version));
        rec.insert("action".into(), json!(s("id")));
        rec.insert("by".into(), json!(s("by")));
        rec.insert("at".into(), json!(iso_seconds(&s("at"))));
        rec.insert("label".into(), json!(s("label")));
        rec.insert("intent".into(), json!(s("intent")));
        return ctx.out.json_records(&ACT_FIELDS, vec![rec], true);
    }
    let fields = [name, format!("v{version}"), s("id"), format!("by @{}", s("by"))];
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&fields.join(sep));
    Ok(())
}

pub fn actions(ctx: &Ctx, a: ActionsArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&ACTIONS_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let mode = if a.deleted { DeletedMode::Any } else { DeletedMode::Exclude };
    let d = resolve(ctx, &api, &a.doc, None, mode)?;
    if !a.deleted && !d["deleted_at"].is_null() {
        return Err(CliError::not_found(format!(
            "Document {} ({}) is deleted",
            d["name"].as_str().unwrap_or(""),
            d["id"].as_str().unwrap_or("")
        ))
        .hint("`annotary restore DOC` brings it back; `--deleted` shows its log anyway"));
    }
    let id = d["id"].as_str().unwrap_or("").to_string();
    let body = api.get(&format!("/api/v1/docs/{id}/actions"))?;
    // Newest first (the log arrives oldest first).
    let mut log = body["actions"].as_array().cloned().unwrap_or_default();
    log.reverse();
    let records: Vec<Map<String, Value>> = log
        .iter()
        .map(|e| {
            let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let mut r = Map::new();
            r.insert("id".into(), json!(s("id")));
            r.insert("by".into(), json!(s("by")));
            r.insert("at".into(), json!(iso_seconds(&s("at"))));
            r.insert("version".into(), e.get("version").cloned().unwrap_or(json!(0)));
            r.insert("label".into(), json!(s("label")));
            r.insert("intent".into(), json!(s("intent")));
            r.insert("at_raw".into(), json!(s("at")));
            r
        })
        .collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&ACTIONS_FIELDS, records, false);
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            vec![
                s("id"),
                format!("@{}", s("by")),
                relative_age(&s("at_raw"), now),
                format!("v{}", r.get("version").and_then(Value::as_u64).unwrap_or(0)),
                s("label"),
            ]
        })
        .collect();
    if ctx.out.tty && rows.is_empty() {
        let available: Vec<String> = body["available"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["id"].as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        ctx.out.note(&format!(
            "no Actions on {} yet{}",
            d["name"].as_str().unwrap_or(""),
            if available.is_empty() {
                " (it declares no Actions)".to_string()
            } else {
                format!(" — available: {}", available.join(", "))
            }
        ));
        return Ok(());
    }
    let table = ctx.out.table(&["ACTION", "BY", "AT", "VER", "LABEL"], &rows, &[]);
    print!("{table}");
    Ok(())
}
