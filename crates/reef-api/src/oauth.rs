//! Authentication endpoints.
//!
//! GitHub-shaped: the OAuth device flow (`/login/device/code`, `/login/oauth/access_token`)
//! that `gh auth login` uses. Reef-shaped (`/api/reef/...`): session sign-in and sign-up
//! for the web UI, device-code approval, personal access tokens, and organization creation.

use crate::AppState;
use crate::auth::{Auth, COOKIE, session_cookie};
use crate::error::{ApiError, ApiResult};
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use reef_core::{Error, Id};
use reef_db::accounts::NewUser;
use reef_db::device::Poll;
use serde_json::{Value, json};

pub fn github_router() -> Router<AppState> {
    Router::new()
        .route("/login/device/code", post(device_code))
        .route("/login/oauth/access_token", post(access_token))
        .route("/login/oauth/authorize", get(authorize_unsupported))
}

pub fn reef_router() -> Router<AppState> {
    Router::new()
        .route("/session", post(sign_in).delete(sign_out).get(session))
        .route("/session/link", get(pending_link))
        .route("/session/link-create", post(link_create))
        .route("/signup", post(sign_up))
        .route("/device", get(device_lookup).post(device_decide))
        .route("/tokens", get(list_tokens).post(create_token))
        .route("/tokens/{id}", axum::routing::delete(delete_token))
        .route("/orgs", post(create_org))
        .route("/password", post(change_password))
        .route("/identities", get(identities))
        .route("/setup", get(setup_status))
}

// ----- OAuth device flow (RFC 8628, GitHub dialect) -----

#[derive(serde::Deserialize)]
pub struct DeviceCodeReq {
    client_id: String,
    #[serde(default)]
    scope: String,
}

/// Accepts form or JSON bodies, answers in the format the `Accept` header asks for.
pub async fn device_code(State(s): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> ApiResult<Response> {
    let req: DeviceCodeReq = parse_body(&headers, &body)?;
    if req.client_id.is_empty() {
        return Err(ApiError(Error::validation("client_id is required")));
    }
    let scopes = req.scope.split([' ', ',']).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(",");
    let start = s.db.device_start(&req.client_id, &scopes).await?;
    let site = s.config.site_url();
    let fields = [
        ("device_code", start.device_code.clone()),
        ("user_code", start.user_code.clone()),
        ("verification_uri", format!("{site}/login/device")),
        ("expires_in", start.expires_in.to_string()),
        ("interval", start.interval.to_string()),
    ];
    Ok(respond(&headers, &fields, true))
}

#[derive(serde::Deserialize)]
pub struct AccessTokenReq {
    client_id: String,
    device_code: Option<String>,
    grant_type: Option<String>,
}

pub async fn access_token(State(s): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> ApiResult<Response> {
    let req: AccessTokenReq = parse_body(&headers, &body)?;
    let err = |code: &str, desc: &str| {
        let f = [
            ("error", code.to_string()),
            ("error_description", desc.to_string()),
            ("error_uri", "https://docs.github.com/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps".to_string()),
        ];
        respond(&headers, &f, false)
    };
    if req.grant_type.as_deref() != Some("urn:ietf:params:oauth:grant-type:device_code") {
        return Ok(err("unsupported_grant_type", "Only the device code grant is supported"));
    }
    let Some(code) = req.device_code else {
        return Ok(err("invalid_request", "device_code is required"));
    };
    let _ = req.client_id;
    match s.db.device_poll(&code).await? {
        Poll::Pending => Ok(err("authorization_pending", "The authorization request is still pending.")),
        Poll::SlowDown => Ok(err("slow_down", "Too many requests have been made in the same timeframe.")),
        Poll::Denied => Ok(err("access_denied", "The user denied the request.")),
        Poll::Expired => Ok(err("expired_token", "This device_code has expired.")),
        Poll::Granted { access_token, scopes } => {
            let f = [("access_token", access_token), ("token_type", "bearer".into()), ("scope", scopes)];
            Ok(respond(&headers, &f, true))
        }
    }
}

fn parse_body<T: serde::de::DeserializeOwned>(headers: &HeaderMap, body: &[u8]) -> ApiResult<T> {
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
    if ct.starts_with("application/json") {
        serde_json::from_slice(body).map_err(|e| ApiError(Error::validation(format!("invalid JSON: {e}"))))
    } else {
        serde_urlencoded::from_bytes(body).map_err(|e| ApiError(Error::validation(format!("invalid form body: {e}"))))
    }
}

/// GitHub answers these endpoints with form encoding unless `Accept: application/json`.
fn respond(headers: &HeaderMap, fields: &[(&str, String)], ok: bool) -> Response {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let status = StatusCode::OK;
    let _ = ok;
    if accept.contains("application/json") {
        let mut m = serde_json::Map::new();
        for (k, v) in fields {
            let val = v.parse::<i64>().map(Value::from).unwrap_or_else(|_| Value::from(v.clone()));
            m.insert(k.to_string(), val);
        }
        (status, Json(Value::Object(m))).into_response()
    } else {
        let body = fields.iter().map(|(k, v)| format!("{k}={}", urlencoding(v))).collect::<Vec<_>>().join("&");
        (status, [(header::CONTENT_TYPE, "application/x-www-form-urlencoded; charset=utf-8")], body).into_response()
    }
}

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub async fn authorize_unsupported() -> impl IntoResponse {
    (StatusCode::NOT_IMPLEMENTED, "The web application flow is not available yet. Use the device flow.")
}

// ----- Reef session API -----

#[derive(serde::Deserialize)]
pub struct SignIn {
    login: String,
    password: String,
    /// Pending identity to link to the account after a successful sign-in.
    link: Option<String>,
}

pub async fn sign_in(
    State(s): State<AppState>,
    headers: HeaderMap,
    peer: crate::ratelimit::Peer,
    Json(b): Json<SignIn>,
) -> ApiResult<Response> {
    if !s.config.standalone() {
        return Err(crate::error::forbidden("Password sign-in is disabled; sign in through the identity provider"));
    }
    let a = s.db.verify_password(b.login.trim(), &b.password).await?;
    let mut sign_in = reef_db::auth::SignIn::default();
    if let Some(link) = b.link.as_deref().filter(|l| !l.is_empty()) {
        let p = s.db.pending_identity(link).await?;
        s.db.link_external_identity(a.id(), &p.issuer, &p.subject, p.email.as_deref()).await?;
        s.db.delete_pending_identity(link).await?;
        // The browser arrived from a provider sign-in: its session ends with that sign-in.
        sign_in = p.sign_in();
    }
    let cookie =
        s.db.create_sign_in_session(
            a.id(),
            crate::ratelimit::client_address(&s, crate::ratelimit::peer_ip(&peer), &headers).as_deref(),
            headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()),
            &sign_in,
        )
        .await?;
    let n = s.db.count_repos_for_owner(a.id(), true).await?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&s, &cookie, reef_db::auth::SESSION_TTL_MS / 1000))],
        Json(shapes::full_user(&s.config, &a, n, true)),
    )
        .into_response())
}

