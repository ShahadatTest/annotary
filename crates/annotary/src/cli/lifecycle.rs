//! `delete DOC` (soft; `write`), `restore DOC` (`write`), `purge DOC` (owner-only; permanent,
//! GCs blobs no other Version references). Result line: `name id deleted|restored|purged`.

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::CliResult;
use crate::output::{FieldSpec, JsonMode};

use super::Ctx;
use super::docref::{DeletedMode, resolve};

#[derive(Args, Debug)]
pub struct DocArg {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
}

pub const FIELDS: FieldSpec =
    FieldSpec { all: &["name", "id", "state", "vault", "version"], default: &["name", "id", "state"] };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Delete,
    Restore,
    Purge,
}

pub fn run(ctx: &Ctx, op: Op, a: DocArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    // `delete` targets live Documents; `restore`/`purge` deleted ones (an id always resolves).
    let mode = match op {
        Op::Delete => DeletedMode::Exclude,
        Op::Restore | Op::Purge => DeletedMode::Only,
    };
    let d = resolve(ctx, &api, &a.doc, None, mode)?;
    let id = d["id"].as_str().unwrap_or("").to_string();
    let result = match op {
        Op::Delete => api.delete(&format!("/api/v1/docs/{id}"))?,
        Op::Restore => api.post(&format!("/api/v1/docs/{id}/restore"), &json!({}))?,
        Op::Purge => api.post(&format!("/api/v1/docs/{id}/purge"), &json!({}))?,
    };
    let s = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let state = s("state");
    if op == Op::Purge {
        ctx.out.note(&format!(
            "purged {} ({}) — {} blob{} removed",
            s("name"),
            id,
            result["blobs_removed"].as_u64().unwrap_or(0),
            if result["blobs_removed"].as_u64() == Some(1) { "" } else { "s" }
        ));
    }
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(s("name")));
        rec.insert("id".into(), json!(id));
        rec.insert("state".into(), json!(state));
        rec.insert("vault".into(), json!(s("vault")));
        rec.insert("version".into(), result["version"].clone());
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    if ctx.out.tty {
        ctx.out.line(&format!("{}  {id}  {state}", s("name")));
    } else {
        ctx.out.line(&format!("{}\t{id}\t{state}", s("name")));
    }
    Ok(())
}
