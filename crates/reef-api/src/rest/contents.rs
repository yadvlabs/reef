//! Branches, tags, commits, and file contents, read from git on a blocking thread.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::rest::repos::load;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use reef_core::Error;
use reef_db::repos::RepoWithOwner;
use reef_git::RepoReader;
use serde_json::{Value, json};

/// Run a closure against the repository on a blocking thread.
pub async fn with_repo<T, F>(s: &AppState, rw: &RepoWithOwner, f: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce(&RepoReader) -> reef_core::Result<T> + Send + 'static,
{
    let path = s.store.path(rw.repo.id());
    let out = tokio::task::spawn_blocking(move || {
        let r = RepoReader::open(&path)?;
        f(&r)
    })
    .await
    .map_err(Error::internal)??;
    Ok(out)
}

#[derive(serde::Deserialize)]
pub struct RefQuery {
    #[serde(rename = "ref")]
    pub r#ref: Option<String>,
    pub sha: Option<String>,
    pub path: Option<String>,
}

fn rev_of(q: &RefQuery, rw: &RepoWithOwner) -> String {
    q.r#ref.clone().or_else(|| q.sha.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| rw.repo.default_branch.clone())
}

pub async fn branches(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    p: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let list = with_repo(&s, &rw, |r| r.branches()).await?;
    let rules = s.db.protections(rw.repo.id()).await?;
    let all: Vec<Value> = list
        .iter()
        .map(|(n, sha)| {
            let mut v = shapes::branch_short(&s.config, &rw, n, sha);
            v["protected"] = json!(rules.iter().any(|r| r.matches(n)));
            v
        })
        .collect();
    let (items, h) = p.slice(&all, &format!("{}/repos/{}/branches", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

pub async fn branch(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let n = name.clone();
    let (sha, c) = with_repo(&s, &rw, move |r| {
        let sha = r.branches()?.into_iter().find(|(b, _)| *b == n).map(|(_, s)| s).ok_or(Error::NotFound)?;
        let c = r.commit(&sha)?;
        Ok((sha, c))
    })
    .await?;
    let full = rw.full_name();
    let protected = s.db.protections_for_branch(rw.repo.id(), &name).await?;
    Ok(Json(json!({
        "name": name,
        "commit": shapes::commit(&s.config, &rw, &c, None, None),
        "_links": {"self": format!("{}/repos/{full}/branches/{name}", s.config.api_url()), "html": format!("{}/{full}/tree/{name}", s.config.site_url())},
        "protected": !protected.is_empty(),
        "protection": {"enabled": !protected.is_empty(), "required_status_checks": {"enforcement_level": if protected.first().map(|p| !p.checks().is_empty()).unwrap_or(false) { "non_admins" } else { "off" }, "contexts": protected.first().map(|p| p.checks()).unwrap_or_default(), "checks": []}},
        "protection_url": format!("{}/repos/{full}/branches/{name}/protection", s.config.api_url()),
        "sha": sha,
    })))
}

pub async fn tags(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    p: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let list = with_repo(&s, &rw, |r| r.tags()).await?;
    let all: Vec<Value> = list.iter().rev().map(|(n, sha)| shapes::tag(&s.config, &rw, n, sha)).collect();
    let (items, h) = p.slice(&all, &format!("{}/repos/{}/tags", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

pub async fn commits(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
    Query(q): Query<RefQuery>,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.is_empty != 0 {
        return Err(ApiError(Error::Conflict("Git Repository is empty.".into())));
    }
    let rev = rev_of(&q, &rw);
    let (page, per) = page.clamp();
    let path = q.path.clone().filter(|p| !p.is_empty());
    let skip = ((page - 1) * per) as usize;
    let list = with_repo(&s, &rw, move |r| r.log(&rev, path.as_deref(), skip, per as usize + 1)).await?;
    let has_next = list.len() as i64 > per;
    let list: Vec<_> = list.into_iter().take(per as usize).collect();
    let mut out = Vec::new();
    for c in &list {
        let author = account_for_signature(&s, &c.author.name, &c.author.email).await;
        let committer = account_for_signature(&s, &c.committer.name, &c.committer.email).await;
        let verification = crate::signing::verify_commit(&s, &rw, c).await;
        out.push(shapes::commit_with_verification(&s.config, &rw, c, author.as_ref(), committer.as_ref(), Some(verification)));
    }
    let mut h = HeaderMap::new();
    if has_next {
        let base = format!("{}/repos/{}/commits", s.config.api_url(), rw.full_name());
        h.insert("link", format!("<{base}?page={}&per_page={per}>; rel=\"next\"", page + 1).parse().unwrap());
    }
    Ok((h, Json(json!(out))))
}

/// The Reef user behind a commit signature: the account whose email matches, or, failing
/// that, the account whose login equals the author name (seeded and imported histories often
/// carry a login as the name with an unregistered email).
pub async fn account_for_signature(s: &AppState, name: &str, email: &str) -> Option<reef_db::accounts::Account> {
    if !email.is_empty()
        && let Ok(a) = s.db.user_by_any_email(email).await
    {
        return Some(a);
    }
    let login = name.trim().to_lowercase();
    if reef_core::names::validate_login(&login).is_ok()
        && let Ok(a) = s.db.account_by_login(&login).await
        && a.is_user()
    {
        return Some(a);
    }
    None
}

pub async fn commit(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, sha)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if let Some(kind) = crate::rest::pulls::media_public(&headers) {
        return crate::rest::pulls::commit_diff(&s, &rw, &sha, kind).await;
    }
    let sha2 = sha.clone();
    let c = with_repo(&s, &rw, move |r| {
        let sha = sha2;
        let id = r.resolve(&sha)?;
        r.commit(&id)
    })
    .await?;
    let author = account_for_signature(&s, &c.author.name, &c.author.email).await;
    let committer = account_for_signature(&s, &c.committer.name, &c.committer.email).await;
    let verification = crate::signing::verify_commit(&s, &rw, &c).await;
    let mut v = shapes::commit_with_verification(&s.config, &rw, &c, author.as_ref(), committer.as_ref(), Some(verification));
    let files =
        if c.parents.is_empty() { vec![] } else { crate::rest::pulls::files_json(&s, &rw, &c.parents[0], &c.id).await.unwrap_or_default() };
    let adds: i64 = files.iter().filter_map(|f| f.get("additions").and_then(|a| a.as_i64())).sum();
    let dels: i64 = files.iter().filter_map(|f| f.get("deletions").and_then(|a| a.as_i64())).sum();
    v.as_object_mut().unwrap().insert("stats".into(), json!({"total": adds + dels, "additions": adds, "deletions": dels}));
    v.as_object_mut().unwrap().insert("files".into(), json!(files));
    Ok(Json(v).into_response())
}

fn wants_raw(headers: &HeaderMap) -> Option<&'static str> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
    if accept.contains("application/vnd.github.raw") || accept.contains("application/vnd.github.v3.raw") {
        Some("raw")
    } else if accept.contains("application/vnd.github.html") || accept.contains("application/vnd.github.v3.html") {
        Some("html")
    } else {
        None
    }
}

pub async fn readme(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<RefQuery>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let rev = rev_of(&q, &rw);
    let rev2 = rev.clone();
    let found = with_repo(&s, &rw, move |r| r.readme(&rev2)).await?;
    let (entry, blob) = found.ok_or(ApiError(Error::NotFound))?;
    Ok(file_response(&s, &rw, &entry, &blob, &rev, wants_raw(&headers)))
}

fn file_response(
    s: &AppState,
    rw: &RepoWithOwner,
    entry: &reef_git::Entry,
    blob: &reef_git::Blob,
    rev: &str,
    raw: Option<&str>,
) -> Response {
    match raw {
        Some("raw") => ([(header::CONTENT_TYPE, "application/vnd.github.raw")], blob.data.clone()).into_response(),
        Some("html") => {
            let text = String::from_utf8_lossy(&blob.data);
            let html = if entry.name.to_lowercase().ends_with(".md") || entry.name.to_lowercase().ends_with(".markdown") {
                crate::render::markdown(&text)
            } else {
                format!("<pre>{}</pre>", ammonia::clean_text(&text))
            };
            ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
        }
        _ => Json(shapes::content_file(&s.config, rw, entry, blob, rev)).into_response(),
    }
}

pub async fn contents_root(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<RefQuery>,
) -> ApiResult<Response> {
    contents_impl(s, auth, headers, owner, repo, String::new(), q).await
}

pub async fn contents(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, path)): Path<(String, String, String)>,
    Query(q): Query<RefQuery>,
) -> ApiResult<Response> {
    contents_impl(s, auth, headers, owner, repo, path, q).await
}

async fn contents_impl(
    s: AppState,
    auth: Auth,
    headers: HeaderMap,
    owner: String,
    repo: String,
    path: String,
    q: RefQuery,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let rev = rev_of(&q, &rw);
    let path = path.trim_matches('/').to_string();
    if path.split('/').any(|seg| seg == "..") {
        return Err(ApiError(Error::NotFound));
    }
    let rev2 = rev.clone();
    let path2 = path.clone();
    enum Found {
        Dir(Vec<reef_git::Entry>, Vec<u64>),
        File(reef_git::Entry, reef_git::Blob),
    }
    let found = with_repo(&s, &rw, move |r| {
        if path2.is_empty() {
            let entries = r.tree(&rev2, "")?;
            let sizes = entries
                .iter()
                .map(|e| if e.kind == reef_git::EntryKind::Tree { 0 } else { r.blob(&e.id).map(|b| b.size).unwrap_or(0) })
                .collect();
            return Ok(Found::Dir(entries, sizes));
        }
        let entry = r.entry(&rev2, &path2)?;
        if entry.kind == reef_git::EntryKind::Tree {
            let entries = r.tree(&rev2, &path2)?;
            let sizes = entries
                .iter()
                .map(|e| if e.kind == reef_git::EntryKind::Tree { 0 } else { r.blob(&e.id).map(|b| b.size).unwrap_or(0) })
                .collect();
            Ok(Found::Dir(entries, sizes))
        } else {
            let blob = r.blob(&entry.id)?;
            Ok(Found::File(entry, blob))
        }
    })
    .await?;
    match found {
        Found::Dir(entries, sizes) => {
            let v: Vec<Value> = entries.iter().zip(sizes).map(|(e, sz)| shapes::content_entry(&s.config, &rw, e, &rev, sz)).collect();
            Ok(Json(json!(v)).into_response())
        }
        Found::File(entry, blob) => Ok(file_response(&s, &rw, &entry, &blob, &rev, wants_raw(&headers))),
    }
}

pub async fn git_refs(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (branches, tags) = with_repo(&s, &rw, |r| Ok((r.branches()?, r.tags()?))).await?;
    let api = s.config.api_url();
    let full = rw.full_name();
    let mk = |name: String, sha: &str| {
        json!({
            "ref": name,
            "node_id": shapes::node_id(shapes::NODE_REF, 0),
            "url": format!("{api}/repos/{full}/git/{name}"),
            "object": {"type": "commit", "sha": sha, "url": format!("{api}/repos/{full}/git/commits/{sha}")},
        })
    };
    let mut out: Vec<Value> = branches.iter().map(|(n, sha)| mk(format!("refs/heads/{n}"), sha)).collect();
    out.extend(tags.iter().map(|(n, sha)| mk(format!("refs/tags/{n}"), sha)));
    Ok(Json(json!(out)))
}

pub async fn git_ref(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let full_ref = format!("refs/{name}");
    let fr = full_ref.clone();
    let sha = with_repo(&s, &rw, move |r| r.resolve(&fr)).await?;
    let api = s.config.api_url();
    let full = rw.full_name();
    Ok(Json(json!({
        "ref": full_ref,
        "node_id": shapes::node_id(shapes::NODE_REF, 0),
        "url": format!("{api}/repos/{full}/git/{full_ref}"),
        "object": {"type": "commit", "sha": sha, "url": format!("{api}/repos/{full}/git/commits/{sha}")},
    })))
}

#[derive(serde::Deserialize)]
pub struct CreateRef {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
}

/// `POST /repos/{o}/{r}/git/refs`: create a branch or tag. Ref writes run detached
/// (`timeout::detached`) so the bookkeeping after them always happens.
pub async fn create_ref(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<CreateRef>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    crate::timeout::detached(create_ref_attached(s, auth, owner, repo, b)).await
}

async fn create_ref_attached(s: AppState, auth: Auth, owner: String, repo: String, b: CreateRef) -> ApiResult<(StatusCode, Json<Value>)> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access"));
    }
    if !b.name.starts_with("refs/heads/") && !b.name.starts_with("refs/tags/") {
        return Err(ApiError(Error::field("Reference", "ref", "invalid", "ref must start with refs/heads/ or refs/tags/")));
    }
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let sha = reef_git::ops::rev_parse(git, &path, &b.sha)
        .await
        .map_err(|_| ApiError(Error::field("Reference", "sha", "invalid", "object does not exist")))?;
    if reef_git::ops::rev_parse(git, &path, &b.name).await.is_ok() {
        return Err(ApiError(Error::Validation { message: "Reference already exists".into(), errors: vec![] }));
    }
    reef_git::ops::update_ref(git, &path, &b.name, &sha, None, "create via api").await?;
    let full = rw.full_name();
    let api = s.config.api_url();
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"ref": b.name, "node_id": shapes::node_id(shapes::NODE_REF, 0), "url": format!("{api}/repos/{full}/git/{}", b.name), "object": {"type": "commit", "sha": sha, "url": format!("{api}/repos/{full}/git/commits/{sha}")}}),
        ),
    ))
}

