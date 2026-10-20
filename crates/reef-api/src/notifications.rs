//! Notifications and the home feed (Reef API), plus GitHub's `/notifications` shape.

use crate::AppState;
use crate::auth::Auth;
use crate::error::ApiResult;
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use reef_core::Id;
use reef_core::time::iso;
use reef_db::notifications::Notification;
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/notifications", get(list).post(mark_all_read))
        .route("/notifications/{id}", axum::routing::patch(mark_read).delete(delete))
        .route("/feed", get(feed))
        .route("/explore", get(explore))
        .route("/explore/owners", get(explore_owners))
}

/// GitHub-compatible `/notifications` (REST v3): list and mark-as-read.
pub fn github_router() -> Router<AppState> {
    Router::new()
        .route("/notifications", get(github_list).put(github_mark_all))
        .route("/notifications/threads/{id}", get(github_thread).patch(github_thread_read))
}

async fn notification_json(s: &AppState, n: &Notification, cache: &mut HashMap<i64, Option<reef_db::accounts::Account>>) -> Value {
    let actor = match n.actor_id {
        Some(a) => {
            if let Some(c) = cache.get(&a) {
                c.clone()
            } else {
                let acc = s.db.account_by_id(Id(a)).await.ok();
                cache.insert(a, acc.clone());
                acc
            }
        }
        None => None,
    };
    let repo = match n.repo_id {
        Some(r) => s.db.repo_with_owner_by_id(Id(r)).await.ok().map(|rw| rw.full_name()),
        None => None,
    };
    json!({
        "id": n.id,
        "subject_type": n.subject_type,
        "subject_id": n.subject_id,
        "reason": n.reason,
        "title": n.title,
        "url": n.url,
        "unread": n.unread != 0,
        "repository": repo,
        "actor": actor.as_ref().map(|a| shapes::simple_user(&s.config, a)),
        "created_at": iso(n.created_at),
        "updated_at": iso(n.updated_at),
    })
}

#[derive(serde::Deserialize, Default)]
pub struct ListQuery {
    all: Option<bool>,
    per_page: Option<i64>,
}

pub async fn list(State(s): State<AppState>, auth: Auth, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let rows = s.db.notifications(u.id(), !q.all.unwrap_or(false), q.per_page.unwrap_or(50).clamp(1, 200)).await?;
    let mut cache = HashMap::new();
    let mut out = Vec::new();
    for n in &rows {
        out.push(notification_json(&s, n, &mut cache).await);
    }
    Ok(Json(json!({"unread_count": s.db.unread_count(u.id()).await?, "notifications": out})))
}

