//! Releases and source archives.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::repos::load;
use crate::shapes;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use reef_core::time::{iso, iso_opt};
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::releases::{NewRelease, Release, ReleasePatch};
use reef_db::repos::{Permission, RepoWithOwner};
use reef_git::ops;
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn release_json(s: &AppState, rw: &RepoWithOwner, r: &Release, author: Option<&Account>, body_html: Option<String>) -> Value {
    let full = rw.full_name();
    let api = format!("{}/repos/{full}", s.config.api_url());
    let site = format!("{}/{full}", s.config.site_url());
    let mut v = json!({
        "id": r.id,
        "node_id": shapes::node_id("RE", r.id),
        "tag_name": r.tag_name,
        "target_commitish": r.target_commitish,
        "name": r.name,
        "body": r.body,
        "draft": r.draft != 0,
        "prerelease": r.prerelease != 0,
        "created_at": iso(r.created_at),
        "published_at": iso_opt(r.published_at),
        "author": author.map(|a| shapes::simple_user(&s.config, a)),
        "assets": [],
        "url": format!("{api}/releases/{}", r.id),
        "html_url": format!("{site}/releases/tag/{}", shapes::urlpath(&r.tag_name)),
        "assets_url": format!("{api}/releases/{}/assets", r.id),
        "upload_url": format!("{api}/releases/{}/assets{{?name,label}}", r.id),
        "tarball_url": format!("{api}/tarball/{}", r.tag_name),
        "zipball_url": format!("{api}/zipball/{}", r.tag_name),
    });
    if let Some(h) = body_html {
        v["body_html"] = json!(h);
    }
    v
}

async fn author_of(s: &AppState, cache: &mut HashMap<i64, Option<Account>>, id: Option<i64>) -> Option<Account> {
    let id = id?;
    if let Some(a) = cache.get(&id) {
        return a.clone();
    }
    let a = s.db.account_by_id(Id(id)).await.ok();
    cache.insert(id, a.clone());
    a
}

#[derive(serde::Deserialize, Default)]
pub struct ListQuery {
    pub html: Option<String>,
}

pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: crate::rest::Page,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let rels = s.db.releases(rw.repo.id(), perm >= Permission::Write).await?;
    let mut cache = HashMap::new();
    let mut all = Vec::new();
    for r in &rels {
        let a = author_of(&s, &mut cache, r.author_id).await;
        let html = q.html.as_deref().map(|_| crate::render::markdown_in_repo(r.body.as_deref().unwrap_or(""), &s.config, &rw));
        all.push(release_json(&s, &rw, r, a.as_ref(), html));
    }
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/releases", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

#[derive(serde::Deserialize)]
pub struct NewReleaseBody {
    tag_name: String,
    target_commitish: Option<String>,
    name: Option<String>,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    generate_release_notes: bool,
}

