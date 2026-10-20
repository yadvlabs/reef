//! Git LFS: batch API, basic transfer adapter and file locks under `/{owner}/{repo}.git/info/lfs`
//! (spec: https://github.com/git-lfs/git-lfs/tree/main/docs/api). Objects are stored under
//! `<data_dir>/lfs/<repo_id>/<oid[0..2]>/<oid[2..4]>/<oid>`; metadata lives in `lfs_objects`.

use crate::AppState;
use crate::auth::Auth;
use crate::error::ApiError;
use crate::githttp::split_wiki;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::TryStreamExt;
use reef_core::time::iso;
use reef_core::{Error, Id};
use reef_db::repos::{Permission, RepoWithOwner};
use serde_json::{Value, json};
use sha2_010::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

pub const CONTENT_TYPE: &str = "application/vnd.git-lfs+json";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/info/lfs/objects/batch", post(batch))
        .route("/{owner}/{repo}/info/lfs/objects/verify", post(verify))
        .route("/{owner}/{repo}/info/lfs/objects/{oid}", get(download).put(upload))
        .route("/{owner}/{repo}/info/lfs/locks", get(list_locks).post(create_lock))
        .route("/{owner}/{repo}/info/lfs/locks/verify", post(verify_locks))
        .route("/{owner}/{repo}/info/lfs/locks/{id}/unlock", post(unlock))
}

fn lfs_json(status: StatusCode, v: Value) -> Response {
    (status, [(header::CONTENT_TYPE, CONTENT_TYPE)], v.to_string()).into_response()
}

fn fail(status: StatusCode, msg: impl Into<String>) -> Response {
    lfs_json(status, json!({"message": msg.into()}))
}

fn unauthorized() -> Response {
    let mut r = fail(StatusCode::UNAUTHORIZED, "Authentication required");
    r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"Reef\""));
    r.headers_mut().insert("LFS-Authenticate", HeaderValue::from_static("Basic realm=\"Reef\""));
    r
}

/// Where an object lives on disk.
pub fn object_path(s: &AppState, repo: Id, oid: &str) -> PathBuf {
    s.config.data_dir.join("lfs").join(repo.0.to_string()).join(&oid[..2]).join(&oid[2..4]).join(oid)
}

fn valid_oid(oid: &str) -> bool {
    oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Load the repository named in the URL and check the caller may read (or write) its objects.
async fn authorize(s: &AppState, auth: &Auth, owner: &str, repo: &str, write: bool) -> Result<RepoWithOwner, Response> {
    if !s.config.lfs.enabled {
        return Err(fail(StatusCode::NOT_FOUND, "Git LFS is disabled on this server"));
    }
    let (repo, wiki) = split_wiki(repo);
    if wiki {
        return Err(fail(StatusCode::NOT_FOUND, "Wikis do not support Git LFS"));
    }
    let rw = match s.db.repo_by_path(owner, repo).await {
        Ok(rw) => rw,
        Err(Error::NotFound) => {
            return Err(if auth.user.is_none() { unauthorized() } else { fail(StatusCode::NOT_FOUND, "Repository not found") });
        }
        Err(e) => return Err(ApiError(e).into_response()),
    };
    let perm = match s.db.permission(auth.user.as_ref(), &rw).await {
        Ok(p) => p,
        Err(e) => return Err(ApiError(e).into_response()),
    };
    let needed = if write { Permission::Write } else { Permission::Read };
    if perm < needed {
        return Err(if auth.user.is_none() { unauthorized() } else { fail(StatusCode::FORBIDDEN, "Permission denied") });
    }
    if write && rw.repo.archived != 0 {
        return Err(fail(StatusCode::FORBIDDEN, "Repository is archived"));
    }
    if write && !auth.has_scope("repo") {
        return Err(fail(StatusCode::FORBIDDEN, "Token lacks the repo scope"));
    }
    Ok(rw)
}

fn parse_json(body: &Bytes) -> Result<Value, Response> {
    serde_json::from_slice(body).map_err(|e| fail(StatusCode::BAD_REQUEST, format!("invalid JSON: {e}")))
}

fn objects_base(s: &AppState, rw: &RepoWithOwner) -> String {
    format!("{}/{}/{}.git/info/lfs/objects", s.config.site_url(), rw.owner.login, rw.repo.name)
}

/// Headers the client must repeat on transfer requests: its own Authorization header.
fn forwarded_auth(headers: &HeaderMap) -> Value {
    match headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        Some(a) => json!({"Authorization": a}),
        None => json!({}),
    }
}

