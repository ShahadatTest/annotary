//! `annotary export FILE.tar` (ticket 13): a consistent tarball of `store/`.
//!
//! The archive holds `store/meta.json`, `store/annotary.sqlite` — a snapshot taken with
//! SQLite's online backup API (one `step(-1)` under a read transaction, so a running server can
//! keep writing and the copy is still a single consistent state; no LOCK is taken) — and every
//! blob file `store/blobs/sha256/ab/<hash>`. `LOCK`, `-wal`/`-shm` and temp files stay out.
//! The snapshot is taken *before* the blobs are copied, so anything committed during the export
//! is simply absent from the snapshot (and its blobs are harmless extras). A purge, GC or
//! derived-blob cleanup that runs during the export — in the server, another process — can
//! remove a blob the snapshot still references, so (CMD-9) the export reads the snapshot's
//! referenced set (`docs::referenced_hashes`, the GC's own definition) and refuses with
//! [`StoreError::ExportIncomplete`] when any of them did not make it into the archive: an export
//! is complete or it fails, never silently short. The caller discards what was written and may
//! retry — a fresh snapshot no longer references what the purge removed (the purge commits its
//! rows before it removes the files). Restoring = untar into a fresh data dir and `serve` there
//! (`store/` is the whole state).

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::blobs::is_blob_hash;
use crate::error::{Result, StoreError};
use crate::meta::Meta;

/// Path prefix of every entry in the archive.
pub const TAR_PREFIX: &str = "store";

/// What [`export_tar`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportReport {
    /// Blob files archived.
    pub blobs: usize,
    /// Bytes of content archived (database snapshot + meta.json + blobs; not the tar overhead).
    pub bytes: u64,
    /// `PRAGMA user_version` of the snapshot.
    pub schema_version: u32,
    /// The Tenant id from `meta.json` (None on an unset-up store).
    pub tenant_id: Option<String>,
}

/// Whether `store_dir` holds a store (`annotary.sqlite` exists).
pub fn has_store(store_dir: &Path) -> bool {
    store_dir.join(crate::DB_FILE).is_file()
}

