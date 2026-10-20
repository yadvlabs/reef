//! Sign-in through an OpenID Connect provider: the
//! authorization code flow with PKCE. Accounts are matched by (issuer, subject), linked by
//! verified email when allowed, or created from the ID token claims.

use crate::AppState;
use crate::auth::session_cookie;
use crate::error::{ApiError, ApiResult};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    ProviderMetadataWithLogout, RedirectUrl, Scope, TokenResponse,
};
use reef_core::{Error, Id};
use reef_db::accounts::{Account, NewUser};
use std::sync::Arc;
use tokio::sync::RwLock;

type Client = CoreClient<
    openidconnect::EndpointSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointMaybeSet,
    openidconnect::EndpointMaybeSet,
>;

/// Provider metadata is discovered on first use and cached; a failure is retried next time.
#[derive(Default)]
pub struct Provider {
    pub(crate) client: RwLock<Option<Arc<Client>>>,
    /// The provider's signing keys as of the last discovery, for verifying logout tokens
    /// (`crate::logout`); rediscovered when a token names a key not in the set.
    pub(crate) keys: RwLock<Option<SigningKeys>>,
    /// The provider's `end_session_endpoint` (RP-initiated logout), from discovery.
    pub(crate) end_session_endpoint: RwLock<Option<String>>,
}

/// A copy of the provider's JWKS (the keys ID tokens are verified with). Refreshing it means
/// rediscovering the provider, which refreshes the ID token verifier's keys as well.
pub(crate) struct SigningKeys {
    pub(crate) set: Arc<CoreJsonWebKeySet>,
    /// When the set was fetched (ms), to rate-limit refreshes.
    pub(crate) fetched_ms: i64,
}

/// What we learn about a person from the provider, whichever channel it came through.
pub struct Identity {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub username: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
}

/// Outcome of resolving an identity to a local account.
pub enum Resolved {
    Account(Account),
    /// Unknown subject whose (unverified) email already belongs to a Reef account, which
    /// the person must link themselves from Settings.
    EmailTaken(String),
    /// Unknown subject whose username or email matches an existing account: the person should
    /// sign in to that account to link it (or explicitly ask for a separate account).
    NeedsLink {
        existing_login: String,
    },
}

