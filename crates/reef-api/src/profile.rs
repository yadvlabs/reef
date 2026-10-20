//! Profile extras beyond the GitHub API: uploaded avatars and pinned repositories, for users
//! and organizations.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::shapes;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use serde_json::{Value, json};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/user/avatar", post(upload_user_avatar).delete(delete_user_avatar))
        .route("/orgs/{org}/avatar", post(upload_org_avatar).delete(delete_org_avatar))
        .route("/users/{login}/pins", get(pins).put(set_pins))
        .route("/user/login", axum::routing::patch(rename_self))
        .route("/orgs/{org}/login", axum::routing::patch(rename_org))
}

const MAX_AVATAR: usize = 2 * 1024 * 1024;

pub fn avatar_dir(s: &AppState) -> std::path::PathBuf {
    s.config.data_dir.join("avatars")
}

/// Stored avatar bytes and their media type, if one was uploaded.
pub async fn stored_avatar(s: &AppState, id: i64) -> Option<(Vec<u8>, String)> {
    let dir = avatar_dir(s);
    let ct = tokio::fs::read_to_string(dir.join(format!("{id}.ct"))).await.ok()?;
    let bytes = tokio::fs::read(dir.join(format!("{id}.img"))).await.ok()?;
    Some((bytes, ct.trim().to_string()))
}

async fn save_avatar(s: &AppState, a: &Account, headers: &HeaderMap, body: Bytes) -> ApiResult<Value> {
    let ct =
        headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim().to_lowercase();
    if !matches!(ct.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp") {
        return Err(ApiError(Error::validation("avatar must be a PNG, JPEG, GIF or WebP image")));
    }
    if body.is_empty() || body.len() > MAX_AVATAR {
        return Err(ApiError(Error::validation("avatar must be between 1 byte and 2 MB")));
    }
    let dir = avatar_dir(s);
    tokio::fs::create_dir_all(&dir).await.map_err(Error::internal)?;
    tokio::fs::write(dir.join(format!("{}.img", a.id)), &body).await.map_err(Error::internal)?;
    tokio::fs::write(dir.join(format!("{}.ct", a.id)), &ct).await.map_err(Error::internal)?;
    s.db.touch_avatar(a.id(), true).await?;
    let a = s.db.account_by_id(a.id()).await?;
    Ok(json!({"avatar_url": shapes::avatar_url(&s.config, &a)}))
}

async fn remove_avatar(s: &AppState, a: &Account) -> ApiResult<StatusCode> {
    let dir = avatar_dir(s);
    let _ = tokio::fs::remove_file(dir.join(format!("{}.img", a.id))).await;
    let _ = tokio::fs::remove_file(dir.join(format!("{}.ct", a.id))).await;
    s.db.touch_avatar(a.id(), false).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload_user_avatar(State(s): State<AppState>, auth: Auth, headers: HeaderMap, body: Bytes) -> ApiResult<Json<Value>> {
    let u = auth.require()?.clone();
    Ok(Json(save_avatar(&s, &u, &headers, body).await?))
}

pub async fn delete_user_avatar(State(s): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    remove_avatar(&s, &u).await
}

async fn org_admin(s: &AppState, auth: &Auth, org: &str) -> ApiResult<Account> {
    let u = auth.require()?;
    let o = s.db.account_by_login(org).await?;
    if !o.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    let admin = u.is_admin() || s.db.org_membership(o.id(), u.id()).await?.map(|m| m.role == "admin").unwrap_or(false);
    if !admin {
        return Err(crate::error::forbidden("Must be an organization owner"));
    }
    Ok(o)
}

pub async fn upload_org_avatar(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    Ok(Json(save_avatar(&s, &o, &headers, body).await?))
}

pub async fn delete_org_avatar(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    remove_avatar(&s, &o).await
}

pub async fn pins(State(s): State<AppState>, auth: Auth, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let a = s.db.account_by_login(&login).await?;
    let mut out = Vec::new();
    for rw in s.db.pinned_repos(a.id()).await? {
        let perm = s.db.permission(auth.user.as_ref(), &rw).await?;
        if perm.can_read() {
            out.push(shapes::repo(&s.config, &rw, Some(perm)));
        }
    }
    Ok(Json(json!(out)))
}

#[derive(serde::Deserialize)]
pub struct SetPins {
    /// Repository names (`name` for the owner's own repositories, or `owner/name`).
    repos: Vec<String>,
}

pub async fn set_pins(State(s): State<AppState>, auth: Auth, Path(login): Path<String>, Json(b): Json<SetPins>) -> ApiResult<Json<Value>> {
    let u = auth.require()?.clone();
    let a = s.db.account_by_login(&login).await?;
    let allowed = u.is_admin()
        || u.id == a.id
        || (a.is_org() && s.db.org_membership(a.id(), u.id()).await?.map(|m| m.role == "admin").unwrap_or(false));
    if !allowed {
        return Err(crate::error::forbidden("You cannot change pins for this profile"));
    }
    let mut ids = Vec::new();
    for r in b.repos.iter().take(6) {
        let (o, n) = match r.split_once('/') {
            Some((o, n)) => (o.to_string(), n.to_string()),
            None => (a.login.clone(), r.clone()),
        };
        let rw = s.db.repo_by_path(&o, &n).await.map_err(|_| ApiError(Error::validation(format!("unknown repository {r}"))))?;
        if !s.db.permission(Some(&u), &rw).await?.can_read() {
            return Err(ApiError(Error::validation(format!("unknown repository {r}"))));
        }
        if rw.owner.id != a.id {
            // Pins may include repositories the profile contributes to; keep it simple: own only.
            return Err(ApiError(Error::validation(format!("{r} is not owned by {}", a.login))));
        }
        ids.push(rw.repo.id());
    }
    s.db.set_pinned_repos(a.id(), &ids).await?;
    pins(State(s), auth, Path(login)).await
}

pub fn _id(_: Id) {}

#[derive(serde::Deserialize)]
pub struct RenameBody {
    login: String,
}

/// Change your own username.
pub async fn rename_self(State(s): State<AppState>, auth: Auth, Json(b): Json<RenameBody>) -> ApiResult<Json<serde_json::Value>> {
    let u = auth.require()?;
    let new = b.login.trim();
    if new != u.login {
        s.db.rename_account(u.id(), new).await?;
    }
    let a = s.db.account_by_id(u.id()).await?;
    Ok(Json(crate::shapes::simple_user(&s.config, &a)))
}

/// Rename an organization (owners and site administrators).
pub async fn rename_org(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
    Json(b): Json<RenameBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let u = auth.require()?;
    let o = s.db.account_by_login(&org).await?;
    if !o.is_org() {
        return Err(crate::error::ApiError(reef_core::Error::NotFound));
    }
    crate::rest::orgs::require_org_admin(&s, u, &o).await?;
    let new = b.login.trim();
    if new != o.login {
        s.db.rename_account(o.id(), new).await?;
    }
    let o = s.db.account_by_id(o.id()).await?;
    Ok(Json(crate::shapes::org_simple(&s.config, &o)))
}
