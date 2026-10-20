//! Commit signature verification (OpenPGP and SSH signatures) against users' registered keys,
//! and parsing of armored GPG keys on upload. Results follow GitHub's `verification` object:
//! `{verified, reason, signature, payload, verified_at}` with reasons `valid`, `unsigned`,
//! `unknown_key`, `bad_email`, `expired_key`, `invalid`, `unknown_signature_type`.

use crate::AppState;
use pgp::composed::{Deserializable, DetachedSignature, SignedPublicKey};
use pgp::types::KeyDetails;
use reef_core::{Error, Id};
use reef_db::repos::RepoWithOwner;
use reef_git::Commit;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A raw commit split into the part that was signed and its signature header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitCommit {
    pub payload: String,
    pub signature: Option<String>,
}

/// Separate the `gpgsig` header (with its continuation lines) from the rest of the raw commit
/// object; the rest is exactly the bytes the signature covers.
pub fn split_signature(raw: &str) -> SplitCommit {
    let (headers, message) = match raw.find("\n\n") {
        Some(i) => (&raw[..i + 1], &raw[i + 1..]),
        None => (raw, ""),
    };
    let mut payload = String::with_capacity(raw.len());
    let mut signature: Option<String> = None;
    let mut in_sig = false;
    for line in headers.split_inclusive('\n') {
        if in_sig {
            if let Some(cont) = line.strip_prefix(' ') {
                signature.get_or_insert_with(String::new).push_str(cont);
                continue;
            }
            in_sig = false;
        }
        if let Some(first) = line.strip_prefix("gpgsig ") {
            signature = Some(first.to_string());
            in_sig = true;
            continue;
        }
        payload.push_str(line);
    }
    payload.push_str(message);
    SplitCommit { payload, signature: signature.map(|s| s.trim_end_matches('\n').to_string() + "\n") }
}

fn unsigned() -> Value {
    json!({"verified": false, "reason": "unsigned", "signature": Value::Null, "payload": Value::Null, "verified_at": Value::Null})
}

fn result(verified: bool, reason: &str, signature: &str, payload: &str) -> Value {
    json!({
        "verified": verified,
        "reason": reason,
        "signature": signature,
        "payload": payload,
        "verified_at": if verified { json!(reef_core::time::iso(reef_core::time::now_ms())) } else { Value::Null },
    })
}

type Cache = Mutex<HashMap<String, (Instant, Value)>>;

fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

const TTL: Duration = Duration::from_secs(300);

/// Forget cached results, e.g. after a user registers a key.
pub fn invalidate_cache() {
    if let Ok(mut c) = cache().lock() {
        c.clear();
    }
}

/// The `verification` object for a commit. Cached per sha for a few minutes.
pub async fn verify_commit(s: &AppState, rw: &RepoWithOwner, c: &Commit) -> Value {
    let key = format!("{}:{}", rw.repo.id, c.id);
    if let Ok(cache) = cache().lock()
        && let Some((at, v)) = cache.get(&key)
        && at.elapsed() < TTL
    {
        return v.clone();
    }
    let v = verify_uncached(s, rw, c).await;
    if let Ok(mut cache) = cache().lock() {
        if cache.len() > 10_000 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), v.clone()));
    }
    v
}

async fn raw_commit(s: &AppState, rw: &RepoWithOwner, sha: &str) -> Option<String> {
    let out = tokio::process::Command::new(s.store.git_binary())
        .arg("-C")
        .arg(s.store.path(rw.repo.id()))
        .args(["cat-file", "commit", sha])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn verify_uncached(s: &AppState, rw: &RepoWithOwner, c: &Commit) -> Value {
    let Some(raw) = raw_commit(s, rw, &c.id).await else { return unsigned() };
    let split = split_signature(&raw);
    let Some(sig) = split.signature else { return unsigned() };
    let payload = split.payload;
    if sig.contains("BEGIN PGP SIGNATURE") {
        verify_pgp(s, c, &sig, &payload).await
    } else if sig.contains("BEGIN SSH SIGNATURE") {
        verify_ssh(s, c, &sig, &payload).await
    } else {
        result(false, "unknown_signature_type", &sig, &payload)
    }
}

async fn committer_account(s: &AppState, c: &Commit) -> Option<reef_db::accounts::Account> {
    crate::rest::contents::account_for_signature(s, &c.committer.name, &c.committer.email).await
}

async fn verify_pgp(s: &AppState, c: &Commit, sig: &str, payload: &str) -> Value {
    let (detached, _) = match DetachedSignature::from_string(sig) {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(error = %e, sha = %c.id, "unparseable PGP signature");
            return result(false, "invalid", sig, payload);
        }
    };
    let issuers: Vec<String> = detached.signature.issuer_key_id().iter().map(|k| hex::encode_upper(k.as_ref())).collect();
    let issuer_fps: Vec<String> = detached.signature.issuer_fingerprint().iter().map(|f| format!("{f:X}")).collect();
    // Candidate keys: any registered key whose primary or subkey id matches the issuer.
    let mut candidates: Vec<reef_db::gpg::GpgKey> = Vec::new();
    for kid in issuers.iter().chain(issuer_fps.iter()) {
        if let Ok(keys) = s.db.gpg_keys_matching(kid).await {
            for k in keys {
                if !candidates.iter().any(|existing| existing.id == k.id) {
                    candidates.push(k);
                }
            }
        }
    }
    if candidates.is_empty() {
        return result(false, "unknown_key", sig, payload);
    }
    let signed_at = detached.signature.created().map(|t| t.as_secs() as i64 * 1000);
    for k in &candidates {
        let (parsed, _) = match SignedPublicKey::from_string(&k.public_key) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let ok = detached.verify(&parsed, payload.as_bytes()).is_ok()
            || parsed.public_subkeys.iter().any(|sk| detached.verify(sk, payload.as_bytes()).is_ok());
        if !ok {
            continue;
        }
        if let (Some(exp), Some(at)) = (k.expires_at, signed_at)
            && at > exp
        {
            return result(false, "expired_key", sig, payload);
        }
        // The key must belong to the committer and list the committer's address.
        let owner_ok = match committer_account(s, c).await {
            Some(a) => a.id == k.user_id,
            None => false,
        };
        let email = c.committer.email.to_lowercase();
        let email_ok = k.emails().iter().any(|e| e.to_lowercase() == email);
        if !owner_ok || !email_ok {
            return result(false, "bad_email", sig, payload);
        }
        return result(true, "valid", sig, payload);
    }
    result(false, "invalid", sig, payload)
}