#[derive(serde::Deserialize)]
pub struct UpdateRef {
    sha: String,
    #[serde(default)]
    force: bool,
}

pub async fn update_ref(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
    Json(b): Json<UpdateRef>,
) -> ApiResult<Json<Value>> {
    crate::timeout::detached(update_ref_attached(s, auth, owner, repo, name, b)).await
}

async fn update_ref_attached(s: AppState, auth: Auth, owner: String, repo: String, name: String, b: UpdateRef) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access"));
    }
    let full_ref = format!("refs/{name}");
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let old = reef_git::ops::rev_parse(git, &path, &full_ref).await.map_err(|_| ApiError(Error::NotFound))?;
    let new = reef_git::ops::rev_parse(git, &path, &b.sha)
        .await
        .map_err(|_| ApiError(Error::field("Reference", "sha", "invalid", "object does not exist")))?;
    if !b.force && !reef_git::ops::is_ancestor(git, &path, &old, &new).await? {
        return Err(ApiError(Error::Validation { message: "Update is not a fast forward".into(), errors: vec![] }));
    }
    reef_git::ops::update_ref(git, &path, &full_ref, &new, Some(&old), "update via api").await?;
    if let Some(branch) = full_ref.strip_prefix("refs/heads/") {
        crate::pulls::on_branch_updated(&s, &rw, branch).await?;
    }
    let full = rw.full_name();
    let api = s.config.api_url();
    Ok(Json(
        json!({"ref": full_ref, "node_id": shapes::node_id(shapes::NODE_REF, 0), "url": format!("{api}/repos/{full}/git/{full_ref}"), "object": {"type": "commit", "sha": new, "url": format!("{api}/repos/{full}/git/commits/{new}")}}),
    ))
}

