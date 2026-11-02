//! Tokens: opaque random `ant_` + 43 base62 chars, sha256 at rest, shown once.

use sha2::{Digest, Sha256};

/// Recognisable prefix of every annotary Token.
pub const TOKEN_PREFIX: &str = "ant_";

/// A freshly minted Token: the id (stored) and the plain text (shown once, never stored).
#[derive(Clone, Debug)]
pub struct NewToken {
    pub id: String,
    pub plain: String,
}

/// Mint a new Token value (not yet stored).
pub fn mint() -> NewToken {
    NewToken {
        id: crate::ids::ulid(),
        plain: format!("{TOKEN_PREFIX}{}", crate::ids::random_base62(crate::ids::TOKEN_BODY_LEN)),
    }
}

/// sha256 hex of the plain Token — the only form kept in the database.
pub fn hash(plain: &str) -> String {
    let digest = Sha256::digest(plain.as_bytes());
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// True when `s` has the shape of a annotary Token (prefix + base62 body).
pub fn looks_like_token(s: &str) -> bool {
    s.strip_prefix(TOKEN_PREFIX)
        .map(|body| !body.is_empty() && body.bytes().all(|b| crate::ids::BASE62.contains(&b)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_tokens_have_prefix_and_length() {
        let t = mint();
        assert!(t.plain.starts_with("ant_"));
        assert_eq!(t.plain.len(), 4 + 43);
        assert!(looks_like_token(&t.plain));
        assert!(!looks_like_token("tok_abc"));
        assert!(!looks_like_token("ant_"));
    }

    #[test]
    fn hash_is_sha256_hex() {
        assert_eq!(hash("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
