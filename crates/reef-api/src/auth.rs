//! Who is calling. Resolves, in order: `Authorization: Bearer|token <secret>`, HTTP Basic
//! (`user:password` or `user:token`), then the session cookie.

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{Method, header};
use base64::Engine;
use reef_core::Error;
use reef_db::accounts::Account;

pub const COOKIE: &str = "reef_session";
const FULL_SCOPES: &str = "repo, read:org, workflow, admin:public_key, admin:org, delete_repo, user";

#[derive(Debug, Clone)]
pub enum Via {
    Anonymous,
    Session,
    Token { scopes: Vec<String> },
    Basic,
}

#[derive(Debug, Clone)]
pub struct Auth {
    pub user: Option<Account>,
    pub via: Via,
}

impl Auth {
    pub fn anonymous() -> Auth {
        Auth { user: None, via: Via::Anonymous }
    }

    pub fn require(&self) -> ApiResult<&Account> {
        self.user.as_ref().ok_or_else(crate::error::unauthorized)
    }

    pub fn require_admin(&self) -> ApiResult<&Account> {
        let u = self.require()?;
        if u.is_admin() { Ok(u) } else { Err(crate::error::forbidden("Must be a site administrator")) }
    }

    /// The value for the `X-OAuth-Scopes` header. Sessions and passwords act like a
    /// fully-scoped token, which is what `gh` expects of a pasted classic token.
    pub fn scopes_header(&self) -> String {
        match &self.via {
            Via::Token { scopes } => scopes.join(", "),
            Via::Session | Via::Basic => FULL_SCOPES.into(),
            Via::Anonymous => String::new(),
        }
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        match &self.via {
            Via::Token { scopes } => scopes.iter().any(|s| s == scope || implied(s, scope)),
            Via::Session | Via::Basic => true,
            Via::Anonymous => false,
        }
    }
}

/// Classic-token scope hierarchy: `repo` implies `public_repo`, `admin:org` implies `read:org`, ...
fn implied(have: &str, want: &str) -> bool {
    matches!(
        (have, want),
        ("repo", "public_repo" | "repo:status" | "repo_deployment" | "repo:invite")
            | ("admin:org", "write:org" | "read:org")
            | ("write:org", "read:org")
            | ("admin:public_key", "write:public_key" | "read:public_key")
            | ("write:public_key", "read:public_key")
            | ("user", "read:user" | "user:email" | "user:follow")
            | ("write:packages", "read:packages")
    )
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    /// Resolved once per request: the outcome, success or failure, is cached in the request
    /// extensions, so the handler after the rate limiter neither re-hashes a password nor
    /// re-introspects a token.
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(a) = parts.extensions.get::<Auth>() {
            return Ok(a.clone());
        }
        if let Some(f) = parts.extensions.get::<AuthFailure>() {
            return Err(ApiError(f.to_error()));
        }
        match resolve(parts, state).await {
            Ok(auth) => {
                parts.extensions.insert(auth.clone());
                Ok(auth)
            }
            Err(e) => {
                let f = AuthFailure::of(&e.0);
                parts.extensions.insert(f);
                Err(e)
            }
        }
    }
}

/// A failed credential resolution, cached on the request (see `Auth::from_request_parts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthFailure {
    Unauthorized(String),
    Forbidden(String),
    NotFound,
    /// Anything else (the database was unreachable, ...): answered 500 again, not retried.
    Other(String),
}

impl AuthFailure {
    fn of(e: &Error) -> AuthFailure {
        match e {
            Error::Unauthorized(m) => AuthFailure::Unauthorized(m.clone()),
            Error::Forbidden(m) => AuthFailure::Forbidden(m.clone()),
            Error::NotFound => AuthFailure::NotFound,
            other => AuthFailure::Other(other.to_string()),
        }
    }

    fn to_error(&self) -> Error {
        match self {
            AuthFailure::Unauthorized(m) => Error::Unauthorized(m.clone()),
            AuthFailure::Forbidden(m) => Error::Forbidden(m.clone()),
            AuthFailure::NotFound => Error::NotFound,
            AuthFailure::Other(m) => Error::msg(m.clone()),
        }
    }
}

