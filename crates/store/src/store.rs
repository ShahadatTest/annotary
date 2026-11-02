//! The `Store`: one SQLite connection behind a mutex (single writer), transactions that append an
//! event for every mutation, and the store directory (`LOCK`, `meta.json`).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::blobs::BlobStore;
use crate::error::{Result, StoreError};
use crate::lock::StoreLock;
use crate::meta::Meta;
use crate::model::*;
use crate::token::NewToken;
use crate::{ids, migrations, time, token};

/// Answers to first-run setup.
#[derive(Clone, Debug)]
pub struct SetupInput {
    pub owner_handle: String,
    pub owner_name: String,
    /// Optional local password (argon2id at rest).
    pub password: Option<String>,
}

/// What first-run setup created.
#[derive(Clone, Debug)]
pub struct SetupOutcome {
    pub tenant_id: String,
    pub owner: MemberView,
    pub vault: Vault,
    /// The owner's first Token, plain text, shown once.
    pub owner_token: NewToken,
}

/// The store. See the crate docs.
pub struct Store {
    conn: Mutex<Connection>,
    dir: PathBuf,
    pub(crate) blobs: BlobStore,
    _lock: StoreLock,
    // ---- ticket 07 ----
    /// Recompute every submitted placement on push and refuse a commit that disagrees (on by
    /// default — the local server; a hosted one may turn it off and trust the edge).
    pub(crate) verify_edge: std::sync::atomic::AtomicBool,
    // ---- end ticket 07 ----
    // ---- ticket 08 ----
    /// Run after every committed transaction (`events.rs`): the server wakes parked long-polls.
    pub(crate) commit_hook: Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync + 'static>>>,
    // ---- end ticket 08 ----
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("dir", &self.dir).finish()
    }
}

impl Store {
    /// Open (or create) the store in `dir`: take `LOCK`, open SQLite in WAL mode, run migrations,
    /// write `meta.json`.
    pub fn open(dir: &Path) -> Result<Store> {
        std::fs::create_dir_all(dir)?;
        let lock = StoreLock::acquire(dir)?;
        let mut conn = Connection::open(dir.join(crate::DB_FILE))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations::migrate(&mut conn)?;
        let blobs = BlobStore::new(dir);
        std::fs::create_dir_all(blobs.root())?;
        let store = Store {
            conn: Mutex::new(conn),
            dir: dir.to_path_buf(),
            blobs,
            _lock: lock,
            verify_edge: std::sync::atomic::AtomicBool::new(true),
            commit_hook: Mutex::new(None),
        };
        store.write_meta()?;
        Ok(store)
    }

    // ---- ticket 07 / 17 ----
    /// Whether the server re-runs what the edge computed for a commit and refuses a mismatch
    /// (default `true`): the Thread `placements` (ticket 07) and the claim that the pushed file
    /// contains the head's changes (`contains_head`, ticket 17).
    pub fn verify_edge(&self) -> bool {
        self.verify_edge.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Turn edge verification on or off (`serve --verify-edge`, alias `--verify-placements`).
    pub fn set_verify_edge(&self, on: bool) {
        self.verify_edge.store(on, std::sync::atomic::Ordering::Relaxed);
    }
    // ---- end ticket 07 / 17 ----

    /// The `store/` directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` inside a write transaction (`BEGIN IMMEDIATE`). Rolls back on `Err`. After a
    /// commit the commit hook runs (ticket 08: the server wakes its parked long-polls) — outside
    /// the connection mutex.
    pub(crate) fn tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let out = {
            let mut conn = self.conn();
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let out = f(&tx)?;
            tx.commit()?;
            out
        };
        self.fire_commit_hook();
        Ok(out)
    }

    fn write_meta(&self) -> Result<()> {
        let created_at = match Meta::read(&self.dir)? {
            Some(m) => m.created_at,
            None => time::now(),
        };
        Meta { schema_version: self.schema_version()?, tenant_id: self.tenant_id()?, created_at }.write(&self.dir)
    }

    /// `PRAGMA user_version` of the open store.
    pub fn schema_version(&self) -> Result<u32> {
        migrations::current_version(&self.conn())
    }

