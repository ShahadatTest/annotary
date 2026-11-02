//! Asset lookups on the [`Store`] (ticket 03): what a blob hash *is* — found through the
//! manifests that reference it — so `/a/{hash}` can serve it with the right content type and
//! offer its web-safe derivative.

use rusqlite::params;

use crate::error::Result;
use crate::store::Store;

/// What the manifests say about a blob hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetInfo {
    /// The hash asked about.
    pub hash: String,
    /// The `type` recorded for it (for a derivative: the original's type — sniff the bytes).
    pub content_type: String,
    /// The `ref` it was pushed under (for a derivative: the original's ref).
    pub reference: String,
    /// The derivative's hash, when the entry has one (only for originals).
    pub derivative: Option<String>,
    /// True when `hash` is itself a derivative of `reference`.
    pub is_derivative: bool,
}

impl Store {
    /// The first manifest entry (newest Version first) whose `hash` or `derivative` is `hash`.
    pub fn asset_info(&self, hash: &str) -> Result<Option<AssetInfo>> {
        if !crate::blobs::is_blob_hash(hash) {
            return Ok(None);
        }
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT manifest FROM versions WHERE instr(manifest, ?1) > 0 ORDER BY at DESC LIMIT 50")?;
        let rows = stmt.query_map(params![hash], |r| r.get::<_, String>(0))?;
        for row in rows {
            let manifest: serde_json::Value = serde_json::from_str(&row?).unwrap_or_default();
            let Some(entries) = manifest.as_array() else { continue };
            for e in entries {
                let s = |k: &str| e.get(k).and_then(|v| v.as_str()).map(str::to_string);
                let (Some(reference), Some(content_type)) = (s("ref"), s("type")) else { continue };
                if s("hash").as_deref() == Some(hash) {
                    return Ok(Some(AssetInfo {
                        hash: hash.to_string(),
                        content_type,
                        reference,
                        derivative: s("derivative"),
                        is_derivative: false,
                    }));
                }
                if s("derivative").as_deref() == Some(hash) {
                    return Ok(Some(AssetInfo {
                        hash: hash.to_string(),
                        content_type,
                        reference,
                        derivative: None,
                        is_derivative: true,
                    }));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use annotary_sidecar::identity::{sha256_hex, version_hash};
    use serde_json::json;

    use super::*;
    use crate::caps::{ASSET_MAX, VERSION_MAX};
    use crate::docs::version_blob_refs;
    use crate::model::PushInput;
    use crate::store::{SetupInput, SetupOutcome};
    use crate::{DeletedFilter, DocFilter, StoreError};

    fn fresh() -> (tempfile::TempDir, Store, SetupOutcome) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap();
        (dir, store, out)
    }

    fn input(store: &Store, name: &str, md: &[u8], manifest: serde_json::Value) -> PushInput {
        let (h, _) = store.put_blob(md).unwrap();
        let mut i = PushInput::new(name, h);
        i.manifest = manifest;
        i
    }

    // spec: S03-09, S03-08, S03-12
    #[test]
    fn asset_only_changes_make_versions_and_derivatives_are_referenced_blobs() {
        let (_d, store, out) = fresh();
        let (png, _) = store.put_blob(b"png-1").unwrap();
        let (png2, _) = store.put_blob(b"png-2").unwrap();
        let (deriv, _) = store.put_blob(b"jpeg-derivative").unwrap();
        let (heic, _) = store.put_blob(b"heic-original").unwrap();
        let m1 = json!([
            {"ref": "img/a.png", "hash": png, "size": 5, "type": "image/png"},
            {"ref": "img/p.heic", "hash": heic, "size": 13, "type": "image/heic", "derivative": deriv, "width": 3, "height": 2},
            {"ref": "img/gone.png", "type": "image/png", "missing": true},
        ]);
        let doc =
            store.create_document(&out.owner.id, &out.vault.id, input(&store, "plan", b"# md\n", m1.clone())).unwrap();
        assert_eq!(
            doc.head.hash,
            version_hash(
                &doc.head.md_hash,
                [("img/a.png", Some(png.as_str())), ("img/p.heic", Some(heic.as_str())), ("img/gone.png", None)]
            )
        );
        // same Markdown, one Asset changed → v2 (not `already`)
        let m2 = json!([
            {"ref": "img/a.png", "hash": png2, "size": 5, "type": "image/png"},
            {"ref": "img/p.heic", "hash": heic, "size": 13, "type": "image/heic", "derivative": deriv},
            {"ref": "img/gone.png", "type": "image/png", "missing": true},
        ]);
        let two = store.push_version(&out.owner.id, &doc.id, input(&store, "plan", b"# md\n", m2.clone())).unwrap();
        assert!(!two.already);
        assert_eq!(two.version.number, 2);
        // dimensions and derivative are stored on the entry; a missing entry has no hash/size
        let stored = &two.version.manifest;
        assert_eq!(stored[1]["derivative"], deriv);
        assert_eq!(stored[2], json!({"ref": "img/gone.png", "type": "image/png", "missing": true}));
        assert_eq!(doc.head.manifest[1]["width"], 3);
        // identical manifest again → already
        assert!(store.push_version(&out.owner.id, &doc.id, input(&store, "plan", b"# md\n", m2)).unwrap().already);
        // the derivative is one of the Version's blob references, so GC keeps it
        let refs = version_blob_refs(&two.version.md_hash, &two.version.manifest, None);
        assert!(refs.contains(&deriv) && refs.contains(&heic) && refs.contains(&png2));
        assert!(store.gc_blobs(Duration::ZERO).unwrap().is_empty());
        // asset_info: originals by hash, derivatives by their hash
        let info = store.asset_info(&heic).unwrap().unwrap();
        assert_eq!(
            (info.reference.as_str(), info.content_type.as_str(), info.is_derivative),
            ("img/p.heic", "image/heic", false)
        );
        assert_eq!(info.derivative.as_deref(), Some(deriv.as_str()));
        let dinfo = store.asset_info(&deriv).unwrap().unwrap();
        assert_eq!((dinfo.reference.as_str(), dinfo.is_derivative), ("img/p.heic", true));
        assert_eq!(store.asset_info(&png).unwrap().unwrap().reference, "img/a.png", "older Versions still answer");
        assert_eq!(store.asset_info(&sha256_hex(b"unknown")).unwrap(), None);
        assert_eq!(store.asset_info("nope").unwrap(), None);
        // purge: blobs only this Document referenced go (incl. the derivative), shared ones stay
        let other = store
            .create_document(
                &out.owner.id,
                &out.vault.id,
                input(
                    &store,
                    "other",
                    b"# other\n",
                    json!([{"ref": "x.png", "hash": png, "size": 5, "type": "image/png"}]),
                ),
            )
            .unwrap();
        store.delete_document(&out.owner.id, &doc.id).unwrap();
        let purged = store.purge_document(&out.owner.id, &doc.id).unwrap();
        let mut removed = purged.blobs_removed.clone();
        removed.sort();
        let mut expect = vec![doc.head.md_hash.clone(), png2.clone(), heic.clone(), deriv.clone()];
        expect.sort();
        assert_eq!(removed, expect);
        assert!(store.has_blob(&png), "still referenced by {}", other.id);
        assert!(!store.has_blob(&deriv));
        let (live, _) = store.list_documents(&DocFilter { deleted: DeletedFilter::Any, ..Default::default() }).unwrap();
        assert_eq!(live.len(), 1);
    }

    // spec: S03-10
    #[test]
    fn commit_caps_are_checked_from_real_blob_sizes() {
        let (_d, store, out) = fresh();
        // Sparse files stand in for huge blobs (no disk, no time).
        let sparse = |name: &str, len: u64| -> String {
            let hash = sha256_hex(name.as_bytes());
            let path = store.blobs().path(&hash);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let f = std::fs::File::create(&path).unwrap();
            f.set_len(len).unwrap();
            hash
        };
        let big = sparse("big", ASSET_MAX + 1);
        let err = store
            .create_document(
                &out.owner.id,
                &out.vault.id,
                input(
                    &store,
                    "plan",
                    b"# md\n",
                    json!([{"ref": "big.mp4", "hash": big, "size": 1, "type": "video/mp4"}]),
                ),
            )
            .unwrap_err();
        assert!(
            matches!(&err, StoreError::TooLarge { what, size, max } if what == "Asset big.mp4" && *size == ASSET_MAX + 1 && *max == ASSET_MAX),
            "{err}"
        );
        assert_eq!(err.to_string(), "Asset big.mp4 is 100.1 MB — the cap is 100 MB");
        // exactly at the cap is fine; six of them together are not
        let entries: Vec<serde_json::Value> = (0..6)
            .map(|i| json!({"ref": format!("v{i}.mp4"), "hash": sparse(&format!("v{i}"), ASSET_MAX), "type": "video/mp4"}))
            .collect();
        let ok = store
            .create_document(&out.owner.id, &out.vault.id, input(&store, "five", b"# five\n", json!(entries[..5])))
            .unwrap();
        assert_eq!(ok.head.manifest.as_array().unwrap().len(), 5);
        assert_eq!(ok.head.manifest[0]["size"], ASSET_MAX, "size comes from the blob, not the claim");
        let err = store
            .create_document(&out.owner.id, &out.vault.id, input(&store, "six", b"# six\n", json!(entries)))
            .unwrap_err();
        assert!(
            matches!(&err, StoreError::TooLarge { size, max, .. } if *size == 6 * ASSET_MAX && *max == VERSION_MAX),
            "{err}"
        );
        assert_eq!(err.to_string(), "the Version's 6 Assets together is 600 MB — the cap is 500 MB");
        // an oversized derivative is refused too
        let (heic, _) = store.put_blob(b"heic").unwrap();
        let err = store
            .create_document(&out.owner.id, &out.vault.id, input(&store, "d", b"# d\n", json!([{"ref": "p.heic", "hash": heic, "type": "image/heic", "derivative": sparse("bigd", ASSET_MAX + 1)}])))
            .unwrap_err();
        assert!(matches!(&err, StoreError::TooLarge { what, .. } if what == "derivative of p.heic"), "{err}");
        // a Markdown blob over its cap
        let md = sparse("bigmd", crate::caps::MARKDOWN_MAX + 1);
        let err = store.create_document(&out.owner.id, &out.vault.id, PushInput::new("m", md)).unwrap_err();
        assert!(matches!(&err, StoreError::TooLarge { what, .. } if what == "the Markdown"), "{err}");
    }
}
