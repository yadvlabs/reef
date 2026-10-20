//! Pull mirrors (periodic fetch from another host), push mirrors (replicate every push to
//! other remotes) and source imports into empty repositories (GitHub's Source Import API
//! shape). Remote work runs in background tasks and from the 30-second tick.
//!
//! Every instance runs the tick, so a sync or push is claimed per mirror with a lease in the
//! database (`Db::claim_mirror`, `Db::claim_push_mirror`): the claiming instance renews the
//! lease while git runs and releases it with the outcome, and a lease left behind by a dead
//! instance expires after `LEASE_MS`. Renewal and release are compare-and-set on the lease
//! value, so an instance whose lease expired and was taken over can neither extend nor
//! release the new holder's: it aborts its own work (killing its git process) and records
//! nothing. Two instances therefore never sync the same mirror, and never speak for each
//! other's outcome.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::repos::load;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use reef_core::time::iso;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::mirrors::{MirrorConfig, MirrorPatch, PushMirror, RepoImport};
use reef_db::repos::RepoWithOwner;
use reef_git::ops::{self, RemoteAuth};
use reef_git::smart::RefUpdate;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::task::AbortHandle;

/// Reef API routes (`/api/reef/repos/{o}/{r}/mirror...`).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{owner}/{repo}/mirror", get(get_mirror).put(put_mirror).delete(delete_mirror))
        .route("/repos/{owner}/{repo}/mirror/sync", post(sync_now))
        .route("/repos/{owner}/{repo}/push-mirrors", get(list_push_mirrors).post(create_push_mirror))
        .route("/repos/{owner}/{repo}/push-mirrors/{id}", delete(delete_push_mirror))
        .route("/repos/{owner}/{repo}/push-mirrors/{id}/push", post(push_now))
        .route("/repos/{owner}/{repo}/lfs", get(lfs_overview))
        .route("/repos/{owner}/{repo}/lfs/locks/{id}", delete(lfs_force_unlock))
}

/// GitHub-shaped import routes (`/repos/{o}/{r}/import`), merged into `/api/v3`.
pub fn rest_router() -> Router<AppState> {
    Router::new().route("/repos/{owner}/{repo}/import", get(import_status).put(import_start).patch(import_start).delete(import_cancel))
}

// ----- shared helpers -----

/// Running imports, so a cancel can abort the fetch and a second start is refused.
static RUNNING_IMPORTS: Mutex<Option<HashMap<i64, AbortHandle>>> = Mutex::new(None);
/// Repositories with a push or sync in flight; keeps the tick from starting a second one.
static BUSY: Mutex<Option<HashSet<i64>>> = Mutex::new(None);
static TICKS: AtomicU64 = AtomicU64::new(0);

fn with_running<T>(f: impl FnOnce(&mut HashMap<i64, AbortHandle>) -> T) -> T {
    let mut g = RUNNING_IMPORTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

fn try_busy(id: i64) -> bool {
    let mut g = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashSet::new).insert(id)
}

fn done_busy(id: i64) {
    let mut g = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(set) = g.as_mut() {
        set.remove(&id);
    }
}

/// How long a claim on a mirror lasts without renewal, and how often a running sync renews it.
const LEASE_MS: i64 = 5 * 60_000;
const LEASE_RENEW: Duration = Duration::from_secs(60);

fn timeout(s: &AppState) -> Duration {
    s.config.mirror.timeout()
}

/// The outcome of work run under a lease.
enum Leased<T> {
    Done {
        held: i64,
        result: T,
    },
    /// The lease was lost to another instance; the work was dropped (which kills the git
    /// process it was running) and nothing may be recorded.
    Lost,
}

