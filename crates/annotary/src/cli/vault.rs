//! `vault list`, `vault create NAME`, `vault rename OLD NEW`, `vault delete NAME` (CMD-5) — the
//! CLI's side of `/api/v1/vaults` (`server/vaults.rs`; the web Settings screen has the same).
//!
//! `list` prints `NAME DOCUMENTS DELETED CREATED` (the Vaults you can read; DELETED = deleted
//! Documents still in it, CREATED relative); `create` and `rename` print the Vault's `list` row,
//! `delete` prints `NAME deleted`. `create`, `rename` and `delete` are owner-only (exit 4); delete
//! refuses a Vault that still holds Documents — deleted ones until purged — and the last Vault
//! (exit 5). `rename` and `delete` also update this machine's profiles on the same server whose
//! `vault` was that Vault (by `vault_id`, or by name when the profile has no id yet); profiles
//! elsewhere follow on their next call (`client::heal_profile`).

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::client::{qs, server_url};
use crate::config::Config;
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;

#[derive(Subcommand, Debug)]
pub enum VaultCmd {
    /// Vaults you can read: name, Documents, deleted Documents, created
    List,
    /// Create a Vault — `*` Grants cover it at once (owner only)
    Create(NameArg),
    /// Rename a Vault: Grants follow; so do your profiles on this server (owner only)
    Rename(RenameArgs),
    /// Delete an empty Vault — purge its deleted Documents first; never the last one (owner only)
    Delete(NameArg),
}

#[derive(Args, Debug)]
pub struct NameArg {
    /// The Vault's name
    pub name: String,
}

#[derive(Args, Debug)]
pub struct RenameArgs {
    /// The Vault's name now
    pub old: String,
    /// Its new name
    pub new: String,
}

/// `--json` fields for `vault list` and `vault create`.
pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &["name", "documents", "deleted", "created", "id", "default_actions"],
    default: &["name", "documents", "deleted", "created", "id"],
};

/// `--json` fields for `vault rename`.
pub const RENAME_FIELDS: FieldSpec = FieldSpec {
    all: &["name", "from", "documents", "deleted", "created", "id", "default_actions", "profiles"],
    default: &["name", "from", "documents", "deleted", "id", "profiles"],
};

/// `--json` fields for `vault delete`.
pub const DELETE_FIELDS: FieldSpec = FieldSpec {
    all: &["name", "id", "deleted", "created", "profiles"],
    default: &["name", "id", "deleted", "profiles"],
};

pub fn run(ctx: &Ctx, cmd: VaultCmd) -> CliResult<()> {
    match cmd {
        VaultCmd::List => list(ctx),
        VaultCmd::Create(a) => create(ctx, a),
        VaultCmd::Rename(a) => rename(ctx, a),
        VaultCmd::Delete(a) => delete(ctx, a),
    }
}

/// A Vault API object → the flat record for text and JSON.
fn record(v: &Value) -> Map<String, Value> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut r = Map::new();
    r.insert("name".into(), json!(s("name")));
    r.insert("documents".into(), json!(v.get("documents").and_then(Value::as_u64).unwrap_or(0)));
    r.insert("deleted".into(), json!(v.get("deleted").and_then(Value::as_u64).unwrap_or(0)));
    r.insert("created".into(), json!(iso_seconds(&s("created_at"))));
    r.insert("id".into(), json!(s("id")));
    r.insert("default_actions".into(), v.get("default_actions").cloned().unwrap_or(json!([])));
    r.insert("created_raw".into(), json!(s("created_at")));
    r
}

fn row(r: &Map<String, Value>, now: jiff::Timestamp) -> Vec<String> {
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0).to_string();
    vec![s("name"), n("documents"), n("deleted"), relative_age(&s("created_raw"), now)]
}

/// One Vault's `list` row as a mutation's result.
fn print_row(ctx: &Ctx, r: &Map<String, Value>) {
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&row(r, jiff::Timestamp::now()).join(sep));
}

fn nonempty<'a>(what: &str, v: &'a str) -> CliResult<&'a str> {
    let v = v.trim();
    if v.is_empty() {
        return Err(CliError::usage(format!("{what} must not be empty")).hint("`annotary vault list` shows the Vaults"));
    }
    Ok(v)
}

