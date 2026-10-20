//! Wikis: a git repository per code repository (`owner/repo.wiki.git`), pages are
//! markdown files at the root, edited through the web or cloned over git.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::repos::load;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::repos::{Permission, RepoWithOwner};
use reef_git::RepoReader;
use reef_git::ops::Identity;
use reef_git::write::{CommitRequest, FileChange, commit_changes};
use serde_json::{Value, json};

/// Wiki writes commit into git; the commit runs detached (`commit_wiki`), so they are timed
/// like any other route.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{owner}/{repo}/wiki", get(list).post(create))
        .route("/repos/{owner}/{repo}/wiki/{*page}", get(show).put(update).delete(delete))
        .route("/orgs/{org}/wiki", get(org_list).post(org_create))
        .route("/orgs/{org}/wiki/{*page}", get(org_show).put(org_update).delete(org_delete))
}

/// Where a wiki lives and how its pages render: a repository's wiki, or an organization's.
enum WikiHome {
    Repo(RepoWithOwner),
    Org(Account),
}

impl WikiHome {
    fn render(&self, s: &AppState, text: &str) -> String {
        match self {
            WikiHome::Repo(rw) => crate::render::markdown_in_repo(text, &s.config, rw),
            WikiHome::Org(_) => crate::render::markdown(text),
        }
    }
    fn clone_url(&self, s: &AppState) -> String {
        match self {
            WikiHome::Repo(rw) => format!("{}/{}.wiki.git", s.config.site_url(), rw.full_name()),
            WikiHome::Org(o) => format!("{}/{}.wiki.git", s.config.site_url(), o.login),
        }
    }
    fn event_repo(&self) -> Option<Id> {
        match self {
            WikiHome::Repo(rw) => Some(rw.repo.id()),
            WikiHome::Org(_) => None,
        }
    }
}

/// Organization wikis: members write, everyone reads.
async fn org_wiki(s: &AppState, auth: &Auth, org: &str, write: bool) -> ApiResult<(WikiHome, std::path::PathBuf)> {
    let o = s.db.account_by_login(org).await?;
    if !o.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    if write {
        let u = auth.require()?;
        let member = u.is_admin() || s.db.org_membership(o.id(), u.id()).await?.is_some();
        if !member {
            return Err(crate::error::forbidden("Must be an organization member to edit the wiki"));
        }
    }
    let path = s.store.ensure_wiki(o.id(), "main").await?;
    Ok((WikiHome::Org(o), path))
}

pub async fn org_list(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    let (home, path) = org_wiki(&s, &auth, &org, false).await?;
    let (head, list) = pages(path).await?;
    Ok(Json(json!({"head": head, "clone_url": home.clone_url(&s), "pages": list})))
}

pub async fn org_show(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, page)): Path<(String, String)>,
    Query(q): Query<ShowQuery>,
) -> ApiResult<Json<Value>> {
    let (home, path) = org_wiki(&s, &auth, &org, false).await?;
    show_inner(&s, &home, path, &page, q).await
}

pub async fn org_create(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
    Json(b): Json<WritePage>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?.clone();
    let (home, path) = org_wiki(&s, &auth, &org, true).await?;
    create_inner(&s, &home, path, &u, b).await
}

pub async fn org_update(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, page)): Path<(String, String)>,
    Json(b): Json<WritePage>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?.clone();
    let (home, path) = org_wiki(&s, &auth, &org, true).await?;
    update_inner(&s, &home, path, &u, &page, b).await
}

