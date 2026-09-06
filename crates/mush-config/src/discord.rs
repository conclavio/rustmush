//! Transport-agnostic Discord link-code helpers (D1, D-D-3).
//!
//! The **server** mints a code (`@discord/link`) and the **gateway** redeems it
//! (the bot's `/link`), so both halves must normalize and hash a code the same
//! way; this module is the one place that knows how. A code is eight symbols
//! from a 32-symbol alphabet with no `0 O 1 I` (about 40 bits), shown as
//! `ABCD-EFGH`, stored only as its SHA-256 hex (the web-token rule), and
//! consumed exactly once by the store.

use rand::Rng;
use sha2::{Digest, Sha256};

/// The code alphabet: 32 symbols, no `0 O 1 I`.
pub const LINK_CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Symbols per code.
pub const LINK_CODE_LEN: usize = 8;

/// How long a minted code stays redeemable, seconds (03 §7, 04 §7).
pub const LINK_CODE_TTL_S: i64 = 600;

/// Mint a fresh code in its display form (`ABCD-EFGH`), from the OS-seeded
/// CSPRNG.
#[must_use]
pub fn generate_link_code() -> String {
    let mut rng = rand::rng();
    let mut out = String::with_capacity(LINK_CODE_LEN + 1);
    for i in 0..LINK_CODE_LEN {
        if i == LINK_CODE_LEN / 2 {
            out.push('-');
        }
        let n = rng.random_range(0..LINK_CODE_ALPHABET.len());
        out.push(char::from(LINK_CODE_ALPHABET[n]));
    }
    out
}

/// Normalize a typed code — case-insensitive, dashes and spaces ignored —
/// into its canonical eight uppercase symbols, or `None` if it is not a
/// well-formed code. The redeem path calls this before hashing so a player
/// may type `abcd-efgh`, `ABCDEFGH`, or `abcd efgh`.
#[must_use]
pub fn normalize_link_code(raw: &str) -> Option<String> {
    let normalized: String = raw
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\t'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if normalized.len() != LINK_CODE_LEN
        || !normalized.bytes().all(|b| LINK_CODE_ALPHABET.contains(&b))
    {
        return None;
    }
    Some(normalized)
}

/// The storage key of a **normalized** code: SHA-256 hex.
#[must_use]
pub fn link_code_hash(normalized: &str) -> String {
    let digest = Sha256::digest(normalized.as_bytes());
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_are_well_formed_and_normalize_to_themselves() {
        for _ in 0..64 {
            let code = generate_link_code();
            assert_eq!(code.len(), LINK_CODE_LEN + 1);
            assert_eq!(&code[4..5], "-");
            let normalized = normalize_link_code(&code).expect("well-formed");
            assert_eq!(normalized, code.replace('-', ""));
            assert!(!normalized.contains(['0', 'O', '1', 'I']));
        }
    }

    #[test]
    fn normalization_is_forgiving_about_case_dashes_and_spaces_only() {
        assert_eq!(
            normalize_link_code("abcd-efgh").as_deref(),
            Some("ABCDEFGH")
        );
        assert_eq!(
            normalize_link_code("ABCD EFGH").as_deref(),
            Some("ABCDEFGH")
        );
        assert_eq!(
            normalize_link_code(" abcdefgh ").as_deref(),
            Some("ABCDEFGH")
        );
        assert_eq!(normalize_link_code("ABCD-EFG"), None, "too short");
        assert_eq!(normalize_link_code("ABCD-EFGHJ"), None, "too long");
        assert_eq!(
            normalize_link_code("ABCD-EFG0"),
            None,
            "0 is not in the alphabet"
        );
        assert_eq!(
            normalize_link_code("ABCD-EFGI"),
            None,
            "I is not in the alphabet"
        );
        assert_eq!(normalize_link_code(""), None);
    }

    #[test]
    fn the_hash_is_stable_hex_over_the_normalized_form() {
        let a = link_code_hash("ABCDEFGH");
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            a,
            link_code_hash(&normalize_link_code("abcd-efgh").unwrap())
        );
        assert_ne!(a, link_code_hash("ABCDEFGJ"));
    }
}
