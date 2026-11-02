//! The Assets a render knows about — the Version's manifest entries, as far as rendering needs
//! them — and how a Markdown destination resolves against them.

use std::collections::HashMap;

use serde_json::Value;

use crate::refs::{Classified, classify, normalize};

/// One manifest entry (`{ref, hash, size, type, missing?, via?, width?, height?, derivative?}`),
/// reduced to what the renderer uses. Build it with [`Asset::from_manifest`] from the JSON the
/// commit carries / the Version stores, so the CLI render and the server's fallback render see
/// the same thing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asset {
    /// The manifest `ref` — the normalised path relative to the root Document's directory.
    pub reference: String,
    /// Content address of the original bytes (`None` when `missing`).
    pub hash: Option<String>,
    /// The content type recorded at push time (`image/png`, `text/markdown`, …).
    pub content_type: String,
    /// The file was referenced but not found on push.
    pub missing: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Content address of the web-safe derivative (HEIC/HEIF/AVIF/TIFF/BMP → PNG/JPEG).
    pub derivative: Option<String>,
}

/// What an Asset is, for the element it becomes (from its content type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    Other,
}

impl Asset {
    /// The renderer's view of a manifest array (entries without a `ref` are skipped).
    pub fn from_manifest(manifest: &Value) -> Vec<Asset> {
        let Some(entries) = manifest.as_array() else { return Vec::new() };
        entries
            .iter()
            .filter_map(|e| {
                let s = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_string);
                let n =
                    |k: &str| e.get(k).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
                Some(Asset {
                    reference: s("ref")?,
                    hash: s("hash"),
                    content_type: s("type").unwrap_or_default(),
                    missing: e.get("missing").and_then(Value::as_bool).unwrap_or(false),
                    width: n("width"),
                    height: n("height"),
                    derivative: s("derivative"),
                })
            })
            .collect()
    }

    pub fn kind(&self) -> Kind {
        let ct = self.content_type.split(';').next().unwrap_or("").trim();
        if ct.starts_with("image/") {
            Kind::Image
        } else if ct.starts_with("video/") {
            Kind::Video
        } else if ct.starts_with("audio/") {
            Kind::Audio
        } else if ct == "application/pdf" {
            Kind::Pdf
        } else if ct.starts_with("text/") {
            Kind::Text
        } else {
            Kind::Other
        }
    }

    /// The stable URL of the original bytes: `/a/{hash}` (`None` when missing).
    pub fn url(&self) -> Option<String> {
        self.hash.as_ref().map(|h| format!("/a/{h}"))
    }

    /// The URL to *display*: `/a/{hash}?derivative=1` when a web-safe derivative exists, else
    /// the original.
    pub fn display_url(&self) -> Option<String> {
        let h = self.hash.as_ref()?;
        Some(if self.derivative.is_some() { format!("/a/{h}?derivative=1") } else { format!("/a/{h}") })
    }
}

/// How a destination written in the Markdown resolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    /// A local reference that is in the manifest (present or `missing`).
    Asset(&'a Asset),
    /// A URL, fragment, `mailto:`, `data:` … — left as written (subject to the URL policy).
    External,
    /// A relative path that is not in the manifest (unsupported type, a directory, …) or an
    /// absolute path — left exactly as written.
    AsWritten,
}

/// The manifest indexed by `ref` for the root Document (its references normalise against `""`).
#[derive(Clone, Debug, Default)]
pub struct Manifest<'a> {
    by_ref: HashMap<&'a str, &'a Asset>,
}

impl<'a> Manifest<'a> {
    pub fn new(assets: &'a [Asset]) -> Manifest<'a> {
        // First entry wins on a duplicate ref (the scan folds duplicates, the server rejects them).
        let mut by_ref = HashMap::with_capacity(assets.len());
        for a in assets {
            by_ref.entry(a.reference.as_str()).or_insert(a);
        }
        Manifest { by_ref }
    }

    pub fn is_empty(&self) -> bool {
        self.by_ref.is_empty()
    }

    /// Resolve a destination as written in the root Document.
    pub fn resolve(&self, target: &str) -> Target<'a> {
        match classify(target) {
            Classified::Ignored => Target::External,
            Classified::Absolute => Target::AsWritten,
            Classified::Relative(p) => {
                let reference = normalize("", &p);
                match self.by_ref.get(reference.as_str()) {
                    Some(a) => Target::Asset(a),
                    None => Target::AsWritten,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_entries_reduce_and_resolve() {
        let assets = Asset::from_manifest(&json!([
            {"ref": "img/a.heic", "hash": "h1", "size": 9, "type": "image/heic", "width": 640, "height": 480, "derivative": "d1"},
            {"ref": "img/missing.png", "type": "image/png", "missing": true},
            {"ref": "docs/notes.md", "hash": "h2", "size": 3, "type": "text/markdown", "via": "api.md", "width": 0},
            {"no_ref": true},
        ]));
        assert_eq!(assets.len(), 3);
        assert_eq!(assets[0].kind(), Kind::Image);
        assert_eq!(assets[0].display_url().as_deref(), Some("/a/h1?derivative=1"));
        assert_eq!(assets[0].url().as_deref(), Some("/a/h1"));
        assert_eq!((assets[0].width, assets[0].height), (Some(640), Some(480)));
        assert!(assets[1].missing && assets[1].hash.is_none() && assets[1].url().is_none());
        assert_eq!(assets[2].kind(), Kind::Text);
        assert_eq!(assets[2].width, None, "0 is not a dimension");
        let m = Manifest::new(&assets);
        assert_eq!(m.resolve("img/a.heic"), Target::Asset(&assets[0]));
        assert_eq!(m.resolve("./img/../img/a.heic?x=1#frag"), Target::Asset(&assets[0]));
        assert_eq!(m.resolve("img/missing.png"), Target::Asset(&assets[1]));
        assert_eq!(m.resolve("https://example.com/a.png"), Target::External);
        assert_eq!(m.resolve("#frag"), Target::External);
        assert_eq!(m.resolve("mailto:a@b.c"), Target::External);
        assert_eq!(m.resolve("/abs/a.png"), Target::AsWritten);
        assert_eq!(m.resolve("other.docx"), Target::AsWritten);
        assert_eq!(m.resolve("docs/"), Target::AsWritten);
    }
}
