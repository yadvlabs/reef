//! Tokens for the runner protocol. Reef issues HS256 JWTs (shared instance key, so any host
//! can verify without a database hit) and verifies the runner's RS256/PS256 client assertions
//! against the RSA public key it registered.

use base64::Engine;
use hmac::{Hmac, Mac};
use reef_core::{Error, Result};
use serde_json::{Value, json};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
type HmacSha256 = Hmac<sha2_010::Sha256>;

/// Who a Reef-issued runner token speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// `config.sh` registration/removal for a scope.
    Registration { scope_kind: String, scope_id: i64 },
    /// A configured runner (listener).
    Runner { runner_id: i64 },
    /// A running job (worker callbacks).
    Job { job_id: i64 },
}

impl Principal {
    fn sub(&self) -> String {
        match self {
            Principal::Registration { scope_kind, scope_id } => format!("reg:{scope_kind}:{scope_id}"),
            Principal::Runner { runner_id } => format!("runner:{runner_id}"),
            Principal::Job { job_id } => format!("job:{job_id}"),
        }
    }
    fn parse(sub: &str) -> Option<Principal> {
        let mut p = sub.split(':');
        match (p.next()?, p.next(), p.next()) {
            ("reg", Some(k), Some(id)) => Some(Principal::Registration { scope_kind: k.to_string(), scope_id: id.parse().ok()? }),
            ("runner", Some(id), None) => Some(Principal::Runner { runner_id: id.parse().ok()? }),
            ("job", Some(id), None) => Some(Principal::Job { job_id: id.parse().ok()? }),
            _ => None,
        }
    }
}

pub fn mint(key: &[u8], p: &Principal, ttl_secs: i64, extra: Value) -> String {
    let now = reef_core::time::now_ms() / 1000;
    let mut claims = json!({"iss": "reef", "sub": p.sub(), "nbf": now - 60, "exp": now + ttl_secs, "jti": reef_core::token::random_hex(8)});
    if let Value::Object(m) = extra {
        for (k, v) in m {
            claims[k] = v;
        }
    }
    let header = B64.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = B64.encode(serde_json::to_vec(&claims).unwrap_or_default());
    let signing = format!("{header}.{payload}");
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(signing.as_bytes());
    format!("{signing}.{}", B64.encode(mac.finalize().into_bytes()))
}

pub fn verify(key: &[u8], token: &str) -> Result<Principal> {
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(Error::Unauthorized("malformed token".into()));
    };
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(format!("{h}.{p}").as_bytes());
    let sig = B64.decode(sig).map_err(|_| Error::Unauthorized("malformed token".into()))?;
    mac.verify_slice(&sig).map_err(|_| Error::Unauthorized("bad token signature".into()))?;
    let claims: Value = serde_json::from_slice(&B64.decode(p).map_err(|_| Error::Unauthorized("malformed token".into()))?)
        .map_err(|_| Error::Unauthorized("malformed token".into()))?;
    let now = reef_core::time::now_ms() / 1000;
    if claims.get("exp").and_then(|e| e.as_i64()).map(|e| e < now).unwrap_or(true) {
        return Err(Error::Unauthorized("token expired".into()));
    }
    claims.get("sub").and_then(|s| s.as_str()).and_then(Principal::parse).ok_or_else(|| Error::Unauthorized("unknown token subject".into()))
}

/// Decoded (unverified) header and claims of any JWT.
pub fn peek(token: &str) -> Option<(Value, Value)> {
    let mut parts = token.split('.');
    let h = B64.decode(parts.next()?).ok()?;
    let p = B64.decode(parts.next()?).ok()?;
    Some((serde_json::from_slice(&h).ok()?, serde_json::from_slice(&p).ok()?))
}

/// Verify the runner's `client_assertion` (RS256 or PS256) with its registered public key
/// (`exponent`/`modulus` base64 as .NET exports them). Returns the claims.
pub fn verify_client_assertion(token: &str, exponent_b64: &str, modulus_b64: &str, expected_aud: &str) -> Result<Value> {
    use rsa::pkcs1v15::Pkcs1v15Sign;
    use rsa::pss::Pss;
    use rsa::{BigUint, RsaPublicKey};
    use sha2_010::{Digest, Sha256};
    let std = base64::engine::general_purpose::STANDARD;
    let mut parts = token.split('.');
    let (Some(h), Some(p), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(Error::Unauthorized("malformed assertion".into()));
    };
    let header: Value = serde_json::from_slice(&B64.decode(h).map_err(|_| Error::Unauthorized("malformed assertion".into()))?)
        .map_err(|_| Error::Unauthorized("malformed assertion".into()))?;
    let claims: Value = serde_json::from_slice(&B64.decode(p).map_err(|_| Error::Unauthorized("malformed assertion".into()))?)
        .map_err(|_| Error::Unauthorized("malformed assertion".into()))?;
    let sig = B64.decode(sig).map_err(|_| Error::Unauthorized("malformed assertion".into()))?;
    let n = BigUint::from_bytes_be(&std.decode(modulus_b64).map_err(|_| Error::Unauthorized("bad public key".into()))?);
    let e = BigUint::from_bytes_be(&std.decode(exponent_b64).map_err(|_| Error::Unauthorized("bad public key".into()))?);
    let key = RsaPublicKey::new(n, e).map_err(|_| Error::Unauthorized("bad public key".into()))?;
    let digest = Sha256::digest(format!("{h}.{p}").as_bytes());
    let ok = match header.get("alg").and_then(|a| a.as_str()).unwrap_or("") {
        "RS256" => key.verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &sig).is_ok(),
        "PS256" => key.verify(Pss::new::<Sha256>(), &digest, &sig).is_ok(),
        other => return Err(Error::Unauthorized(format!("unsupported assertion algorithm {other}"))),
    };
    if !ok {
        return Err(Error::Unauthorized("assertion signature invalid".into()));
    }
    let now = reef_core::time::now_ms() / 1000;
    // Five-minute assertions; allow generous clock skew either way.
    if claims.get("exp").and_then(|e| e.as_i64()).map(|e| e + 300 < now).unwrap_or(true) {
        return Err(Error::Unauthorized(format!("assertion expired. Current server time is {}", reef_core::time::iso(now * 1000))));
    }
    if claims.get("nbf").and_then(|e| e.as_i64()).map(|n| n - 300 > now).unwrap_or(false) {
        return Err(Error::Unauthorized(format!("assertion not yet valid. Current server time is {}", reef_core::time::iso(now * 1000))));
    }
    let aud = claims.get("aud").and_then(|a| a.as_str()).unwrap_or("");
    if aud.trim_end_matches('/') != expected_aud.trim_end_matches('/') {
        return Err(Error::Unauthorized("assertion audience mismatch".into()));
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = b"0123456789abcdef0123456789abcdef";
        let t = mint(key, &Principal::Job { job_id: 42 }, 60, json!({"orch_id": "x"}));
        assert_eq!(verify(key, &t).unwrap(), Principal::Job { job_id: 42 });
        assert!(verify(b"other-key-other-key-other-key-00", &t).is_err());
        let (h, c) = peek(&t).unwrap();
        assert_eq!(h["alg"], "HS256");
        assert_eq!(c["orch_id"], "x");
        let reg = mint(key, &Principal::Registration { scope_kind: "repo".into(), scope_id: 7 }, 60, json!({}));
        assert_eq!(verify(key, &reg).unwrap(), Principal::Registration { scope_kind: "repo".into(), scope_id: 7 });
    }
}
