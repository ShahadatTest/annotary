//! `versions DOC`: every Version, oldest first — `number by at message same_as`.

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::CliResult;
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;
use super::docref::{DeletedMode, resolve};

#[derive(Args, Debug)]
pub struct VersionsArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["number", "by", "at", "message", "same_as", "name", "hash"],
    default: &["number", "by", "at", "message", "same_as"],
};

pub fn run(ctx: &Ctx, a: VersionsArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let doc = resolve(ctx, &api, &a.doc, None, DeletedMode::Any)?;
    let id = doc["id"].as_str().unwrap_or("");
    let body = api.get(&format!("/api/v1/docs/{id}/versions"))?;
    let versions = body["versions"].as_array().cloned().unwrap_or_default();
    let records: Vec<Map<String, Value>> = versions
        .iter()
        .map(|v| {
            let mut r = Map::new();
            let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            r.insert("number".into(), v.get("number").cloned().unwrap_or(json!(0)));
            r.insert("by".into(), json!(s("by")));
            r.insert("at".into(), json!(iso_seconds(&s("at"))));
            r.insert("message".into(), v.get("message").cloned().unwrap_or(Value::Null));
            r.insert("same_as".into(), v.get("same_as").cloned().unwrap_or(Value::Null));
            r.insert("name".into(), json!(s("name")));
            r.insert("hash".into(), json!(s("hash")));
            r.insert("at_raw".into(), json!(s("at")));
            r
        })
        .collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&FIELDS, records, false);
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            vec![
                format!("v{}", r.get("number").and_then(Value::as_u64).unwrap_or(0)),
                format!("@{}", s("by")),
                relative_age(&s("at_raw"), now),
                r.get("message")
                    .and_then(Value::as_str)
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| "-".into()),
                r.get("same_as").and_then(Value::as_u64).map(|k| format!("v{k}")).unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    let table = ctx.out.table(&["VER", "BY", "AT", "MESSAGE", "SAME_AS"], &rows, &[]);
    print!("{table}");
    Ok(())
}