/// Copy the database at `store_dir/annotary.sqlite` to `dest` as one consistent snapshot
/// (SQLite online backup API, read-only source connection; a live writer is fine).
pub fn snapshot_db(store_dir: &Path, dest: &Path) -> Result<u32> {
    let src_path = store_dir.join(crate::DB_FILE);
    if !src_path.is_file() {
        return Err(StoreError::Invalid { what: "store", reason: format!("no store at {}", store_dir.display()) });
    }
    let src =
        Connection::open_with_flags(&src_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    src.busy_timeout(std::time::Duration::from_secs(10))?;
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    let mut dst = Connection::open(dest)?;
    {
        let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
        // -1 = every page in one step, under one read transaction: the copy is consistent.
        let done = backup.step(-1)?;
        if !matches!(done, rusqlite::backup::StepResult::Done) {
            return Err(StoreError::Invalid { what: "store", reason: format!("snapshot did not complete: {done:?}") });
        }
    }
    let version = crate::migrations::current_version(&dst)?;
    drop(dst);
    // The backup may leave a rollback journal of the destination behind; nothing needs it.
    let _ = std::fs::remove_file(PathBuf::from(format!("{}-journal", dest.display())));
    Ok(version)
}

/// Every blob file under `store_dir/blobs`, as `(hash, path)`, sorted.
pub fn blob_files(store_dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let sha = store_dir.join(crate::blobs::BLOBS_DIR).join("sha256");
    let Ok(shards) = std::fs::read_dir(&sha) else { return Ok(out) };
    for shard in shards.flatten() {
        if !shard.path().is_dir() {
            continue;
        }
        for f in std::fs::read_dir(shard.path())?.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if is_blob_hash(&name) && f.path().is_file() {
                out.push((name, f.path()));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Blob hashes the database snapshot at `db` references (every Version's Markdown, Assets and
/// rendered HTML — `docs::referenced_hashes`, what the GC keeps).
pub fn snapshot_references(db: &Path) -> Result<BTreeSet<String>> {
    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let tx = conn.transaction()?;
    crate::docs::referenced_hashes(&tx)
}

/// Write the tarball of `store_dir` to `out`. Fails with [`StoreError::ExportIncomplete`] (after
/// writing to `out`, which the caller then discards) when a blob the snapshot references was
/// not there to copy.
pub fn export_tar<W: Write>(store_dir: &Path, out: W) -> Result<ExportReport> {
    export_tar_with(store_dir, out, &mut || {})
}

/// [`export_tar`] with a seam between the database snapshot and the blob copy (tests run a
/// purge / GC there — the interleaving CMD-9 is about).
fn export_tar_with<W: Write>(store_dir: &Path, out: W, after_snapshot: &mut dyn FnMut()) -> Result<ExportReport> {
    if !has_store(store_dir) {
        return Err(StoreError::Invalid { what: "store", reason: format!("no store at {}", store_dir.display()) });
    }
    let mut report = ExportReport::default();
    let tmp = tempfile::Builder::new().prefix("annotary-export-").tempdir()?;
    let snapshot = tmp.path().join(crate::DB_FILE);
    report.schema_version = snapshot_db(store_dir, &snapshot)?;
    report.tenant_id = Meta::read(store_dir)?.and_then(|m| m.tenant_id);
    // CMD-9: every blob the snapshot references must reach the archive.
    let mut unwritten = snapshot_references(&snapshot)?;
    after_snapshot();

    let mut tar = tar::Builder::new(out);
    tar.mode(tar::HeaderMode::Deterministic);
    let meta_path = store_dir.join(crate::META_FILE);
    if meta_path.is_file() {
        report.bytes += append_file(&mut tar, &meta_path, &format!("{TAR_PREFIX}/{}", crate::META_FILE))?;
    }
    report.bytes += append_file(&mut tar, &snapshot, &format!("{TAR_PREFIX}/{}", crate::DB_FILE))?;
    for (hash, path) in blob_files(store_dir)? {
        let name = format!("{TAR_PREFIX}/{}/sha256/{}/{hash}", crate::blobs::BLOBS_DIR, &hash[..2]);
        match append_file(&mut tar, &path, &name) {
            Ok(n) => {
                report.bytes += n;
                report.blobs += 1;
                unwritten.remove(&hash);
            }
            // An unreferenced blob GC'd between the listing and the copy is harmless; a
            // referenced one stays in `unwritten` and fails the export below.
            Err(StoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    if !unwritten.is_empty() {
        return Err(StoreError::ExportIncomplete { missing: unwritten.into_iter().collect() });
    }
    let mut out = tar.into_inner()?;
    out.flush()?;
    Ok(report)
}

fn append_file<W: Write>(tar: &mut tar::Builder<W>, path: &Path, name: &str) -> Result<u64> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut header = tar::Header::new_gnu();
    header.set_size(len);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::Regular);
    tar.append_data(&mut header, name, &mut file)?;
    Ok(len)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::model::PushInput;
    use crate::store::{SetupInput, Store};

    // spec: S13-06
    #[test]
    fn export_holds_meta_snapshot_and_every_blob_and_restores_to_a_working_store() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("store");
        let store = Store::open(&store_dir).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None })
            .unwrap();
        let (h1, _) = store.put_blob(b"# one\n").unwrap();
        let doc = store.create_document(&out.owner.id, &out.vault.id, PushInput::new("one", h1.clone())).unwrap();
        let (h2, _) = store.put_blob(b"# two\n").unwrap();
        store.push_version(&out.owner.id, &doc.id, PushInput::new("one", h2.clone())).unwrap();
        // A temp file and the LOCK must not travel.
        std::fs::write(store.blobs.path(&h1).parent().unwrap().join(".x.1.tmp"), b"partial").unwrap();

        // Export while the store is open (LOCK held, WAL live) — the snapshot must still be consistent.
        let mut tar_bytes = Vec::new();
        let report = export_tar(&store_dir, &mut tar_bytes).unwrap();
        assert_eq!(report.blobs, 2);
        assert_eq!(report.schema_version, crate::SCHEMA_VERSION);
        assert_eq!(report.tenant_id, out.tenant_id.clone().into());
        assert!(report.bytes > 0);

        let names: Vec<String> = tar::Archive::new(&tar_bytes[..])
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names[0], "store/meta.json");
        assert_eq!(names[1], "store/annotary.sqlite");
        let mut blobs: Vec<String> =
            names.iter().filter(|n| n.starts_with("store/blobs/")).cloned().collect::<Vec<_>>();
        blobs.sort();
        let mut expect =
            vec![format!("store/blobs/sha256/{}/{h1}", &h1[..2]), format!("store/blobs/sha256/{}/{h2}", &h2[..2])];
        expect.sort();
        assert_eq!(blobs, expect);
        assert!(names.iter().all(|n| !n.contains("LOCK") && !n.contains(".tmp") && !n.contains("-wal")));

        // Restore into a fresh data dir and open it: the Document and both blobs are there.
        let restore = tempfile::tempdir().unwrap();
        tar::Archive::new(&tar_bytes[..]).unpack(restore.path()).unwrap();
        let restored = Store::open(&restore.path().join("store")).unwrap();
        assert!(restored.is_set_up().unwrap());
        assert_eq!(restored.tenant_id().unwrap(), out.tenant_id.clone().into());
        let d = restored.document(&doc.id).unwrap().expect("document restored");
        assert_eq!(d.head.number, 2);
        assert_eq!(restored.read_blob(&h1).unwrap().unwrap(), b"# one\n");
        assert_eq!(restored.read_blob(&h2).unwrap().unwrap(), b"# two\n");
        assert_eq!(restored.list_members().unwrap().len(), 1);
        // A store dir without a database is refused.
        let empty = tempfile::tempdir().unwrap();
        assert!(matches!(export_tar(empty.path(), Vec::new()), Err(StoreError::Invalid { what: "store", .. })));
    }

    /// The blob entries (`store/blobs/sha256/ab/<hash>` → hash) of a tar.
    fn archived_blobs(tar_bytes: &[u8]) -> BTreeSet<String> {
        tar::Archive::new(tar_bytes)
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string())
            .filter_map(|n| n.strip_prefix("store/blobs/sha256/").map(|r| r[3..].to_string()))
            .collect()
    }

    /// The blobs the archived database snapshot references.
    fn archived_references(tar_bytes: &[u8]) -> BTreeSet<String> {
        let dir = tempfile::tempdir().unwrap();
        tar::Archive::new(tar_bytes).unpack(dir.path()).unwrap();
        snapshot_references(&dir.path().join("store").join(crate::DB_FILE)).unwrap()
    }

    // spec: S21-01
    #[test]
    fn export_racing_a_purge_and_gc_is_complete_or_fails_never_silently_short() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("store");
        let store = Store::open(&store_dir).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None })
            .unwrap();
        let (keep, _) = store.put_blob(b"# keep\n").unwrap();
        store.create_document(&out.owner.id, &out.vault.id, PushInput::new("keep", keep.clone())).unwrap();
        let (gone, _) = store.put_blob(b"# gone\n").unwrap();
        let doc = store.create_document(&out.owner.id, &out.vault.id, PushInput::new("gone", gone.clone())).unwrap();
        store.delete_document(&out.owner.id, &doc.id).unwrap();

        // The interleaving: the snapshot still lists the deleted Document's Version, then — before
        // the blobs are copied — the server purges it (rows, then its blob) and runs a GC sweep.
        let mut tar_bytes = Vec::new();
        let result = export_tar_with(&store_dir, &mut tar_bytes, &mut || {
            let purged = store.purge_document(&out.owner.id, &doc.id).unwrap();
            assert_eq!(purged.blobs_removed, vec![gone.clone()]);
            store.gc_sweep(Duration::ZERO).unwrap();
        });
        match result {
            // A complete archive would be fine: every blob its database references is in it.
            Ok(_) => {
                let missing: Vec<_> =
                    archived_references(&tar_bytes).difference(&archived_blobs(&tar_bytes)).cloned().collect();
                assert!(missing.is_empty(), "export wrote an archive missing referenced blobs {missing:?}");
            }
            Err(StoreError::ExportIncomplete { missing }) => assert_eq!(missing, vec![gone.clone()]),
            Err(e) => panic!("unexpected error {e}"),
        }

        // Re-running after the purge: the fresh snapshot no longer references the purged blob,
        // so the export is complete.
        let mut tar_bytes = Vec::new();
        let report = export_tar(&store_dir, &mut tar_bytes).unwrap();
        assert_eq!(report.blobs, 1);
        assert_eq!(archived_blobs(&tar_bytes), BTreeSet::from([keep.clone()]));
        assert_eq!(archived_references(&tar_bytes), BTreeSet::from([keep]));
    }
}