/// Run `work` while renewing the lease `held` every `LEASE_RENEW` through `renew` (a
/// compare-and-set that fails once the lease is no longer ours).
async fn with_lease<T, Fut>(held: i64, renew: impl Fn(i64) -> Fut, work: impl Future<Output = T>) -> Leased<T>
where
    Fut: Future<Output = reef_core::Result<Option<i64>>>,
{
    tokio::pin!(work);
    let mut held = held;
    loop {
        tokio::select! {
            result = &mut work => return Leased::Done { held, result },
            _ = tokio::time::sleep(LEASE_RENEW) => match renew(held).await {
                Ok(Some(h)) => held = h,
                Ok(None) => return Leased::Lost,
                Err(e) => tracing::warn!(error = %e, "mirror: renewing the lease failed; retrying"),
            },
        }
    }
}

/// Accept `https://`, `http://`, `ssh://`, `git://` and scp-style `host:path` URLs; local
/// paths and `file://` only when `[mirror] allow_file_urls` is on.
/// Scheme and shape checks, then the outbound address guard (see `outbound`).
pub async fn validate_remote_url(s: &AppState, url: &str) -> ApiResult<String> {
    let url = validate_remote_shape(s, url)?;
    crate::outbound::check_url(s, &url).await.map_err(|r| ApiError(Error::validation(format!("remote URL: {r}"))))?;
    Ok(url)
}

fn validate_remote_shape(s: &AppState, url: &str) -> ApiResult<String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(ApiError(Error::validation("a remote URL is required")));
    }
    if url.contains(char::is_whitespace) || url.contains('\0') || url.starts_with('-') {
        return Err(ApiError(Error::validation("invalid remote URL")));
    }
    let lower = url.to_ascii_lowercase();
    let scheme = lower.split("://").next().filter(|_| lower.contains("://"));
    match scheme {
        Some("https") | Some("http") | Some("ssh") | Some("git") | Some("git+ssh") | Some("ssh+git") => Ok(url.to_string()),
        Some("file") => {
            if s.config.mirror.allow_file_urls {
                Ok(url.to_string())
            } else {
                Err(ApiError(Error::validation("file:// URLs are not allowed on this server")))
            }
        }
        Some(other) => Err(ApiError(Error::validation(format!("unsupported URL scheme {other}://")))),
        None => {
            if url.starts_with('/') || url.starts_with('.') {
                if s.config.mirror.allow_file_urls {
                    Ok(url.to_string())
                } else {
                    Err(ApiError(Error::validation("local paths are not allowed on this server")))
                }
            } else if url.contains(':') {
                Ok(url.to_string()) // scp-style git@host:owner/repo.git
            } else {
                Err(ApiError(Error::validation("remote URL must include a scheme, e.g. https://")))
            }
        }
    }
}

fn auth_of(username: &Option<String>, password: &Option<String>) -> RemoteAuth {
    RemoteAuth { username: username.clone(), password: password.clone() }
}

/// Diff two ref snapshots into the updates a push would have produced.
fn ref_updates(before: &[(String, String)], after: &[(String, String)]) -> Vec<RefUpdate> {
    let b: HashMap<&str, &str> = before.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let a: HashMap<&str, &str> = after.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let mut out = Vec::new();
    for (name, sha) in after {
        match b.get(name.as_str()) {
            Some(old) if *old == sha => {}
            Some(old) => out.push(RefUpdate { old: old.to_string(), new: sha.clone(), name: name.clone() }),
            None => out.push(RefUpdate { old: RefUpdate::ZERO.into(), new: sha.clone(), name: name.clone() }),
        }
    }
    for (name, sha) in before {
        if !a.contains_key(name.as_str()) {
            out.push(RefUpdate { old: sha.clone(), new: RefUpdate::ZERO.into(), name: name.clone() });
        }
    }
    out
}

/// Refuse pushes into a repository that mirrors another one (its content comes from the source).
pub async fn deny_push_if_mirror(s: &AppState, rw: &RepoWithOwner) -> Result<(), Response> {
    if is_pull_mirror(s, rw).await {
        return Err(
            (StatusCode::FORBIDDEN, format!("{} is a mirror; push to its source repository instead\n", rw.full_name())).into_response()
        );
    }
    Ok(())
}

