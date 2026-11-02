//! Vault management beyond create (ticket 12 — the web Settings screen): rename, delete, and
//! the Document counts the screen shows. Owner-only at the API (`server/vaults.rs`).
//!
//! - **Rename** ([`Store::rename_vault`]): the name is what Grants, `--vault`, profiles and the
//!   Sidecar's `vault` field carry, so the Grants of every Membership that name the Vault follow
//!   the rename in the same transaction (rights are unchanged, no `member.granted` events — the
//!   one `vault.renamed {vault, name, from}` event records it). Sidecars on disk keep the old name
//!   until the next push or pull rewrites them (push binds by Document id); a profile's `vault`
//!   follows through [`Store::resolve_profile_vault`] (CMD-5).
//! - **Delete** ([`Store::delete_vault`]): only an empty Vault (no Documents at all — deleted ones
//!   must be purged first, `VaultNotEmpty`) that is not the last one (`LastVault` — a Tenant
//!   always has a Vault, as first-run setup guarantees); Grants naming it are dropped from every
//!   Membership (`vault.deleted {vault, name, grants_dropped[]}`).
//! - **A profile's Vault across renames** (CMD-5, [`Store::resolve_profile_vault`]): the CLI sends
//!   the profile's `vault` name and, once it has learnt it, the Vault's id; the events log
//!   (`vault.created` / `vault.renamed` / `vault.deleted`, all `doc_id IS NULL`) says which Vault
//!   ids ever carried a name, so a profile follows its Vault through any chain of renames — even
//!   when a new Vault later takes the old name — while a hand-edited `vault` (a name its stale id
//!   never carried) wins over the id.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::error::{Result, StoreError};
use crate::model::{Grant, Vault};
use crate::store::{Store, append_event};

/// Documents a Vault holds: every row, and how many of those are deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VaultCount {
    pub documents: u64,
    pub deleted: u64,
}

/// Validate a Vault name (create and rename share it): not empty, no `/`, no control
/// characters, and never `*` (the wildcard of a Grant).
pub fn validate_vault_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.chars().any(char::is_control) {
        return Err(StoreError::Invalid { what: "vault name", reason: "must not be empty or contain '/'".into() });
    }
    if name == "*" {
        return Err(StoreError::Invalid { what: "vault name", reason: "'*' means every Vault in a Grant".into() });
    }
    Ok(name)
}

fn load_vault(tx: &Transaction, id: &str) -> Result<Option<Vault>> {
    Ok(tx
        .query_row("SELECT id, name, default_actions, created_at FROM vaults WHERE id = ?1", [id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })
        .optional()?
        .map(|(id, name, actions, created_at)| Vault {
            id,
            name,
            default_actions: serde_json::from_str(&actions).unwrap_or_default(),
            created_at,
        }))
}