pub async fn org_delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, page)): Path<(String, String)>,
    Query(q): Query<HeadQuery>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    let (home, path) = org_wiki(&s, &auth, &org, true).await?;
    let file = format!("{}.md", slug(&page));
    let head = head_or_current(q.head, &path).await?;
    write_page(&s, &home, &path, &u, &file, WritePage { title: None, content: String::new(), message: None, head }, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The head an edit or deletion is checked against: the one the editor started from (the
/// web UI always sends it, so a concurrent edit is refused), or, when an API client sends
/// none, the wiki's current head (the last write wins, as on GitHub).
async fn head_or_current(head: Option<String>, path: &std::path::Path) -> ApiResult<Option<String>> {
    match head.filter(|h| !h.is_empty()) {
        Some(h) => Ok(Some(h)),
        None => Ok(pages(path.to_path_buf()).await?.0),
    }
}

/// Page title <-> file name: `Getting Started` is stored as `Getting-Started.md`.
pub fn slug(title: &str) -> String {
    let s: String = title.trim().chars().map(|c| if c.is_whitespace() || c == '/' { '-' } else { c }).collect();
    s.trim_matches('-').to_string()
}

pub fn title_of(file: &str) -> String {
    file.trim_end_matches(".md").replace('-', " ")
}

fn identity(a: &Account, host: &str) -> (String, String) {
    (a.name.clone().unwrap_or_else(|| a.login.clone()), a.email.clone().unwrap_or_else(|| format!("{}@users.noreply.{host}", a.login)))
}

async fn wiki_repo(s: &AppState, auth: &Auth, owner: &str, repo: &str, write: bool) -> ApiResult<(RepoWithOwner, std::path::PathBuf)> {
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if rw.repo.has_wiki == 0 {
        return Err(ApiError(Error::NotFound));
    }
    if write && perm < Permission::Write {
        return Err(crate::error::forbidden("Must have write access to edit the wiki"));
    }
    let path = s.store.ensure_wiki(rw.repo.id(), "main").await?;
    Ok((rw, path))
}

#[derive(serde::Serialize)]
struct PageInfo {
    title: String,
    file: String,
    size: u64,
}

async fn pages(path: std::path::PathBuf) -> ApiResult<(Option<String>, Vec<PageInfo>)> {
    tokio::task::spawn_blocking(move || {
        let r = RepoReader::open(&path)?;
        if r.is_empty() {
            return Ok((None, vec![]));
        }
        let head = r.resolve("HEAD")?;
        let entries = r.tree("HEAD", "")?;
        let mut out = Vec::new();
        for e in entries {
            if e.kind == reef_git::EntryKind::Blob && e.name.ends_with(".md") {
                let size = r.blob(&e.id).map(|b| b.size).unwrap_or(0);
                out.push(PageInfo { title: title_of(&e.name), file: e.name, size });
            }
        }
        Ok::<_, Error>((Some(head), out))
    })
    .await
    .map_err(Error::internal)?
    .map_err(ApiError)
}

pub async fn list(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, path) = wiki_repo(&s, &auth, &owner, &repo, false).await?;
    let (head, list) = pages(path).await?;
    Ok(Json(json!({
        "head": head,
        "clone_url": WikiHome::Repo(rw).clone_url(&s),
        "pages": list,
    })))
}

#[derive(serde::Deserialize, Default)]
pub struct ShowQuery {
    history: Option<String>,
}

pub async fn show(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, page)): Path<(String, String, String)>,
    Query(q): Query<ShowQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, path) = wiki_repo(&s, &auth, &owner, &repo, false).await?;
    show_inner(&s, &WikiHome::Repo(rw), path, &page, q).await
}

async fn show_inner(s: &AppState, home: &WikiHome, path: std::path::PathBuf, page: &str, q: ShowQuery) -> ApiResult<Json<Value>> {
    let file = format!("{}.md", slug(page));
    let f2 = file.clone();
    let history = matches!(q.history.as_deref(), Some("1") | Some("true"));
    let (head, blob, log) = tokio::task::spawn_blocking(move || {
        let r = RepoReader::open(&path)?;
        if r.is_empty() {
            return Err(Error::NotFound);
        }
        let head = r.resolve("HEAD")?;
        let blob = r.blob_at("HEAD", &f2)?;
        let log = if history { r.log("HEAD", Some(&f2), 0, 50)? } else { vec![] };
        Ok::<_, Error>((head, blob, log))
    })
    .await
    .map_err(Error::internal)??;
    let text = String::from_utf8_lossy(&blob.data).into_owned();
    Ok(Json(json!({
        "title": title_of(&file),
        "file": file,
        "head": head,
        "content": text,
        "html": home.render(s, &text),
        "history": log.iter().map(|c| json!({"sha": c.id, "message": c.summary(), "author": c.author.name, "date": reef_core::time::iso(c.committer.time_ms)})).collect::<Vec<_>>(),
    })))
}

#[derive(serde::Deserialize)]
pub struct WritePage {
    title: Option<String>,
    content: String,
    message: Option<String>,
    head: Option<String>,
}

async fn write_page(
    s: &AppState,
    home: &WikiHome,
    path: &std::path::Path,
    actor: &Account,
    file: &str,
    b: WritePage,
    delete: bool,
) -> ApiResult<String> {
    let msg = b
        .message
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("{} {}", if delete { "Delete" } else { "Update" }, title_of(file)));
    let changes =
        vec![FileChange { path: file.to_string(), content: if delete { None } else { Some(b.content.into_bytes()) }, executable: false }];
    commit_wiki(s, home.event_repo(), path, actor, b.head.filter(|h| !h.is_empty()), msg, changes, file.to_string()).await
}

/// Commit `changes` to the wiki's `main` and record the `wiki` event for `page`. Detached
/// (`timeout::detached`): a request cut short never leaves a commit without its event.
#[allow(clippy::too_many_arguments)]
async fn commit_wiki(
    s: &AppState,
    event_repo: Option<Id>,
    path: &std::path::Path,
    actor: &Account,
    expected_head: Option<String>,
    message: String,
    changes: Vec<FileChange>,
    page: String,
) -> ApiResult<String> {
    let (s, path, actor) = (s.clone(), path.to_path_buf(), actor.clone());
    crate::timeout::detached(async move {
        let (name, email) = identity(&actor, &s.config.site_host());
        let commit = commit_changes(
            s.store.git_binary(),
            &path,
            s.store.tmp_dir(),
            CommitRequest {
                branch: "main".into(),
                expected_head,
                message,
                author: Identity { name: &name, email: &email },
                committer: Identity { name: &name, email: &email },
                changes,
            },
        )
        .await?;
        if let Some(rid) = event_repo {
            let _ = s.db.record_event(rid, Some(actor.id()), "wiki", &json!({"page": page, "commit": commit})).await;
        }
        Ok(commit)
    })
    .await
}

pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<WritePage>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?.clone();
    let (rw, path) = wiki_repo(&s, &auth, &owner, &repo, true).await?;
    create_inner(&s, &WikiHome::Repo(rw), path, &u, b).await
}

async fn create_inner(
    s: &AppState,
    home: &WikiHome,
    path: std::path::PathBuf,
    u: &Account,
    b: WritePage,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let title = b.title.clone().unwrap_or_default();
    if slug(&title).is_empty() {
        return Err(ApiError(Error::validation("title is required")));
    }
    let file = format!("{}.md", slug(&title));
    let (head, existing) = pages(path.clone()).await?;
    if existing.iter().any(|p| p.file.eq_ignore_ascii_case(&file)) {
        return Err(ApiError(Error::Conflict("a page with that title already exists".into())));
    }
    let b = WritePage { head: Some(head.unwrap_or_default()), message: b.message.or(Some(format!("Create {}", title_of(&file)))), ..b };
    let commit = write_page(s, home, &path, u, &file, b, false).await?;
    Ok((StatusCode::CREATED, Json(json!({"file": file, "title": title_of(&file), "commit": commit}))))
}

pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, page)): Path<(String, String, String)>,
    Json(b): Json<WritePage>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?.clone();
    let (rw, path) = wiki_repo(&s, &auth, &owner, &repo, true).await?;
    update_inner(&s, &WikiHome::Repo(rw), path, &u, &page, b).await
}

async fn update_inner(
    s: &AppState,
    home: &WikiHome,
    path: std::path::PathBuf,
    u: &Account,
    page: &str,
    b: WritePage,
) -> ApiResult<Json<Value>> {
    let file = format!("{}.md", slug(page));
    // Renames: a different title writes the new file and removes the old one.
    let new_file = b.title.as_deref().map(slug).filter(|t| !t.is_empty()).map(|t| format!("{t}.md")).unwrap_or_else(|| file.clone());
    let mut changes = vec![FileChange { path: new_file.clone(), content: Some(b.content.clone().into_bytes()), executable: false }];
    if new_file != file {
        changes.push(FileChange { path: file.clone(), content: None, executable: false });
    }
    let msg = b.message.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| format!("Update {}", title_of(&new_file)));
    let head = head_or_current(b.head.clone(), &path).await?;
    let commit = commit_wiki(s, home.event_repo(), &path, u, head, msg, changes, new_file.clone()).await?;
    Ok(Json(json!({"file": new_file, "title": title_of(&new_file), "commit": commit})))
}

pub async fn delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, page)): Path<(String, String, String)>,
    Query(q): Query<HeadQuery>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    let (rw, path) = wiki_repo(&s, &auth, &owner, &repo, true).await?;
    let file = format!("{}.md", slug(&page));
    let head = head_or_current(q.head, &path).await?;
    write_page(&s, &WikiHome::Repo(rw), &path, &u, &file, WritePage { title: None, content: String::new(), message: None, head }, true)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize, Default)]
pub struct HeadQuery {
    head: Option<String>,
}

pub fn _id(_: Id) {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(std::path::PathBuf);
    impl Dir {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    // An API client that edits a page without saying which head it started from edits the
    // current version; one that does is still refused when the wiki moved (commit_changes).
    #[tokio::test]
    async fn edits_without_a_head_target_the_current_one() {
        let root = std::env::temp_dir().join(format!("reef-wiki-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dir = Dir(root);
        let bare = dir.path().join("w.wiki.git");
        git(dir.path(), &["init", "-q", "--bare", bare.to_str().unwrap()]);
        assert_eq!(head_or_current(None, &bare).await.unwrap_or_else(|e| panic!("{}", e.0)), None, "an empty wiki has no head");
        let work = dir.path().join("work");
        git(dir.path(), &["clone", "-q", bare.to_str().unwrap(), work.to_str().unwrap()]);
        std::fs::write(work.join("Home.md"), "# Home\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-qm", "Home"]);
        git(&work, &["push", "-q", "origin", "HEAD:main"]);
        let tip = git(&work, &["rev-parse", "HEAD"]);
        assert_eq!(head_or_current(None, &bare).await.unwrap_or_else(|e| panic!("{}", e.0)), Some(tip.clone()));
        assert_eq!(head_or_current(Some(String::new()), &bare).await.unwrap_or_else(|e| panic!("{}", e.0)), Some(tip));
        assert_eq!(head_or_current(Some("abc".into()), &bare).await.unwrap_or_else(|e| panic!("{}", e.0)), Some("abc".into()));
    }
}
