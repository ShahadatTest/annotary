//! `/api/v1/members` (list: any Member; add: owner-only) and `/api/v1/me`.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::MemberView;

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;

/// JSON shape of a Member (shared by list/add/me).
pub fn member_json(m: &MemberView) -> Value {
    json!({
        "id": m.id,
        "handle": m.handle,
        "name": m.display_name,
        "role": m.role.as_str(),
        "grants": m.grants,
        "created_at": m.created_at,
        "disabled_at": m.disabled_at,
        "has_password": m.has_password,
    })
}

/// `GET /api/v1/members` → `{"members":[…]}`. Any Member may list (handles are needed for
/// `@mentions`); mutations are owner-only.
pub async fn list(State(st): State<AppState>, _auth: Auth) -> Result<Json<Value>, ApiError> {
    let members = st.store.list_members()?;
    Ok(Json(json!({ "members": members.iter().map(member_json).collect::<Vec<_>>() })))
}

/// `GET /api/v1/me` → the caller.
/// CMD-5: also reports the profile's Vault (`vaults::echo_profile_vault`) — what `profile list`
/// reconciles.
pub async fn me(
    State(st): State<AppState>,
    auth: Auth,
    pv: super::vaults::ProfileVaultHeader,
) -> Result<Json<Value>, ApiError> {
    super::vaults::echo_profile_vault(&st, &auth, &pv);
    let m = st.store.member_by_id(&auth.0.member_id)?.ok_or_else(|| ApiError::not_found("member not found"))?;
    Ok(Json(member_json(&m)))
}

#[derive(Debug, Deserialize)]
pub struct AddMemberBody {
    pub handle: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /api/v1/members {handle, name?}` (owner-only) → 201 with the Member and its Token
/// (`token` — plain text, shown once).
pub async fn add(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<AddMemberBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    auth.require_owner("member add")?;
    let Json(body) = body?;
    let handle = body.handle.trim();
    let (member, token) = st.store.add_member(&auth.0.member_id, handle, body.name.as_deref())?;
    tracing::info!("member @{} added by @{}", member.handle, auth.0.handle);
    let mut v = member_json(&member);
    v["token"] = json!(token.plain);
    Ok((StatusCode::CREATED, Json(v)))
}
