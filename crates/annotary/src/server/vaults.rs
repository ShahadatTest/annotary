//! `/api/v1/vaults`: the Vaults the caller can read (what `push` needs to pick a Vault), and —
//! ticket 12, owner-only — rename and delete (create is `access::create_vault`, ticket 06; the
//! default Actions are `actions::vault_get/vault_set`, ticket 05).
//!
//! | route | who | what |
//! |---|---|---|
//! | `GET /api/v1/vaults` | any Member | `{vaults[]}` — the Vaults the caller holds `read` on |
//! | `POST /api/v1/vaults {name}` | owner | create (ticket 06, `access.rs`) |
//! | `POST /api/v1/vaults/{name}/rename {name}` | owner | rename; Grants naming the Vault follow; 409 `conflict` when the name is taken |
//! | `DELETE /api/v1/vaults/{name}` | owner | delete an empty Vault; 409 `vault_not_empty` (Documents — deleted ones included — still in it), 409 `last_vault` |
//!
//! Rename answers with the Vault plus its Document counts (`documents`, `deleted`) and `from`;
//! delete with the Vault as it was and `deleted: true`. CMD-5: the list rows carry the counts too
//! (`vault list`).
//!
//! **A profile's Vault across renames (CMD-5).** The CLI sends its profile's `vault` (and, once
//! known, the Vault's id) on every request as [`PROFILE_VAULT`] / [`PROFILE_VAULT_ID`] (the name
//! percent-encoded with `client::qs`: header values are ASCII), beside the usual `vault=` query or
//! body field (so an older server behaves as before). A Vault-scoped route ([`scoped_vault`]:
//! `GET /api/v1/docs`, `…/docs/resolve`, `POST /api/v1/docs`, `GET /api/v1/mentions`) whose
//! `vault` is absent or equal to the header resolves it with `Store::resolve_profile_vault` —
//! through any renames, in the same round trip — and answers with [`RESOLVED_VAULT`] /
//! [`RESOLVED_VAULT_ID`]; a deleted one answers with [`DELETED_VAULT`] and the route falls back to
//! every readable Vault (`POST /api/v1/docs` refuses: 404 `vault_deleted`). The CLI rewrites the
//! profile from those headers. `GET /api/v1/vaults` and `GET /api/v1/me` only echo
//! ([`echo_profile_vault`]). A resolved Vault is echoed only when the caller holds the right the
//! route needs on it (else 403 naming the profile's name, nothing echoed); an explicit `vault`
//! nobody has (404) names the Vault it was renamed to only to a caller who may read that one.

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequestParts, Path, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::{ProfileVault, Right, Vault};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use crate::client::{qs, unqs};

/// Request: the CLI profile's `vault` name (percent-encoded).
pub const PROFILE_VAULT: &str = "x-annotary-profile-vault";
/// Request: the id the CLI recorded for that Vault (`vault_id` in `config.toml`), when it has one.
pub const PROFILE_VAULT_ID: &str = "x-annotary-profile-vault-id";
/// Response: the profile's Vault now (percent-encoded name) …
pub const RESOLVED_VAULT: &str = "x-annotary-resolved-vault";
/// … and its id.
pub const RESOLVED_VAULT_ID: &str = "x-annotary-resolved-vault-id";
/// Response: the profile's Vault was deleted (its percent-encoded name as the profile had it).
pub const DELETED_VAULT: &str = "x-annotary-deleted-vault";

/// What the response says about the profile's Vault (filled by the route, written by
/// [`echo_layer`]).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Echo {
    Resolved { name: String, id: String },
    Deleted { name: String },
}

#[derive(Clone, Default)]
struct EchoSlot(Arc<Mutex<Option<Echo>>>);

/// The profile's Vault a request carries ([`PROFILE_VAULT`] / [`PROFILE_VAULT_ID`]); `name` is
/// `None` without the header (a browser, iOS, curl, an older CLI).
pub struct ProfileVaultHeader {
    pub name: Option<String>,
    pub id: Option<String>,
    slot: Option<EchoSlot>,
}

impl ProfileVaultHeader {
    /// No profile Vault (the web pages).
    pub fn none() -> ProfileVaultHeader {
        ProfileVaultHeader { name: None, id: None, slot: None }
    }