pub async fn delete_ref(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    crate::timeout::detached(delete_ref_attached(s, auth, owner, repo, name)).await
}

async fn delete_ref_attached(s: AppState, auth: Auth, owner: String, repo: String, name: String) -> ApiResult<StatusCode> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access"));
    }
    let full_ref = format!("refs/{name}");
    if let Some(branch) = full_ref.strip_prefix("refs/heads/") {
        if branch == rw.repo.default_branch {
            return Err(ApiError(Error::Validation { message: "Cannot delete the default branch".into(), errors: vec![] }));
        }
        let rules = s.db.protections_for_branch(rw.repo.id(), branch).await?;
        if rules
            .iter()
            .any(|r| r.allow_deletions == 0 && !(auth.user.as_ref().map(|u| u.is_admin()).unwrap_or(false) && r.enforce_admins == 0))
        {
            return Err(ApiError(Error::Validation { message: "Branch is protected".into(), errors: vec![] }));
        }
    }
    let path = s.store.path(rw.repo.id());
    reef_git::ops::rev_parse(s.store.git_binary(), &path, &full_ref)
        .await
        .map_err(|_| ApiError(Error::Validation { message: "Reference does not exist".into(), errors: vec![] }))?;
    reef_git::ops::delete_ref(s.store.git_binary(), &path, &full_ref).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn _status(_: StatusCode) {}

