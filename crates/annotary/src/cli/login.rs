//! `login` and `logout` (ticket 06).
//!
//! `login` (Phase 1): prompts for the server (default: `--server`, `ANNOTARY_SERVER`, the
//! named profile's server, else `http://127.0.0.1:7377`) and the local password, exchanges the
//! password for a Token (`POST /api/v1/login`), saves it in a profile (`-p NAME`, default: the
//! handle). There is no default profile: later calls name it with `-p NAME`. It refuses (exit 5) with a hint
//! when the server has no local password — the CLI never has an unauthenticated owner path.
//! Prompts go to stderr (so stdout is the one result line); on a non-TTY each prompt reads a
//! stdin line, and `ANNOTARY_PASSWORD` answers the password prompt.
//!
//! `logout`: revokes the named profile's (`-p NAME`) Token on the server (`POST /api/v1/logout`, best
//! effort) and removes it from the profile (server/vault stay for the next `login`).

use clap::Args;
use serde_json::{Map, json};

use crate::client::{Api, CredSource, Credential, EnvVars};
use crate::config::{Config, Profile};
use crate::exit::{CliError, CliResult, Exit};
use crate::output::{FieldSpec, JsonMode};
use crate::paths;

use super::Ctx;
use super::prompt::{Prompter, Sink};

#[derive(Args, Debug)]
#[command(
    after_help = "Prompts for the server and the local password (stderr; on a non-TTY each prompt reads a stdin line, \
ANNOTARY_PASSWORD answers the password), saves the token in profile NAME (-p NAME; default: your handle); later calls pass -p NAME. \
Refuses when the server has no local password — use a token then."
)]
pub struct LoginArgs {
    /// Server URL (skips the prompt) [default: ANNOTARY_SERVER, the profile's server, http://127.0.0.1:7377]
    #[arg(long, value_name = "URL")]
    pub server: Option<String>,
}

#[derive(Args, Debug)]
#[command(after_help = "Acts on -p NAME: revokes its token on the server (best effort) and removes \
the token from the profile; server and vault stay for the next login.")]
pub struct LogoutArgs {}

pub const LOGIN_FIELDS: FieldSpec = FieldSpec {
    all: &["profile", "server", "handle", "role", "config"],
    default: &["profile", "server", "handle", "role"],
};

pub const LOGOUT_FIELDS: FieldSpec =
    FieldSpec { all: &["profile", "handle", "revoked", "config"], default: &["profile", "handle", "revoked"] };

/// The refusal when the server has no local password.
fn no_password(server: &str) -> CliError {
    CliError::refused(format!("no local password is set on {server}")).hint(format!(
        "the owner sets one at {server}/settings (in a browser on the server's machine, where it trusts you \
             as the owner); until then use a token: ANNOTARY_TOKEN=… (printed by first-run setup) or \
             `annotary member add HANDLE` for an agent"
    ))
}