    fn echo(&self, e: Echo) {
        if let Some(slot) = &self.slot {
            *slot.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
        }
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(name)?.to_str().ok()?;
    unqs(raw.trim()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl<S: Send + Sync> FromRequestParts<S> for ProfileVaultHeader {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let name = header_text(&parts.headers, PROFILE_VAULT);
        let id = name.as_ref().and_then(|_| header_text(&parts.headers, PROFILE_VAULT_ID));
        Ok(ProfileVaultHeader { name, id, slot: parts.extensions.get::<EchoSlot>().cloned() })
    }
}

/// Middleware (the whole router): a request carrying [`PROFILE_VAULT`] gets a slot the route
/// fills, and the response — success or error — carries what it says.
pub async fn echo_layer(mut req: Request, next: Next) -> Response {
    if !req.headers().contains_key(PROFILE_VAULT) {
        return next.run(req).await;
    }
    let slot = EchoSlot::default();
    req.extensions_mut().insert(slot.clone());
    let mut resp = next.run(req).await;
    let echo = slot.0.lock().unwrap_or_else(|p| p.into_inner()).take();
    let h = resp.headers_mut();
    let val = |s: &str| HeaderValue::from_str(&qs(s)).ok();
    match echo {
        Some(Echo::Resolved { name, id }) => {
            if let (Some(n), Some(i)) = (val(&name), val(&id)) {
                h.insert(RESOLVED_VAULT, n);
                h.insert(RESOLVED_VAULT_ID, i);
            }
        }
        Some(Echo::Deleted { name }) => {
            if let Some(n) = val(&name) {
                h.insert(DELETED_VAULT, n);
            }
        }
        None => {}
    }
    resp
}

/// The Vault a Vault-scoped request covers.
#[derive(Clone, Debug)]
pub enum Scope {
    /// One Vault (named, or the profile's — resolved through renames); the caller holds the right.
    Vault(Vault),
    /// No Vault named: every Vault the caller can read.
    Every,
    /// The profile's Vault was deleted (echoed); the route decides (list-like ones: `Every`).
    Deleted(String),
}

/// A 404 for a Vault `name` nobody has; when it was renamed (one live Vault carried it) and the
/// caller may read that Vault, the hint names it.
pub fn vault_not_found(st: &AppState, auth: &Auth, name: &str) -> ApiError {
    let err = ApiError::not_found(format!("no Vault {name}"));
    match st.store.renamed_from_hint(name) {
        Ok(Some(v)) if auth.0.has_right(&v.name, Right::Read) => {
            err.hint(format!("Vault {name} was renamed to {}", v.name))
        }
        _ => err,
    }
}

/// Resolve the profile's Vault for a route that only reports it (`GET /api/v1/vaults`, `/me`):
/// echoed when readable (or deleted); never an error.
pub fn echo_profile_vault(st: &AppState, auth: &Auth, pv: &ProfileVaultHeader) {
    let Some(name) = pv.name.as_deref() else { return };
    match st.store.resolve_profile_vault(name, pv.id.as_deref()) {
        Ok(ProfileVault::Resolved(v)) if auth.0.has_right(&v.name, Right::Read) => {
            pv.echo(Echo::Resolved { name: v.name, id: v.id })
        }
        Ok(ProfileVault::Deleted { old_name }) => pv.echo(Echo::Deleted { name: old_name }),
        _ => {}
    }
}

/// The Vault a Vault-scoped route covers: `explicit` (`vault=` / the body's `vault`) by name, or —
/// when that is absent or equal to the profile's Vault in [`PROFILE_VAULT`] — the profile's Vault
/// resolved through renames (echoed). The caller must hold `right` on the Vault (403 otherwise,
/// naming the Vault as the request did); an unknown name is 404 (with a rename hint).
pub fn scoped_vault(
    st: &AppState,
    auth: &Auth,
    pv: &ProfileVaultHeader,
    explicit: Option<&str>,
    right: Right,
) -> Result<Scope, ApiError> {
    let explicit = explicit.map(str::trim).filter(|v| !v.is_empty());
    if let Some(name) = pv.name.as_deref()
        && explicit.is_none_or(|e| e == name)
    {
        return match st.store.resolve_profile_vault(name, pv.id.as_deref())? {
            ProfileVault::Resolved(v) => {
                if !auth.0.has_right(&v.name, right) {
                    return Err(ApiError::missing_right(right.as_str(), name, &auth.0.handle));
                }
                pv.echo(Echo::Resolved { name: v.name.clone(), id: v.id.clone() });
                Ok(Scope::Vault(v))
            }
            ProfileVault::Deleted { old_name } => {
                pv.echo(Echo::Deleted { name: old_name.clone() });
                Ok(Scope::Deleted(old_name))
            }
            ProfileVault::NotFound => Err(vault_not_found(st, auth, name)),
        };
    }
    match explicit {
        Some(name) => {
            let v = st.store.vault_by_name(name)?.ok_or_else(|| vault_not_found(st, auth, name))?;
            auth.require_right(&v.name, right)?;
            Ok(Scope::Vault(v))
        }
        None => Ok(Scope::Every),
    }
}

/// [`scoped_vault`] as the Vault ids a read covers (a deleted profile Vault ⇒ every readable one).
pub fn scoped_vault_ids(
    st: &AppState,
    auth: &Auth,
    pv: &ProfileVaultHeader,
    explicit: Option<&str>,
) -> Result<(Vec<String>, Option<String>), ApiError> {
    Ok(match scoped_vault(st, auth, pv, explicit, Right::Read)? {
        Scope::Vault(v) => (vec![v.id], Some(v.name)),
        Scope::Every | Scope::Deleted(_) => (readable_vaults(st, auth)?.into_iter().map(|v| v.id).collect(), None),
    })
}

pub fn vault_json(v: &Vault) -> Value {
    json!({
        "id": v.id,
        "name": v.name,
        "default_actions": v.default_actions,
        "created_at": v.created_at,
    })
}

/// Vaults the caller holds `read` on (through their Grants — the owner's `*` grant covers all),
/// oldest first.
pub fn readable_vaults(st: &AppState, auth: &Auth) -> Result<Vec<Vault>, ApiError> {
    let all = st.store.list_vaults()?;
    Ok(all.into_iter().filter(|v| auth.0.has_right(&v.name, Right::Read)).collect())
}

/// `GET /api/v1/vaults` → `{vaults[]}` with each Vault's Document counts (CMD-5: `vault list`).
pub async fn list(State(st): State<AppState>, auth: Auth, pv: ProfileVaultHeader) -> Result<Json<Value>, ApiError> {
    echo_profile_vault(&st, &auth, &pv);
    let vaults = readable_vaults(&st, &auth)?;
    let rows = vaults.iter().map(|v| vault_json_counted(&st, v)).collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!({ "vaults": rows })))
}