// ----- writing files through the API (and the web editor) -----

#[derive(serde::Deserialize)]
pub struct WriteFile {
    message: String,
    /// Base64 file content (absent when deleting).
    content: Option<String>,
    /// Current blob sha; required to update or delete an existing file.
    sha: Option<String>,
    branch: Option<String>,
    committer: Option<GitIdentity>,
    author: Option<GitIdentity>,
}

#[derive(serde::Deserialize, Clone)]
pub struct GitIdentity {
    name: String,
    email: String,
}

fn ident_of(u: &reef_db::accounts::Account, host: &str, override_: Option<&GitIdentity>) -> (String, String) {
    match override_ {
        Some(i) if !i.name.trim().is_empty() && i.email.contains('@') => (i.name.clone(), i.email.clone()),
        _ => (
            u.name.clone().unwrap_or_else(|| u.login.clone()),
            u.email.clone().unwrap_or_else(|| format!("{}@users.noreply.{host}", u.login)),
        ),
    }
}

/// Branch rules that block direct web commits: pull request required or locked.
async fn check_writable_branch(s: &AppState, rw: &RepoWithOwner, branch: &str, admin: bool) -> ApiResult<()> {
    for p in s.db.protections_for_branch(rw.repo.id(), branch).await? {
        if admin && p.enforce_admins == 0 {
            continue;
        }
        if p.lock_branch != 0 {
            return Err(ApiError(Error::Conflict(format!("branch {branch} is locked"))));
        }
        if p.require_pull_request != 0 {
            return Err(ApiError(Error::Conflict(format!("changes to {branch} must be made through a pull request"))));
        }
    }
    Ok(())
}

