//! The CLI's side of the wire: credential resolution and a small HTTP client over `ureq`.
//!
//! Credential order: the `-p/--profile` flag → `ANNOTARY_TOKEN` (env) → `ANNOTARY_PROFILE`
//! → exit 3; there is no default profile. Server URL: `ANNOTARY_SERVER` → the profile's
//! `server` → `http://127.0.0.1:7377`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::config::{Config, Profile};
use crate::exit::{CliError, CliResult, Exit};

/// Connect timeout: a stopped server must fail fast (exit 6), never hang.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Whole-request timeout for ordinary calls (`wait` sets its own, longer, per request).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest response body the CLI reads (a Version's Markdown or Sidecar is far below this).
pub const BODY_LIMIT: u64 = 512 * 1024 * 1024;

/// Percent-encode a query-string value (RFC 3986 unreserved characters kept).
pub fn qs(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Undo [`qs`]: `%XX` escapes back to bytes, read as UTF-8 (`None` for a broken escape or
/// invalid UTF-8). CMD-5: the Vault names in the `X-Annotary-*-Vault` headers travel as
/// [`qs`] text, since an HTTP header value is ASCII.
pub fn unqs(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The environment the CLI consults (read once so tests can pass their own).
#[derive(Clone, Debug, Default)]
pub struct EnvVars {
    pub token: Option<String>,
    pub server: Option<String>,
    pub profile: Option<String>,
}

impl EnvVars {
    pub fn from_process() -> EnvVars {
        let get = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        EnvVars { token: get("ANNOTARY_TOKEN"), server: get("ANNOTARY_SERVER"), profile: get("ANNOTARY_PROFILE") }
    }
}

/// Where the credential came from (for messages).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredSource {
    Env,
    Profile(String),
}

/// A resolved credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credential {
    pub server: String,
    pub token: String,
    pub source: CredSource,
    /// The selected profile's `vault`, when a profile was used.
    pub vault: Option<String>,
    /// CMD-5: the id recorded beside that `vault` (`vault_id` in `config.toml`), when known.
    pub vault_id: Option<String>,
    /// CMD-5: the profile `vault` came from (the `-p` / `ANNOTARY_PROFILE` one) — the one a
    /// renamed or deleted Vault is healed in.
    pub vault_profile: Option<String>,
}

/// Resolve the credential and server URL. Who the caller is is always said, never assumed
/// (CMD bug: an agent fell back to the owner's default profile and commented as them):
/// `-p/--profile` (the flag) > `ANNOTARY_TOKEN` > `ANNOTARY_PROFILE`; there is no default
/// profile. The flag wins over the environment so an agent that names its own profile is never
/// overridden by a token or profile exported in the human's shell.
pub fn resolve_credential(config_path: &Path, profile_flag: Option<&str>, env: &EnvVars) -> CliResult<Credential> {
    let config = Config::load(config_path)?;
    let display = crate::paths::display(config_path);
    let flag = profile_flag.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let named = |name: &str| -> CliResult<Profile> {
        config.profiles.get(name).cloned().ok_or_else(|| {
            CliError::no_credential(format!("profile '{name}' not found in {display}"))
                .hint(format!("profiles: {}", config.profile_names_display()))
        })
    };
    let from_profile = |name: String, p: Profile| -> CliResult<Credential> {
        let server = env.server.clone().or(p.server.clone());
        let Some(token) = p.token.filter(|t| !t.trim().is_empty()) else {
            return Err(CliError::no_credential(format!("profile '{name}' in {display} has no token"))
                .hint("add `token = \"ant_…\"` to it, or run `annotary login`"));
        };
        let vault_profile = p.vault.as_ref().map(|_| name.clone());
        Ok(Credential {
            server: server_url(server),
            token,
            source: CredSource::Profile(name),
            vault: p.vault,
            vault_id: p.vault_id,
            vault_profile,
        })
    };

    if let Some(name) = flag {
        let p = named(&name)?;
        return from_profile(name, p);
    }
    if let Some(tok) = &env.token {
        // A profile named in the environment only scopes the Vault; the env token is who you are.
        let p = match &env.profile {
            Some(name) => Some(named(name)?),
            None => None,
        };
        let server = env.server.clone().or_else(|| p.as_ref().and_then(|p| p.server.clone()));
        let (vault, vault_id) = p.map(|p| (p.vault, p.vault_id)).unwrap_or_default();
        let vault_profile = vault.as_ref().and_then(|_| env.profile.clone());
        return Ok(Credential {
            server: server_url(server),
            token: tok.clone(),
            source: CredSource::Env,
            vault,
            vault_id,
            vault_profile,
        });
    }
    if let Some(name) = env.profile.clone() {
        let p = named(&name)?;
        return from_profile(name, p);
    }

    let mut detail = format!("profiles in {display}: {}", config.profile_names_display());
    if let Some(d) = &config.legacy_default {
        detail.push_str(&format!(" (its `default = {d:?}` is no longer used — pass -p {d})"));
    }
    Err(CliError::no_credential("no credential")
        .hint("say who you are: pass -p NAME (`annotary profile list` shows whose each one is), or set ANNOTARY_TOKEN")
        .detail(detail))
}

/// The server URL a profile points at (`ANNOTARY_SERVER` > its `server` > the default), as
/// [`Credential::server`] has it.
pub fn server_url(server: Option<String>) -> String {
    server.unwrap_or_else(|| crate::DEFAULT_SERVER.to_string()).trim_end_matches('/').to_string()
}

// ---- CMD-5: a profile's Vault across renames (the server side is `server/vaults.rs`) ----

/// Request header: the profile's `vault` (percent-encoded, [`qs`]).
pub const PROFILE_VAULT_HEADER: &str = "X-Annotary-Profile-Vault";
/// Request header: the Vault id recorded beside it (`vault_id`).
pub const PROFILE_VAULT_ID_HEADER: &str = "X-Annotary-Profile-Vault-Id";
/// Response headers: where the profile's Vault is now (name percent-encoded, id) / that it was
/// deleted (its percent-encoded name).
pub const RESOLVED_VAULT_HEADER: &str = "X-Annotary-Resolved-Vault";
pub const RESOLVED_VAULT_ID_HEADER: &str = "X-Annotary-Resolved-Vault-Id";
pub const DELETED_VAULT_HEADER: &str = "X-Annotary-Deleted-Vault";

/// The profile Vault a client sends, and where to heal it.
#[derive(Clone, Debug)]
struct VaultBinding {
    profile: String,
    config: PathBuf,
    name: String,
    id: Option<String>,
    quiet: bool,
}

/// What a response said about the profile's Vault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultNews {
    Resolved { name: String, id: String },
    Deleted { name: String },
}