#[derive(serde::Deserialize)]
pub struct LinkRef {
    link: String,
}

/// Describe a pending identity for the login page.
pub async fn pending_link(State(s): State<AppState>, Query(q): Query<LinkRef>) -> ApiResult<Json<Value>> {
    let p = s.db.pending_identity(&q.link).await?;
    Ok(Json(json!({"provider": provider_name(&s), "username": p.username, "email": p.email, "name": p.name, "return_to": p.return_to})))
}

/// The person wants a separate account for the pending identity rather than linking.
pub async fn link_create(
    State(s): State<AppState>,
    headers: HeaderMap,
    peer: crate::ratelimit::Peer,
    Json(b): Json<LinkRef>,
) -> ApiResult<Response> {
    let p = s.db.pending_identity(&b.link).await?;
    let ident = crate::oidc::Identity {
        issuer: p.issuer.clone(),
        subject: p.subject.clone(),
        email: p.email.clone(),
        email_verified: p.email_verified != 0,
        username: p.username.clone(),
        name: p.name.clone(),
        picture: None,
    };
    let a = crate::oidc::create_separate_account(&s, &ident).await?;
    s.db.delete_pending_identity(&b.link).await?;
    let cookie =
        s.db.create_sign_in_session(
            a.id(),
            crate::ratelimit::client_address(&s, crate::ratelimit::peer_ip(&peer), &headers).as_deref(),
            headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()),
            &p.sign_in(),
        )
        .await?;
    let n = s.db.count_repos_for_owner(a.id(), true).await?;
    Ok((
        [(header::SET_COOKIE, session_cookie(&s, &cookie, reef_db::auth::SESSION_TTL_MS / 1000))],
        Json(shapes::full_user(&s.config, &a, n, true)),
    )
        .into_response())
}