/// Commit one file change through the API (and the web editor), detached
/// (`timeout::detached`): the commit, the branch move and the push bookkeeping
/// (`githttp::after_push`) always complete together.
async fn write_file(
    s: AppState,
    auth: Auth,
    owner: String,
    repo: String,
    path: String,
    b: WriteFile,
    delete: bool,
) -> ApiResult<(StatusCode, Json<Value>)> {
    crate::timeout::detached(async move { write_file_attached(&s, &auth, &owner, &repo, &path, b, delete).await }).await
}

async fn write_file_attached(
    s: &AppState,
    auth: &Auth,
    owner: &str,
    repo: &str,
    path: &str,
    b: WriteFile,
    delete: bool,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?.clone();
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if perm < reef_db::repos::Permission::Write {
        return Err(crate::error::forbidden("Must have push access to write to this repository"));
    }
    let path = path.trim_matches('/').to_string();
    if path.is_empty() || path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") || path.contains('\\') {
        return Err(ApiError(Error::field("Contents", "path", "invalid", "path is invalid")));
    }
    let branch = b.branch.clone().filter(|x| !x.is_empty()).unwrap_or_else(|| rw.repo.default_branch.clone());
    reef_core::names::validate_branch(&branch)?;
    check_writable_branch(s, &rw, &branch, perm.can_admin()).await?;
    let repo_path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let head = reef_git::ops::rev_parse(git, &repo_path, &format!("refs/heads/{branch}")).await.ok();
    if head.is_none() && rw.repo.is_empty == 0 && branch != rw.repo.default_branch {
        return Err(ApiError(Error::field("Contents", "branch", "invalid", "Branch not found")));
    }
    // Existing file: the caller must prove it saw the current version.
    let existing = match &head {
        Some(h) => {
            let h2 = h.clone();
            let p2 = path.clone();
            with_repo(s, &rw, move |r| match r.entry(&h2, &p2) {
                Ok(e) => Ok(Some(e)),
                Err(Error::NotFound) => Ok(None),
                Err(e) => Err(e),
            })
            .await?
        }
        None => None,
    };
    match (&existing, &b.sha) {
        (Some(e), Some(sha)) if &e.id != sha => return Err(ApiError(Error::Conflict(format!("{path} does not match {sha}")))),
        (Some(_), None) => {
            return Err(ApiError(Error::field(
                "Contents",
                "sha",
                "missing_field",
                if delete { "sha is required to delete a file" } else { "sha is required when updating an existing file" },
            )));
        }
        (None, _) if delete => return Err(ApiError(Error::NotFound)),
        _ => {}
    }
    if existing.as_ref().map(|e| e.kind == reef_git::EntryKind::Tree).unwrap_or(false) {
        return Err(ApiError(Error::field("Contents", "path", "invalid", "path is a directory")));
    }
    let content = if delete {
        None
    } else {
        let raw =
            b.content.as_deref().ok_or_else(|| ApiError(Error::field("Contents", "content", "missing_field", "content is required")))?;
        let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        Some(
            base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .map_err(|_| ApiError(Error::field("Contents", "content", "invalid", "content is not valid base64")))?,
        )
    };
    let host = s.config.site_host();
    let (an, ae) = ident_of(&u, &host, b.author.as_ref());
    let (cn, ce) = ident_of(&u, &host, b.committer.as_ref());
    let message = if b.message.trim().is_empty() {
        format!(
            "{} {path}",
            if delete {
                "Delete"
            } else if existing.is_some() {
                "Update"
            } else {
                "Create"
            }
        )
    } else {
        b.message.clone()
    };
    let executable = existing.as_ref().map(|e| e.kind == reef_git::EntryKind::Executable).unwrap_or(false);
    let commit = reef_git::write::commit_changes(
        git,
        &repo_path,
        s.store.tmp_dir(),
        reef_git::write::CommitRequest {
            branch: branch.clone(),
            expected_head: head.clone(),
            message,
            author: reef_git::ops::Identity { name: &an, email: &ae },
            committer: reef_git::ops::Identity { name: &cn, email: &ce },
            changes: vec![reef_git::write::FileChange { path: path.clone(), content, executable }],
        },
    )
    .await?;
    let update = reef_git::smart::RefUpdate {
        old: head.clone().unwrap_or_else(|| reef_git::smart::RefUpdate::ZERO.to_string()),
        new: commit.clone(),
        name: format!("refs/heads/{branch}"),
    };
    crate::githttp::after_push(s, &rw, Some(&u), &[update]).await?;
    let c2 = commit.clone();
    let p2 = path.clone();
    let (commit_obj, entry, blob) = with_repo(s, &rw, move |r| {
        let c = r.commit(&c2)?;
        if delete {
            return Ok((c, None, None));
        }
        let e = r.entry(&c2, &p2)?;
        let b = r.blob(&e.id)?;
        Ok((c, Some(e), Some(b)))
    })
    .await?;
    let content_json = match (&entry, &blob) {
        (Some(e), Some(bl)) => {
            let mut v = shapes::content_entry(&s.config, &rw, e, &branch, bl.size);
            v["content"] = Value::Null;
            v["encoding"] = Value::Null;
            v
        }
        _ => Value::Null,
    };
    let status = if existing.is_some() || delete { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(json!({"content": content_json, "commit": shapes::commit(&s.config, &rw, &commit_obj, Some(&u), Some(&u))}))))
}

