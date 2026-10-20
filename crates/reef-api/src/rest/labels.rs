//! Repository labels and milestones.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::rest::repos::load;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use reef_core::{Error, Id};
use reef_db::repos::Permission;
use serde_json::{Value, json};

pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let all: Vec<Value> = s.db.labels(rw.repo.id()).await?.iter().map(|l| shapes::label(&s.config, &rw, l)).collect();
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/labels", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

#[derive(serde::Deserialize)]
pub struct NewLabel {
    name: String,
    color: Option<String>,
    description: Option<String>,
}

pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<NewLabel>,
) -> ApiResult<impl IntoResponse> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Triage {
        return Err(crate::error::forbidden("Must have triage access"));
    }
    let color = b.color.unwrap_or_else(random_color);
    let l = s.db.create_label(rw.repo.id(), &b.name, &color, b.description.as_deref(), false).await?;
    Ok((StatusCode::CREATED, Json(shapes::label(&s.config, &rw, &l))))
}

fn random_color() -> String {
    reef_core::token::random_hex(3)
}

pub async fn show(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let l = s.db.label_by_name(rw.repo.id(), &name).await?;
    Ok(Json(shapes::label(&s.config, &rw, &l)))
}

#[derive(serde::Deserialize)]
pub struct UpdateLabel {
    new_name: Option<String>,
    name: Option<String>,
    color: Option<String>,
    description: Option<Option<String>>,
}

pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
    Json(b): Json<UpdateLabel>,
) -> ApiResult<Json<Value>> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Triage {
        return Err(crate::error::forbidden("Must have triage access"));
    }
    let l = s.db.label_by_name(rw.repo.id(), &name).await?;
    let new_name = b.new_name.or(b.name);
    let l = s.db.update_label(Id(l.id), new_name.as_deref(), b.color.as_deref(), b.description.as_ref().map(|d| d.as_deref())).await?;
    Ok(Json(shapes::label(&s.config, &rw, &l)))
}

pub async fn delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Triage {
        return Err(crate::error::forbidden("Must have triage access"));
    }
    let l = s.db.label_by_name(rw.repo.id(), &name).await?;
    s.db.delete_label(Id(l.id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- milestones -----

#[derive(serde::Deserialize, Default)]
pub struct MilestoneQuery {
    state: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
}

pub async fn list_milestones(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
    Query(q): Query<MilestoneQuery>,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let mut ms = s.db.milestones(rw.repo.id(), q.state.as_deref().unwrap_or("open")).await?;
    if q.sort.as_deref() == Some("completeness") {
        ms.sort_by_key(|m| m.number);
    }
    if q.direction.as_deref() == Some("desc") {
        ms.reverse();
    }
    let mut all = Vec::new();
    for m in &ms {
        let (o, c) = s.db.milestone_counts(Id(m.id)).await?;
        all.push(shapes::milestone(&s.config, &rw, m, o, c, None));
    }
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/milestones", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

#[derive(serde::Deserialize)]
pub struct NewMilestone {
    title: String,
    state: Option<String>,
    description: Option<String>,
    due_on: Option<String>,
}

pub async fn create_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<NewMilestone>,
) -> ApiResult<impl IntoResponse> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have write access"));
    }
    let due = match b.due_on.as_deref() {
        Some(d) => Some(
            reef_core::time::parse_iso(d)
                .ok_or_else(|| ApiError(Error::field("Milestone", "due_on", "invalid", "due_on must be an ISO 8601 timestamp")))?,
        ),
        None => None,
    };
    let m = s.db.create_milestone(rw.repo.id(), &b.title, b.description.as_deref(), due, b.state.as_deref().unwrap_or("open")).await?;
    Ok((StatusCode::CREATED, Json(shapes::milestone(&s.config, &rw, &m, 0, 0, auth.user.as_ref()))))
}

pub async fn show_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let m = s.db.milestone_by_number(rw.repo.id(), number).await?;
    let (o, c) = s.db.milestone_counts(Id(m.id)).await?;
    Ok(Json(shapes::milestone(&s.config, &rw, &m, o, c, None)))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateMilestone {
    title: Option<String>,
    state: Option<String>,
    description: Option<Option<String>>,
    due_on: Option<Option<String>>,
}

pub async fn update_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<UpdateMilestone>,
) -> ApiResult<Json<Value>> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have write access"));
    }
    let m = s.db.milestone_by_number(rw.repo.id(), number).await?;
    let due = match &b.due_on {
        Some(Some(d)) => Some(Some(
            reef_core::time::parse_iso(d)
                .ok_or_else(|| ApiError(Error::field("Milestone", "due_on", "invalid", "due_on must be an ISO 8601 timestamp")))?,
        )),
        Some(None) => Some(None),
        None => None,
    };
    let m =
        s.db.update_milestone(Id(m.id), b.title.as_deref(), b.description.as_ref().map(|d| d.as_deref()), due, b.state.as_deref()).await?;
    let (o, c) = s.db.milestone_counts(Id(m.id)).await?;
    Ok(Json(shapes::milestone(&s.config, &rw, &m, o, c, None)))
}

pub async fn delete_milestone(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have write access"));
    }
    let m = s.db.milestone_by_number(rw.repo.id(), number).await?;
    s.db.delete_milestone(Id(m.id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn milestone_labels(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let m = s.db.milestone_by_number(rw.repo.id(), number).await?;
    let f = reef_db::issues::IssueFilter { state: "all".into(), milestone: Some(m.number.to_string()), ..Default::default() };
    let (issues, _) = s.db.list_issues(Some(rw.repo.id()), &f, 1, 500).await?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for i in issues {
        for l in i.labels {
            if seen.insert(l.id) {
                out.push(shapes::label(&s.config, &rw, &l));
            }
        }
    }
    Ok(Json(json!(out)))
}
