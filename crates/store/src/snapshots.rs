//! Rotating consistent snapshots of the store under `DATA_DIR/snapshots/` (CMD-1).
//!
//! The live `store/annotary.sqlite` is in WAL mode and held open by the server, so a backup
//! that copies the data dir file by file (an hourly restic run, say) can
//! read the database and its `-wal` at different moments. The server therefore writes an
//! [`export_tar`] of the store into `snapshots/` on a schedule (`server/snapshots.rs`): a file
//! copy of a finished tar is always consistent, and restoring is the same as for
//! `annotary export` (untar into a fresh data dir, `serve --data-dir` there).
//!
//! - Name: `annotary-YYYYMMDDTHHMMSSZ-sN.tar`, `N` = the store's last event `seq` read before
//!   the snapshot (every mutation appends an event, so an unchanged `seq` means unchanged
//!   content). Names sort chronologically.
//! - A tick whose `seq` equals the newest snapshot's writes nothing and only sets that file's
//!   mtime to now, so "how old is the newest snapshot" (the health check) reads as "when was the
//!   store last checked", and an idle store is not re-copied every interval.
//! - Written to `.annotary-….tar.tmp` and renamed, so a half-written tar never carries the
//!   final name; leftover temp files are removed on the next write. The folder is `0700` and the
//!   files `0600` (they hold the whole store, password and Token hashes included).
//! - After a write, all but the newest `keep` snapshots are removed.

use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crate::backup::export_tar;
use crate::error::Result;
use crate::{ExportReport, StoreError};

/// The folder under the data dir that holds the snapshots.
pub const SNAPSHOTS_DIR: &str = "snapshots";
const PREFIX: &str = "annotary-";
const SUFFIX: &str = ".tar";

/// One snapshot file in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotFile {
    pub path: PathBuf,
    /// `YYYYMMDDTHHMMSSZ` (UTC) from the name.
    pub stamp: String,
    /// The store's last event `seq` when the snapshot was taken.
    pub seq: i64,
}

/// What [`take_snapshot`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotOutcome {
    /// A new snapshot was written; `removed` are the old ones rotated out.
    Written { file: SnapshotFile, report: ExportReport, removed: Vec<PathBuf> },
    /// Nothing changed since the newest snapshot; its mtime was refreshed.
    Unchanged { file: SnapshotFile },
}

/// The file name for a snapshot taken at `now` with last event `seq`.
pub fn snapshot_name(now: jiff::Timestamp, seq: i64) -> String {
    format!("{PREFIX}{}-s{seq}{SUFFIX}", now.strftime("%Y%m%dT%H%M%SZ"))
}

/// `(stamp, seq)` of a snapshot file name, or `None` for anything else.
pub fn parse_snapshot_name(name: &str) -> Option<(String, i64)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (stamp, seq) = rest.split_once("-s")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let ok = stamp.len() == 16
        && digits(&stamp[..8])
        && &stamp[8..9] == "T"
        && digits(&stamp[9..15])
        && &stamp[15..] == "Z"
        && digits(seq);
    if !ok {
        return None;
    }
    Some((stamp.to_string(), seq.parse().ok()?))
}

/// Every snapshot in `dir`, oldest first (an absent folder is empty).
pub fn list_snapshots(dir: &Path) -> Result<Vec<SnapshotFile>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some((stamp, seq)) = parse_snapshot_name(&name)
            && entry.path().is_file()
        {
            out.push(SnapshotFile { path: entry.path(), stamp, seq });
        }
    }
    out.sort_by(|a, b| (&a.stamp, a.seq).cmp(&(&b.stamp, b.seq)));
    Ok(out)
}

