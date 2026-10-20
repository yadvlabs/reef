use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::repos::{NewRepo, Permission, RepoPatch, RepoWithOwner};
use serde_json::{Value, json};

/// Load `owner/repo` and check the caller can at least read it. A private repository the
/// caller cannot see is a 404, as on GitHub.
pub async fn load(s: &AppState, auth: &Auth, owner: &str, repo: &str) -> ApiResult<(RepoWithOwner, Permission)> {
    let rw = s.db.repo_by_path(owner, repo).await?;
    let perm = s.db.permission(auth.user.as_ref(), &rw).await?;
    if !perm.can_read() {
        return Err(ApiError(Error::NotFound));
    }
    Ok((rw, perm))
}

pub async fn show(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let mut v = shapes::repo(&s.config, &rw, Some(perm));
    if let Some(pid) = rw.repo.parent_id
        && let Ok(parent) = s.db.repo_with_owner_by_id(Id(pid)).await
        && s.db.permission(auth.user.as_ref(), &parent).await?.can_read()
    {
        v["parent"] = shapes::repo(&s.config, &parent, None);
        v["source"] = shapes::repo(&s.config, &parent, None);
    }
    if let Some(l) = license_of(&s, &rw).await {
        v["license"] = l;
    }
    // Reef extensions the web UI uses for its buttons.
    if let Some(u) = &auth.user {
        v["viewer_starred"] = json!(s.db.is_starred(u.id(), rw.repo.id()).await?);
        v["viewer_subscription"] = match s.db.watch_state(u.id(), rw.repo.id()).await? {
            Some(w) => json!(if w.ignored != 0 { "ignored" } else { "subscribed" }),
            None => Value::Null,
        };
    }
    Ok(Json(v))
}

#[derive(serde::Deserialize, Default)]
pub struct CreateRepo {
    name: String,
    description: Option<String>,
    homepage: Option<String>,
    #[serde(default)]
    private: bool,
    visibility: Option<String>,
    has_issues: Option<bool>,
    has_projects: Option<bool>,
    has_wiki: Option<bool>,
    #[serde(default)]
    is_template: bool,
    #[serde(default)]
    auto_init: bool,
    default_branch: Option<String>,
    // Accepted and ignored for now.
    gitignore_template: Option<String>,
    license_template: Option<String>,
    team_id: Option<i64>,
}

pub async fn create_repo(s: &AppState, actor: &Account, owner: &Account, b: CreateRepo) -> ApiResult<(RepoWithOwner, Permission)> {
    if !s.db.can_create_in(actor, owner).await? {
        return Err(crate::error::forbidden("You do not have permission to create repositories on this account"));
    }
    let private = b.private || matches!(b.visibility.as_deref(), Some("private") | Some("internal") | Some("PRIVATE") | Some("INTERNAL"));
    let name = reef_core::names::sanitize_repo_name(b.name.trim());
    let _ = (b.gitignore_template, b.license_template, b.team_id);
    let new = NewRepo {
        name,
        description: b.description.filter(|d| !d.is_empty()),
        homepage: b.homepage.filter(|d| !d.is_empty()),
        private,
        default_branch: b.default_branch,
        has_issues: b.has_issues,
        has_projects: b.has_projects,
        has_wiki: b.has_wiki,
        is_template: b.is_template,
    };
    let rw = create_repository(s, actor, owner, new, b.auto_init).await?;
    let perm = s.db.permission(Some(actor), &rw).await?;
    Ok((rw, perm))
}

/// Create a repository: its row, its git storage, the default labels, the optional first
/// commit (`auto_init`) and the `repository.created` event. The caller has checked that
/// `actor` may create in `owner`. REST and GraphQL both create through here, detached
/// (`timeout::detached`): a request that is cut short never leaves a row without storage.
pub async fn create_repository(
    s: &AppState,
    actor: &Account,
    owner: &Account,
    new: NewRepo,
    auto_init: bool,
) -> reef_core::Result<RepoWithOwner> {
    let (s, actor, owner) = (s.clone(), actor.clone(), owner.clone());
    crate::timeout::detached(async move {
        let repo = s.db.create_repo(owner.id(), new).await?;
        if let Err(e) = s.store.init(repo.id(), &repo.default_branch).await {
            let _ = s.db.delete_repo(repo.id()).await;
            return Err(e);
        }
        s.db.ensure_default_labels(repo.id()).await?;
        if auto_init && let Err(e) = self::auto_init(&s, &repo, &actor).await {
            tracing::warn!(error = %e, "auto_init failed");
        }
        s.db.record_event(repo.id(), Some(actor.id()), "repository.created", &json!({})).await?;
        s.db.repo_with_owner_by_id(repo.id()).await
    })
    .await
}

