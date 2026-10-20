//! Actions secrets: encrypted at rest with an instance key, and accepted from clients as
//! libsodium sealed boxes (what `gh secret set` produces against `/actions/secrets/public-key`).

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use reef_core::{Error, Result};
use std::path::Path;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Instance-wide keys, created on first use under `<data_dir>/actions/keys/`.
#[derive(Clone)]
pub struct Keys {
    aes: [u8; 32],
    box_secret: [u8; 32],
    pub box_public: [u8; 32],
    pub key_id: String,
    /// HMAC key for runner/job JWTs.
    pub jwt: [u8; 32],
}

impl Keys {
    pub fn load_or_create(data_dir: &Path) -> Result<Keys> {
        let dir = data_dir.join("actions").join("keys");
        std::fs::create_dir_all(&dir).map_err(Error::internal)?;
        let aes = key_file(&dir.join("secrets.key"))?;
        let box_secret = key_file(&dir.join("sealedbox.key"))?;
        let secret = crypto_box::SecretKey::from(box_secret);
        let public = secret.public_key();
        let box_public: [u8; 32] = *public.as_bytes();
        let key_id = hex::encode(&reef_core::token::hash(&hex::encode(box_public))[..16]);
        let jwt = key_file(&dir.join("runner-jwt.key"))?;
        Ok(Keys { aes, box_secret, box_public, key_id, jwt })
    }

    pub fn public_key_b64(&self) -> String {
        B64.encode(self.box_public)
    }

    /// Encrypt for storage: base64(nonce || ciphertext).
    pub fn seal(&self, plain: &str) -> Result<String> {
        let cipher = Aes256Gcm::new_from_slice(&self.aes).map_err(|e| Error::msg(e.to_string()))?;
        let nonce_bytes: [u8; 12] = rand::random();
        let nonce = Nonce::from(nonce_bytes);
        let ct = cipher.encrypt(&nonce, plain.as_bytes()).map_err(|e| Error::msg(e.to_string()))?;
        let mut out = nonce_bytes.to_vec();
        out.extend(ct);
        Ok(B64.encode(out))
    }

    pub fn open(&self, stored: &str) -> Result<String> {
        let raw = B64.decode(stored).map_err(|e| Error::msg(e.to_string()))?;
        if raw.len() < 12 {
            return Err(Error::msg("corrupt secret"));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.aes).map_err(|e| Error::msg(e.to_string()))?;
        let nonce_bytes: [u8; 12] = raw[..12].try_into().map_err(|_| Error::msg("corrupt secret"))?;
        let pt = cipher.decrypt(&Nonce::from(nonce_bytes), &raw[12..]).map_err(|_| Error::msg("secret could not be decrypted"))?;
        String::from_utf8(pt).map_err(|e| Error::msg(e.to_string()))
    }

    /// Open a libsodium sealed box produced with our public key (GitHub's upload format).
    pub fn unseal_from_client(&self, encrypted_b64: &str) -> Result<String> {
        let raw = B64.decode(encrypted_b64.trim()).map_err(|_| Error::validation("encrypted_value is not valid base64"))?;
        let secret = crypto_box::SecretKey::from(self.box_secret);
        let pt =
            secret.unseal(&raw).map_err(|_| Error::validation("encrypted_value could not be decrypted with the current public key"))?;
        String::from_utf8(pt).map_err(|_| Error::validation("secret is not UTF-8"))
    }
}

/// Read a 32-byte key, or create it once with `O_EXCL` (mode 0600). An existing file that
/// cannot be read or has the wrong size is an error: it is never overwritten, since a fresh key
/// would silently orphan every secret and token minted with the old one.
fn key_file(p: &Path) -> Result<[u8; 32]> {
    use std::io::{Read, Write};
    match std::fs::File::open(p) {
        Ok(mut f) => {
            let mut b = Vec::new();
            f.read_to_end(&mut b).map_err(|e| Error::msg(format!("actions key {}: {e}", p.display())))?;
            return <[u8; 32]>::try_from(b.as_slice()).map_err(|_| Error::msg(format!("actions key {}: expected 32 bytes, found {}; refusing to regenerate it (restore it from backup or delete it deliberately)", p.display(), b.len())));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::msg(format!("actions key {}: {e}", p.display()))),
    }
    let k: [u8; 32] = rand::random();
    // Write the key to a private temporary file next to the final name, then link it into
    // place: the link either creates the final name atomically or fails because a sibling
    // instance created it first. A reader therefore sees a complete file or none at all,
    // never a partially written one.
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = p.with_file_name(format!("{name}.tmp-{}-{}", std::process::id(), reef_core::token::random_hex(4)));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let written = opts.open(&tmp).and_then(|mut f| {
        f.write_all(&k)?;
        f.sync_all()
    });
    let linked = written.and_then(|()| std::fs::hard_link(&tmp, p));
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => {
            let _ = OsRng;
            Ok(k)
        }
        // Another instance on the shared state directory created it first: use theirs.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => key_file(p),
        Err(e) => Err(Error::msg(format!("actions key {}: {e}", p.display()))),
    }
}

#[cfg(test)]
mod key_file_tests {
    use super::key_file;

    #[test]
    fn keys_are_created_once_atomically_and_never_regenerated() {
        let dir = std::env::temp_dir().join(format!("reef-keys-{}", reef_core::token::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("aes.key");
        let k1 = key_file(&p).unwrap();
        let k2 = key_file(&p).unwrap();
        assert_eq!(k1, k2, "the second call reads the first key");
        assert_eq!(std::fs::read(&p).unwrap().len(), 32);
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(leftovers, vec!["aes.key"], "no temporary file is left behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A partial file (what a non-atomic writer could have left) is an error, not a new key.
        let short = dir.join("short.key");
        std::fs::write(&short, b"abc").unwrap();
        let err = key_file(&short).unwrap_err().to_string();
        assert!(err.contains("expected 32 bytes"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