#[derive(serde::Deserialize)]
struct BatchObject {
    oid: String,
    size: i64,
}

pub async fn batch(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let req = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let operation = req.get("operation").and_then(|v| v.as_str()).unwrap_or("");
    let write = match operation {
        "download" => false,
        "upload" => true,
        _ => return fail(StatusCode::UNPROCESSABLE_ENTITY, "operation must be download or upload"),
    };
    let rw = match authorize(&s, &auth, &owner, &repo, write).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    if let Some(t) = req.get("transfers").and_then(|v| v.as_array())
        && !t.is_empty()
        && !t.iter().any(|x| x.as_str() == Some("basic"))
    {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "only the basic transfer adapter is supported");
    }
    if let Some(algo) = req.get("hash_algo").and_then(|v| v.as_str())
        && algo != "sha256"
    {
        return fail(StatusCode::CONFLICT, "only sha256 object ids are supported");
    }
    let objects: Vec<BatchObject> = match serde_json::from_value(req.get("objects").cloned().unwrap_or(Value::Array(vec![]))) {
        Ok(o) => o,
        Err(e) => return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("objects: {e}")),
    };
    let oids: Vec<String> = objects.iter().map(|o| o.oid.clone()).collect();
    let present = match s.db.lfs_objects_present(rw.repo.id(), &oids).await {
        Ok(p) => p,
        Err(e) => return ApiError(e).into_response(),
    };
    let base = objects_base(&s, &rw);
    let hdr = forwarded_auth(&headers);
    let max = s.config.lfs.max_object_size as i64;
    let mut out = Vec::with_capacity(objects.len());
    for o in objects {
        let mut entry = json!({"oid": o.oid, "size": o.size, "authenticated": true});
        if !valid_oid(&o.oid) {
            entry["error"] = json!({"code": 422, "message": "invalid object id"});
        } else if write {
            if o.size < 0 || o.size > max {
                entry["error"] = json!({"code": 422, "message": format!("object exceeds the maximum size of {max} bytes")});
            } else if !present.contains(&o.oid) {
                entry["actions"] = json!({
                    "upload": {"href": format!("{base}/{}", o.oid), "header": hdr, "expires_in": 3600},
                    "verify": {"href": format!("{base}/verify"), "header": hdr, "expires_in": 3600},
                });
            }
        } else if present.contains(&o.oid) && object_path(&s, rw.repo.id(), &o.oid).exists() {
            entry["actions"] = json!({"download": {"href": format!("{base}/{}", o.oid), "header": hdr, "expires_in": 3600}});
        } else {
            entry["error"] = json!({"code": 404, "message": "Object does not exist"});
        }
        out.push(entry);
    }
    lfs_json(StatusCode::OK, json!({"transfer": "basic", "objects": out, "hash_algo": "sha256"}))
}

