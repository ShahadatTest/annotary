//! Member settings and Devices over HTTP (CMD-23). Every route is the caller's own: no route
//! reads or changes another Member's settings or Devices, owners included.
//!
//! | route | what |
//! |---|---|
//! | `GET /api/v1/me/settings/catalog` | the catalog both clients render from (`settings::catalog_json`) |
//! | `GET /api/v1/me/settings?device=ID` | every setting resolved for that Device (default: the request's own Device; none ⇒ All devices / default), each `{key, kind, value, source}`; `vaults[]` the caller can read (Default Vault's choices) |
//! | `GET /api/v1/me/settings/matrix` | each device setting's All devices value, every non-forgotten Device's own value, what each resolves to, `mixed` |
//! | `PUT /api/v1/me/settings {key, value, scope?, devices?, stamp?}` | write one setting; `scope` is `user` (the default for a user setting), `all` or `devices` (+ `devices[]`); `null` at `devices` is a tombstone; `stamp` = a client's (iOS sync) stamp |
//! | `DELETE /api/v1/me/settings/{key}?scope=user\|all\|device&device=ID` | Reset (server-stamped) |
//! | `GET /api/v1/me/devices` | the caller's Devices (`?all=1` adds forgotten ones) and `current` |
//! | `PATCH /api/v1/me/devices/{id} {name}` | rename (never Web) |
//! | `POST /api/v1/me/devices/{id}/forget` | revoke the Tokens / sessions last used from it, mark it forgotten (never Web) |
//! | `POST /api/v1/me/devices/{id}/copy-from {from}` | copy every device setting of `from` onto `{id}` (`all` = All devices) as new changes |
//! | `GET /api/v1/me/devices/{id}/sync` | per device setting `{own?: {value, stamp}, all?: {value, stamp}}` (what iOS syncs from) |
//! | `POST /api/v1/me/rename {name}` | a Member renames themselves (the owner route stays) |
//!
//! The request's Device: a session (any browser, or a loopback page) is **Web**; a Token request
//! names its Device with `X-Annotary-Device: ID` (+ `X-Annotary-Device-Info: kind=iphone;
//! name=iPhone 17 Pro`, which names a Device the server has not seen). `auth::resolve` checks the
//! id on every request (400 `bad_device`, 409 `device_forgotten`) and stamps it seen; only these
//! routes make a Device (`Store::ensure_device`).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::settings::{self, SettingKind, WEB_DEVICE};
use annotary_store::{AuthVia, DeviceView, PutOutcome, PutSetting, Scope};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::members::member_json;
use super::vaults::readable_vaults;

/// The header naming the request's Device.
pub const DEVICE_HEADER: &str = "x-annotary-device";
/// The header that names a Device the server has not seen: `kind=iphone; name=iPhone 17 Pro`.
pub const DEVICE_INFO_HEADER: &str = "x-annotary-device-info";

/// The Device id the request names in its header, if any (unchecked).
pub fn header_device(headers: &HeaderMap) -> Option<String> {
    headers
        .get(DEVICE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `kind=…; name=…` from the info header.
fn header_info(headers: &HeaderMap) -> Option<(String, String)> {
    let raw = headers.get(DEVICE_INFO_HEADER)?.to_str().ok()?;
    let (mut kind, mut name) = (String::new(), String::new());
    for part in raw.split(';') {
        if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "kind" => kind = v.trim().to_string(),
                "name" => name = percent_decode(v.trim()),
                _ => {}
            }
        }
    }
    Some((kind, name))
}

/// Header values are ASCII: a client may percent-encode a name (`iPad Pro 13%E2%80%B3`).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// The request's own Device: Web for a session; the header's Device for a Token (made on first
/// use, here only); `None` for a Token request that names none (the CLI, an agent).
pub fn request_device(st: &AppState, auth: &Auth, headers: &HeaderMap) -> Result<Option<DeviceView>, ApiError> {
    let id = match &auth.0.via {
        AuthVia::Session { .. } => Some(WEB_DEVICE.to_string()),
        AuthVia::Token { .. } => header_device(headers),
    };
    let Some(id) = id else { return Ok(None) };
    let info = header_info(headers);
    let info = info.as_ref().map(|(k, n)| (k.as_str(), n.as_str()));
    Ok(Some(st.store.ensure_device(&auth.0.member_id, &id, info)?))
}

/// A Device the route names: the request's own (made on first use) or one the Member has.
fn named_device(st: &AppState, auth: &Auth, headers: &HeaderMap, id: &str) -> Result<DeviceView, ApiError> {
    let own = match &auth.0.via {
        AuthVia::Session { .. } => Some(WEB_DEVICE.to_string()),
        AuthVia::Token { .. } => header_device(headers),
    };
    if own.as_deref() == Some(id) || id == WEB_DEVICE {
        if let Some(d) = request_device(st, auth, headers)?.filter(|d| d.id == id) {
            return Ok(d);
        }
        return Ok(st.store.ensure_device(&auth.0.member_id, id, None)?);
    }
    if !settings::is_device_id(id) {
        return Err(annotary_store::StoreError::BadDevice(id.to_string()).into());
    }
    st.store.device(&auth.0.member_id, id)?.ok_or_else(|| ApiError::not_found(format!("no Device {id}")))
}