/// `DELETE /api/reef/session`: end this session. A session opened through the identity provider
/// answers `{"logout_url": ...}`, the provider's end_session URL, where the browser goes next so
/// the provider sign-in ends too; otherwise `{"logout_url": null}`.
pub async fn sign_out(State(s): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let parts = fake_parts(&headers);
    let mut logout_url = Value::Null;
    if let Some(c) = crate::auth::cookie_value(&parts, COOKIE) {
        let sign_in = s.db.session_sign_in(&c).await.unwrap_or_default();
        let _ = s.db.delete_session(&c).await;
        if sign_in.is_oidc()
            && let Some(url) = crate::oidc::end_session_url(&s, sign_in.id_token.as_deref()).await
        {
            logout_url = json!(url);
        }
    }
    Ok(([(header::SET_COOKIE, format!("{COOKIE}=; Path=/; HttpOnly; Max-Age=0"))], Json(json!({"logout_url": logout_url}))).into_response())
}

fn fake_parts(headers: &HeaderMap) -> axum::http::request::Parts {
    let mut req = axum::http::Request::new(());
    *req.headers_mut() = headers.clone();
    req.into_parts().0
}

pub async fn session(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    match &auth.user {
        Some(u) => {
            let n = s.db.count_repos_for_owner(u.id(), true).await?;
            let orgs = s.db.user_orgs(u.id()).await?;
            Ok(Json(json!({
                "user": shapes::full_user(&s.config, u, n, true),
                "orgs": orgs.iter().map(|o| shapes::org_simple(&s.config, o)).collect::<Vec<_>>(),
                "instance": instance_json(&s).await,
                "unread_notifications": s.db.unread_count(u.id()).await.unwrap_or(0),
            })))
        }
        None => Ok(Json(json!({"user": Value::Null, "orgs": [], "instance": instance_json(&s).await}))),
    }
}

/// The provider's display name for the sign-in button and messages (`[oidc] display_name`).
pub fn provider_name(s: &AppState) -> String {
    s.config.oidc().map(|g| g.display_name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "SSO".into())
}

async fn instance_json(s: &AppState) -> Value {
    let name = crate::admin::effective(s, "instance_name").await;
    let allow_signup = crate::admin::effective(s, "allow_signup").await == "true";
    let announcement = crate::admin::effective(s, "announcement").await;
    json!({
        "name": name,
        "site_url": s.config.site_url(),
        "api_url": s.config.api_url(),
        "ssh_host": s.config.ssh_host(),
        "ssh_port": s.config.ssh_port(),
        "allow_signup": allow_signup && s.config.standalone(),
        "announcement": if announcement.is_empty() { Value::Null } else { json!(crate::render::markdown(&announcement)) },
        "password_login": s.config.standalone(),
        "oidc": if s.config.oidc().is_some() { json!({"name": provider_name(s), "login_url": "/login/oidc"}) } else { Value::Null },
        "version": crate::VERSION,
    })
}

/// Setup state for the first-run experience: no users yet means the first sign-up is the
/// site administrator.
pub async fn setup_status(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let n = s.db.count_accounts("user").await?;
    let allow = crate::admin::effective(&s, "allow_signup").await == "true";
    Ok(Json(json!({"needs_setup": n == 0 && s.config.standalone(), "allow_signup": (allow || n == 0) && s.config.standalone()})))
}

#[derive(serde::Deserialize)]
pub struct SignUp {
    login: String,
    email: String,
    password: String,
    name: Option<String>,
}

pub async fn sign_up(
    State(s): State<AppState>,
    headers: HeaderMap,
    peer: crate::ratelimit::Peer,
    Json(b): Json<SignUp>,
) -> ApiResult<Response> {
    if !s.config.standalone() {
        return Err(crate::error::forbidden("Sign-up is disabled; accounts come from the identity provider"));
    }
    let first = s.db.count_accounts("user").await? == 0;
    if crate::admin::effective(&s, "allow_signup").await != "true" && !first {
        return Err(crate::error::forbidden("Sign-up is disabled on this instance"));
    }
    let a =
        s.db.create_user(NewUser {
            login: b.login.trim(),
            email: b.email.trim(),
            password: Some(&b.password),
            name: b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()),
            is_admin: first,
        })
        .await?;
    let cookie =
        s.db.create_session(
            a.id(),
            crate::ratelimit::client_address(&s, crate::ratelimit::peer_ip(&peer), &headers).as_deref(),
            headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, session_cookie(&s, &cookie, reef_db::auth::SESSION_TTL_MS / 1000))],
        Json(shapes::full_user(&s.config, &a, 0, true)),
    )
        .into_response())
}

#[derive(serde::Deserialize)]
pub struct DeviceLookup {
    user_code: String,
}