pub fn login(ctx: &Ctx, a: LoginArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LOGIN_FIELDS, Vec::new(), true);
    }
    let config_path = ctx.data.config_toml();
    let mut config = Config::load(&config_path)?;
    let env = EnvVars::from_process();
    let named = ctx.profile_name();
    let existing = named.as_ref().and_then(|n| config.profiles.get(n).cloned());
    let p = Prompter::new(Sink::Stderr, "");

    // 1. Server.
    let default_server = a
        .server
        .clone()
        .or_else(|| env.server.clone())
        .or_else(|| existing.as_ref().and_then(|e| e.server.clone()))
        .unwrap_or_else(|| crate::DEFAULT_SERVER.to_string());
    let server = match &a.server {
        Some(s) => s.trim().to_string(),
        None => p.ask("server", &default_server, None),
    };
    let server = server.trim_end_matches('/').to_string();
    if !(server.starts_with("http://") || server.starts_with("https://")) {
        return Err(CliError::usage(format!("server {server:?} must be an http(s) URL")));
    }
    let anon = Api::anonymous(&server);
    let health = anon.health()?;
    if health["password_set"] != json!(true) {
        return Err(no_password(&server));
    }

    // 2. Password → Token.
    let Some(password) = p.ask_password("password", Some("ANNOTARY_PASSWORD")) else {
        return Err(CliError::usage("password must not be empty"));
    };
    let me = anon.post("/api/v1/login", &json!({ "password": password }))?;
    let handle = me["handle"].as_str().unwrap_or("").to_string();
    let role = me["role"].as_str().unwrap_or("").to_string();
    let token = me["token"].as_str().unwrap_or("").to_string();
    if handle.is_empty() || token.is_empty() {
        return Err(CliError::internal("login response without handle/token"));
    }

    // 3. Save the profile (server + token; an existing profile keeps its vault).
    let name = named.unwrap_or_else(|| handle.clone());
    let mut profile = config.profiles.get(&name).cloned().unwrap_or_default();
    profile.server = Some(server.clone());
    profile.token = Some(token);
    config.profiles.insert(name.clone(), profile);
    config.save(&config_path)?;
    let display = paths::display(&config_path);
    if !ctx.out.is_json() {
        ctx.out.line(&format!("Signed in as @{handle} ({role}). Saved profile \"{name}\" → {display}"));
        ctx.out.note(&format!("use it with -p {name}"));
    }

    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("profile".into(), json!(name));
        rec.insert("server".into(), json!(server));
        rec.insert("handle".into(), json!(handle));
        rec.insert("role".into(), json!(role));
        rec.insert("config".into(), json!(display));
        return ctx.out.json_records(&LOGIN_FIELDS, vec![rec], true);
    }
    Ok(())
}

pub fn logout(ctx: &Ctx, _a: LogoutArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LOGOUT_FIELDS, Vec::new(), true);
    }
    let config_path = ctx.data.config_toml();
    let mut config = Config::load(&config_path)?;
    let display = paths::display(&config_path);
    let Some(name) = ctx.profile_name() else {
        return Err(CliError::usage("logout needs a profile — pass -p NAME")
            .hint(format!("profiles in {display}: {}", config.profile_names_display())));
    };
    let Some(profile) = config.profiles.get(&name).cloned() else {
        return Err(CliError::new(Exit::NotFound, "not_found", format!("profile '{name}' not found in {display}"))
            .hint(format!("profiles: {}", config.profile_names_display())));
    };
    let Some(token) = profile.token.clone().filter(|t| !t.trim().is_empty()) else {
        ctx.out.note(&format!("profile \"{name}\" has no token — nothing to forget"));
        return finish_logout(ctx, &name, "", false, &display);
    };
    // Revoke on the server (best effort: unreachable / already invalid still forgets locally).
    let server =
        EnvVars::from_process().server.or(profile.server.clone()).unwrap_or_else(|| crate::DEFAULT_SERVER.to_string());
    let cred = Credential {
        server,
        token,
        source: CredSource::Profile(name.clone()),
        vault: profile.vault.clone(),
        vault_id: None,
        vault_profile: None,
    };
    let api = Api::new(&cred);
    let (handle, revoked) = match api.post("/api/v1/logout", &json!({})) {
        Ok(r) => (r["handle"].as_str().unwrap_or("").to_string(), true),
        Err(e) => {
            ctx.out.note(&format!("(server did not revoke the token: {})", e.message));
            (String::new(), false)
        }
    };
    let entry = config.profiles.entry(name.clone()).or_insert_with(Profile::default);
    entry.token = None;
    config.save(&config_path)?;
    finish_logout(ctx, &name, &handle, revoked, &display)
}

fn finish_logout(ctx: &Ctx, name: &str, handle: &str, revoked: bool, display: &str) -> CliResult<()> {
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("profile".into(), json!(name));
        rec.insert("handle".into(), json!(handle));
        rec.insert("revoked".into(), json!(revoked));
        rec.insert("config".into(), json!(display));
        return ctx.out.json_records(&LOGOUT_FIELDS, vec![rec], true);
    }
    let who = if handle.is_empty() { String::new() } else { format!(" (@{handle})") };
    let what = if revoked { "token revoked and removed" } else { "token removed" };
    ctx.out.line(&format!("Signed out{who}: profile \"{name}\" — {what}"));
    Ok(())
}
