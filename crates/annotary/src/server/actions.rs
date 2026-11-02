//! `/api/v1/docs/{id}/actions` and the Vault default Actions (ticket 05).
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /api/v1/docs/{id}/actions` | read | `{id, name, version, available[], inherited, action, actions[]}` — the effective declaration, the head Action and the log (oldest first) |
//! | `POST /api/v1/docs/{id}/actions {action, version?}` | action | take an Action on head (201); `version` ≠ head ⇒ 409 `not_head`; an id not in the declaration ⇒ 409 `undeclared_action` (the hint lists the available ids) |
//! | `GET /api/v1/vaults/{name}/actions` | read | `{vault, available[]}` — the Vault defaults |
//! | `PUT /api/v1/vaults/{name}/actions {available[]}` | owner | replace the Vault defaults (`{id, label?, intent?}` list; `[]` allowed); affects only Documents without their own declaration, only future Actions |
//!
//! An Action is `{id, by, at, version, label, intent}` (`by` a handle; `label`/`intent`
//! snapshotted when taken). Declarations arrive with the commit (`actions_available` on
//! `POST /api/v1/docs` and `…/versions`, `docs.rs`) — the server validates them (400
//! `invalid_actions`) and stores them on the Document.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::{ActionDecl, ActionView, DocumentView, Right, StoreError};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;
use super::vaults::vault_json;

/// An Action as the API shows it.
pub fn action_json(a: &ActionView) -> Value {
    json!({
        "id": a.id,
        "by": a.by_handle,
        "at": a.at,
        "version": a.version,
        "label": a.label,
        "intent": a.intent,
    })
}

pub fn decl_json(d: &ActionDecl) -> Value {
    json!({ "id": d.id, "label": d.label, "intent": d.intent })
}

/// Map the store's Action errors onto API errors that name the Document.
fn map_err(doc: &DocumentView, e: StoreError) -> ApiError {
    match e {
        StoreError::NotHead { expected, head } => {
            ApiError::new(StatusCode::CONFLICT, "not_head", format!("head is v{head}, not v{expected}"))
                .hint(format!("run `annotary show {}` and act on v{head} once you have read it", doc.head.name))
        }
        StoreError::UndeclaredAction { action, available } => {
            let mut err = ApiError::new(
                StatusCode::CONFLICT,
                "undeclared_action",
                format!("no Action {action:?} on {} ({})", doc.head.name, doc.id),
            )
            .hint(if available.is_empty() {
                "this Document declares no Actions (`actions.available: []` in its Sidecar)".to_string()
            } else {
                format!("available: {}", available.join(", "))
            });
            err.extra = Some(Box::new(json!({ "available": available })));
            err
        }
        other => other.into(),
    }
}

pub async fn list(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let (available, inherited) = st.store.available_actions(&doc)?;
    let log = st.store.actions(&doc.id)?;
    let head = st.store.head_action(&doc.id, doc.head.number)?;
    Ok(Json(json!({
        "id": doc.id,
        "name": doc.head.name,
        "version": doc.head.number,
        "available": available.iter().map(decl_json).collect::<Vec<_>>(),
        "inherited": inherited,
        "action": head.as_ref().map(action_json).unwrap_or(Value::Null),
        "actions": log.iter().map(action_json).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct ActBody {
    /// The Action id (from the effective declaration).
    pub action: String,
    /// The head Version the caller looked at (refused when the head is not that).
    #[serde(default)]
    pub version: Option<u32>,
}

pub async fn act(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Result<Json<ActBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Action)?;
    let Json(body) = body?;
    let action = body.action.trim();
    if action.is_empty() {
        return Err(ApiError::invalid("empty action id").hint("`annotary show DOC` lists the available Actions"));
    }
    let a = st.store.act(&auth.0.member_id, &doc.id, action, body.version).map_err(|e| map_err(&doc, e))?;
    tracing::info!("action {} v{} on {} ({}) by @{}", a.id, a.version, doc.head.name, doc.id, auth.0.handle);
    let mut v = action_json(&a);
    v["doc"] = json!(doc.id);
    v["name"] = json!(doc.head.name);
    Ok((StatusCode::CREATED, Json(v)))
}

fn load_vault(st: &AppState, name: &str) -> Result<annotary_store::Vault, ApiError> {
    st.store.vault_by_name(name.trim())?.ok_or_else(|| ApiError::not_found(format!("no Vault {}", name.trim())))
}

pub async fn vault_get(
    State(st): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let v = load_vault(&st, &name)?;
    auth.require_right(&v.name, Right::Read)?;
    let mut j = vault_json(&v);
    j["available"] = v.default_actions.clone();
    Ok(Json(j))
}

#[derive(Debug, Deserialize)]
pub struct VaultActionsBody {
    pub available: Value,
}

pub async fn vault_set(
    State(st): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
    body: Result<Json<VaultActionsBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("editing Vault default Actions")?;
    let v = load_vault(&st, &name)?;
    let Json(body) = body?;
    let v = st.store.set_vault_actions(&auth.0.member_id, &v.id, &body.available)?;
    tracing::info!("vault {} default Actions set by @{}", v.name, auth.0.handle);
    let mut j = vault_json(&v);
    j["available"] = v.default_actions.clone();
    Ok(Json(j))
}