async fn resolve(parts: &Parts, state: &AppState) -> ApiResult<Auth> {
    if let Some(v) = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        let (scheme, rest) = v.split_once(' ').unwrap_or((v, ""));
        match scheme.to_ascii_lowercase().as_str() {
            "bearer" | "token" => return by_token(state, rest.trim()).await,
            "basic" => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(rest.trim())
                    .map_err(|_| Error::Unauthorized("Bad credentials".into()))?;
                let raw = String::from_utf8_lossy(&raw).into_owned();
                let (user, secret) = raw.split_once(':').unwrap_or((&raw, ""));
                // A token may be passed as either half of basic auth, as git clients do.
                let looks_like_token =
                    |s: &str| s.starts_with("fgp_") || s.starts_with("fgo_") || s.starts_with(reef_db::actions::JOB_TOKEN_PREFIX);
                if looks_like_token(secret) {
                    let a = by_token(state, secret).await?;
                    check_login_matches(&a, user)?;
                    return Ok(a);
                }
                if looks_like_token(user) {
                    return by_token(state, user).await;
                }
                if secret.is_empty() {
                    return Err(Error::Unauthorized("Bad credentials".into()).into());
                }
                let acct = state.db.verify_password(user, secret).await?;
                return Ok(Auth { user: Some(acct), via: Via::Basic });
            }
            _ => return Err(Error::Unauthorized("Bad credentials".into()).into()),
        }
    }
    if let Some(cookie) = cookie_value(parts, COOKIE) {
        match state.db.session_by_cookie(&cookie).await {
            Ok(s) => {
                let acct = state.db.account_by_id(reef_core::Id(s.user_id)).await?;
                if acct.suspended != 0 {
                    return Ok(Auth::anonymous());
                }
                if !safe_method(&parts.method) && !same_site(parts, state) {
                    return Err(Error::Forbidden("cross-site request rejected".into()).into());
                }
                return Ok(Auth { user: Some(acct), via: Via::Session });
            }
            Err(Error::NotFound) | Err(Error::Unauthorized(_)) => return Ok(Auth::anonymous()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Auth::anonymous())
}

pub const ACTIONS_BOT: &str = "reef-actions[bot]";

async fn by_token(state: &AppState, secret: &str) -> ApiResult<Auth> {
    // A provider's back-channel logout tokens are signed like its ID tokens; they say who signed
    // out and are never a credential.
    if crate::logout::is_logout_token(secret) {
        return Err(Error::Unauthorized("a logout token is not a credential".into()).into());
    }
    if secret.starts_with(reef_db::actions::JOB_TOKEN_PREFIX) {
        let t = state.db.job_token(secret).await.map_err(|_| Error::Unauthorized("Bad credentials".into()))?;
        let mut bot = state.db.ensure_system_user(ACTIONS_BOT, "Reef Actions").await?;
        bot.job_repo = Some(t.repo_id);
        bot.job_id = Some(t.job_id);
        let scopes: Vec<String> = ["repo", "workflow", "read:org"].iter().map(|s| s.to_string()).collect();
        return Ok(Auth { user: Some(bot), via: Via::Token { scopes } });
    }
    let t = state.db.token_by_plain(secret).await?;
    let acct = state.db.account_by_id(reef_core::Id(t.user_id)).await?;
    if acct.suspended != 0 {
        return Err(Error::Forbidden("account suspended".into()).into());
    }
    let scopes = t.scope_list().iter().map(|s| s.to_string()).collect();
    Ok(Auth { user: Some(acct), via: Via::Token { scopes } })
}

fn check_login_matches(a: &Auth, login: &str) -> ApiResult<()> {
    // GitHub ignores the username when a token is used as the password. So do we, except
    // that a clearly different login is rejected to catch misconfigured credentials.
    if login.is_empty() || login.contains('@') {
        return Ok(());
    }
    match &a.user {
        Some(u) if u.login.eq_ignore_ascii_case(login) => Ok(()),
        Some(_) => Ok(()),
        None => Err(crate::error::unauthorized()),
    }
}

fn safe_method(m: &Method) -> bool {
    matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// CSRF guard for cookie-authenticated mutations: modern browsers send `Sec-Fetch-Site`;
/// older ones send `Origin`. Non-browser clients (no such headers) pass.
fn same_site(parts: &Parts, state: &AppState) -> bool {
    if let Some(sfs) = parts.headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        return matches!(sfs, "same-origin" | "none");
    }
    if let Some(origin) = parts.headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        return origin.trim_end_matches('/') == state.config.site_url();
    }
    true
}

pub fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    for h in parts.headers.get_all(header::COOKIE) {
        let s = h.to_str().ok()?;
        for kv in s.split(';') {
            let kv = kv.trim();
            if let Some((k, v)) = kv.split_once('=')
                && k == name
            {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub fn session_cookie(state: &AppState, value: &str, max_age_secs: i64) -> String {
    format!(
        "{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{}",
        if state.config.is_https() { "; Secure" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed resolution is cached like a successful one: the handler's extraction after
    /// the rate limiter's does not verify the password again.
    #[tokio::test]
    async fn a_failed_resolution_is_not_repeated() {
        let s = crate::testing::state("authcache", |_| {}).await;
        crate::testing::user_with_token(&s, "cached").await;
        let basic = |pw: &str| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("cached:{pw}")));
        let (mut parts, _) =
            axum::http::Request::get("/api/v3/user").header("authorization", basic("wrong")).body(()).unwrap().into_parts();
        let Err(err) = Auth::from_request_parts(&mut parts, &s).await else { panic!("bad password") };
        assert!(matches!(err.0, Error::Unauthorized(_)));
        assert!(matches!(parts.extensions.get::<AuthFailure>(), Some(AuthFailure::Unauthorized(_))));
        // Were it resolved again, the (now correct) password would pass.
        parts.headers.insert(header::AUTHORIZATION, basic("correct horse").parse().unwrap());
        let Err(err) = Auth::from_request_parts(&mut parts, &s).await else { panic!("the cached failure answers") };
        assert!(matches!(err.0, Error::Unauthorized(_)));
        let (mut fresh, _) =
            axum::http::Request::get("/api/v3/user").header("authorization", basic("correct horse")).body(()).unwrap().into_parts();
        assert!(Auth::from_request_parts(&mut fresh, &s).await.is_ok(), "a new request resolves afresh");
    }
}