pub async fn download(State(s): State<AppState>, auth: Auth, Path((owner, repo, oid)): Path<(String, String, String)>) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, false).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    if !valid_oid(&oid) {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "invalid object id");
    }
    let meta = match s.db.lfs_object(rw.repo.id(), &oid).await {
        Ok(Some(m)) => m,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "Object does not exist"),
        Err(e) => return ApiError(e).into_response(),
    };
    let file = match tokio::fs::File::open(object_path(&s, rw.repo.id(), &oid)).await {
        Ok(f) => f,
        Err(_) => return fail(StatusCode::NOT_FOUND, "Object is missing from storage"),
    };
    let stream = tokio_util::io::ReaderStream::new(file);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, meta.size)
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .header(header::CONTENT_DISPOSITION, "attachment")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub async fn upload(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, oid)): Path<(String, String, String)>,
    req: Request,
) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, true).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    if !valid_oid(&oid) {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "invalid object id");
    }
    let max = s.config.lfs.max_object_size;
    if let Some(len) = req.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok())
        && len > max
    {
        return fail(StatusCode::INSUFFICIENT_STORAGE, format!("object exceeds the maximum size of {max} bytes"));
    }
    let dest = object_path(&s, rw.repo.id(), &oid);
    if dest.exists() {
        // Idempotent: the client may retry after a lost response.
        if let Ok(md) = tokio::fs::metadata(&dest).await {
            let _ = s.db.insert_lfs_object(rw.repo.id(), &oid, md.len() as i64).await;
        }
        return StatusCode::OK.into_response();
    }
    let tmp_dir = s.config.data_dir.join("lfs").join("tmp");
    if let Err(e) = tokio::fs::create_dir_all(&tmp_dir).await {
        return ApiError(Error::internal(e)).into_response();
    }
    let tmp = tmp_dir.join(format!("{}-{}", oid, reef_core::id::next().0));
    let mut file = match tokio::fs::File::create(&tmp).await {
        Ok(f) => f,
        Err(e) => return ApiError(Error::internal(e)).into_response(),
    };
    let mut hasher = Sha256::new();
    let mut total: u64 = 0;
    let mut stream = req.into_body().into_data_stream();
    let mut failure: Option<Response> = None;
    loop {
        match stream.try_next().await {
            Ok(Some(chunk)) => {
                total += chunk.len() as u64;
                if total > max {
                    failure = Some(fail(StatusCode::INSUFFICIENT_STORAGE, format!("object exceeds the maximum size of {max} bytes")));
                    break;
                }
                hasher.update(&chunk);
                if let Err(e) = file.write_all(&chunk).await {
                    failure = Some(ApiError(Error::internal(e)).into_response());
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                failure = Some(fail(StatusCode::BAD_REQUEST, format!("upload interrupted: {e}")));
                break;
            }
        }
    }
    if let Err(e) = file.flush().await {
        failure.get_or_insert(ApiError(Error::internal(e)).into_response());
    }
    drop(file);
    if let Some(r) = failure {
        let _ = tokio::fs::remove_file(&tmp).await;
        return r;
    }
    let digest = hex::encode(hasher.finalize());
    if digest != oid {
        let _ = tokio::fs::remove_file(&tmp).await;
        return fail(StatusCode::UNPROCESSABLE_ENTITY, format!("content does not match the object id (got sha256:{digest})"));
    }
    if let Some(parent) = dest.parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        let _ = tokio::fs::remove_file(&tmp).await;
        return ApiError(Error::internal(e)).into_response();
    }
    if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return ApiError(Error::internal(e)).into_response();
    }
    if let Err(e) = s.db.insert_lfs_object(rw.repo.id(), &oid, total as i64).await {
        return ApiError(e).into_response();
    }
    refresh_size(&s, &rw).await;
    StatusCode::OK.into_response()
}

pub async fn verify(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>, body: Bytes) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, true).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let req = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let oid = req.get("oid").and_then(|v| v.as_str()).unwrap_or("");
    let size = req.get("size").and_then(|v| v.as_i64()).unwrap_or(-1);
    if !valid_oid(oid) {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "invalid object id");
    }
    match s.db.lfs_object(rw.repo.id(), oid).await {
        Ok(Some(m)) if m.size == size && object_path(&s, rw.repo.id(), oid).exists() => StatusCode::OK.into_response(),
        Ok(Some(m)) => fail(StatusCode::UNPROCESSABLE_ENTITY, format!("size mismatch: stored {} bytes, expected {size}", m.size)),
        Ok(None) => fail(StatusCode::NOT_FOUND, "Object does not exist"),
        Err(e) => ApiError(e).into_response(),
    }
}

// ----- locks -----

