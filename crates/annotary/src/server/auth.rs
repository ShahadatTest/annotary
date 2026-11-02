//! Auth: `Authorization: Bearer <token>` (the CLI) or the `cd_session` cookie (a browser) →
//! `AuthMember` or 401. Rights checks live on the extracted value (`require_owner`,
//! `require_right`) so every handler reads the same way.
//!
//! API routes (`/api/v1/…`) never sign anyone in by themselves: neither cookie nor bearer ⇒
//! 401. Web routes (`/`, `/a/{hash}`, the pages of tickets 10/12) sit behind
//! [`super::web::require_web_auth`], which additionally trusts a loopback browser as the founding
//! owner while no local password is set (issuing the session cookie) and otherwise redirects to
//! `/login`; it stores the resolved [`Auth`] in the request extensions, which the extractor
//! reads first.

use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::request::Parts;

use annotary_store::{AuthMember, Right};

use super::app::AppState;
use super::error::ApiError;

/// Name of the browser session cookie.
pub const SESSION_COOKIE: &str = "cd_session";

/// The authenticated caller.
#[derive(Clone, Debug)]
pub struct Auth(pub AuthMember);

impl Auth {
    /// 403 unless the caller is an owner.
    pub fn require_owner(&self, what: &str) -> Result<(), ApiError> {
        if self.0.is_owner() { Ok(()) } else { Err(ApiError::owner_only(what)) }
    }

    /// 403 unless the caller holds `right` on `vault`.
    pub fn require_right(&self, vault: &str, right: Right) -> Result<(), ApiError> {
        if self.0.has_right(vault, right) {
            Ok(())
        } else {
            Err(ApiError::missing_right(right.as_str(), vault, &self.0.handle))
        }
    }
}

/// Pull the bearer token out of the headers, if any.
pub fn bearer_token(parts: &Parts) -> Option<String> {
    bearer_from(&parts.headers)
}

fn bearer_from(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let t = rest.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// The `cd_session` cookie value, if any (ticket 06).
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    for v in headers.get_all(axum::http::header::COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        for pair in s.split(';') {
            let pair = pair.trim();
            if let Some(val) = pair.strip_prefix(SESSION_COOKIE).and_then(|r| r.strip_prefix('='))
                && !val.trim().is_empty()
            {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

/// What a request carries: a bearer Token, a session cookie, or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presented {
    Bearer(String),
    Cookie(String),
    Nothing,
}

pub fn presented(headers: &HeaderMap) -> Presented {
    if let Some(t) = bearer_from(headers) {
        Presented::Bearer(t)
    } else if let Some(c) = session_cookie(headers) {
        Presented::Cookie(c)
    } else {
        Presented::Nothing
    }
}

/// Resolve the credential a request presents (bearer first, then cookie). `Ok(None)` when it
/// presents nothing; `Err(unauthorized)` when it presents something invalid.
pub fn resolve(state: &AppState, headers: &HeaderMap) -> Result<Option<Auth>, ApiError> {
    match presented(headers) {
        Presented::Bearer(t) => match state.store.authenticate(&t)? {
            Some(m) => {
                check_device(state, headers, &m)?;
                Ok(Some(Auth(m)))
            }
            None => Err(ApiError::unauthorized()),
        },
        Presented::Cookie(c) => match state.store.authenticate_session(&c)? {
            Some(m) => Ok(Some(Auth(m))),
            None => Err(ApiError::session_expired()),
        },
        Presented::Nothing => Ok(None),
    }
}

// ---- CMD-23 ----
/// A Token request that names its Device (`X-Annotary-Device`): a malformed id is 400
/// `bad_device`, a forgotten one 409 `device_forgotten`; a known one is stamped seen (and the
/// Token's `device_id`), outside the commit hook. An unknown id is left for the settings routes
/// to make. A session is always the Web Device (stamped in `authenticate_session`).
fn check_device(state: &AppState, headers: &HeaderMap, m: &AuthMember) -> Result<(), ApiError> {
    let Some(id) = super::me_settings::header_device(headers) else { return Ok(()) };
    if !annotary_store::settings::is_device_id(&id) {
        return Err(annotary_store::StoreError::BadDevice(id).into());
    }
    let token = match &m.via {
        annotary_store::AuthVia::Token { id } => Some(id.as_str()),
        annotary_store::AuthVia::Session { .. } => None,
    };
    match state.store.touch_device(&m.member_id, &id, token)? {
        annotary_store::DeviceTouch::Forgotten => Err(annotary_store::StoreError::DeviceForgotten(id).into()),
        _ => Ok(()),
    }
}
// ---- end CMD-23 ----

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        // Already resolved by the web middleware (which may have just issued the cookie).
        if let Some(a) = parts.extensions.get::<Auth>() {
            return Ok(a.clone());
        }
        match resolve(state, &parts.headers)? {
            Some(a) => Ok(a),
            None => Err(ApiError::missing_credential()),
        }
    }
}