/// Read [`RESOLVED_VAULT_HEADER`] / [`DELETED_VAULT_HEADER`] off a response.
pub fn vault_news(headers: &ureq::http::HeaderMap) -> Option<VaultNews> {
    let text = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).and_then(unqs).filter(|s| !s.is_empty());
    if let Some(name) = text(DELETED_VAULT_HEADER) {
        return Some(VaultNews::Deleted { name });
    }
    Some(VaultNews::Resolved { name: text(RESOLVED_VAULT_HEADER)?, id: text(RESOLVED_VAULT_ID_HEADER)? })
}

/// Apply `news` to profile `profile` in `config` when its `vault` is still `sent` (a concurrent
/// hand edit wins): a rename sets `vault` + `vault_id`, a deletion clears both. Returns the note
/// to print (`None` for a silent backfill of `vault_id`, or when nothing changed).
pub fn heal_profile(
    config: &Path,
    profile: &str,
    sent: (&str, Option<&str>),
    news: &VaultNews,
) -> CliResult<Option<String>> {
    let mut cfg = Config::load(config)?;
    let Some(p) = cfg.profiles.get_mut(profile) else { return Ok(None) };
    if p.vault.as_deref().map(str::trim) != Some(sent.0) {
        return Ok(None);
    }
    let note = match news {
        VaultNews::Resolved { name, id } => {
            if (name.as_str(), Some(id.as_str())) == (sent.0, sent.1) && p.vault_id.as_deref() == Some(id) {
                return Ok(None);
            }
            p.vault = Some(name.clone());
            p.vault_id = Some(id.clone());
            (name != sent.0)
                .then(|| format!("note: Vault \"{}\" is now \"{name}\" — updated profile \"{profile}\"", sent.0))
        }
        VaultNews::Deleted { name } => {
            p.vault = None;
            p.vault_id = None;
            Some(format!("note: Vault \"{name}\" was deleted — cleared it from profile \"{profile}\""))
        }
    };
    cfg.save(config)?;
    Ok(note)
}

