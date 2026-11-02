//! The asset manifest a commit carries — validated and normalised before it is stored.
//!
//! An entry is `{ref, hash, size, type, missing?, via?, width?, height?, derivative?}` (ticket
//! 03). The server (this module) checks the shape, that every referenced blob is in the store,
//! and the caps in [`crate::caps`]; sizes are taken from the blob store (authoritative), unknown
//! keys are dropped, and the keys come out in that fixed order so stored manifests are canonical.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::blobs::{BlobStore, is_blob_hash};
use crate::caps::{ASSET_MAX, MARKDOWN_MAX, VERSION_MAX};
use crate::error::{Result, StoreError};

/// The keys an entry may carry, in canonical order.
pub const ENTRY_KEYS: [&str; 9] = ["ref", "hash", "size", "type", "missing", "via", "width", "height", "derivative"];

/// A validated, normalised manifest plus the total Asset bytes it references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validated {
    pub manifest: Value,
    /// Sum of the Assets' sizes (originals; derivatives and the Markdown excluded).
    pub total_size: u64,
}

fn invalid(msg: impl Into<String>) -> StoreError {
    StoreError::InvalidManifest(msg.into())
}

/// A manifest `ref` is a relative, slash-separated path: not empty, not absolute, no NUL, no
/// trailing slash, no backslashes.
pub fn check_ref(r: &str) -> std::result::Result<(), &'static str> {
    if r.is_empty() {
        return Err("must not be empty");
    }
    if r.starts_with('/') {
        return Err("must be relative (no leading '/')");
    }
    if r.contains('\\') {
        return Err("must use '/' as the separator");
    }
    if r.contains('\0') || r.chars().any(|c| c.is_control()) {
        return Err("must not contain control characters");
    }
    if r.ends_with('/') || r == "." || r == ".." || r.ends_with("/.") || r.ends_with("/..") {
        return Err("must name a file");
    }
    if r.len() > 4096 {
        return Err("longer than 4096 bytes");
    }
    Ok(())
}