async fn lock_json(s: &AppState, l: &reef_db::lfs::LfsLock) -> Value {
    let name = match s.db.account_by_id(Id(l.owner_id)).await {
        Ok(a) => a.login,
        Err(_) => "unknown".into(),
    };
    json!({"id": l.id.to_string(), "path": l.path, "locked_at": iso(l.created_at), "owner": {"name": name}})
}

pub async fn create_lock(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>, body: Bytes) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, true).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let Some(user) = auth.user.as_ref() else { return unauthorized() };
    let req = match parse_json(&body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = req.get("path").and_then(|v| v.as_str()).unwrap_or("").trim_matches('/');
    if path.is_empty() || path.split('/').any(|seg| seg == "..") {
        return fail(StatusCode::UNPROCESSABLE_ENTITY, "path is required");
    }
    match s.db.lfs_lock_by_path(rw.repo.id(), path).await {
        Ok(Some(existing)) => {
            let lock = lock_json(&s, &existing).await;
            return lfs_json(StatusCode::CONFLICT, json!({"lock": lock, "message": "already created lock"}));
        }
        Ok(None) => {}
        Err(e) => return ApiError(e).into_response(),
    }
    match s.db.create_lfs_lock(rw.repo.id(), path, user.id()).await {
        Ok(l) => {
            let lock = lock_json(&s, &l).await;
            lfs_json(StatusCode::CREATED, json!({"lock": lock}))
        }
        Err(Error::Conflict(_)) => fail(StatusCode::CONFLICT, "already created lock"),
        Err(e) => ApiError(e).into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct LocksQuery {
    path: Option<String>,
    id: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

pub async fn list_locks(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<LocksQuery>,
) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, false).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let locks = if let Some(id) = q.id.as_deref().and_then(|i| i.parse::<i64>().ok()) {
        match s.db.lfs_lock(rw.repo.id(), Id(id)).await {
            Ok(l) => l.into_iter().collect(),
            Err(e) => return ApiError(e).into_response(),
        }
    } else {
        let after = q.cursor.as_deref().and_then(|c| c.parse::<i64>().ok());
        match s.db.lfs_locks(rw.repo.id(), q.path.as_deref(), after, limit + 1).await {
            Ok(l) => l,
            Err(e) => return ApiError(e).into_response(),
        }
    };
    let next = if locks.len() as i64 > limit { locks.get(limit as usize - 1).map(|l| l.id.to_string()) } else { None };
    let mut out = Vec::new();
    for l in locks.iter().take(limit as usize) {
        out.push(lock_json(&s, l).await);
    }
    let mut v = json!({"locks": out});
    if let Some(n) = next {
        v["next_cursor"] = json!(n);
    }
    lfs_json(StatusCode::OK, v)
}

pub async fn verify_locks(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>, body: Bytes) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, true).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let Some(user) = auth.user.as_ref() else { return unauthorized() };
    let req = parse_json(&body).unwrap_or(Value::Null);
    let limit = req.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
    let after = req.get("cursor").and_then(|v| v.as_str()).and_then(|c| c.parse::<i64>().ok());
    let locks = match s.db.lfs_locks(rw.repo.id(), None, after, limit + 1).await {
        Ok(l) => l,
        Err(e) => return ApiError(e).into_response(),
    };
    let next = if locks.len() as i64 > limit { locks.get(limit as usize - 1).map(|l| l.id.to_string()) } else { None };
    let (mut ours, mut theirs) = (Vec::new(), Vec::new());
    for l in locks.iter().take(limit as usize) {
        let j = lock_json(&s, l).await;
        if l.owner_id == user.id {
            ours.push(j);
        } else {
            theirs.push(j);
        }
    }
    let mut v = json!({"ours": ours, "theirs": theirs});
    if let Some(n) = next {
        v["next_cursor"] = json!(n);
    }
    lfs_json(StatusCode::OK, v)
}

