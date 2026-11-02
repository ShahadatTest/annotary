//! Assets, computed at the edge (ticket 03): the push-time scan of a Markdown file for the local
//! files it references, the manifest entries that describe them, and the blobs to upload.
//!
//! - [`types`] — the supported-type constant (extension → content type, kind, scan?, derivative?).
//! - [`scan`] — pure reference extraction + classification + path normalisation (no filesystem).
//! - [`media`] — image/video dimensions and web-safe derivatives.
//! - [`scan_document`] (here) — walks the filesystem: resolves each relative reference against
//!   the referencing file's directory, reads and hashes it, records `missing: true` when it is
//!   not there, follows linked Markdown recursively (`via`, cycles by seen hash, depth ≤
//!   [`MAX_DEPTH`]), and collects the warnings `push` prints (and `--strict` turns into exit 5).
//!
//! Manifest entry: `{ref, hash, size, type, missing?, via?, width?, height?, derivative?}` —
//! `ref` is the normalised path relative to the root Document's directory (what `pull --assets`
//! writes to), `via` the `ref` of the Markdown Asset that referenced it.

pub mod media;
pub mod scan;
pub mod types;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use annotary_sidecar::identity::sha256_hex;

use self::scan::{Classified, classify, dir_of, normalize, references};
use self::types::{SupportedType, extension_of, type_of};

/// Linked Markdown is followed this many links deep from the root Document; deeper references
/// are not followed (warning).
pub const MAX_DEPTH: usize = 5;

/// One manifest entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub reference: String,
    pub hash: Option<String>,
    pub size: Option<u64>,
    pub content_type: &'static str,
    pub missing: bool,
    pub via: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub derivative: Option<String>,
}

impl Entry {
    /// The wire/stored form (keys in the canonical order, empty ones omitted).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("ref".into(), json!(self.reference));
        if let Some(h) = &self.hash {
            m.insert("hash".into(), json!(h));
        }
        if let Some(s) = self.size {
            m.insert("size".into(), json!(s));
        }
        m.insert("type".into(), json!(self.content_type));
        if self.missing {
            m.insert("missing".into(), json!(true));
        }
        if let Some(v) = &self.via {
            m.insert("via".into(), json!(v));
        }
        if let Some(w) = self.width {
            m.insert("width".into(), json!(w));
        }
        if let Some(h) = self.height {
            m.insert("height".into(), json!(h));
        }
        if let Some(d) = &self.derivative {
            m.insert("derivative".into(), json!(d));
        }
        Value::Object(m)
    }
}

/// Where a blob's bytes come from at upload time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// Re-read from disk when uploading (the hash is verified again then).
    File(PathBuf),
    /// Produced in memory (a derivative).
    Bytes(Vec<u8>),
}

/// A blob the Version references (Assets and derivatives; not the Markdown itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    pub hash: String,
    pub size: u64,
    /// The `ref` this blob was found under (for messages).
    pub label: String,
    pub source: Source,
}

impl Blob {
    /// The bytes to upload. Errors when a file changed since it was hashed.
    pub fn read(&self) -> Result<Vec<u8>, String> {
        match &self.source {
            Source::Bytes(b) => Ok(b.clone()),
            Source::File(p) => {
                let bytes = std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
                if sha256_hex(&bytes) != self.hash {
                    return Err(format!("{} changed while pushing — push again", self.label));
                }
                Ok(bytes)
            }
        }
    }
}

/// The kinds of push warning (`--strict` turns every one into exit 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarningKind {
    Missing,
    Unsupported,
    Absolute,
    Depth,
    Derivative,
}

/// One warning line (without the `warning: ` prefix).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub kind: WarningKind,
    pub text: String,
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// Manifest entries in document order (recursive entries right after their referrer).
    pub entries: Vec<Entry>,
    /// Blobs to (maybe) upload, unique by hash.
    pub blobs: Vec<Blob>,
    pub warnings: Vec<Warning>,
}

impl Scan {
    /// The `manifest` array a commit carries.
    pub fn manifest_json(&self) -> Value {
        Value::Array(self.entries.iter().map(Entry::to_json).collect())
    }

    /// Sum of the Assets' sizes (originals only — what the per-Version cap counts).
    pub fn total_size(&self) -> u64 {
        self.entries.iter().filter_map(|e| e.size).sum()
    }

    /// `ref`s of the missing entries, in order.
    pub fn missing_refs(&self) -> Vec<&str> {
        self.entries.iter().filter(|e| e.missing).map(|e| e.reference.as_str()).collect()
    }