    /// The Tenant id, once first-run setup has happened.
    pub fn tenant_id(&self) -> Result<Option<String>> {
        self.setting("tenant_id")
    }

    /// True once first-run setup has created the Tenant.
    pub fn is_set_up(&self) -> Result<bool> {
        Ok(self.tenant_id()?.is_some())
    }

    /// Read a setting.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    // ---------------------------------------------------------------- first-run setup

    /// First-run setup: create the Tenant, the owner Member + Membership (role owner, `*` grant),
    /// the first Vault `notes`, and the owner's first Token. One transaction, four events.
    pub fn setup(&self, input: SetupInput) -> Result<SetupOutcome> {
        if self.is_set_up()? {
            return Err(StoreError::AlreadySetUp);
        }
        validate_handle(&input.owner_handle)?;
        let display_name = clean_display_name(&input.owner_name, &input.owner_handle);
        let password_hash = match input.password.as_deref().filter(|p| !p.is_empty()) {
            Some(pw) => Some(hash_password(pw)?),
            None => None,
        };
        let outcome = self.tx(|tx| {
            let now = time::now();
            let tenant_id = ids::ulid();
            tx.execute("INSERT INTO settings(key, value) VALUES ('tenant_id', ?1)", [&tenant_id])?;
            tx.execute("INSERT INTO settings(key, value) VALUES ('created_at', ?1)", [&now])?;
            append_event(tx, None, "tenant.created", None, serde_json::json!({ "tenant_id": tenant_id }))?;

            let owner_id = ids::ulid();
            tx.execute(
                "INSERT INTO members(id, display_name, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![owner_id, display_name, password_hash, now],
            )?;
            let grants = vec![Grant::all()];
            tx.execute(
                "INSERT INTO memberships(member_id, handle, role, grants, created_at) VALUES (?1, ?2, 'owner', ?3, ?4)",
                params![owner_id, input.owner_handle, serde_json::to_string(&grants)?, now],
            )?;
            append_event(
                tx,
                Some(&owner_id),
                "member.added",
                None,
                serde_json::json!({
                    "member": owner_id, "handle": input.owner_handle, "name": display_name,
                    "role": "owner", "grants": grants,
                }),
            )?;

            let vault = Vault {
                id: ids::ulid(),
                name: crate::FIRST_VAULT.to_string(),
                default_actions: default_vault_actions(),
                created_at: now.clone(),
            };
            tx.execute(
                "INSERT INTO vaults(id, name, default_actions, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![vault.id, vault.name, serde_json::to_string(&vault.default_actions)?, now],
            )?;
            append_event(
                tx,
                Some(&owner_id),
                "vault.created",
                None,
                serde_json::json!({ "vault": vault.id, "name": vault.name }),
            )?;

            let owner_token = insert_token(tx, &owner_id, &owner_id, Some("first-run"), &now)?;
            let owner = load_member(tx, &owner_id)?.ok_or(StoreError::MemberNotFound)?;
            Ok(SetupOutcome { tenant_id, owner, vault, owner_token })
        })?;
        self.write_meta()?;
        Ok(outcome)
    }

    // ---------------------------------------------------------------- members

    /// `member add`: Member + Membership (role member, `*` grant with all four Rights) + Token,
    /// in one transaction with its events. Returns the Token plain text (shown once).
    pub fn add_member(&self, by: &str, handle: &str, name: Option<&str>) -> Result<(MemberView, NewToken)> {
        validate_handle(handle)?;
        let display_name = clean_display_name(name.unwrap_or(""), handle);
        self.tx(|tx| {
            if handle_exists(tx, handle)? {
                return Err(StoreError::HandleTaken(handle.to_string()));
            }
            let now = time::now();
            let id = ids::ulid();
            tx.execute(
                "INSERT INTO members(id, display_name, created_at) VALUES (?1, ?2, ?3)",
                params![id, display_name, now],
            )?;
            let grants = vec![Grant::all()];
            tx.execute(
                "INSERT INTO memberships(member_id, handle, role, grants, created_at) VALUES (?1, ?2, 'member', ?3, ?4)",
                params![id, handle, serde_json::to_string(&grants)?, now],
            )?;
            append_event(
                tx,
                Some(by),
                "member.added",
                None,
                serde_json::json!({
                    "member": id, "handle": handle, "name": display_name, "role": "member", "grants": grants,
                }),
            )?;
            let token = insert_token(tx, by, &id, Some("member add"), &now)?;
            let view = load_member(tx, &id)?.ok_or(StoreError::MemberNotFound)?;
            Ok((view, token))
        })
    }