fn vaults_json(st: &AppState, auth: &Auth) -> Result<Value, ApiError> {
    Ok(readable_vaults(st, auth)?.iter().map(|v| json!({ "id": v.id, "name": v.name })).collect())
}

fn resolved_json(r: &annotary_store::Resolved) -> Value {
    json!({ "key": r.def.key, "kind": r.def.kind.as_str(), "value": r.value, "source": r.source })
}

/// `GET /api/v1/me/settings/catalog`.
pub async fn catalog(_auth: Auth) -> Json<Value> {
    Json(settings::catalog_json())
}

#[derive(Debug, Deserialize)]
pub struct DeviceQuery {
    pub device: Option<String>,
}

/// `GET /api/v1/me/settings?device=ID`.
pub async fn get(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Query(q): Query<DeviceQuery>,
) -> Result<Json<Value>, ApiError> {
    let device = match q.device.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(id) => Some(named_device(&st, &auth, &headers, id)?),
        None => request_device(&st, &auth, &headers)?,
    };
    let rows = st.store.resolved_settings(&auth.0.member_id, device.as_ref().map(|d| d.id.as_str()))?;
    Ok(Json(json!({
        "device": device.as_ref().map(DeviceView::to_json),
        "settings": rows.iter().map(resolved_json).collect::<Vec<_>>(),
        "vaults": vaults_json(&st, &auth)?,
    })))
}

/// `GET /api/v1/me/settings/matrix`.
pub async fn matrix(State(st): State<AppState>, auth: Auth, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let current = request_device(&st, &auth, &headers)?;
    let (devices, entries) = st.store.settings_matrix(&auth.0.member_id)?;
    let settings: serde_json::Map<String, Value> =
        entries.iter().map(|e| (e.def.key.to_string(), e.to_json())).collect();
    Ok(Json(json!({
        "current": current.map(|d| d.id),
        "devices": devices.iter().map(DeviceView::to_json).collect::<Vec<_>>(),
        "settings": settings,
    })))
}

#[derive(Debug, Deserialize)]
pub struct PutBody {
    pub key: String,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub devices: Option<Vec<String>>,
    #[serde(default)]
    pub stamp: Option<String>,
}

fn parse_scope(key: &str, scope: Option<&str>, devices: Option<Vec<String>>) -> Result<Scope, ApiError> {
    let def = settings::setting(key).ok_or_else(|| {
        ApiError::from(annotary_store::StoreError::Setting(settings::Refusal::UnknownSetting(key.into())))
    })?;
    match scope.map(str::trim) {
        None | Some("") if def.kind == SettingKind::User => Ok(Scope::User),
        None | Some("") => Err(ApiError::invalid(format!(
            "{key} is a device setting: say scope all (every Device) or devices (+ devices[])"
        ))),
        Some("user") => Ok(Scope::User),
        Some("all") => Ok(Scope::All),
        Some("devices" | "device") => {
            let ids = devices.unwrap_or_default();
            if ids.is_empty() {
                return Err(ApiError::invalid("scope devices needs devices[] (or device=ID)"));
            }
            for id in &ids {
                if !settings::is_device_id(id) {
                    return Err(annotary_store::StoreError::BadDevice(id.clone()).into());
                }
            }
            Ok(Scope::Devices(ids))
        }
        Some(other) => Err(ApiError::invalid(format!("scope {other:?}: use user, all or devices"))),
    }
}

/// What a write answers: whether it changed anything, its stamp, the setting resolved for the
/// request's Device and (for a device setting) its matrix entry.
fn write_answer(
    st: &AppState,
    auth: &Auth,
    headers: &HeaderMap,
    key: &str,
    out: PutOutcome,
) -> Result<Json<Value>, ApiError> {
    let device = request_device(st, auth, headers)?;
    let r = st.store.resolved_setting(&auth.0.member_id, key, device.as_ref().map(|d| d.id.as_str()))?;
    let mut v = json!({
        "changed": out.changed, "stamp": out.stamp, "key": key, "value": r.value, "source": r.source,
    });
    if let Some(e) = st.store.matrix_entry(&auth.0.member_id, key)? {
        v["matrix"] = e.to_json();
    }
    Ok(Json(v))
}

/// Make sure a Device a write names exists: the request's own is made on first use.
fn ensure_named(st: &AppState, auth: &Auth, headers: &HeaderMap, scope: &Scope) -> Result<(), ApiError> {
    if let Scope::Devices(ids) = scope {
        for id in ids {
            named_device(st, auth, headers, id)?;
        }
    }
    Ok(())
}

/// `PUT /api/v1/me/settings`.
pub async fn put(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    body: Result<Json<PutBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body?;
    let scope = parse_scope(&body.key, body.scope.as_deref(), body.devices)?;
    ensure_named(&st, &auth, &headers, &scope)?;
    let put = PutSetting { key: body.key.clone(), value: body.value, scope, stamp: body.stamp };
    let out = st.store.put_setting(&auth.0.member_id, &put)?;
    write_answer(&st, &auth, &headers, &body.key, out)
}

