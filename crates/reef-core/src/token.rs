//! Secrets: random tokens, hashing for storage, and the prefixes clients recognise.

use sha2::{Digest, Sha256};

/// Personal access token prefix. Like GitHub's `ghp_`, it lets secret scanners spot leaks.
pub const PAT_PREFIX: &str = "fgp_";
/// OAuth user access token prefix (issued by the device flow), like GitHub's `gho_`.
pub const OAUTH_PREFIX: &str = "fgo_";
/// Session cookie value prefix.
pub const SESSION_PREFIX: &str = "fgs_";

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Random alphanumeric string of `len` characters from the OS RNG.
pub fn random_string(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::fill(&mut bytes[..]);
    bytes.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect()
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    rand::fill(&mut b[..]);
    hex::encode(b)
}

/// A user code for the device flow, in GitHub's `XXXX-XXXX` shape.
pub fn user_code() -> String {
    const A: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ"; // no vowels, no ambiguous glyphs
    let mut b = [0u8; 8];
    rand::fill(&mut b[..]);
    let s: String = b.iter().map(|x| A[(*x as usize) % A.len()] as char).collect();
    format!("{}-{}", &s[..4], &s[4..])
}

/// Mint a new bearer secret with the given prefix. Returns the plaintext to hand out once.
pub fn mint(prefix: &str) -> String {
    format!("{prefix}{}", random_string(36))
}

/// The value stored in the database for a secret. SHA-256 is right here: the secret is
/// high-entropy and random, so a fast hash is safe and gives O(1) lookup by hash.
pub fn hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Constant-time comparison of two byte strings of equal length.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