/// Snapshot `store_dir` into `dir` unless the newest snapshot already has `seq`; keep the newest
/// `keep` (at least 1). `seq` is the store's last event `seq`, read *before* calling this, so a
/// write that lands during the snapshot makes the next tick take another one.
pub fn take_snapshot(
    store_dir: &Path,
    dir: &Path,
    seq: i64,
    now: jiff::Timestamp,
    keep: usize,
) -> Result<SnapshotOutcome> {
    create_private_dir(dir)?;
    let existing = list_snapshots(dir)?;
    if let Some(newest) = existing.last()
        && newest.seq == seq
    {
        File::options().write(true).open(&newest.path)?.set_modified(now_system(now))?;
        return Ok(SnapshotOutcome::Unchanged { file: newest.clone() });
    }
    remove_temp_files(dir);

    let name = snapshot_name(now, seq);
    let path = dir.join(&name);
    let tmp = dir.join(format!(".{name}.tmp"));
    let report = match write_tar(store_dir, &tmp) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    std::fs::rename(&tmp, &path)?;
    let (stamp, seq) = parse_snapshot_name(&name).expect("snapshot_name round-trips");
    let file = SnapshotFile { path, stamp, seq };

    let mut all = list_snapshots(dir)?;
    let keep = keep.max(1);
    let mut removed = Vec::new();
    if all.len() > keep {
        for old in all.drain(..all.len() - keep) {
            if old.path != file.path {
                std::fs::remove_file(&old.path)?;
                removed.push(old.path);
            }
        }
    }
    Ok(SnapshotOutcome::Written { file, report, removed })
}

fn write_tar(store_dir: &Path, tmp: &Path) -> Result<ExportReport> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let file = opts.open(tmp)?;
    let mut out = BufWriter::new(file);
    let report = export_tar(store_dir, &mut out)?;
    let file = out.into_inner().map_err(|e| StoreError::Io(e.into_error()))?;
    file.sync_all()?;
    Ok(report)
}

