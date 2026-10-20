//! Session-message encryption for the runner: a per-session AES-256 key delivered under
//! RSA-OAEP(SHA-256) to the runner's public key, and AES-CBC/PKCS7 message bodies.

use aes::Aes256;
use aes::cipher::{BlockEncrypt, KeyInit};
use base64::Engine;
use reef_core::{Error, Result};

const STD: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Encrypt a fresh AES key for the runner (`encryptionKey.value`, `encrypted: true`,
/// `useFipsEncryption: true` → OAEP with SHA-256).
pub fn wrap_key(key: &[u8; 32], exponent_b64: &str, modulus_b64: &str) -> Result<String> {
    use rsa::{BigUint, Oaep, RsaPublicKey};
    let n = BigUint::from_bytes_be(&STD.decode(modulus_b64).map_err(|_| Error::msg("bad runner public key"))?);
    let e = BigUint::from_bytes_be(&STD.decode(exponent_b64).map_err(|_| Error::msg("bad runner public key"))?);
    let pk = RsaPublicKey::new(n, e).map_err(|_| Error::msg("bad runner public key"))?;
    let mut rng = rand_legacy::rngs::OsRng;
    let ct = pk.encrypt(&mut rng, Oaep::new::<sha2_010::Sha256>(), key).map_err(|e| Error::msg(format!("rsa encrypt: {e}")))?;
    Ok(STD.encode(ct))
}

/// AES-256-CBC with PKCS7 padding; returns (base64 iv, base64 ciphertext).
pub fn encrypt_body(key: &[u8; 32], plain: &[u8]) -> (String, String) {
    let iv: [u8; 16] = rand::random();
    let cipher = Aes256::new_from_slice(key).expect("aes key");
    let pad = 16 - (plain.len() % 16);
    let mut data = plain.to_vec();
    data.extend(std::iter::repeat_n(pad as u8, pad));
    let mut prev = iv;
    for chunk in data.chunks_mut(16) {
        for (b, p) in chunk.iter_mut().zip(prev.iter()) {
            *b ^= p;
        }
        let mut block = aes::Block::default();
        block.copy_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
        prev.copy_from_slice(chunk);
    }
    (STD.encode(iv), STD.encode(data))
}

pub fn new_key() -> [u8; 32] {
    rand::random()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockDecrypt;

    #[test]
    fn cbc_roundtrip() {
        let key = new_key();
        let (iv, ct) = encrypt_body(&key, b"hello runner, this is a message longer than a block");
        let iv = STD.decode(iv).unwrap();
        let ct = STD.decode(ct).unwrap();
        let cipher = Aes256::new_from_slice(&key).unwrap();
        let mut prev = iv.clone();
        let mut out = Vec::new();
        for chunk in ct.chunks(16) {
            let mut block = aes::Block::default();
            block.copy_from_slice(chunk);
            cipher.decrypt_block(&mut block);
            for (b, p) in block.iter_mut().zip(prev.iter()) {
                *b ^= p;
            }
            out.extend_from_slice(&block);
            prev = chunk.to_vec();
        }
        let pad = *out.last().unwrap() as usize;
        out.truncate(out.len() - pad);
        assert_eq!(out, b"hello runner, this is a message longer than a block");
    }
}