/// Delete a repository's row and its git storage, detached (see `create_repository`). REST,
/// GraphQL and organization deletion all delete through here.
pub async fn delete_repository(s: &AppState, repo: Id) -> reef_core::Result<()> {
    let s = s.clone();
    crate::timeout::detached(async move {
        s.db.delete_repo(repo).await?;
        s.store.delete(repo).await
    })
    .await
}

/// Create the first commit with a README, like GitHub's `auto_init`.
async fn auto_init(s: &AppState, repo: &reef_db::repos::Repo, actor: &Account) -> reef_core::Result<()> {
    let path = s.store.path(repo.id());
    let tmp = s.store.tmp_dir().join(format!("reef-init-{}", repo.id));
    tokio::fs::create_dir_all(&tmp).await?;
    let readme = format!("# {}\n\n{}\n", repo.name, repo.description.clone().unwrap_or_default());
    tokio::fs::write(tmp.join("README.md"), readme).await?;
    let git = s.store.git_binary().to_path_buf();
    let run = |args: Vec<String>| {
        let git = git.clone();
        let tmp = tmp.clone();
        let name = actor.name.clone().unwrap_or_else(|| actor.login.clone());
        let email = actor.email.clone().unwrap_or_else(|| format!("{}@users.noreply.reef", actor.login));
        async move {
            let out = tokio::process::Command::new(&git)
                .args(&args)
                .current_dir(&tmp)
                .env("GIT_AUTHOR_NAME", &name)
                .env("GIT_AUTHOR_EMAIL", &email)
                .env("GIT_COMMITTER_NAME", &name)
                .env("GIT_COMMITTER_EMAIL", &email)
                .output()
                .await?;
            if !out.status.success() {
                return Err(Error::msg(format!("git {:?}: {}", args, String::from_utf8_lossy(&out.stderr))));
            }
            Ok::<(), Error>(())
        }
    };
    run(vec!["init".into(), "-q".into(), "-b".into(), repo.default_branch.clone()]).await?;
    run(vec!["add".into(), "README.md".into()]).await?;
    run(vec!["commit".into(), "-q".into(), "-m".into(), "Initial commit".into()]).await?;
    run(vec!["push".into(), "-q".into(), path.to_string_lossy().into_owned(), format!("HEAD:refs/heads/{}", repo.default_branch)]).await?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let size = s.store.size_kb(repo.id()).await.ok();
    s.db.touch_repo_push(repo.id(), size, false).await?;
    if let Ok(rw) = s.db.repo_with_owner_by_id(repo.id()).await {
        refresh_language(s, &rw).await;
    }
    Ok(())
}

pub async fn create_for_current(State(s): State<AppState>, auth: Auth, Json(b): Json<CreateRepo>) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    if !auth.has_scope("repo") && !auth.has_scope("public_repo") {
        return Err(crate::error::forbidden("Token requires the repo scope"));
    }
    let (rw, perm) = create_repo(&s, u, u, b).await?;
    Ok((StatusCode::CREATED, Json(shapes::repo(&s.config, &rw, Some(perm)))))
}

pub async fn create_in_org(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
    Json(b): Json<CreateRepo>,
) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let o = s.db.account_by_login(&org).await?;
    if !o.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    let (rw, perm) = create_repo(&s, u, &o, b).await?;
    Ok((StatusCode::CREATED, Json(shapes::repo(&s.config, &rw, Some(perm)))))
}

#[derive(serde::Deserialize)]
pub struct UpdateRepo {
    name: Option<String>,
    description: Option<String>,
    homepage: Option<String>,
    private: Option<bool>,
    visibility: Option<String>,
    has_issues: Option<bool>,
    has_projects: Option<bool>,
    has_wiki: Option<bool>,
    is_template: Option<bool>,
    default_branch: Option<String>,
    allow_squash_merge: Option<bool>,
    allow_merge_commit: Option<bool>,
    allow_rebase_merge: Option<bool>,
    allow_auto_merge: Option<bool>,
    delete_branch_on_merge: Option<bool>,
    archived: Option<bool>,
}

pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<UpdateRepo>,
) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    let private = b.private.or(match b.visibility.as_deref() {
        Some("private") | Some("internal") => Some(true),
        Some("public") => Some(false),
        _ => None,
    });
    if let Some(db) = &b.default_branch {
        // The branch must exist unless the repository is empty.
        let path = s.store.path(rw.repo.id());
        let db = db.clone();
        let exists = tokio::task::spawn_blocking(move || {
            let r = reef_git::RepoReader::open(&path)?;
            Ok::<bool, Error>(r.is_empty() || r.branches()?.iter().any(|(n, _)| *n == db))
        })
        .await
        .map_err(Error::internal)??;
        if !exists {
            return Err(ApiError(Error::validation("default branch does not exist")));
        }
    }
    let patch = RepoPatch {
        name: b.name,
        description: b.description.map(|d| if d.is_empty() { None } else { Some(d) }),
        homepage: b.homepage.map(|d| if d.is_empty() { None } else { Some(d) }),
        private,
        default_branch: b.default_branch.clone(),
        has_issues: b.has_issues,
        has_projects: b.has_projects,
        has_wiki: b.has_wiki,
        archived: b.archived,
        is_template: b.is_template,
        allow_squash_merge: b.allow_squash_merge,
        allow_merge_commit: b.allow_merge_commit,
        allow_rebase_merge: b.allow_rebase_merge,
        allow_auto_merge: b.allow_auto_merge,
        delete_branch_on_merge: b.delete_branch_on_merge,
        topics: None,
    };
    // Detached: the recorded default branch and the repository's HEAD move together.
    let (s2, id, default_branch) = (s.clone(), rw.repo.id(), b.default_branch);
    let updated = crate::timeout::detached(async move {
        let updated = s2.db.update_repo(id, patch).await?;
        if let Some(db) = default_branch {
            s2.store.set_head(&s2.store.path(updated.id()), &db).await?;
        }
        Ok::<_, Error>(updated)
    })
    .await?;
    let rw = s.db.repo_with_owner_by_id(updated.id()).await?;
    Ok(Json(shapes::repo(&s.config, &rw, Some(perm))))
}

pub async fn delete(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    if !auth.has_scope("delete_repo") {
        return Err(crate::error::forbidden("Must have the delete_repo scope to delete a repository."));
    }
    delete_repository(&s, rw.repo.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    r#type: Option<String>,
    affiliation: Option<String>,
    visibility: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
}

fn sort_repos(list: &mut [RepoWithOwner], q: &ListQuery) {
    let desc = match q.direction.as_deref() {
        Some("asc") => false,
        Some("desc") => true,
        _ => !matches!(q.sort.as_deref(), Some("full_name")),
    };
    match q.sort.as_deref() {
        Some("created") => list.sort_by_key(|r| r.repo.created_at),
        Some("pushed") => list.sort_by_key(|r| r.repo.pushed_at.unwrap_or(0)),
        Some("full_name") => list.sort_by_key(|a| a.full_name().to_lowercase()),
        _ => list.sort_by_key(|r| r.repo.updated_at),
    }
    if desc {
        list.reverse();
    }
}

fn filter_visibility(list: Vec<RepoWithOwner>, q: &ListQuery) -> Vec<RepoWithOwner> {
    match q.visibility.as_deref().or(q.r#type.as_deref()) {
        Some("public") => list.into_iter().filter(|r| !r.repo.is_private()).collect(),
        Some("private") => list.into_iter().filter(|r| r.repo.is_private()).collect(),
        _ => list,
    }
}

pub async fn list_for_current(
    State(s): State<AppState>,
    auth: Auth,
    page: Page,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let affiliation = match q.r#type.as_deref() {
        Some("owner") => "owner".to_string(),
        Some("member") => "collaborator,organization_member".to_string(),
        _ => q.affiliation.clone().unwrap_or_else(|| "owner,collaborator,organization_member".into()),
    };
    let mut list = filter_visibility(s.db.repos_for_user(u, &affiliation).await?, &q);
    sort_repos(&mut list, &q);
    let all: Vec<Value> = list.iter().map(|r| shapes::repo(&s.config, r, None)).collect();
    let (items, h) = page.slice(&all, &format!("{}/user/repos", s.config.api_url()));
    Ok((h, Json(json!(items))))
}