pub async fn is_pull_mirror(s: &AppState, rw: &RepoWithOwner) -> bool {
    matches!(s.db.mirror_config(rw.repo.id()).await, Ok(Some(_)))
}

// ----- pull mirrors -----

fn mirror_json(m: &MirrorConfig) -> Value {
    json!({
        "pull_url": m.pull_url,
        "username": m.username,
        "has_password": m.password.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        "interval_minutes": m.interval_minutes,
        "last_synced_at": m.last_synced_at.map(iso),
        "last_error": m.last_error,
        "next_sync_at": m.next_sync_at.map(iso),
        "created_at": iso(m.created_at),
        "updated_at": iso(m.updated_at),
    })
}

async fn load_admin(s: &AppState, auth: &Auth, owner: &str, repo: &str) -> ApiResult<RepoWithOwner> {
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    Ok(rw)
}

pub async fn get_mirror(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let m = s.db.mirror_config(rw.repo.id()).await?;
    Ok(Json(json!({"mirror": m.as_ref().map(mirror_json), "allow_file_urls": s.config.mirror.allow_file_urls})))
}

#[derive(serde::Deserialize)]
pub struct PutMirror {
    pull_url: String,
    username: Option<String>,
    /// Omit to keep the stored password; send an empty string to clear it.
    password: Option<String>,
    interval_minutes: Option<i64>,
}

pub async fn put_mirror(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<PutMirror>,
) -> ApiResult<Json<Value>> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let url = validate_remote_url(&s, &b.pull_url).await?;
    let username = b.username.as_deref().map(str::trim).filter(|u| !u.is_empty());
    let password = b.password.as_deref().map(|p| if p.is_empty() { None } else { Some(p) });
    let m =
        s.db.set_mirror_config(
            rw.repo.id(),
            MirrorPatch { pull_url: &url, username, password, interval_minutes: b.interval_minutes.unwrap_or(60) },
        )
        .await?;
    spawn_sync(s.clone(), rw.repo.id());
    Ok(Json(json!({"mirror": mirror_json(&m)})))
}