async fn verify_ssh(s: &AppState, c: &Commit, sig: &str, payload: &str) -> Value {
    let Some(account) = committer_account(s, c).await else { return result(false, "unknown_key", sig, payload) };
    let keys = s.db.ssh_signing_keys(account.id()).await.unwrap_or_default();
    if keys.is_empty() {
        return result(false, "unknown_key", sig, payload);
    }
    let principal = c.committer.email.clone();
    let dir = s.store.tmp_dir().join(format!("reef-sig-{}-{}", std::process::id(), reef_core::id::next().0));
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        return result(false, "invalid", sig, payload);
    }
    let allowed = dir.join("allowed_signers");
    let sigfile = dir.join("commit.sig");
    let mut lines = String::new();
    for k in &keys {
        lines.push_str(&format!("{principal} namespaces=\"git\" {}\n", k.key));
    }
    let _ = tokio::fs::write(&allowed, lines).await;
    let _ = tokio::fs::write(&sigfile, sig).await;
    let mut cmd = tokio::process::Command::new("ssh-keygen");
    cmd.args(["-Y", "verify", "-f"]).arg(&allowed).args(["-I", &principal, "-n", "git", "-s"]).arg(&sigfile);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let verdict = match cmd.spawn() {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(payload.as_bytes()).await;
            }
            match child.wait_with_output().await {
                Ok(out) if out.status.success() => Some(true),
                Ok(out) => {
                    tracing::debug!(sha = %c.id, stderr = %String::from_utf8_lossy(&out.stderr), "ssh signature rejected");
                    Some(false)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ssh-keygen failed to run");
                    None
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "ssh-keygen is not available; SSH commit signatures cannot be verified");
            None
        }
    };
    let _ = tokio::fs::remove_dir_all(&dir).await;
    match verdict {
        Some(true) => result(true, "valid", sig, payload),
        _ => result(false, "invalid", sig, payload),
    }
}

// ----- key parsing on upload -----

/// What Reef stores about an uploaded armored public key.
pub struct ParsedGpgKey {
    pub key_id: String,
    pub fingerprint: String,
    pub emails: Vec<String>,
    pub subkeys: Value,
    pub can_sign: bool,
    pub can_certify: bool,
    pub can_encrypt_comms: bool,
    pub can_encrypt_storage: bool,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub raw_key: String,
}

fn email_of_user_id(uid: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(uid);
    if let (Some(a), Some(b)) = (s.rfind('<'), s.rfind('>'))
        && b > a
    {
        return Some(s[a + 1..b].trim().to_string());
    }
    let t = s.trim();
    if t.contains('@') && !t.contains(' ') {
        return Some(t.to_string());
    }
    None
}

