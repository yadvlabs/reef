//! Single logout (OpenID Connect Back-Channel Logout). When a sign-in ends at the identity
//! provider, the provider POSTs a logout token to the `logout_uri` registered for this Reef
//! (`uri`), and Reef ends the web sessions opened from that sign-in
//! (the ID token's `sid`, kept with the session), or every web session of the account when the
//! token names only the subject (suspension, deletion, "sign out everywhere"). API tokens and
//! runner registrations are not sessions and stay.
//!
//! Answers: 200 when handled (also when nothing matched), 400 `{"error":"invalid_request"}` for a
//! token that does not verify (providers do not retry 4xx), 503 when Reef cannot act right now
//! (providers retry 5xx with backoff).

use crate::AppState;
use crate::config::Config;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use openidconnect::core::{CoreJsonWebKey, CoreJsonWebKeySet, CoreJwsSigningAlgorithm};
use openidconnect::{JsonWebKey, JsonWebKeyUse};
use serde_json::Value;
use std::sync::Arc;

/// Where the provider POSTs logout tokens, next to `/login/oidc` and `/login/oidc/callback`. On the
/// console host (`site_url`): requests on the `api.` host are rewritten onto the REST API.
pub const PATH: &str = "/login/oidc/backchannel-logout";

/// The `events` member that makes a JWT a logout token.
pub const BACKCHANNEL_LOGOUT_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";

/// Clock skew tolerated on `iat`, `exp` and `nbf` (seconds).
pub const LEEWAY_SECS: i64 = 60;

/// Keys are rediscovered for an unknown `kid` at most this often.
const KEY_REFRESH_MS: i64 = 30_000;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// The back-channel `logout_uri` to register with the provider for this Reef.
pub fn uri(config: &Config) -> String {
    format!("{}{PATH}", config.site_url())
}

/// A verified logout token: whom it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogoutEvent {
    pub issuer: String,
    /// The account (the provider's subject).
    pub subject: Option<String>,
    /// The sign-in; `None` means every session of the account.
    pub sid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// Signed with a key (`kid`) not in the key set: refresh the keys and try again.
    UnknownKey,
    /// Not a valid logout token for this client.
    Invalid(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::UnknownKey => f.write_str("signed with an unknown key"),
            TokenError::Invalid(m) => f.write_str(m),
        }
    }
}

fn invalid(m: impl Into<String>) -> TokenError {
    TokenError::Invalid(m.into())
}

/// The `sid` claim of an ID token (or logout token), if any.
pub fn sid_claim(claims: &Value) -> Option<String> {
    str_claim(claims, "sid")
}

fn str_claim(claims: &Value, name: &str) -> Option<String> {
    claims.get(name).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from)
}

fn num_claim(claims: &Value, name: &str) -> Result<Option<i64>, TokenError> {
    match claims.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).map(Some).ok_or_else(|| invalid(format!("{name} is not a number"))),
    }
}

/// Whether a bearer secret is a logout token (a JWT typed `logout+jwt` or carrying `events`).
/// A logout token says who signed out; it is never a credential.
pub fn is_logout_token(token: &str) -> bool {
    let Some((header, claims)) = crate::actions::jwt::peek(token.trim()) else { return false };
    let typ = header.get("typ").and_then(|t| t.as_str()).unwrap_or("").to_ascii_lowercase();
    typ == "logout+jwt" || typ == "application/logout+jwt" || claims.get("events").is_some()
}