// ---- ticket 12: rename / delete (owner-only) ----

pub fn load_vault(st: &AppState, name: &str) -> Result<Vault, ApiError> {
    st.store
        .vault_by_name(name.trim())?
        .ok_or_else(|| ApiError::not_found(format!("no Vault {}", name.trim())).hint("`GET /api/v1/vaults` lists them"))
}

/// [`load_vault`] for the owner's rename / delete: a 404 for a Vault renamed away names the new
/// name (CMD-5), else the usual hint.
fn load_vault_for(st: &AppState, auth: &Auth, name: &str) -> Result<Vault, ApiError> {
    match st.store.vault_by_name(name.trim())? {
        Some(v) => Ok(v),
        None => {
            let err = vault_not_found(st, auth, name.trim());
            Err(if err.hint.is_some() { err } else { err.hint("`GET /api/v1/vaults` lists them") })
        }
    }
}

/// A Vault with its Document counts (what the Settings screen shows).
pub fn vault_json_counted(st: &AppState, v: &Vault) -> Result<Value, ApiError> {
    let c = st.store.vault_count(&v.id)?;
    let mut j = vault_json(v);
    j["documents"] = json!(c.documents);
    j["deleted"] = json!(c.deleted);
    Ok(j)
}

#[derive(Debug, Deserialize)]
pub struct RenameBody {
    pub name: String,
}

/// `POST /api/v1/vaults/{name}/rename {name}` (owner).
pub async fn rename(
    State(st): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
    body: Result<Json<RenameBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("vault rename")?;
    let Json(body) = body?;
    let v = load_vault_for(&st, &auth, &name)?;
    let renamed = st.store.rename_vault(&auth.0.member_id, &v.id, &body.name)?;
    if renamed.name != v.name {
        tracing::info!("vault {} renamed to {} by @{}", v.name, renamed.name, auth.0.handle);
    }
    let mut j = vault_json_counted(&st, &renamed)?;
    j["from"] = json!(v.name);
    Ok(Json(j))
}

/// `DELETE /api/v1/vaults/{name}` (owner) → the Vault as it was, `deleted: true`.
pub async fn delete(State(st): State<AppState>, auth: Auth, Path(name): Path<String>) -> Result<Json<Value>, ApiError> {
    auth.require_owner("vault delete")?;
    let v = load_vault_for(&st, &auth, &name)?;
    let gone = st.store.delete_vault(&auth.0.member_id, &v.id)?;
    tracing::info!("vault {} deleted by @{}", gone.name, auth.0.handle);
    let mut j = vault_json(&gone);
    j["deleted"] = json!(true);
    Ok(Json(j))
}
// ---- end ticket 12 ----
