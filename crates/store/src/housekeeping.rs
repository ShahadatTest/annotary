//! Housekeeping (ticket 13): the "purge deleted after N days" setting, auto-purge, and
//! derived-blob cleanup. The server's hourly ticker (`server/housekeeping.rs`) runs, in order,
//! [`Store::auto_purge`] → [`Store::gc_blobs`] → [`Store::clean_derived`]; an explicit `purge`
//! runs the GC too. Nothing here ever drops a blob a Version still references.
//!
//! - The setting lives in `settings` under [`AUTO_PURGE_KEY`] (absent / `0` = off — the default);
//!   `setting.auto_purge {days}` is its event.
//! - Auto-purge purges every deleted Document whose `deleted_at` is `days` or more days before
//!   `now` through the same path as `purge DOC` (`document.purged {name, auto: true}`, no actor).
//! - Derived-blob cleanup drops rendered HTML the server will never serve again (its recorded
//!   `renderer` is older than the minimum — the read path re-renders such Versions anyway, see
//!   ticket 09's `ensure_html`): the Version's `html_hash`/`renderer` are cleared in one
//!   transaction with a `housekeeping.cleaned {versions, html_blobs}` event, the blobs nothing
//!   else references are removed, and leftover temp files of interrupted blob writes are swept.
//!   Which renderers count as stale is the caller's predicate (the store does not depend on the
//!   renderer crate).

use std::collections::BTreeSet;
use std::time::Duration;

use jiff::Timestamp;
use rusqlite::{OptionalExtension, params};

use crate::docs::{PurgeOutcome, referenced_hashes};
use crate::error::{Result, StoreError};
use crate::store::{Store, append_event};
use crate::time;

/// `settings` key of the auto-purge setting (days as a decimal string; absent = off).
pub const AUTO_PURGE_KEY: &str = "auto_purge_days";

/// Largest accepted value (a century — anything bigger is a typo).
pub const AUTO_PURGE_MAX_DAYS: u32 = 36_500;

/// What [`Store::clean_derived`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DerivedCleanup {
    /// Versions whose stale rendered HTML was un-recorded.
    pub html_dropped: usize,
    /// HTML blobs removed because no Version references them any more.
    pub blobs_removed: Vec<String>,
    /// Bytes those blobs occupied.
    pub bytes: u64,
    /// Leftover temp files removed from the blob tree.
    pub temp_removed: usize,
}

impl Store {
    /// The auto-purge setting: `Some(days)` when on, `None` when off (the default).
    pub fn auto_purge_days(&self) -> Result<Option<u32>> {
        Ok(self.setting(AUTO_PURGE_KEY)?.and_then(|v| v.trim().parse::<u32>().ok()).filter(|d| *d > 0))
    }

    /// Turn auto-purge on (`Some(days)`, 1..=[`AUTO_PURGE_MAX_DAYS`]) or off (`None` / `Some(0)`).
    /// Returns whether the setting changed (an unchanged setting writes nothing, no event).
    pub fn set_auto_purge_days(&self, by: &str, days: Option<u32>) -> Result<bool> {
        let days = days.filter(|d| *d > 0);
        if let Some(d) = days
            && d > AUTO_PURGE_MAX_DAYS
        {
            return Err(StoreError::Invalid {
                what: "auto_purge_days",
                reason: format!("{d} is more than {AUTO_PURGE_MAX_DAYS} days"),
            });
        }
        if self.auto_purge_days()? == days {
            return Ok(false);
        }
        self.tx(|tx| {
            match days {
                Some(d) => {
                    tx.execute(
                        "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![AUTO_PURGE_KEY, d.to_string()],
                    )?;
                }
                None => {
                    tx.execute("DELETE FROM settings WHERE key = ?1", [AUTO_PURGE_KEY])?;
                }
            }
            append_event(tx, Some(by), "setting.auto_purge", None, serde_json::json!({ "days": days }))?;
            Ok(true)
        })
    }