fn list(ctx: &Ctx) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let body = api.get("/api/v1/vaults")?;
    let records: Vec<Map<String, Value>> =
        body["vaults"].as_array().map(|a| a.iter().map(record).collect()).unwrap_or_default();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, records, false);
    }
    if ctx.out.tty && records.is_empty() {
        ctx.out.note("no Vaults you can read — ask an owner for a grant");
        return Ok(());
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = records.iter().map(|r| row(r, now)).collect();
    let table = ctx.out.table(&["NAME", "DOCUMENTS", "DELETED", "CREATED"], &rows, &[]);
    print!("{table}");
    Ok(())
}

fn create(ctx: &Ctx, a: NameArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let name = nonempty("NAME", &a.name)?;
    let api = ctx.api()?;
    let created = api.post("/api/v1/vaults", &json!({ "name": name }))?;
    let rec = record(&created);
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, vec![rec], true);
    }
    print_row(ctx, &rec);
    Ok(())
}

fn rename(ctx: &Ctx, a: RenameArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&RENAME_FIELDS, Vec::new(), true);
    }
    let old = nonempty("OLD", &a.old)?;
    let new = nonempty("NEW", &a.new)?;
    let api = ctx.api()?;
    let renamed = api.post(&format!("/api/v1/vaults/{}/rename", qs(old)), &json!({ "name": new }))?;
    let mut rec = record(&renamed);
    let from = renamed["from"].as_str().unwrap_or(old).to_string();
    let name = rec["name"].as_str().unwrap_or("").to_string();
    let id = rec["id"].as_str().unwrap_or("").to_string();
    let profiles = if from == name {
        ctx.out.note(&format!("Vault \"{name}\" already has that name"));
        Vec::new()
    } else {
        update_profiles(ctx, &api.server, &id, &from, Some(&name))?
    };
    rec.insert("from".into(), json!(from));
    rec.insert("profiles".into(), json!(profiles));
    if ctx.out.is_json() {
        return ctx.out.json_records(&RENAME_FIELDS, vec![rec], true);
    }
    print_row(ctx, &rec);
    Ok(())
}

fn delete(ctx: &Ctx, a: NameArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&DELETE_FIELDS, Vec::new(), true);
    }
    let name = nonempty("NAME", &a.name)?;
    let api = ctx.api()?;
    let gone = api.delete(&format!("/api/v1/vaults/{}", qs(name)))?;
    let s = |k: &str| gone.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let profiles = update_profiles(ctx, &api.server, &s("id"), &s("name"), None)?;
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(s("name")));
        rec.insert("id".into(), json!(s("id")));
        rec.insert("deleted".into(), json!(true));
        rec.insert("created".into(), json!(iso_seconds(&s("created_at"))));
        rec.insert("profiles".into(), json!(profiles));
        return ctx.out.json_records(&DELETE_FIELDS, vec![rec], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&[s("name"), "deleted".to_string()].join(sep));
    Ok(())
}

/// After a rename (`to` = the new name) or a delete (`to` = `None`): every profile in this
/// machine's `config.toml` that points at `server` and whose `vault` is that Vault — by
/// `vault_id`, or by the old name when it has no id yet — follows (or forgets it). Returns their
/// names; a note names each one.
fn update_profiles(ctx: &Ctx, server: &str, id: &str, from: &str, to: Option<&str>) -> CliResult<Vec<String>> {
    let path = ctx.data.config_toml();
    let mut config = Config::load(&path)?;
    let mut changed = Vec::new();
    for (pname, p) in config.profiles.iter_mut() {
        if server_url(p.server.clone()) != server {
            continue;
        }
        let ours = match p.vault_id.as_deref() {
            Some(pid) => pid == id,
            None => p.vault.as_deref().map(str::trim) == Some(from),
        };
        if !ours || p.vault.is_none() {
            continue;
        }
        match to {
            Some(name) => {
                p.vault = Some(name.to_string());
                p.vault_id = Some(id.to_string());
                ctx.out.note(&format!("profile \"{pname}\" now uses Vault \"{name}\""));
            }
            None => {
                p.vault = None;
                p.vault_id = None;
                ctx.out.note(&format!("profile \"{pname}\" no longer names a Vault (\"{from}\" was deleted)"));
            }
        }
        changed.push(pname.clone());
    }
    if !changed.is_empty() {
        config.save(&path)?;
    }
    Ok(changed)
}
