//! Documents, Versions and blobs on the [`Store`]: create / push (with `already` and `same_as`
//! derived from hashes), list / resolve, soft delete / restore / purge (+ blob GC).
//!
//! Every mutation runs in one `Store::tx` and appends its event before commit:
//! `document.created {vault, name}`, `version.pushed {version, name, hash, message?, same_as?}`,
//! `document.deleted|restored|purged {name}`. The Store's single-writer transaction is the
//! per-Document lock around number allocation + event append. Later tickets extend the same
//! places: the manifest is validated/normalised by `crate::manifest` (ticket 03; caps in
//! `crate::caps`; [`version_blob_refs`] lists Asset + derivative hashes), 04/05 call
//! [`bump_activity`] from Thread/Action events and add their rows to `purge_document`, 07 stores
//! `placements`, 09 `renderer` / `html_hash`.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use rusqlite::{OptionalExtension, Transaction, params};

use annotary_sidecar::identity::version_hash;

use crate::blobs::BlobStore;
use crate::error::{Result, StoreError};
use crate::model::*;
use crate::store::{Store, append_event};
use crate::{ids, time};

/// What `purge_document` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub id: String,
    pub name: String,
    /// Blob files removed because no remaining Version referenced them.
    pub blobs_removed: Vec<String>,
}

const DOC_SELECT: &str = "SELECT d.id, d.vault_id, vl.name, d.created_at, d.created_by, d.deleted_at, d.last_activity_at,
        v.number, v.name, v.hash, v.md_hash, v.by, ms.handle, v.at, v.message, v.manifest, v.placements, v.renderer, v.html_hash,
        d.actions_available
     FROM documents d
     JOIN vaults vl ON vl.id = d.vault_id
     JOIN versions v ON v.doc_id = d.id AND v.number = (SELECT MAX(number) FROM versions WHERE doc_id = d.id)
     JOIN memberships ms ON ms.member_id = v.by";

const VERSION_SELECT: &str = "SELECT v.doc_id, v.number, v.name, v.hash, v.md_hash, v.by, ms.handle, v.at, v.message,
        v.manifest, v.placements, v.renderer, v.html_hash
     FROM versions v JOIN memberships ms ON ms.member_id = v.by";

