//! Identifiers and random material.

/// Base62 alphabet used for Token bodies.
pub const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Number of base62 characters in a Token body: 62^43 > 2^256, i.e. > 32 bytes of entropy.
pub const TOKEN_BODY_LEN: usize = 43;

/// New ULID (Member ids, Vault ids, Token ids, Tenant id).
pub fn ulid() -> String {
    ulid::Ulid::new().to_string()
}

/// `n` cryptographically random base62 characters (rejection sampling, unbiased).
pub fn random_base62(n: usize) -> String {
    let mut out = String::with_capacity(n);
    let mut buf = [0u8; 64];
    while out.len() < n {
        getrandom::fill(&mut buf).expect("OS randomness available");
        for b in buf {
            // 62 * 4 = 248: reject 248..=255 to keep the distribution uniform.
            if b < 248 {
                out.push(BASE62[(b % 62) as usize] as char);
                if out.len() == n {
                    break;
                }
            }
        }
    }
    out
}

/// New random Document id: 8 chars from the base32 alphabet without `0/1/o/l`
/// ([`annotary_sidecar::identity::DOC_ID_ALPHABET`]). Uniqueness is checked at insert.
pub fn document_id() -> String {
    use annotary_sidecar::identity::{DOC_ID_ALPHABET, DOC_ID_LEN};
    let mut buf = [0u8; DOC_ID_LEN];
    getrandom::fill(&mut buf).expect("OS randomness available");
    buf.iter().map(|b| DOC_ID_ALPHABET[(b % 32) as usize] as char).collect()
}

/// Fill `buf` with OS randomness.
pub fn random_bytes(buf: &mut [u8]) {
    getrandom::fill(buf).expect("OS randomness available");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base62_has_requested_length_and_alphabet() {
        let s = random_base62(43);
        assert_eq!(s.len(), 43);
        assert!(s.bytes().all(|b| BASE62.contains(&b)));
        assert_ne!(random_base62(43), s);
    }

    // spec: S02-26 — random ids (two draws differ), never sequential.
    #[test]
    fn document_ids_are_8_chars_of_the_alphabet() {
        let id = document_id();
        assert!(annotary_sidecar::identity::is_document_id(&id), "{id}");
        assert_ne!(document_id(), id);
    }

    #[test]
    fn ulids_are_26_chars() {
        assert_eq!(ulid().len(), 26);
    }
}