    /// All Members (including disabled), oldest first.
    pub fn list_members(&self) -> Result<Vec<MemberView>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{MEMBER_SELECT} ORDER BY ms.created_at, ms.handle"))?;
        let rows = stmt.query_map([], row_to_member)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// One Member by handle.
    pub fn member_by_handle(&self, handle: &str) -> Result<Option<MemberView>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("{MEMBER_SELECT} WHERE ms.handle = ?1"), [handle], row_to_member).optional()?)
    }

    /// One Member by id.
    pub fn member_by_id(&self, id: &str) -> Result<Option<MemberView>> {
        let conn = self.conn();
        Ok(conn.query_row(&format!("{MEMBER_SELECT} WHERE m.id = ?1"), [id], row_to_member).optional()?)
    }

    /// Replace a Member's Grants (`member grant`, ticket 06). Appends `member.granted`.
    pub fn set_grants(&self, by: &str, member_id: &str, grants: Vec<Grant>) -> Result<MemberView> {
        self.tx(|tx| {
            let n = tx.execute(
                "UPDATE memberships SET grants = ?1 WHERE member_id = ?2",
                params![serde_json::to_string(&grants)?, member_id],
            )?;
            if n == 0 {
                return Err(StoreError::MemberNotFound);
            }
            append_event(
                tx,
                Some(by),
                "member.granted",
                None,
                serde_json::json!({ "member": member_id, "grants": grants }),
            )?;
            load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)
        })
    }

    /// Disable a Member (never delete): their Tokens and sessions stop working, history and
    /// attribution stay, the handle stays taken. The last owner cannot be disabled
    /// (`LastOwner`); disabling an already disabled Member changes nothing (`Ok(false)`).
    pub fn disable_member(&self, by: &str, member_id: &str) -> Result<bool> {
        self.tx(|tx| {
            let m = load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
            if m.is_disabled() {
                return Ok(false);
            }
            if m.role == Role::Owner && crate::access::active_owner_count(tx)? <= 1 {
                return Err(StoreError::LastOwner(m.handle));
            }
            let now = time::now();
            tx.execute("UPDATE members SET disabled_at = ?1 WHERE id = ?2", params![now, member_id])?;
            tx.execute("UPDATE memberships SET disabled_at = ?1 WHERE member_id = ?2", params![now, member_id])?;
            append_event(
                tx,
                Some(by),
                "member.disabled",
                None,
                serde_json::json!({ "member": member_id, "handle": m.handle }),
            )?;
            Ok(true)
        })
    }

    // ---------------------------------------------------------------- tokens

    /// Mint an additional Token for a Member.
    pub fn mint_token(&self, by: &str, member_id: &str, name: Option<&str>) -> Result<NewToken> {
        self.tx(|tx| {
            if load_member(tx, member_id)?.is_none() {
                return Err(StoreError::MemberNotFound);
            }
            let now = time::now();
            insert_token(tx, by, member_id, name, &now)
        })
    }

    /// Revoke a Token: it stops authenticating immediately. `Ok(false)` when it was already
    /// revoked (idempotent); `TokenNotFound` for an unknown id.
    pub fn revoke_token(&self, by: &str, token_id: &str) -> Result<bool> {
        self.tx(|tx| {
            let (member_id, revoked): (String, Option<String>) = tx
                .query_row("SELECT member_id, revoked_at FROM tokens WHERE id = ?1", [token_id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
                .ok_or(StoreError::TokenNotFound)?;
            if revoked.is_some() {
                return Ok(false);
            }
            let now = time::now();
            tx.execute("UPDATE tokens SET revoked_at = ?1 WHERE id = ?2", params![now, token_id])?;
            append_event(
                tx,
                Some(by),
                "token.revoked",
                None,
                serde_json::json!({ "token": token_id, "member": member_id }),
            )?;
            Ok(true)
        })
    }

    /// Tokens of a Member (never the plain text).
    pub fn tokens_for_member(&self, member_id: &str) -> Result<Vec<TokenInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, member_id, name, created_at, expires_at, last_used_at, revoked_at, hash
             FROM tokens WHERE member_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([member_id], |r| {
            Ok(TokenInfo {
                id: r.get(0)?,
                member_id: r.get(1)?,
                name: r.get(2)?,
                created_at: r.get(3)?,
                expires_at: r.get(4)?,
                last_used_at: r.get(5)?,
                revoked_at: r.get(6)?,
                hash: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Resolve a plain Token to its Member (`None` = unknown, revoked, expired, or a disabled
    /// Member/Membership). Stamps `last_used_at` on success.
    pub fn authenticate(&self, plain: &str) -> Result<Option<AuthMember>> {
        if !token::looks_like_token(plain) {
            return Ok(None);
        }
        let hash = token::hash(plain);
        let now = time::now();
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let found = tx
            .query_row(
                "SELECT t.id, t.expires_at, m.id, m.display_name, ms.handle, ms.role, ms.grants
                 FROM tokens t
                 JOIN members m ON m.id = t.member_id
                 JOIN memberships ms ON ms.member_id = m.id
                 WHERE t.hash = ?1 AND t.revoked_at IS NULL
                   AND m.disabled_at IS NULL AND ms.disabled_at IS NULL",
                [&hash],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((token_id, expires_at, member_id, display_name, handle, role, grants)) = found else {
            return Ok(None);
        };
        if let Some(exp) = expires_at
            && exp <= now
        {
            return Ok(None);
        }
        tx.execute("UPDATE tokens SET last_used_at = ?1 WHERE id = ?2", params![now, token_id])?;
        tx.commit()?;
        Ok(Some(AuthMember {
            member_id,
            handle,
            display_name,
            role: Role::parse(&role).unwrap_or(Role::Member),
            grants: serde_json::from_str(&grants)?,
            via: AuthVia::Token { id: token_id },
        }))
    }

    // ---------------------------------------------------------------- vaults

    /// Create a Vault with the default Actions (owner-only at the API; ticket 06's `vault create`).
    /// Appends `vault.created`. Names are unique.
    pub fn create_vault(&self, by: &str, name: &str) -> Result<Vault> {
        // ticket 12: the name rule is shared with rename (`vaults.rs`); a taken name is `VaultNameTaken`.
        let name = crate::vaults::validate_vault_name(name)?;
        self.tx(|tx| {
            let exists: bool =
                tx.query_row("SELECT 1 FROM vaults WHERE name = ?1", [name], |_| Ok(())).optional()?.is_some();
            if exists {
                return Err(StoreError::VaultNameTaken(name.to_string()));
            }
            let now = time::now();
            let vault = Vault {
                id: ids::ulid(),
                name: name.to_string(),
                default_actions: default_vault_actions(),
                created_at: now.clone(),
            };
            tx.execute(
                "INSERT INTO vaults(id, name, default_actions, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![vault.id, vault.name, serde_json::to_string(&vault.default_actions)?, now],
            )?;
            append_event(
                tx,
                Some(by),
                "vault.created",
                None,
                serde_json::json!({ "vault": vault.id, "name": vault.name }),
            )?;
            Ok(vault)
        })
    }

    /// All Vaults, oldest first.
    pub fn list_vaults(&self) -> Result<Vec<Vault>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, name, default_actions, created_at FROM vaults ORDER BY created_at, name")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, name, actions, created_at) = r?;
            out.push(Vault { id, name, default_actions: serde_json::from_str(&actions)?, created_at });
        }
        Ok(out)
    }

    // ---------------------------------------------------------------- events

    /// Events with `seq > since`, ascending.
    pub fn events_since(&self, since: i64) -> Result<Vec<Event>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT seq, at, by, type, doc_id, payload FROM events WHERE seq > ?1 ORDER BY seq")?;
        let rows = stmt.query_map([since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, at, by, kind, doc_id, payload) = r?;
            out.push(Event { seq, at, by, kind, doc_id, payload: serde_json::from_str(&payload)? });
        }
        Ok(out)
    }

    /// The latest `seq` (0 when there are no events yet).
    pub fn last_seq(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |r| r.get(0))?)
    }
}

pub(crate) const MEMBER_SELECT: &str = "SELECT m.id, ms.handle, m.display_name, ms.role, ms.grants, ms.created_at,
    m.disabled_at, m.password_hash IS NOT NULL
    FROM members m JOIN memberships ms ON ms.member_id = m.id";

pub(crate) fn row_to_member(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemberView> {
    let grants: String = r.get(4)?;
    let role: String = r.get(3)?;
    Ok(MemberView {
        id: r.get(0)?,
        handle: r.get(1)?,
        display_name: r.get(2)?,
        role: Role::parse(&role).unwrap_or(Role::Member),
        grants: serde_json::from_str(&grants).unwrap_or_default(),
        created_at: r.get(5)?,
        disabled_at: r.get(6)?,
        has_password: r.get(7)?,
    })
}

pub(crate) fn load_member(tx: &Transaction, id: &str) -> Result<Option<MemberView>> {
    Ok(tx.query_row(&format!("{MEMBER_SELECT} WHERE m.id = ?1"), [id], row_to_member).optional()?)
}

pub(crate) fn handle_exists(tx: &Transaction, handle: &str) -> Result<bool> {
    Ok(tx.query_row("SELECT 1 FROM memberships WHERE handle = ?1", [handle], |_| Ok(())).optional()?.is_some())
}

/// Insert a freshly minted Token for `member_id` (minted by `by`), appending its event.
/// Returns the plain text — the only time it exists outside the caller's hands.
pub(crate) fn insert_token(
    tx: &Transaction,
    by: &str,
    member_id: &str,
    name: Option<&str>,
    now: &str,
) -> Result<NewToken> {
    insert_token_expiring(tx, by, member_id, name, None, now)
}

/// `insert_token` with an optional `expires_at` (ticket 06: `token mint --expires`).
pub(crate) fn insert_token_expiring(
    tx: &Transaction,
    by: &str,
    member_id: &str,
    name: Option<&str>,
    expires_at: Option<&str>,
    now: &str,
) -> Result<NewToken> {
    let t = token::mint();
    tx.execute(
        "INSERT INTO tokens(id, member_id, hash, name, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![t.id, member_id, token::hash(&t.plain), name, now, expires_at],
    )?;
    append_event(
        tx,
        Some(by),
        "token.minted",
        None,
        serde_json::json!({ "token": t.id, "member": member_id, "name": name, "expires_at": expires_at }),
    )?;
    Ok(t)
}

/// Append one event inside the caller's transaction. Every mutation calls this before commit so
/// `seq` and the mutation are atomic. The failpoint sits between the mutation and this insert.
pub(crate) fn append_event(
    tx: &Transaction,
    by: Option<&str>,
    kind: &str,
    doc_id: Option<&str>,
    payload: serde_json::Value,
) -> Result<i64> {
    fail::fail_point!("store.before_event_insert", |_| Err(StoreError::Failpoint));
    tx.execute(
        "INSERT INTO events(at, by, type, doc_id, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![time::now(), by, kind, doc_id, serde_json::to_string(&payload)?],
    )?;
    Ok(tx.last_insert_rowid())
}

fn clean_display_name(name: &str, fallback: &str) -> String {
    let n = name.trim();
    if n.is_empty() { fallback.to_string() } else { n.to_string() }
}

/// argon2id PHC string for a local password.
pub fn hash_password(password: &str) -> Result<String> {
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
    let salt = SaltString::generate(&mut OsRng);
    argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| StoreError::Password(e.to_string()))
}

/// Verify a local password against its argon2id PHC string.
pub fn verify_password(password: &str, phc: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(phc)
        .map(|h| argon2::Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        (dir, store)
    }

    fn setup(store: &Store) -> SetupOutcome {
        store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap()
    }

    #[test]
    fn setup_creates_owner_vault_and_token() {
        let (_d, store) = fresh();
        assert!(!store.is_set_up().unwrap());
        let out = setup(&store);
        assert!(store.is_set_up().unwrap());
        assert_eq!(out.owner.handle, "rowan");
        assert_eq!(out.owner.role, Role::Owner);
        assert_eq!(out.owner.grants, vec![Grant::all()]);
        assert_eq!(out.vault.name, "notes");
        assert!(out.owner_token.plain.starts_with("ant_"));
        let auth = store.authenticate(&out.owner_token.plain).unwrap().unwrap();
        assert!(auth.is_owner());
        assert!(matches!(setup_again(&store), Err(StoreError::AlreadySetUp)));
        let meta = Meta::read(store.dir()).unwrap().unwrap();
        assert_eq!(meta.tenant_id, Some(out.tenant_id));
        assert_eq!(meta.schema_version, crate::SCHEMA_VERSION);
    }

    fn setup_again(store: &Store) -> Result<SetupOutcome> {
        store.setup(SetupInput { owner_handle: "x".into(), owner_name: "x".into(), password: None })
    }

    #[test]
    fn password_is_argon2id_and_verifies() {
        let (_d, store) = fresh();
        store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "".into(), password: Some("hunter2".into()) })
            .unwrap();
        let m = store.member_by_handle("rowan").unwrap().unwrap();
        assert!(m.has_password);
        assert_eq!(m.display_name, "rowan", "empty display name falls back to the handle");
        let phc = hash_password("hunter2").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_password("hunter2", &phc));
        assert!(!verify_password("hunter3", &phc));
    }

    #[test]
    fn add_member_is_one_step_and_handles_are_unique_forever() {
        let (_d, store) = fresh();
        let out = setup(&store);
        let (m, tok) = store.add_member(&out.owner.id, "scribe", Some("Scribe")).unwrap();
        assert_eq!(m.role, Role::Member);
        assert_eq!(m.grants, vec![Grant::all()]);
        assert_eq!(store.authenticate(&tok.plain).unwrap().unwrap().handle, "scribe");
        assert!(matches!(store.add_member(&out.owner.id, "scribe", None), Err(StoreError::HandleTaken(_))));
        store.disable_member(&out.owner.id, &m.id).unwrap();
        assert!(matches!(store.add_member(&out.owner.id, "scribe", None), Err(StoreError::HandleTaken(_))));
        assert!(store.authenticate(&tok.plain).unwrap().is_none(), "disabled member's token stops working");
        assert!(matches!(store.add_member(&out.owner.id, "Bad_Handle", None), Err(StoreError::InvalidHandle { .. })));
        assert_eq!(store.list_members().unwrap().len(), 2, "disabled members are kept");
    }

    #[test]
    fn tokens_are_hashed_at_rest_and_revocable() {
        let (_d, store) = fresh();
        let out = setup(&store);
        let toks = store.tokens_for_member(&out.owner.id).unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].hash, token::hash(&out.owner_token.plain));
        assert!(toks[0].last_used_at.is_none());
        assert!(store.authenticate(&out.owner_token.plain).unwrap().is_some());
        assert!(store.tokens_for_member(&out.owner.id).unwrap()[0].last_used_at.is_some());
        store.revoke_token(&out.owner.id, &toks[0].id).unwrap();
        assert!(store.authenticate(&out.owner_token.plain).unwrap().is_none());
        assert!(store.authenticate("ant_nope").unwrap().is_none());
        assert!(store.authenticate("").unwrap().is_none());
    }

    #[test]
    fn every_mutation_appends_an_event_with_increasing_seq() {
        let (_d, store) = fresh();
        assert_eq!(store.last_seq().unwrap(), 0);
        let out = setup(&store);
        let after_setup = store.last_seq().unwrap();
        assert_eq!(after_setup, 4, "tenant, owner member, vault, owner token");
        store.add_member(&out.owner.id, "linter", None).unwrap();
        assert_eq!(store.last_seq().unwrap(), 6, "member + token");
        let events = store.events_since(0).unwrap();
        let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(events[0].kind, "tenant.created");
        assert_eq!(events[4].kind, "member.added");
        assert_eq!(store.events_since(4).unwrap().len(), 2);
    }

    #[test]
    fn reopen_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store");
        let tenant = {
            let store = Store::open(&path).unwrap();
            setup(&store).tenant_id
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.tenant_id().unwrap(), Some(tenant));
        assert_eq!(store.list_vaults().unwrap()[0].name, "notes");
    }
}
