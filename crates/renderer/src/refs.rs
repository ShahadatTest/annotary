//! What a Markdown destination *is* — shared by the push-time Asset scan
//! (`annotary::assets::scan` re-exports these) and the renderer, so what the scanner turns
//! into a manifest entry is exactly what the renderer rewrites to `/a/{hash}`.
//!
//! - [`MARKDOWN_OPTIONS`] — the one Markdown superset (GFM tables / task lists / strikethrough /
//!   footnotes + math + YAML front matter); scanner and renderer parse with the same options.
//! - [`classify`] — relative path (Asset candidate) vs ignored (URL, `#frag`, `mailto:`,
//!   `data:`, …) vs absolute filesystem path / `file://` (warned and ignored).
//! - [`normalize`] — lexical `.`/`..` normalisation of a relative path against the referencing
//!   file's directory, relative to the root Document's directory (= the manifest `ref`).

use pulldown_cmark::Options;

/// The one Markdown superset (GFM + tables / task lists / footnotes + math + front matter).
pub const MARKDOWN_OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_FOOTNOTES)
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_MATH)
    .union(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

/// What a target is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classified {
    /// A relative path (percent-decoded, query and fragment stripped) — an Asset candidate.
    Relative(String),
    /// URL, fragment, `mailto:`, `data:`, … — ignored silently.
    Ignored,
    /// An absolute filesystem path or `file://` — warned and ignored (not portable).
    Absolute,
}

/// Classify a link/image destination as written.
pub fn classify(target: &str) -> Classified {
    let t = target.trim();
    let t = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')).unwrap_or(t).trim();
    if t.is_empty() || t.starts_with('#') {
        return Classified::Ignored;
    }
    if let Some(colon) = t.find(':') {
        let scheme = &t[..colon];
        // RFC 3986 also allows '.', but `notes.md:1` is a file, not a scheme; no common scheme has one.
        let looks_like_scheme = scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-'));
        if looks_like_scheme {
            if scheme.len() == 1 {
                // `C:\photos\x.png` — a drive letter, not a scheme.
                return Classified::Absolute;
            }
            return if scheme.eq_ignore_ascii_case("file") { Classified::Absolute } else { Classified::Ignored };
        }
    }
    if t.starts_with("//") {
        return Classified::Ignored;
    }
    if t.starts_with('/') || t.starts_with('\\') {
        return Classified::Absolute;
    }
    let end = t.find(['?', '#']).unwrap_or(t.len());
    let path = percent_decode(&t[..end]);
    if path.is_empty() { Classified::Ignored } else { Classified::Relative(path) }
}

/// Decode `%XX` sequences (invalid ones are left alone; the result must be UTF-8 or the input
/// is returned unchanged).
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Lexically join `rel` onto `base_dir` (both slash-separated, relative to the root Document's
/// directory; `""` = the root directory) and normalise `.`/`..`: `docs` + `../img/x.png` →
/// `img/x.png`; `""` + `./a/../b.md` → `b.md`; leading `..` are kept (`../shared/x.png`).
pub fn normalize(base_dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for comp in base_dir.split('/').chain(rel.split('/')) {
        match comp {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&"..") | None => parts.push(".."),
                Some(_) => {
                    parts.pop();
                }
            },
            c => parts.push(c),
        }
    }
    parts.join("/")
}

/// The directory part of a normalised ref (`docs/notes.md` → `docs`; `notes.md` → `""`).
pub fn dir_of(reference: &str) -> &str {
    reference.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}