#[derive(Debug, Deserialize)]
pub struct ResetQuery {
    pub scope: Option<String>,
    pub device: Option<String>,
}

/// `DELETE /api/v1/me/settings/{key}?scope=user|all|device&device=ID`.
pub async fn reset(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(key): Path<String>,
    Query(q): Query<ResetQuery>,
) -> Result<Json<Value>, ApiError> {
    let scope = parse_scope(&key, q.scope.as_deref(), q.device.map(|d| vec![d]))?;
    ensure_named(&st, &auth, &headers, &scope)?;
    let out = st.store.reset_setting(&auth.0.member_id, &key, &scope)?;
    write_answer(&st, &auth, &headers, &key, out)
}

#[derive(Debug, Deserialize)]
pub struct DevicesQuery {
    pub all: Option<String>,
}

/// `GET /api/v1/me/devices`.
pub async fn devices(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Query(q): Query<DevicesQuery>,
) -> Result<Json<Value>, ApiError> {
    let current = request_device(&st, &auth, &headers)?;
    let all = q.all.as_deref().is_some_and(|v| v == "1" || v == "true");
    let list = st.store.list_devices(&auth.0.member_id, all)?;
    Ok(Json(json!({
        "current": current.map(|d| d.id),
        "devices": list.iter().map(DeviceView::to_json).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct NameBody {
    pub name: String,
}

/// `PATCH /api/v1/me/devices/{id} {name}`.
pub async fn rename_device(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Result<Json<NameBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body?;
    if id != WEB_DEVICE && !settings::is_device_id(&id) {
        return Err(annotary_store::StoreError::BadDevice(id).into());
    }
    let d = st.store.rename_device(&auth.0.member_id, &id, &body.name)?;
    Ok(Json(d.to_json()))
}

/// `POST /api/v1/me/devices/{id}/forget`.
pub async fn forget_device(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if id != WEB_DEVICE && !settings::is_device_id(&id) {
        return Err(annotary_store::StoreError::BadDevice(id).into());
    }
    let (forgotten, tokens, sessions) = st.store.forget_device(&auth.0.member_id, &id)?;
    if forgotten {
        tracing::info!("@{} forgot a Device ({tokens} Tokens revoked, {sessions} sessions ended)", auth.0.handle);
    }
    let d = st.store.device(&auth.0.member_id, &id)?.ok_or_else(|| ApiError::not_found(format!("no Device {id}")))?;
    Ok(Json(json!({
        "device": d.to_json(), "forgotten": forgotten, "tokens_revoked": tokens, "sessions_ended": sessions,
    })))
}

#[derive(Debug, Deserialize)]
pub struct CopyBody {
    pub from: String,
}

/// `POST /api/v1/me/devices/{id}/copy-from {from}` (`{id}` = `all`: All devices).
pub async fn copy_from(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Result<Json<CopyBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body?;
    let from = named_device(&st, &auth, &headers, body.from.trim())?;
    let to = if id == "all" { None } else { Some(named_device(&st, &auth, &headers, &id)?.id) };
    let n = st.store.copy_device_settings(&auth.0.member_id, &from.id, to.as_deref())?;
    let (devices, entries) = st.store.settings_matrix(&auth.0.member_id)?;
    let settings: serde_json::Map<String, Value> =
        entries.iter().map(|e| (e.def.key.to_string(), e.to_json())).collect();
    Ok(Json(json!({
        "copied": n, "from": from.id, "to": to.unwrap_or_else(|| "all".into()),
        "devices": devices.iter().map(DeviceView::to_json).collect::<Vec<_>>(),
        "settings": settings,
    })))
}

/// `GET /api/v1/me/devices/{id}/sync`.
pub async fn sync(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let d = named_device(&st, &auth, &headers, &id)?;
    if d.forgotten_at.is_some() {
        return Err(annotary_store::StoreError::DeviceForgotten(id).into());
    }
    let mut settings = serde_json::Map::new();
    for (def, own, all) in st.store.device_sync(&auth.0.member_id, &d.id)? {
        let mut e = serde_json::Map::new();
        if let Some(o) = own {
            e.insert("own".into(), o.to_json());
        }
        if let Some(a) = all {
            e.insert("all".into(), a.to_json());
        }
        settings.insert(def.key.to_string(), Value::Object(e));
    }
    Ok(Json(json!({ "device": d.to_json(), "server_time_ms": settings::wall_ms(), "settings": settings })))
}

/// `POST /api/v1/me/rename {name}`: the caller's own display name (same rules as the owner route).
pub async fn rename_self(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<NameBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body?;
    let m = st.store.rename_member(&auth.0.member_id, &auth.0.member_id, &body.name)?;
    Ok(Json(member_json(&m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decodes_a_device_name() {
        assert_eq!(percent_decode("iPad Pro 13%E2%80%B3"), "iPad Pro 13″");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
