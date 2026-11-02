//! `mentions [--since T] [--vault V]`: open Threads mentioning you across every Document you can
//! read — `DOC ID AUTHOR ANCHOR BODY REPLIES AGE`, most recent activity first (ticket 04).

use clap::Args;
use serde_json::{Map, Value};

use crate::client::qs;
use crate::exit::CliResult;
use crate::output::{FieldSpec, JsonMode};

use super::Ctx;
use super::docref::parse_since;
use super::thread::{list_record, print_rows, text_row};

#[derive(Args, Debug)]
pub struct MentionsArgs {
    /// Activity since: 2h, 3d, 2026-08-17, 2026-08-17T10:00Z
    #[arg(long, value_name = "T")]
    pub since: Option<String>,
    /// Only this Vault (default: profile `vault`, else every Vault you can read)
    #[arg(long, value_name = "NAME")]
    pub vault: Option<String>,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &[
        "doc", "id", "thread", "author", "anchor", "body", "mentions", "replies", "created", "updated", "version",
        "vault",
    ],
    default: &["doc", "id", "thread", "author", "anchor", "body", "replies", "created"],
};

pub fn run(ctx: &Ctx, a: MentionsArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let mut path = String::from("/api/v1/mentions?");
    if let Some(v) =
        a.vault.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).or_else(|| ctx.profile_vault())
    {
        path.push_str(&format!("&vault={}", qs(&v)));
    }
    if let Some(t) = &a.since {
        path.push_str(&format!("&since={}", qs(&parse_since(t, jiff::Timestamp::now())?)));
    }
    let body = api.get(&path)?;
    let hits = body["mentions"].as_array().cloned().unwrap_or_default();
    if ctx.out.is_json() {
        let records: Vec<Map<String, Value>> = hits
            .iter()
            .map(|h| {
                let t = list_record(&h["thread"]);
                let mut r = Map::new();
                r.insert("doc".into(), h["doc"]["name"].clone());
                r.insert("id".into(), h["doc"]["id"].clone());
                r.insert("thread".into(), t.get("id").cloned().unwrap_or(Value::Null));
                for k in ["author", "anchor", "body", "mentions", "replies", "created", "updated", "version"] {
                    r.insert(k.into(), t.get(k).cloned().unwrap_or(Value::Null));
                }
                r.insert("vault".into(), h["doc"]["vault"].clone());
                r
            })
            .collect();
        return ctx.out.json_records(&FIELDS, records, false);
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = hits
        .iter()
        .map(|h| {
            let mut row = vec![h["doc"]["name"].as_str().unwrap_or("").to_string()];
            row.extend(text_row(&h["thread"], &ctx.out, now));
            row
        })
        .collect();
    print_rows(&ctx.out, &["DOC", "ID", "AUTHOR", "ANCHOR", "BODY", "REPLIES", "AGE"], rows);
    if ctx.out.tty && hits.is_empty() {
        ctx.out.note("no open Threads mention you");
    }
    Ok(())
}