/// HTTP client bound to one server + Token (or no Token at all: `login` and `health`).
pub struct Api {
    agent: ureq::Agent,
    pub server: String,
    /// `Authorization: Bearer …` value, or the empty string for an anonymous client (ticket 06).
    token: String,
    /// CMD-5: the profile's Vault, sent on every request and healed from every response.
    vault: Mutex<Option<VaultBinding>>,
}

impl Api {
    pub fn new(cred: &Credential) -> Api {
        Api::with_token(&cred.server, &cred.token)
    }

    /// CMD-5: send `cred`'s profile Vault (and its id) with every request; when a response says
    /// it was renamed or deleted, rewrite that profile in `config` (a note on stderr unless
    /// `quiet`). Without a profile `vault` this is [`Api::new`].
    pub fn with_profile_vault(self, cred: &Credential, config: &Path, quiet: bool) -> Api {
        let name = cred.vault.as_deref().map(str::trim).filter(|v| !v.is_empty());
        if let (Some(name), Some(profile)) = (name, &cred.vault_profile) {
            *self.vault.lock().unwrap_or_else(|e| e.into_inner()) = Some(VaultBinding {
                profile: profile.clone(),
                config: config.to_path_buf(),
                name: name.to_string(),
                id: cred.vault_id.clone().filter(|i| !i.trim().is_empty()),
                quiet,
            });
        }
        self
    }