/// Find or create the Reef account behind an identity: known link first, then a verified
/// email match, then a fresh account (first account becomes site admin).
pub async fn resolve_account(s: &AppState, id: &Identity) -> ApiResult<Resolved> {
    let Identity { issuer, subject, email, email_verified, username, name, .. } = id;
    // 1. Known identity.
    let mut account = s.db.user_by_external_identity(issuer, subject).await?;
    // 1b. Known under a former issuer URL of this same provider (`previous_issuers`): move it
    // to the current issuer. Only listed issuers: another provider may reuse the subject.
    if account.is_none()
        && let Some(from) = rekey_from_previous_issuer(s, issuer, subject).await?
    {
        account = s.db.user_by_external_identity(issuer, subject).await?;
        if let Some(a) = &account {
            tracing::info!(from = %from, to = %issuer, subject = %subject, login = %a.login, "moved provider identity to the current issuer");
        }
    }
    // 2. Link by verified email.
    if account.is_none()
        && s.config.oidc().is_some_and(|g| g.link_by_email)
        && *email_verified
        && let Some(e) = email
        && let Ok(a) = s.db.user_by_login_or_email(e).await
    {
        s.db.link_external_identity(a.id(), issuer, subject, Some(e)).await?;
        account = Some(a);
    }
    // 3. Create.
    if account.is_none() {
        if let Some(e) = email
            && let Ok(a) = s.db.user_by_login_or_email(e).await
        {
            return Ok(Resolved::NeedsLink { existing_login: a.login });
        }
        if let Some(u) = username
            && let Ok(a) = s.db.account_by_login(u).await
            && !a.is_org()
        {
            return Ok(Resolved::NeedsLink { existing_login: a.login });
        }
        let base = username
            .clone()
            .or_else(|| email.as_deref().and_then(|e| e.split('@').next().map(String::from)))
            .unwrap_or_else(|| format!("user-{}", &subject[..subject.len().min(8)]));
        let base: String = base
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .chars()
            .take(36)
            .collect();
        let base = if base.is_empty() { "user".to_string() } else { base };
        let mail = email.clone().unwrap_or_else(|| format!("{subject}@{}", hostname(issuer)));
        let first = s.db.count_accounts("user").await? == 0;
        let mut login = base.clone();
        let mut created = None;
        for attempt in 0..20 {
            if attempt > 0 {
                login = format!("{base}-{}", attempt + 1);
            }
            match s.db.create_user(NewUser { login: &login, email: &mail, password: None, name: name.as_deref(), is_admin: first }).await {
                Ok(a) => {
                    created = Some(a);
                    break;
                }
                Err(Error::Validation { .. }) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let a = created.ok_or_else(|| ApiError(Error::msg("could not allocate a login for the new account")))?;
        s.db.link_external_identity(a.id(), issuer, subject, email.as_deref()).await?;
        account = Some(a);
    }
    Ok(Resolved::Account(account.unwrap()))
}

/// Re-key (`previous`, `subject`) to (`issuer`, `subject`) for the first configured previous
/// issuer that has it; returns that issuer. A listed URL matches with or without a trailing
/// slash.
async fn rekey_from_previous_issuer(s: &AppState, issuer: &str, subject: &str) -> ApiResult<Option<String>> {
    let Some(g) = s.config.oidc() else { return Ok(None) };
    for p in &g.previous_issuers {
        let base = p.trim_end_matches('/');
        if base.is_empty() || base == issuer.trim_end_matches('/') {
            continue;
        }
        for from in [base.to_string(), format!("{base}/")] {
            if s.db.rekey_external_identity(&from, issuer, subject).await? {
                return Ok(Some(from));
            }
        }
    }
    Ok(None)
}

/// Create a fresh account for an identity regardless of name/email collisions on other
/// accounts (the person explicitly asked for a separate account).
pub async fn create_separate_account(s: &AppState, id: &Identity) -> ApiResult<Account> {
    let Identity { issuer, subject, email, username, name, .. } = id;
    let base = username
        .clone()
        .or_else(|| email.as_deref().and_then(|e| e.split('@').next().map(String::from)))
        .unwrap_or_else(|| format!("user-{}", &subject[..subject.len().min(8)]));
    let base: String = base
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(36)
        .collect();
    let base = if base.is_empty() { "user".to_string() } else { base };
    let mail = match email {
        Some(e) if s.db.user_by_login_or_email(e).await.is_err() => e.clone(),
        _ => format!("{subject}@{}", hostname(issuer)),
    };
    let mut login = base.clone();
    for attempt in 0..20 {
        if attempt > 0 {
            login = format!("{base}-{}", attempt + 1);
        }
        match s.db.create_user(NewUser { login: &login, email: &mail, password: None, name: name.as_deref(), is_admin: false }).await {
            Ok(a) => {
                s.db.link_external_identity(a.id(), issuer, subject, email.as_deref()).await?;
                return Ok(a);
            }
            Err(Error::Validation { .. }) | Err(Error::Conflict(_)) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ApiError(Error::msg("could not allocate a login for the new account")))
}

/// An HTTP client for the provider: no redirects, and `[oidc].ca_file` trusted.
fn http(s: &AppState) -> ApiResult<reqwest::Client> {
    s.config.provider_http().redirect(reqwest::redirect::Policy::none()).build().map_err(|e| ApiError(Error::internal(e)))
}

pub(crate) async fn client(s: &AppState) -> ApiResult<Arc<Client>> {
    if let Some(c) = s.oidc.client.read().await.as_ref() {
        return Ok(c.clone());
    }
    let Some(cfg) = s.config.oidc() else {
        return Err(ApiError(Error::NotFound));
    };
    let issuer =
        IssuerUrl::new(cfg.issuer.trim_end_matches('/').to_string()).map_err(|e| ApiError(Error::msg(format!("oidc issuer: {e}"))))?;
    let meta = ProviderMetadataWithLogout::discover_async(issuer, &http(s)?)
        .await
        .map_err(|e| ApiError(Error::msg(format!("oidc discovery failed: {e}"))))?;
    let keys = SigningKeys { set: Arc::new(CoreJsonWebKeySet::new(meta.jwks().keys().clone())), fetched_ms: reef_core::time::now_ms() };
    *s.oidc.keys.write().await = Some(keys);
    *s.oidc.end_session_endpoint.write().await = meta.additional_metadata().end_session_endpoint.as_ref().map(|u| u.url().to_string());
    let redirect = RedirectUrl::new(format!("{}/login/oidc/callback", s.config.site_url()))
        .map_err(|e| ApiError(Error::msg(format!("redirect url: {e}"))))?;
    let c = CoreClient::from_provider_metadata(
        meta,
        ClientId::new(cfg.client_id.clone()),
        s.config.oidc_client_secret().map(|v| ClientSecret::new(v.to_string())),
    )
    .set_redirect_uri(redirect);
    let c = Arc::new(c);
    *s.oidc.client.write().await = Some(c.clone());
    Ok(c)
}

/// Where the browser goes to end the provider sign-in after Reef's own sign-out (RP-initiated
/// logout): the provider's `end_session_endpoint` with `id_token_hint` (when the session kept
/// its ID token), `client_id` and `post_logout_redirect_uri = <site_url>/`. The provider may
/// then tell its other clients through the back channel. `None` when the provider has no such
/// endpoint or cannot be reached.
pub async fn end_session_url(s: &AppState, id_token: Option<&str>) -> Option<String> {
    let cfg = s.config.oidc()?;
    let known = s.oidc.end_session_endpoint.read().await.clone();
    let endpoint = match known {
        Some(e) => e,
        None => {
            if let Err(e) = client(s).await {
                tracing::warn!(error = %e.0, "sign-out: provider discovery failed; ending the Reef session only");
                return None;
            }
            s.oidc.end_session_endpoint.read().await.clone()?
        }
    };
    let mut url = reqwest::Url::parse(&endpoint).ok()?;
    {
        let mut q = url.query_pairs_mut();
        if let Some(t) = id_token.filter(|t| !t.is_empty()) {
            q.append_pair("id_token_hint", t);
        }
        q.append_pair("client_id", &cfg.client_id);
        q.append_pair("post_logout_redirect_uri", &format!("{}/", s.config.site_url()));
    }
    Some(url.into())
}

#[derive(serde::Deserialize, Default)]
pub struct StartQuery {
    return_to: Option<String>,
}

/// `GET /login/oidc`: send the browser to the provider.
pub async fn start(State(s): State<AppState>, Query(q): Query<StartQuery>) -> ApiResult<Response> {
    let c = client(&s).await?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut req = c
        .authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)
        .set_pkce_challenge(pkce_challenge);
    for sc in s.config.oidc().map(|g| &g.scopes[..]).unwrap_or(&[]) {
        if sc != "openid" {
            req = req.add_scope(Scope::new(sc.clone()));
        }
    }
    let (url, state, nonce) = req.url();
    let return_to = q.return_to.filter(|r| r.starts_with('/') && !r.starts_with("//"));
    s.db.create_login_state(state.secret(), nonce.secret(), pkce_verifier.secret(), return_to.as_deref()).await?;
    Ok(Redirect::to(url.as_str()).into_response())
}

#[derive(serde::Deserialize, Default)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// `GET /login/oidc/callback`: finish the flow and start a Reef session.
pub async fn callback(
    State(s): State<AppState>,
    headers: HeaderMap,
    peer: crate::ratelimit::Peer,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    if let Some(e) = q.error {
        return Ok(
            Redirect::to(&format!("/login?error={}", urlenc(&format!("{e}: {}", q.error_description.unwrap_or_default())))).into_response()
        );
    }
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return Err(ApiError(Error::validation("missing code or state")));
    };
    let st = s.db.take_login_state(&state).await?;
    let c = client(&s).await?;
    let tokens = c
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| ApiError(Error::msg(format!("token endpoint not configured: {e}"))))?
        .set_pkce_verifier(PkceCodeVerifier::new(st.pkce))
        .request_async(&http(&s)?)
        .await
        .map_err(|e| ApiError(Error::Unauthorized(format!("token exchange failed: {e}"))))?;
    let id_token = tokens.id_token().ok_or_else(|| ApiError(Error::Unauthorized("provider returned no ID token".into())))?;
    let claims = id_token
        .claims(&c.id_token_verifier(), &Nonce::new(st.nonce))
        .map_err(|e| ApiError(Error::Unauthorized(format!("invalid ID token: {e}"))))?;
    let issuer = claims.issuer().to_string();
    let subject = claims.subject().to_string();
    let email = claims.email().map(|e| e.to_string());
    let email_verified = claims.email_verified().unwrap_or(false);
    let username = claims.preferred_username().map(|u| u.to_string());
    let name = claims.name().and_then(|n| n.get(None)).map(|n| n.to_string());
    let picture = claims.picture().and_then(|p| p.get(None)).map(|p| p.to_string());
    // Raw claims: `sid`, the sign-in this session belongs to, so that a back-channel logout for
    // that sign-in ends it (`crate::logout`), and the optional admin claim.
    let raw_claims = crate::actions::jwt::peek(&id_token.to_string()).map(|(_, c)| c).unwrap_or_default();
    // The ID token is kept with the session as the `id_token_hint` for signing out.
    let sign_in = reef_db::auth::SignIn { sid: crate::logout::sid_claim(&raw_claims), id_token: Some(id_token.to_string()) };

    let ident = Identity { issuer, subject, email, email_verified, username, name, picture };
    let a = match resolve_account(&s, &ident).await? {
        Resolved::Account(a) => a,
        Resolved::EmailTaken(_) => return Ok(Redirect::to("/login?error=email-taken").into_response()),
        Resolved::NeedsLink { existing_login } => {
            let id =
                s.db.create_pending_identity(
                    &ident.issuer,
                    &ident.subject,
                    ident.email.as_deref(),
                    ident.email_verified,
                    ident.username.as_deref(),
                    ident.name.as_deref(),
                    st.return_to.as_deref(),
                    &sign_in,
                )
                .await?;
            return Ok(Redirect::to(&format!("/login?link={id}&hint={}", urlenc(&existing_login))).into_response());
        }
    };
    if ident.picture.is_some() {
        let _ = s.db.set_external_avatar(a.id(), ident.picture.as_deref()).await;
    }
    apply_admin_claim(&s, &a, &raw_claims).await;
    let a = s.db.account_by_id(a.id()).await?;
    if a.suspended != 0 {
        return Err(ApiError(Error::Forbidden("account suspended".into())));
    }
    let cookie =
        s.db.create_sign_in_session(
            a.id(),
            crate::ratelimit::client_address(&s, crate::ratelimit::peer_ip(&peer), &headers).as_deref(),
            headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()),
            &sign_in,
        )
        .await?;
    let to = st.return_to.unwrap_or_else(|| "/".into());
    Ok(([(header::SET_COOKIE, session_cookie(&s, &cookie, reef_db::auth::SESSION_TTL_MS / 1000))], Redirect::to(&to)).into_response())
}