/// Verify a logout token from `issuer` for `client_id` against the issuer's signing keys, as
/// OpenID Connect Back-Channel Logout 1.0 section 2.6 asks: signature, `iss`, `aud`, `iat`,
/// `exp` (with leeway), the back-channel logout event, no `nonce`, and `sub` and/or `sid`.
pub fn verify_logout_token(
    raw: &str,
    issuer: &str,
    client_id: &str,
    keys: &CoreJsonWebKeySet,
    now_secs: i64,
) -> Result<LogoutEvent, TokenError> {
    let mut parts = raw.trim().split('.');
    let (Some(h), Some(p), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(invalid("malformed token"));
    };
    let decode_json = |s: &str| -> Result<Value, TokenError> {
        let bytes = B64.decode(s).map_err(|_| invalid("malformed token"))?;
        serde_json::from_slice::<Value>(&bytes).ok().filter(Value::is_object).ok_or_else(|| invalid("malformed token"))
    };
    let header = decode_json(h)?;
    let claims = decode_json(p)?;
    let sig = B64.decode(sig).map_err(|_| invalid("malformed signature"))?;

    // Signature: an asymmetric algorithm, a key from the issuer's set.
    let alg: CoreJwsSigningAlgorithm =
        header.get("alg").cloned().and_then(|a| serde_json::from_value(a).ok()).ok_or_else(|| invalid("unsupported algorithm"))?;
    if matches!(
        alg,
        CoreJwsSigningAlgorithm::None
            | CoreJwsSigningAlgorithm::HmacSha256
            | CoreJwsSigningAlgorithm::HmacSha384
            | CoreJwsSigningAlgorithm::HmacSha512
    ) {
        return Err(invalid("unsupported algorithm"));
    }
    if header.get("crit").is_some() {
        return Err(invalid("unsupported critical header"));
    }
    let kid = header.get("kid").and_then(|k| k.as_str());
    let candidates: Vec<&CoreJsonWebKey> = keys
        .keys()
        .iter()
        .filter(|k| kid.is_none_or(|kid| k.key_id().is_some_and(|id| id.as_str() == kid)))
        .filter(|k| k.key_use().is_none_or(|u| u.allows_signature()))
        .collect();
    if candidates.is_empty() {
        return Err(TokenError::UnknownKey);
    }
    let message = format!("{h}.{p}");
    if !candidates.iter().any(|k| k.verify_signature(&alg, message.as_bytes(), &sig).is_ok()) {
        return Err(invalid("bad signature"));
    }

    // Claims.
    let iss = str_claim(&claims, "iss").ok_or_else(|| invalid("no iss"))?;
    if iss.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return Err(invalid(format!("issuer {iss} is not trusted")));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == client_id,
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if !aud_ok {
        return Err(invalid("audience does not include this client"));
    }
    let iat = num_claim(&claims, "iat")?.ok_or_else(|| invalid("no iat"))?;
    if iat > now_secs + LEEWAY_SECS {
        return Err(invalid("issued in the future"));
    }
    let exp = num_claim(&claims, "exp")?.ok_or_else(|| invalid("no exp"))?;
    if exp + LEEWAY_SECS < now_secs {
        return Err(invalid("expired"));
    }
    if let Some(nbf) = num_claim(&claims, "nbf")?
        && nbf - LEEWAY_SECS > now_secs
    {
        return Err(invalid("not valid yet"));
    }
    if !claims.get("events").and_then(|e| e.as_object()).is_some_and(|e| e.contains_key(BACKCHANNEL_LOGOUT_EVENT)) {
        return Err(invalid("not a logout token"));
    }
    if claims.get("nonce").is_some_and(|n| !n.is_null()) {
        return Err(invalid("a logout token carries no nonce"));
    }
    let subject = str_claim(&claims, "sub");
    let sid = str_claim(&claims, "sid");
    if subject.is_none() && sid.is_none() {
        return Err(invalid("logout token names neither an account nor a session"));
    }
    Ok(LogoutEvent { issuer: iss, subject, sid })
}

/// Why a logout request was not handled.
enum Failure {
    /// The token is not valid: 400.
    Invalid(String),
    /// Reef cannot tell right now (provider unreachable, database): 503, the provider retries.
    Unavailable(String),
}

/// The provider's signing keys; rediscovered (at most every `KEY_REFRESH_MS`) when `refresh`.
async fn signing_keys(s: &AppState, refresh: bool) -> Result<Arc<CoreJsonWebKeySet>, String> {
    if let Some(k) = s.oidc.keys.read().await.as_ref()
        && (!refresh || reef_core::time::now_ms() - k.fetched_ms < KEY_REFRESH_MS)
    {
        return Ok(k.set.clone());
    }
    if refresh {
        *s.oidc.client.write().await = None;
    }
    crate::oidc::client(s).await.map_err(|e| e.0.to_string())?;
    s.oidc.keys.read().await.as_ref().map(|k| k.set.clone()).ok_or_else(|| "no signing keys".to_string())
}