    /// The headers every request carries: the Token, and the profile's Vault (CMD-5).
    fn decorate<B>(&self, mut req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        if let Some(h) = self.auth_header() {
            req = req.header("Authorization", h);
        }
        if let Some(b) = self.vault.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            req = req.header(PROFILE_VAULT_HEADER, qs(&b.name));
            if let Some(id) = &b.id {
                req = req.header(PROFILE_VAULT_ID_HEADER, qs(id));
            }
        }
        req
    }

    /// A response's word on the profile's Vault: heal the profile (best effort) and send the
    /// new name / id (or nothing, once deleted) from then on.
    fn observe(&self, headers: &ureq::http::HeaderMap) {
        let mut guard = self.vault.lock().unwrap_or_else(|e| e.into_inner());
        let (Some(b), Some(news)) = (guard.as_ref(), vault_news(headers)) else { return };
        let note = match heal_profile(&b.config, &b.profile, (&b.name, b.id.as_deref()), &news) {
            Ok(n) => n,
            Err(e) => Some(format!("note: could not update profile \"{}\": {}", b.profile, e.message)),
        };
        if let Some(n) = note
            && !b.quiet
        {
            eprintln!("{n}");
        }
        match news {
            VaultNews::Resolved { name, id } => {
                if let Some(b) = guard.as_mut() {
                    b.name = name;
                    b.id = Some(id);
                }
            }
            VaultNews::Deleted { .. } => *guard = None,
        }
    }
    // ---- end CMD-5 ----

    // ---- ticket 06 ----
    /// A client with no credential (`login`, `health`).
    pub fn anonymous(server: &str) -> Api {
        Api::with_token(server, "")
    }

    fn with_token(server: &str, token: &str) -> Api {
        Api::with_tls(server, token, ureq::tls::TlsConfig::default())
    }

    /// CMD-18: the TLS settings are ureq's defaults (rustls, the bundled webpki roots) for every
    /// CLI client; `tests/cli_tls.rs` passes its own root to reach a local https server.
    pub fn with_tls(server: &str, token: &str, tls: ureq::tls::TlsConfig) -> Api {
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(REQUEST_TIMEOUT))
            .user_agent(format!("annotary/{}", crate::VERSION))
            .build();
        Api {
            agent: config.new_agent(),
            server: server.trim_end_matches('/').to_string(),
            token: token.to_string(),
            vault: Mutex::new(None),
        }
    }

    /// The Token this client sends (empty for an anonymous one).
    pub fn token(&self) -> &str {
        &self.token
    }

    /// `GET /api/v1/health` (unauthenticated) → `{ok, version, url, started_at, set_up, password_set}`.
    pub fn health(&self) -> CliResult<Value> {
        self.get("/api/v1/health")
    }

    /// The `Authorization` header value, if this client has a Token.
    fn auth_header(&self) -> Option<String> {
        if self.token.is_empty() { None } else { Some(format!("Bearer {}", self.token)) }
    }
    // ---- end ticket 06 ----

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.server, path)
    }

    /// GET `path` (e.g. `/api/v1/members`) → JSON body.
    pub fn get(&self, path: &str) -> CliResult<Value> {
        let req = self.decorate(self.agent.get(self.url(path)));
        self.finish(req.call())
    }

    // ---- ticket 08 ----
    /// GET `path` with its own whole-request timeout (longer than `REQUEST_TIMEOUT`): `wait`'s
    /// long-poll asks the server to park up to 30 s per request.
    pub fn get_timeout(&self, path: &str, timeout: Duration) -> CliResult<Value> {
        let req = self.decorate(self.agent.get(self.url(path)).config().timeout_global(Some(timeout)).build());
        self.finish(req.call())
    }
    // ---- end ticket 08 ----

    /// POST `path` with a JSON body → JSON body.
    pub fn post(&self, path: &str, body: &Value) -> CliResult<Value> {
        let req = self.decorate(self.agent.post(self.url(path)));
        self.finish(req.send_json(body))
    }

    /// PUT `path` with a JSON body → JSON body (ticket 04: edit a Comment).
    pub fn put_json(&self, path: &str, body: &Value) -> CliResult<Value> {
        let req = self.decorate(self.agent.put(self.url(path)));
        self.finish(req.send_json(body))
    }

    /// PATCH `path` with a JSON body → JSON body (CMD-23: rename a Device).
    pub fn patch_json(&self, path: &str, body: &Value) -> CliResult<Value> {
        let req = self.decorate(self.agent.patch(self.url(path)));
        self.finish(req.send_json(body))
    }

    /// DELETE `path` → JSON body.
    pub fn delete(&self, path: &str) -> CliResult<Value> {
        let req = self.decorate(self.agent.delete(self.url(path)));
        self.finish(req.call())
    }

    /// PUT raw bytes (`application/octet-stream`) → JSON body. Used for blob uploads.
    pub fn put_bytes(&self, path: &str, bytes: &[u8]) -> CliResult<Value> {
        let req = self.decorate(self.agent.put(self.url(path)).content_type("application/octet-stream"));
        self.finish(req.send(bytes))
    }

    /// GET `path` → the raw response body (blobs, Sidecar text). Errors are JSON as usual.
    pub fn get_bytes(&self, path: &str) -> CliResult<Vec<u8>> {
        let req = self.decorate(self.agent.get(self.url(path)));
        let res = req.call();
        let mut resp = res.map_err(|e| self.map_transport(e))?;
        self.observe(resp.headers());
        let status = resp.status().as_u16();
        let bytes = resp.body_mut().with_config().limit(BODY_LIMIT).read_to_vec().map_err(|e| self.map_transport(e))?;
        if (200..300).contains(&status) {
            return Ok(bytes);
        }
        let text = String::from_utf8_lossy(&bytes).to_string();
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Err(map_status(status, &value))
    }

    /// The web page of a Document on this server.
    pub fn doc_url(&self, id: &str) -> String {
        format!("{}/d/{id}", self.server)
    }

    // ---- ticket 18 ----
    /// The link to a Document (and optionally a Thread) for a person on another device: the
    /// server's `doc_link` (public URL, through the link relay when set — `health` says), else
    /// this client's own `…/d/{id}[#tN]`. `public` = it was built from the public URL.
    pub fn doc_link(&self, id: &str, thread: Option<&str>) -> CliResult<(String, bool)> {
        let h = self.health()?;
        let link = annotary_store::doc_link(h["public_url"].as_str(), h["link_base"].as_str(), id, thread);
        Ok(match link {
            Some(l) => (l, true),
            None => (format!("{}{}", self.doc_url(id), thread.map(|t| format!("#{t}")).unwrap_or_default()), false),
        })
    }

    /// [`Api::doc_link`] for display on a TTY (`show`, `push`): the public link when the server
    /// has one, else `doc_url` — never an error (an old server without the fields, a hiccup).
    pub fn display_link(&self, id: &str) -> String {
        self.doc_link(id, None).map(|(l, _)| l).unwrap_or_else(|_| self.doc_url(id))
    }
    // ---- end ticket 18 ----

    fn finish(&self, res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> CliResult<Value> {
        let mut resp = res.map_err(|e| self.map_transport(e))?;
        self.observe(resp.headers());
        let status = resp.status().as_u16();
        let text =
            resp.body_mut().with_config().limit(BODY_LIMIT).read_to_string().map_err(|e| self.map_transport(e))?;
        let value: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()))
        };
        if (200..300).contains(&status) {
            return Ok(value);
        }
        Err(map_status(status, &value))
    }

    fn map_transport(&self, e: ureq::Error) -> CliError {
        // CMD-18: a TLS failure (an untrusted or expired certificate, a name mismatch) arrives as
        // an io error wrapping rustls' own; say so instead of "not reachable".
        let tls = match &e {
            ureq::Error::Rustls(r) => Some(r.to_string()),
            ureq::Error::Io(io) => io.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()).map(|r| r.to_string()),
            _ => None,
        };
        if let Some(why) = tls {
            return CliError::new(Exit::Unreachable, "tls", format!("TLS with {} failed: {why}", self.server))
                .hint("the server's certificate must be valid for its name and signed by a public CA");
        }
        match e {
            ureq::Error::Io(_)
            | ureq::Error::Timeout(_)
            | ureq::Error::HostNotFound
            | ureq::Error::ConnectionFailed
            | ureq::Error::Protocol(_) => CliError::unreachable(&self.server),
            other => CliError::usage(format!("invalid server URL {}: {other}", self.server)),
        }
    }
}