fn row_to_version(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<VersionView> {
    let manifest: String = r.get(offset + 9)?;
    let placements: String = r.get(offset + 10)?;
    Ok(VersionView {
        doc_id: r.get(offset)?,
        number: r.get::<_, i64>(offset + 1)? as u32,
        name: r.get(offset + 2)?,
        hash: r.get(offset + 3)?,
        md_hash: r.get(offset + 4)?,
        by: r.get(offset + 5)?,
        by_handle: r.get(offset + 6)?,
        at: r.get(offset + 7)?,
        message: r.get(offset + 8)?,
        manifest: serde_json::from_str(&manifest).unwrap_or(serde_json::Value::Array(Vec::new())),
        placements: serde_json::from_str(&placements).unwrap_or(serde_json::Value::Array(Vec::new())),
        renderer: r.get(offset + 11)?,
        html_hash: r.get(offset + 12)?,
        same_as: None,
    })
}

fn row_to_document(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentView> {
    let manifest: String = r.get(15)?;
    let placements: String = r.get(16)?;
    let id: String = r.get(0)?;
    let actions_available: Option<String> = r.get(19)?; // ticket 05
    Ok(DocumentView {
        vault_id: r.get(1)?,
        vault_name: r.get(2)?,
        created_at: r.get(3)?,
        created_by: r.get(4)?,
        deleted_at: r.get(5)?,
        last_activity_at: r.get(6)?,
        head: VersionView {
            doc_id: id.clone(),
            number: r.get::<_, i64>(7)? as u32,
            name: r.get(8)?,
            hash: r.get(9)?,
            md_hash: r.get(10)?,
            by: r.get(11)?,
            by_handle: r.get(12)?,
            at: r.get(13)?,
            message: r.get(14)?,
            manifest: serde_json::from_str(&manifest).unwrap_or(serde_json::Value::Array(Vec::new())),
            placements: serde_json::from_str(&placements).unwrap_or(serde_json::Value::Array(Vec::new())),
            renderer: r.get(17)?,
            html_hash: r.get(18)?,
            same_as: None,
        },
        id,
        actions_available: actions_available.and_then(|a| serde_json::from_str(&a).ok()),
    })
}

/// `same_as`: the lowest earlier Version with the same identity, or None.
fn same_as(conn: &rusqlite::Connection, doc_id: &str, hash: &str, number: u32) -> Result<Option<u32>> {
    Ok(conn
        .query_row(
            "SELECT MIN(number) FROM versions WHERE doc_id = ?1 AND hash = ?2 AND number < ?3",
            params![doc_id, hash, number],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .map(|n| n as u32))
}

/// The `(ref, hash)` pairs of a manifest (missing entries have no hash but still count).
pub fn manifest_pairs(manifest: &serde_json::Value) -> Vec<(String, Option<String>)> {
    manifest
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let r = e.get("ref")?.as_str()?;
                    Some((r.to_string(), e.get("hash").and_then(|h| h.as_str()).map(str::to_string)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The Version identity for a commit: sha256 over the Markdown blob hash + sorted manifest.
pub fn identity_of(md_hash: &str, manifest: &serde_json::Value) -> String {
    let pairs = manifest_pairs(manifest);
    version_hash(md_hash, pairs.iter().map(|(r, h)| (r.as_str(), h.as_deref())))
}

/// Every blob hash a Version row references: its Markdown, each manifest entry's `hash` and
/// `derivative` (ticket 03), and its rendered `html_hash` (ticket 09). Extend here only.
pub fn version_blob_refs(md_hash: &str, manifest: &serde_json::Value, html_hash: Option<&str>) -> Vec<String> {
    let mut out = vec![md_hash.to_string()];
    if let Some(entries) = manifest.as_array() {
        for e in entries {
            for key in ["hash", "derivative"] {
                if let Some(h) = e.get(key).and_then(|v| v.as_str()) {
                    out.push(h.to_string());
                }
            }
        }
    }
    if let Some(h) = html_hash {
        out.push(h.to_string());
    }
    out
}

/// Bump a Document's `last_activity_at` (any Version/Thread/Action event). `pub(crate)` for the
/// Thread/Action mutations of later tickets.
pub(crate) fn bump_activity(tx: &Transaction, doc_id: &str, now: &str) -> Result<()> {
    tx.execute("UPDATE documents SET last_activity_at = ?1 WHERE id = ?2", params![now, doc_id])?;
    Ok(())
}

pub(crate) fn load_document(conn: &rusqlite::Connection, id: &str) -> Result<Option<DocumentView>> {
    let doc = conn.query_row(&format!("{DOC_SELECT} WHERE d.id = ?1"), [id], row_to_document).optional()?;
    match doc {
        Some(mut d) => {
            d.head.same_as = same_as(conn, &d.id, &d.head.hash, d.head.number)?;
            Ok(Some(d))
        }
        None => Ok(None),
    }
}

/// `(hash, md_hash, manifest)` of Version `number` (ticket 17 reads the base and the head).
fn version_content(tx: &Transaction, doc_id: &str, number: u32) -> Result<(String, String, serde_json::Value)> {
    let (hash, md, manifest): (String, String, String) = tx
        .query_row(
            "SELECT hash, md_hash, manifest FROM versions WHERE doc_id = ?1 AND number = ?2",
            params![doc_id, number as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::VersionNotFound { doc: doc_id.to_string(), number })?;
    Ok((hash, md, serde_json::from_str(&manifest)?))
}

fn head_number(tx: &Transaction, doc_id: &str) -> Result<u32> {
    Ok(tx.query_row("SELECT COALESCE(MAX(number), 0) FROM versions WHERE doc_id = ?1", [doc_id], |r| {
        r.get::<_, i64>(0)
    })? as u32)
}

fn insert_version(
    tx: &Transaction,
    by: &str,
    doc_id: &str,
    number: u32,
    input: &PushInput,
    hash: &str,
    now: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO versions(doc_id, number, name, hash, md_hash, by, at, message, manifest, placements, renderer, html_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            doc_id,
            number as i64,
            input.name,
            hash,
            input.md_hash,
            by,
            now,
            input.message,
            serde_json::to_string(&input.manifest)?,
            serde_json::to_string(&input.placements)?,
            input.renderer,
            input.html_hash,
        ],
    )?;
    Ok(())
}

impl Store {
    /// The blob store under `store/blobs`.
    pub fn blobs(&self) -> &BlobStore {
        &self.blobs
    }

    /// Store a blob (any content). Serialised with purge/GC through the store mutex so an
    /// uploaded-but-not-yet-committed blob is never swept from under a push.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<(String, bool)> {
        let _guard = self.conn();
        self.blobs.put(bytes)
    }

    /// Store a blob only if it hashes to `expected`.
    pub fn put_blob_expecting(&self, expected: &str, bytes: &[u8]) -> Result<(String, bool)> {
        let _guard = self.conn();
        self.blobs.put_expecting(expected, bytes)
    }

    pub fn has_blob(&self, hash: &str) -> bool {
        self.blobs.has(hash)
    }

    pub fn read_blob(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        self.blobs.read(hash)
    }

    /// Of `hashes`, the ones the store does not have (phase 1 of the two-phase push).
    pub fn lacking_blobs(&self, hashes: &[String]) -> Vec<String> {
        hashes.iter().filter(|h| !self.blobs.has(h)).cloned().collect()
    }

    // ---------------------------------------------------------------- vaults

    pub fn vault_by_name(&self, name: &str) -> Result<Option<Vault>> {
        Ok(self.list_vaults()?.into_iter().find(|v| v.name == name))
    }

    pub fn vault_by_id(&self, id: &str) -> Result<Option<Vault>> {
        Ok(self.list_vaults()?.into_iter().find(|v| v.id == id))
    }

    // ---------------------------------------------------------------- create / push

    /// Create a Document with v1 in `vault_id`. The Markdown blob and every manifest blob must
    /// already be stored; the manifest is validated and normalised (`crate::manifest`, caps in
    /// `crate::caps`).
    pub fn create_document(&self, by: &str, vault_id: &str, mut input: PushInput) -> Result<DocumentView> {
        validate_name(&input.name)?;
        if !self.blobs.has(&input.md_hash) {
            return Err(StoreError::BlobMissing(input.md_hash.clone()));
        }
        self.check_render_fields(&mut input)?; // ticket 09
        input.manifest = crate::manifest::validate(&self.blobs, &input.md_hash, &input.manifest)?.manifest;
        let declaration = input.actions_available.as_ref().map(crate::actions::validate_declaration).transpose()?; // ticket 05
        let hash = identity_of(&input.md_hash, &input.manifest);
        let id = self.tx(|tx| {
            let vault: Option<String> =
                tx.query_row("SELECT name FROM vaults WHERE id = ?1", [vault_id], |r| r.get(0)).optional()?;
            let Some(vault_name) = vault else {
                return Err(StoreError::VaultNotFound(vault_id.to_string()));
            };
            let now = time::now();
            let id = loop {
                let candidate = ids::document_id();
                let taken: bool = tx
                    .query_row("SELECT 1 FROM documents WHERE id = ?1", [&candidate], |_| Ok(()))
                    .optional()?
                    .is_some();
                if !taken {
                    break candidate;
                }
            };
            tx.execute(
                "INSERT INTO documents(id, vault_id, created_at, created_by, last_activity_at) VALUES (?1, ?2, ?3, ?4, ?3)",
                params![id, vault_id, now, by],
            )?;
            insert_version(tx, by, &id, 1, &input, &hash, &now)?;
            append_event(
                tx,
                Some(by),
                "document.created",
                Some(&id),
                serde_json::json!({ "vault": vault_name, "name": input.name }),
            )?;
            append_event(
                tx,
                Some(by),
                "version.pushed",
                Some(&id),
                serde_json::json!({ "version": 1, "name": input.name, "hash": hash, "message": input.message }),
            )?;
            if let Some(decls) = &declaration {
                crate::actions::declare(tx, by, &id, &input.name, decls)?; // ticket 05
            }
            Ok(id)
        })?;
        Ok(self.document(&id)?.expect("just created"))
    }

    /// Push onto an existing Document: no-op when identical to head (`already`), a new Version
    /// otherwise (`same_as` when identical to an older one). Ticket 17: when `input.base_version`
    /// is behind the head the push is accepted only with `discard_head_changes`, or with
    /// `contains_head` == the head — rechecked with `concurrent::check_head_changes` while
    /// [`Store::verify_edge`] is on — else [`StoreError::HeadMoved`].
    pub fn push_version(&self, by: &str, doc_id: &str, mut input: PushInput) -> Result<PushOutcome> {
        validate_name(&input.name)?;
        if !self.blobs.has(&input.md_hash) {
            return Err(StoreError::BlobMissing(input.md_hash.clone()));
        }
        self.check_render_fields(&mut input)?; // ticket 09
        input.manifest = crate::manifest::validate(&self.blobs, &input.md_hash, &input.manifest)?.manifest;
        let declaration = input.actions_available.as_ref().map(crate::actions::validate_declaration).transpose()?; // ticket 05
        let hash = identity_of(&input.md_hash, &input.manifest);
        let (number, already, head_moved, discarded, reanchored) = self.tx(|tx| {
            let deleted: Option<Option<String>> =
                tx.query_row("SELECT deleted_at FROM documents WHERE id = ?1", [doc_id], |r| r.get(0)).optional()?;
            match deleted {
                None => return Err(StoreError::DocumentNotFound(doc_id.to_string())),
                Some(Some(_)) => return Err(StoreError::DocumentDeleted(doc_id.to_string())),
                Some(None) => {}
            }
            let head = head_number(tx, doc_id)?;
            let head_moved = match input.base_version {
                Some(base) if base < head => {
                    let mut stmt = tx.prepare(
                        "SELECT ms.handle FROM versions v JOIN memberships ms ON ms.member_id = v.by
                         WHERE v.doc_id = ?1 AND v.number > ?2 ORDER BY v.number",
                    )?;
                    let mut seen = Vec::new();
                    for h in stmt.query_map(params![doc_id, base as i64], |r| r.get::<_, String>(0))? {
                        let h = h?;
                        if !seen.contains(&h) {
                            seen.push(h);
                        }
                    }
                    Some(HeadMoved { base, head, by: seen })
                }
                _ => None,
            };
            let (head_hash, head_md, head_manifest) = version_content(tx, doc_id, head)?;
            // ticket 05: the per-Document declaration is applied whether or not the content
            // changed (`already` still lets a Sidecar edit declare Actions).
            if let Some(decls) = &declaration {
                crate::actions::declare(tx, by, doc_id, &input.name, decls)?;
            }
            if head_hash == hash {
                return Ok((head, true, head_moved, Vec::new(), None));
            }
            let read_text = |md: &str| -> Result<String> {
                let bytes = self.blobs.read(md)?.ok_or_else(|| StoreError::BlobMissing(md.to_string()))?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            };
            let text_n = read_text(&head_md)?;
            let text_n1 = read_text(&input.md_hash)?;
            // ---- ticket 17: a moved head needs the head's changes in the file, or a discard.
            let mut discarded = Vec::new();
            if let Some(hm) = &head_moved {
                let refuse = |reason| {
                    StoreError::HeadMoved(crate::concurrent::HeadMovedError {
                        name: input.name.clone(),
                        base: hm.base,
                        head: hm.head,
                        by: hm.by.clone(),
                        reason,
                    })
                };
                if input.discard_head_changes {
                    discarded = (hm.base + 1..=hm.head).collect();
                } else {
                    match input.contains_head {
                        None => return Err(refuse(crate::concurrent::HeadMovedReason::Unchecked)),
                        Some(checked) if checked != head => {
                            return Err(refuse(crate::concurrent::HeadMovedReason::MovedAgain { checked }));
                        }
                        Some(_) if self.verify_edge() => {
                            let (_, base_md, base_manifest) = version_content(tx, doc_id, hm.base)?;
                            let check = crate::concurrent::check_head_changes(
                                &read_text(&base_md)?,
                                &text_n,
                                &text_n1,
                                &base_manifest,
                                &head_manifest,
                                &input.manifest,
                            );
                            if !check.is_contained() {
                                return Err(refuse(crate::concurrent::HeadMovedReason::Missing));
                            }
                        }
                        Some(_) => {}
                    }
                }
            }
            // ---- end ticket 17
            let number = head + 1;
            let now = time::now();
            insert_version(tx, by, doc_id, number, &input, &hash, &now)?;
            bump_activity(tx, doc_id, &now)?;
            let same = same_as(tx, doc_id, &hash, number)?;
            let mut payload = serde_json::json!({
                "version": number, "name": input.name, "hash": hash, "message": input.message, "same_as": same,
            });
            if !discarded.is_empty() {
                payload["discarded"] = serde_json::json!(discarded); // ticket 17
            }
            append_event(tx, Some(by), "version.pushed", Some(doc_id), payload)?;
            // ---- ticket 07: re-anchor every Thread from the head onto the new Version, in the
            // same transaction (the Version and its Threads' placements are one fact).
            let reanchored = crate::anchors::apply_on_push(
                tx,
                doc_id,
                head,
                number,
                &text_n,
                &text_n1,
                &input.manifest,
                &input.asset_lines,
                &input.placements,
                input.placements_from,
                self.verify_edge(),
            )?;
            // ---- end ticket 07
            Ok((number, false, head_moved, discarded, Some(reanchored)))
        })?;
        let version = self.version(doc_id, number)?.expect("just written");
        Ok(PushOutcome { version, already, head_moved, discarded, reanchored })
    }

    // ---------------------------------------------------------------- reads

    /// One Document (deleted or not) with its head Version.
    pub fn document(&self, id: &str) -> Result<Option<DocumentView>> {
        load_document(&self.conn(), id)
    }

    /// Documents matching `filter`, newest activity first. Returns `(rows, total_matched)`; rows
    /// are cut to `filter.limit`.
    pub fn list_documents(&self, filter: &DocFilter) -> Result<(Vec<DocumentView>, usize)> {
        let conn = self.conn();
        let mut sql = format!("{DOC_SELECT} WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        match filter.deleted {
            DeletedFilter::Exclude => sql.push_str(" AND d.deleted_at IS NULL"),
            DeletedFilter::Only => sql.push_str(" AND d.deleted_at IS NOT NULL"),
            DeletedFilter::Any => {}
        }
        if let Some(ids) = &filter.vault_ids {
            if ids.is_empty() {
                return Ok((Vec::new(), 0));
            }
            let marks: Vec<String> = ids.iter().enumerate().map(|(i, _)| format!("?{}", args.len() + i + 1)).collect();
            sql.push_str(&format!(" AND d.vault_id IN ({})", marks.join(", ")));
            for id in ids {
                args.push(Box::new(id.clone()));
            }
        }
        if let Some(since) = &filter.since {
            args.push(Box::new(since.clone()));
            sql.push_str(&format!(" AND d.last_activity_at >= ?{}", args.len()));
        }
        if let Some(exact) = &filter.name_exact {
            args.push(Box::new(exact.clone()));
            sql.push_str(&format!(" AND v.name = ?{}", args.len()));
        }
        sql.push_str(" ORDER BY d.last_activity_at DESC, d.created_at DESC, d.id");
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(params.as_slice(), row_to_document)?;
        let needle = filter.name_contains.as_ref().map(|s| s.to_lowercase());
        let mut all = Vec::new();
        for r in rows {
            let d = r?;
            if let Some(n) = &needle
                && !d.head.name.to_lowercase().contains(n.as_str())
            {
                continue;
            }
            all.push(d);
        }
        let total = all.len();
        if let Some(limit) = filter.limit {
            all.truncate(limit);
        }
        for d in &mut all {
            d.head.same_as = same_as(&conn, &d.id, &d.head.hash, d.head.number)?;
        }
        Ok((all, total))
    }

    /// Every Version of a Document, ascending, with `same_as` derived.
    pub fn versions(&self, doc_id: &str) -> Result<Vec<VersionView>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{VERSION_SELECT} WHERE v.doc_id = ?1 ORDER BY v.number"))?;
        let rows = stmt.query_map([doc_id], |r| row_to_version(r, 0))?;
        let mut out = Vec::new();
        for r in rows {
            let mut v = r?;
            v.same_as = same_as(&conn, doc_id, &v.hash, v.number)?;
            out.push(v);
        }
        Ok(out)
    }

    /// One Version.
    pub fn version(&self, doc_id: &str, number: u32) -> Result<Option<VersionView>> {
        let conn = self.conn();
        let v = conn
            .query_row(
                &format!("{VERSION_SELECT} WHERE v.doc_id = ?1 AND v.number = ?2"),
                params![doc_id, number as i64],
                |r| row_to_version(r, 0),
            )
            .optional()?;
        match v {
            Some(mut v) => {
                v.same_as = same_as(&conn, doc_id, &v.hash, v.number)?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    }

    // ---------------------------------------------------------------- delete / restore / purge

    /// Soft delete: hidden by default, everything retained.
    pub fn delete_document(&self, by: &str, id: &str) -> Result<DocumentView> {
        self.tx(|tx| {
            let now = time::now();
            let n = tx.execute(
                "UPDATE documents SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![now, id],
            )?;
            if n == 0 {
                return match tx.query_row("SELECT 1 FROM documents WHERE id = ?1", [id], |_| Ok(())).optional()? {
                    Some(()) => Err(StoreError::DocumentDeleted(id.to_string())),
                    None => Err(StoreError::DocumentNotFound(id.to_string())),
                };
            }
            let name = head_name(tx, id)?;
            append_event(tx, Some(by), "document.deleted", Some(id), serde_json::json!({ "name": name }))?;
            Ok(())
        })?;
        Ok(self.document(id)?.expect("exists"))
    }

    /// Undo a soft delete.
    pub fn restore_document(&self, by: &str, id: &str) -> Result<DocumentView> {
        self.tx(|tx| {
            let n =
                tx.execute("UPDATE documents SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL", [id])?;
            if n == 0 {
                return match tx.query_row("SELECT 1 FROM documents WHERE id = ?1", [id], |_| Ok(())).optional()? {
                    Some(()) => Err(StoreError::DocumentNotDeleted(id.to_string())),
                    None => Err(StoreError::DocumentNotFound(id.to_string())),
                };
            }
            let name = head_name(tx, id)?;
            append_event(tx, Some(by), "document.restored", Some(id), serde_json::json!({ "name": name }))?;
            Ok(())
        })?;
        Ok(self.document(id)?.expect("exists"))
    }

    /// Permanently remove a *deleted* Document and drop the blobs no other Version references.
    /// The rows go in one transaction; the files right after, under the same store guard.
    pub fn purge_document(&self, by: &str, id: &str) -> Result<PurgeOutcome> {
        self.purge_document_as(Some(by), id)
    }

    // ---- ticket 13 ----
    /// `purge_document` with an optional actor: `None` is the housekeeping ticker's auto-purge
    /// (event `document.purged {name, auto: true}` with no `by`).
    pub(crate) fn purge_document_as(&self, by: Option<&str>, id: &str) -> Result<PurgeOutcome> {
        // ---- end ticket 13 ----
        let mut conn = self.conn();
        let (name, candidates) = {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let deleted: Option<Option<String>> =
                tx.query_row("SELECT deleted_at FROM documents WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            match deleted {
                None => return Err(StoreError::DocumentNotFound(id.to_string())),
                Some(None) => return Err(StoreError::DocumentNotDeleted(id.to_string())),
                Some(Some(_)) => {}
            }
            let name = head_name(&tx, id)?;
            let mut candidates: BTreeSet<String> = BTreeSet::new();
            {
                let mut stmt = tx.prepare("SELECT md_hash, manifest, html_hash FROM versions WHERE doc_id = ?1")?;
                for row in stmt.query_map([id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
                })? {
                    let (md, manifest, html) = row?;
                    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap_or_default();
                    candidates.extend(version_blob_refs(&md, &manifest, html.as_deref()));
                }
            }
            tx.execute("DELETE FROM actions WHERE doc_id = ?1", [id])?; // ticket 05
            tx.execute("DELETE FROM thread_events WHERE doc_id = ?1", [id])?; // ticket 04
            tx.execute("DELETE FROM comments WHERE doc_id = ?1", [id])?; // ticket 04
            tx.execute("DELETE FROM threads WHERE doc_id = ?1", [id])?; // ticket 04
            tx.execute("DELETE FROM versions WHERE doc_id = ?1", [id])?;
            tx.execute("DELETE FROM documents WHERE id = ?1", [id])?;
            // ticket 13: an auto-purge (no actor) is marked `auto: true`.
            let payload = match by {
                Some(_) => serde_json::json!({ "name": name }),
                None => serde_json::json!({ "name": name, "auto": true }),
            };
            append_event(&tx, by, "document.purged", Some(id), payload)?;
            let still_referenced = referenced_hashes(&tx)?;
            for h in &still_referenced {
                candidates.remove(h);
            }
            tx.commit()?;
            (name, candidates)
        };
        self.fire_commit_hook(); // ticket 08: `document.purged` wakes long-polls like any event
        let mut removed = Vec::new();
        for h in candidates {
            if self.blobs.remove(&h)? {
                removed.push(h);
            }
        }
        Ok(PurgeOutcome { id: id.to_string(), name, blobs_removed: removed })
    }

    /// Sweep the blob store: remove every file no Version references, skipping files younger
    /// than `grace` (an upload whose commit is still on its way). Returns the hashes removed.
    /// Ticket 13's hourly ticker calls this.
    pub fn gc_blobs(&self, grace: Duration) -> Result<Vec<String>> {
        Ok(self.gc_sweep(grace)?.0)
    }

    // ---- ticket 13 ----
    /// The sweep behind [`Store::gc_blobs`], also counting the bytes reclaimed (the ticker's
    /// report). The referenced set is read in one transaction and the files removed right after
    /// under the same store guard, so a commit cannot slip in between.
    pub fn gc_sweep(&self, grace: Duration) -> Result<(Vec<String>, u64)> {
        let mut conn = self.conn();
        let referenced = {
            let tx = conn.transaction()?;
            let r = referenced_hashes(&tx)?;
            tx.commit()?;
            r
        };
        let cutoff = SystemTime::now().checked_sub(grace).unwrap_or(SystemTime::UNIX_EPOCH);
        let mut removed = Vec::new();
        let mut bytes = 0u64;
        for (hash, modified) in self.blobs.all()? {
            if referenced.contains(&hash) || modified > cutoff {
                continue;
            }
            let size = self.blobs.size(&hash)?.unwrap_or(0);
            if self.blobs.remove(&hash)? {
                removed.push(hash);
                bytes += size;
            }
        }
        Ok((removed, bytes))
    }
    // ---- end ticket 13 ----
}

fn head_name(tx: &Transaction, doc_id: &str) -> Result<String> {
    Ok(tx.query_row("SELECT name FROM versions WHERE doc_id = ?1 ORDER BY number DESC LIMIT 1", [doc_id], |r| {
        r.get(0)
    })?)
}

/// Every blob hash any Version still references.
pub(crate) fn referenced_hashes(tx: &Transaction) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let mut stmt = tx.prepare("SELECT md_hash, manifest, html_hash FROM versions")?;
    for row in
        stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?
    {
        let (md, manifest, html) = row?;
        let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap_or_default();
        out.extend(version_blob_refs(&md, &manifest, html.as_deref()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use annotary_sidecar::identity::{is_document_id, sha256_hex, version_hash};

    use super::*;
    use crate::store::{SetupInput, SetupOutcome};

    fn fresh() -> (tempfile::TempDir, Store, SetupOutcome) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap();
        (dir, store, out)
    }

    fn input(store: &Store, name: &str, md: &[u8]) -> PushInput {
        let (h, _) = store.put_blob(md).unwrap();
        PushInput::new(name, h)
    }

    /// Wait until `crate::time::now()` is past `since`, so the next mutation's millisecond
    /// timestamp is strictly greater (two operations can share a millisecond).
    fn after_millisecond(since: &str) {
        while crate::time::now().as_str() <= since {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // spec: S02-01, S02-11, S02-26, S02-27
    #[test]
    fn create_document_makes_v1_with_a_short_id_and_events() {
        let (_d, store, out) = fresh();
        let seq0 = store.last_seq().unwrap();
        let mut inp = input(&store, "plan", b"# Plan\n");
        inp.message = Some("first draft".into());
        let doc = store.create_document(&out.owner.id, &out.vault.id, inp).unwrap();
        assert!(is_document_id(&doc.id), "{}", doc.id);
        assert_eq!(doc.name(), "plan");
        assert_eq!(doc.head.number, 1);
        assert_eq!(doc.head.by_handle, "rowan");
        assert_eq!(doc.head.message.as_deref(), Some("first draft"));
        assert_eq!(doc.head.md_hash, sha256_hex(b"# Plan\n"));
        assert_eq!(
            doc.head.hash,
            version_hash(&doc.head.md_hash, []),
            "identity = sha256 over md hash + empty manifest"
        );
        assert_eq!(doc.vault_name, "notes");
        assert!(!doc.is_deleted());
        assert_eq!(doc.last_activity_at, doc.created_at);
        let events = store.events_since(seq0).unwrap();
        assert_eq!(
            events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec!["document.created", "version.pushed"]
        );
        assert_eq!(events[0].doc_id.as_deref(), Some(doc.id.as_str()));
        assert_eq!(events[1].payload["version"], 1);
        assert_eq!(events[1].payload["name"], "plan");
        assert!(store.read_blob(&doc.head.md_hash).unwrap().is_some());
        assert!(store.blobs().path(&doc.head.md_hash).starts_with(store.dir().join("blobs/sha256")));
    }

    // spec: S02-09, S02-10, S02-11, S02-27
    #[test]
    fn push_is_noop_on_head_and_derives_same_as_for_older_content() {
        let (_d, store, out) = fresh();
        let doc = store.create_document(&out.owner.id, &out.vault.id, input(&store, "plan", b"v1\n")).unwrap();
        let seq1 = store.last_seq().unwrap();
        // identical to head → already, no event, no activity change
        let again = store.push_version(&out.owner.id, &doc.id, input(&store, "plan", b"v1\n")).unwrap();
        assert!(again.already);
        assert_eq!(again.version.number, 1);
        assert_eq!(store.last_seq().unwrap(), seq1);
        // new content → v2, a millisecond after creation so activity strictly moves
        after_millisecond(&doc.created_at);
        let two = store.push_version(&out.owner.id, &doc.id, input(&store, "plan", b"v2\n")).unwrap();
        assert!(!two.already);
        assert_eq!(two.version.number, 2);
        assert_eq!(two.version.same_as, None);
        assert_eq!(store.last_seq().unwrap(), seq1 + 1);
        // identical to v1 → v3 (same content as v1), never a revert
        let three = store.push_version(&out.owner.id, &doc.id, input(&store, "plan", b"v1\n")).unwrap();
        assert_eq!(three.version.number, 3);
        assert_eq!(three.version.same_as, Some(1));
        let ev = store.events_since(seq1 + 1).unwrap();
        assert_eq!(ev[0].kind, "version.pushed");
        assert_eq!(ev[0].payload["same_as"], 1);
        // versions are immutable rows, ascending, all kept
        let vs = store.versions(&doc.id).unwrap();
        assert_eq!(vs.iter().map(|v| v.number).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(vs[0].hash, vs[2].hash);
        assert_eq!(vs.iter().map(|v| v.same_as).collect::<Vec<_>>(), vec![None, None, Some(1)]);
        let d = store.document(&doc.id).unwrap().unwrap();
        assert_eq!(d.head.number, 3);
        assert_eq!(d.head.same_as, Some(1));
        assert!(d.last_activity_at > d.created_at);
        // a name change is just a push under another name; the head's name is the Document's
        let four = store.push_version(&out.owner.id, &doc.id, input(&store, "Phase 1 — Plan", b"v4\n")).unwrap();
        assert_eq!(four.version.name, "Phase 1 — Plan");
        assert_eq!(store.document(&doc.id).unwrap().unwrap().name(), "Phase 1 — Plan");
        assert_eq!(store.versions(&doc.id).unwrap()[0].name, "plan", "names are recorded per Version");
    }

    // spec: S02-12, S17-08, S17-09
    #[test]
    fn a_moved_head_needs_the_heads_changes_or_a_discard() {
        use crate::concurrent::{HeadMovedError, HeadMovedReason};
        let (_d, store, out) = fresh();
        let (agent, _) = store.add_member(&out.owner.id, "scribe", None).unwrap();
        let doc = store.create_document(&out.owner.id, &out.vault.id, input(&store, "plan", b"a\nb\nc\n")).unwrap();
        store.push_version(&agent.id, &doc.id, input(&store, "plan", b"a\nB\nc\n")).unwrap();
        store.push_version(&agent.id, &doc.id, input(&store, "plan", b"a\nB\nC\n")).unwrap();
        let refused = |inp: PushInput| match store.push_version(&out.owner.id, &doc.id, inp) {
            Err(StoreError::HeadMoved(e)) => e,
            other => panic!("expected head_moved, got {other:?}"),
        };
        let seq = store.last_seq().unwrap();
        // Over a moved head without a claim ⇒ refused, nothing written.
        let mut inp = input(&store, "plan", b"A\nb\nc\n");
        inp.base_version = Some(1);
        assert_eq!(
            refused(inp.clone()),
            HeadMovedError {
                name: "plan".into(),
                base: 1,
                head: 3,
                by: vec!["scribe".into()],
                reason: HeadMovedReason::Unchecked
            }
        );
        // Claiming an older head ⇒ "moved again"; claiming the head while the file lacks its
        // changes ⇒ the recheck refuses (verify on).
        inp.contains_head = Some(2);
        assert_eq!(refused(inp.clone()).reason, HeadMovedReason::MovedAgain { checked: 2 });
        inp.contains_head = Some(3);
        assert_eq!(refused(inp.clone()).reason, HeadMovedReason::Missing);
        assert_eq!(store.last_seq().unwrap(), seq);
        assert_eq!(store.document(&doc.id).unwrap().unwrap().head.number, 3);
        // With verify off the claim is trusted (a hosted server may trust the edge).
        store.set_verify_edge(false);
        let mut trusted = input(&store, "plan", b"A\nb\nc\nd\n");
        trusted.base_version = Some(1);
        trusted.contains_head = Some(3);
        assert_eq!(store.push_version(&out.owner.id, &doc.id, trusted).unwrap().version.number, 4);
        store.set_verify_edge(true);
        // The file holds the head's changes ⇒ accepted, the head_moved report kept.
        let mut merged = input(&store, "plan", b"A\nB\nC\nd\ne\n");
        merged.base_version = Some(1);
        merged.contains_head = Some(4);
        let o = store.push_version(&out.owner.id, &doc.id, merged).unwrap();
        assert_eq!((o.version.number, o.discarded.clone()), (5, vec![]));
        assert_eq!(o.head_moved, Some(HeadMoved { base: 1, head: 4, by: vec!["scribe".into(), "rowan".into()] }));
        // Discard ⇒ accepted over the head; the event names the discarded Versions.
        let mut over = input(&store, "plan", b"mine\n");
        over.base_version = Some(3);
        over.discard_head_changes = true;
        let o = store.push_version(&out.owner.id, &doc.id, over).unwrap();
        assert_eq!((o.version.number, o.discarded), (6, vec![4, 5]));
        let ev = store
            .events_page(&crate::EventQuery { until: store.last_seq().unwrap(), limit: 1000, ..Default::default() })
            .unwrap();
        let last = &ev.last().unwrap().event;
        assert_eq!(
            (last.kind.as_str(), last.payload["discarded"].clone()),
            ("version.pushed", serde_json::json!([4, 5]))
        );
        // base == head ⇒ nothing to check; identical to head ⇒ `already`, no check either.
        let mut same = input(&store, "plan", b"mine2\n");
        same.base_version = Some(6);
        assert_eq!(store.push_version(&out.owner.id, &doc.id, same).unwrap().head_moved, None);
        let mut stale_same = input(&store, "plan", b"mine2\n");
        stale_same.base_version = Some(2);
        assert!(store.push_version(&out.owner.id, &doc.id, stale_same).unwrap().already);
    }

    // spec: S02-32, S02-05
    #[test]
    fn commit_needs_the_blob_and_a_valid_name() {
        let (_d, store, out) = fresh();
        let missing = PushInput::new("plan", sha256_hex(b"never uploaded"));
        assert!(matches!(
            store.create_document(&out.owner.id, &out.vault.id, missing),
            Err(StoreError::BlobMissing(_))
        ));
        assert!(matches!(
            store.create_document(&out.owner.id, &out.vault.id, input(&store, ".hidden", b"x")),
            Err(StoreError::InvalidName { .. })
        ));
        assert!(matches!(
            store.create_document(&out.owner.id, "no-such-vault", input(&store, "plan", b"x")),
            Err(StoreError::VaultNotFound(_))
        ));
        assert!(matches!(
            store.push_version(&out.owner.id, "zzzzzzzz", input(&store, "plan", b"x")),
            Err(StoreError::DocumentNotFound(_))
        ));
        assert_eq!(store.lacking_blobs(&[sha256_hex(b"x"), sha256_hex(b"nope")]), vec![sha256_hex(b"nope")]);
        assert!(matches!(store.put_blob_expecting("00", b"x"), Err(StoreError::BlobHashMismatch { .. })));
    }

    // spec: S02-17, S02-18, S02-27
    #[test]
    fn delete_restore_purge_and_blob_gc_keep_shared_blobs() {
        let (_d, store, out) = fresh();
        let a = store.create_document(&out.owner.id, &out.vault.id, input(&store, "a", b"shared\n")).unwrap();
        store.push_version(&out.owner.id, &a.id, input(&store, "a", b"only-a\n")).unwrap();
        let b = store.create_document(&out.owner.id, &out.vault.id, input(&store, "b", b"shared\n")).unwrap();
        let shared = sha256_hex(b"shared\n");
        let only_a = sha256_hex(b"only-a\n");

        // hidden by default, present with Only/Any
        let d = store.delete_document(&out.owner.id, &a.id).unwrap();
        assert!(d.is_deleted());
        assert!(matches!(store.delete_document(&out.owner.id, &a.id), Err(StoreError::DocumentDeleted(_))));
        let (live, _) = store.list_documents(&DocFilter::default()).unwrap();
        assert_eq!(live.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec![b.id.as_str()]);
        let (del, _) = store.list_documents(&DocFilter { deleted: DeletedFilter::Only, ..Default::default() }).unwrap();
        assert_eq!(del.len(), 1);
        let (any, _) = store.list_documents(&DocFilter { deleted: DeletedFilter::Any, ..Default::default() }).unwrap();
        assert_eq!(any.len(), 2);
        assert_eq!(store.versions(&a.id).unwrap().len(), 2, "versions retained");
        // pushing to a deleted Document is refused
        assert!(matches!(
            store.push_version(&out.owner.id, &a.id, input(&store, "a", b"z")),
            Err(StoreError::DocumentDeleted(_))
        ));
        // restore
        assert!(!store.restore_document(&out.owner.id, &a.id).unwrap().is_deleted());
        assert!(matches!(store.restore_document(&out.owner.id, &a.id), Err(StoreError::DocumentNotDeleted(_))));
        // purge needs a deleted Document
        assert!(matches!(store.purge_document(&out.owner.id, &a.id), Err(StoreError::DocumentNotDeleted(_))));
        store.delete_document(&out.owner.id, &a.id).unwrap();
        let seq = store.last_seq().unwrap();
        let purged = store.purge_document(&out.owner.id, &a.id).unwrap();
        assert_eq!(purged.name, "a");
        assert_eq!(purged.blobs_removed, vec![only_a.clone()], "the shared blob stays (b references it)");
        assert!(store.has_blob(&shared));
        assert!(!store.has_blob(&only_a));
        assert!(store.document(&a.id).unwrap().is_none());
        assert!(store.versions(&a.id).unwrap().is_empty());
        let ev = store.events_since(seq).unwrap();
        assert_eq!(ev.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["document.purged"]);
        assert!(matches!(store.purge_document(&out.owner.id, &a.id), Err(StoreError::DocumentNotFound(_))));
        // the sweep leaves referenced and fresh blobs alone
        store.put_blob(b"orphan").unwrap();
        assert!(store.gc_blobs(Duration::from_secs(3600)).unwrap().is_empty(), "young files survive the grace period");
        // (the refused push above left its uploaded blob `z` unreferenced too)
        let mut swept = store.gc_blobs(Duration::ZERO).unwrap();
        swept.sort();
        let mut expect = vec![sha256_hex(b"orphan"), sha256_hex(b"z")];
        expect.sort();
        assert_eq!(swept, expect);
        assert!(store.has_blob(&shared));
    }

    // spec: S02-20, S02-21, S02-06
    #[test]
    fn list_sorts_by_activity_filters_and_limits() {
        let (_d, store, out) = fresh();
        let a = store.create_document(&out.owner.id, &out.vault.id, input(&store, "plan", b"a")).unwrap();
        std::thread::sleep(Duration::from_millis(3));
        let b = store.create_document(&out.owner.id, &out.vault.id, input(&store, "plan", b"b")).unwrap();
        std::thread::sleep(Duration::from_millis(3));
        let c = store.create_document(&out.owner.id, &out.vault.id, input(&store, "Roadmap", b"c")).unwrap();
        // duplicate Names allowed; newest activity first
        let (all, total) = store.list_documents(&DocFilter::default()).unwrap();
        assert_eq!(total, 3);
        assert_eq!(
            all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            vec![c.id.as_str(), b.id.as_str(), a.id.as_str()]
        );
        // a push bumps activity
        std::thread::sleep(Duration::from_millis(3));
        store.push_version(&out.owner.id, &a.id, input(&store, "plan", b"a2")).unwrap();
        let (all, _) = store.list_documents(&DocFilter::default()).unwrap();
        assert_eq!(all[0].id, a.id);
        // limit + total
        let (two, total) = store.list_documents(&DocFilter { limit: Some(2), ..Default::default() }).unwrap();
        assert_eq!((two.len(), total), (2, 3));
        // substring (case-insensitive) and exact
        let (r, _) =
            store.list_documents(&DocFilter { name_contains: Some("ROAD".into()), ..Default::default() }).unwrap();
        assert_eq!(r.len(), 1);
        let (p, _) =
            store.list_documents(&DocFilter { name_exact: Some("plan".into()), ..Default::default() }).unwrap();
        assert_eq!(p.len(), 2, "exact Name lookup lists every duplicate");
        let (p, _) =
            store.list_documents(&DocFilter { name_exact: Some("Plan".into()), ..Default::default() }).unwrap();
        assert!(p.is_empty(), "exact is case-sensitive: Names are verbatim");
        // since
        let since = store.document(&a.id).unwrap().unwrap().last_activity_at;
        let (s, _) = store.list_documents(&DocFilter { since: Some(since), ..Default::default() }).unwrap();
        assert_eq!(s.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec![a.id.as_str()]);
        // vault filter
        let (v, _) = store.list_documents(&DocFilter { vault_ids: Some(vec![]), ..Default::default() }).unwrap();
        assert!(v.is_empty());
        let (v, _) = store
            .list_documents(&DocFilter { vault_ids: Some(vec![out.vault.id.clone()]), ..Default::default() })
            .unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(store.vault_by_name("notes").unwrap().unwrap().id, out.vault.id);
        assert!(store.vault_by_id("nope").unwrap().is_none());
    }
}