    /// The blob for `hash`, if any.
    pub fn blob(&self, hash: &str) -> Option<&Blob> {
        self.blobs.iter().find(|b| b.hash == hash)
    }
}

/// Knobs (tests turn derivatives off to stay fast and machine-independent).
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub derivatives: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { derivatives: true }
    }
}

/// Scan the Markdown at `md_path` (already read into `md_bytes`) for its Assets.
pub fn scan_document(md_path: &Path, md_bytes: &[u8]) -> Scan {
    scan_document_with(md_path, md_bytes, Options::default())
}

/// [`scan_document`] with options.
pub fn scan_document_with(md_path: &Path, md_bytes: &[u8], opts: Options) -> Scan {
    let root_dir = md_path.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
    let root_dir = root_dir.unwrap_or_else(|| PathBuf::from("."));
    let root_ref = normalize("", md_path.file_name().and_then(|f| f.to_str()).unwrap_or(""));
    let mut w = Walker {
        root_dir,
        root_ref,
        opts,
        seen_hashes: HashSet::from([sha256_hex(md_bytes)]),
        by_ref: HashMap::new(),
        by_hash: HashSet::new(),
        warned: HashSet::new(),
        out: Scan::default(),
    };
    let text = String::from_utf8_lossy(md_bytes).into_owned();
    w.walk(&text, "", None, 1);
    w.out
}

struct Walker {
    root_dir: PathBuf,
    root_ref: String,
    opts: Options,
    /// Content hashes of every Markdown scanned so far (the root first) — the cycle guard.
    seen_hashes: HashSet<String>,
    by_ref: HashMap<String, usize>,
    by_hash: HashSet<String>,
    warned: HashSet<String>,
    out: Scan,
}

impl Walker {
    fn warn(&mut self, kind: WarningKind, key: String, text: String) {
        if self.warned.insert(format!("{kind:?}:{key}")) {
            self.out.warnings.push(Warning { kind, text });
        }
    }

    fn add_blob(&mut self, hash: &str, size: u64, label: &str, source: Source) {
        if self.by_hash.insert(hash.to_string()) {
            self.out.blobs.push(Blob { hash: hash.to_string(), size, label: label.to_string(), source });
        }
    }