pub async fn unlock(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, String)>,
    body: Bytes,
) -> Response {
    let rw = match authorize(&s, &auth, &owner, &repo, true).await {
        Ok(rw) => rw,
        Err(r) => return r,
    };
    let Some(user) = auth.user.as_ref() else { return unauthorized() };
    let Ok(id) = id.parse::<i64>() else { return fail(StatusCode::NOT_FOUND, "lock not found") };
    let force = parse_json(&body).ok().and_then(|v| v.get("force").and_then(|f| f.as_bool())).unwrap_or(false);
    let lock = match s.db.lfs_lock(rw.repo.id(), Id(id)).await {
        Ok(Some(l)) => l,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "lock not found"),
        Err(e) => return ApiError(e).into_response(),
    };
    if lock.owner_id != user.id {
        let perm = s.db.permission(Some(user), &rw).await.unwrap_or(Permission::None);
        if !force {
            return fail(StatusCode::FORBIDDEN, "the lock is owned by another user; use --force to override");
        }
        if perm < Permission::Maintain {
            return fail(StatusCode::FORBIDDEN, "forcing another user's lock needs maintain access");
        }
    }
    let j = lock_json(&s, &lock).await;
    if let Err(e) = s.db.delete_lfs_lock(rw.repo.id(), Id(id)).await {
        return ApiError(e).into_response();
    }
    lfs_json(StatusCode::OK, json!({"lock": j}))
}

// ----- housekeeping -----

/// Recompute the repository size as git objects plus LFS bytes.
pub async fn refresh_size(s: &AppState, rw: &RepoWithOwner) {
    let disk = s.store.size_kb(rw.repo.id()).await.unwrap_or(rw.repo.size_kb);
    let lfs = s.db.lfs_stats(rw.repo.id()).await.map(|(_, bytes)| bytes / 1024).unwrap_or(0);
    let _ = s.db.set_repo_size_kb(rw.repo.id(), disk + lfs).await;
}

/// Remove LFS storage of repositories that no longer exist (repository deletion does not
/// know about LFS; this runs from the periodic tick).
pub async fn sweep_orphans(s: &AppState) {
    let root = s.config.data_dir.join("lfs");
    let Ok(mut dir) = tokio::fs::read_dir(&root).await else { return };
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(id) = name.parse::<i64>() else { continue };
        if !s.db.repo_exists(Id(id)).await.unwrap_or(true) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
            let _ = s.db.delete_lfs_objects(Id(id)).await;
            tracing::info!(repo = id, "removed LFS objects of a deleted repository");
        }
    }
    if let Ok(ids) = s.db.lfs_repo_ids().await {
        for id in ids {
            if !s.db.repo_exists(Id(id)).await.unwrap_or(true) {
                let _ = s.db.delete_lfs_objects(Id(id)).await;
            }
        }
    }
    // Abandoned uploads older than a day.
    if let Ok(mut tmp) = tokio::fs::read_dir(root.join("tmp")).await {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400);
        while let Ok(Some(entry)) = tmp.next_entry().await {
            if let Ok(md) = entry.metadata().await
                && md.modified().map(|m| m < cutoff).unwrap_or(false)
            {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

/// The LFS batch endpoint and a short-lived token for `git-lfs-authenticate` over SSH.
pub async fn ssh_authenticate(s: &AppState, user: &reef_db::accounts::Account, rw: &RepoWithOwner) -> reef_core::Result<Value> {
    let expires = reef_core::time::now_ms() + 3_600_000;
    let (plain, _) = s.db.create_token(user.id(), "lfs", "git-lfs-authenticate", &["repo"], None, Some(expires)).await?;
    Ok(json!({
        "href": format!("{}/{}/{}.git/info/lfs", s.config.site_url(), rw.owner.login, rw.repo.name),
        "header": {"Authorization": format!("Bearer {plain}")},
        "expires_in": 3600,
        "expires_at": iso(expires),
    }))
}

/// Drop expired short-lived LFS tokens (they would be rejected anyway; this keeps the table tidy).
pub async fn expire_tokens(s: &AppState) {
    let _ = reef_db::sqlx::query("DELETE FROM access_tokens WHERE kind = 'lfs' AND expires_at IS NOT NULL AND expires_at < $1")
        .bind(reef_core::time::now_ms())
        .execute(s.db.pool())
        .await;
}
