//! Pairing a device (ticket 18, the iOS app's R2): one request mints a Token for a Member and
//! returns everything a phone needs to use it — the `annotary://pair` URL and its QR code.
//!
//! | route | who | what |
//! |---|---|---|
//! | `POST /api/v1/pairing {member?, name?}` | any Member for themselves; owner for another | 201 `{pair_url, server, profile, member, token, token_id, name, qr_svg}` |
//!
//! - The Token goes through the ordinary store path (`mint_token_expiring`, no expiry, named
//!   `name` — default `device`), so it is listed, revocable and audited (`token.minted`) like any
//!   other; the plain text appears in this response only.
//! - `server` is the stored public URL (`serve --url`); without one ⇒ 409 `no_public_url` (the
//!   phone could not reach a loopback URL), checked before anything is minted.
//! - `profile` = `<handle>@<first DNS label of the host>` (an IP host: `<handle>@<ip>`) —
//!   `annotary_store::pair_profile`; `pair_url` = `annotary://pair?v=1&server=…&token=…&profile=…`
//!   (percent-encoded, `annotary_store::pair_url`); `qr_svg` = an inline SVG of `pair_url`
//!   (`crate::qr::svg`).
//!
//! Errors: 403 `managing another Member's tokens is owner-only`, 404 `no Member @h`, 409
//! `no_public_url`, 409 `disabled` (a disabled Member gets no Token).

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use super::access::{load_member, subject};
use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;

/// Default Token name of a paired device.
pub const DEFAULT_NAME: &str = "device";

#[derive(Debug, Deserialize, Default)]
pub struct PairBody {
    #[serde(default)]
    pub member: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// The 409 when no public URL is set.
pub fn no_public_url() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "no_public_url",
        "no public URL is set — a phone cannot reach this server's loopback address",
    )
    .hint("annotary serve --url http://HOST:PORT")
}

/// `POST /api/v1/pairing {member?, name?}` → 201.
pub async fn pair(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<PairBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(body) = body?;
    let target = match body.member.as_deref().map(str::trim).filter(|m| !m.is_empty() && *m != "*") {
        Some(h) => subject(&st, &auth, Some(h))?.ok_or_else(|| ApiError::internal("no Member"))?,
        None => load_member(&st, &auth.0.handle)?,
    };
    let server = st.store.public_url()?.ok_or_else(no_public_url)?;
    let name = body.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(DEFAULT_NAME).to_string();
    let t = st.store.mint_token_expiring(&auth.0.member_id, &target.id, Some(&name), None)?;
    tracing::info!("pairing: token {} minted for @{} by @{}", t.id, target.handle, auth.0.handle);
    let profile = annotary_store::pair_profile(&target.handle, &server);
    let pair_url = annotary_store::pair_url(&server, &t.plain, &profile);
    let qr_svg = crate::qr::svg(&pair_url);
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "pair_url": pair_url,
            "server": server,
            "profile": profile,
            "member": target.handle,
            "token": t.plain,
            "token_id": t.id,
            "name": name,
            "qr_svg": qr_svg,
        })),
    ))
}
