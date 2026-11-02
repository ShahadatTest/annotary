//! `member add HANDLE [--name NAME]`, `member list`, and (ticket 06) `member rename HANDLE NEW`,
//! `member disable HANDLE`, `member grant HANDLE VAULT|* RIGHTS`, `member role HANDLE owner|member`.
//!
//! The mutating verbs (owner-only) print the Member's updated `member list` row
//! (`@handle name role grants`, `role (disabled)` once disabled) — one line, tab-separated
//! (TTY: two spaces); `--json` gives the same record as `member list`.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use annotary_store::Right;

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds};

use super::Ctx;

#[derive(Subcommand, Debug)]
pub enum MemberCmd {
    /// One step: Member + Membership + grant * (read,comment,action,write) + token, printed once
    /// as ANNOTARY_TOKEN=… — this is how you add an agent (owner only)
    Add(AddArgs),
    /// Members: handle, name, role, grants
    List,
    // ---- ticket 06 ----
    /// Change a Member's display name — the handle itself never changes (owner only)
    Rename(RenameArgs),
    /// Disable a Member: tokens stop working, history and handle stay (owner only)
    Disable(HandleArg),
    /// Set a Member's rights on one Vault (or `*` = every Vault, incl. future ones), e.g.
    /// `grant linter notes read,comment`; RIGHTS `none` removes that grant (owner only)
    Grant(GrantArgs),
    /// Make a Member an owner, or an owner a member — the last owner cannot step down (owner only)
    Role(RoleArgs),
    // ---- end ticket 06 ----
}

#[derive(Args, Debug)]
pub struct HandleArg {
    /// The Member's handle (with or without @)
    pub handle: String,
}

#[derive(Args, Debug)]
pub struct RenameArgs {
    /// The Member's handle (with or without @)
    pub handle: String,
    /// The new display name (quote it if it has spaces)
    pub name: String,
}

#[derive(Args, Debug)]
pub struct GrantArgs {
    /// The Member's handle (with or without @)
    pub handle: String,
    /// A Vault name, or `*` for every Vault including ones created later
    pub vault: String,
    /// Comma list of read, comment, action, write (read is implied by the others); `none` removes the grant
    pub rights: String,
}

#[derive(Args, Debug)]
pub struct RoleArgs {
    /// The Member's handle (with or without @)
    pub handle: String,
    /// owner | member
    #[arg(value_parser = ["owner", "member"])]
    pub role: String,
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// kebab-case, unique per Tenant forever (e.g. scribe)
    pub handle: String,
    /// Display name (default: the handle)
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
}

/// `--json` fields for `member list`.
pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &["handle", "name", "role", "grants", "disabled", "created", "id"],
    default: &["handle", "name", "role", "grants", "disabled"],
};

/// `--json` fields for `member add`.
pub const ADD_FIELDS: FieldSpec = FieldSpec {
    all: &["handle", "name", "role", "grants", "token", "created", "id"],
    default: &["handle", "name", "role", "grants", "token"],
};

pub fn run(ctx: &Ctx, cmd: MemberCmd) -> CliResult<()> {
    match cmd {
        MemberCmd::List => list(ctx),
        MemberCmd::Add(a) => add(ctx, a),
        // ---- ticket 06 ----
        MemberCmd::Rename(a) => rename(ctx, a),
        MemberCmd::Disable(a) => disable(ctx, a),
        MemberCmd::Grant(a) => grant(ctx, a),
        MemberCmd::Role(a) => role(ctx, a),
        // ---- end ticket 06 ----
    }
}

/// A handle argument without its optional `@`.
pub fn clean_handle(h: &str) -> String {
    h.trim().trim_start_matches('@').to_string()
}

/// Text form of a grants array: `*:read,comment,action,write notes:read` (or `-`).
pub fn grants_text(grants: &Value) -> String {
    let parts: Vec<String> = grants
        .as_array()
        .map(|a| {
            a.iter()
                .map(|g| {
                    let vault = g.get("vault").and_then(Value::as_str).unwrap_or("?");
                    let rights: Vec<&str> = g
                        .get("rights")
                        .and_then(Value::as_array)
                        .map(|r| r.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    format!("{vault}:{}", rights.join(","))
                })
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() { "-".to_string() } else { parts.join(" ") }
}

/// A member API object → the flat record used by both text and JSON output.
fn record(m: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    r.insert("handle".into(), json!(s("handle")));
    r.insert("name".into(), json!(s("name")));
    r.insert("role".into(), json!(s("role")));
    r.insert("grants".into(), m.get("grants").cloned().unwrap_or(json!([])));
    r.insert("disabled".into(), json!(m.get("disabled_at").is_some_and(|v| !v.is_null())));
    r.insert("created".into(), json!(iso_seconds(&s("created_at"))));
    r.insert("id".into(), json!(s("id")));
    if let Some(t) = m.get("token") {
        r.insert("token".into(), t.clone());
    }
    r
}

fn list(ctx: &Ctx) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&crate::output::JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let body = api.get("/api/v1/members")?;
    let members = body.get("members").and_then(Value::as_array).cloned().unwrap_or_default();
    let records: Vec<Map<String, Value>> = members.iter().map(record).collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, records, false);
    }
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let disabled = r.get("disabled").and_then(Value::as_bool).unwrap_or(false);
            let role = if disabled { format!("{} (disabled)", s("role")) } else { s("role") };
            vec![format!("@{}", s("handle")), s("name"), role, grants_text(&r["grants"])]
        })
        .collect();
    let table = ctx.out.table(&["HANDLE", "NAME", "ROLE", "GRANTS"], &rows, &[]);
    print!("{table}");
    Ok(())
}

