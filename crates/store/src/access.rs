//! Access model completion (ticket 06) on the [`Store`]: Member management (rename, role,
//! Grants; `disable_member` lives in `store.rs`), Tokens with an optional expiry, the local
//! password with `login`, and browser sessions.
//!
//! - **Rights** are `read / comment / action / write` per Vault; `read` is implied by any other.
//!   A Membership holds a list of Grants `{vault | "*", rights[]}`; the Grant that applies to a
//!   Vault is the Vault's own entry when there is one, else `*` ([`Grant::applicable`]) — so `*`
//!   covers Vaults created later and a Vault-specific Grant restricts (or widens) that one Vault.
//! - **Owner-only, never grantable:** Member/Membership management (add, rename, disable, role,
//!   Grants), Vault create, Vault default Actions, purge, the local password. `role ∈
//!   owner|member`; the last active owner cannot be demoted or disabled ([`StoreError::LastOwner`]).
//! - **Tokens** belong to the Member: shown once, sha256 at rest, optional `name`/`expires_at`,
//!   `last_used_at` stamped on use, revoke (idempotent). A disabled Member's Tokens stop working.
//! - **Local password** (argon2id PHC string in `members.password_hash`) — set/changed/removed
//!   by an owner for their own Member ([`Store::set_password`]); [`Store::login`] exchanges it
//!   for a fresh Token (`WrongPassword` / `PasswordNotSet`). While no active Member has a
//!   password, a browser on loopback is trusted as the founding owner ([`Store::password_set`],
//!   [`Store::founding_owner`]).
//! - **Sessions** (`sessions` table, schema v1): the cookie value is random base62, stored as
//!   its sha256 (`sessions.id`); [`Store::authenticate_session`] resolves it like a Token and
//!   stamps `last_seen_at`; [`Store::end_session`] is logout. Both `open_session` and
//!   `end_session` append events (`session.opened {member, via}` / `session.ended {member}`).

use rusqlite::{OptionalExtension, Transaction, params};

use crate::error::{Result, StoreError};
use crate::model::*;
use crate::store::{
    MEMBER_SELECT, Store, append_event, hash_password, insert_token_expiring, load_member, row_to_member,
    verify_password,
};
use crate::token::NewToken;
use crate::{ids, time, token};

/// How long a browser session stays valid without being renewed (days).
pub const SESSION_TTL_DAYS: i64 = 365;

/// Prefix of a session cookie value (so a pasted one is never mistaken for a Token).
pub const SESSION_PREFIX: &str = "cds_";

/// A Token row together with its Member's handle (for `token list`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRow {
    pub info: TokenInfo,
    pub handle: String,
    pub display_name: String,
    /// The Member (not just this Token) is disabled.
    pub member_disabled: bool,
}

impl TokenRow {
    /// `active` / `revoked` / `expired` as of `now`.
    pub fn status(&self, now: &str) -> &'static str {
        if self.info.revoked_at.is_some() {
            "revoked"
        } else if self.info.expires_at.as_deref().is_some_and(|e| e <= now) {
            "expired"
        } else {
            "active"
        }
    }
}

/// A freshly opened browser session: `plain` is the cookie value (never stored), `id` its
/// sha256 as stored.
#[derive(Clone, Debug)]
pub struct NewSession {
    pub id: String,
    pub plain: String,
    pub member_id: String,
    pub expires_at: String,
}

/// Number of active (not disabled) owner Memberships.
pub(crate) fn active_owner_count(tx: &Transaction) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT COUNT(*) FROM memberships ms JOIN members m ON m.id = ms.member_id
         WHERE ms.role = 'owner' AND ms.disabled_at IS NULL AND m.disabled_at IS NULL",
        [],
        |r| r.get(0),
    )?)
}

fn active_member(tx: &Transaction, member_id: &str) -> Result<MemberView> {
    let m = load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
    if m.is_disabled() {
        return Err(StoreError::MemberDisabled(m.handle));
    }
    Ok(m)
}

/// Order rights canonically (`read, comment, action, write`) and drop duplicates.
fn normalise_rights(rights: &[Right]) -> Vec<Right> {
    Right::ALL.iter().copied().filter(|r| rights.contains(r)).collect()
}

