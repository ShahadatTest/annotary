//! Content-addressed blob files: `store/blobs/sha256/ab/<hash>` (Markdown, Assets, rendered
//! HTML, image derivatives — Tenant-wide de-dup). Writes are atomic (temp file + rename); a blob
//! is either wholly there or not. Rows in SQLite only carry hashes; see `Store::purge_document`
//! / `Store::gc_blobs` for reclaiming files nothing references.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use annotary_sidecar::identity::sha256_hex;

use crate::error::{Result, StoreError};

/// Directory name under `store/`.
pub const BLOBS_DIR: &str = "blobs";

/// The blob store rooted at `store/blobs`.
#[derive(Clone, Debug)]
pub struct BlobStore {
    root: PathBuf,
}

/// True for a 64-char lowercase hex sha256.
pub fn is_blob_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl BlobStore {
    pub fn new(store_dir: &Path) -> BlobStore {
        BlobStore { root: store_dir.join(BLOBS_DIR) }
    }

    /// `store/blobs`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `store/blobs/sha256/ab/<hash>` (no existence check).
    pub fn path(&self, hash: &str) -> PathBuf {
        self.root.join("sha256").join(&hash[..2.min(hash.len())]).join(hash)
    }

    pub fn has(&self, hash: &str) -> bool {
        is_blob_hash(hash) && self.path(hash).is_file()
    }

    /// Store `bytes`; returns `(hash, created)` — `created` is false when the blob already existed.
    pub fn put(&self, bytes: &[u8]) -> Result<(String, bool)> {
        let hash = sha256_hex(bytes);
        let path = self.path(&hash);
        if path.is_file() {
            return Ok((hash, false));
        }
        let dir = path.parent().expect("blob path has a parent");
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{hash}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, bytes)?;
        match std::fs::rename(&tmp, &path) {
            Ok(()) => Ok((hash, true)),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                if path.is_file() { Ok((hash, false)) } else { Err(e.into()) }
            }
        }
    }

    /// Store `bytes` only if they hash to `expected`.
    pub fn put_expecting(&self, expected: &str, bytes: &[u8]) -> Result<(String, bool)> {
        let actual = sha256_hex(bytes);
        if actual != expected {
            return Err(StoreError::BlobHashMismatch { expected: expected.to_string(), actual });
        }
        self.put(bytes)
    }

    pub fn read(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        if !is_blob_hash(hash) {
            return Ok(None);
        }
        match std::fs::read(self.path(hash)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Size in bytes of a stored blob.
    pub fn size(&self, hash: &str) -> Result<Option<u64>> {
        if !is_blob_hash(hash) {
            return Ok(None);
        }
        match std::fs::metadata(self.path(hash)) {
            Ok(m) => Ok(Some(m.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a blob file; true when something was removed.
    pub fn remove(&self, hash: &str) -> Result<bool> {
        if !is_blob_hash(hash) {
            return Ok(false);
        }
        match std::fs::remove_file(self.path(hash)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    // ---- ticket 13 ----
    /// Remove leftover temp files of interrupted writes (`.<hash>.<pid>.tmp` next to the blobs)
    /// older than `grace` — the "derived-blob cleanup" half that is pure filesystem. Returns how
    /// many were removed.
    pub fn sweep_temp(&self, grace: std::time::Duration) -> Result<usize> {
        let cutoff = SystemTime::now().checked_sub(grace).unwrap_or(SystemTime::UNIX_EPOCH);
        let mut removed = 0;
        let sha = self.root.join("sha256");
        let Ok(shards) = std::fs::read_dir(&sha) else { return Ok(0) };
        for shard in shards.flatten() {
            if !shard.path().is_dir() {
                continue;
            }
            for f in std::fs::read_dir(shard.path())?.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                if !(name.starts_with('.') && name.ends_with(".tmp")) {
                    continue;
                }
                let modified = f.metadata()?.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if modified > cutoff {
                    continue;
                }
                match std::fs::remove_file(f.path()) {
                    Ok(()) => removed += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(removed)
    }
    // ---- end ticket 13 ----

    /// Every stored blob with its modification time (for the GC sweep).
    pub fn all(&self) -> Result<Vec<(String, SystemTime)>> {
        let mut out = Vec::new();
        let sha = self.root.join("sha256");
        let Ok(shards) = std::fs::read_dir(&sha) else { return Ok(out) };
        for shard in shards.flatten() {
            if !shard.path().is_dir() {
                continue;
            }
            for f in std::fs::read_dir(shard.path())?.flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                if !is_blob_hash(&name) {
                    continue;
                }
                let modified = f.metadata()?.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((name, modified));
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S02-26
    #[test]
    fn blobs_are_content_addressed_and_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let bs = BlobStore::new(dir.path());
        let (h, created) = bs.put(b"# hello\n").unwrap();
        assert!(created);
        assert_eq!(h, sha256_hex(b"# hello\n"));
        assert!(bs.path(&h).ends_with(format!("blobs/sha256/{}/{h}", &h[..2])));
        let (h2, created) = bs.put(b"# hello\n").unwrap();
        assert_eq!(h, h2);
        assert!(!created);
        assert!(bs.has(&h));
        assert_eq!(bs.read(&h).unwrap().unwrap(), b"# hello\n");
        assert_eq!(bs.size(&h).unwrap(), Some(8));
        assert!(matches!(bs.put_expecting("00", b"x"), Err(StoreError::BlobHashMismatch { .. })));
        assert!(!bs.has("../etc/passwd"));
        assert_eq!(bs.read("nope").unwrap(), None);
        assert_eq!(bs.all().unwrap().len(), 1);
        assert!(bs.remove(&h).unwrap());
        assert!(!bs.remove(&h).unwrap());
        assert!(!bs.has(&h));
        assert!(is_blob_hash(&h));
        assert!(!is_blob_hash(&h.to_uppercase()));
    }
}