pub async fn delete_mirror(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    s.db.delete_mirror_config(rw.repo.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST .../mirror/sync`: sync now. `{"status":"started"}` when this instance took the
/// mirror's lease and is syncing; `{"status":"queued"}` when a sync is already running (here or
/// on another instance): the request is remembered and the holder syncs again when it is done.
pub async fn sync_now(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    if s.db.mirror_config(rw.repo.id()).await?.is_none() {
        return Err(ApiError(Error::NotFound));
    }
    let repo = rw.repo.id();
    s.db.mirror_sync_now(repo).await?;
    match claim_sync(&s, repo).await {
        Some(held) => {
            let s2 = s.clone();
            tokio::spawn(async move { run_sync(&s2, repo, held).await });
            Ok(Json(json!({"status": "started"})))
        }
        None => Ok(Json(json!({"status": "queued"}))),
    }
}

fn spawn_sync(s: AppState, repo: Id) {
    tokio::spawn(async move {
        sync_mirror(&s, repo).await;
    });
}

/// Fetch the source of a pull mirror and process what changed as if it had been pushed.
/// Runs only when this instance wins the mirror's lease (the mirror must be due), and
/// records an outcome only while it still holds it.
pub async fn sync_mirror(s: &AppState, repo: Id) {
    if let Some(held) = claim_sync(s, repo).await {
        run_sync(s, repo, held).await;
    }
}

/// Take the lease on a due mirror (and this instance's busy mark). `None` when it is not due
/// or a sync is running here or elsewhere.
async fn claim_sync(s: &AppState, repo: Id) -> Option<i64> {
    if !try_busy(repo.0) {
        return None;
    }
    match s.db.claim_mirror(repo, LEASE_MS).await {
        Ok(Some(h)) => Some(h),
        Ok(None) => {
            done_busy(repo.0);
            None
        }
        Err(e) => {
            tracing::warn!(repo = repo.0, error = %e, "mirror: claiming the lease failed");
            done_busy(repo.0);
            None
        }
    }
}

/// Sync under the lease `held` (taken by `claim_sync`) and record the outcome; then, while
/// "sync now" requests arrived during the run (they leave the mirror due), sync again.
async fn run_sync(s: &AppState, repo: Id, held: i64) {
    let mut held = held;
    let syncs = &crate::metrics::registry().mirror_syncs;
    loop {
        let db = s.db.clone();
        let renew = move |h| {
            let db = db.clone();
            async move { db.renew_mirror_lease(repo, h, LEASE_MS).await }
        };
        // What this run fetches from, read before it starts: the generation it honours.
        let started = s.db.mirror_config(repo).await.ok().flatten().map(|c| MirrorSource::of(&c));
        match with_lease(held, renew, sync_mirror_inner(s, repo)).await {
            Leased::Lost => {
                tracing::warn!(repo = repo.0, "mirror sync aborted: the lease was lost to another instance");
                syncs.with_label_values(&["error"]).inc();
                break;
            }
            Leased::Done { held, result } => {
                if let Err(e) = &result {
                    tracing::warn!(repo = repo.0, error = %e, "mirror sync failed");
                }
                syncs.with_label_values(&[if result.is_ok() { "ok" } else { "error" }]).inc();
                // The source changed while this run fetched (`put_mirror` also sets
                // `sync_requested`; this covers a change the flag missed): make it due now so
                // the loop below syncs from the new source straight away.
                let now = s.db.mirror_config(repo).await.ok().flatten().map(|c| MirrorSource::of(&c));
                if now.is_some() && now != started {
                    tracing::info!(repo = repo.0, "mirror source changed during the sync; syncing again");
                    let _ = s.db.mirror_sync_now(repo).await;
                }
                match s.db.mirror_synced(repo, held, result.as_ref().err().map(|e| e.to_string()).as_deref()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(repo = repo.0, "mirror sync outcome not recorded: the lease was no longer held");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(repo = repo.0, error = %e, "mirror sync outcome could not be recorded");
                        break;
                    }
                }
            }
        }
        // Due again only when a "sync now" arrived meanwhile.
        match s.db.claim_mirror(repo, LEASE_MS).await {
            Ok(Some(h)) => held = h,
            _ => break,
        }
    }
    done_busy(repo.0);
}

/// Where a pull mirror fetches from: its generation. A sync honours the one it started with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MirrorSource {
    url: String,
    username: Option<String>,
    password: Option<String>,
}

impl MirrorSource {
    fn of(c: &MirrorConfig) -> MirrorSource {
        MirrorSource { url: c.pull_url.clone(), username: c.username.clone(), password: c.password.clone() }
    }
}

async fn sync_mirror_inner(s: &AppState, repo: Id) -> reef_core::Result<()> {
    let Some(cfg) = s.db.mirror_config(repo).await? else { return Ok(()) };
    crate::outbound::check_url(s, &cfg.pull_url).await.map_err(|r| Error::validation(format!("remote URL: {r}")))?;
    let rw = s.db.repo_with_owner_by_id(repo).await?;
    let path = s.store.path(repo);
    let git = s.store.git_binary();
    let before = ops::all_refs(git, &path).await?;
    ops::fetch_mirror(git, &path, &cfg.pull_url, &auth_of(&cfg.username, &cfg.password), timeout(s)).await?;
    let after = ops::all_refs(git, &path).await?;
    let updates = ref_updates(&before, &after);
    if (rw.repo.is_empty != 0 || before.is_empty())
        && let Ok(Some(head)) = ops::remote_head(git, &cfg.pull_url, &auth_of(&cfg.username, &cfg.password), timeout(s)).await
        && after.iter().any(|(n, _)| n == &format!("refs/heads/{head}"))
    {
        let _ = s.store.set_head(&path, &head).await;
        let _ = s.db.update_repo(repo, reef_db::repos::RepoPatch { default_branch: Some(head), ..Default::default() }).await;
    }
    if !updates.is_empty() || rw.repo.is_empty != 0 {
        let rw = s.db.repo_with_owner_by_id(repo).await?;
        crate::githttp::after_push(s, &rw, None, &updates).await?;
    }
    Ok(())
}

// ----- push mirrors -----

fn push_mirror_json(m: &PushMirror) -> Value {
    json!({
        "id": m.id,
        "url": m.url,
        "username": m.username,
        "has_password": m.password.as_deref().map(|p| !p.is_empty()).unwrap_or(false),
        "pending": m.pending != 0,
        "last_attempt_at": m.last_attempt_at.map(iso),
        "last_pushed_at": m.last_pushed_at.map(iso),
        "last_error": m.last_error,
        "created_at": iso(m.created_at),
    })
}

pub async fn list_push_mirrors(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let list = s.db.push_mirrors(rw.repo.id()).await?;
    Ok(Json(json!({"push_mirrors": list.iter().map(push_mirror_json).collect::<Vec<_>>()})))
}

#[derive(serde::Deserialize)]
pub struct NewPushMirror {
    url: String,
    username: Option<String>,
    password: Option<String>,
}

pub async fn create_push_mirror(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<NewPushMirror>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let url = validate_remote_url(&s, &b.url).await?;
    let username = b.username.as_deref().map(str::trim).filter(|u| !u.is_empty());
    let password = b.password.as_deref().filter(|p| !p.is_empty());
    let m = s.db.create_push_mirror(rw.repo.id(), &url, username, password).await?;
    spawn_push(s.clone(), m.clone());
    Ok((StatusCode::CREATED, Json(push_mirror_json(&m))))
}

pub async fn delete_push_mirror(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    s.db.delete_push_mirror(rw.repo.id(), Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn push_now(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let m = s.db.push_mirror(rw.repo.id(), Id(id)).await?;
    s.db.push_mirror_mark_pending(rw.repo.id(), Id(id)).await?;
    spawn_push(s.clone(), m);
    Ok(Json(json!({"status": "started"})))
}

fn spawn_push(s: AppState, m: PushMirror) {
    tokio::spawn(async move {
        push_one(&s, &m).await;
    });
}

/// Push one mirror under its lease and record the outcome; while the repository was pushed
/// to again during the push (a newer pending generation), push again. Losing the claim to a
/// holder leaves `pending` set, so the holder or the next tick pushes.
async fn push_one(s: &AppState, m: &PushMirror) {
    let key = -m.id; // distinct from the repository's own busy key
    if !try_busy(key) {
        return;
    }
    let id = Id(m.id);
    let mut claim = match s.db.claim_push_mirror(id, LEASE_MS).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            done_busy(key);
            return;
        }
        Err(e) => {
            tracing::warn!(repo = m.repo_id, mirror = m.id, error = %e, "push mirror: claiming the lease failed");
            done_busy(key);
            return;
        }
    };
    let path = s.store.path(Id(m.repo_id));
    loop {
        let work = async {
            match crate::outbound::check_url(s, &m.url).await {
                Ok(()) => ops::push_mirror(s.store.git_binary(), &path, &m.url, &auth_of(&m.username, &m.password), timeout(s)).await,
                Err(r) => Err(Error::validation(format!("remote URL: {r}"))),
            }
        };
        let db = s.db.clone();
        let renew = move |h| {
            let db = db.clone();
            async move { db.renew_push_mirror_lease(id, h, LEASE_MS).await }
        };
        match with_lease(claim.lease, renew, work).await {
            Leased::Lost => {
                tracing::warn!(repo = m.repo_id, mirror = m.id, "push mirror aborted: the lease was lost to another instance");
                break;
            }
            Leased::Done { held, result } => {
                if let Err(e) = &result {
                    tracing::warn!(repo = m.repo_id, mirror = m.id, error = %e, "push mirror failed");
                }
                match s.db.push_mirror_result(id, held, claim.generation, result.as_ref().err().map(|e| e.to_string()).as_deref()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(repo = m.repo_id, mirror = m.id, "push mirror outcome not recorded: the lease was no longer held");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(repo = m.repo_id, mirror = m.id, error = %e, "push mirror outcome could not be recorded");
                        break;
                    }
                }
                if result.is_err() {
                    break; // failures retry from the tick after a pause
                }
            }
        }
        // Still pending only when a newer push was marked while this one ran.
        match s.db.claim_push_mirror(id, LEASE_MS).await {
            Ok(Some(c)) => claim = c,
            _ => break,
        }
    }
    done_busy(key);
}

/// Called after every push into a repository: replicate to push mirrors and refresh the size.
pub async fn after_push(s: &AppState, rw: &RepoWithOwner) {
    if let Ok(list) = s.db.push_mirrors(rw.repo.id()).await
        && !list.is_empty()
    {
        let _ = s.db.push_mirrors_mark_pending(rw.repo.id()).await;
        for m in list {
            spawn_push(s.clone(), m);
        }
    }
    if let Ok((count, _)) = s.db.lfs_stats(rw.repo.id()).await
        && count > 0
    {
        crate::lfs::refresh_size(s, rw).await;
    }
}

// ----- imports -----

fn import_json(s: &AppState, rw: &RepoWithOwner, i: Option<&RepoImport>) -> Value {
    let api = s.config.api_url();
    let full = rw.full_name();
    let base = json!({
        "url": format!("{api}/repos/{full}/import"),
        "html_url": format!("{}/{full}/import", s.config.site_url()),
        "authors_url": format!("{api}/repos/{full}/import/authors"),
        "repository_url": format!("{api}/repos/{full}"),
    });
    let Some(i) = i else {
        let mut v = base;
        v["status"] = json!("none");
        v["status_text"] = json!("No import has been started");
        return v;
    };
    let (status, text) = match i.status.as_str() {
        "importing" => ("importing", "Importing commits, branches and tags"),
        "complete" => ("complete", "Import complete"),
        "cancelled" => ("cancelled", "Import cancelled"),
        _ => ("error", "Import failed"),
    };
    let mut v = base;
    v["vcs"] = json!(i.vcs);
    v["use_lfs"] = json!(false);
    v["vcs_url"] = json!(i.vcs_url);
    v["status"] = json!(status);
    v["status_text"] = json!(text);
    v["commit_count"] = json!(i.commit_count);
    v["message"] = json!(i.message);
    v["started_at"] = json!(iso(i.started_at));
    v["finished_at"] = json!(i.finished_at.map(iso));
    v["has_large_files"] = json!(false);
    v
}

pub async fn import_status(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = s.db.repo_import(rw.repo.id()).await?;
    if i.is_none() {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(import_json(&s, &rw, i.as_ref())))
}

#[derive(serde::Deserialize)]
pub struct StartImport {
    #[serde(default)]
    vcs: Option<String>,
    vcs_url: String,
    vcs_username: Option<String>,
    vcs_password: Option<String>,
}

pub async fn import_start(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<StartImport>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let actor = auth.require()?.clone();
    if let Some(v) = b.vcs.as_deref()
        && v != "git"
    {
        return Err(ApiError(Error::validation("only vcs=git imports are supported")));
    }
    let url = validate_remote_url(&s, &b.vcs_url).await?;
    if let Some(existing) = s.db.repo_import(rw.repo.id()).await?
        && existing.status == "importing"
        && with_running(|r| r.contains_key(&rw.repo.id))
    {
        return Err(ApiError(Error::Conflict("an import is already running".into())));
    }
    let git_empty = tokio::task::spawn_blocking({
        let p = s.store.path(rw.repo.id());
        move || reef_git::RepoReader::open(&p).map(|r| r.is_empty()).unwrap_or(true)
    })
    .await
    .map_err(Error::internal)?;
    if !git_empty {
        return Err(ApiError(Error::validation("the repository is not empty; imports need an empty repository")));
    }
    let username = b.vcs_username.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    let password = b.vcs_password.filter(|p| !p.is_empty());
    // Detached: the import row and the task that works on it are started together.
    let (s2, repo) = (s.clone(), rw.repo.id());
    let imp = crate::timeout::detached(async move {
        let imp = s2.db.start_repo_import(repo, "git", &url, username.as_deref(), password.as_deref(), Some(actor.id())).await?;
        spawn_import(s2, repo, Some(actor));
        Ok::<_, Error>(imp)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(import_json(&s, &rw, Some(&imp)))))
}

pub async fn import_cancel(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    if let Some(h) = with_running(|r| r.remove(&rw.repo.id)) {
        h.abort();
        let _ = s.db.finish_repo_import(rw.repo.id(), "cancelled", Some("cancelled by user"), None).await;
    } else {
        s.db.delete_repo_import(rw.repo.id()).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn spawn_import(s: AppState, repo: Id, actor: Option<Account>) {
    let s2 = s.clone();
    let task = tokio::spawn(async move {
        let result = run_import(&s2, repo, actor.as_ref()).await;
        with_running(|r| {
            r.remove(&repo.0);
        });
        done_busy(repo.0);
        match result {
            Ok(count) => {
                let _ = s2.db.finish_repo_import(repo, "complete", None, Some(count)).await;
            }
            Err(e) => {
                tracing::warn!(repo = repo.0, error = %e, "import failed");
                let _ = s2.db.finish_repo_import(repo, "error", Some(&e.to_string()), None).await;
            }
        }
    });
    with_running(|r| {
        r.insert(repo.0, task.abort_handle());
    });
}

async fn run_import(s: &AppState, repo: Id, actor: Option<&Account>) -> reef_core::Result<i64> {
    let Some(imp) = s.db.repo_import(repo).await? else { return Err(Error::NotFound) };
    let _ = try_busy(repo.0);
    let path = s.store.path(repo);
    let git = s.store.git_binary();
    let auth = auth_of(&imp.username, &imp.password);
    crate::outbound::check_url(s, &imp.vcs_url).await.map_err(|r| Error::validation(format!("remote URL: {r}")))?;
    ops::fetch_mirror(git, &path, &imp.vcs_url, &auth, timeout(s)).await?;
    let refs = ops::all_refs(git, &path).await?;
    if let Ok(Some(head)) = ops::remote_head(git, &imp.vcs_url, &auth, timeout(s)).await
        && refs.iter().any(|(n, _)| n == &format!("refs/heads/{head}"))
    {
        s.store.set_head(&path, &head).await?;
        s.db.update_repo(repo, reef_db::repos::RepoPatch { default_branch: Some(head), ..Default::default() }).await?;
    }
    let updates: Vec<RefUpdate> =
        refs.iter().map(|(n, sha)| RefUpdate { old: RefUpdate::ZERO.into(), new: sha.clone(), name: n.clone() }).collect();
    let rw = s.db.repo_with_owner_by_id(repo).await?;
    crate::githttp::after_push(s, &rw, actor, &updates).await?;
    ops::count_all_commits(git, &path).await
}

// ----- LFS overview for the settings page -----

pub async fn lfs_overview(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    let (count, bytes) = s.db.lfs_stats(rw.repo.id()).await?;
    let objects = s.db.lfs_objects_of(rw.repo.id(), 50).await?;
    let locks = s.db.lfs_locks(rw.repo.id(), None, None, 200).await?;
    let mut lock_rows = Vec::new();
    for l in &locks {
        let owner = s.db.account_by_id(Id(l.owner_id)).await.map(|a| a.login).unwrap_or_else(|_| "unknown".into());
        lock_rows.push(json!({"id": l.id, "path": l.path, "owner": owner, "locked_at": iso(l.created_at)}));
    }
    Ok(Json(json!({
        "enabled": s.config.lfs.enabled,
        "max_object_size": s.config.lfs.max_object_size,
        "object_count": count,
        "total_bytes": bytes,
        "endpoint": format!("{}/{}/{}.git/info/lfs", s.config.site_url(), rw.owner.login, rw.repo.name),
        "objects": objects.iter().map(|o| json!({"oid": o.oid, "size": o.size, "created_at": iso(o.created_at)})).collect::<Vec<_>>(),
        "locks": lock_rows,
    })))
}

pub async fn lfs_force_unlock(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = load_admin(&s, &auth, &owner, &repo).await?;
    s.db.delete_lfs_lock(rw.repo.id(), Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- tick -----

/// Called from the server's periodic tick: sync due pull mirrors, retry push mirrors, resume
/// imports abandoned by a restart, and every twenty ticks sweep LFS leftovers.
pub async fn tick(s: &AppState) {
    if let Ok(due) = s.db.due_mirrors(5).await {
        for m in due {
            spawn_sync(s.clone(), Id(m.repo_id));
        }
    }
    // Retry failed pushes after five minutes; fresh ones go immediately.
    if let Ok(list) = s.db.pending_push_mirrors(300_000, 10).await {
        for m in list {
            spawn_push(s.clone(), m);
        }
    }
    if let Ok(stale) = s.db.stale_repo_imports(10 * 60_000).await {
        for i in stale {
            if !with_running(|r| r.contains_key(&i.repo_id)) {
                let actor = match i.actor_id {
                    Some(a) => s.db.account_by_id(Id(a)).await.ok(),
                    None => None,
                };
                spawn_import(s.clone(), Id(i.repo_id), actor);
            }
        }
    }
    let n = TICKS.fetch_add(1, Ordering::Relaxed);
    if n.is_multiple_of(20) {
        crate::lfs::sweep_orphans(s).await;
        crate::lfs::expire_tokens(s).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Sync now" while another instance holds the lease is queued, not dropped.
    #[tokio::test]
    async fn sync_now_is_queued_while_another_instance_syncs() {
        use tower::ServiceExt;
        let s = crate::testing::state("syncnow", |_| {}).await;
        let (u, token) = crate::testing::user_with_token(&s, "mirrorer").await;
        let repo = s.db.create_repo(u.id(), reef_db::repos::NewRepo { name: "m".into(), ..Default::default() }).await.unwrap();
        s.db.set_mirror_config(
            repo.id(),
            MirrorPatch { pull_url: "https://example.invalid/src.git", username: None, password: None, interval_minutes: 60 },
        )
        .await
        .unwrap();
        let held = s.db.claim_mirror(repo.id(), 60_000).await.unwrap().expect("another instance syncs");
        let mut req = axum::http::Request::post("/api/reef/repos/mirrorer/m/mirror/sync")
            .header("authorization", format!("token {token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::from(([203, 0, 113, 91], 40000))));
        let res = crate::router(s.clone()).oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&http_body_util::BodyExt::collect(res.into_body()).await.unwrap().to_bytes()).unwrap();
        assert_eq!(body, json!({"status": "queued"}));
        assert_eq!(s.db.mirror_config(repo.id()).await.unwrap().unwrap().sync_requested, 1, "remembered for the holder");
        assert!(s.db.mirror_synced(repo.id(), held, None).await.unwrap());
        assert_eq!(s.db.mirror_config(repo.id()).await.unwrap().unwrap().next_sync_at, Some(0), "the holder's outcome keeps it due");
    }

    #[test]
    fn ref_diff_reports_creates_updates_and_deletes() {
        let before = vec![("refs/heads/main".to_string(), "a".to_string()), ("refs/heads/gone".to_string(), "b".to_string())];
        let after = vec![("refs/heads/main".to_string(), "c".to_string()), ("refs/tags/v1".to_string(), "d".to_string())];
        let u = ref_updates(&before, &after);
        assert_eq!(u.len(), 3);
        assert!(u.iter().any(|x| x.name == "refs/heads/main" && x.old == "a" && x.new == "c"));
        assert!(u.iter().any(|x| x.name == "refs/tags/v1" && x.is_create()));
        assert!(u.iter().any(|x| x.name == "refs/heads/gone" && x.is_delete()));
    }
}
