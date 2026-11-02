//! `profile list | add NAME [--server URL] [--token TOKEN] [--vault NAME] | remove NAME`
//! (ticket 06) — the CLI's side of `~/.annotary/config.toml`.
//!
//! `list` prints `NAME SERVER MEMBER` (`MEMBER` = `@handle` from each server's `/api/v1/me`,
//! `-` when the server is unreachable or rejects the token) — how an agent finds its own profile.
//! Tokens are never printed. There is no default profile: every call says who it is (`-p NAME`).

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::client::{Api, CredSource, Credential};
use crate::config::{Config, Profile};
use crate::exit::{CliError, CliResult, Exit};
use crate::output::{FieldSpec, JsonMode};
use crate::paths;

use super::Ctx;

#[derive(Subcommand, Debug)]
pub enum ProfileCmd {
    /// Profiles in config.toml: name, server, member (whose each one is)
    List,
    /// Add a profile (refuses to overwrite one; `logout`/`remove` first, or edit config.toml)
    Add(AddArgs),
    /// Remove a profile
    Remove(NameArg),
}

#[derive(Args, Debug)]
pub struct AddArgs {
    /// Profile name (what -p/--profile selects)
    pub name: String,
    /// Server URL [default: http://127.0.0.1:7377]
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,
    /// Token (ant_…); without one the profile is not usable until `login` fills it
    #[arg(long, value_name = "TOKEN")]
    pub token: Option<String>,
    /// Vault this profile works in by default
    #[arg(long, value_name = "NAME")]
    pub vault: Option<String>,
}

#[derive(Args, Debug)]
pub struct NameArg {
    /// Profile name
    pub name: String,
}

pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &["name", "server", "member", "role", "vault", "has_token"],
    default: &["name", "server", "member", "vault"],
};

pub const ONE_FIELDS: FieldSpec =
    FieldSpec { all: &["name", "server", "vault", "has_token", "config"], default: &["name", "server", "vault"] };

pub fn run(ctx: &Ctx, cmd: ProfileCmd) -> CliResult<()> {
    match cmd {
        ProfileCmd::List => list(ctx),
        ProfileCmd::Add(a) => add(ctx, a),
        ProfileCmd::Remove(a) => remove(ctx, a),
    }
}

/// Ask a profile's server who its token is (`/api/v1/me`): `(handle, role)` or `None`. CMD-5: the
/// same request carries the profile's Vault, so a Vault renamed or deleted elsewhere is healed in
/// `config.toml` (`client::heal_profile`).
fn whoami(ctx: &Ctx, name: &str, p: &Profile) -> Option<(String, String)> {
    let token = p.token.clone().filter(|t| !t.trim().is_empty())?;
    let server = p.server.clone().unwrap_or_else(|| crate::DEFAULT_SERVER.to_string());
    let cred = Credential {
        server,
        token,
        source: CredSource::Profile(name.to_string()),
        vault: p.vault.clone(),
        vault_id: p.vault_id.clone(),
        vault_profile: Some(name.to_string()),
    };
    let api = Api::new(&cred).with_profile_vault(&cred, &ctx.data.config_toml(), ctx.out.quiet);
    let me = api.get("/api/v1/me").ok()?;
    Some((me["handle"].as_str()?.to_string(), me["role"].as_str().unwrap_or("").to_string()))
}

fn record(name: &str, p: &Profile, who: Option<(String, String)>) -> Map<String, Value> {
    let mut r = Map::new();
    r.insert("name".into(), json!(name));
    r.insert("server".into(), json!(p.server.clone().unwrap_or_else(|| crate::DEFAULT_SERVER.to_string())));
    r.insert("member".into(), json!(who.as_ref().map(|w| w.0.clone()).unwrap_or_default()));
    r.insert("role".into(), json!(who.as_ref().map(|w| w.1.clone()).unwrap_or_default()));
    r.insert("vault".into(), json!(p.vault.clone().unwrap_or_default()));
    r.insert("has_token".into(), json!(p.token.as_deref().is_some_and(|t| !t.trim().is_empty())));
    r
}

fn list(ctx: &Ctx) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let path = ctx.data.config_toml();
    let config = Config::load(&path)?;
    let who: Vec<Option<(String, String)>> = config.profiles.iter().map(|(name, p)| whoami(ctx, name, p)).collect();
    // Re-read: `whoami` may have healed a profile's `vault` (CMD-5).
    let config = Config::load(&path)?;
    let records: Vec<Map<String, Value>> =
        config.profiles.iter().zip(who).map(|((name, p), w)| record(name, p, w)).collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, records, false);
    }
    if ctx.out.tty && records.is_empty() {
        ctx.out.note(&format!(
            "no profiles in {} — `annotary login` or `annotary profile add`",
            paths::display(&ctx.data.config_toml())
        ));
        return Ok(());
    }
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let member = if s("member").is_empty() { "-".to_string() } else { format!("@{}", s("member")) };
            vec![s("name"), s("server"), member]
        })
        .collect();
    let table = ctx.out.table(&["NAME", "SERVER", "MEMBER"], &rows, &[]);
    print!("{table}");
    Ok(())
}

fn print_one(ctx: &Ctx, name: &str, p: &Profile, verb: &str, display: &str) -> CliResult<()> {
    if ctx.out.is_json() {
        let mut rec = record(name, p, None);
        rec.remove("member");
        rec.remove("role");
        rec.insert("config".into(), json!(display));
        return ctx.out.json_records(&ONE_FIELDS, vec![rec], true);
    }
    ctx.out.line(&format!("profile \"{name}\" {verb} → {display}"));
    Ok(())
}

fn add(ctx: &Ctx, a: AddArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&ONE_FIELDS, Vec::new(), true);
    }
    let name = a.name.trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return Err(CliError::usage(format!("profile name {name:?} must be a single word")));
    }
    let path = ctx.data.config_toml();
    let display = paths::display(&path);
    let mut config = Config::load(&path)?;
    if config.profiles.contains_key(&name) {
        return Err(CliError::refused(format!("profile '{name}' already exists in {display}"))
            .hint("`annotary profile remove NAME` first, or `annotary login -p NAME` to replace its token"));
    }
    if let Some(s) = &a.server
        && !(s.starts_with("http://") || s.starts_with("https://"))
    {
        return Err(CliError::usage(format!("--server {s:?} must be an http(s) URL")));
    }
    let profile = Profile {
        server: a.server.as_deref().map(|s| s.trim().trim_end_matches('/').to_string()),
        token: a.token.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
        vault: a.vault.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string),
        vault_id: None, // CMD-5: the first call to the server fills it
    };
    config.profiles.insert(name.clone(), profile.clone());
    config.save(&path)?;
    if profile.token.is_none() {
        ctx.out.note(&format!("profile \"{name}\" has no token yet — `annotary login -p {name}` fills it"));
    }
    print_one(ctx, &name, &profile, "saved", &display)
}

fn remove(ctx: &Ctx, a: NameArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&ONE_FIELDS, Vec::new(), true);
    }
    let name = a.name.trim().to_string();
    let path = ctx.data.config_toml();
    let display = paths::display(&path);
    let mut config = Config::load(&path)?;
    let Some(profile) = config.profiles.remove(&name) else {
        return Err(CliError::new(Exit::NotFound, "not_found", format!("profile '{name}' not found in {display}"))
            .hint(format!("profiles: {}", config.profile_names_display())));
    };
    config.save(&path)?;
    print_one(ctx, &name, &profile, "removed", &display)
}