async fn verify(s: &AppState, token: &str) -> Result<LogoutEvent, Failure> {
    // Garbage is refused before the provider is asked for its keys.
    if crate::actions::jwt::peek(token).is_none() {
        return Err(Failure::Invalid("malformed token".into()));
    }
    let cfg = s.config.oidc().ok_or_else(|| Failure::Invalid("no identity provider is configured".into()))?;
    let now = reef_core::time::now_ms() / 1000;
    let keys = signing_keys(s, false).await.map_err(Failure::Unavailable)?;
    match verify_logout_token(token, &cfg.issuer, &cfg.client_id, &keys, now) {
        Ok(ev) => Ok(ev),
        Err(TokenError::Invalid(m)) => Err(Failure::Invalid(m)),
        Err(TokenError::UnknownKey) => {
            // A rotated key: rediscover once.
            let keys = signing_keys(s, true).await.map_err(Failure::Unavailable)?;
            verify_logout_token(token, &cfg.issuer, &cfg.client_id, &keys, now).map_err(|e| Failure::Invalid(e.to_string()))
        }
    }
}

/// End the Reef web sessions a logout event is about and forget what was cached for them.
/// Returns how many sessions ended.
pub async fn end_sessions(s: &AppState, ev: &LogoutEvent) -> reef_core::Result<u64> {
    if let Some(sid) = &ev.sid {
        return s.db.delete_sessions_by_sid(sid).await;
    }
    let Some(sub) = &ev.subject else { return Ok(0) };
    let Some(user) = s.db.external_identity_user(&ev.issuer, sub).await? else { return Ok(0) };
    // Everything of the account ended (suspension, deletion, "sign out everywhere").
    s.db.delete_user_sessions(user).await
}

