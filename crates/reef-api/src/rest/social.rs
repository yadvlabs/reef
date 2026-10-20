//! Stars, watching (subscriptions), and forks.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::rest::repos::load;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use reef_core::Error;
use reef_core::time::iso;
use reef_db::repos::{NewRepo, Permission};
use serde_json::{Value, json};

// ----- stars -----

pub async fn is_starred(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if s.db.is_starred(u.id(), rw.repo.id()).await? { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError(Error::NotFound)) }
}

pub async fn star(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    s.db.star(u.id(), rw.repo.id()).await?;
    let _ = s.db.record_event(rw.repo.id(), Some(u.id()), "watch", &json!({"action": "started"})).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unstar(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    s.db.unstar(u.id(), rw.repo.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn stargazers(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let all: Vec<Value> = s.db.stargazers(rw.repo.id()).await?.iter().map(|a| shapes::simple_user(&s.config, a)).collect();
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/stargazers", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

async fn starred_list(
    s: &AppState,
    auth: &Auth,
    user: reef_core::Id,
    url: String,
    page: Page,
) -> ApiResult<(axum::http::HeaderMap, Json<Value>)> {
    let mut all = Vec::new();
    for rw in s.db.starred_repos(user).await? {
        let perm = s.db.permission(auth.user.as_ref(), &rw).await?;
        if perm.can_read() {
            all.push(shapes::repo(&s.config, &rw, Some(perm)));
        }
    }
    let (items, h) = page.slice(&all, &url);
    Ok((h, Json(json!(items))))
}

pub async fn current_starred(State(s): State<AppState>, auth: Auth, page: Page) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?.clone();
    starred_list(&s, &auth, u.id(), format!("{}/user/starred", s.config.api_url()), page).await
}

pub async fn user_starred(State(s): State<AppState>, auth: Auth, Path(login): Path<String>, page: Page) -> ApiResult<impl IntoResponse> {
    let a = s.db.account_by_login(&login).await?;
    starred_list(&s, &auth, a.id(), format!("{}/users/{}/starred", s.config.api_url(), a.login), page).await
}

// ----- watching -----

fn subscription_json(s: &AppState, rw: &reef_db::repos::RepoWithOwner, w: &reef_db::social::Watch) -> Value {
    json!({
        "subscribed": w.ignored == 0,
        "ignored": w.ignored != 0,
        "reason": Value::Null,
        "created_at": iso(w.created_at),
        "url": format!("{}/repos/{}/subscription", s.config.api_url(), rw.full_name()),
        "repository_url": format!("{}/repos/{}", s.config.api_url(), rw.full_name()),
    })
}

pub async fn subscription(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    match s.db.watch_state(u.id(), rw.repo.id()).await? {
        Some(w) => Ok(Json(subscription_json(&s, &rw, &w))),
        None => Err(ApiError(Error::NotFound)),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct SetSubscription {
    subscribed: Option<bool>,
    ignored: Option<bool>,
}

pub async fn set_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    body: Option<Json<SetSubscription>>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let b = body.map(|b| b.0).unwrap_or_default();
    let ignored = b.ignored.unwrap_or(false) || b.subscribed == Some(false);
    let w = s.db.set_watch(u.id(), rw.repo.id(), ignored).await?;
    Ok(Json(subscription_json(&s, &rw, &w)))
}

pub async fn delete_subscription(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    s.db.unwatch(u.id(), rw.repo.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn subscribers(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let all: Vec<Value> = s.db.watchers(rw.repo.id()).await?.iter().map(|a| shapes::simple_user(&s.config, a)).collect();
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/subscribers", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

pub async fn current_subscriptions(State(s): State<AppState>, auth: Auth, page: Page) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?.clone();
    let mut all = Vec::new();
    for rw in s.db.watched_repos(u.id()).await? {
        let perm = s.db.permission(Some(&u), &rw).await?;
        if perm.can_read() {
            all.push(shapes::repo(&s.config, &rw, Some(perm)));
        }
    }
    let (items, h) = page.slice(&all, &format!("{}/user/subscriptions", s.config.api_url()));
    Ok((h, Json(json!(items))))
}

// ----- forks -----

#[derive(serde::Deserialize, Default)]
pub struct ForkBody {
    organization: Option<String>,
    name: Option<String>,
    #[allow(dead_code)]
    default_branch_only: Option<bool>,
}

pub async fn list_forks(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let mut all = Vec::new();
    for f in s.db.forks_of(rw.repo.id()).await? {
        let perm = s.db.permission(auth.user.as_ref(), &f).await?;
        if perm.can_read() {
            all.push(shapes::repo(&s.config, &f, Some(perm)));
        }
    }
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/forks", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

/// Fork into the caller's account or an organization. Returns the existing fork when the
/// owner already has one, like GitHub.
pub async fn create_fork(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    body: Option<Json<ForkBody>>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?.clone();
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let b = body.map(|b| b.0).unwrap_or_default();
    let target = match &b.organization {
        Some(o) => s.db.account_by_login(o).await?,
        None => u.clone(),
    };
    if !s.db.can_create_in(&u, &target).await? {
        return Err(crate::error::forbidden("You do not have permission to create repositories on this account"));
    }
    if target.id == rw.owner.id {
        return Err(ApiError(Error::Conflict("cannot fork a repository into its own owner".into())));
    }
    if let Some(existing) = s.db.fork_by_owner(rw.repo.id(), target.id()).await? {
        let perm = s.db.permission(Some(&u), &existing).await?;
        return Ok((StatusCode::ACCEPTED, Json(fork_json(&s, &existing, &rw, perm))));
    }
    let name = b.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| rw.repo.name.clone());
    let frw = fork_repository(&s, &u, &rw, &target, name).await?;
    Ok((StatusCode::ACCEPTED, Json(fork_json(&s, &frw, &rw, Permission::Admin))))
}

/// Copy `parent` into a new repository `name` owned by `target`: the row, the git storage
/// (a bare clone sharing nothing with the parent), the fork link and the `fork` event. The
/// caller has checked that `actor` may create in `target`. Detached (`timeout::detached`): a
/// request cut short never leaves a fork row without its storage.
pub async fn fork_repository(
    s: &AppState,
    actor: &reef_db::accounts::Account,
    parent: &reef_db::repos::RepoWithOwner,
    target: &reef_db::accounts::Account,
    name: String,
) -> reef_core::Result<reef_db::repos::RepoWithOwner> {
    let (s, actor, rw, target) = (s.clone(), actor.clone(), parent.clone(), target.clone());
    crate::timeout::detached(async move {
        let new =
            s.db.create_repo(
                target.id(),
                NewRepo {
                    name,
                    description: rw.repo.description.clone(),
                    homepage: rw.repo.homepage.clone(),
                    private: rw.repo.is_private(),
                    default_branch: Some(rw.repo.default_branch.clone()),
                    has_issues: Some(false),
                    has_projects: Some(true),
                    has_wiki: Some(false),
                    is_template: false,
                },
            )
            .await?;
        let src = s.store.path(rw.repo.id());
        let dst = s.store.path(new.id());
        if let Err(e) = reef_git::clone::fork_bare(s.store.git_binary(), &src, &dst).await {
            let _ = s.db.delete_repo(new.id()).await;
            return Err(e);
        }
        s.db.mark_fork(new.id(), rw.repo.id()).await?;
        let size = s.store.size_kb(new.id()).await.ok();
        s.db.touch_repo_push(new.id(), size, rw.repo.is_empty != 0).await?;
        let _ =
            s.db.record_event(rw.repo.id(), Some(actor.id()), "fork", &json!({"forkee": format!("{}/{}", target.login, new.name)})).await;
        s.db.repo_with_owner_by_id(new.id()).await
    })
    .await
}

pub fn fork_json(s: &AppState, fork: &reef_db::repos::RepoWithOwner, parent: &reef_db::repos::RepoWithOwner, perm: Permission) -> Value {
    let mut v = shapes::repo(&s.config, fork, Some(perm));
    v["parent"] = shapes::repo(&s.config, parent, None);
    v["source"] = shapes::repo(&s.config, parent, None);
    v
}