fn count_in(tx: &Transaction, vault_id: &str) -> Result<VaultCount> {
    let (documents, deleted): (i64, i64) =
        tx.query_row("SELECT COUNT(*), COUNT(deleted_at) FROM documents WHERE vault_id = ?1", [vault_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    Ok(VaultCount { documents: documents as u64, deleted: deleted as u64 })
}

/// Every Membership's Grants that name `vault`: `(member_id, handle, grants)`.
fn memberships_naming(tx: &Transaction, vault: &str) -> Result<Vec<(String, String, Vec<Grant>)>> {
    let mut stmt = tx.prepare("SELECT member_id, handle, grants FROM memberships")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = Vec::new();
    for r in rows {
        let (id, handle, grants) = r?;
        let grants: Vec<Grant> = serde_json::from_str(&grants).unwrap_or_default();
        if grants.iter().any(|g| g.vault == vault) {
            out.push((id, handle, grants));
        }
    }
    Ok(out)
}

/// Where a profile's Vault (`name`, maybe its last known id) is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileVault {
    /// The live Vault (its `name` may differ from the one asked about: it was renamed).
    Resolved(Vault),
    /// Every Vault that ever carried the name (or the id that did) is deleted.
    Deleted { old_name: String },
    /// No Vault ever had the name, or more than one live Vault did (ambiguous).
    NotFound,
}

impl Store {
    /// The ids of every Vault that was ever named `name` — created with it, renamed from or to
    /// it, or deleted while carrying it — from the events log.
    fn vault_ids_ever_named(&self, name: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT json_extract(payload, '$.vault') FROM events
             WHERE doc_id IS NULL AND type IN ('vault.created', 'vault.renamed', 'vault.deleted')
               AND (json_extract(payload, '$.name') = ?1 OR json_extract(payload, '$.from') = ?1)
             ORDER BY seq",
        )?;
        let rows = stmt.query_map([name], |r| r.get::<_, Option<String>>(0))?;
        let mut ids: Vec<String> = Vec::new();
        for r in rows {
            if let Some(id) = r?
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// Resolve a profile's Vault (CMD-5): `name` is the profile's `vault`, `vault_id` the id the
    /// CLI recorded beside it, if any.
    ///
    /// - With an id that ever carried `name`: that Vault wherever it is now (renamed any number
    ///   of times, even when a newer Vault took `name`), or `Deleted` once it is gone.
    /// - Otherwise (no id, or an id that never carried `name` — the human edited `vault` and the
    ///   old id stayed in the file, so the name wins): the live Vault named `name`; else the one
    ///   live Vault that ever carried `name`; else `Deleted` when every Vault that carried it is
    ///   gone; else `NotFound` (never named so, or more than one live Vault was).
    pub fn resolve_profile_vault(&self, name: &str, vault_id: Option<&str>) -> Result<ProfileVault> {
        let name = name.trim();
        let vaults = self.list_vaults()?;
        let live = |id: &str| vaults.iter().find(|v| v.id == id).cloned();
        let mut named: Option<Vec<String>> = None;
        if let Some(id) = vault_id.map(str::trim).filter(|i| !i.is_empty()) {
            if let Some(v) = live(id).filter(|v| v.name == name) {
                return Ok(ProfileVault::Resolved(v));
            }
            let ids = self.vault_ids_ever_named(name)?;
            if ids.iter().any(|i| i == id) {
                return Ok(match live(id) {
                    Some(v) => ProfileVault::Resolved(v),
                    None => ProfileVault::Deleted { old_name: name.to_string() },
                });
            }
            named = Some(ids);
        }
        if let Some(v) = vaults.iter().find(|v| v.name == name) {
            return Ok(ProfileVault::Resolved(v.clone()));
        }
        let ids = match named {
            Some(ids) => ids,
            None => self.vault_ids_ever_named(name)?,
        };
        let alive: Vec<Vault> = ids.iter().filter_map(|i| live(i)).collect();
        Ok(match (alive.len(), ids.is_empty()) {
            (1, _) => ProfileVault::Resolved(alive.into_iter().next().expect("one")),
            (0, false) => ProfileVault::Deleted { old_name: name.to_string() },
            _ => ProfileVault::NotFound,
        })
    }

    /// The live Vault an unknown `name` was renamed to, when exactly one live Vault ever carried
    /// it (for the hint on an explicit `--vault OLD`'s 404); `None` when a Vault is named `name`
    /// today, none ever was, or it is ambiguous.
    pub fn renamed_from_hint(&self, name: &str) -> Result<Option<Vault>> {
        Ok(match self.resolve_profile_vault(name, None)? {
            ProfileVault::Resolved(v) if v.name != name.trim() => Some(v),
            _ => None,
        })
    }

    /// How many Documents (and how many of them deleted) a Vault holds.
    pub fn vault_count(&self, vault_id: &str) -> Result<VaultCount> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        count_in(&tx, vault_id)
    }

    /// Rename a Vault (owner-only at the API). The Grants naming the old name follow; a rename
    /// to the same name is a no-op (no event); a taken name ⇒ `VaultNameTaken`.
    pub fn rename_vault(&self, by: &str, vault_id: &str, new_name: &str) -> Result<Vault> {
        let new_name = validate_vault_name(new_name)?.to_string();
        self.tx(|tx| {
            let v = load_vault(tx, vault_id)?.ok_or_else(|| StoreError::VaultNotFound(vault_id.to_string()))?;
            if v.name == new_name {
                return Ok(v);
            }
            let taken: bool = tx
                .query_row("SELECT 1 FROM vaults WHERE name = ?1 AND id <> ?2", params![new_name, vault_id], |_| Ok(()))
                .optional()?
                .is_some();
            if taken {
                return Err(StoreError::VaultNameTaken(new_name));
            }
            tx.execute("UPDATE vaults SET name = ?1 WHERE id = ?2", params![new_name, vault_id])?;
            let mut followed = Vec::new();
            for (member_id, handle, mut grants) in memberships_naming(tx, &v.name)? {
                for g in grants.iter_mut().filter(|g| g.vault == v.name) {
                    g.vault = new_name.clone();
                }
                tx.execute(
                    "UPDATE memberships SET grants = ?1 WHERE member_id = ?2",
                    params![serde_json::to_string(&grants)?, member_id],
                )?;
                followed.push(handle);
            }
            append_event(
                tx,
                Some(by),
                "vault.renamed",
                None,
                serde_json::json!({ "vault": v.id, "name": new_name, "from": v.name, "grants_followed": followed }),
            )?;
            load_vault(tx, vault_id)?.ok_or_else(|| StoreError::VaultNotFound(vault_id.to_string()))
        })
    }

    /// Delete an empty Vault (owner-only at the API): `VaultNotEmpty` while any Document — live
    /// or deleted — is in it, `LastVault` for the only Vault. Grants naming it are dropped from
    /// every Membership. Returns the Vault as it was.
    pub fn delete_vault(&self, by: &str, vault_id: &str) -> Result<Vault> {
        self.tx(|tx| {
            let v = load_vault(tx, vault_id)?.ok_or_else(|| StoreError::VaultNotFound(vault_id.to_string()))?;
            let count = count_in(tx, vault_id)?;
            if count.documents > 0 {
                return Err(StoreError::VaultNotEmpty {
                    name: v.name,
                    documents: count.documents,
                    deleted: count.deleted,
                });
            }
            let vaults: i64 = tx.query_row("SELECT COUNT(*) FROM vaults", [], |r| r.get(0))?;
            if vaults <= 1 {
                return Err(StoreError::LastVault(v.name));
            }
            let mut dropped = Vec::new();
            for (member_id, handle, grants) in memberships_naming(tx, &v.name)? {
                let grants: Vec<Grant> = grants.into_iter().filter(|g| g.vault != v.name).collect();
                tx.execute(
                    "UPDATE memberships SET grants = ?1 WHERE member_id = ?2",
                    params![serde_json::to_string(&grants)?, member_id],
                )?;
                dropped.push(handle);
            }
            tx.execute("DELETE FROM vaults WHERE id = ?1", [vault_id])?;
            append_event(
                tx,
                Some(by),
                "vault.deleted",
                None,
                serde_json::json!({ "vault": v.id, "name": v.name, "grants_dropped": dropped }),
            )?;
            Ok(v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Right;
    use crate::store::SetupInput;

    fn store() -> (tempfile::TempDir, Store, String) {
        let tmp = tempfile::tempdir().unwrap();
        let st = Store::open(tmp.path()).unwrap();
        let out =
            st.setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None }).unwrap();
        let id = out.owner.id;
        (tmp, st, id)
    }

    #[test]
    fn rename_follows_grants_and_refuses_taken_or_wildcard_names() {
        let (_tmp, st, owner) = store();
        let drafts = st.create_vault(&owner, "drafts").unwrap();
        let (m, _) = st.add_member(&owner, "linter", None).unwrap();
        st.grant(&owner, &m.id, "drafts", &[Right::Read, Right::Comment]).unwrap();
        st.grant(&owner, &m.id, "*", &[]).unwrap();
        assert!(
            matches!(st.rename_vault(&owner, &drafts.id, "notes"), Err(StoreError::VaultNameTaken(n)) if n == "notes")
        );
        assert!(matches!(st.rename_vault(&owner, &drafts.id, "*"), Err(StoreError::Invalid { .. })));
        assert!(matches!(st.rename_vault(&owner, &drafts.id, "a/b"), Err(StoreError::Invalid { .. })));
        let before = st.events_since(0).unwrap().len();
        // Same name: no-op, no event.
        st.rename_vault(&owner, &drafts.id, "drafts").unwrap();
        assert_eq!(st.events_since(0).unwrap().len(), before);
        let v = st.rename_vault(&owner, &drafts.id, "reviews").unwrap();
        assert_eq!(v.name, "reviews");
        assert!(st.vault_by_name("drafts").unwrap().is_none());
        let m = st.member_by_id(&m.id).unwrap().unwrap();
        assert_eq!(m.grants, vec![Grant { vault: "reviews".into(), rights: vec![Right::Read, Right::Comment] }]);
        let ev = st.events_since(0).unwrap().into_iter().last().unwrap();
        assert_eq!(ev.kind, "vault.renamed");
        assert_eq!(ev.payload["from"], "drafts");
        assert_eq!(ev.payload["name"], "reviews");
        assert_eq!(ev.payload["grants_followed"], serde_json::json!(["linter"]));
        assert!(matches!(st.rename_vault(&owner, "nope", "x"), Err(StoreError::VaultNotFound(_))));
    }

    // spec: S26-05
    #[test]
    fn a_profile_vault_follows_renames_by_id_and_the_name_wins_when_hand_edited() {
        use ProfileVault::*;
        let (_tmp, st, owner) = store();
        let a = st.create_vault(&owner, "a").unwrap();
        let other = st.create_vault(&owner, "other").unwrap();
        // Unchanged: by name alone (backfills the id) and by name + id.
        assert_eq!(st.resolve_profile_vault("a", None).unwrap(), Resolved(a.clone()));
        assert_eq!(st.resolve_profile_vault(" a ", Some(&a.id)).unwrap(), Resolved(a.clone()));
        // a -> b -> c, then a new Vault takes `a`.
        st.rename_vault(&owner, &a.id, "b").unwrap();
        let c = st.rename_vault(&owner, &a.id, "c").unwrap();
        // (d) a legacy profile (no id) follows the chain while nothing else is named `a`.
        assert_eq!(st.resolve_profile_vault("a", None).unwrap(), Resolved(c.clone()));
        assert_eq!(st.resolve_profile_vault("b", None).unwrap(), Resolved(c.clone()));
        assert_eq!(st.renamed_from_hint("a").unwrap(), Some(c.clone()));
        assert_eq!(st.renamed_from_hint("c").unwrap(), None, "a live name is no rename");
        let new_a = st.create_vault(&owner, "a").unwrap();
        // (c) with the id: still `c`, never the new `a`; without it the live name wins.
        assert_eq!(st.resolve_profile_vault("a", Some(&a.id)).unwrap(), Resolved(c.clone()));
        assert_eq!(st.resolve_profile_vault("b", Some(&a.id)).unwrap(), Resolved(c.clone()));
        assert_eq!(st.resolve_profile_vault("a", None).unwrap(), Resolved(new_a.clone()));
        assert_eq!(st.renamed_from_hint("a").unwrap(), None);
        // `b` was only ever the first Vault: unambiguous without an id.
        assert_eq!(st.resolve_profile_vault("b", None).unwrap(), Resolved(c.clone()));
        // (e) a hand-edited name its stale id never carried: the name wins.
        assert_eq!(st.resolve_profile_vault("other", Some(&a.id)).unwrap(), Resolved(other.clone()));
        assert_eq!(st.resolve_profile_vault("other", Some("01NOTANID")).unwrap(), Resolved(other.clone()));
        // Never a Vault: not found (with or without an id).
        assert_eq!(st.resolve_profile_vault("nowhere", None).unwrap(), NotFound);
        assert_eq!(st.resolve_profile_vault("nowhere", Some(&a.id)).unwrap(), NotFound);
        // Two live Vaults once named `x` (and neither is now): ambiguous without an id.
        let x1 = st.create_vault(&owner, "x").unwrap();
        st.rename_vault(&owner, &x1.id, "x1").unwrap();
        let x2 = st.create_vault(&owner, "x").unwrap();
        let x2 = st.rename_vault(&owner, &x2.id, "x2").unwrap();
        assert_eq!(st.resolve_profile_vault("x", None).unwrap(), NotFound);
        assert_eq!(st.resolve_profile_vault("x", Some(&x2.id)).unwrap(), Resolved(x2));
        // (g) deleted: by id once gone, and by name when every Vault that carried it is gone.
        st.delete_vault(&owner, &a.id).unwrap();
        assert_eq!(st.resolve_profile_vault("c", Some(&a.id)).unwrap(), Deleted { old_name: "c".into() });
        assert_eq!(st.resolve_profile_vault("b", Some(&a.id)).unwrap(), Deleted { old_name: "b".into() });
        assert_eq!(st.resolve_profile_vault("c", None).unwrap(), Deleted { old_name: "c".into() });
        assert_eq!(st.resolve_profile_vault("a", Some(&a.id)).unwrap(), Deleted { old_name: "a".into() });
        assert_eq!(st.resolve_profile_vault("a", None).unwrap(), Resolved(new_a));
        // Non-ASCII names resolve like any other.
        let u = st.create_vault(&owner, "Übersicht").unwrap();
        let u2 = st.rename_vault(&owner, &u.id, "Überblick").unwrap();
        assert_eq!(st.resolve_profile_vault("Übersicht", Some(&u.id)).unwrap(), Resolved(u2.clone()));
        assert_eq!(st.resolve_profile_vault("Übersicht", None).unwrap(), Resolved(u2));
    }

    #[test]
    fn delete_only_empty_never_the_last_and_drops_grants() {
        let (_tmp, st, owner) = store();
        let notes = st.vault_by_name("notes").unwrap().unwrap();
        assert!(matches!(st.delete_vault(&owner, &notes.id), Err(StoreError::LastVault(n)) if n == "notes"));
        let drafts = st.create_vault(&owner, "drafts").unwrap();
        let (m, _) = st.add_member(&owner, "linter", None).unwrap();
        st.grant(&owner, &m.id, "drafts", &[Right::Write]).unwrap();
        // A Document in it: refused (even deleted ones count until purged).
        let (hash, _) = st.put_blob(b"# x\n").unwrap();
        let doc = st.create_document(&owner, &drafts.id, crate::model::PushInput::new("x", &hash)).unwrap();
        st.delete_document(&owner, &doc.id).unwrap();
        assert_eq!(st.vault_count(&drafts.id).unwrap(), VaultCount { documents: 1, deleted: 1 });
        match st.delete_vault(&owner, &drafts.id) {
            Err(StoreError::VaultNotEmpty { name, documents, deleted }) => {
                assert_eq!((name.as_str(), documents, deleted), ("drafts", 1, 1));
            }
            other => panic!("{other:?}"),
        }
        st.purge_document(&owner, &doc.id).unwrap();
        let gone = st.delete_vault(&owner, &drafts.id).unwrap();
        assert_eq!(gone.name, "drafts");
        assert!(st.vault_by_name("drafts").unwrap().is_none());
        // The `drafts` Grant is gone; the `*` Grant `add_member` gave stays.
        assert_eq!(st.member_by_id(&m.id).unwrap().unwrap().grants, vec![Grant::all()]);
        let ev = st.events_since(0).unwrap().into_iter().last().unwrap();
        assert_eq!(ev.kind, "vault.deleted");
        assert_eq!(ev.payload["grants_dropped"], serde_json::json!(["linter"]));
        assert!(matches!(st.delete_vault(&owner, &drafts.id), Err(StoreError::VaultNotFound(_))));
    }
}