fn add(ctx: &Ctx, a: AddArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&crate::output::JsonMode::ListFields) {
        return ctx.out.json_records(&ADD_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let mut body = json!({ "handle": a.handle.trim() });
    if let Some(n) = &a.name {
        body["name"] = json!(n);
    }
    let created = api.post("/api/v1/members", &body)?;
    let rec = record(&created);
    let handle = rec["handle"].as_str().unwrap_or("").to_string();
    let role = rec["role"].as_str().unwrap_or("").to_string();
    let token = created.get("token").and_then(Value::as_str).unwrap_or("").to_string();
    ctx.out.note(&format!("@{handle} added: {role}, grants {}", grants_pretty(&rec["grants"])));
    ctx.out.note("Token (shown once):");
    if ctx.out.is_json() {
        return ctx.out.json_records(&ADD_FIELDS, vec![rec], true);
    }
    if ctx.out.tty {
        ctx.out.line("");
        ctx.out.line(&format!("  ANNOTARY_TOKEN={token}   # put this in the agent's environment"));
        ctx.out.line("");
    } else {
        ctx.out.line(&format!("ANNOTARY_TOKEN={token}"));
    }
    Ok(())
}

// ---- ticket 06 ----

/// Print one Member's row (the `member list` shape) as the result of a mutation.
fn print_member(ctx: &Ctx, m: &Value) -> CliResult<()> {
    let rec = record(m);
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, vec![rec], true);
    }
    let s = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let disabled = rec.get("disabled").and_then(Value::as_bool).unwrap_or(false);
    let role = if disabled { format!("{} (disabled)", s("role")) } else { s("role") };
    let fields = [format!("@{}", s("handle")), s("name"), role, grants_text(&rec["grants"])];
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&fields.join(sep));
    Ok(())
}

fn rename(ctx: &Ctx, a: RenameArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let name = a.name.trim();
    if name.is_empty() {
        return Err(CliError::usage("NEW must not be empty").hint("member rename HANDLE \"Display Name\""));
    }
    let api = ctx.api()?;
    let handle = clean_handle(&a.handle);
    let m = api.post(&format!("/api/v1/members/{handle}/rename"), &json!({ "name": name }))?;
    print_member(ctx, &m)
}

fn disable(ctx: &Ctx, a: HandleArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let handle = clean_handle(&a.handle);
    let m = api.post(&format!("/api/v1/members/{handle}/disable"), &json!({}))?;
    if m["changed"] == json!(false) {
        ctx.out.note(&format!("@{handle} was already disabled"));
    }
    print_member(ctx, &m)
}

/// Parse `read,comment` (or `none`) into rights; unknown names are a usage error.
pub fn parse_rights(text: &str) -> CliResult<Vec<Right>> {
    let t = text.trim();
    if t.is_empty() {
        return Err(
            CliError::usage("RIGHTS must not be empty").hint("read, comment, action, write (comma list) or none")
        );
    }
    if t == "none" || t == "-" {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in t.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let r = Right::parse(p).ok_or_else(|| {
            CliError::usage(format!("unknown right {p:?}"))
                .hint("rights: read, comment, action, write (comma list) or none")
        })?;
        if !out.contains(&r) {
            out.push(r);
        }
    }
    Ok(out)
}

fn grant(ctx: &Ctx, a: GrantArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let rights = parse_rights(&a.rights)?;
    let vault = a.vault.trim();
    if vault.is_empty() {
        return Err(CliError::usage("VAULT must not be empty").hint("a Vault name, or * for every Vault"));
    }
    let api = ctx.api()?;
    let handle = clean_handle(&a.handle);
    let names: Vec<&str> = rights.iter().map(|r| r.as_str()).collect();
    let m = api.post(&format!("/api/v1/members/{handle}/grants"), &json!({ "vault": vault, "rights": names }))?;
    print_member(ctx, &m)
}

fn role(ctx: &Ctx, a: RoleArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let handle = clean_handle(&a.handle);
    let m = api.post(&format!("/api/v1/members/{handle}/role"), &json!({ "role": a.role }))?;
    if m["changed"] == json!(false) {
        ctx.out.note(&format!("@{handle} is already {}", a.role));
    }
    print_member(ctx, &m)
}

// ---- end ticket 06 ----

/// Human form for the `member add` note: `* (read,comment,action,write)`.
fn grants_pretty(grants: &Value) -> String {
    let parts: Vec<String> = grants
        .as_array()
        .map(|a| {
            a.iter()
                .map(|g| {
                    let vault = g.get("vault").and_then(Value::as_str).unwrap_or("?");
                    let rights: Vec<&str> = g
                        .get("rights")
                        .and_then(Value::as_array)
                        .map(|r| r.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    format!("{vault} ({})", rights.join(","))
                })
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() { "none".to_string() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_render() {
        let g = json!([{"vault":"*","rights":["read","comment","action","write"]},{"vault":"notes","rights":["read"]}]);
        assert_eq!(grants_text(&g), "*:read,comment,action,write notes:read");
        assert_eq!(grants_pretty(&g), "* (read,comment,action,write), notes (read)");
        assert_eq!(grants_text(&json!([])), "-");
    }

    // spec: S06-06
    #[test]
    fn rights_parse() {
        assert_eq!(parse_rights("read,comment").unwrap(), vec![Right::Read, Right::Comment]);
        assert_eq!(parse_rights(" write , write ").unwrap(), vec![Right::Write]);
        assert!(parse_rights("none").unwrap().is_empty());
        assert!(parse_rights("admin").is_err());
        assert!(parse_rights("").is_err());
        assert_eq!(clean_handle("@linter"), "linter");
    }
}