pub fn parse_gpg_key(armored: &str) -> Result<ParsedGpgKey, Error> {
    let armored = armored.trim();
    if !armored.contains("BEGIN PGP PUBLIC KEY BLOCK") {
        return Err(Error::field(
            "GpgKey",
            "armored_public_key",
            "invalid",
            "key is invalid: paste an ASCII-armored public key (gpg --armor --export <id>)",
        ));
    }
    let (key, _) = SignedPublicKey::from_string(armored)
        .map_err(|e| Error::field("GpgKey", "armored_public_key", "invalid", format!("key is invalid: {e}")))?;
    if let Err(e) = key.verify_bindings() {
        return Err(Error::field("GpgKey", "armored_public_key", "invalid", format!("key is invalid: {e}")));
    }
    let created_at = key.created_at().as_secs() as i64 * 1000;
    let self_sig = key.details.users.iter().flat_map(|u| u.signatures.iter()).next();
    let expires_at = self_sig.and_then(|sg| sg.key_expiration_time()).map(|d| created_at + d.as_secs() as i64 * 1000);
    let flags = self_sig.map(|sg| sg.key_flags());
    let (mut can_sign, mut can_certify, mut can_encrypt_comms, mut can_encrypt_storage) = match &flags {
        Some(f) => (f.sign(), f.certify(), f.encrypt_comms(), f.encrypt_storage()),
        None => (true, true, false, false),
    };
    let emails: Vec<String> = key.details.users.iter().filter_map(|u| email_of_user_id(u.id.id())).collect();
    let mut subkeys = Vec::new();
    for sk in &key.public_subkeys {
        let sig = sk.signatures.first();
        let f = sig.map(|sg| sg.key_flags());
        let (s_sign, s_cert, s_comms, s_storage) = match &f {
            Some(f) => (f.sign(), f.certify(), f.encrypt_comms(), f.encrypt_storage()),
            None => (false, false, false, false),
        };
        can_sign |= s_sign;
        can_certify |= s_cert;
        can_encrypt_comms |= s_comms;
        can_encrypt_storage |= s_storage;
        let sk_created = sk.created_at().as_secs() as i64 * 1000;
        subkeys.push(json!({
            "key_id": hex::encode_upper(sk.legacy_key_id().as_ref()),
            "fingerprint": format!("{:X}", sk.fingerprint()),
            "algorithm": format!("{:?}", sk.algorithm()),
            "can_sign": s_sign,
            "can_certify": s_cert,
            "can_encrypt_comms": s_comms,
            "can_encrypt_storage": s_storage,
            "created_at": reef_core::time::iso(sk_created),
            "expires_at": sig.and_then(|sg| sg.key_expiration_time()).map(|d| reef_core::time::iso(sk_created + d.as_secs() as i64 * 1000)),
        }));
    }
    Ok(ParsedGpgKey {
        key_id: hex::encode_upper(key.legacy_key_id().as_ref()),
        fingerprint: format!("{:X}", key.fingerprint()),
        emails,
        subkeys: Value::Array(subkeys),
        can_sign,
        can_certify,
        can_encrypt_comms,
        can_encrypt_storage,
        created_at,
        expires_at,
        raw_key: armored.to_string(),
    })
}

/// GitHub's GPG key JSON.
pub fn gpg_key_json(k: &reef_db::gpg::GpgKey) -> Value {
    let emails: Vec<Value> = k.emails().into_iter().map(|e| json!({"email": e, "verified": true})).collect();
    let subkeys: Vec<Value> = k
        .subkeys()
        .into_iter()
        .map(|mut sk| {
            if let Some(o) = sk.as_object_mut() {
                o.insert("id".into(), json!(k.id));
                o.insert("primary_key_id".into(), json!(k.id));
                o.insert("public_key".into(), json!(""));
                o.insert("emails".into(), json!([]));
                o.insert("subkeys".into(), json!([]));
                o.insert("raw_key".into(), Value::Null);
            }
            sk
        })
        .collect();
    json!({
        "id": k.id,
        "name": Value::Null,
        "primary_key_id": k.primary_key_id,
        "key_id": k.key_id,
        "fingerprint": k.fingerprint,
        "public_key": k.public_key,
        "emails": emails,
        "subkeys": subkeys,
        "can_sign": k.can_sign != 0,
        "can_encrypt_comms": k.can_encrypt_comms != 0,
        "can_encrypt_storage": k.can_encrypt_storage != 0,
        "can_certify": k.can_certify != 0,
        "created_at": reef_core::time::iso(k.created_at),
        "expires_at": k.expires_at.map(reef_core::time::iso),
        "revoked": false,
        "raw_key": k.public_key,
    })
}

pub fn _unused(_: Id) {}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNED: &str = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nparent 0000000000000000000000000000000000000001\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n iQEzBAABCAAdFiEE\n =abcd\n -----END PGP SIGNATURE-----\n\nHello\n";

    #[test]
    fn splits_gpgsig_header() {
        let s = split_signature(SIGNED);
        assert_eq!(
            s.payload,
            "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nparent 0000000000000000000000000000000000000001\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nHello\n"
        );
        assert_eq!(s.signature.as_deref(), Some("-----BEGIN PGP SIGNATURE-----\n\niQEzBAABCAAdFiEE\n=abcd\n-----END PGP SIGNATURE-----\n"));
    }

    #[test]
    fn unsigned_commit_has_no_signature() {
        let raw = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nmsg\n";
        let s = split_signature(raw);
        assert_eq!(s.payload, raw);
        assert!(s.signature.is_none());
    }

    #[test]
    fn extracts_emails_from_user_ids() {
        assert_eq!(email_of_user_id(b"Ann Example <ann@example.com>").as_deref(), Some("ann@example.com"));
        assert_eq!(email_of_user_id(b"ann@example.com").as_deref(), Some("ann@example.com"));
        assert_eq!(email_of_user_id(b"Ann Example"), None);
    }
}
