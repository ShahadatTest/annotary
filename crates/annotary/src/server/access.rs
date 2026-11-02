//! Access model completion (ticket 06): Member management, Tokens, `login`/`logout`, the local
//! password, Vault create.
//!
//! | route | who | what |
//! |---|---|---|
//! | `GET /api/v1/members/{handle}` | any Member | one Member; owner or self also gets `tokens[]` |
//! | `POST /api/v1/members/{handle}/rename {name}` | owner | display name (the handle never changes) |
//! | `POST /api/v1/members/{handle}/disable` | owner | disable, never delete (`changed`); 409 `last_owner` |
//! | `POST /api/v1/members/{handle}/role {role}` | owner | `owner|member`; 409 `last_owner` when demoting the last one |
//! | `POST /api/v1/members/{handle}/grants {vault, rights[]}` | owner | set the Grant on one Vault (`*` allowed; `[]` removes it) |
//! | `PUT /api/v1/members/{handle}/grants {grants[]}` | owner | replace the whole Grant list |
//! | `GET /api/v1/tokens?member=HANDLE` | own; owner: any / all | Tokens (never the plain text) with `last_used_at`, `status` |
//! | `POST /api/v1/tokens {member?, name?, expires_at?}` | own; owner: any | 201, `token` plain shown once |
//! | `DELETE /api/v1/tokens/{id}` | own or owner | revoke (`revoked: false` when it already was) |
//! | `POST /api/v1/login {password}` | nobody (unauthenticated) | exchange the local password for a Token; 401 `wrong_password`, 409 `no_password`, 429 `too_many_attempts` after repeated failures (CMD-4) |
//! | `POST /api/v1/logout` | any | revoke the bearer Token / end the cookie session |
//! | `GET /api/v1/settings` · `PUT /api/v1/settings/password {password|null}` | owner | `{password_set, my_password, auto_purge_days (ticket 13), public_url, link_base (ticket 18)}`; set/change/remove the caller's local password |
//! | `POST /api/v1/vaults {name}` | owner | create a Vault (`*` Grants cover it at once) |
//!
//! Errors: 409 `last_owner`, 409 `disabled` (`@h is disabled`), 401 `wrong_password`, 409
//! `no_password`, 400 `invalid_grant`, 404 `no Vault X` / `no Member @h` / `no Token ID`.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::{AuthVia, Grant, MemberView, Right, Role, StoreError, TokenRow, time};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::members::member_json;
use super::vaults::vault_json;

/// A Token as the API shows it (never the plain text, never the hash).
pub fn token_json(t: &TokenRow) -> Value {
    json!({
        "id": t.info.id,
        "member": t.handle,
        "member_id": t.info.member_id,
        "name": t.info.name,
        "created_at": t.info.created_at,
        "expires_at": t.info.expires_at,
        "last_used_at": t.info.last_used_at,
        "revoked_at": t.info.revoked_at,
        "status": t.status(&time::now()),
        "member_disabled": t.member_disabled,
    })
}

pub(crate) fn load_member(st: &AppState, handle: &str) -> Result<MemberView, ApiError> {
    let handle = handle.trim().trim_start_matches('@');
    st.store
        .member_by_handle(handle)?
        .ok_or_else(|| ApiError::not_found(format!("no Member @{handle}")).hint("`annotary member list` shows who"))
}

fn parse_rights(v: &Value) -> Result<Vec<Right>, ApiError> {
    let arr = v.as_array().ok_or_else(|| ApiError::invalid("rights must be an array"))?;
    let mut out = Vec::new();
    for r in arr {
        let s = r.as_str().ok_or_else(|| ApiError::invalid("rights must be strings"))?;
        let right = Right::parse(s.trim()).ok_or_else(|| {
            ApiError::invalid(format!("unknown right {s:?}")).hint("rights: read, comment, action, write")
        })?;
        out.push(right);
    }
    Ok(out)
}

// ---------------------------------------------------------------- members

pub async fn get_member(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let m = load_member(&st, &handle)?;
    let mut v = member_json(&m);
    if auth.0.is_owner() || auth.0.member_id == m.id {
        let tokens = st.store.list_tokens(Some(&m.id))?;
        v["tokens"] = json!(tokens.iter().map(token_json).collect::<Vec<_>>());
    }
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub name: String,
}

pub async fn rename(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
    body: Result<Json<RenameBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("member rename")?;
    let Json(body) = body?;
    let m = load_member(&st, &handle)?;
    let m = st.store.rename_member(&auth.0.member_id, &m.id, &body.name)?;
    tracing::info!("member @{} renamed to {:?} by @{}", m.handle, m.display_name, auth.0.handle);
    Ok(Json(member_json(&m)))
}