pub async fn put_contents(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, path)): Path<(String, String, String)>,
    Json(b): Json<WriteFile>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    write_file(s, auth, owner, repo, path, b, false).await
}

pub async fn delete_contents(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, path)): Path<(String, String, String)>,
    Json(b): Json<WriteFile>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    write_file(s, auth, owner, repo, path, b, true).await
}

#[derive(serde::Deserialize, Default)]
pub struct TreeQuery {
    recursive: Option<String>,
}

/// `GET /repos/{o}/{r}/git/trees/{sha}[?recursive=1]`.
pub async fn git_tree(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, sha)): Path<(String, String, String)>,
    Query(q): Query<TreeQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let recursive = q.recursive.map(|r| r != "0" && r != "false").unwrap_or(false);
    let rev = sha.clone();
    let (root, entries, truncated) = with_repo(&s, &rw, move |r| {
        let root = r.resolve(&rev)?;
        let mut out: Vec<(reef_git::Entry, u64)> = Vec::new();
        let mut stack = vec![String::new()];
        let mut truncated = false;
        'outer: while let Some(dir) = stack.pop() {
            for e in r.tree(&rev, &dir)? {
                if out.len() >= 100_000 {
                    truncated = true;
                    break 'outer;
                }
                let size = if e.kind == reef_git::EntryKind::Tree { 0 } else { r.blob(&e.id).map(|b| b.size).unwrap_or(0) };
                if recursive && e.kind == reef_git::EntryKind::Tree {
                    stack.push(e.path.clone());
                }
                out.push((e, size));
            }
        }
        out.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        Ok((root, out, truncated))
    })
    .await?;
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    let tree: Vec<Value> = entries
        .iter()
        .map(|(e, size)| {
            let is_tree = e.kind == reef_git::EntryKind::Tree;
            let mut v = json!({
                "path": e.path,
                "mode": format!("{:06o}", e.mode),
                "type": if is_tree { "tree" } else if e.kind == reef_git::EntryKind::Submodule { "commit" } else { "blob" },
                "sha": e.id,
                "url": format!("{api}/git/{}/{}", if is_tree { "trees" } else { "blobs" }, e.id),
            });
            if !is_tree {
                v["size"] = json!(size);
            }
            v
        })
        .collect();
    Ok(Json(json!({"sha": root, "url": format!("{api}/git/trees/{root}"), "tree": tree, "truncated": truncated})))
}

