use std::path::PathBuf;

/// Errors from the store. The server maps these onto HTTP statuses; the CLI onto exit codes.
#[derive(thiserror::Error, Debug)]
pub enum StoreError {
    #[error("store {path} is locked by another annotary serve{}", pid.map(|p| format!(" (pid {p})")).unwrap_or_default())]
    Locked { path: PathBuf, pid: Option<u32> },
    #[error("store schema is v{found} but this annotary only knows up to v{supported}")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("store is not set up yet (no Tenant)")]
    NotSetUp,
    #[error("store is already set up")]
    AlreadySetUp,
    #[error("handle @{0} is already taken (handles are unique per Tenant forever)")]
    HandleTaken(String),
    #[error("invalid handle '{handle}': {reason}")]
    InvalidHandle { handle: String, reason: &'static str },
    #[error("invalid {what}: {reason}")]
    Invalid { what: &'static str, reason: String },
    #[error("member not found")]
    MemberNotFound,
    #[error("token not found")]
    TokenNotFound,
    #[error("invalid Name {name:?}: {reason}")]
    InvalidName { name: String, reason: &'static str },
    #[error("no Vault {0}")]
    VaultNotFound(String),
    #[error("no Document {0}")]
    DocumentNotFound(String),
    #[error("Document {doc} has no v{number}")]
    VersionNotFound { doc: String, number: u32 },
    #[error("blob {0} is not in the store — upload it first")]
    BlobMissing(String),
    #[error("blob content hashes to {actual}, not {expected}")]
    BlobHashMismatch { expected: String, actual: String },
    /// Ticket 17: a push onto a moved head that does not (verifiably) contain the head's changes.
    #[error("{0}")]
    HeadMoved(crate::concurrent::HeadMovedError),
    #[error("Document {0} is deleted")]
    DocumentDeleted(String),
    #[error("Document {0} is not deleted")]
    DocumentNotDeleted(String),
    // ---- ticket 04: Threads / Comments ----
    #[error("no Thread {thread} in Document {doc}")]
    ThreadNotFound { doc: String, thread: String },
    #[error("no Comment {comment} in Document {doc}")]
    CommentNotFound { doc: String, comment: String },
    #[error("Comment {0} is deleted")]
    CommentDeleted(String),
    #[error("only the author (@{author}) or an owner may {what} {comment}")]
    NotAuthor { comment: String, author: String, what: &'static str },
    #[error("head is v{head}, not v{expected}")]
    NotHead { expected: u32, head: u32 },
    #[error("{0}")]
    InvalidRange(String),
    #[error("range {range} is not on head v{head}: {reason}")]
    RangeNotOnHead { range: String, head: u32, reason: String },
    #[error("Comment bodies in this Document total {current} of {cap} characters — {adding} more would exceed the cap")]
    BodyCap { cap: usize, current: usize, adding: usize },
    // ---- end ticket 04 ----
    // ---- ticket 07: re-anchoring ----
    #[error("invalid placements: {0}")]
    InvalidPlacements(String),
    #[error("no Asset {reference} on head v{head}{}", if available.is_empty() { " (the Version references no Assets)".to_string() } else { format!(" — referenced: {}", available.join(", ")) })]
    AssetNotOnHead { reference: String, head: u32, available: Vec<String> },
    #[error("Asset {reference} is missing on head v{head} (no content to anchor to)")]
    AssetMissing { reference: String, head: u32 },
    // ---- end ticket 07 ----
    // ---- ticket 03: Assets ----
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("{what} is {} — the cap is {}", crate::caps::human_size(*size), crate::caps::human_size(*max))]
    TooLarge { what: String, size: u64, max: u64 },
    // ---- end ticket 03 ----
    // ---- ticket 05: Actions ----
    #[error("no Action {action} on this Document{}", if available.is_empty() { " (it declares no Actions)".to_string() } else { format!(" — available: {}", available.join(", ")) })]
    UndeclaredAction { action: String, available: Vec<String> },
    #[error("invalid Action declaration: {0}")]
    InvalidActions(String),
    // ---- end ticket 05 ----
    // ---- ticket 06: access model ----
    #[error("@{0} is the last owner — a Tenant always has one")]
    LastOwner(String),
    #[error("@{0} is disabled")]
    MemberDisabled(String),
    #[error("no local password is set")]
    PasswordNotSet,
    #[error("wrong password")]
    WrongPassword,
    #[error("invalid Grant: {0}")]
    InvalidGrant(String),
    // ---- end ticket 06 ----
    // ---- ticket 12: Vault rename / delete ----
    #[error("Vault {0} already exists")]
    VaultNameTaken(String),
    #[error("Vault {name} still holds {documents} Document{}{}", if *documents == 1 { "" } else { "s" }, if *deleted > 0 { format!(" ({deleted} deleted — purge them first)") } else { String::new() })]
    VaultNotEmpty { name: String, documents: u64, deleted: u64 },
    #[error("{0} is the last Vault — a Tenant always has one")]
    LastVault(String),
    // ---- end ticket 12 ----
    // ---- CMD-9: export is complete or fails ----
    /// `export_tar`: blobs the database snapshot references were not in the store when the
    /// export came to copy them (a purge or GC ran during the export, or they are missing).
    #[error("export incomplete: {} blob{} the database snapshot references {} not in the store", missing.len(), if missing.len() == 1 { "" } else { "s" }, if missing.len() == 1 { "is" } else { "are" })]
    ExportIncomplete { missing: Vec<String> },
    // ---- end CMD-9 ----
    // ---- CMD-23: Member settings and Devices ----
    #[error("{0}")]
    Setting(crate::settings::Refusal),
    #[error("Device id {0:?} is not `web` or 26 characters of Crockford base32")]
    BadDevice(String),
    #[error("no Device {0}")]
    DeviceNotFound(String),
    #[error("Device {0} was forgotten")]
    DeviceForgotten(String),
    #[error("you already have {0} Devices — forget one in Settings → Devices")]
    TooManyDevices(usize),
    #[error("Web stands for every browser: it can't be renamed or forgotten")]
    WebDevice,
    #[error("stamp {stamp} is more than five minutes ahead of the server's clock")]
    ClockSkew { stamp: String, server_ms: u64 },
    #[error("{key} already holds a change stamped {stored_stamp}, not older than {stamp}")]
    Stale { key: String, stamp: String, stored_value: Option<Box<serde_json::Value>>, stored_stamp: String },
    // ---- end CMD-23 ----
    #[error("failpoint triggered")]
    Failpoint,
    #[error("password hashing failed: {0}")]
    Password(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;
