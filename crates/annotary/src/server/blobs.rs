//! Content-addressed blobs over HTTP — the two-phase push's transport:
//!
//! - `POST /api/v1/blobs/check {hashes:[…]}` **or** the manifest form
//!   `{md_hash, assets:[{ref, hash, size, type, derivative?, …}]}` → `{lacking:[…]}` (which of
//!   the Markdown / Asset / derivative hashes to upload). Claimed Asset sizes over the caps are
//!   refused here already (413 `too_large`), before any bytes fly.
//! - `PUT /api/v1/blobs/{hash}` raw bytes → `{hash, size, created}` (400 when the bytes do not
//!   hash to `{hash}`; 413 `too_large` past [`annotary_store::caps::ASSET_MAX`] — the body is
//!   read as a stream and capped, never buffered past the cap).
//! - `GET /api/v1/blobs/{hash}` → the raw bytes (`application/octet-stream`).
//!
//! Uploading needs `write` on some Vault, reading `read` on some Vault (a blob is not tied to a
//! Vault; hashes are unguessable). The web-facing `/a/{hash}` route with content types lives in
//! `assets.rs`; the caps are `annotary_store::caps` and the commit-time manifest checks
//! `annotary_store::manifest`.

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::Right;
use annotary_store::blobs::is_blob_hash;
use annotary_store::caps::{ASSET_MAX, VERSION_MAX};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;

/// After a body passes the cap, this many more bytes are read and discarded so the client sees
/// the typed 413 instead of a reset connection.
const DRAIN_BUDGET: u64 = 8 * 1024 * 1024;

pub(crate) fn require_any_right(auth: &Auth, right: Right, what: &str) -> Result<(), ApiError> {
    if auth.0.grants.iter().any(|g| g.covers(&g.vault, right)) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!("{what} needs the `{right}` right on a Vault (you are @{})", auth.0.handle),
        )
        .hint(format!("ask an owner: annotary member grant {} VAULT {right}", auth.0.handle)))
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct CheckBody {
    #[serde(default)]
    pub hashes: Vec<String>,
    /// Manifest form: the Markdown blob's hash …
    #[serde(default)]
    pub md_hash: Option<String>,
    /// … and the manifest entries (`hash`, `derivative`, `size` are read).
    #[serde(default)]
    pub assets: Vec<Value>,
    /// … and the rendered HTML's hash (ticket 09).
    #[serde(default)]
    pub html_hash: Option<String>,
}

pub async fn check(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<CheckBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    require_any_right(&auth, Right::Write, "blob upload")?;
    let Json(body) = body?;
    let mut hashes = body.hashes.clone();
    if let Some(md) = &body.md_hash {
        hashes.push(md.clone());
    }
    if let Some(h) = &body.html_hash {
        hashes.push(h.clone()); // ticket 09
    }
    let mut total: u64 = 0;
    for (i, a) in body.assets.iter().enumerate() {
        let obj = a.as_object().ok_or_else(|| ApiError::invalid(format!("assets[{i}] is not an object")))?;
        let reference = obj.get("ref").and_then(Value::as_str).unwrap_or("?");
        if obj.get("missing").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        for key in ["hash", "derivative"] {
            match obj.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::String(h)) => hashes.push(h.clone()),
                Some(_) => return Err(ApiError::invalid(format!("assets[{i}] {key} is not a hash"))),
            }
        }
        if let Some(size) = obj.get("size").and_then(Value::as_u64) {
            if size > ASSET_MAX {
                return Err(ApiError::too_large(format!("Asset {reference}"), size, ASSET_MAX));
            }
            total = total.saturating_add(size);
        }
    }
    if total > VERSION_MAX {
        return Err(ApiError::too_large("the Version's Assets together".to_string(), total, VERSION_MAX));
    }
    if let Some(bad) = hashes.iter().find(|h| !is_blob_hash(h)) {
        return Err(ApiError::invalid(format!("{bad:?} is not a sha256 hex hash")));
    }
    hashes.sort();
    hashes.dedup();
    Ok(Json(json!({ "lacking": st.store.lacking_blobs(&hashes) })))
}

pub async fn put(
    State(st): State<AppState>,
    auth: Auth,
    Path(hash): Path<String>,
    body: Body,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_any_right(&auth, Right::Write, "blob upload")?;
    if !is_blob_hash(&hash) {
        return Err(ApiError::invalid(format!("{hash:?} is not a sha256 hex hash")));
    }
    let bytes = read_capped(body, ASSET_MAX).await?;
    let (hash, created) = st.store.put_blob_expecting(&hash, &bytes)?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(json!({ "hash": hash, "size": bytes.len(), "created": created }))))
}

/// Read a request body up to `cap` bytes. Past the cap the rest is drained (within a budget)
/// and the typed `too_large` error is returned; nothing beyond `cap` is ever buffered.
async fn read_capped(mut body: Body, cap: u64) -> Result<Vec<u8>, ApiError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut total: u64 = 0;
    let mut over = false;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| ApiError::invalid(format!("body read failed: {e}")))?;
        let Ok(data) = frame.into_data() else { continue };
        total += data.len() as u64;
        if over {
            if total > cap + DRAIN_BUDGET {
                break;
            }
            continue;
        }
        if total > cap {
            over = true;
            buf = Vec::new();
            continue;
        }
        buf.extend_from_slice(&data);
    }
    if over {
        return Err(ApiError::too_large("this blob".to_string(), total, cap));
    }
    Ok(buf)
}

pub async fn get(State(st): State<AppState>, auth: Auth, Path(hash): Path<String>) -> Result<Response, ApiError> {
    require_any_right(&auth, Right::Read, "blob read")?;
    let Some(bytes) = st.store.read_blob(&hash)? else {
        return Err(ApiError::not_found(format!("no blob {hash}")));
    };
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}