// ----- raw files and blame (Reef-specific, used by the web UI) -----

/// `GET /repos/{owner}/{repo}/raw/{rev}/{*path}`: the blob bytes with a content type guessed
/// from the file name. Text-like files are served as `text/plain` so browsers never execute them.
pub async fn raw(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, rev, path)): Path<(String, String, String, String)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let path = path.trim_matches('/').to_string();
    if path.is_empty() || path.split('/').any(|seg| seg == "..") {
        return Err(ApiError(Error::NotFound));
    }
    let rev2 = rev.clone();
    let path2 = path.clone();
    let blob = with_repo(&s, &rw, move |r| r.blob_at(&rev2, &path2)).await?;
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let essence = mime.essence_str().to_string();
    let safe = if essence.starts_with("image/") && essence != "image/svg+xml"
        || essence.starts_with("video/")
        || essence.starts_with("audio/")
        || essence == "application/pdf"
        || essence == "application/octet-stream"
    {
        essence
    } else if blob.is_binary() {
        "application/octet-stream".to_string()
    } else {
        "text/plain; charset=utf-8".to_string()
    };
    let name = path.rsplit('/').next().unwrap_or("file").to_string();
    Ok((
        [
            (header::CONTENT_TYPE, safe),
            (header::CONTENT_DISPOSITION, format!("inline; filename=\"{}\"", name.replace('"', ""))),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CACHE_CONTROL, "private, max-age=60".to_string()),
        ],
        blob.data,
    )
        .into_response())
}

/// One blame hunk: consecutive lines last touched by the same commit.
#[derive(Debug, Default, Clone)]
struct BlameHunk {
    sha: String,
    author: String,
    email: String,
    time: i64,
    summary: String,
    start: usize,
    end: usize,
}

