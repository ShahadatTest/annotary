//! annotary store: SQLite (WAL) is the truth.
//!
//! One `Store` per `store/` directory. Opening takes `store/LOCK` (advisory flock) so two servers
//! never write the same data, runs forward-only migrations (`PRAGMA user_version`), and mirrors
//! the schema version + Tenant id into `store/meta.json`.
//!
//! Every mutation appends a row to `events` in the same transaction (Tenant-wide strictly
//! monotonic `seq`); see [`Store`] for the API (`store.rs`: setup, members, tokens, vaults,
//! events; `docs.rs`: Documents, Versions, blobs, purge/GC; `threads.rs`: Threads, Comments,
//! mentions, the body cap; `actions.rs`: declarations + the Action log; `access.rs`: rename/role/
//! Grants, Tokens with expiry, the local password + `login`, browser sessions; `events.rs`: the
//! events log as a read surface — page query, commit hook, ticket 08; `housekeeping.rs`: the
//! auto-purge setting, auto-purge and derived-blob cleanup behind the hourly ticker, ticket 13;
//! `backup.rs`: the consistent `store/` tarball behind `export`; `vaults.rs`: Vault rename /
//! delete + Document counts, ticket 12; `links.rs`: the `public_url` / `link_base` settings and
//! the link / pairing helpers, ticket 18; `devices.rs` + `settings.rs`: Devices and Member
//! settings, CMD-23) and `migrations.rs` for the schema.
//! Blobs (Markdown, Assets, derived files) are content-addressed files under `store/blobs`
//! ([`BlobStore`]).

pub mod access;
pub mod actions;
// ---- ticket 07 ----
pub mod anchors;
// ---- end ticket 07 ----
pub mod assets;
// ---- ticket 13 ----
pub mod backup;
// ---- end ticket 13 ----
pub mod blobs;
pub mod caps;
pub mod concurrent;
// CMD-23: Devices and the stored Member settings; `settings` is the catalog, stamps and colors.
pub mod devices;
pub mod docs;
pub mod error;
// ---- ticket 08 ----
pub mod events;
// ---- end ticket 08 ----
// ---- ticket 13 ----
pub mod housekeeping;
// ---- end ticket 13 ----
pub mod ids;
// ---- ticket 18 ----
pub mod links;
// ---- end ticket 18 ----
pub mod lock;
pub mod manifest;
pub mod meta;
pub mod migrations;
pub mod model;
// ---- ticket 09 ----
pub mod render;
// ---- end ticket 09 ----
// CMD-1: the server's rotating consistent snapshots under DATA_DIR/snapshots/.
pub mod snapshots;
// CMD-23
pub mod settings;
mod store;
pub mod threads;
pub mod time;
pub mod token;
// ---- ticket 12 ----
pub mod vaults;
// ---- end ticket 12 ----

pub use access::{NewSession, SESSION_PREFIX, SESSION_TTL_DAYS, TokenRow};
pub use actions::{ActionDecl, ActionView, validate_action_id, validate_declaration};
pub use anchors::{Reanchored, SubmittedPlacement, compute_placements, parse_placements};
pub use assets::AssetInfo;
// ---- ticket 13 ----
pub use backup::{ExportReport, export_tar, snapshot_db};
pub use housekeeping::{AUTO_PURGE_KEY, AUTO_PURGE_MAX_DAYS, DerivedCleanup};
pub use snapshots::{SNAPSHOTS_DIR, SnapshotFile, SnapshotOutcome, list_snapshots, take_snapshot};
// ---- end ticket 13 ----
pub use blobs::BlobStore;
pub use concurrent::{HeadChanges, HeadMovedError, HeadMovedReason, check_head_changes, handles_word, versions_word};
pub use devices::{
    DeviceTouch, DeviceView, MAX_DEVICES, MatrixEntry, PutOutcome, PutSetting, Resolved, Scope, StoredValue,
};
pub use docs::{PurgeOutcome, identity_of, manifest_pairs, version_blob_refs};
pub use error::{Result, StoreError};
pub use events::{CommentBrief, CommitHook, EventQuery, EventRow, is_member_event};
// ---- ticket 18 ----
pub use links::{
    LINK_BASE_KEY, PUBLIC_URL_KEY, authority, doc_link, normalize_base_url, pair_profile, pair_url, percent_encode,
};
// ---- end ticket 18 ----
pub use lock::StoreLock;
pub use meta::Meta;
pub use migrations::SCHEMA_VERSION;
pub use model::*;
pub use store::{SetupInput, SetupOutcome, Store, hash_password, verify_password};
// ---- ticket 12 ----
pub use vaults::{ProfileVault, VaultCount, validate_vault_name};
// ---- end ticket 12 ----
pub use threads::{
    BODY_CAP, CommentView, MentionHit, NewThread, Resolution, ThreadFilter, ThreadSort, ThreadStatusFilter,
    ThreadSubject, ThreadView, Tombstone, comment_id, mention_candidates, parse_comment_id, parse_thread_id, thread_id,
};

/// Name of the SQLite file inside `store/`.
pub const DB_FILE: &str = "annotary.sqlite";
/// Name of the advisory lock file inside `store/`.
pub const LOCK_FILE: &str = "LOCK";
/// Name of the metadata file inside `store/`.
pub const META_FILE: &str = "meta.json";
/// Name of the first Vault created by first-run setup.
pub const FIRST_VAULT: &str = "notes";