    /// Deleted Documents due for auto-purge at `now`: `deleted_at` at least `days` days ago.
    /// `now` is injectable so the ticker is testable without waiting. Empty when the setting is off.
    pub fn auto_purge_due(&self, now: Timestamp) -> Result<Vec<String>> {
        let Some(days) = self.auto_purge_days()? else { return Ok(Vec::new()) };
        let cutoff =
            now.checked_sub(jiff::SignedDuration::from_secs(i64::from(days) * 86_400)).unwrap_or(Timestamp::MIN);
        // Stored timestamps share one fixed UTC format, so text order is time order.
        let cutoff = time::format(cutoff);
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id FROM documents WHERE deleted_at IS NOT NULL AND deleted_at <= ?1 ORDER BY deleted_at, id",
        )?;
        let ids = stmt.query_map([cutoff], |r| r.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Purge every Document [`Store::auto_purge_due`] lists (each through the `purge DOC` path:
    /// rows in one transaction, `document.purged {name, auto: true}`, then its blobs). A
    /// Document restored in between is skipped.
    pub fn auto_purge(&self, now: Timestamp) -> Result<Vec<PurgeOutcome>> {
        let mut out = Vec::new();
        for id in self.auto_purge_due(now)? {
            match self.purge_document_as(None, &id) {
                Ok(o) => out.push(o),
                Err(StoreError::DocumentNotFound(_)) | Err(StoreError::DocumentNotDeleted(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Derived-blob cleanup: un-record rendered HTML whose `renderer` `is_stale` (the read path
    /// never serves it — it re-renders), remove the HTML blobs nothing references any more, and
    /// sweep temp files older than `grace`. One transaction for the rows (with a
    /// `housekeeping.cleaned` event when anything was cleared); files right after, under the
    /// same store guard.
    pub fn clean_derived(&self, is_stale: &dyn Fn(Option<&str>) -> bool, grace: Duration) -> Result<DerivedCleanup> {
        let mut report = DerivedCleanup::default();
        let candidates = {
            let mut conn = self.conn();
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let rows: Vec<(String, i64, String, Option<String>)> = {
                let mut stmt =
                    tx.prepare("SELECT doc_id, number, html_hash, renderer FROM versions WHERE html_hash IS NOT NULL")?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>();
                rows?
            };
            let stale: Vec<_> = rows.into_iter().filter(|(_, _, _, renderer)| is_stale(renderer.as_deref())).collect();
            if stale.is_empty() {
                tx.commit()?;
                BTreeSet::new()
            } else {
                let mut candidates: BTreeSet<String> = BTreeSet::new();
                for (doc_id, number, html_hash, _) in &stale {
                    tx.execute(
                        "UPDATE versions SET html_hash = NULL, renderer = NULL WHERE doc_id = ?1 AND number = ?2",
                        params![doc_id, number],
                    )?;
                    candidates.insert(html_hash.clone());
                }
                append_event(
                    &tx,
                    None,
                    "housekeeping.cleaned",
                    None,
                    serde_json::json!({ "versions": stale.len(), "html_blobs": candidates.len() }),
                )?;
                let still_referenced = referenced_hashes(&tx)?;
                candidates.retain(|h| !still_referenced.contains(h));
                tx.commit()?;
                report.html_dropped = stale.len();
                // Files go under the same guard as the rows (like `purge_document`).
                for h in &candidates {
                    let size = self.blobs.size(h)?.unwrap_or(0);
                    if self.blobs.remove(h)? {
                        report.blobs_removed.push(h.clone());
                        report.bytes += size;
                    }
                }
                candidates
            }
        };
        if !candidates.is_empty() || report.html_dropped > 0 {
            self.fire_commit_hook();
        }
        report.temp_removed = self.blobs.sweep_temp(grace)?;
        Ok(report)
    }

    /// Where a blob lives on disk (`store/blobs/sha256/ab/<hash>`, no existence check) — tests
    /// age files to cross the GC grace period.
    pub fn blob_path(&self, hash: &str) -> std::path::PathBuf {
        self.blobs.path(hash)
    }

    /// A Version's recorded render fields (`html_hash`, `renderer`) — what cleanup clears.
    pub fn version_render_fields(&self, doc_id: &str, number: u32) -> Result<Option<(Option<String>, Option<String>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT html_hash, renderer FROM versions WHERE doc_id = ?1 AND number = ?2",
                params![doc_id, number as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PushInput;
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

    fn days_ago(n: i64) -> Timestamp {
        Timestamp::now().checked_sub(jiff::SignedDuration::from_secs(n * 86_400)).unwrap()
    }

    // spec: S13-07, S13-08
    #[test]
    fn auto_purge_setting_is_off_by_default_validated_and_evented() {
        let (_d, store, out) = fresh();
        assert_eq!(store.auto_purge_days().unwrap(), None);
        let seq = store.last_seq().unwrap();
        assert!(store.set_auto_purge_days(&out.owner.id, Some(30)).unwrap());
        assert_eq!(store.auto_purge_days().unwrap(), Some(30));
        assert!(!store.set_auto_purge_days(&out.owner.id, Some(30)).unwrap(), "unchanged ⇒ no write");
        assert!(store.set_auto_purge_days(&out.owner.id, Some(0)).unwrap(), "0 = off");
        assert_eq!(store.auto_purge_days().unwrap(), None);
        assert!(!store.set_auto_purge_days(&out.owner.id, None).unwrap());
        assert!(matches!(
            store.set_auto_purge_days(&out.owner.id, Some(AUTO_PURGE_MAX_DAYS + 1)),
            Err(StoreError::Invalid { what: "auto_purge_days", .. })
        ));
        let ev = store.events_since(seq).unwrap();
        assert_eq!(ev.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["setting.auto_purge"; 2]);
        assert_eq!(ev[0].payload["days"], 30);
        assert!(ev[1].payload["days"].is_null());
    }

    // spec: S13-08, S13-10
    #[test]
    fn auto_purge_respects_the_setting_and_n_days_and_keeps_shared_blobs() {
        let (_d, store, out) = fresh();
        let shared = b"# shared\n";
        let old = store.create_document(&out.owner.id, &out.vault.id, input(&store, "old", shared)).unwrap();
        store.push_version(&out.owner.id, &old.id, input(&store, "old", b"# old only\n")).unwrap();
        let fresh_doc =
            store.create_document(&out.owner.id, &out.vault.id, input(&store, "fresh", b"# fresh\n")).unwrap();
        let keeper = store.create_document(&out.owner.id, &out.vault.id, input(&store, "keeper", shared)).unwrap();
        let live = store.create_document(&out.owner.id, &out.vault.id, input(&store, "live", b"# live\n")).unwrap();
        store.delete_document(&out.owner.id, &old.id).unwrap();
        store.delete_document(&out.owner.id, &fresh_doc.id).unwrap();
        // Backdate `old`'s deletion by 40 days.
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE documents SET deleted_at = ?1 WHERE id = ?2",
                params![time::format(days_ago(40)), old.id],
            )
            .unwrap();
        }
        // Off ⇒ nothing is due, nothing purged, even at a far-future `now`.
        assert!(store.auto_purge_due(days_ago(-10_000)).unwrap().is_empty());
        assert!(store.auto_purge(days_ago(-10_000)).unwrap().is_empty());
        // On with N = 30: only `old` (40 d) is due; `fresh` (just now) and live Documents are not.
        store.set_auto_purge_days(&out.owner.id, Some(30)).unwrap();
        assert_eq!(store.auto_purge_due(Timestamp::now()).unwrap(), vec![old.id.clone()]);
        // N = 60: nothing is due yet.
        store.set_auto_purge_days(&out.owner.id, Some(60)).unwrap();
        assert!(store.auto_purge_due(Timestamp::now()).unwrap().is_empty());
        store.set_auto_purge_days(&out.owner.id, Some(30)).unwrap();
        let seq = store.last_seq().unwrap();
        let purged = store.auto_purge(Timestamp::now()).unwrap();
        assert_eq!(purged.len(), 1);
        assert_eq!(purged[0].id, old.id);
        assert_eq!(purged[0].name, "old");
        // The blob shared with `keeper` stays; `old`'s own second Version blob goes.
        let shared_hash = annotary_sidecar::identity::sha256_hex(shared);
        assert!(store.has_blob(&shared_hash), "a blob another Document references is never dropped");
        assert_eq!(purged[0].blobs_removed, vec![annotary_sidecar::identity::sha256_hex(b"# old only\n")]);
        assert!(store.document(&old.id).unwrap().is_none());
        assert!(store.document(&fresh_doc.id).unwrap().unwrap().is_deleted(), "too young to purge");
        assert!(store.document(&keeper.id).unwrap().is_some());
        assert!(store.document(&live.id).unwrap().is_some());
        let ev = store.events_since(seq).unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].kind, "document.purged");
        assert_eq!(ev[0].by, None, "auto-purge has no actor");
        assert_eq!(ev[0].payload["auto"], true);
        assert_eq!(ev[0].payload["name"], "old");
        // A second run is a no-op.
        assert!(store.auto_purge(Timestamp::now()).unwrap().is_empty());
        // Restored in between ⇒ skipped: backdate `fresh`, then restore it before the run.
        {
            let conn = store.conn();
            conn.execute(
                "UPDATE documents SET deleted_at = ?1 WHERE id = ?2",
                params![time::format(days_ago(40)), fresh_doc.id],
            )
            .unwrap();
        }
        assert_eq!(store.auto_purge_due(Timestamp::now()).unwrap(), vec![fresh_doc.id.clone()]);
        store.restore_document(&out.owner.id, &fresh_doc.id).unwrap();
        assert!(store.auto_purge(Timestamp::now()).unwrap().is_empty());
        assert!(store.document(&fresh_doc.id).unwrap().is_some());
    }

    // spec: S13-09, S13-11
    #[test]
    fn clean_derived_drops_stale_html_keeps_current_and_shared_and_sweeps_temp_files() {
        let (_d, store, out) = fresh();
        let a = store.create_document(&out.owner.id, &out.vault.id, input(&store, "a", b"# a\n")).unwrap();
        let b = store.create_document(&out.owner.id, &out.vault.id, input(&store, "b", b"# b\n")).unwrap();
        let c = store.create_document(&out.owner.id, &out.vault.id, input(&store, "c", b"# b\n")).unwrap();
        let (html_a, _) = store.put_blob(b"<h1 data-l=\"1-1\">a</h1>\n").unwrap();
        let (html_b, _) = store.put_blob(b"<h1 data-l=\"1-1\">b</h1>\n").unwrap();
        let (html_c, _) = store.put_blob(b"<h1 data-l=\"1-1\">c</h1>\n").unwrap();
        store.set_version_html(&out.owner.id, &a.id, 1, &html_a, "0.9.0").unwrap(); // stale
        store.set_version_html(&out.owner.id, &b.id, 1, &html_b, "0.9.0").unwrap(); // stale, but...
        store.set_version_html(&out.owner.id, &c.id, 1, &html_b, "1.0.0").unwrap(); // ...shared with a current one
        let d = store.create_document(&out.owner.id, &out.vault.id, input(&store, "d", b"# d\n")).unwrap();
        store.set_version_html(&out.owner.id, &d.id, 1, &html_c, "1.0.0").unwrap(); // current
        // A leftover temp file from an interrupted write, old enough to sweep, and a fresh one.
        let shard = store.blobs.path(&html_a).parent().unwrap().to_path_buf();
        let old_tmp = shard.join(".deadbeef.1.tmp");
        std::fs::write(&old_tmp, b"partial").unwrap();
        let fresh_tmp = shard.join(".cafe.2.tmp");
        std::fs::write(&fresh_tmp, b"partial").unwrap();
        let long_ago = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        std::fs::File::open(&old_tmp).unwrap().set_modified(long_ago).unwrap();
        let is_stale = |r: Option<&str>| r != Some("1.0.0");
        let seq = store.last_seq().unwrap();
        let rep = store.clean_derived(&is_stale, Duration::from_secs(3600)).unwrap();
        assert_eq!(rep.html_dropped, 2, "a and b");
        assert_eq!(rep.blobs_removed, vec![html_a.clone()], "b's blob is still referenced by c");
        assert!(rep.bytes > 0);
        assert_eq!(rep.temp_removed, 1);
        assert!(!old_tmp.exists());
        assert!(fresh_tmp.exists(), "a temp file younger than the grace period survives");
        assert!(!store.has_blob(&html_a));
        assert!(store.has_blob(&html_b));
        assert!(store.has_blob(&html_c));
        assert_eq!(store.version_render_fields(&a.id, 1).unwrap(), Some((None, None)));
        assert_eq!(store.version_render_fields(&b.id, 1).unwrap(), Some((None, None)));
        assert_eq!(store.version_render_fields(&c.id, 1).unwrap(), Some((Some(html_b.clone()), Some("1.0.0".into()))));
        assert_eq!(store.version_render_fields(&d.id, 1).unwrap(), Some((Some(html_c.clone()), Some("1.0.0".into()))));
        let ev = store.events_since(seq).unwrap();
        assert_eq!(ev.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), vec!["housekeeping.cleaned"]);
        assert_eq!(ev[0].payload["versions"], 2);
        assert_eq!(ev[0].payload["html_blobs"], 2);
        // Nothing stale left ⇒ nothing happens, no event.
        let seq = store.last_seq().unwrap();
        let rep = store.clean_derived(&is_stale, Duration::from_secs(3600)).unwrap();
        assert_eq!(rep, DerivedCleanup::default());
        assert_eq!(store.last_seq().unwrap(), seq);
        // The Markdown and current HTML blobs all survive a full GC too.
        assert!(store.gc_blobs(Duration::ZERO).unwrap().is_empty());
    }
}