pub async fn mark_all_read(State(s): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.mark_read(u.id(), None).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn mark_read(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.mark_read(u.id(), Some(Id(id))).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.delete_notification(u.id(), Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- feed -----

/// Activity in the repositories the user owns, belongs to, collaborates on, starred or watches.
pub async fn feed(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let repos = s.db.repos_of_interest(u.id()).await?;
    let events = s.db.feed(&repos, 60).await?;
    let mut repo_cache: HashMap<i64, Option<reef_db::repos::RepoWithOwner>> = HashMap::new();
    let mut actor_cache: HashMap<i64, Option<reef_db::accounts::Account>> = HashMap::new();
    let mut out = Vec::new();
    for e in events {
        if let std::collections::hash_map::Entry::Vacant(slot) = repo_cache.entry(e.repo_id) {
            slot.insert(s.db.repo_with_owner_by_id(Id(e.repo_id)).await.ok());
        }
        let Some(rw) = repo_cache.get(&e.repo_id).cloned().flatten() else { continue };
        if !s.db.permission(Some(u), &rw).await?.can_read() {
            continue;
        }
        let actor = match e.actor_id {
            Some(a) => {
                if let std::collections::hash_map::Entry::Vacant(slot) = actor_cache.entry(a) {
                    slot.insert(s.db.account_by_id(Id(a)).await.ok());
                }
                actor_cache.get(&a).cloned().flatten()
            }
            None => None,
        };
        let payload: Value = serde_json::from_str(&e.payload).unwrap_or(Value::Null);
        out.push(json!({
            "id": e.id,
            "kind": e.kind,
            "payload": payload,
            "repository": {"full_name": rw.full_name(), "owner": rw.owner.login, "name": rw.repo.name, "private": rw.repo.is_private(), "description": rw.repo.description},
            "actor": actor.as_ref().map(|a| shapes::simple_user(&s.config, a)),
            "created_at": iso(e.created_at),
        }));
    }
    Ok(Json(json!({"events": out})))
}

/// Recently updated public repositories, for the home page's "Explore" column.
pub async fn explore(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let list = s.db.recent_public_repos(12).await?;
    let mut out = Vec::new();
    for rw in list {
        let perm = s.db.permission(auth.user.as_ref(), &rw).await?;
        out.push(shapes::repo(&s.config, &rw, Some(perm)));
    }
    Ok(Json(json!(out)))
}

/// Who is on this Reef: every organization, and every user with a public
/// repository, each with its latest public repositories (the home page's
/// "Explore" column groups by owner).
pub async fn explore_owners(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let mut out = Vec::new();
    for kind in ["org", "user"] {
        for a in s.db.list_accounts(kind, 0, 500).await? {
            let repos = s.db.repos_for_owner(&a, None, 1, 6).await?; // no viewer: public only
            let public_repos = s.db.count_repos_for_owner(a.id(), false).await?;
            if kind == "user" && repos.is_empty() {
                continue;
            }
            let repos: Vec<Value> = repos.iter().map(|rw| shapes::repo(&s.config, rw, None)).collect();
            let mut owner = if kind == "org" { shapes::org_simple(&s.config, &a) } else { shapes::simple_user(&s.config, &a) };
            owner.as_object_mut().unwrap().insert("type".into(), json!(shapes::account_type(&a)));
            out.push(json!({ "owner": owner, "public_repos": public_repos, "repos": repos }));
        }
    }
    Ok(Json(json!(out)))
}

// ----- GitHub shape -----

fn thread_json(s: &AppState, n: &Notification, repo: Option<&reef_db::repos::RepoWithOwner>) -> Value {
    let api = s.config.api_url();
    let subject_type = match n.subject_type.as_str() {
        "issue" => "Issue",
        "pull" => "PullRequest",
        "release" => "Release",
        "run" => "CheckSuite",
        _ => "Other",
    };
    json!({
        "id": n.id.to_string(),
        "unread": n.unread != 0,
        "reason": n.reason,
        "updated_at": iso(n.updated_at),
        "last_read_at": if n.unread != 0 { Value::Null } else { json!(iso(n.updated_at)) },
        "subject": {"title": n.title, "url": Value::Null, "latest_comment_url": Value::Null, "type": subject_type},
        "repository": repo.map(|rw| shapes::repo(&s.config, rw, None)),
        "url": format!("{api}/notifications/threads/{}", n.id),
        "subscription_url": format!("{api}/notifications/threads/{}/subscription", n.id),
    })
}

#[derive(serde::Deserialize, Default)]
pub struct GithubListQuery {
    all: Option<bool>,
    per_page: Option<i64>,
}

pub async fn github_list(State(s): State<AppState>, auth: Auth, Query(q): Query<GithubListQuery>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let rows = s.db.notifications(u.id(), !q.all.unwrap_or(false), q.per_page.unwrap_or(50).clamp(1, 100)).await?;
    let mut out = Vec::new();
    for n in &rows {
        let repo = match n.repo_id {
            Some(r) => s.db.repo_with_owner_by_id(Id(r)).await.ok(),
            None => None,
        };
        out.push(thread_json(&s, n, repo.as_ref()));
    }
    Ok(Json(json!(out)))
}

pub async fn github_mark_all(State(s): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.mark_read(u.id(), None).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn github_thread(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let n = s.db.notifications(u.id(), false, 1000).await?.into_iter().find(|n| n.id == id).ok_or(reef_core::Error::NotFound)?;
    let repo = match n.repo_id {
        Some(r) => s.db.repo_with_owner_by_id(Id(r)).await.ok(),
        None => None,
    };
    Ok(Json(thread_json(&s, &n, repo.as_ref())))
}

pub async fn github_thread_read(State(s): State<AppState>, auth: Auth, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    s.db.mark_read(u.id(), Some(Id(id))).await?;
    Ok(StatusCode::RESET_CONTENT)
}