pub async fn list_for_user(
    State(s): State<AppState>,
    auth: Auth,
    Path(login): Path<String>,
    page: Page,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let a = s.db.account_by_login(&login).await?;
    let mut list = filter_visibility(s.db.repos_for_owner(&a, auth.user.as_ref(), 1, 10_000).await?, &q);
    sort_repos(&mut list, &q);
    let all: Vec<Value> = list.iter().map(|r| shapes::repo(&s.config, r, None)).collect();
    let (items, h) = page.slice(&all, &format!("{}/users/{}/repos", s.config.api_url(), a.login));
    Ok((h, Json(json!(items))))
}

pub async fn list_for_org(
    State(s): State<AppState>,
    auth: Auth,
    Path(login): Path<String>,
    page: Page,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let a = s.db.account_by_login(&login).await?;
    if !a.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    let mut list = filter_visibility(s.db.repos_for_owner(&a, auth.user.as_ref(), 1, 10_000).await?, &q);
    sort_repos(&mut list, &q);
    let all: Vec<Value> = list.iter().map(|r| shapes::repo(&s.config, r, None)).collect();
    let (items, h) = page.slice(&all, &format!("{}/orgs/{}/repos", s.config.api_url(), a.login));
    Ok((h, Json(json!(items))))
}

pub async fn list_public(State(s): State<AppState>, Query(q): Query<super::users::Since>) -> ApiResult<Json<Value>> {
    let list = s.db.public_repos(q.since, 100).await?;
    Ok(Json(json!(list.iter().map(|r| shapes::repo(&s.config, r, None)).collect::<Vec<_>>())))
}

// ----- collaborators -----

pub async fn collaborators(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access to view collaborators."));
    }
    let mut out = Vec::new();
    // The owner (or org admins) always appear first, as on GitHub.
    let mut seen = std::collections::HashSet::new();
    if rw.owner.is_user() {
        out.push(collab_json(&s, &rw.owner, Permission::Admin));
        seen.insert(rw.owner.id);
    } else {
        for (a, role) in s.db.org_members(rw.repo.owner_id.into(), None).await? {
            let p = if role == "admin" { Permission::Admin } else { Permission::Read };
            out.push(collab_json(&s, &a, p));
            seen.insert(a.id);
        }
    }
    for (a, p) in s.db.collaborators(rw.repo.id()).await? {
        if seen.insert(a.id) {
            out.push(collab_json(&s, &a, p));
        }
    }
    Ok(Json(json!(out)))
}

fn collab_json(s: &AppState, a: &Account, p: Permission) -> Value {
    let mut v = shapes::simple_user(&s.config, a);
    let o = v.as_object_mut().unwrap();
    o.insert(
        "permissions".into(),
        json!({"pull": p >= Permission::Read, "triage": p >= Permission::Triage, "push": p >= Permission::Write, "maintain": p >= Permission::Maintain, "admin": p >= Permission::Admin}),
    );
    o.insert("role_name".into(), json!(p.as_str()));
    v
}

pub async fn is_collaborator(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, login)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let a = s.db.account_by_login(&login).await?;
    let p = s.db.permission(Some(&a), &rw).await?;
    Ok(if p.can_read() && (rw.repo.is_private() || p > Permission::Read) { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND })
}

pub async fn collaborator_permission(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, login)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access."));
    }
    let a = s.db.account_by_login(&login).await?;
    let p = s.db.permission(Some(&a), &rw).await?;
    let legacy = match p {
        Permission::Admin => "admin",
        Permission::Maintain | Permission::Write => "write",
        Permission::Triage | Permission::Read => "read",
        Permission::None => "none",
    };
    Ok(Json(json!({"permission": legacy, "role_name": p.as_str(), "user": collab_json(&s, &a, p)})))
}

#[derive(serde::Deserialize, Default)]
pub struct AddCollaborator {
    permission: Option<String>,
}