fn hostname(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("provider").split(':').next().unwrap_or("provider").to_string()
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn _id(_: Id) {}

/// A provider claim may make the person a site administrator: when `[oidc] admin_claim`
/// names a boolean claim and the ID token carries it as `true`, the account is promoted.
/// Nothing is ever demoted here.
pub async fn apply_admin_claim(s: &AppState, a: &Account, claims: &serde_json::Value) {
    let Some(claim) = s.config.oidc().map(|g| g.admin_claim.trim()).filter(|c| !c.is_empty()) else { return };
    if claims.get(claim).and_then(|v| v.as_bool()) == Some(true) && !a.is_admin() {
        match s.db.set_admin(a.id(), true).await {
            Ok(()) => tracing::info!(login = %a.login, claim, "site administrator by provider claim"),
            Err(e) => tracing::warn!(error = %e, login = %a.login, "could not grant site administrator from the provider claim"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use crate::config::OidcConfig;

    const OLD: &str = "https://old.example.com";
    const NEW: &str = "https://id.example.com";

    async fn state(previous_issuers: &[&str]) -> AppState {
        let dir = std::env::temp_dir().join(format!("reef-oidc-test-{}", reef_core::token::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config { data_dir: dir.clone(), ..Default::default() };
        cfg.auth.mode = "oidc".into();
        cfg.oidc = Some(OidcConfig {
            issuer: NEW.into(),
            previous_issuers: previous_issuers.iter().map(|p| p.to_string()).collect(),
            ..Default::default()
        });
        let db = reef_db::Db::connect(&cfg.database_url()).await.unwrap();
        let store = reef_git::Store::new(dir.join("repos"), "git");
        AppState::new(db, store, cfg).unwrap()
    }

    /// An account linked under `issuer`, as the provider's old URL left it.
    async fn alice(s: &AppState, issuer: &str) -> Account {
        let a =
            s.db.create_user(NewUser { login: "alice", email: "alice@example.com", password: None, name: None, is_admin: false })
                .await
                .unwrap();
        s.db.link_external_identity(a.id(), issuer, "sub-alice", Some("alice@example.com")).await.unwrap();
        a
    }

    async fn resolve(s: &AppState) -> Resolved {
        resolve_account(s, &signing_in()).await.map_err(|e| e.0).unwrap()
    }

    /// The same person signing in after the provider's issuer URL changed.
    fn signing_in() -> Identity {
        Identity {
            issuer: NEW.into(),
            subject: "sub-alice".into(),
            email: Some("alice@example.com".into()),
            email_verified: true,
            username: Some("alice".into()),
            name: None,
            picture: None,
        }
    }

    #[tokio::test]
    async fn a_listed_previous_issuer_signs_the_account_in() {
        let s = state(&["https://other.example", OLD]).await;
        let a = alice(&s, OLD).await;
        match resolve(&s).await {
            Resolved::Account(got) => assert_eq!(got.id, a.id),
            _ => panic!("expected a sign-in"),
        }
        // Re-keyed once: the identity now lives under the current issuer only.
        assert_eq!(s.db.external_identity_user(NEW, "sub-alice").await.unwrap(), Some(a.id()));
        assert_eq!(s.db.external_identity_user(OLD, "sub-alice").await.unwrap(), None);
        assert!(matches!(resolve(&s).await, Resolved::Account(got) if got.id == a.id));
    }

    #[tokio::test]
    async fn a_trailing_slash_does_not_matter() {
        let s = state(&[&format!("{OLD}/")]).await;
        let a = alice(&s, OLD).await;
        assert!(matches!(resolve(&s).await, Resolved::Account(got) if got.id == a.id));
    }

    #[tokio::test]
    async fn an_unlisted_issuer_is_never_trusted() {
        let s = state(&["https://other.example"]).await;
        let a = alice(&s, OLD).await;
        // Same subject under an issuer that is not listed: pending a link, as before.
        match resolve(&s).await {
            Resolved::NeedsLink { existing_login } => assert_eq!(existing_login, "alice"),
            _ => panic!("expected a pending link"),
        }
        assert_eq!(s.db.external_identity_user(OLD, "sub-alice").await.unwrap(), Some(a.id()));
        assert_eq!(s.db.external_identity_user(NEW, "sub-alice").await.unwrap(), None);
    }
}