pub async fn device_lookup(State(s): State<AppState>, auth: Auth, Query(q): Query<DeviceLookup>) -> ApiResult<Json<Value>> {
    auth.require()?;
    let g = s.db.device_by_user_code(&q.user_code).await?;
    Ok(Json(json!({
        "user_code": g.user_code,
        "client_id": g.client_id,
        "client_name": reef_db::device::client_name(&g.client_id),
        "scopes": g.scopes.split(',').filter(|x| !x.is_empty()).collect::<Vec<_>>(),
        "status": g.status,
        "expires_at": reef_core::time::iso(g.expires_at),
    })))
}

#[derive(serde::Deserialize)]
pub struct DeviceDecision {
    user_code: String,
    #[serde(default = "yes")]
    approve: bool,
}
fn yes() -> bool {
    true
}

pub async fn device_decide(State(s): State<AppState>, auth: Auth, Json(b): Json<DeviceDecision>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    s.db.device_decide(&b.user_code, u.id(), b.approve).await?;
    Ok(Json(json!({"ok": true, "approved": b.approve})))
}

pub async fn list_tokens(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let ts = s.db.list_tokens(u.id()).await?;
    Ok(Json(json!(ts.iter().map(|t| shapes::token(t, None)).collect::<Vec<_>>())))
}

#[derive(serde::Deserialize)]
pub struct NewToken {
    name: String,
    #[serde(default)]
    scopes: Vec<String>,
    expires_in_days: Option<i64>,
}

pub async fn create_token(State(s): State<AppState>, auth: Auth, Json(b): Json<NewToken>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    if !matches!(auth.via, crate::auth::Via::Session | crate::auth::Via::Basic) {
        return Err(crate::error::forbidden("Tokens can only be created from a browser session or with a password"));
    }
    let name = b.name.trim();
    if name.is_empty() {
        return Err(ApiError(Error::validation("name is required")));
    }
    let scopes: Vec<&str> = if b.scopes.is_empty() {
        vec!["repo", "read:org", "workflow", "admin:public_key", "delete_repo", "user"]
    } else {
        b.scopes.iter().map(String::as_str).collect()
    };
    let expires = b.expires_in_days.filter(|d| *d > 0).map(|d| reef_core::time::now_ms() + d * 86_400_000);
    let (plain, t) = s.db.create_token(u.id(), "pat", name, &scopes, None, expires).await?;
    Ok((StatusCode::CREATED, Json(shapes::token(&t, Some(&plain)))))
}

pub async fn delete_token(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.delete_token(u.id(), Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct ChangePassword {
    current_password: Option<String>,
    new_password: String,
}

/// Change (or set, for accounts created through an identity provider) the password.
/// Every other session of the user is ended.
pub async fn change_password(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Json(b): Json<ChangePassword>,
) -> ApiResult<Response> {
    let u = auth.require()?;
    if !matches!(auth.via, crate::auth::Via::Session | crate::auth::Via::Basic) {
        return Err(crate::error::forbidden("Passwords can only be changed from a browser session"));
    }
    if s.db.has_password(u.id()).await? {
        let cur = b.current_password.unwrap_or_default();
        s.db.verify_password(&u.login, &cur)
            .await
            .map_err(|_| ApiError(Error::field("User", "current_password", "invalid", "current password is incorrect")))?;
    }
    s.db.set_password(u.id(), &b.new_password).await?;
    s.db.delete_user_sessions(u.id()).await?;
    let cookie = s.db.create_session(u.id(), None, headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok())).await?;
    Ok(([(header::SET_COOKIE, session_cookie(&s, &cookie, reef_db::auth::SESSION_TTL_MS / 1000))], Json(json!({"ok": true})))
        .into_response())
}

/// Linked identity providers and whether a password is set.
pub async fn identities(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let ids = s.db.external_identities(u.id()).await?;
    Ok(Json(json!({
        "has_password": s.db.has_password(u.id()).await?,
        "identities": ids.iter().map(|(iss, sub, email)| json!({"issuer": iss, "subject": sub, "email": email})).collect::<Vec<_>>(),
    })))
}

#[derive(serde::Deserialize)]
pub struct NewOrg {
    login: String,
    name: Option<String>,
    description: Option<String>,
}

pub async fn create_org(State(s): State<AppState>, auth: Auth, Json(b): Json<NewOrg>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let o =
        s.db.create_org(b.login.trim(), u.id(), b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()), b.description.as_deref())
            .await?;
    Ok((StatusCode::CREATED, Json(shapes::org_full(&s.config, &o, 0, true))))
}

pub fn _form(_: Form<()>) {}