fn reply(status: StatusCode, body: Option<&'static str>) -> Response {
    let mut res = match body {
        Some(b) => (status, [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))], b).into_response(),
        None => status.into_response(),
    };
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// `POST /login/oidc/backchannel-logout` (`application/x-www-form-urlencoded`,
/// `logout_token=<JWT>`). Unauthenticated: the signed token is the authentication, and no
/// session or CSRF check applies (it is a server-to-server call).
pub async fn backchannel_logout(State(s): State<AppState>, body: Bytes) -> Response {
    if s.config.oidc().is_none() {
        return reply(StatusCode::NOT_FOUND, None);
    }
    let token = serde_urlencoded::from_bytes::<Vec<(String, String)>>(&body)
        .ok()
        .and_then(|form| form.into_iter().find(|(k, _)| k == "logout_token").map(|(_, v)| v))
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        tracing::warn!("back-channel logout without a logout_token");
        return reply(StatusCode::BAD_REQUEST, Some(r#"{"error":"invalid_request"}"#));
    };
    let ev = match verify(&s, &token).await {
        Ok(ev) => ev,
        Err(Failure::Invalid(reason)) => {
            tracing::warn!(%reason, "refused a logout token");
            return reply(StatusCode::BAD_REQUEST, Some(r#"{"error":"invalid_request"}"#));
        }
        Err(Failure::Unavailable(error)) => {
            tracing::warn!(%error, "cannot verify a logout token now");
            return reply(StatusCode::SERVICE_UNAVAILABLE, Some(r#"{"error":"temporarily_unavailable"}"#));
        }
    };
    match end_sessions(&s, &ev).await {
        Ok(n) => {
            tracing::info!(
                sub = ev.subject.as_deref().unwrap_or(""),
                sid = ev.sid.as_deref().unwrap_or(""),
                sessions = n,
                "back-channel logout"
            );
            reply(StatusCode::OK, None)
        }
        Err(error) => {
            tracing::warn!(%error, "back-channel logout: could not end sessions");
            reply(StatusCode::SERVICE_UNAVAILABLE, Some(r#"{"error":"temporarily_unavailable"}"#))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc::SigningKeys;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use p256::ecdsa::signature::Signer;
    use p256::ecdsa::{Signature, SigningKey};
    use reef_db::accounts::NewUser;
    use reef_db::auth::SignIn;
    use serde_json::json;
    use tower::ServiceExt;

    const ISSUER: &str = "https://id.test";
    const CLIENT: &str = "reef-test";
    const KID: &str = "test-1-key";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32].into()).expect("test key")
    }

    fn jwks(sk: &SigningKey, kid: &str) -> CoreJsonWebKeySet {
        let pt = sk.verifying_key().to_encoded_point(false);
        let jwk = json!({"kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": kid, "x": B64.encode(pt.x().unwrap()), "y": B64.encode(pt.y().unwrap())});
        serde_json::from_value(json!({"keys": [jwk]})).expect("jwks")
    }

    fn sign(sk: &SigningKey, header: Value, claims: Value) -> String {
        let signing = format!("{}.{}", B64.encode(header.to_string()), B64.encode(claims.to_string()));
        let sig: Signature = sk.sign(signing.as_bytes());
        format!("{signing}.{}", B64.encode(sig.to_bytes()))
    }

    fn now() -> i64 {
        reef_core::time::now_ms() / 1000
    }

    /// Claims as a provider sends them (`sid` omitted when `None`).
    fn claims(sub: &str, sid: Option<&str>) -> Value {
        let mut c = json!({"iss": ISSUER, "sub": sub, "aud": [CLIENT], "iat": now(), "exp": now() + 120, "jti": "j1", "events": {BACKCHANNEL_LOGOUT_EVENT: {}}});
        if let Some(sid) = sid {
            c["sid"] = json!(sid);
        }
        c
    }

    fn header() -> Value {
        json!({"alg": "ES256", "kid": KID, "typ": "logout+jwt"})
    }

    fn mint(claims: Value) -> String {
        sign(&key(7), header(), claims)
    }

    fn check(token: &str) -> Result<LogoutEvent, TokenError> {
        verify_logout_token(token, ISSUER, CLIENT, &jwks(&key(7), KID), now())
    }

    #[test]
    fn verifies_logout_tokens() {
        let ev = check(&mint(claims("sub-alice", Some("sid-1")))).unwrap();
        assert_eq!(ev, LogoutEvent { issuer: ISSUER.into(), subject: Some("sub-alice".into()), sid: Some("sid-1".into()) });
        let ev = check(&mint(claims("sub-alice", None))).unwrap();
        assert_eq!(ev.sid, None);
        // sid alone is enough; a plain string audience too; trailing slash on the issuer.
        let mut c = claims("", Some("sid-2"));
        c.as_object_mut().unwrap().remove("sub");
        c["aud"] = json!(CLIENT);
        c["iss"] = json!(format!("{ISSUER}/"));
        assert_eq!(check(&mint(c)).unwrap().sid.as_deref(), Some("sid-2"));
        // Within the leeway.
        let mut c = claims("sub-alice", None);
        c["exp"] = json!(now() - 30);
        assert!(check(&mint(c)).is_ok());
    }

    #[test]
    fn refuses_what_is_not_a_logout_token_for_us() {
        let bad = |mutate: &dyn Fn(&mut Value)| {
            let mut c = claims("sub-alice", Some("sid-1"));
            mutate(&mut c);
            check(&mint(c))
        };
        let reason = |r: Result<LogoutEvent, TokenError>| match r {
            Err(TokenError::Invalid(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        };
        assert!(reason(bad(&|c| c["aud"] = json!(["someone-else"]))).contains("audience"));
        assert!(reason(bad(&|c| c["iss"] = json!("https://evil.test"))).contains("not trusted"));
        assert!(
            reason(bad(&|c| {
                c.as_object_mut().unwrap().remove("events");
            }))
            .contains("not a logout token")
        );
        assert!(reason(bad(&|c| c["events"] = json!({"http://schemas.openid.net/event/other": {}}))).contains("not a logout token"));
        assert!(reason(bad(&|c| c["nonce"] = json!("n-1"))).contains("nonce"));
        assert!(
            reason(bad(&|c| {
                let m = c.as_object_mut().unwrap();
                m.remove("sub");
                m.remove("sid");
            }))
            .contains("neither")
        );
        assert!(
            reason(bad(&|c| {
                c.as_object_mut().unwrap().remove("iat");
            }))
            .contains("no iat")
        );
        assert!(reason(bad(&|c| c["iat"] = json!(now() + 600))).contains("future"));
        assert!(reason(bad(&|c| c["exp"] = json!(now() - 120))).contains("expired"));
        assert!(
            reason(bad(&|c| {
                c.as_object_mut().unwrap().remove("exp");
            }))
            .contains("no exp")
        );
        assert!(reason(bad(&|c| c["nbf"] = json!(now() + 600))).contains("not valid yet"));

        // Signatures: another key under the same kid, an unknown kid, none/HS256, tampering.
        let c = claims("sub-alice", Some("sid-1"));
        assert_eq!(reason(check(&sign(&key(9), header(), c.clone()))), "bad signature", "wrong key");
        assert_eq!(check(&sign(&key(7), json!({"alg": "ES256", "kid": "rotated"}), c.clone())), Err(TokenError::UnknownKey));
        assert_eq!(reason(check(&sign(&key(7), json!({"alg": "none", "kid": KID}), c.clone()))), "unsupported algorithm");
        assert_eq!(reason(check(&sign(&key(7), json!({"alg": "HS256", "kid": KID}), c.clone()))), "unsupported algorithm");
        let t = mint(c.clone());
        let mut parts: Vec<&str> = t.split('.').collect();
        let forged = B64.encode(claims("gk-bob", None).to_string());
        parts[1] = &forged;
        assert_eq!(reason(check(&parts.join("."))), "bad signature", "tampered claims");
        assert_eq!(reason(check("not-a-jwt")), "malformed token");
    }

    #[test]
    fn logout_tokens_are_recognised() {
        assert!(is_logout_token(&mint(claims("sub-alice", Some("sid-1")))));
        assert!(is_logout_token(&sign(&key(7), json!({"alg": "ES256", "typ": "JWT"}), claims("sub-alice", None))), "events claim");
        assert!(is_logout_token(&sign(&key(7), json!({"alg": "ES256", "typ": "logout+jwt"}), json!({"sub": "x"}))), "typ");
        assert!(!is_logout_token(&sign(&key(7), json!({"alg": "ES256", "typ": "JWT"}), json!({"sub": "x"}))));
        assert!(!is_logout_token("fgp_0123456789abcdef"));
        assert!(!is_logout_token("gkp_0123456789abcdef"));
        assert_eq!(sid_claim(&json!({"sid": "s-1"})).as_deref(), Some("s-1"));
        assert_eq!(sid_claim(&json!({"sid": ""})), None);
    }

    #[test]
    fn registers_the_endpoint_next_to_the_oidc_routes() {
        let cfg = Config { site_url: "https://reef.example.com/".into(), ..Default::default() };
        assert_eq!(uri(&cfg), "https://reef.example.com/login/oidc/backchannel-logout");
    }

    // ----- the endpoint, against a real (SQLite) database and the router -----

    async fn state() -> AppState {
        let dir = std::env::temp_dir().join(format!("reef-logout-test-{}", reef_core::token::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config { data_dir: dir.clone(), ..Default::default() };
        cfg.auth.mode = "oidc".into();
        cfg.oidc = Some(crate::config::OidcConfig { issuer: ISSUER.into(), client_id: CLIENT.into(), ..Default::default() });
        let db = reef_db::Db::connect(&cfg.database_url()).await.unwrap();
        let store = reef_git::Store::new(dir.join("repos"), "git");
        let s = AppState::new(db, store, cfg).unwrap();
        // The provider's keys, as discovery would have left them.
        *s.oidc.keys.write().await = Some(SigningKeys { set: Arc::new(jwks(&key(7), KID)), fetched_ms: reef_core::time::now_ms() });
        *s.oidc.end_session_endpoint.write().await = Some(format!("{ISSUER}/end_session"));
        s
    }

    async fn post(s: &AppState, body: String, extra: &[(&str, &str)]) -> (StatusCode, String, Option<String>) {
        let mut req = Request::post(PATH).header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let res = crate::router(s.clone()).oneshot(req.body(Body::from(body)).unwrap()).await.unwrap();
        let status = res.status();
        let cc = res.headers().get(header::CACHE_CONTROL).and_then(|v| v.to_str().ok()).map(String::from);
        let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
        (status, body, cc)
    }

    /// A provider sign-in with an ID token.
    fn gk(sid: &str) -> SignIn {
        SignIn { sid: Some(sid.into()), id_token: Some(format!("id-token-of-{sid}")) }
    }

    fn form(token: &str) -> String {
        format!("logout_token={token}")
    }

    async fn alive(s: &AppState, cookie: &str) -> bool {
        s.db.session_by_cookie(cookie).await.is_ok()
    }

    #[tokio::test]
    async fn backchannel_logout_ends_the_sign_in() {
        let s = state().await;
        let alice =
            s.db.create_user(NewUser {
                login: "alice",
                email: "alice@example.com",
                password: Some("correct horse"),
                name: None,
                is_admin: false,
            })
            .await
            .unwrap();
        let bob =
            s.db.create_user(NewUser { login: "bob", email: "bob@example.com", password: None, name: None, is_admin: false })
                .await
                .unwrap();
        s.db.link_external_identity(alice.id(), ISSUER, "sub-alice", None).await.unwrap();
        s.db.link_external_identity(bob.id(), ISSUER, "gk-bob", None).await.unwrap();
        let a1 = s.db.create_sign_in_session(alice.id(), None, None, &gk("sid-1")).await.unwrap();
        let a2 = s.db.create_sign_in_session(alice.id(), None, None, &gk("sid-2")).await.unwrap();
        let a3 = s.db.create_session(alice.id(), None, None).await.unwrap();
        let b1 = s.db.create_sign_in_session(bob.id(), None, None, &gk("sid-b")).await.unwrap();
        assert_eq!(s.db.session_by_cookie(&a1).await.unwrap().sid.as_deref(), Some("sid-1"));
        assert_eq!(s.db.session_by_cookie(&a3).await.unwrap().sid, None);
        let (pat, _) = s.db.create_token(alice.id(), "pat", "laptop", &["repo"], None, None).await.unwrap();

        // Refused tokens change nothing and are not retried (4xx).
        for c in [
            {
                let mut c = claims("sub-alice", Some("sid-1"));
                c["aud"] = json!(["other-client"]);
                c
            },
            {
                let mut c = claims("sub-alice", Some("sid-1"));
                c["iss"] = json!("https://evil.test");
                c
            },
            {
                let mut c = claims("sub-alice", Some("sid-1"));
                c.as_object_mut().unwrap().remove("events");
                c
            },
            {
                let mut c = claims("sub-alice", Some("sid-1"));
                c["nonce"] = json!("n");
                c
            },
        ] {
            let (status, body, cc) = post(&s, form(&mint(c)), &[]).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body, r#"{"error":"invalid_request"}"#);
            assert_eq!(cc.as_deref(), Some("no-store"));
        }
        assert_eq!(post(&s, String::new(), &[]).await.0, StatusCode::BAD_REQUEST, "no logout_token");
        assert_eq!(post(&s, form("garbage"), &[]).await.0, StatusCode::BAD_REQUEST);
        assert!(alive(&s, &a1).await);

        // A logout token is no credential, whichever way it is presented.
        let token = mint(claims("sub-alice", Some("sid-1")));
        for auth in [format!("Bearer {token}"), format!("token {token}")] {
            let res = crate::router(s.clone())
                .oneshot(Request::get("/api/v3/user").header(header::AUTHORIZATION, auth).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }

        // sid: only that sign-in's session ends. A server-to-server POST, even one a browser
        // would mark cross-site, is not held to the CSRF check.
        let (status, body, cc) = post(&s, form(&token), &[("sec-fetch-site", "cross-site"), ("origin", "https://id.test")]).await;
        assert_eq!((status, body.as_str(), cc.as_deref()), (StatusCode::OK, "", Some("no-store")));
        assert!(!alive(&s, &a1).await);
        assert!(alive(&s, &a2).await && alive(&s, &a3).await && alive(&s, &b1).await);
        // Replays and unknown sessions are fine: nothing (more) to end.
        assert_eq!(post(&s, form(&token), &[]).await.0, StatusCode::OK);

        // sub only: every web session of the account ends; API tokens and other accounts stay.
        let (status, _, _) = post(&s, form(&mint(claims("sub-alice", None))), &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!alive(&s, &a2).await && !alive(&s, &a3).await);
        assert!(alive(&s, &b1).await);
        assert!(s.db.token_by_plain(&pat).await.is_ok());
        // An unknown subject is fine too.
        assert_eq!(post(&s, form(&mint(claims("gk-nobody", None))), &[]).await.0, StatusCode::OK);

        // Reef cannot act (database down): 503, so the provider retries.
        s.db.pool().close().await;
        let (status, body, _) = post(&s, form(&mint(claims("gk-bob", Some("sid-b")))), &[]).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, r#"{"error":"temporarily_unavailable"}"#);
    }

    #[tokio::test]
    async fn pending_identities_carry_the_sid() {
        let s = state().await;
        let id = s.db.create_pending_identity(ISSUER, "gk-carol", None, false, Some("carol"), None, None, &gk("sid-c")).await.unwrap();
        assert_eq!(s.db.pending_identity(&id).await.unwrap().sign_in(), gk("sid-c"));
        let id = s.db.create_pending_identity(ISSUER, "gk-dave", None, false, Some("dave"), None, None, &SignIn::default()).await.unwrap();
        assert!(!s.db.pending_identity(&id).await.unwrap().sign_in().is_oidc());
    }

    async fn sign_out(s: &AppState, cookie: &str) -> (StatusCode, Value, Option<String>) {
        let req = Request::delete("/api/reef/session")
            .header(header::COOKIE, format!("{}={cookie}", crate::auth::COOKIE))
            .body(Body::empty())
            .unwrap();
        let res = crate::router(s.clone()).oneshot(req).await.unwrap();
        let status = res.status();
        let set_cookie = res.headers().get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).map(String::from);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap(), set_cookie)
    }

    #[tokio::test]
    async fn signing_out_of_reef_ends_the_provider_sign_in() {
        let s = state().await;
        let alice =
            s.db.create_user(NewUser {
                login: "alice",
                email: "alice@example.com",
                password: Some("correct horse"),
                name: None,
                is_admin: false,
            })
            .await
            .unwrap();

        // A provider session: the browser is sent to end_session with the ID token as hint.
        let c = s.db.create_sign_in_session(alice.id(), None, None, &gk("sid-1")).await.unwrap();
        assert_eq!(s.db.session_sign_in(&c).await.unwrap(), gk("sid-1"));
        let (status, body, set_cookie) = sign_out(&s, &c).await;
        assert_eq!(status, StatusCode::OK);
        assert!(set_cookie.unwrap().contains("Max-Age=0"));
        assert!(!alive(&s, &c).await, "the local session ends first");
        let url = reqwest::Url::parse(body["logout_url"].as_str().expect("logout_url")).unwrap();
        assert_eq!(url.as_str().split('?').next(), Some("https://id.test/end_session"));
        let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        assert_eq!(q.get("id_token_hint").map(String::as_str), Some("id-token-of-sid-1"));
        assert_eq!(q.get("client_id").map(String::as_str), Some(CLIENT));
        assert_eq!(q.get("post_logout_redirect_uri").map(String::as_str), Some("http://localhost:4920/"));

        // Linked through a pending identity without an ID token: no hint, still end_session.
        let c = s.db.create_sign_in_session(alice.id(), None, None, &SignIn { sid: Some("sid-2".into()), id_token: None }).await.unwrap();
        let (_, body, _) = sign_out(&s, &c).await;
        let url = body["logout_url"].as_str().expect("logout_url");
        assert!(url.starts_with("https://id.test/end_session?client_id=reef-test&post_logout_redirect_uri="), "{url}");

        // A password session (and an unknown cookie) signs out of Reef only, as before.
        let c = s.db.create_session(alice.id(), None, None).await.unwrap();
        let (status, body, _) = sign_out(&s, &c).await;
        assert_eq!((status, &body), (StatusCode::OK, &json!({"logout_url": null})));
        assert!(!alive(&s, &c).await);
        assert_eq!(sign_out(&s, "fgs_unknown").await.1, json!({"logout_url": null}));
    }
}
