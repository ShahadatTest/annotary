//! Document ids and Version identity — the two things every side (CLI, server, WASM) must
//! compute the same way.
//!
//! - A Document id is 8 lowercase base32 characters from an alphabet without `0/1/o/l`.
//! - A Version's identity is `sha256` over the Markdown blob's hash and the sorted
//!   `(ref, asset-hash)` manifest, so an asset-only change makes a new Version and `same_as` can
//!   be derived from stored hashes alone.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use sha2::{Digest, Sha256};

/// The 32 characters a Document id is drawn from (no `0`, `1`, `o`, `l`).
pub const DOC_ID_ALPHABET: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";

/// Length of a Document id.
pub const DOC_ID_LEN: usize = 8;

/// True when `s` has the exact shape of a Document id.
pub fn is_document_id(s: &str) -> bool {
    s.len() == DOC_ID_LEN && s.bytes().all(|b| DOC_ID_ALPHABET.contains(&b))
}

/// Lowercase hex sha256 of `bytes` — the content address of a blob.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The Version identity for a Markdown blob hash plus its asset manifest as `(ref, hash)` pairs
/// (`None` = a missing Asset, which still counts). Order of the input does not matter.
pub fn version_hash<'a>(md_hash: &str, assets: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> String {
    let mut pairs: Vec<(&str, Option<&str>)> = assets.into_iter().collect();
    pairs.sort();
    let mut input = String::with_capacity(md_hash.len() + pairs.len() * 96);
    input.push_str(md_hash);
    for (r, h) in pairs {
        input.push('\n');
        input.push_str(r);
        input.push('\t');
        input.push_str(h.unwrap_or(""));
    }
    sha256_hex(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_shape_and_alphabet() {
        assert!(is_document_id("k7q2m9xd"));
        assert!(!is_document_id("k7q2m9x"));
        assert!(!is_document_id("k7q2m9x0"), "0 is not in the alphabet");
        assert!(!is_document_id("k7q2m9xl"), "l is not in the alphabet");
        assert!(!is_document_id("K7Q2M9XD"));
        assert_eq!(DOC_ID_ALPHABET.len(), 32);
        for c in b"01ol" {
            assert!(!DOC_ID_ALPHABET.contains(c));
        }
    }

    #[test]
    fn sha256_is_hex() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn version_hash_depends_on_manifest_not_its_order() {
        let md = sha256_hex(b"# hi\n");
        let none = version_hash(&md, []);
        assert_eq!(none, sha256_hex(md.as_bytes()), "no assets: sha256 of the Markdown blob hash text");
        let a = version_hash(&md, [("img/a.png", Some("aa")), ("b.md", Some("bb"))]);
        let b = version_hash(&md, [("b.md", Some("bb")), ("img/a.png", Some("aa"))]);
        assert_eq!(a, b);
        assert_ne!(a, none);
        assert_ne!(version_hash(&md, [("img/a.png", None)]), version_hash(&md, [("img/a.png", Some("aa"))]));
        assert_ne!(version_hash(&sha256_hex(b"other"), []), none);
    }
}