/// Generate simple notes: the commits since the previous release (or the last 50).
async fn generated_notes(s: &AppState, rw: &RepoWithOwner, target: &str) -> String {
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let prev = s.db.releases(rw.repo.id(), false).await.unwrap_or_default().into_iter().next();
    let shas = match &prev {
        Some(p) => ops::rev_list(git, &path, &p.tag_name, target).await.unwrap_or_default(),
        None => vec![],
    };
    let p2 = path.clone();
    let t2 = target.to_string();
    let commits = tokio::task::spawn_blocking(move || {
        let r = reef_git::RepoReader::open(&p2).ok()?;
        if shas.is_empty() { r.log(&t2, None, 0, 50).ok() } else { Some(shas.iter().filter_map(|sha| r.commit(sha).ok()).collect()) }
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default();
    let mut out = String::from("## What's Changed\n");
    for c in commits {
        out.push_str(&format!("* {} ({})\n", c.summary(), &c.id[..7]));
    }
    if let Some(p) = prev {
        out.push_str(&format!("\n**Full Changelog**: {}/{}/compare/{}...{}\n", s.config.site_url(), rw.full_name(), p.tag_name, target));
    }
    out
}

/// Create a release, and its tag when publishing a new one. Detached
/// (`timeout::detached`): a tag is never created without its release row.
pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<NewReleaseBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    crate::timeout::detached(create_attached(s, auth, owner, repo, b)).await
}

async fn create_attached(s: AppState, auth: Auth, owner: String, repo: String, b: NewReleaseBody) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?.clone();
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have push access to create a release"));
    }
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let target = b.target_commitish.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| rw.repo.default_branch.clone());
    let tag = b.tag_name.trim().to_string();
    // The tag is created at the target when it does not exist yet (like GitHub, on publish).
    let tag_exists = ops::rev_parse(git, &path, &format!("refs/tags/{tag}")).await.is_ok();
    if !tag_exists && !b.draft {
        let sha = ops::rev_parse(git, &path, &target)
            .await
            .map_err(|_| ApiError(Error::field("Release", "target_commitish", "invalid", "target_commitish does not exist")))?;
        reef_core::names::validate_branch(&tag)
            .map_err(|_| ApiError(Error::field("Release", "tag_name", "invalid", "tag_name is not a valid tag")))?;
        ops::update_ref(git, &path, &format!("refs/tags/{tag}"), &sha, None, "release").await?;
    }
    let body = match (&b.body, b.generate_release_notes) {
        (Some(x), _) if !x.trim().is_empty() => Some(x.clone()),
        (_, true) => Some(generated_notes(&s, &rw, if tag_exists || !b.draft { &tag } else { &target }).await),
        _ => b.body.clone(),
    };
    let r =
        s.db.create_release(
            rw.repo.id(),
            NewRelease {
                tag_name: &tag,
                target_commitish: &target,
                name: b.name.as_deref(),
                body: body.as_deref(),
                draft: b.draft,
                prerelease: b.prerelease,
                author: Some(u.id()),
            },
        )
        .await?;
    s.db.record_event(
        rw.repo.id(),
        Some(u.id()),
        if b.draft { "release.drafted" } else { "release.published" },
        &json!({"tag": tag, "id": r.id}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(release_json(&s, &rw, &r, Some(&u), None))))
}

async fn visible(s: &AppState, rw: &RepoWithOwner, perm: Permission, r: Release) -> ApiResult<Release> {
    if r.draft != 0 && perm < Permission::Write {
        return Err(ApiError(Error::NotFound));
    }
    let _ = s;
    let _ = rw;
    Ok(r)
}

pub async fn show(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let r = s.db.release_by_id(Id(id)).await?;
    if r.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let r = visible(&s, &rw, perm, r).await?;
    let a = match r.author_id {
        Some(a) => s.db.account_by_id(Id(a)).await.ok(),
        None => None,
    };
    let html = q.html.map(|_| crate::render::markdown_in_repo(r.body.as_deref().unwrap_or(""), &s.config, &rw));
    Ok(Json(release_json(&s, &rw, &r, a.as_ref(), html)))
}

pub async fn latest(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = s.db.latest_release(rw.repo.id()).await?;
    let a = match r.author_id {
        Some(a) => s.db.account_by_id(Id(a)).await.ok(),
        None => None,
    };
    Ok(Json(release_json(&s, &rw, &r, a.as_ref(), None)))
}

pub async fn by_tag(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, tag)): Path<(String, String, String)>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let r = s.db.release_by_tag(rw.repo.id(), &tag).await?;
    let r = visible(&s, &rw, perm, r).await?;
    let a = match r.author_id {
        Some(a) => s.db.account_by_id(Id(a)).await.ok(),
        None => None,
    };
    let html = q.html.map(|_| crate::render::markdown_in_repo(r.body.as_deref().unwrap_or(""), &s.config, &rw));
    Ok(Json(release_json(&s, &rw, &r, a.as_ref(), html)))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateRelease {
    tag_name: Option<String>,
    target_commitish: Option<String>,
    name: Option<String>,
    body: Option<String>,
    draft: Option<bool>,
    prerelease: Option<bool>,
}