pub async fn disable(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("member disable")?;
    let m = load_member(&st, &handle)?;
    let changed = st.store.disable_member(&auth.0.member_id, &m.id)?;
    if changed {
        tracing::info!("member @{} disabled by @{}", m.handle, auth.0.handle);
    }
    let m = load_member(&st, &handle)?;
    let mut v = member_json(&m);
    v["changed"] = json!(changed);
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
pub struct RoleBody {
    pub role: String,
}

pub async fn set_role(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
    body: Result<Json<RoleBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("member role")?;
    let Json(body) = body?;
    let role = Role::parse(body.role.trim())
        .ok_or_else(|| ApiError::invalid(format!("role {:?}: use owner or member", body.role)))?;
    let m = load_member(&st, &handle)?;
    let (m, changed) = st.store.set_role(&auth.0.member_id, &m.id, role)?;
    if changed {
        tracing::info!("member @{} is now {} (by @{})", m.handle, m.role, auth.0.handle);
    }
    let mut v = member_json(&m);
    v["changed"] = json!(changed);
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
pub struct GrantBody {
    pub vault: String,
    pub rights: Value,
}

/// `POST …/grants {vault, rights}`: set the Grant on one Vault (`*` for every Vault incl.
/// future ones); `rights: []` removes that Grant.
pub async fn grant(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
    body: Result<Json<GrantBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("member grant")?;
    let Json(body) = body?;
    let rights = parse_rights(&body.rights)?;
    let m = load_member(&st, &handle)?;
    let m = st.store.grant(&auth.0.member_id, &m.id, &body.vault, &rights)?;
    tracing::info!("member @{} grants now {:?} (by @{})", m.handle, m.grants, auth.0.handle);
    Ok(Json(member_json(&m)))
}

#[derive(Debug, Deserialize)]
pub struct GrantsBody {
    pub grants: Vec<Grant>,
}

/// `PUT …/grants {grants: [{vault, rights}]}`: replace the whole list.
pub async fn set_grants(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
    body: Result<Json<GrantsBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("member grant")?;
    let Json(body) = body?;
    let m = load_member(&st, &handle)?;
    let m = st.store.replace_grants(&auth.0.member_id, &m.id, body.grants)?;
    tracing::info!("member @{} grants now {:?} (by @{})", m.handle, m.grants, auth.0.handle);
    Ok(Json(member_json(&m)))
}

// ---------------------------------------------------------------- tokens

#[derive(Debug, Deserialize, Default)]
pub struct TokensQuery {
    /// A handle (owner: any Member; `*` or absent = every Member for an owner, own for others).
    pub member: Option<String>,
}

/// Whose Tokens a caller may see/mint/revoke: their own always; an owner anyone's.
pub(crate) fn subject(st: &AppState, auth: &Auth, member: Option<&str>) -> Result<Option<MemberView>, ApiError> {
    match member.map(str::trim).filter(|m| !m.is_empty()) {
        None => Ok(if auth.0.is_owner() { None } else { Some(load_member(st, &auth.0.handle)?) }),
        Some("*") => {
            auth.require_owner("listing every Member's tokens")?;
            Ok(None)
        }
        Some(h) => {
            let m = load_member(st, h)?;
            if m.id != auth.0.member_id {
                auth.require_owner("managing another Member's tokens")?;
            }
            Ok(Some(m))
        }
    }
}

pub async fn list_tokens(
    State(st): State<AppState>,
    auth: Auth,
    Query(q): Query<TokensQuery>,
) -> Result<Json<Value>, ApiError> {
    let who = subject(&st, &auth, q.member.as_deref())?;
    let rows = st.store.list_tokens(who.as_ref().map(|m| m.id.as_str()))?;
    Ok(Json(json!({ "tokens": rows.iter().map(token_json).collect::<Vec<_>>() })))
}

#[derive(Debug, Deserialize, Default)]
pub struct MintBody {
    #[serde(default)]
    pub member: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// `POST /api/v1/tokens` → 201 `{token, …}` (plain text shown once).
pub async fn mint_token(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<MintBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(body) = body?;
    let target = match subject(&st, &auth, body.member.as_deref())? {
        Some(m) => m,
        None => load_member(&st, &auth.0.handle)?,
    };
    let expires_at = match body.expires_at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(e) => Some(
            time::parse(e)
                .map(time::format)
                .ok_or_else(|| ApiError::invalid(format!("expires_at {e:?} is not an ISO-8601 timestamp")))?,
        ),
        None => None,
    };
    let t = st.store.mint_token_expiring(&auth.0.member_id, &target.id, body.name.as_deref(), expires_at.as_deref())?;
    tracing::info!("token {} minted for @{} by @{}", t.id, target.handle, auth.0.handle);
    let row = st.store.token_row(&t.id)?.ok_or_else(|| ApiError::internal("minted token vanished"))?;
    let mut v = token_json(&row);
    v["token"] = json!(t.plain);
    Ok((StatusCode::CREATED, Json(v)))
}

/// `DELETE /api/v1/tokens/{id}` → revoke.
pub async fn revoke_token(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = id.trim();
    let row = st.store.token_row(id)?.ok_or_else(|| ApiError::not_found(format!("no Token {id}")))?;
    if row.info.member_id != auth.0.member_id {
        auth.require_owner("revoking another Member's token")?;
    }
    let revoked = st.store.revoke_token(&auth.0.member_id, id)?;
    if revoked {
        tracing::info!("token {id} of @{} revoked by @{}", row.handle, auth.0.handle);
    }
    let row = st.store.token_row(id)?.ok_or_else(|| ApiError::internal("token vanished"))?;
    let mut v = token_json(&row);
    v["revoked"] = json!(revoked);
    Ok(Json(v))
}

// ---------------------------------------------------------------- login / logout / password

#[derive(Debug, Deserialize, Default)]
pub struct LoginBody {
    #[serde(default)]
    pub password: String,
}

/// `POST /api/v1/login {password}` — the one unauthenticated mutation: the local password
/// buys a Token (`login`) of the Member it belongs to. CMD-4: shares the web form's failed-login
/// throttle (429 `too_many_attempts` + `Retry-After` while locked out, `login_guard.rs`).
pub async fn login(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<LoginBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(body) = body?;
    let ip = super::login_guard::client_ip(addr.ip(), &headers);
    if let Err(l) = st.login_throttle.check(ip, std::time::Instant::now()) {
        return Ok(super::login_guard::too_many_attempts(l));
    }
    let (m, t) = match st.store.login(&body.password) {
        Ok(x) => x,
        Err(StoreError::WrongPassword) => {
            super::login_guard::note_failure(&st, ip, "api");
            return Err(StoreError::WrongPassword.into());
        }
        Err(e) => return Err(e.into()),
    };
    st.login_throttle.succeed(ip);
    tracing::info!("@{} logged in (token {})", m.handle, t.id);
    let mut v = member_json(&m);
    v["token"] = json!(t.plain);
    v["token_id"] = json!(t.id);
    Ok(Json(v).into_response())
}

/// `POST /api/v1/logout`: a bearer caller revokes the Token it came with; a cookie caller ends
/// its session (and the cookie is cleared).
pub async fn logout(State(st): State<AppState>, auth: Auth) -> Result<Response, ApiError> {
    match &auth.0.via {
        AuthVia::Token { id } => {
            st.store.revoke_token(&auth.0.member_id, id)?;
            tracing::info!("@{} logged out (token {id} revoked)", auth.0.handle);
            Ok(Json(json!({ "handle": auth.0.handle, "ended": "token", "token_id": id })).into_response())
        }
        AuthVia::Session { id } => {
            st.store.end_session(id)?;
            let mut resp = Json(json!({ "handle": auth.0.handle, "ended": "session" })).into_response();
            resp.headers_mut().append(header::SET_COOKIE, super::web::clear_cookie_header());
            Ok(resp)
        }
    }
}

/// `GET /api/v1/settings` (owner) — what ticket 06 knows: `password_set`.
pub async fn settings(State(st): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require_owner("server settings")?;
    Ok(Json(json!({
        "password_set": st.store.password_set()?,
        "my_password": st.store.member_by_id(&auth.0.member_id)?.is_some_and(|m| m.has_password),
        // ---- ticket 13: "purge deleted after N days" (null = off); PUT settings/auto-purge sets it ----
        "auto_purge_days": st.store.auto_purge_days()?,
        // ---- end ticket 13 ----
        // ---- ticket 18: set by `serve --url` / `--link-base` (null = unset) ----
        "public_url": st.store.public_url()?,
        "link_base": st.store.link_base()?,
        // ---- end ticket 18 ----
    })))
}

#[derive(Debug, Deserialize, Default)]
pub struct PasswordBody {
    #[serde(default)]
    pub password: Option<String>,
}

/// `PUT /api/v1/settings/password {password | null}` (owner): set, change or remove the
/// caller's local password. Setting one makes the login page appear on the next loopback request.
pub async fn set_password(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<PasswordBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("the local password")?;
    let Json(body) = body?;
    let has = st.store.set_password(&auth.0.member_id, &auth.0.member_id, body.password.as_deref())?;
    tracing::info!("local password {} by @{}", if has { "set" } else { "removed" }, auth.0.handle);
    Ok(Json(json!({ "handle": auth.0.handle, "has_password": has, "password_set": st.store.password_set()? })))
}

// ---------------------------------------------------------------- vaults

#[derive(Debug, Deserialize)]
pub struct VaultBody {
    pub name: String,
}

/// `POST /api/v1/vaults {name}` (owner) → 201.
pub async fn create_vault(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<VaultBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require_owner("vault create")?;
    let Json(body) = body?;
    // A taken name is 409 `conflict` (`StoreError::VaultNameTaken`, mapped in `error.rs` since ticket 12).
    let v = st.store.create_vault(&auth.0.member_id, &body.name)?;
    tracing::info!("vault {} created by @{}", v.name, auth.0.handle);
    Ok((StatusCode::CREATED, Json(vault_json(&v))))
}