pub async fn add_collaborator(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, login)): Path<(String, String, String)>,
    body: Option<Json<AddCollaborator>>,
) -> ApiResult<impl IntoResponse> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    let a = s.db.account_by_login(&login).await?;
    if !a.is_user() {
        return Err(ApiError(Error::validation("collaborator must be a user")));
    }
    let p = body.and_then(|b| b.0.permission).as_deref().and_then(Permission::parse).unwrap_or(Permission::Write);
    s.db.set_collaborator(rw.repo.id(), a.id(), p).await?;
    // GitHub returns 201 with an invitation; Reef adds directly and returns 204.
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_collaborator(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, login)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let a = s.db.account_by_login(&login).await?;
    if !perm.can_admin() && auth.user.as_ref().map(|u| u.id) != Some(a.id) {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    s.db.remove_collaborator(rw.repo.id(), a.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn topics(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    Ok(Json(json!({"names": rw.repo.topics()})))
}

#[derive(serde::Deserialize)]
pub struct TopicsBody {
    names: Vec<String>,
}

pub async fn set_topics(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<TopicsBody>,
) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    let mut names: Vec<String> = Vec::new();
    for n in b.names {
        let t = n.trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        if t.len() > 50 || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || t.starts_with('-') {
            return Err(ApiError(Error::field(
                "Repository",
                "topics",
                "invalid",
                format!("'{t}' is not a valid topic: use lowercase letters, numbers and hyphens (max 50)"),
            )));
        }
        if !names.contains(&t) {
            names.push(t);
        }
    }
    if names.len() > 20 {
        return Err(ApiError(Error::field("Repository", "topics", "invalid", "a repository can have at most 20 topics")));
    }
    s.db.update_repo(rw.repo.id(), RepoPatch { topics: Some(names.clone()), ..Default::default() }).await?;
    Ok(Json(json!({"names": names})))
}

/// Bytes per language at the default branch, by file extension (a linguist-lite).
pub async fn compute_languages(s: &AppState, rw: &RepoWithOwner) -> reef_core::Result<std::collections::BTreeMap<&'static str, u64>> {
    if rw.repo.is_empty != 0 {
        return Ok(Default::default());
    }
    let branch = rw.repo.default_branch.clone();
    crate::rest::contents::with_repo(s, rw, move |r| {
        let mut totals: std::collections::BTreeMap<&'static str, u64> = Default::default();
        let mut stack = vec![String::new()];
        let mut seen = 0usize;
        while let Some(dir) = stack.pop() {
            let Ok(entries) = r.tree(&branch, &dir) else { continue };
            for e in entries {
                seen += 1;
                if seen > 20_000 {
                    return Ok(totals);
                }
                match e.kind {
                    reef_git::EntryKind::Tree if !matches!(e.name.as_str(), "node_modules" | "vendor" | "target" | "dist" | ".git") => {
                        stack.push(e.path.clone());
                    }
                    reef_git::EntryKind::Blob | reef_git::EntryKind::Executable => {
                        if let Some(lang) = crate::languages::detect(&e.name) {
                            let size = r.blob(&e.id).map(|b| b.size).unwrap_or(0);
                            *totals.entry(lang).or_default() += size;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(totals)
    })
    .await
    .map_err(|e| e.0)
}

/// Languages sorted by bytes, largest first.
fn language_pairs(totals: std::collections::BTreeMap<&'static str, u64>) -> Vec<(&'static str, u64)> {
    let mut pairs: Vec<(&str, u64)> = totals.into_iter().filter(|(_, n)| *n > 0).collect();
    pairs.sort_by_key(|p| std::cmp::Reverse(p.1));
    pairs
}

/// Store the primary language when it changed (listings show it without a tree walk).
async fn store_language(s: &AppState, rw: &RepoWithOwner, pairs: &[(&'static str, u64)]) {
    let top = pairs.first().map(|(k, _)| *k);
    if rw.repo.language.as_deref() != top
        && let Err(e) = s.db.set_language(rw.repo.id(), top).await
    {
        tracing::warn!(error = %e, repo = %rw.full_name(), "storing primary language failed");
    }
}

/// Recompute and store the primary language (after pushes and the initial commit).
pub async fn refresh_language(s: &AppState, rw: &RepoWithOwner) {
    match compute_languages(s, rw).await {
        Ok(totals) => store_language(s, rw, &language_pairs(totals)).await,
        Err(e) => tracing::warn!(error = %e, repo = %rw.full_name(), "computing languages failed"),
    }
}

pub async fn languages(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let pairs = language_pairs(compute_languages(&s, &rw).await?);
    store_language(&s, &rw, &pairs).await;
    let mut out = serde_json::Map::new();
    for (k, v) in pairs {
        out.insert(k.to_string(), json!(v));
    }
    Ok(Json(Value::Object(out)))
}

/// Commit authors on the default branch, most commits first, mapped to accounts by email.
pub async fn contributors(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: crate::rest::Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.is_empty != 0 {
        return Ok((axum::http::HeaderMap::new(), Json(json!([]))));
    }
    let path = s.store.path(rw.repo.id());
    let out = tokio::process::Command::new(s.store.git_binary())
        .arg("-C")
        .arg(&path)
        .args(["shortlog", "-sne", "--no-merges", &rw.repo.default_branch])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(Error::internal)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut all = Vec::new();
    let mut seen: std::collections::HashSet<i64> = Default::default();
    for line in text.lines() {
        let t = line.trim();
        let Some((count, rest)) = t.split_once('\t') else { continue };
        let count: i64 = count.trim().parse().unwrap_or(0);
        let (name, email) = match rest.rsplit_once(" <") {
            Some((n, e)) => (n.trim(), e.trim_end_matches('>')),
            None => (rest, ""),
        };
        let account = crate::rest::contents::account_for_signature(&s, name, email).await;
        match account {
            Some(a) => {
                if !seen.insert(a.id) {
                    if let Some(prev) = all.iter_mut().find(|v: &&mut Value| v["id"] == json!(a.id)) {
                        prev["contributions"] = json!(prev["contributions"].as_i64().unwrap_or(0) + count);
                    }
                    continue;
                }
                let mut v = shapes::simple_user(&s.config, &a);
                v["contributions"] = json!(count);
                all.push(v);
            }
            None => all.push(json!({"login": null, "name": name, "email": email, "type": "Anonymous", "contributions": count, "avatar_url": format!("{}/avatars/{}", s.config.site_url(), shapes::urlpath(name))})),
        }
    }
    all.sort_by(|a, b| b["contributions"].as_i64().cmp(&a["contributions"].as_i64()));
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/contributors", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

pub async fn empty_list(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    load(&s, &auth, &owner, &repo).await?;
    Ok(Json(json!([])))
}

pub async fn not_found(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    load(&s, &auth, &owner, &repo).await?;
    Err(ApiError(Error::NotFound))
}

pub async fn actions_runs(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    load(&s, &auth, &owner, &repo).await?;
    Ok(Json(json!({"total_count": 0, "workflow_runs": []})))
}

pub async fn actions_workflows(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    load(&s, &auth, &owner, &repo).await?;
    Ok(Json(json!({"total_count": 0, "workflows": []})))
}

pub fn _unused(_: Id) {}

/// Detected license of the default branch, in GitHub's `license` shape.
pub async fn license_of(s: &AppState, rw: &RepoWithOwner) -> Option<Value> {
    if rw.repo.is_empty != 0 {
        return None;
    }
    let branch = rw.repo.default_branch.clone();
    let found = crate::rest::contents::with_repo(s, rw, move |r| r.license(&branch)).await.ok().flatten()?;
    let (entry, blob) = found;
    let mut v = crate::license::detect(&String::from_utf8_lossy(&blob.data));
    v["path"] = json!(entry.path);
    Some(v)
}

/// `GET /repos/{o}/{r}/license`: the license file with its detected license.
pub async fn license(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.is_empty != 0 {
        return Err(ApiError(Error::NotFound));
    }
    let branch = rw.repo.default_branch.clone();
    let (entry, blob) = crate::rest::contents::with_repo(&s, &rw, move |r| r.license(&branch)).await?.ok_or(ApiError(Error::NotFound))?;
    let text = String::from_utf8_lossy(&blob.data).into_owned();
    let mut v = shapes::content_file(&s.config, &rw, &entry, &blob, &rw.repo.default_branch);
    v["license"] = crate::license::detect(&text);
    v["text"] = json!(text);
    Ok(Json(v))
}