/// Edit a release; publishing a draft creates its tag. Detached like `create`.
pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Json(b): Json<UpdateRelease>,
) -> ApiResult<Json<Value>> {
    crate::timeout::detached(update_attached(s, auth, owner, repo, id, b)).await
}

async fn update_attached(s: AppState, auth: Auth, owner: String, repo: String, id: i64, b: UpdateRelease) -> ApiResult<Json<Value>> {
    let u = auth.require()?.clone();
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have push access to edit a release"));
    }
    let r = s.db.release_by_id(Id(id)).await?;
    if r.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    // Publishing a draft creates the tag if needed.
    if r.draft != 0 && b.draft == Some(false) {
        let path = s.store.path(rw.repo.id());
        let git = s.store.git_binary();
        let tag = b.tag_name.clone().unwrap_or_else(|| r.tag_name.clone());
        if ops::rev_parse(git, &path, &format!("refs/tags/{tag}")).await.is_err() {
            let target = b.target_commitish.clone().unwrap_or_else(|| r.target_commitish.clone());
            let sha = ops::rev_parse(git, &path, &target)
                .await
                .map_err(|_| ApiError(Error::field("Release", "target_commitish", "invalid", "target_commitish does not exist")))?;
            ops::update_ref(git, &path, &format!("refs/tags/{tag}"), &sha, None, "release").await?;
        }
    }
    let r =
        s.db.update_release(
            r.id(),
            ReleasePatch {
                tag_name: b.tag_name.as_deref(),
                target_commitish: b.target_commitish.as_deref(),
                name: b.name.as_deref().map(Some),
                body: b.body.as_deref().map(Some),
                draft: b.draft,
                prerelease: b.prerelease,
            },
        )
        .await?;
    Ok(Json(release_json(&s, &rw, &r, Some(&u), None)))
}

pub async fn delete(State(s): State<AppState>, auth: Auth, Path((owner, repo, id)): Path<(String, String, i64)>) -> ApiResult<StatusCode> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have push access to delete a release"));
    }
    let r = s.db.release_by_id(Id(id)).await?;
    if r.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.delete_release(r.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /repos/{o}/{r}/tarball/{ref}` and `/zipball/{ref}`: `git archive` streamed with a
/// GitHub-style `owner-repo-sha/` prefix.
pub async fn tarball(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, r#ref)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    archive_response(&s, &rw, "tarball", &r#ref).await
}

pub async fn zipball(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, r#ref)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    archive_response(&s, &rw, "zipball", &r#ref).await
}

pub async fn tarball_default(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let b = rw.repo.default_branch.clone();
    archive_response(&s, &rw, "tarball", &b).await
}

pub async fn zipball_default(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let b = rw.repo.default_branch.clone();
    archive_response(&s, &rw, "zipball", &b).await
}

pub async fn archive_response(s: &AppState, rw: &RepoWithOwner, kind: &str, r#ref: &str) -> ApiResult<Response> {
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let sha = ops::rev_parse(git, &path, r#ref).await.map_err(|_| ApiError(Error::NotFound))?;
    let (format, ext, ctype) = match kind {
        "tarball" => ("tar.gz", "tar.gz", "application/x-gzip"),
        "zipball" => ("zip", "zip", "application/zip"),
        _ => return Err(ApiError(Error::NotFound)),
    };
    let short = &sha[..7];
    let prefix = format!("{}-{}-{}/", rw.owner.login, rw.repo.name, short);
    let mut child = tokio::process::Command::new(git)
        .arg("-C")
        .arg(&path)
        .args(["archive", "--format", format, "--prefix", &prefix, &sha])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(Error::internal)?;
    let stdout = child.stdout.take().ok_or_else(|| ApiError(Error::msg("no stdout")))?;
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    let stream = tokio_util::io::ReaderStream::new(stdout);
    let filename = format!("{}-{}-{}.{}", rw.owner.login, rw.repo.name, short, ext);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename={filename}"))
        .body(Body::from_stream(stream))
        .map_err(Error::internal)?)
}