/// Parse `git blame --porcelain` output into hunks, merging consecutive lines of one commit.
fn parse_porcelain(text: &str) -> Vec<BlameHunk> {
    let mut meta: std::collections::HashMap<String, (String, String, i64, String)> = Default::default();
    let mut hunks: Vec<BlameHunk> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(head) = lines.next() {
        let mut parts = head.split(' ');
        let (Some(sha), Some(_orig), Some(fin)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if sha.len() != 40 {
            continue;
        }
        let final_line: usize = fin.parse().unwrap_or(0);
        let mut author = String::new();
        let mut email = String::new();
        let mut time = 0i64;
        let mut summary = String::new();
        // Header key/value lines until the tab-prefixed content line.
        while let Some(l) = lines.peek() {
            if l.starts_with('\t') {
                lines.next();
                break;
            }
            let l = lines.next().unwrap();
            if let Some(v) = l.strip_prefix("author ") {
                author = v.to_string();
            } else if let Some(v) = l.strip_prefix("author-mail ") {
                email = v.trim_matches(|c| c == '<' || c == '>').to_string();
            } else if let Some(v) = l.strip_prefix("author-time ") {
                time = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("summary ") {
                summary = v.to_string();
            }
        }
        if !author.is_empty() {
            meta.insert(sha.to_string(), (author.clone(), email.clone(), time, summary.clone()));
        } else if let Some((a, e, t, s)) = meta.get(sha) {
            author = a.clone();
            email = e.clone();
            time = *t;
            summary = s.clone();
        }
        match hunks.last_mut() {
            Some(h) if h.sha == sha && h.end + 1 == final_line => h.end = final_line,
            _ => hunks.push(BlameHunk { sha: sha.to_string(), author, email, time, summary, start: final_line, end: final_line }),
        }
    }
    hunks
}

/// `GET /repos/{owner}/{repo}/blame/{*path}?ref=`: blame hunks for a file, newest-first is not
/// implied; hunks are in file order with `lines: [start, end]` (1-based, inclusive).
pub async fn blame(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, path)): Path<(String, String, String)>,
    Query(q): Query<RefQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let rev = rev_of(&q, &rw);
    let path = path.trim_matches('/').to_string();
    if path.is_empty() || path.split('/').any(|seg| seg == "..") || rev.starts_with('-') {
        return Err(ApiError(Error::NotFound));
    }
    let dir = s.store.path(rw.repo.id());
    let out = tokio::process::Command::new(s.store.git_binary())
        .arg("-C")
        .arg(&dir)
        .args(["blame", "--porcelain", &rev, "--", &path])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(Error::internal)?;
    if !out.status.success() {
        return Err(ApiError(Error::NotFound));
    }
    let hunks = parse_porcelain(&String::from_utf8_lossy(&out.stdout));
    let mut by_email: std::collections::HashMap<String, Option<reef_db::accounts::Account>> = Default::default();
    let mut result = Vec::with_capacity(hunks.len());
    for h in hunks {
        let key = h.email.to_lowercase();
        if !by_email.contains_key(&key) {
            let found = account_for_signature(&s, &h.author, &key).await;
            by_email.insert(key.clone(), found);
        }
        let account = by_email.get(&key).cloned().flatten();
        result.push(json!({
            "sha": h.sha,
            "author": h.author,
            "author_email": h.email,
            "author_login": account.as_ref().map(|a| a.login.clone()),
            "avatar_url": account.as_ref().map(|a| shapes::simple_user(&s.config, a)["avatar_url"].clone()).unwrap_or_else(|| json!(format!("{}/avatars/{}", s.config.site_url(), shapes::urlpath(&h.author)))),
            "date": reef_core::time::iso(h.time * 1000),
            "summary": h.summary,
            "lines": [h.start, h.end],
        }));
    }
    Ok(Json(json!(result)))
}

#[cfg(test)]
mod blame_tests {
    use super::parse_porcelain;

    #[test]
    fn merges_consecutive_lines_and_reuses_headers() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1 1 2\nauthor Ann\nauthor-mail <ann@example.com>\nauthor-time 1700000000\nsummary First\n\tline one\naaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 2 2\n\tline two\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 1 3 1\nauthor Bob\nauthor-mail <bob@example.com>\nauthor-time 1700000100\nsummary Second\n\tline three\n";
        let hunks = parse_porcelain(text);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].author, "Ann");
        assert_eq!(hunks[0].email, "ann@example.com");
        assert_eq!((hunks[0].start, hunks[0].end), (1, 2));
        assert_eq!(hunks[1].summary, "Second");
        assert_eq!((hunks[1].start, hunks[1].end), (3, 3));
    }
}