/// Map an HTTP error status + `{error,message,hint}` body onto a `CliError`.
pub fn map_status(status: u16, body: &Value) -> CliError {
    let kind = body.get("error").and_then(Value::as_str).map(str::to_string);
    let message = body
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| body.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("server returned HTTP {status}"));
    let hint = body.get("hint").and_then(Value::as_str).map(str::to_string);
    let (exit, default_kind) = match status {
        401 => (Exit::NoCredential, "unauthorized"),
        403 => (Exit::MissingRight, "forbidden"),
        404 => (Exit::NotFound, "not_found"),
        409 => (Exit::Refused, "conflict"),
        413 => (Exit::Refused, "too_large"),
        // CMD-4: sign-ins paused after repeated wrong passwords.
        429 => (Exit::Refused, "too_many_attempts"),
        400 | 422 => (Exit::Usage, "invalid"),
        _ => (Exit::Usage, "server_error"),
    };
    let mut err = CliError::new(exit, kind.unwrap_or_else(|| default_kind.to_string()), message);
    if let Some(h) = hint {
        err = err.hint(h);
    }
    // An ambiguous Name lists its candidates as `id name vN updated` detail lines.
    if let Some(cands) = body.get("candidates").and_then(Value::as_array) {
        let now = jiff::Timestamp::now();
        for c in cands {
            let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let v = c.get("version").and_then(Value::as_u64).unwrap_or(0);
            err = err.detail(format!(
                "  {}  {}  v{}  {}",
                s("id"),
                s("name"),
                v,
                crate::output::relative_age(&s("updated_at"), now)
            ));
        }
    }
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &tempfile::TempDir, text: &str) -> std::path::PathBuf {
        let p = dir.path().join("config.toml");
        std::fs::write(&p, text).unwrap();
        p
    }

    const TWO: &str = "[profiles.rowan]\nserver = \"http://127.0.0.1:7377\"\ntoken = \"ant_j\"\n[profiles.linter]\ntoken = \"ant_c\"\nvault = \"notes\"\n";

    // spec: S01-14
    #[test]
    fn env_token_wins_over_env_profile_and_env_server_overrides_it() {
        let d = tempfile::tempdir().unwrap();
        let p = cfg(&d, TWO);
        let env = EnvVars {
            token: Some("ant_env".into()),
            server: Some("http://127.0.0.1:1".into()),
            profile: Some("linter".into()),
        };
        let c = resolve_credential(&p, None, &env).unwrap();
        assert_eq!(c.token, "ant_env");
        assert_eq!(c.server, "http://127.0.0.1:1");
        assert_eq!(c.source, CredSource::Env);
        assert_eq!(c.vault.as_deref(), Some("notes"), "the env profile still scopes the Vault");
    }

    // spec: S01-14
    #[test]
    fn the_profile_flag_beats_a_token_or_profile_in_the_environment() {
        // An agent passes `-p agent`; the human's shell exported their own token and profile.
        let d = tempfile::tempdir().unwrap();
        let p = cfg(&d, TWO);
        let env = EnvVars { token: Some("ant_env".into()), server: None, profile: Some("rowan".into()) };
        let c = resolve_credential(&p, Some("linter"), &env).unwrap();
        assert_eq!((c.token.as_str(), c.source), ("ant_c", CredSource::Profile("linter".into())));
        // ANNOTARY_SERVER still points any profile elsewhere (where, not who).
        let env = EnvVars { server: Some("http://127.0.0.1:1".into()), ..Default::default() };
        assert_eq!(resolve_credential(&p, Some("rowan"), &env).unwrap().server, "http://127.0.0.1:1");
    }

    #[test]
    fn profile_flag_and_env_profile_select() {
        let d = tempfile::tempdir().unwrap();
        let p = cfg(&d, TWO);
        let c = resolve_credential(&p, Some("linter"), &EnvVars::default()).unwrap();
        assert_eq!(c.token, "ant_c");
        assert_eq!(c.server, crate::DEFAULT_SERVER, "profile without server falls back to the default URL");
        assert_eq!(c.vault.as_deref(), Some("notes"));
        let env = EnvVars { profile: Some("rowan".into()), ..Default::default() };
        let c = resolve_credential(&p, None, &env).unwrap();
        assert_eq!(c.source, CredSource::Profile("rowan".into()));
    }

    // spec: S01-14, S01-15
    #[test]
    fn there_is_no_default_profile_even_when_an_old_config_names_one() {
        let d = tempfile::tempdir().unwrap();
        let p = cfg(&d, TWO);
        let err = resolve_credential(&p, None, &EnvVars::default()).unwrap_err();
        assert_eq!(err.exit, Exit::NoCredential);
        assert_eq!(
            err.text_line(),
            "error: no credential — say who you are: pass -p NAME (`annotary profile list` shows whose each one is), or set ANNOTARY_TOKEN"
        );
        assert_eq!(err.detail, vec![format!("profiles in {}: linter, rowan", crate::paths::display(&p))]);
        // The bug: `default = "rowan"` made every caller — agents too — act as @rowan.
        let p2 = cfg(&d, &format!("default = \"rowan\"\n{TWO}"));
        let err = resolve_credential(&p2, None, &EnvVars::default()).unwrap_err();
        assert_eq!(err.exit, Exit::NoCredential);
        assert_eq!(
            err.detail,
            vec![format!(
                "profiles in {}: linter, rowan (its `default = \"rowan\"` is no longer used — pass -p rowan)",
                crate::paths::display(&p2)
            )]
        );
    }

    #[test]
    fn missing_profile_and_no_config() {
        let d = tempfile::tempdir().unwrap();
        let p = cfg(&d, TWO);
        let err = resolve_credential(&p, Some("nope"), &EnvVars::default()).unwrap_err();
        assert_eq!(err.exit, Exit::NoCredential);
        assert!(err.message.contains("profile 'nope' not found"));
        assert_eq!(err.hint.as_deref(), Some("profiles: linter, rowan"));
        let none = d.path().join("missing.toml");
        let err = resolve_credential(&none, None, &EnvVars::default()).unwrap_err();
        assert_eq!(err.exit, Exit::NoCredential);
        assert!(err.detail[0].ends_with(": none"));
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(qs("plan"), "plan");
        assert_eq!(qs("Übersicht 2026/a?b=c&d"), "%C3%9Cbersicht%202026%2Fa%3Fb%3Dc%26d");
    }

    // spec: S26-08
    #[test]
    fn header_vault_names_round_trip_through_percent_encoding() {
        for name in ["notes", "Übersicht 2026", "a%b", "emoji 🚀", "x+y"] {
            assert_eq!(unqs(&qs(name)).as_deref(), Some(name));
            assert!(qs(name).is_ascii());
        }
        assert_eq!(unqs("plain"), Some("plain".into()));
        assert_eq!(unqs("%zz"), None);
        assert_eq!(unqs("%C3"), None, "a lone UTF-8 lead byte");
        assert_eq!(unqs("50%"), None);
    }

    #[test]
    fn status_mapping() {
        let e = map_status(401, &serde_json::json!({"error":"unauthorized","message":"bad token","hint":"h"}));
        assert_eq!(
            (e.exit, e.kind.as_str(), e.message.as_str(), e.hint.as_deref()),
            (Exit::NoCredential, "unauthorized", "bad token", Some("h"))
        );
        assert_eq!(map_status(403, &Value::Null).exit, Exit::MissingRight);
        assert_eq!(map_status(404, &Value::Null).exit, Exit::NotFound);
        assert_eq!(map_status(409, &Value::Null).exit, Exit::Refused);
        assert_eq!(map_status(413, &Value::Null).exit, Exit::Refused);
        assert_eq!(map_status(413, &Value::Null).kind, "too_large");
        assert_eq!(map_status(400, &Value::String("nope".into())).message, "nope");
        assert_eq!(map_status(500, &Value::Null).exit, Exit::Usage);
    }
}
