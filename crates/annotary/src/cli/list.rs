//! `list [NAME] [--vault V] [--deleted] [--mentions-me] [--action ID|none] [--since T] [--limit N]`:
//! `id name vN action|- open updated`, most recent activity first, 50 rows by default with a
//! truncation note on stderr. Without `--vault` (or a profile `vault`) every readable Vault is
//! listed. `--mentions-me` / `--action` come alive with tickets 04/05 (today: no Document has an
//! Action or an open Thread, and the filters say so honestly).

use clap::Args;
use serde_json::{Map, Value, json};

use crate::client::qs;
use crate::exit::CliResult;
use crate::output::{Align, FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;
use super::docref::parse_since;

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Only Documents whose Name contains this text
    pub name: Option<String>,
    /// Vault to list (default: profile `vault`, else every Vault you can read)
    #[arg(long, value_name = "NAME")]
    pub vault: Option<String>,
    /// Deleted Documents (only those)
    #[arg(long)]
    pub deleted: bool,
    /// Only Documents with an open Thread mentioning you
    #[arg(long)]
    pub mentions_me: bool,
    /// Only Documents whose head has this Action (`none` = no Action)
    #[arg(long, value_name = "ID|none")]
    pub action: Option<String>,
    /// Activity since: 2h, 3d, 2026-08-17, 2026-08-17T10:00Z
    #[arg(long, value_name = "T")]
    pub since: Option<String>,
    /// Maximum rows (default 50)
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["id", "name", "version", "action", "open", "updated", "vault", "by", "message", "deleted", "created"],
    default: &["id", "name", "version", "action", "open", "updated"],
};

pub fn run(ctx: &Ctx, a: ListArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let mut path = format!("/api/v1/docs?limit={}", a.limit);
    if let Some(v) =
        a.vault.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).or_else(|| ctx.profile_vault())
    {
        path.push_str(&format!("&vault={}", qs(&v)));
    }
    if let Some(n) = a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        path.push_str(&format!("&name={}", qs(n)));
    }
    if a.deleted {
        path.push_str("&deleted=only");
    }
    if a.mentions_me {
        path.push_str("&mentions_me=1");
    }
    if let Some(act) = a.action.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        path.push_str(&format!("&action={}", qs(act)));
    }
    if let Some(s) = &a.since {
        path.push_str(&format!("&since={}", qs(&parse_since(s, jiff::Timestamp::now())?)));
    }
    let body = api.get(&path)?;
    let docs = body["docs"].as_array().cloned().unwrap_or_default();
    let total = body["total"].as_u64().unwrap_or(docs.len() as u64);
    let records: Vec<Map<String, Value>> = docs.iter().map(record).collect();
    if body["truncated"].as_bool().unwrap_or(false) {
        ctx.out.note(&format!("showing {} of {total} — `--limit N` for more", docs.len()));
    }
    if ctx.out.is_json() {
        return ctx.out.json_records(&FIELDS, records, false);
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            vec![
                s("id"),
                s("name"),
                format!("v{}", r.get("version").and_then(Value::as_u64).unwrap_or(0)),
                r.get("action").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| "-".into()),
                r.get("open").and_then(Value::as_u64).unwrap_or(0).to_string(),
                relative_age(&s("updated_raw"), now),
            ]
        })
        .collect();
    let table = ctx.out.table(
        &["ID", "NAME", "VER", "ACTION", "OPEN", "UPDATED"],
        &rows,
        &[Align::Left, Align::Left, Align::Left, Align::Left, Align::Right, Align::Left],
    );
    print!("{table}");
    Ok(())
}

/// A doc API object → the flat record used by text and JSON output.
pub fn record(d: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    r.insert("id".into(), json!(s("id")));
    r.insert("name".into(), json!(s("name")));
    r.insert("version".into(), d.get("version").cloned().unwrap_or(json!(0)));
    r.insert("action".into(), d.get("action").and_then(|a| a.get("id")).cloned().unwrap_or(Value::Null));
    r.insert("open".into(), d.get("open_threads").cloned().unwrap_or(json!(0)));
    r.insert("updated".into(), json!(iso_seconds(&s("updated_at"))));
    r.insert("vault".into(), json!(s("vault")));
    r.insert("by".into(), json!(s("by")));
    r.insert("message".into(), d.get("message").cloned().unwrap_or(Value::Null));
    r.insert("deleted".into(), json!(d.get("deleted_at").is_some_and(|v| !v.is_null())));
    r.insert("created".into(), json!(iso_seconds(&s("created_at"))));
    // Not a `--json` field: the raw stored timestamp for relative ages in text mode.
    r.insert("updated_raw".into(), json!(s("updated_at")));
    r
}