fn session_expiry(now: &str) -> String {
    let start = time::parse(now).unwrap_or_else(jiff::Timestamp::now);
    let end =
        start.checked_add(jiff::SignedDuration::from_hours(SESSION_TTL_DAYS * 24)).unwrap_or(jiff::Timestamp::MAX);
    time::format(end)
}

impl Store {
    // ---------------------------------------------------------------- members

    /// `member rename HANDLE NEW`: change the display name (the handle never changes). Appends
    /// `member.renamed {member, handle, name}`; a no-op rename appends nothing.
    pub fn rename_member(&self, by: &str, member_id: &str, name: &str) -> Result<MemberView> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Invalid { what: "display name", reason: "must not be empty".into() });
        }
        if name.chars().any(char::is_control) {
            return Err(StoreError::Invalid {
                what: "display name",
                reason: "must not contain control characters".into(),
            });
        }
        self.tx(|tx| {
            let m = load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
            if m.display_name == name {
                return Ok(m);
            }
            tx.execute("UPDATE members SET display_name = ?1 WHERE id = ?2", params![name, member_id])?;
            append_event(
                tx,
                Some(by),
                "member.renamed",
                None,
                serde_json::json!({ "member": member_id, "handle": m.handle, "name": name }),
            )?;
            load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)
        })
    }

    /// `member role HANDLE owner|member`. Demoting the last active owner is refused
    /// (`LastOwner`); a disabled Member's role cannot change (`MemberDisabled`). Returns the
    /// Member and whether anything changed (`member.role_set {member, handle, role}` only then).
    pub fn set_role(&self, by: &str, member_id: &str, role: Role) -> Result<(MemberView, bool)> {
        self.tx(|tx| {
            let m = active_member(tx, member_id)?;
            if m.role == role {
                return Ok((m, false));
            }
            if role == Role::Member && active_owner_count(tx)? <= 1 {
                return Err(StoreError::LastOwner(m.handle));
            }
            tx.execute("UPDATE memberships SET role = ?1 WHERE member_id = ?2", params![role.as_str(), member_id])?;
            append_event(
                tx,
                Some(by),
                "member.role_set",
                None,
                serde_json::json!({ "member": member_id, "handle": m.handle, "role": role.as_str() }),
            )?;
            Ok((load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)?, true))
        })
    }

    /// Validate and normalise a Grant list: every `vault` is `*` or an existing Vault name, each
    /// Vault appears once (a later entry replaces an earlier one), rights are de-duplicated in
    /// canonical order and never empty (an empty list means "no Grant on that Vault" — drop the
    /// entry instead).
    pub fn normalise_grants(&self, grants: Vec<Grant>) -> Result<Vec<Grant>> {
        let vaults: Vec<String> = self.list_vaults()?.into_iter().map(|v| v.name).collect();
        let mut out: Vec<Grant> = Vec::new();
        for g in grants {
            let vault = g.vault.trim().to_string();
            if vault != "*" && !vaults.contains(&vault) {
                return Err(StoreError::VaultNotFound(vault));
            }
            let rights = normalise_rights(&g.rights);
            if rights.is_empty() {
                return Err(StoreError::InvalidGrant(format!(
                    "rights for {vault} must not be empty (drop the Grant instead)"
                )));
            }
            out.retain(|x| x.vault != vault);
            out.push(Grant { vault, rights });
        }
        Ok(out)
    }

    /// `member grant HANDLE VAULT|* RIGHTS`: set the Member's Grant on one Vault (or `*`) to
    /// exactly `rights`, keeping the other entries; an empty `rights` removes that entry.
    /// `VaultNotFound` for an unknown Vault; a disabled Member cannot be granted anything.
    /// Appends `member.granted {member, handle, grants}` (the resulting list).
    pub fn grant(&self, by: &str, member_id: &str, vault: &str, rights: &[Right]) -> Result<MemberView> {
        let vault = vault.trim();
        if vault != "*" && self.vault_by_name(vault)?.is_none() {
            return Err(StoreError::VaultNotFound(vault.to_string()));
        }
        let rights = normalise_rights(rights);
        self.tx(|tx| {
            let m = active_member(tx, member_id)?;
            let mut grants: Vec<Grant> = m.grants.into_iter().filter(|g| g.vault != vault).collect();
            if !rights.is_empty() {
                grants.push(Grant { vault: vault.to_string(), rights });
            }
            grants.sort_by_key(|g| if g.vault == "*" { String::new() } else { g.vault.clone() });
            write_grants(tx, by, member_id, &m.handle, &grants)
        })
    }

    /// Replace a Member's whole Grant list (validated by [`Store::normalise_grants`]).
    pub fn replace_grants(&self, by: &str, member_id: &str, grants: Vec<Grant>) -> Result<MemberView> {
        let mut grants = self.normalise_grants(grants)?;
        grants.sort_by_key(|g| if g.vault == "*" { String::new() } else { g.vault.clone() });
        self.tx(|tx| {
            let m = active_member(tx, member_id)?;
            write_grants(tx, by, member_id, &m.handle, &grants)
        })
    }

    /// The Tenant's founding owner: the earliest active owner Membership (whom loopback trust
    /// signs a browser in as while no password is set).
    pub fn founding_owner(&self) -> Result<Option<MemberView>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                &format!(
                    "{MEMBER_SELECT} WHERE ms.role = 'owner' AND ms.disabled_at IS NULL AND m.disabled_at IS NULL
                     ORDER BY ms.created_at, ms.handle LIMIT 1"
                ),
                [],
                row_to_member,
            )
            .optional()?)
    }

    // ---------------------------------------------------------------- tokens

    /// `token mint`: a Token for `member_id` with an optional name and expiry (stored
    /// timestamp text). Refused for a disabled Member. Returns the plain text (shown once).
    pub fn mint_token_expiring(
        &self,
        by: &str,
        member_id: &str,
        name: Option<&str>,
        expires_at: Option<&str>,
    ) -> Result<NewToken> {
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        if let Some(e) = expires_at {
            let now = time::now();
            if time::parse(e).is_none() {
                return Err(StoreError::Invalid { what: "expiry", reason: format!("{e:?} is not a timestamp") });
            }
            if e <= now.as_str() {
                return Err(StoreError::Invalid { what: "expiry", reason: "must be in the future".into() });
            }
        }
        self.tx(|tx| {
            active_member(tx, member_id)?;
            let now = time::now();
            insert_token_expiring(tx, by, member_id, name, expires_at, &now)
        })
    }

    /// Tokens (never the plain text), oldest first: every Member's, or one Member's.
    pub fn list_tokens(&self, member_id: Option<&str>) -> Result<Vec<TokenRow>> {
        let conn = self.conn();
        let sql = "SELECT t.id, t.member_id, t.name, t.created_at, t.expires_at, t.last_used_at, t.revoked_at, t.hash,
                          ms.handle, m.display_name, m.disabled_at IS NOT NULL
                   FROM tokens t JOIN members m ON m.id = t.member_id JOIN memberships ms ON ms.member_id = m.id
                   WHERE (?1 IS NULL OR t.member_id = ?1)
                   ORDER BY t.created_at, t.id";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([member_id], row_to_token)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// One Token by id.
    pub fn token_row(&self, id: &str) -> Result<Option<TokenRow>> {
        let conn = self.conn();
        let sql = "SELECT t.id, t.member_id, t.name, t.created_at, t.expires_at, t.last_used_at, t.revoked_at, t.hash,
                          ms.handle, m.display_name, m.disabled_at IS NOT NULL
                   FROM tokens t JOIN members m ON m.id = t.member_id JOIN memberships ms ON ms.member_id = m.id
                   WHERE t.id = ?1";
        Ok(conn.query_row(sql, [id], row_to_token).optional()?)
    }

    // ---------------------------------------------------------------- password + login

    /// Set, change (`Some`) or remove (`None` / empty) the local password of `member_id`
    /// (argon2id at rest). Appends `password.set` / `password.removed {member}` — never the
    /// hash. Returns whether the Member has a password afterwards.
    pub fn set_password(&self, by: &str, member_id: &str, password: Option<&str>) -> Result<bool> {
        let password = password.filter(|p| !p.is_empty());
        let phc = match password {
            Some(p) => Some(hash_password(p)?),
            None => None,
        };
        self.tx(|tx| {
            let m = active_member(tx, member_id)?;
            tx.execute("UPDATE members SET password_hash = ?1 WHERE id = ?2", params![phc, member_id])?;
            let kind = if phc.is_some() { "password.set" } else { "password.removed" };
            if phc.is_some() || m.has_password {
                append_event(tx, Some(by), kind, None, serde_json::json!({ "member": member_id, "handle": m.handle }))?;
            }
            Ok(phc.is_some())
        })
    }

    /// True while some active Member holds a local password — then a browser must log in
    /// instead of being trusted as the owner on loopback.
    pub fn password_set(&self) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM members m JOIN memberships ms ON ms.member_id = m.id
                 WHERE m.password_hash IS NOT NULL AND m.disabled_at IS NULL AND ms.disabled_at IS NULL LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The active Members holding a password (owner Members, normally exactly one).
    fn password_holders(&self) -> Result<Vec<(MemberView, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT m.id, ms.handle, m.display_name, ms.role, ms.grants, ms.created_at, m.disabled_at,
                    m.password_hash IS NOT NULL, m.password_hash
             FROM members m JOIN memberships ms ON ms.member_id = m.id
             WHERE m.password_hash IS NOT NULL AND m.disabled_at IS NULL AND ms.disabled_at IS NULL
             ORDER BY ms.created_at, ms.handle",
        )?;
        let rows = stmt.query_map([], |r| {
            let m = row_to_member(r)?;
            Ok((m, r.get::<_, String>(8)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Whose password this is: the active Member holding a password that `password` verifies
    /// against. `PasswordNotSet` when no active Member has one; `WrongPassword` otherwise.
    pub fn check_password(&self, password: &str) -> Result<MemberView> {
        let holders = self.password_holders()?;
        if holders.is_empty() {
            return Err(StoreError::PasswordNotSet);
        }
        holders
            .into_iter()
            .find(|(_, phc)| verify_password(password, phc))
            .map(|(m, _)| m)
            .ok_or(StoreError::WrongPassword)
    }

    /// `login`: exchange the local password for a fresh Token (named `login`) of the Member it
    /// belongs to ([`Store::check_password`] decides whose).
    pub fn login(&self, password: &str) -> Result<(MemberView, NewToken)> {
        let m = self.check_password(password)?;
        let member_id = m.id.clone();
        let tok = self.tx(|tx| {
            let now = time::now();
            insert_token_expiring(tx, &member_id, &member_id, Some("login"), None, &now)
        })?;
        Ok((m, tok))
    }

    /// Verify `password` against `member_id`'s stored hash (for a browser login page that
    /// already knows whom it signs in). False when the Member has no password.
    pub fn verify_member_password(&self, member_id: &str, password: &str) -> Result<bool> {
        let phc: Option<String> = self
            .conn()
            .query_row("SELECT password_hash FROM members WHERE id = ?1 AND disabled_at IS NULL", [member_id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        Ok(phc.is_some_and(|p| verify_password(password, &p)))
    }

    // ---------------------------------------------------------------- sessions

    /// Open a browser session for `member_id` (`via` = `login` | `loopback` for the event).
    /// Returns the cookie value (plain, never stored) — the session lives `SESSION_TTL_DAYS`.
    pub fn open_session(&self, member_id: &str, via: &str) -> Result<NewSession> {
        let plain = format!("{SESSION_PREFIX}{}", ids::random_base62(ids::TOKEN_BODY_LEN));
        let id = token::hash(&plain);
        self.tx(|tx| {
            let m = active_member(tx, member_id)?;
            let now = time::now();
            let expires_at = session_expiry(&now);
            tx.execute(
                "INSERT INTO sessions(id, member_id, created_at, expires_at, last_seen_at, device_id)
                 VALUES (?1, ?2, ?3, ?4, ?3, 'web')",
                params![id, member_id, now, expires_at],
            )?;
            // CMD-23: every browser is the Member's Web Device.
            crate::devices::ensure_web(tx, member_id)?;
            append_event(
                tx,
                Some(member_id),
                "session.opened",
                None,
                serde_json::json!({ "member": member_id, "handle": m.handle, "via": via }),
            )?;
            Ok(NewSession { id: id.clone(), plain: plain.clone(), member_id: member_id.to_string(), expires_at })
        })
    }

    /// Resolve a session cookie value to its Member (`None` = unknown, expired, or a disabled
    /// Member/Membership). Stamps `last_seen_at`.
    pub fn authenticate_session(&self, plain: &str) -> Result<Option<AuthMember>> {
        if !plain.starts_with(SESSION_PREFIX) || plain.len() < SESSION_PREFIX.len() + 8 {
            return Ok(None);
        }
        let id = token::hash(plain);
        let now = time::now();
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let found = tx
            .query_row(
                "SELECT s.expires_at, m.id, m.display_name, ms.handle, ms.role, ms.grants
                 FROM sessions s
                 JOIN members m ON m.id = s.member_id
                 JOIN memberships ms ON ms.member_id = m.id
                 WHERE s.id = ?1 AND m.disabled_at IS NULL AND ms.disabled_at IS NULL",
                [&id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((expires_at, member_id, display_name, handle, role, grants)) = found else {
            return Ok(None);
        };
        if let Some(exp) = expires_at
            && exp <= now
        {
            return Ok(None);
        }
        tx.execute("UPDATE sessions SET last_seen_at = ?1 WHERE id = ?2", params![now, id])?;
        // CMD-23: a session is the Web Device; seen at most once a minute, outside the hook.
        crate::devices::touch_web(&tx, &member_id)?;
        tx.commit()?;
        Ok(Some(AuthMember {
            member_id,
            handle,
            display_name,
            role: Role::parse(&role).unwrap_or(Role::Member),
            grants: serde_json::from_str(&grants)?,
            via: AuthVia::Session { id },
        }))
    }

    // ---- CMD-8 ----
    /// Re-resolve a Credential that already authenticated (`AuthMember.via`) to its Member as
    /// it is now: `None` once the Token is revoked or expired, the session ended or expired, or
    /// the Member / Membership disabled; otherwise the current role and Grants. Read-only (no
    /// `last_used_at` / `last_seen_at` stamp, no commit hook), so a long-lived request (the SSE
    /// stream) can call it on every wake without waking anyone.
    pub fn reauthenticate(&self, via: &AuthVia) -> Result<Option<AuthMember>> {
        let (sql, id) = match via {
            AuthVia::Token { id } => (
                "SELECT t.expires_at, m.id, m.display_name, ms.handle, ms.role, ms.grants
                 FROM tokens t
                 JOIN members m ON m.id = t.member_id
                 JOIN memberships ms ON ms.member_id = m.id
                 WHERE t.id = ?1 AND t.revoked_at IS NULL
                   AND m.disabled_at IS NULL AND ms.disabled_at IS NULL",
                id,
            ),
            AuthVia::Session { id } => (
                "SELECT s.expires_at, m.id, m.display_name, ms.handle, ms.role, ms.grants
                 FROM sessions s
                 JOIN members m ON m.id = s.member_id
                 JOIN memberships ms ON ms.member_id = m.id
                 WHERE s.id = ?1 AND m.disabled_at IS NULL AND ms.disabled_at IS NULL",
                id,
            ),
        };
        let found = self
            .conn()
            .query_row(sql, [id], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .optional()?;
        let Some((expires_at, member_id, display_name, handle, role, grants)) = found else {
            return Ok(None);
        };
        if expires_at.is_some_and(|exp| exp <= time::now()) {
            return Ok(None);
        }
        Ok(Some(AuthMember {
            member_id,
            handle,
            display_name,
            role: Role::parse(&role).unwrap_or(Role::Member),
            grants: serde_json::from_str(&grants)?,
            via: via.clone(),
        }))
    }
    // ---- end CMD-8 ----

    /// End a session by its stored id (logout). `Ok(false)` when there was none.
    pub fn end_session(&self, session_id: &str) -> Result<bool> {
        self.tx(|tx| {
            let member: Option<String> =
                tx.query_row("SELECT member_id FROM sessions WHERE id = ?1", [session_id], |r| r.get(0)).optional()?;
            let Some(member) = member else {
                return Ok(false);
            };
            tx.execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
            append_event(tx, Some(&member), "session.ended", None, serde_json::json!({ "member": member }))?;
            Ok(true)
        })
    }
}

fn write_grants(tx: &Transaction, by: &str, member_id: &str, handle: &str, grants: &[Grant]) -> Result<MemberView> {
    tx.execute(
        "UPDATE memberships SET grants = ?1 WHERE member_id = ?2",
        params![serde_json::to_string(grants)?, member_id],
    )?;
    append_event(
        tx,
        Some(by),
        "member.granted",
        None,
        serde_json::json!({ "member": member_id, "handle": handle, "grants": grants }),
    )?;
    load_member(tx, member_id)?.ok_or(StoreError::MemberNotFound)
}

fn row_to_token(r: &rusqlite::Row<'_>) -> rusqlite::Result<TokenRow> {
    Ok(TokenRow {
        info: TokenInfo {
            id: r.get(0)?,
            member_id: r.get(1)?,
            name: r.get(2)?,
            created_at: r.get(3)?,
            expires_at: r.get(4)?,
            last_used_at: r.get(5)?,
            revoked_at: r.get(6)?,
            hash: r.get(7)?,
        },
        handle: r.get(8)?,
        display_name: r.get(9)?,
        member_disabled: r.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SetupInput;

    fn fresh() -> (tempfile::TempDir, Store, MemberView) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None })
            .unwrap();
        (dir, store, out.owner)
    }

    fn kinds(store: &Store) -> Vec<String> {
        store.events_since(0).unwrap().into_iter().map(|e| e.kind).collect()
    }

    // spec: S06-03, S06-04, S06-05
    #[test]
    fn rename_role_disable_and_the_last_owner_rule() {
        let (_d, store, owner) = fresh();
        let (linter, _) = store.add_member(&owner.id, "linter", None).unwrap();
        // Rename changes the display name only; the handle stays; mentions key on ids anyway.
        let m = store.rename_member(&owner.id, &linter.id, "Linter CLI").unwrap();
        assert_eq!((m.handle.as_str(), m.display_name.as_str()), ("linter", "Linter CLI"));
        assert!(store.rename_member(&owner.id, &linter.id, "  ").is_err());
        assert_eq!(store.rename_member(&owner.id, &linter.id, "Linter CLI").unwrap().display_name, "Linter CLI");
        // The last owner cannot be demoted or disabled.
        assert!(
            matches!(store.set_role(&owner.id, &owner.id, Role::Member), Err(StoreError::LastOwner(h)) if h == "rowan")
        );
        assert!(matches!(store.disable_member(&owner.id, &owner.id), Err(StoreError::LastOwner(_))));
        // Promote linter, then rowan may step down; linter is now the last owner.
        let (m, changed) = store.set_role(&owner.id, &linter.id, Role::Owner).unwrap();
        assert!(changed && m.role == Role::Owner);
        assert!(!store.set_role(&owner.id, &linter.id, Role::Owner).unwrap().1, "idempotent");
        assert_eq!(store.set_role(&linter.id, &owner.id, Role::Member).unwrap().0.role, Role::Member);
        assert!(matches!(store.set_role(&linter.id, &linter.id, Role::Member), Err(StoreError::LastOwner(_))));
        assert_eq!(store.founding_owner().unwrap().unwrap().handle, "linter");
        // Disable is idempotent and keeps the row (history intact); the handle stays taken.
        assert!(store.disable_member(&linter.id, &owner.id).unwrap());
        assert!(!store.disable_member(&linter.id, &owner.id).unwrap());
        assert!(store.member_by_handle("rowan").unwrap().unwrap().is_disabled());
        assert!(matches!(store.add_member(&linter.id, "rowan", None), Err(StoreError::HandleTaken(_))));
        assert!(matches!(store.set_role(&linter.id, &owner.id, Role::Owner), Err(StoreError::MemberDisabled(_))));
        assert!(matches!(store.grant(&linter.id, &owner.id, "*", &[Right::Read]), Err(StoreError::MemberDisabled(_))));
        assert!(matches!(
            store.mint_token_expiring(&linter.id, &owner.id, None, None),
            Err(StoreError::MemberDisabled(_))
        ));
        let ks = kinds(&store);
        for k in ["member.renamed", "member.role_set", "member.disabled"] {
            assert!(ks.contains(&k.to_string()), "{k} in {ks:?}");
        }
    }

    // spec: S06-01, S06-02
    #[test]
    fn grants_upsert_per_vault_and_validate_vault_names() {
        let (_d, store, owner) = fresh();
        let (linter, _) = store.add_member(&owner.id, "linter", None).unwrap();
        assert_eq!(linter.grants, vec![Grant::all()]);
        let m = store.grant(&owner.id, &linter.id, "notes", &[Right::Comment, Right::Comment, Right::Read]).unwrap();
        assert_eq!(m.grants.len(), 2);
        assert_eq!(m.grants[1], Grant { vault: "notes".into(), rights: vec![Right::Read, Right::Comment] });
        assert!(matches!(
            store.grant(&owner.id, &linter.id, "nope", &[Right::Read]),
            Err(StoreError::VaultNotFound(_))
        ));
        // Empty rights remove the entry; `*` can be dropped too.
        let m = store.grant(&owner.id, &linter.id, "*", &[]).unwrap();
        assert_eq!(m.grants, vec![Grant { vault: "notes".into(), rights: vec![Right::Read, Right::Comment] }]);
        let m = store.grant(&owner.id, &linter.id, "notes", &[]).unwrap();
        assert!(m.grants.is_empty());
        // replace_grants normalises: later entry wins, rights ordered, unknown Vault refused.
        store.create_vault(&owner.id, "drafts").unwrap();
        let m = store
            .replace_grants(
                &owner.id,
                &linter.id,
                vec![
                    Grant { vault: "drafts".into(), rights: vec![Right::Write, Right::Read] },
                    Grant { vault: "*".into(), rights: vec![Right::Read] },
                    Grant { vault: "drafts".into(), rights: vec![Right::Action] },
                ],
            )
            .unwrap();
        assert_eq!(
            m.grants,
            vec![
                Grant { vault: "*".into(), rights: vec![Right::Read] },
                Grant { vault: "drafts".into(), rights: vec![Right::Action] }
            ]
        );
        assert!(matches!(
            store.replace_grants(&owner.id, &linter.id, vec![Grant { vault: "*".into(), rights: vec![] }]),
            Err(StoreError::InvalidGrant(_))
        ));
        assert!(matches!(
            store.replace_grants(&owner.id, &linter.id, vec![Grant { vault: "zzz".into(), rights: vec![Right::Read] }]),
            Err(StoreError::VaultNotFound(_))
        ));
    }

    // spec: S06-07, S06-08
    #[test]
    fn tokens_expire_list_and_revoke_idempotently() {
        let (_d, store, owner) = fresh();
        let (linter, _) = store.add_member(&owner.id, "linter", None).unwrap();
        let past = "2020-01-01T00:00:00.000Z";
        assert!(matches!(
            store.mint_token_expiring(&owner.id, &linter.id, Some("x"), Some(past)),
            Err(StoreError::Invalid { .. })
        ));
        assert!(store.mint_token_expiring(&owner.id, &linter.id, None, Some("soon")).is_err());
        let future = "2999-01-01T00:00:00.000Z";
        let t = store.mint_token_expiring(&owner.id, &linter.id, Some("ci"), Some(future)).unwrap();
        assert!(store.authenticate(&t.plain).unwrap().is_some());
        let all = store.list_tokens(None).unwrap();
        assert_eq!(all.len(), 3, "owner first-run + linter add + this one");
        let mine = store.list_tokens(Some(&linter.id)).unwrap();
        assert_eq!(mine.len(), 2);
        let row = store.token_row(&t.id).unwrap().unwrap();
        assert_eq!(
            (row.handle.as_str(), row.info.name.as_deref(), row.info.expires_at.as_deref()),
            ("linter", Some("ci"), Some(future))
        );
        assert_eq!(row.status(&time::now()), "active");
        assert!(store.revoke_token(&owner.id, &t.id).unwrap());
        assert!(!store.revoke_token(&owner.id, &t.id).unwrap(), "second revoke changes nothing");
        assert!(store.authenticate(&t.plain).unwrap().is_none());
        assert_eq!(store.token_row(&t.id).unwrap().unwrap().status(&time::now()), "revoked");
        assert!(matches!(store.revoke_token(&owner.id, "nope"), Err(StoreError::TokenNotFound)));
        // An expired token stops authenticating without being revoked.
        let conn = rusqlite::Connection::open(store.dir().join(crate::DB_FILE)).unwrap();
        let t2 = store.mint_token_expiring(&owner.id, &linter.id, None, Some(future)).unwrap();
        conn.execute("UPDATE tokens SET expires_at = ?1 WHERE id = ?2", params![past, t2.id]).unwrap();
        assert!(store.authenticate(&t2.plain).unwrap().is_none());
        assert_eq!(store.token_row(&t2.id).unwrap().unwrap().status(&time::now()), "expired");
    }

    // spec: S06-09, S06-10, S06-11
    #[test]
    fn password_login_and_sessions() {
        let (_d, store, owner) = fresh();
        assert!(!store.password_set().unwrap());
        assert!(matches!(store.login("x"), Err(StoreError::PasswordNotSet)));
        assert!(store.set_password(&owner.id, &owner.id, Some("hunter2")).unwrap());
        assert!(store.password_set().unwrap());
        assert!(matches!(store.login("nope"), Err(StoreError::WrongPassword)));
        let (m, tok) = store.login("hunter2").unwrap();
        assert_eq!(m.handle, "rowan");
        assert_eq!(store.authenticate(&tok.plain).unwrap().unwrap().handle, "rowan");
        assert_eq!(store.token_row(&tok.id).unwrap().unwrap().info.name.as_deref(), Some("login"));
        assert!(store.verify_member_password(&owner.id, "hunter2").unwrap());
        assert!(!store.verify_member_password(&owner.id, "hunter3").unwrap());
        // The events never carry the hash; the db never carries the plain password.
        let events = store.events_since(0).unwrap();
        assert!(events.iter().any(|e| e.kind == "password.set"));
        assert!(!serde_json::to_string(&events).unwrap().contains("hunter2"));
        // Sessions: cookie value never stored, resolves to the Member, ends on logout.
        let s = store.open_session(&owner.id, "login").unwrap();
        assert!(s.plain.starts_with(SESSION_PREFIX));
        assert_eq!(s.id, token::hash(&s.plain));
        let a = store.authenticate_session(&s.plain).unwrap().unwrap();
        assert_eq!(a.handle, "rowan");
        assert_eq!(a.via, AuthVia::Session { id: s.id.clone() });
        assert!(store.authenticate_session("cds_nope").unwrap().is_none());
        assert!(store.authenticate_session(&tok.plain).unwrap().is_none(), "a Token is not a session");
        assert!(store.authenticate(&s.plain).unwrap().is_none(), "a session is not a Token");
        assert!(store.end_session(&s.id).unwrap());
        assert!(!store.end_session(&s.id).unwrap());
        assert!(store.authenticate_session(&s.plain).unwrap().is_none());
        // Removing the password: loopback trust returns.
        assert!(!store.set_password(&owner.id, &owner.id, None).unwrap());
        assert!(!store.password_set().unwrap());
        assert!(!store.verify_member_password(&owner.id, "hunter2").unwrap());
        // A disabled Member's session dies with them.
        let (linter, _) = store.add_member(&owner.id, "linter", None).unwrap();
        let s = store.open_session(&linter.id, "loopback").unwrap();
        assert!(store.authenticate_session(&s.plain).unwrap().is_some());
        store.disable_member(&owner.id, &linter.id).unwrap();
        assert!(store.authenticate_session(&s.plain).unwrap().is_none());
    }
}
