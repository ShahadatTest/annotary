//! JSON API errors: `{error, message, hint}` with the HTTP status the CLI maps onto exit codes
//! (401 → 3, 403 → 4, 404 → 2, 409 → 5, 413 → 5, 400 → 1). Ticket 06: 409 `last_owner` /
//! `disabled` / `no_password` → 5, 401 `wrong_password` → 3, 400 `invalid_grant` → 1. Ticket 07:
//! 400 `invalid_placements` → 1, 409 `asset_not_on_head` / `asset_missing` → 5.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use annotary_store::StoreError;

#[derive(Clone, Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
    /// Extra top-level fields merged into the body (e.g. `candidates` on an ambiguous Name).
    /// Boxed to keep `Result<_, ApiError>` small.
    pub extra: Option<Box<serde_json::Value>>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError { status, code, message: message.into(), hint: None, extra: None }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> ApiError {
        self.hint = Some(hint.into());
        self
    }

    pub fn unauthorized() -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "invalid, revoked or expired token").hint(
            "check ANNOTARY_TOKEN or the profile's token; owners: `annotary login`; agents: ask an owner for a new token",
        )
    }

    pub fn missing_credential() -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "no credential")
            .hint("send `Authorization: Bearer <token>`; the CLI takes ANNOTARY_TOKEN or a profile")
    }

    // ---- ticket 06 ----
    /// A session cookie that no longer resolves (ended, expired, or its Member disabled).
    pub fn session_expired() -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "session expired or ended")
            .hint("sign in again at /login")
    }
    // ---- end ticket 06 ----

    pub fn owner_only(what: &str) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, "forbidden", format!("{what} is owner-only"))
            .hint("ask an owner (`annotary member list` shows who)")
    }

    pub fn missing_right(right: &str, vault: &str, handle: &str) -> ApiError {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!("missing right `{right}` on Vault {vault} (you are @{handle})"),
        )
        .hint(format!("ask an owner: annotary member grant {handle} {vault} {right}"))
    }

    pub fn not_found(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn conflict(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn invalid(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid", message)
    }

    pub fn internal(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message)
    }

    /// A size cap was exceeded (413 `too_large`; the CLI maps it to exit 5). `what` names the
    /// thing (`the Markdown`, `Asset img/x.png`, …); `size`/`max` in bytes go into the body too.
    pub fn too_large(what: String, size: u64, max: u64) -> ApiError {
        use annotary_store::caps::human_size;
        let mut e = ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too_large",
            format!("{what} is {} — the cap is {}", human_size(size), human_size(max)),
        )
        .hint("caps: Markdown 2 MB, each Asset 100 MB, all Assets of a Version 500 MB");
        e.extra = Some(Box::new(json!({ "what": what, "size": size, "max": max })));
        e
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.code, "message": self.message });
        if let Some(h) = self.hint {
            body["hint"] = json!(h);
        }
        if let Some(extra) = self.extra
            && let serde_json::Value::Object(extra) = *extra
        {
            for (k, v) in extra {
                body[k] = v;
            }
        }
        (self.status, Json(body)).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::HandleTaken(_) => ApiError::conflict(e.to_string()),
            StoreError::InvalidHandle { .. } | StoreError::Invalid { .. } => ApiError::invalid(e.to_string()),
            StoreError::InvalidName { .. } => ApiError::new(StatusCode::BAD_REQUEST, "invalid_name", e.to_string())
                .hint(
                    "Names are the filename stem: not empty, no path separators or control characters, no leading '.'",
                ),
            StoreError::MemberNotFound
            | StoreError::TokenNotFound
            | StoreError::VaultNotFound(_)
            | StoreError::DocumentNotFound(_)
            | StoreError::VersionNotFound { .. } => ApiError::not_found(e.to_string()),
            StoreError::BlobMissing(_) => ApiError::new(StatusCode::CONFLICT, "blob_missing", e.to_string())
                .hint("upload the blob (PUT /api/v1/blobs/{hash}) and commit again"),
            StoreError::BlobHashMismatch { .. } => ApiError::invalid(e.to_string()),
            // ---- ticket 17 ----
            StoreError::HeadMoved(ref hm) => {
                use annotary_store::HeadMovedReason;
                let hint = match hm.reason {
                    HeadMovedReason::Unchecked => {
                        "push with a annotary that checks your file contains them (it shows what is missing), or pass --discard-head-changes"
                    }
                    HeadMovedReason::Missing => "put them into your file and push again",
                    HeadMovedReason::MovedAgain { .. } => "push again",
                };
                let mut err = ApiError::new(StatusCode::CONFLICT, "head_moved", e.to_string()).hint(hint);
                err.extra = Some(Box::new(json!({ "base": hm.base, "head": hm.head, "by": hm.by })));
                err
            }
            // ---- end ticket 17 ----
            StoreError::DocumentDeleted(_) => ApiError::new(StatusCode::CONFLICT, "deleted", e.to_string())
                .hint("`annotary restore DOC` brings it back"),
            StoreError::DocumentNotDeleted(_) => ApiError::new(StatusCode::CONFLICT, "not_deleted", e.to_string()),
            // ---- ticket 04: Threads / Comments ----
            StoreError::ThreadNotFound { .. } | StoreError::CommentNotFound { .. } => {
                ApiError::not_found(e.to_string())
            }
            StoreError::CommentDeleted(_) => ApiError::new(StatusCode::CONFLICT, "deleted", e.to_string())
                .hint("a tombstoned Comment cannot be edited; reply instead"),
            StoreError::NotAuthor { .. } => ApiError::new(StatusCode::FORBIDDEN, "forbidden", e.to_string()),
            StoreError::NotHead { .. } => ApiError::new(StatusCode::CONFLICT, "not_head", e.to_string())
                .hint("pull and re-read before commenting"),
            StoreError::InvalidRange(_) => ApiError::invalid(e.to_string()),
            StoreError::RangeNotOnHead { .. } => {
                ApiError::new(StatusCode::CONFLICT, "range_not_on_head", e.to_string())
                    .hint("pull the head Version and quote a range that exists in it")
            }
            StoreError::BodyCap { .. } => ApiError::new(StatusCode::CONFLICT, "body_cap", e.to_string())
                .hint("tombstone or shorten Comments in this Document, or start a new Document"),
            // ---- end ticket 04 ----
            // ---- ticket 07: re-anchoring ----
            StoreError::InvalidPlacements(_) => {
                ApiError::new(StatusCode::BAD_REQUEST, "invalid_placements", e.to_string()).hint(
                    "placements are [{thread, status, range?, line?, exact?}] computed by the same annotary build the server runs — upgrade both, or push without placements and let the server place the Threads",
                )
            }
            StoreError::AssetNotOnHead { .. } => {
                ApiError::new(StatusCode::CONFLICT, "asset_not_on_head", e.to_string())
                    .hint("the ref is the Asset's path as the manifest lists it (`annotary pull DOC` shows assets[])")
            }
            StoreError::AssetMissing { .. } => ApiError::new(StatusCode::CONFLICT, "asset_missing", e.to_string())
                .hint("push the Version with the file present, then comment on it"),
            // ---- end ticket 07 ----
            // ---- ticket 03: Assets ----
            StoreError::InvalidManifest(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid_manifest", e.to_string())
                .hint("manifest entries are {ref, hash, size, type, missing?, via?, width?, height?, derivative?}"),
            StoreError::TooLarge { what, size, max } => ApiError::too_large(what, size, max),
            // ---- end ticket 03 ----
            // ---- ticket 05: Actions ----
            StoreError::UndeclaredAction { .. } => {
                ApiError::new(StatusCode::CONFLICT, "undeclared_action", e.to_string())
                    .hint("`annotary show DOC` lists the available Actions")
            }
            StoreError::InvalidActions(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid_actions", e.to_string())
                .hint(
                    "a declaration is a list of {id, label, intent} — kebab-case ids, intent positive|neutral|negative",
                ),
            // ---- end ticket 05 ----
            // ---- ticket 06: access model ----
            StoreError::LastOwner(_) => ApiError::new(StatusCode::CONFLICT, "last_owner", e.to_string())
                .hint("make another Member an owner first: annotary member role HANDLE owner"),
            StoreError::MemberDisabled(_) => ApiError::new(StatusCode::CONFLICT, "disabled", e.to_string())
                .hint("disabled Members keep their history but cannot be granted, promoted or given tokens"),
            StoreError::PasswordNotSet => ApiError::new(StatusCode::CONFLICT, "no_password", e.to_string()).hint(
                "an owner sets one (PUT /api/v1/settings/password, or the web Settings); until then use a token: \
                 ANNOTARY_TOKEN=… or `annotary member add HANDLE`",
            ),
            StoreError::WrongPassword => {
                ApiError::new(StatusCode::UNAUTHORIZED, "wrong_password", e.to_string()).hint("try again")
            }
            StoreError::InvalidGrant(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid_grant", e.to_string())
                .hint("a Grant is {vault | \"*\", rights[]} with rights from read, comment, action, write"),
            // ---- end ticket 06 ----
            // ---- ticket 12: Vault rename / delete ----
            StoreError::VaultNameTaken(_) => ApiError::conflict(e.to_string()),
            StoreError::VaultNotEmpty { .. } => ApiError::new(StatusCode::CONFLICT, "vault_not_empty", e.to_string())
                .hint("delete and purge its Documents first (`annotary purge DOC`), then delete the Vault"),
            StoreError::LastVault(_) => ApiError::new(StatusCode::CONFLICT, "last_vault", e.to_string())
                .hint("create another Vault first"),
            // ---- end ticket 12 ----
            // ---- CMD-23: Member settings and Devices ----
            StoreError::Setting(ref r) => {
                use annotary_store::settings::Refusal;
                match r {
                    Refusal::UnknownSetting(_) => ApiError::new(StatusCode::BAD_REQUEST, "unknown_setting", e.to_string())
                        .hint("GET /api/v1/me/settings/catalog lists them (`annotary settings list`)"),
                    Refusal::Invalid { .. } => ApiError::new(StatusCode::BAD_REQUEST, "invalid_setting", e.to_string()),
                    Refusal::LowContrast { ratio, .. } => {
                        let mut err = ApiError::new(StatusCode::BAD_REQUEST, "low_contrast", e.to_string())
                            .hint("pick a lighter color for light pages, a brighter one for dark pages");
                        err.extra = Some(Box::new(json!({ "ratio": ratio })));
                        err
                    }
                }
            }
            StoreError::BadDevice(_) => ApiError::new(StatusCode::BAD_REQUEST, "bad_device", e.to_string()),
            StoreError::DeviceNotFound(_) => ApiError::not_found(e.to_string()),
            StoreError::DeviceForgotten(_) => ApiError::new(StatusCode::CONFLICT, "device_forgotten", e.to_string())
                .hint("a forgotten Device id is never used again: make a new one"),
            StoreError::TooManyDevices(_) => {
                ApiError::new(StatusCode::CONFLICT, "too_many_devices", e.to_string())
            }
            StoreError::WebDevice => ApiError::new(StatusCode::CONFLICT, "web_device", e.to_string())
                .hint("signing out of browsers is what Sign out does"),
            StoreError::ClockSkew { server_ms, .. } => {
                let mut err = ApiError::new(StatusCode::CONFLICT, "clock_skew", e.to_string())
                    .hint("set this device's clock right; the change syncs once it is");
                err.extra = Some(Box::new(json!({ "server_time_ms": server_ms })));
                err
            }
            StoreError::Stale { ref stored_value, ref stored_stamp, .. } => {
                let mut err = ApiError::new(StatusCode::CONFLICT, "stale", e.to_string())
                    .hint("take the stored value (pull again)");
                err.extra = Some(Box::new(json!({ "stored": { "value": stored_value, "stamp": stored_stamp } })));
                err
            }
            // ---- end CMD-23 ----
            StoreError::NotSetUp => ApiError::internal("store is not set up"),
            other => {
                tracing::error!("store error: {other}");
                ApiError::internal(format!("store error: {other}"))
            }
        }
    }
}

impl From<axum::extract::rejection::JsonRejection> for ApiError {
    fn from(e: axum::extract::rejection::JsonRejection) -> Self {
        ApiError::invalid(format!("bad JSON body: {}", e.body_text()))
    }
}