    /// `text` is a Markdown file whose ref-directory is `base_dir`; its references become entries
    /// at `depth` (1 = referenced by the root Document).
    fn walk(&mut self, text: &str, base_dir: &str, via: Option<&str>, depth: usize) {
        let via_note = |v: Option<&str>| v.map(|v| format!(", via {v}")).unwrap_or_default();
        for raw in references(text) {
            let rel = match classify(&raw.target) {
                Classified::Ignored => continue,
                Classified::Absolute => {
                    let t = raw.target.trim().to_string();
                    self.warn(
                        WarningKind::Absolute,
                        t.clone(),
                        format!("absolute path {t}{} — ignored (not portable)", via_note(via)),
                    );
                    continue;
                }
                Classified::Relative(p) => p,
            };
            let reference = normalize(base_dir, &rel);
            if reference.is_empty() || reference == self.root_ref {
                continue; // a directory, or the Document linking to itself
            }
            if self.by_ref.contains_key(&reference) {
                continue;
            }
            let Some(ty) = type_of(&reference) else {
                let ext = extension_of(&reference).map(|e| format!(".{e}")).unwrap_or_else(|| "no extension".into());
                self.warn(
                    WarningKind::Unsupported,
                    reference.clone(),
                    format!(
                        "unsupported asset {reference} ({ext}{}) — not uploaded, link left as written",
                        via_note(via)
                    ),
                );
                continue;
            };
            let path = self.root_dir.join(&reference);
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    let why = match e.kind() {
                        std::io::ErrorKind::NotFound => "not found".to_string(),
                        _ if path.is_dir() => "is a directory".to_string(),
                        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
                        _ => format!("unreadable: {e}"),
                    };
                    self.warn(
                        WarningKind::Missing,
                        reference.clone(),
                        format!("missing asset {reference} ({why}{}) — pushed with missing:true", via_note(via)),
                    );
                    self.push_entry(Entry {
                        reference,
                        hash: None,
                        size: None,
                        content_type: ty.content_type,
                        missing: true,
                        via: via.map(str::to_string),
                        width: None,
                        height: None,
                        derivative: None,
                    });
                    continue;
                }
            };
            let hash = sha256_hex(&bytes);
            let mut entry = Entry {
                reference: reference.clone(),
                hash: Some(hash.clone()),
                size: Some(bytes.len() as u64),
                content_type: ty.content_type,
                missing: false,
                via: via.map(str::to_string),
                width: None,
                height: None,
                derivative: None,
            };
            self.add_blob(&hash, bytes.len() as u64, &reference, Source::File(path));
            self.probe(&mut entry, &bytes, ty);
            self.push_entry(entry);
            if ty.scan {
                let first_time = self.seen_hashes.insert(hash);
                if !first_time {
                    continue; // cycle or a second copy: already scanned
                }
                let inner = String::from_utf8_lossy(&bytes).into_owned();
                if depth < MAX_DEPTH {
                    self.walk(&inner, dir_of(&reference), Some(&reference), depth + 1);
                } else if references(&inner).iter().any(|r| matches!(classify(&r.target), Classified::Relative(_))) {
                    self.warn(
                        WarningKind::Depth,
                        reference.clone(),
                        format!("depth: {reference} is {MAX_DEPTH} links deep — its own references are not followed"),
                    );
                }
            }
        }
    }

    fn push_entry(&mut self, entry: Entry) {
        self.by_ref.insert(entry.reference.clone(), self.out.entries.len());
        self.out.entries.push(entry);
    }

    /// Dimensions and, where needed, a web-safe derivative.
    fn probe(&mut self, entry: &mut Entry, bytes: &[u8], ty: &SupportedType) {
        if !matches!(ty.kind, types::Kind::Image | types::Kind::Video) {
            return;
        }
        let mut dims = media::dimensions(bytes, ty.content_type);
        if ty.derivative && self.opts.derivatives {
            match media::derivative(bytes, ty) {
                Ok(d) => {
                    let dh = sha256_hex(&d);
                    let dtype = types::sniff_derivative_type(&d);
                    if let Some(dd) = media::dimensions(&d, dtype) {
                        dims = Some(dd);
                    }
                    self.add_blob(&dh, d.len() as u64, &format!("derivative of {}", entry.reference), Source::Bytes(d));
                    entry.derivative = Some(dh);
                }
                Err(e) => {
                    let r = entry.reference.clone();
                    self.warn(
                        WarningKind::Derivative,
                        r.clone(),
                        format!("no web derivative for {r} ({e}) — original stored as is"),
                    );
                }
            }
        }
        if let Some((w, h)) = dims {
            entry.width = Some(w);
            entry.height = Some(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets").join(name)).unwrap()
    }

    fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn quick(md_path: &Path, md: &[u8]) -> Scan {
        scan_document_with(md_path, md, Options { derivatives: false })
    }

    fn refs(s: &Scan) -> Vec<(String, Option<String>, bool)> {
        s.entries.iter().map(|e| (e.reference.clone(), e.via.clone(), e.missing)).collect()
    }

    fn warnings(s: &Scan) -> Vec<(WarningKind, String)> {
        s.warnings.iter().map(|w| (w.kind, w.text.clone())).collect()
    }

    // spec: S03-01, S03-02, S03-04, S03-06
    #[test]
    fn scan_resolves_reads_hashes_and_warns() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "docs/img/architecture.png", &fixture("a.png"));
        write(root, "docs/photo.jpg", &fixture("a.jpg"));
        write(root, "shared/logo.gif", &fixture("a.gif"));
        write(root, "docs/notes.docx", b"docx");
        write(root, "docs/README", b"no extension");
        std::fs::create_dir_all(root.join("docs/adir.png")).unwrap();
        let md = "\
![arch](img/architecture.png)
![again](./img/../img/architecture.png)
[photo](photo.jpg \"t\") and [shared](../shared/logo.gif) and [me](plan.md) and [me2](./plan.md#top)
![missing](img/cli-screenshot.png)
[doc](notes.docx) [readme](README) [dir](adir.png)
[abs](/srv/pics/x.png) <img src=\"file:///tmp/x.png\"> [web](https://x/y.png) [mail](mailto:a@b) [frag](#x)
<img src=\"pct%20name.webp\">
";
        write(root, "docs/plan.md", md.as_bytes());
        let s = quick(&root.join("docs/plan.md"), md.as_bytes());
        assert_eq!(
            refs(&s),
            vec![
                ("img/architecture.png".to_string(), None, false),
                ("photo.jpg".to_string(), None, false),
                ("../shared/logo.gif".to_string(), None, false),
                ("img/cli-screenshot.png".to_string(), None, true),
                ("adir.png".to_string(), None, true),
                ("pct name.webp".to_string(), None, true),
            ]
        );
        let png = &s.entries[0];
        assert_eq!(png.hash.as_deref(), Some(sha256_hex(&fixture("a.png")).as_str()));
        assert_eq!(png.size, Some(fixture("a.png").len() as u64));
        assert_eq!(png.content_type, "image/png");
        assert_eq!((png.width, png.height), (Some(32), Some(20)));
        assert_eq!(s.entries[1].content_type, "image/jpeg");
        assert_eq!(s.entries[2].content_type, "image/gif");
        let missing = &s.entries[3];
        assert_eq!((missing.hash.as_deref(), missing.size, missing.content_type), (None, None, "image/png"));
        assert_eq!(s.blobs.len(), 3, "one blob per distinct present file");
        assert_eq!(s.blobs[0].label, "img/architecture.png");
        assert!(matches!(&s.blobs[0].source, Source::File(p) if p.ends_with("docs/img/architecture.png")));
        assert_eq!(s.blobs[0].read().unwrap(), fixture("a.png"));
        assert_eq!(s.total_size(), s.blobs.iter().map(|b| b.size).sum::<u64>());
        assert_eq!(s.missing_refs(), vec!["img/cli-screenshot.png", "adir.png", "pct name.webp"]);
        assert_eq!(
            warnings(&s),
            vec![
                (
                    WarningKind::Missing,
                    "missing asset img/cli-screenshot.png (not found) — pushed with missing:true".into()
                ),
                (
                    WarningKind::Unsupported,
                    "unsupported asset notes.docx (.docx) — not uploaded, link left as written".into()
                ),
                (
                    WarningKind::Unsupported,
                    "unsupported asset README (no extension) — not uploaded, link left as written".into()
                ),
                (WarningKind::Missing, "missing asset adir.png (is a directory) — pushed with missing:true".into()),
                (WarningKind::Absolute, "absolute path /srv/pics/x.png — ignored (not portable)".into()),
                (WarningKind::Absolute, "absolute path file:///tmp/x.png — ignored (not portable)".into()),
                (WarningKind::Missing, "missing asset pct name.webp (not found) — pushed with missing:true".into()),
            ]
        );
        // The manifest JSON has the canonical keys and omits what is absent.
        let m = s.manifest_json();
        assert_eq!(
            m[0].as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
            vec!["ref", "hash", "size", "type", "width", "height"]
        );
        assert_eq!(m[3], json!({"ref": "img/cli-screenshot.png", "type": "image/png", "missing": true}));
        // A file that changes between hash and upload is caught.
        write(root, "docs/photo.jpg", b"changed");
        assert!(s.blobs[1].read().unwrap_err().contains("changed while pushing"));
        // No references at all → empty scan; a Markdown with no parent dir resolves against ".".
        assert_eq!(quick(Path::new("plan.md"), b"# nothing\n"), Scan::default());
    }

    // spec: S03-03
    #[test]
    fn linked_markdown_is_scanned_recursively_with_via_cycles_and_depth() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "img/flow.svg", &fixture("a.svg"));
        write(root, "img/deep.png", &fixture("b.png"));
        write(
            root,
            "api-notes.md",
            b"# API\n\n![flow](img/flow.svg)\n\nBack to [plan](plan.md) and [self](api-notes.md)\n",
        );
        write(root, "docs/d1.md", b"[d2](d2.md) ![deep](../img/deep.png)\n");
        write(root, "docs/d2.md", b"[d3](d3.md) [loop](d1.md)\n");
        write(root, "docs/d3.md", b"[d4](d4.md)\n");
        write(root, "docs/d4.md", b"[d5](d5.md)\n");
        write(root, "docs/d5.md", b"[d6](d6.md) [flow again](../img/flow.svg)\n");
        write(root, "docs/d6.md", b"never scanned\n");
        write(root, "notes.txt", b"[not scanned](nope.png)\n");
        write(root, "copy.md", b"# API\n\n![flow](img/flow.svg)\n\nBack to [plan](plan.md) and [self](api-notes.md)\n");
        let md = b"See [API notes](api-notes.md), [d1](docs/d1.md), [txt](notes.txt) and [copy](copy.md).\n";
        write(root, "plan.md", md);
        let s = quick(&root.join("plan.md"), md);
        assert_eq!(
            refs(&s),
            vec![
                ("api-notes.md".to_string(), None, false),
                ("img/flow.svg".to_string(), Some("api-notes.md".into()), false),
                ("docs/d1.md".to_string(), None, false),
                ("docs/d2.md".to_string(), Some("docs/d1.md".into()), false),
                ("docs/d3.md".to_string(), Some("docs/d2.md".into()), false),
                ("docs/d4.md".to_string(), Some("docs/d3.md".into()), false),
                ("docs/d5.md".to_string(), Some("docs/d4.md".into()), false),
                ("img/deep.png".to_string(), Some("docs/d1.md".into()), false),
                ("notes.txt".to_string(), None, false),
                ("copy.md".to_string(), None, false),
            ]
        );
        assert_eq!(s.entries[1].content_type, "image/svg+xml");
        assert_eq!((s.entries[1].width, s.entries[1].height), (Some(120), Some(80)));
        assert_eq!(s.entries[0].content_type, "text/markdown");
        assert_eq!(s.entries[8].content_type, "text/plain");
        // d6.md (six links deep) is not followed; d5.md's own references warn once.
        assert_eq!(
            warnings(&s),
            vec![(
                WarningKind::Depth,
                "depth: docs/d5.md is 5 links deep — its own references are not followed".into()
            )]
        );
        // copy.md has api-notes.md's content: listed, but not scanned again (seen hash).
        assert_eq!(s.entries[9].hash, s.entries[0].hash);
        assert_eq!(s.blobs.len(), 9, "copy.md shares api-notes.md's blob");
        // A chain that ends at depth 5 has no depth warning; the root linking to itself is not an entry.
        write(root, "docs/d5.md", b"no links here\n");
        let s2 = quick(&root.join("plan.md"), md);
        assert!(s2.warnings.is_empty(), "{:?}", s2.warnings);
        assert_eq!(s2.entries.len(), 10);
    }

    // spec: S03-07, S03-08
    #[test]
    fn dimensions_and_derivatives_land_in_the_manifest() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root, "img/a.bmp", &fixture("a.bmp"));
        write(root, "img/a.tiff", &fixture("a.tiff"));
        write(root, "img/a.mp4", &fixture("a.mp4"));
        write(root, "img/a.mp3", &fixture("a.mp3"));
        write(root, "img/a.heic", &fixture("a.heic"));
        let md = b"![b](img/a.bmp) ![t](img/a.tiff) <video src=\"img/a.mp4\"></video> <audio src=img/a.mp3></audio> ![h](img/a.heic)\n";
        write(root, "plan.md", md);
        let s = scan_document(&root.join("plan.md"), md);
        let by_ref = |r: &str| s.entries.iter().find(|e| e.reference == r).unwrap().clone();
        for r in ["img/a.bmp", "img/a.tiff"] {
            let e = by_ref(r);
            assert_eq!((e.width, e.height), (Some(32), Some(20)), "{r}");
            let dh = e.derivative.clone().expect("derivative");
            let blob = s.blob(&dh).expect("derivative blob");
            assert!(matches!(&blob.source, Source::Bytes(b) if b.starts_with(b"\x89PNG")));
            assert!(blob.label.starts_with("derivative of img/a."), "{}", blob.label);
            assert_ne!(Some(&dh), e.hash.as_ref());
        }
        let v = by_ref("img/a.mp4");
        assert_eq!((v.width, v.height, v.derivative.is_none()), (Some(48), Some(32), true));
        let a = by_ref("img/a.mp3");
        assert_eq!((a.width, a.height, a.derivative.is_none()), (None, None, true));
        let h = by_ref("img/a.heic");
        assert_eq!((h.width, h.height), (Some(32), Some(20)));
        if media::converter().is_some() {
            let dh = h.derivative.clone().expect("HEIC derivative on a machine with a converter");
            assert!(matches!(&s.blob(&dh).unwrap().source, Source::Bytes(b) if b.starts_with(&[0xFF, 0xD8])));
            assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        } else {
            assert!(h.derivative.is_none());
            assert_eq!(s.warnings.len(), 1);
            assert_eq!(s.warnings[0].kind, WarningKind::Derivative);
            assert!(s.warnings[0].text.starts_with("no web derivative for img/a.heic (no converter"));
        }
        // manifest JSON carries width/height/derivative
        let m = s.manifest_json();
        assert_eq!(m[0]["width"], 32);
        assert!(m[0]["derivative"].is_string());
        // derivatives off: same entries, no derivative blobs, no derivative warnings
        let q = scan_document_with(&root.join("plan.md"), md, Options { derivatives: false });
        assert!(q.entries.iter().all(|e| e.derivative.is_none()));
        assert!(q.warnings.is_empty());
        assert_eq!(q.blobs.len(), 5);
    }
}