/// Validate `manifest` (the commit's `manifest` field) for a Version whose Markdown blob is
/// `md_hash`. Every non-missing entry's blob (and derivative) must already be in `blobs`.
pub fn validate(blobs: &BlobStore, md_hash: &str, manifest: &Value) -> Result<Validated> {
    let md_size = blobs.size(md_hash)?.ok_or_else(|| StoreError::BlobMissing(md_hash.to_string()))?;
    if md_size > MARKDOWN_MAX {
        return Err(StoreError::TooLarge { what: "the Markdown".into(), size: md_size, max: MARKDOWN_MAX });
    }
    let entries = manifest.as_array().ok_or_else(|| invalid("manifest must be an array"))?;
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::with_capacity(entries.len());
    let mut total: u64 = 0;
    for (i, e) in entries.iter().enumerate() {
        let obj = e.as_object().ok_or_else(|| invalid(format!("manifest[{i}] is not an object")))?;
        let s = |k: &str| obj.get(k).and_then(Value::as_str);
        let r = s("ref").ok_or_else(|| invalid(format!("manifest[{i}] has no `ref`")))?;
        check_ref(r).map_err(|why| invalid(format!("manifest[{i}] ref {r:?} {why}")))?;
        if !seen.insert(r) {
            return Err(invalid(format!("manifest lists ref {r:?} twice")));
        }
        let content_type = s("type").map(str::trim).filter(|t| !t.is_empty());
        let Some(content_type) = content_type else {
            return Err(invalid(format!("manifest entry {r:?} has no `type`")));
        };
        let missing = match obj.get("missing") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(invalid(format!("manifest entry {r:?}: `missing` must be a boolean"))),
        };
        let via = match obj.get("via") {
            None | Some(Value::Null) => None,
            Some(Value::String(v)) if !v.trim().is_empty() => {
                check_ref(v).map_err(|why| invalid(format!("manifest entry {r:?} via {v:?} {why}")))?;
                Some(v.clone())
            }
            Some(_) => return Err(invalid(format!("manifest entry {r:?}: `via` must be a ref"))),
        };
        let mut n = Map::new();
        n.insert("ref".into(), json!(r));
        if missing {
            n.insert("type".into(), json!(content_type));
            n.insert("missing".into(), json!(true));
            if let Some(v) = via {
                n.insert("via".into(), json!(v));
            }
            out.push(Value::Object(n));
            continue;
        }
        let hash =
            s("hash").ok_or_else(|| invalid(format!("manifest entry {r:?} has no `hash` (and is not missing)")))?;
        if !is_blob_hash(hash) {
            return Err(invalid(format!("manifest entry {r:?}: hash {hash:?} is not a sha256 hex hash")));
        }
        let size = blobs.size(hash)?.ok_or_else(|| StoreError::BlobMissing(hash.to_string()))?;
        if size > ASSET_MAX {
            return Err(StoreError::TooLarge { what: format!("Asset {r}"), size, max: ASSET_MAX });
        }
        total += size;
        n.insert("hash".into(), json!(hash));
        n.insert("size".into(), json!(size));
        n.insert("type".into(), json!(content_type));
        if let Some(v) = via {
            n.insert("via".into(), json!(v));
        }
        for dim in ["width", "height"] {
            match obj.get(dim) {
                None | Some(Value::Null) => {}
                Some(v) => match v.as_u64() {
                    Some(0) => {}
                    Some(px) => {
                        n.insert(dim.into(), json!(px));
                    }
                    None => return Err(invalid(format!("manifest entry {r:?}: `{dim}` must be a positive integer"))),
                },
            }
        }
        match obj.get("derivative") {
            None | Some(Value::Null) => {}
            Some(Value::String(d)) if is_blob_hash(d) => {
                let dsize = blobs.size(d)?.ok_or_else(|| StoreError::BlobMissing(d.clone()))?;
                if dsize > ASSET_MAX {
                    return Err(StoreError::TooLarge {
                        what: format!("derivative of {r}"),
                        size: dsize,
                        max: ASSET_MAX,
                    });
                }
                n.insert("derivative".into(), json!(d));
            }
            Some(_) => return Err(invalid(format!("manifest entry {r:?}: `derivative` must be a sha256 hex hash"))),
        }
        out.push(Value::Object(n));
    }
    if total > VERSION_MAX {
        let n = out.len();
        return Err(StoreError::TooLarge {
            what: format!("the Version's {n} Assets together"),
            size: total,
            max: VERSION_MAX,
        });
    }
    Ok(Validated { manifest: Value::Array(out), total_size: total })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, BlobStore) {
        let d = tempfile::tempdir().unwrap();
        let bs = BlobStore::new(d.path());
        (d, bs)
    }

    // spec: S03-10, S03-11
    #[test]
    fn manifest_is_normalised_and_typed_errors_name_the_problem() {
        let (_d, bs) = store();
        let (md, _) = bs.put(b"# doc\n").unwrap();
        let (png, _) = bs.put(b"PNG bytes").unwrap();
        let (deriv, _) = bs.put(b"derivative bytes").unwrap();
        let ok = json!([
            {"extra": 1, "type": "image/heic", "size": 999, "ref": "img/a.heic", "hash": png, "width": 32, "height": 20, "derivative": deriv, "via": null},
            {"ref": "img/missing.png", "type": "image/png", "missing": true, "hash": "ignored"},
            {"ref": "docs/notes.md", "type": "text/markdown", "hash": png, "via": "api.md", "width": 0},
        ]);
        let v = validate(&bs, &md, &ok).unwrap();
        assert_eq!(v.total_size, 18, "sizes come from the blob store, derivatives excluded");
        let entries = v.manifest.as_array().unwrap();
        // canonical key order, unknown keys dropped, size corrected
        assert_eq!(
            entries[0].as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
            vec!["ref", "hash", "size", "type", "width", "height", "derivative"]
        );
        assert_eq!(entries[0]["size"], 9);
        assert_eq!(entries[1], json!({"ref": "img/missing.png", "type": "image/png", "missing": true}));
        assert_eq!(
            entries[2],
            json!({"ref": "docs/notes.md", "hash": png, "size": 9, "type": "text/markdown", "via": "api.md"})
        );
        assert_eq!(validate(&bs, &md, &json!([])).unwrap().manifest, json!([]));

        let bad = |m: Value| validate(&bs, &md, &m).unwrap_err().to_string();
        assert!(bad(json!({})).contains("must be an array"));
        assert!(bad(json!([1])).contains("not an object"));
        assert!(bad(json!([{"type": "x"}])).contains("no `ref`"));
        assert!(bad(json!([{"ref": "/abs.png", "type": "image/png", "hash": png}])).contains("relative"));
        assert!(bad(json!([{"ref": "a.png", "hash": png}])).contains("no `type`"));
        assert!(bad(json!([{"ref": "a.png", "type": "image/png"}])).contains("no `hash`"));
        assert!(bad(json!([{"ref": "a.png", "type": "image/png", "hash": "zz"}])).contains("not a sha256"));
        assert!(
            bad(json!([{"ref": "a.png", "type": "image/png", "hash": png, "width": -1}])).contains("positive integer")
        );
        assert!(
            bad(json!([{"ref": "a.png", "type": "image/png", "hash": png, "derivative": 5}])).contains("derivative")
        );
        assert!(bad(json!([{"ref": "a.png", "type": "image/png", "hash": png, "via": "/x"}])).contains("via"));
        assert!(bad(json!([{"ref": "a.png", "type": "image/png", "missing": "yes"}])).contains("boolean"));
        assert!(
            bad(json!([{"ref": "a.png", "type": "image/png", "hash": png}, {"ref": "a.png", "type": "image/png", "hash": png}]))
                .contains("twice")
        );
        // a blob that was never uploaded is a typed BlobMissing (409 on the wire)
        let never = annotary_sidecar::identity::sha256_hex(b"never");
        assert!(matches!(
            validate(&bs, &md, &json!([{"ref": "a.png", "type": "image/png", "hash": never}])),
            Err(StoreError::BlobMissing(h)) if h == never
        ));
        assert!(matches!(
            validate(&bs, &md, &json!([{"ref": "a.png", "type": "image/png", "hash": png, "derivative": never}])),
            Err(StoreError::BlobMissing(_))
        ));
        assert!(matches!(validate(&bs, &never, &json!([])), Err(StoreError::BlobMissing(_))));
        for r in ["", "/x", "a\\b", "dir/", ".", "..", "x/..", "a\u{1}b"] {
            assert!(check_ref(r).is_err(), "{r:?}");
        }
        for r in ["a.png", "img/a.png", "../shared/x.md", "sp ace/ü.pdf"] {
            assert!(check_ref(r).is_ok(), "{r:?}");
        }
    }
}