/// Create `dir` `0700`, or bring an existing one (made by hand, say) back to `0700`.
fn create_private_dir(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(dir)?.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Leftover `.annotary-….tar.tmp` files of an interrupted write.
fn remove_temp_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&format!(".{PREFIX}")) && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn now_system(now: jiff::Timestamp) -> std::time::SystemTime {
    std::time::SystemTime::from(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PushInput;
    use crate::store::{SetupInput, Store};

    fn ts(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    struct Fixture {
        store: Store,
        owner: String,
        vault: String,
    }

    fn setup(dir: &Path) -> Fixture {
        let store = Store::open(&dir.join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None })
            .unwrap();
        Fixture { owner: out.owner.id.clone(), vault: out.vault.id.clone(), store }
    }

    /// A new Document; its id.
    fn push(f: &Fixture, name: &str, md: &str) -> String {
        let (h, _) = f.store.put_blob(md.as_bytes()).unwrap();
        f.store.create_document(&f.owner, &f.vault, PushInput::new(name, h)).unwrap().id
    }

    #[test]
    fn names_round_trip_and_sort_chronologically() {
        let n = snapshot_name(ts("2026-10-21T00:15:07Z"), 42);
        assert_eq!(n, "annotary-20261021T001507Z-s42.tar");
        assert_eq!(parse_snapshot_name(&n), Some(("20261021T001507Z".into(), 42)));
        for bad in [
            "annotary-20261021T001507Z-s42.tar.tmp",
            ".annotary-20261021T001507Z-s42.tar.tmp",
            "annotary-20261021T001507Z.tar",
            "annotary-2026102T001507Z-s4.tar",
            "annotary-20261021T001507Z-s.tar",
            "annotary-20261021X001507Z-s4.tar",
            "other.tar",
        ] {
            assert_eq!(parse_snapshot_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_snapshot_is_a_consistent_store_that_reopens_with_every_document() {
        let dir = tempfile::tempdir().unwrap();
        let f = setup(dir.path());
        let store = &f.store;
        let plan = push(&f, "plan", "# Plan\n\nhello\n");
        let snaps = dir.path().join(SNAPSHOTS_DIR);
        let seq = store.last_seq().unwrap();
        let out = take_snapshot(store.dir(), &snaps, seq, ts("2026-10-21T00:00:00Z"), 4).unwrap();
        let SnapshotOutcome::Written { file, report, removed } = out else { panic!("expected a write: {out:?}") };
        assert!(removed.is_empty());
        assert_eq!(file.seq, seq);
        assert_eq!(report.schema_version, crate::SCHEMA_VERSION);

        // The store keeps writing after the snapshot; the snapshot does not see it.
        let later = push(&f, "later", "# Later\n");

        // Restore drill in miniature: untar into a fresh data dir and open the store there.
        let restored = tempfile::tempdir().unwrap();
        tar::Archive::new(File::open(&file.path).unwrap()).unpack(restored.path()).unwrap();
        let back = Store::open(&restored.path().join("store")).unwrap();
        assert!(back.document(&plan).unwrap().is_some(), "the Document pushed before the snapshot is there");
        assert!(back.document(&later).unwrap().is_none(), "the one pushed after it is not");
        assert_eq!(back.last_seq().unwrap(), seq);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&snaps).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&file.path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn an_unchanged_store_is_not_copied_again_only_its_newest_snapshot_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let f = setup(dir.path());
        let store = &f.store;
        let snaps = dir.path().join(SNAPSHOTS_DIR);
        let seq = store.last_seq().unwrap();
        let first = take_snapshot(store.dir(), &snaps, seq, ts("2026-10-21T00:00:00Z"), 4).unwrap();
        let SnapshotOutcome::Written { file, .. } = first else { panic!() };
        let later = ts("2026-10-21T00:15:00Z");
        let again = take_snapshot(store.dir(), &snaps, seq, later, 4).unwrap();
        assert_eq!(again, SnapshotOutcome::Unchanged { file: file.clone() });
        assert_eq!(list_snapshots(&snaps).unwrap().len(), 1);
        let mtime = std::fs::metadata(&file.path).unwrap().modified().unwrap();
        assert_eq!(mtime, now_system(later), "the newest snapshot's mtime says when the store was last checked");
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_open_snapshots_folder_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = setup(dir.path());
        let snaps = dir.path().join(SNAPSHOTS_DIR);
        std::fs::create_dir(&snaps).unwrap();
        std::fs::set_permissions(&snaps, std::fs::Permissions::from_mode(0o755)).unwrap();
        take_snapshot(f.store.dir(), &snaps, f.store.last_seq().unwrap(), ts("2026-10-21T00:00:00Z"), 4).unwrap();
        assert_eq!(std::fs::metadata(&snaps).unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn rotation_keeps_the_newest_and_clears_interrupted_writes() {
        let dir = tempfile::tempdir().unwrap();
        let f = setup(dir.path());
        let store = &f.store;
        let snaps = dir.path().join(SNAPSHOTS_DIR);
        let start = ts("2026-10-21T00:00:00Z");
        let mut written = Vec::new();
        for i in 0..6 {
            push(&f, &format!("d{i}"), &format!("# d{i}\n"));
            let now = start + jiff::SignedDuration::from_mins(15 * i);
            if i == 4 {
                std::fs::write(snaps.join(".annotary-20261021T000000Z-s1.tar.tmp"), b"half").unwrap();
            }
            match take_snapshot(store.dir(), &snaps, store.last_seq().unwrap(), now, 3).unwrap() {
                SnapshotOutcome::Written { file, .. } => written.push(file),
                other => panic!("expected a write: {other:?}"),
            }
        }
        let left = list_snapshots(&snaps).unwrap();
        assert_eq!(left, written[3..].to_vec(), "the three newest stay, oldest first");
        let names: Vec<String> =
            std::fs::read_dir(&snaps).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        assert_eq!(names.len(), 3, "no temp file left: {names:?}");
    }

    // CMD-23: the deploy's `pre-schema-N.tar` (and an older one it kept aside) sit in the same
    // folder; rotation and the temp-file sweep only ever touch this module's own names.
    #[test]
    fn rotation_never_touches_the_deploys_pre_schema_exports() {
        let dir = tempfile::tempdir().unwrap();
        let f = setup(dir.path());
        let snaps = dir.path().join(SNAPSHOTS_DIR);
        create_private_dir(&snaps).unwrap();
        let kept = ["pre-schema-4.tar", "pre-schema-4.20261030T000000Z.tar", ".pre-schema-4.tar.123.tmp"];
        for k in kept {
            std::fs::write(snaps.join(k), b"export").unwrap();
        }
        let start = ts("2026-10-21T00:00:00Z");
        for i in 0..4 {
            push(&f, &format!("d{i}"), &format!("# d{i}\n"));
            let now = start + jiff::SignedDuration::from_mins(15 * i);
            take_snapshot(f.store.dir(), &snaps, f.store.last_seq().unwrap(), now, 1).unwrap();
        }
        assert_eq!(list_snapshots(&snaps).unwrap().len(), 1);
        for k in kept {
            assert_eq!(std::fs::read(snaps.join(k)).unwrap(), b"export", "{k} must survive rotation");
        }
    }

    #[test]
    fn a_missing_store_is_an_error_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let snaps = dir.path().join(SNAPSHOTS_DIR);
        assert!(take_snapshot(&dir.path().join("store"), &snaps, 1, ts("2026-10-21T00:00:00Z"), 4).is_err());
        assert_eq!(std::fs::read_dir(&snaps).unwrap().count(), 0);
    }
}
