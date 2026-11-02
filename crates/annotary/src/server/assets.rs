//! `GET /a/{hash}[?derivative=1]` — every Asset a Version references, at a stable URL, with the
//! content type its manifest entry recorded (a derivative is sniffed: PNG or JPEG). This is the
//! URL the web UI (ticket 10) uses for inline media and new-tab links. Content-addressed ⇒
//! immutable ⇒ cached forever. `?derivative=1` serves the web-safe derivative when the entry
//! has one, else the original. Needs `read` on some Vault (like `/api/v1/blobs/{hash}`;
//! ticket 10/12 add the browser cookie to `Auth`).

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use annotary_store::Right;

use super::app::AppState;
use super::auth::Auth;
use super::blobs::require_any_right;
use super::error::ApiError;
use crate::assets::types::sniff_derivative_type;

#[derive(Debug, Deserialize, Default)]
pub struct AssetQuery {
    /// `1`/`true` = the web-safe derivative when there is one.
    pub derivative: Option<String>,
}

pub async fn get(
    State(st): State<AppState>,
    auth: Auth,
    Path(hash): Path<String>,
    Query(q): Query<AssetQuery>,
) -> Result<Response, ApiError> {
    require_any_right(&auth, Right::Read, "asset read")?;
    let Some(info) = st.store.asset_info(&hash)? else {
        return Err(ApiError::not_found(format!("no Asset {hash}"))
            .hint("Assets are addressed by the hash in a Version's manifest"));
    };
    let want_derivative = matches!(q.derivative.as_deref().map(str::trim), Some("1") | Some("true") | Some("yes"));
    let (serve_hash, is_derivative) = match (&info.derivative, want_derivative, info.is_derivative) {
        (Some(d), true, _) => (d.clone(), true),
        (_, _, is_d) => (info.hash.clone(), is_d),
    };
    let Some(bytes) = st.store.read_blob(&serve_hash)? else {
        return Err(ApiError::not_found(format!("no Asset {hash}")));
    };
    let content_type = if is_derivative {
        sniff_derivative_type(&bytes).to_string()
    } else if info.content_type.starts_with("text/") || info.content_type == "image/svg+xml" {
        format!("{}; charset=utf-8", info.content_type)
    } else {
        info.content_type.clone()
    };
    let base = info.reference.rsplit('/').next().unwrap_or(&info.reference).replace(['"', '\\', '\r', '\n'], "_");
    let filename = if is_derivative {
        let stem = base.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| base.clone());
        let ext = if content_type.starts_with("image/png") { "png" } else { "jpg" };
        format!("{stem}.{ext}")
    } else {
        base
    };
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, name: header::HeaderName, value: &str| {
        if let Ok(v) = header::HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(h, header::CONTENT_TYPE, &content_type);
    set(h, header::CACHE_CONTROL, "public, max-age=31536000, immutable");
    set(h, header::CONTENT_DISPOSITION, &format!("inline; filename=\"{filename}\""));
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::ETAG, &format!("\"{serve_hash}\""));
    if content_type.starts_with("image/svg+xml") {
        // SVG can carry scripts; served same-origin as the app it must never run them.
        set(h, header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'");
    }
    Ok(resp)
}
