//! GitHub's Actions REST API: workflows, runs, jobs, logs, runners, artifacts, secrets and
//! variables, plus check runs derived from jobs.

use super::schedule;
use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::rest::repos::load;
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use reef_core::time::{iso, iso_opt};
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::actions::{Job, Run, RunFilter, Runner, Workflow};
use reef_db::repos::{Permission, RepoWithOwner};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/actions/runner-registration", post(super::runner::registration))
        .route("/repos/{owner}/{repo}/actions/workflows", get(workflows))
        .route("/repos/{owner}/{repo}/actions/workflows/{id}", get(workflow))
        .route("/repos/{owner}/{repo}/actions/workflows/{id}/runs", get(workflow_runs))
        .route("/repos/{owner}/{repo}/actions/workflows/{id}/dispatches", post(dispatch))
        .route("/repos/{owner}/{repo}/actions/workflows/{id}/enable", put(enable_workflow))
        .route("/repos/{owner}/{repo}/actions/workflows/{id}/disable", put(disable_workflow))
        .route("/repos/{owner}/{repo}/actions/runs", get(runs))
        .route("/repos/{owner}/{repo}/actions/runs/{id}", get(run).delete(delete_run))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/jobs", get(run_jobs))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/attempts/{attempt}", get(run_attempt))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/attempts/{attempt}/jobs", get(run_attempt_jobs))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/attempts/{attempt}/logs", get(run_logs))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/logs", get(run_logs_latest).delete(delete_run_logs))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/cancel", post(cancel_run))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/rerun", post(rerun))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/rerun-failed-jobs", post(rerun_failed))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/artifacts", get(run_artifacts))
        .route("/repos/{owner}/{repo}/actions/runs/{id}/timing", get(run_timing))
        .route("/repos/{owner}/{repo}/actions/jobs/{id}", get(job))
        .route("/repos/{owner}/{repo}/actions/jobs/{id}/logs", get(job_logs))
        .route("/repos/{owner}/{repo}/actions/jobs/{id}/rerun", post(rerun_job))
        .route("/repos/{owner}/{repo}/actions/runners", get(repo_runners))
        .route("/repos/{owner}/{repo}/actions/runners/{id}", get(repo_runner).delete(delete_repo_runner))
        .route("/repos/{owner}/{repo}/actions/runners/registration-token", post(repo_registration_token))
        .route("/repos/{owner}/{repo}/actions/runners/remove-token", post(repo_remove_token))
        .route("/repos/{owner}/{repo}/actions/runners/downloads", get(runner_downloads))
        .route("/orgs/{org}/actions/runners", get(org_runners))
        .route("/orgs/{org}/actions/runners/{id}", get(org_runner).delete(delete_org_runner))
        .route("/orgs/{org}/actions/runners/registration-token", post(org_registration_token))
        .route("/orgs/{org}/actions/runners/remove-token", post(org_remove_token))
        .route("/orgs/{org}/actions/runners/downloads", get(runner_downloads_org))
        .route("/repos/{owner}/{repo}/actions/artifacts", get(artifacts))
        .route("/repos/{owner}/{repo}/actions/artifacts/{id}", get(artifact).delete(delete_artifact))
        .route("/repos/{owner}/{repo}/actions/artifacts/{id}/{format}", get(download_artifact))
        .route("/repos/{owner}/{repo}/actions/secrets", get(repo_secrets))
        .route("/repos/{owner}/{repo}/actions/secrets/public-key", get(repo_public_key))
        .route("/repos/{owner}/{repo}/actions/secrets/{name}", get(repo_secret).put(put_repo_secret).delete(delete_repo_secret))
        .route("/repos/{owner}/{repo}/actions/variables", get(repo_variables).post(create_repo_variable))
        .route("/repos/{owner}/{repo}/actions/variables/{name}", get(repo_variable).patch(patch_repo_variable).delete(delete_repo_variable))
        .route("/orgs/{org}/actions/secrets", get(org_secrets))
        .route("/orgs/{org}/actions/secrets/public-key", get(org_public_key))
        .route("/orgs/{org}/actions/secrets/{name}", get(org_secret).put(put_org_secret).delete(delete_org_secret))
        .route("/orgs/{org}/actions/variables", get(org_variables).post(create_org_variable))
        .route("/orgs/{org}/actions/variables/{name}", get(org_variable).patch(patch_org_variable).delete(delete_org_variable))
        .route("/repos/{owner}/{repo}/commits/{git_ref}/check-runs", get(check_runs))
        .route("/repos/{owner}/{repo}/commits/{git_ref}/check-suites", get(check_suites))
        .route("/repos/{owner}/{repo}/check-runs/{id}", get(check_run))
        .route("/repos/{owner}/{repo}/actions/permissions", get(permissions))
}

// ----- shapes -----

fn workflow_json(s: &AppState, rw: &RepoWithOwner, w: &Workflow) -> Value {
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    json!({
        "id": w.id,
        "node_id": shapes::node_id("W", w.id),
        "name": w.name,
        "path": w.path,
        "state": if w.state == "disabled_manually" { "disabled_manually" } else if w.state == "deleted" { "deleted" } else { "active" },
        "created_at": iso(w.created_at),
        "updated_at": iso(w.updated_at),
        "url": format!("{api}/actions/workflows/{}", w.id),
        "html_url": format!("{}/{}/blob/{}/{}", s.config.site_url(), rw.full_name(), rw.repo.default_branch, w.path),
        "badge_url": format!("{}/{}/workflows/{}/badge.svg", s.config.site_url(), rw.full_name(), shapes::urlpath(&w.name)),
    })
}

pub async fn run_json(s: &AppState, rw: &RepoWithOwner, r: &Run, cache: &mut HashMap<i64, Option<Account>>) -> Value {
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    let site = format!("{}/{}", s.config.site_url(), rw.full_name());
    let actor = account(s, cache, r.actor_id).await;
    let triggering = account(s, cache, r.triggering_actor_id).await.or(actor.clone());
    let head_commit = {
        let path = s.store.path(rw.repo.id());
        let sha = r.head_sha.clone();
        tokio::task::spawn_blocking(move || reef_git::RepoReader::open(&path).ok()?.commit(&sha).ok()).await.ok().flatten()
    };
    let workflow = s.db.workflow_by_id(Id(r.workflow_id)).await.ok();
    let mut prs = Vec::new();
    if let Some(n) = r.pull_number {
        prs.push(json!({"number": n, "url": format!("{api}/pulls/{n}"), "head": {"ref": r.head_branch, "sha": r.head_sha, "repo": {"id": rw.repo.id, "name": rw.repo.name, "url": api}}, "base": {"ref": rw.repo.default_branch, "sha": "", "repo": {"id": rw.repo.id, "name": rw.repo.name, "url": api}}}));
    }
    json!({
        "id": r.id,
        "name": r.name,
        "node_id": shapes::node_id("WFR", r.id),
        "head_branch": r.head_branch,
        "head_sha": r.head_sha,
        "path": r.path,
        "display_title": r.display_title,
        "run_number": r.run_number,
        "run_attempt": r.run_attempt,
        "event": r.event,
        "status": r.status,
        "conclusion": r.conclusion,
        "workflow_id": r.workflow_id,
        "check_suite_id": r.id,
        "check_suite_node_id": shapes::node_id("CS", r.id),
        "url": format!("{api}/actions/runs/{}", r.id),
        "html_url": format!("{site}/actions/runs/{}", r.id),
        "pull_requests": prs,
        "referenced_workflows": [],
        "created_at": iso(r.created_at),
        "updated_at": iso(r.updated_at),
        "run_started_at": iso_opt(r.started_at.or(Some(r.created_at))),
        "actor": actor.as_ref().map(|a| shapes::simple_user(&s.config, a)),
        "triggering_actor": triggering.as_ref().map(|a| shapes::simple_user(&s.config, a)),
        "jobs_url": format!("{api}/actions/runs/{}/jobs", r.id),
        "logs_url": format!("{api}/actions/runs/{}/logs", r.id),
        "check_suite_url": format!("{api}/check-suites/{}", r.id),
        "artifacts_url": format!("{api}/actions/runs/{}/artifacts", r.id),
        "cancel_url": format!("{api}/actions/runs/{}/cancel", r.id),
        "rerun_url": format!("{api}/actions/runs/{}/rerun", r.id),
        "previous_attempt_url": if r.run_attempt > 1 { json!(format!("{api}/actions/runs/{}/attempts/{}", r.id, r.run_attempt - 1)) } else { Value::Null },
        "workflow_url": format!("{api}/actions/workflows/{}", r.workflow_id),
        "head_commit": head_commit.map(|c| json!({"id": c.id, "tree_id": c.tree, "message": c.message, "timestamp": iso(c.committer.time_ms), "author": {"name": c.author.name, "email": c.author.email}, "committer": {"name": c.committer.name, "email": c.committer.email}})),
        "repository": shapes::repo(&s.config, rw, None),
        "head_repository": shapes::repo(&s.config, rw, None),
        "workflow_name": workflow.map(|w| w.name),
        "error": r.error,
    })
}

async fn account(s: &AppState, cache: &mut HashMap<i64, Option<Account>>, id: Option<i64>) -> Option<Account> {
    let id = id?;
    if let Some(a) = cache.get(&id) {
        return a.clone();
    }
    let a = s.db.account_by_id(Id(id)).await.ok();
    cache.insert(id, a.clone());
    a
}

fn step_status(state: &str) -> &'static str {
    match state {
        "completed" => "completed",
        "inProgress" => "in_progress",
        "pending" => "queued",
        _ => "queued",
    }
}

fn result_conclusion(r: Option<&str>) -> Option<&'static str> {
    match r {
        Some("succeeded") | Some("succeededWithIssues") => Some("success"),
        Some("failed") => Some("failure"),
        Some("canceled") => Some("cancelled"),
        Some("skipped") => Some("skipped"),
        Some("abandoned") => Some("failure"),
        _ => None,
    }
}

/// Steps from the timeline records (type `Task`, ordered).
pub fn job_steps(j: &Job) -> Vec<Value> {
    let mut steps: Vec<(i64, Value)> = j
        .timeline_json()
        .iter()
        .filter(|r| r.get("type").and_then(|t| t.as_str()) == Some("Task"))
        .map(|r| {
            let order = r.get("order").and_then(|o| o.as_i64()).unwrap_or(0);
            (
                order,
                json!({
                    "name": r.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                    "status": step_status(r.get("state").and_then(|st| st.as_str()).unwrap_or("pending")),
                    "conclusion": result_conclusion(r.get("result").and_then(|x| x.as_str())),
                    "number": order,
                    "started_at": r.get("startTime").cloned().unwrap_or(Value::Null),
                    "completed_at": r.get("finishTime").cloned().unwrap_or(Value::Null),
                    "record_id": r.get("id").cloned().unwrap_or(Value::Null),
                    "log_id": r.pointer("/log/id").cloned().unwrap_or(Value::Null),
                }),
            )
        })
        .collect();
    steps.sort_by_key(|(o, _)| *o);
    steps.into_iter().map(|(_, v)| v).collect()
}

pub fn job_json(s: &AppState, rw: &RepoWithOwner, run: &Run, j: &Job) -> Value {
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    let site = format!("{}/{}", s.config.site_url(), rw.full_name());
    json!({
        "id": j.id,
        "run_id": j.run_id,
        "workflow_name": run.name,
        "head_branch": run.head_branch,
        "run_url": format!("{api}/actions/runs/{}", j.run_id),
        "run_attempt": j.run_attempt,
        "node_id": shapes::node_id("CR", j.id),
        "head_sha": run.head_sha,
        "url": format!("{api}/actions/jobs/{}", j.id),
        "html_url": format!("{site}/actions/runs/{}/job/{}", j.run_id, j.id),
        "status": match j.status.as_str() { "waiting" => "waiting", "queued" => "queued", "in_progress" => "in_progress", _ => "completed" },
        "conclusion": j.conclusion,
        "created_at": iso(j.created_at),
        "started_at": iso_opt(j.started_at),
        "completed_at": iso_opt(j.completed_at),
        "name": j.name,
        "steps": job_steps(j),
        "check_run_url": format!("{api}/check-runs/{}", j.id),
        "labels": j.labels(),
        "runner_id": j.runner_id,
        "runner_name": j.runner_name,
        "runner_group_id": 1,
        "runner_group_name": "Default",
        "job_key": j.job_key,
        "matrix": j.matrix_json(),
    })
}

fn runner_json(r: &Runner) -> Value {
    let now = reef_core::time::now_ms();
    json!({
        "id": r.id,
        "name": r.name,
        "os": r.os.as_deref().map(|o| if o.to_lowercase().contains("darwin") || o.to_lowercase().contains("mac") { "macOS" } else if o.to_lowercase().contains("windows") { "Windows" } else { "Linux" }).unwrap_or("Linux"),
        "status": if r.online(now) { "online" } else { "offline" },
        "busy": r.busy != 0,
        "ephemeral": r.ephemeral != 0,
        "labels": r.labels().iter().enumerate().map(|(i, l)| json!({"id": i + 1, "name": l, "type": if matches!(l.as_str(), "self-hosted" | "Linux" | "Windows" | "macOS" | "X64" | "ARM64" | "ARM") { "read-only" } else { "custom" }})).collect::<Vec<_>>(),
        "version": r.version,
        "last_seen_at": iso_opt(r.last_seen_at),
    })
}

fn artifact_json(s: &AppState, rw: &RepoWithOwner, run: Option<&Run>, a: &reef_db::actions::Artifact) -> Value {
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    json!({
        "id": a.id,
        "node_id": shapes::node_id("A", a.id),
        "name": a.name,
        "size_in_bytes": a.size,
        "url": format!("{api}/actions/artifacts/{}", a.id),
        "archive_download_url": format!("{api}/actions/artifacts/{}/zip", a.id),
        "expired": a.expired != 0 || a.expires_at.map(|e| e < reef_core::time::now_ms()).unwrap_or(false),
        "created_at": iso(a.created_at),
        "expires_at": iso_opt(a.expires_at),
        "updated_at": iso(a.updated_at),
        "workflow_run": run.map(|r| json!({"id": r.id, "repository_id": r.repo_id, "head_repository_id": r.repo_id, "head_branch": r.head_branch, "head_sha": r.head_sha})),
    })
}

// ----- workflows -----

async fn load_write(s: &AppState, auth: &Auth, owner: &str, repo: &str) -> ApiResult<RepoWithOwner> {
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have write access to this repository"));
    }
    Ok(rw)
}

pub async fn workflows(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.is_empty == 0 && s.db.workflows(rw.repo.id()).await?.is_empty() {
        // Lazily discover workflows for repositories pushed before Actions existed.
        let path = s.store.path(rw.repo.id());
        if let Ok(sha) = reef_git::ops::rev_parse(s.store.git_binary(), &path, &rw.repo.default_branch).await {
            let _ = super::plan::sync_workflows(&s, &rw, &sha).await;
        }
    }
    let list = s.db.workflows(rw.repo.id()).await?;
    Ok(Json(json!({"total_count": list.len(), "workflows": list.iter().map(|w| workflow_json(&s, &rw, w)).collect::<Vec<_>>()})))
}

pub async fn workflow(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let w = s.db.workflow_by_ref(rw.repo.id(), &id).await?;
    Ok(Json(workflow_json(&s, &rw, &w)))
}

#[derive(serde::Deserialize, Default)]
pub struct RunsQuery {
    actor: Option<String>,
    branch: Option<String>,
    event: Option<String>,
    status: Option<String>,
    head_sha: Option<String>,
    created: Option<String>,
    exclude_pull_requests: Option<bool>,
}

async fn list_runs(
    s: &AppState,
    auth: &Auth,
    rw: &RepoWithOwner,
    workflow: Option<Id>,
    q: RunsQuery,
    page: Page,
) -> ApiResult<Json<Value>> {
    let actor = match &q.actor {
        Some(a) => Some(s.db.account_by_login(a).await?.id()),
        None => None,
    };
    let created_after = q
        .created
        .as_deref()
        .and_then(|c| c.trim_start_matches(">=").trim_start_matches('>').parse::<String>().ok())
        .and_then(|d| reef_core::time::parse_iso(&d).or_else(|| reef_core::time::parse_iso(&format!("{d}T00:00:00Z"))));
    let f = RunFilter {
        workflow,
        actor,
        branch: q.branch.clone(),
        event: q.event.clone(),
        status: q.status.clone(),
        head_sha: q.head_sha.clone(),
        created_after,
    };
    let (rows, total) =
        s.db.runs(rw.repo.id(), &f, page.per_page.clamp(1, 100), (page.page.max(1) - 1) * page.per_page.clamp(1, 100)).await?;
    let mut cache = HashMap::new();
    let mut out = Vec::new();
    for r in &rows {
        if q.exclude_pull_requests == Some(true) && r.event == "pull_request" {
            continue;
        }
        out.push(run_json(s, rw, r, &mut cache).await);
    }
    let _ = auth;
    Ok(Json(json!({"total_count": total, "workflow_runs": out})))
}

pub async fn runs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    list_runs(&s, &auth, &rw, None, q, page).await
}

pub async fn workflow_runs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, String)>,
    page: Page,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let w = s.db.workflow_by_ref(rw.repo.id(), &id).await?;
    list_runs(&s, &auth, &rw, Some(w.id()), q, page).await
}

#[derive(serde::Deserialize)]
pub struct Dispatch {
    #[serde(rename = "ref")]
    git_ref: String,
    inputs: Option<Value>,
}

pub async fn dispatch(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, String)>,
    Json(b): Json<Dispatch>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let w = s.db.workflow_by_ref(rw.repo.id(), &id).await?;
    let runs = super::dispatch(&s, &rw, &u, &w.path, &b.git_ref, b.inputs.unwrap_or(json!({}))).await?;
    if runs.is_empty() {
        return Err(ApiError(Error::Validation { message: "Workflow does not have 'workflow_dispatch' trigger".into(), errors: vec![] }));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn enable_workflow(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let w = s.db.workflow_by_ref(rw.repo.id(), &id).await?;
    s.db.set_workflow_state(w.id(), "active").await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn disable_workflow(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let w = s.db.workflow_by_ref(rw.repo.id(), &id).await?;
    s.db.set_workflow_state(w.id(), "disabled_manually").await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- runs -----

async fn load_run(s: &AppState, rw: &RepoWithOwner, id: i64) -> ApiResult<Run> {
    let r = s.db.run_by_id(Id(id)).await?;
    if r.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    Ok(r)
}

pub async fn run(State(s): State<AppState>, auth: Auth, Path((owner, repo, id)): Path<(String, String, i64)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    let mut cache = HashMap::new();
    Ok(Json(run_json(&s, &rw, &r, &mut cache).await))
}

pub async fn run_attempt(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id, _attempt)): Path<(String, String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    run(State(s), auth, Path((owner, repo, id))).await
}

#[derive(serde::Deserialize, Default)]
pub struct JobsQuery {
    filter: Option<String>,
}

pub async fn run_jobs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Query(q): Query<JobsQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    let attempt = if q.filter.as_deref() == Some("all") { None } else { Some(r.run_attempt) };
    let jobs = s.db.jobs_of_run(r.id(), attempt).await?;
    Ok(Json(json!({"total_count": jobs.len(), "jobs": jobs.iter().map(|j| job_json(&s, &rw, &r, j)).collect::<Vec<_>>()})))
}

pub async fn run_attempt_jobs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id, attempt)): Path<(String, String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    let jobs = s.db.jobs_of_run(r.id(), Some(attempt)).await?;
    Ok(Json(json!({"total_count": jobs.len(), "jobs": jobs.iter().map(|j| job_json(&s, &rw, &r, j)).collect::<Vec<_>>()})))
}

pub async fn cancel_run(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    if r.status == "completed" {
        return Err(ApiError(Error::Conflict("Cannot cancel a workflow run that is completed.".into())));
    }
    schedule::cancel_run(&s, &r).await?;
    Ok(StatusCode::ACCEPTED)
}

pub async fn rerun(State(s): State<AppState>, auth: Auth, Path((owner, repo, id)): Path<(String, String, i64)>) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    schedule::rerun(&s, &r, Some(&u), false).await?;
    Ok(StatusCode::CREATED)
}

pub async fn rerun_failed(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    schedule::rerun(&s, &r, Some(&u), true).await?;
    Ok(StatusCode::CREATED)
}

pub async fn rerun_job(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?.clone();
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let j = s.db.job_by_id(Id(id)).await?;
    if j.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let r = s.db.run_by_id(Id(j.run_id)).await?;
    schedule::rerun(&s, &r, Some(&u), true).await?;
    Ok(StatusCode::CREATED)
}

pub async fn delete_run(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    if r.status != "completed" {
        return Err(ApiError(Error::Conflict("Cannot delete a run that is not completed.".into())));
    }
    s.db.delete_run(r.id()).await?;
    s.actions.files.remove_run(r.id).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_run_logs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    let _ = tokio::fs::remove_dir_all(s.actions.files.root().join(format!("logs/{}", r.id))).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_timing(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    let ms = r.completed_at.unwrap_or(reef_core::time::now_ms()) - r.started_at.unwrap_or(r.created_at);
    Ok(Json(
        json!({"billable": {"UBUNTU": {"total_ms": ms, "jobs": s.db.jobs_of_run(r.id(), Some(r.run_attempt)).await?.len()}}, "run_duration_ms": ms}),
    ))
}

/// The full text of a job: the final step logs when uploaded, else the live feed.
pub async fn job_log_text(s: &AppState, j: &Job) -> String {
    let mut out = String::new();
    let steps = job_steps(j);
    let mut used_final = false;
    for st in &steps {
        let name = st.get("name").and_then(|n| n.as_str()).unwrap_or("step");
        out.push_str(&format!("##[group]{name}\n"));
        let mut text = String::new();
        if let Some(log_id) = st.get("log_id").and_then(|l| l.as_i64())
            && let Ok(l) = s.db.job_log(log_id).await
            && let Ok(b) = s.actions.files.read(&l.path).await
        {
            text = String::from_utf8_lossy(&b).into_owned();
            used_final = true;
        }
        if text.is_empty()
            && let Some(rid) = st.get("record_id").and_then(|r| r.as_str())
            && let Ok(b) = s.actions.files.read(&format!("logs/{}/{}/live-{}.log", j.run_id, j.id, rid)).await
        {
            text = String::from_utf8_lossy(&b).into_owned();
        }
        out.push_str(&text);
        if !text.ends_with('\n') && !text.is_empty() {
            out.push('\n');
        }
        out.push_str("##[endgroup]\n");
    }
    if steps.is_empty() {
        // No steps yet: whatever the job record itself logged.
        if let Ok(b) = s.actions.files.read(&format!("logs/{}/{}/live-{}.log", j.run_id, j.id, j.job_guid)).await {
            out.push_str(&String::from_utf8_lossy(&b));
        }
    }
    let _ = used_final;
    out
}

pub async fn job_logs(State(s): State<AppState>, auth: Auth, Path((owner, repo, id)): Path<(String, String, i64)>) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let j = s.db.job_by_id(Id(id)).await?;
    if j.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let text = job_log_text(&s, &j).await;
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response())
}

/// The run's logs as a zip: `<job name>/<n>_<step name>.txt` like GitHub.
async fn logs_zip(s: &AppState, rw: &RepoWithOwner, r: &Run, attempt: i64) -> ApiResult<Response> {
    let jobs = s.db.jobs_of_run(r.id(), Some(attempt)).await?;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let safe = |x: &str| x.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect::<String>();
        for j in &jobs {
            let steps = job_steps(j);
            let mut whole = String::new();
            for (i, st) in steps.iter().enumerate() {
                let name = st.get("name").and_then(|n| n.as_str()).unwrap_or("step");
                let mut text = String::new();
                if let Some(log_id) = st.get("log_id").and_then(|l| l.as_i64())
                    && let Ok(l) = s.db.job_log(log_id).await
                    && let Ok(b) = s.actions.files.read(&l.path).await
                {
                    text = String::from_utf8_lossy(&b).into_owned();
                }
                if text.is_empty()
                    && let Some(rid) = st.get("record_id").and_then(|x| x.as_str())
                    && let Ok(b) = s.actions.files.read(&format!("logs/{}/{}/live-{}.log", j.run_id, j.id, rid)).await
                {
                    text = String::from_utf8_lossy(&b).into_owned();
                }
                zip.start_file(format!("{}/{}_{}.txt", safe(&j.name), i + 1, safe(name)), opts).map_err(Error::internal)?;
                zip.write_all(text.as_bytes()).map_err(Error::internal)?;
                whole.push_str(&text);
            }
            zip.start_file(format!("{}.txt", safe(&j.name)), opts).map_err(Error::internal)?;
            zip.write_all(whole.as_bytes()).map_err(Error::internal)?;
        }
        zip.finish().map_err(Error::internal)?;
    }
    let _ = rw;
    Ok(([(header::CONTENT_TYPE, "application/zip"), (header::CONTENT_DISPOSITION, "attachment; filename=logs.zip")], buf.into_inner())
        .into_response())
}

pub async fn run_logs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id, attempt)): Path<(String, String, i64, i64)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    logs_zip(&s, &rw, &r, attempt).await
}

pub async fn run_logs_latest(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    logs_zip(&s, &rw, &r, r.run_attempt).await
}

pub async fn job(State(s): State<AppState>, auth: Auth, Path((owner, repo, id)): Path<(String, String, i64)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let j = s.db.job_by_id(Id(id)).await?;
    if j.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let r = s.db.run_by_id(Id(j.run_id)).await?;
    Ok(Json(job_json(&s, &rw, &r, &j)))
}

// ----- runners -----

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

async fn repo_admin(s: &AppState, auth: &Auth, owner: &str, repo: &str) -> ApiResult<RepoWithOwner> {
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    Ok(rw)
}

fn runners_list(list: Vec<Runner>) -> Json<Value> {
    Json(json!({"total_count": list.len(), "runners": list.iter().map(runner_json).collect::<Vec<_>>()}))
}

pub async fn repo_runners(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    let mut list = s.db.runners("repo", rw.repo.id()).await?;
    if rw.owner.is_org() {
        list.extend(s.db.runners("org", rw.owner.id()).await?);
    }
    Ok(runners_list(list))
}

pub async fn org_runners(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    Ok(runners_list(s.db.runners("org", o.id()).await?))
}

pub async fn repo_runner(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    let r = s.db.runner_by_id(Id(id)).await?;
    let in_scope = (r.scope_kind == "repo" && r.scope_id == rw.repo.id) || (r.scope_kind == "org" && r.scope_id == rw.owner.id);
    if !in_scope {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(runner_json(&r)))
}

pub async fn org_runner(State(s): State<AppState>, auth: Auth, Path((org, id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    let r = s.db.runner_by_id(Id(id)).await?;
    if r.scope_kind != "org" || r.scope_id != o.id {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(runner_json(&r)))
}

pub async fn delete_repo_runner(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    let r = s.db.runner_by_id(Id(id)).await?;
    if r.scope_kind != "repo" || r.scope_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.delete_runner(r.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_org_runner(State(s): State<AppState>, auth: Auth, Path((org, id)): Path<(String, i64)>) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    let r = s.db.runner_by_id(Id(id)).await?;
    if r.scope_kind != "org" || r.scope_id != o.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.delete_runner(r.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn token_json(s: &AppState, kind: &str, id: Id, purpose: &str, by: Option<Id>) -> ApiResult<Json<Value>> {
    let (token, expires) = s.db.create_runner_token(kind, id, purpose, by, 3_600_000).await?;
    Ok(Json(json!({"token": token, "expires_at": iso(expires)})))
}

pub async fn repo_registration_token(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    Ok((StatusCode::CREATED, token_json(&s, "repo", rw.repo.id(), "register", auth.user.as_ref().map(|u| u.id())).await?))
}

pub async fn repo_remove_token(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    Ok((StatusCode::CREATED, token_json(&s, "repo", rw.repo.id(), "remove", auth.user.as_ref().map(|u| u.id())).await?))
}

pub async fn org_registration_token(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let o = org_admin(&s, &auth, &org).await?;
    Ok((StatusCode::CREATED, token_json(&s, "org", o.id(), "register", auth.user.as_ref().map(|u| u.id())).await?))
}

pub async fn org_remove_token(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<(StatusCode, Json<Value>)> {
    let o = org_admin(&s, &auth, &org).await?;
    Ok((StatusCode::CREATED, token_json(&s, "org", o.id(), "remove", auth.user.as_ref().map(|u| u.id())).await?))
}

const RUNNER_VERSION: &str = "2.337.0";

fn downloads(s: &AppState) -> Json<Value> {
    let base = s.config.actions.runner_download_base.trim_end_matches('/');
    let list: Vec<Value> =
        [("osx", "x64"), ("osx", "arm64"), ("linux", "x64"), ("linux", "arm64"), ("linux", "arm"), ("win", "x64"), ("win", "arm64")]
            .iter()
            .map(|(os, arch)| {
                let ext = if *os == "win" { "zip" } else { "tar.gz" };
                let file = format!("actions-runner-{os}-{arch}-{RUNNER_VERSION}.{ext}");
                json!({"os": os, "architecture": arch, "download_url": format!("{base}/v{RUNNER_VERSION}/{file}"), "filename": file})
            })
            .collect();
    Json(json!(list))
}

pub async fn runner_downloads(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    repo_admin(&s, &auth, &owner, &repo).await?;
    Ok(downloads(&s))
}

pub async fn runner_downloads_org(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    org_admin(&s, &auth, &org).await?;
    Ok(downloads(&s))
}

// ----- artifacts -----

#[derive(serde::Deserialize, Default)]
pub struct ArtifactsQuery {
    name: Option<String>,
}

pub async fn artifacts(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
    Query(q): Query<ArtifactsQuery>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let per = page.per_page.clamp(1, 100);
    let (rows, total) = s.db.artifacts_of_repo(rw.repo.id(), q.name.as_deref(), per, (page.page.max(1) - 1) * per).await?;
    let mut out = Vec::new();
    for a in &rows {
        let run = s.db.run_by_id(Id(a.run_id)).await.ok();
        out.push(artifact_json(&s, &rw, run.as_ref(), a));
    }
    Ok(Json(json!({"total_count": total, "artifacts": out})))
}

pub async fn run_artifacts(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let r = load_run(&s, &rw, id).await?;
    let list = s.db.artifacts_of_run(r.id()).await?;
    Ok(Json(json!({"total_count": list.len(), "artifacts": list.iter().map(|a| artifact_json(&s, &rw, Some(&r), a)).collect::<Vec<_>>()})))
}

pub async fn artifact(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let a = s.db.artifact_by_id(Id(id)).await?;
    if a.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let run = s.db.run_by_id(Id(a.run_id)).await.ok();
    Ok(Json(artifact_json(&s, &rw, run.as_ref(), &a)))
}

pub async fn delete_artifact(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let rw = load_write(&s, &auth, &owner, &repo).await?;
    let a = s.db.artifact_by_id(Id(id)).await?;
    if a.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    if let Some(p) = &a.path {
        let _ = s.actions.files.remove(p).await;
    }
    s.db.delete_artifact(Id(a.id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download_artifact(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id, _format)): Path<(String, String, i64, String)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let a = s.db.artifact_by_id(Id(id)).await?;
    if a.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let Some(p) = &a.path else { return Err(ApiError(Error::NotFound)) };
    // Stream the zip from disk: artifacts can be large and must not be held in memory.
    let file = match tokio::fs::File::open(s.actions.files.abs(p)).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ApiError(Error::NotFound)),
        Err(e) => return Err(ApiError(Error::internal(e))),
    };
    let len = file.metadata().await.ok().map(|m| m.len());
    let mut res = Response::new(axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, axum::http::HeaderValue::from_static("application/zip"));
    if let Ok(v) = axum::http::HeaderValue::from_str(&format!("attachment; filename={}.zip", a.name)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    if let Some(len) = len {
        h.insert(header::CONTENT_LENGTH, axum::http::HeaderValue::from(len));
    }
    Ok(res)
}

// ----- secrets & variables -----

fn secret_json(sec: &reef_db::actions::Secret) -> Value {
    json!({"name": sec.name, "created_at": iso(sec.created_at), "updated_at": iso(sec.updated_at), "visibility": if sec.scope_kind == "org" { "all" } else { "private" }})
}

fn variable_json(v: &reef_db::actions::Variable) -> Value {
    json!({"name": v.name, "value": v.value, "created_at": iso(v.created_at), "updated_at": iso(v.updated_at), "visibility": if v.scope_kind == "org" { "all" } else { "private" }})
}

fn public_key(s: &AppState) -> Json<Value> {
    Json(json!({"key_id": s.actions.keys.key_id, "key": s.actions.keys.public_key_b64()}))
}

#[derive(serde::Deserialize)]
pub struct PutSecret {
    encrypted_value: String,
    key_id: Option<String>,
}

async fn put_secret(s: &AppState, kind: &str, id: Id, name: &str, b: PutSecret) -> ApiResult<StatusCode> {
    if let Some(k) = &b.key_id
        && k != &s.actions.keys.key_id
    {
        return Err(ApiError(Error::validation("key_id does not match the current public key")));
    }
    let plain = s.actions.keys.unseal_from_client(&b.encrypted_value)?;
    let existed = s.db.secret(kind, id, name).await.is_ok();
    s.db.set_secret(kind, id, name, &s.actions.keys.seal(&plain)?).await?;
    Ok(if existed { StatusCode::NO_CONTENT } else { StatusCode::CREATED })
}

pub async fn repo_secrets(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    let list = s.db.secrets("repo", rw.repo.id()).await?;
    Ok(Json(json!({"total_count": list.len(), "secrets": list.iter().map(secret_json).collect::<Vec<_>>()})))
}

pub async fn repo_public_key(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    load_write(&s, &auth, &owner, &repo).await?;
    Ok(public_key(&s))
}

pub async fn repo_secret(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    Ok(Json(secret_json(&s.db.secret("repo", rw.repo.id(), &name).await?)))
}

pub async fn put_repo_secret(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
    Json(b): Json<PutSecret>,
) -> ApiResult<StatusCode> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    put_secret(&s, "repo", rw.repo.id(), &name, b).await
}

pub async fn delete_repo_secret(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    s.db.delete_secret("repo", rw.repo.id(), &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct VariableBody {
    name: Option<String>,
    value: String,
}

pub async fn repo_variables(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Write {
        return Err(crate::error::forbidden("Must have write access"));
    }
    let list = s.db.variables("repo", rw.repo.id()).await?;
    Ok(Json(json!({"total_count": list.len(), "variables": list.iter().map(variable_json).collect::<Vec<_>>()})))
}

pub async fn create_repo_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<VariableBody>,
) -> ApiResult<StatusCode> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    let name = b.name.ok_or_else(|| ApiError(Error::validation("name is required")))?;
    s.db.set_variable("repo", rw.repo.id(), &name, &b.value).await?;
    Ok(StatusCode::CREATED)
}

pub async fn repo_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    Ok(Json(variable_json(&s.db.variable("repo", rw.repo.id(), &name).await?)))
}

pub async fn patch_repo_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
    Json(b): Json<VariableBody>,
) -> ApiResult<StatusCode> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    s.db.set_variable("repo", rw.repo.id(), b.name.as_deref().unwrap_or(&name), &b.value).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_repo_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, name)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let rw = repo_admin(&s, &auth, &owner, &repo).await?;
    s.db.delete_variable("repo", rw.repo.id(), &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn org_secrets(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    let list = s.db.secrets("org", o.id()).await?;
    Ok(Json(json!({"total_count": list.len(), "secrets": list.iter().map(secret_json).collect::<Vec<_>>()})))
}

pub async fn org_public_key(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    org_admin(&s, &auth, &org).await?;
    Ok(public_key(&s))
}

pub async fn org_secret(State(s): State<AppState>, auth: Auth, Path((org, name)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    Ok(Json(secret_json(&s.db.secret("org", o.id(), &name).await?)))
}

pub async fn put_org_secret(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, name)): Path<(String, String)>,
    Json(b): Json<PutSecret>,
) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    put_secret(&s, "org", o.id(), &name, b).await
}

pub async fn delete_org_secret(State(s): State<AppState>, auth: Auth, Path((org, name)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    s.db.delete_secret("org", o.id(), &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn org_variables(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    let list = s.db.variables("org", o.id()).await?;
    Ok(Json(json!({"total_count": list.len(), "variables": list.iter().map(variable_json).collect::<Vec<_>>()})))
}

pub async fn create_org_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
    Json(b): Json<VariableBody>,
) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    let name = b.name.ok_or_else(|| ApiError(Error::validation("name is required")))?;
    s.db.set_variable("org", o.id(), &name, &b.value).await?;
    Ok(StatusCode::CREATED)
}

pub async fn org_variable(State(s): State<AppState>, auth: Auth, Path((org, name)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let o = org_admin(&s, &auth, &org).await?;
    Ok(Json(variable_json(&s.db.variable("org", o.id(), &name).await?)))
}

pub async fn patch_org_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, name)): Path<(String, String)>,
    Json(b): Json<VariableBody>,
) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    s.db.set_variable("org", o.id(), b.name.as_deref().unwrap_or(&name), &b.value).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_org_variable(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, name)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let o = org_admin(&s, &auth, &org).await?;
    s.db.delete_variable("org", o.id(), &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- check runs (derived from jobs) -----

fn check_run_json(s: &AppState, rw: &RepoWithOwner, run: &Run, j: &Job) -> Value {
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    let status = match j.status.as_str() {
        "in_progress" => "in_progress",
        "completed" => "completed",
        _ => "queued",
    };
    json!({
        "id": j.id,
        "node_id": shapes::node_id("CR", j.id),
        "head_sha": run.head_sha,
        "external_id": j.job_guid,
        "url": format!("{api}/check-runs/{}", j.id),
        "html_url": format!("{}/{}/actions/runs/{}/job/{}", s.config.site_url(), rw.full_name(), j.run_id, j.id),
        "details_url": format!("{}/{}/actions/runs/{}/job/{}", s.config.site_url(), rw.full_name(), j.run_id, j.id),
        "status": status,
        "conclusion": j.conclusion,
        "started_at": iso_opt(j.started_at),
        "completed_at": iso_opt(j.completed_at),
        "output": {"title": j.conclusion, "summary": Value::Null, "text": Value::Null, "annotations_count": 0, "annotations_url": format!("{api}/check-runs/{}/annotations", j.id)},
        "name": j.name,
        "check_suite": {"id": j.run_id},
        "app": {"id": 15368, "slug": "github-actions", "name": "GitHub Actions"},
        "pull_requests": [],
    })
}

pub async fn check_runs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, git_ref)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let path = s.store.path(rw.repo.id());
    let sha = reef_git::ops::rev_parse(s.store.git_binary(), &path, &git_ref).await.map_err(|_| ApiError(Error::NotFound))?;
    let jobs = s.db.jobs_for_sha(rw.repo.id(), &sha).await?;
    let mut runs: HashMap<i64, Run> = HashMap::new();
    let mut out = Vec::new();
    for j in &jobs {
        if !runs.contains_key(&j.run_id)
            && let Ok(r) = s.db.run_by_id(Id(j.run_id)).await
        {
            runs.insert(j.run_id, r);
        }
        if let Some(r) = runs.get(&j.run_id) {
            out.push(check_run_json(&s, &rw, r, j));
        }
    }
    Ok(Json(json!({"total_count": out.len(), "check_runs": out})))
}

pub async fn check_suites(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, git_ref)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let path = s.store.path(rw.repo.id());
    let sha = reef_git::ops::rev_parse(s.store.git_binary(), &path, &git_ref).await.map_err(|_| ApiError(Error::NotFound))?;
    let runs = s.db.runs_for_sha(rw.repo.id(), &sha).await?;
    let api = format!("{}/repos/{}", s.config.api_url(), rw.full_name());
    let suites: Vec<Value> = runs.iter().map(|r| json!({"id": r.id, "node_id": shapes::node_id("CS", r.id), "head_branch": r.head_branch, "head_sha": r.head_sha, "status": r.status, "conclusion": r.conclusion, "url": format!("{api}/check-suites/{}", r.id), "app": {"id": 15368, "slug": "github-actions", "name": "GitHub Actions"}, "created_at": iso(r.created_at), "updated_at": iso(r.updated_at)})).collect();
    Ok(Json(json!({"total_count": suites.len(), "check_suites": suites})))
}

pub async fn check_run(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let j = s.db.job_by_id(Id(id)).await?;
    if j.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let r = s.db.run_by_id(Id(j.run_id)).await?;
    Ok(Json(check_run_json(&s, &rw, &r, &j)))
}

pub async fn permissions(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    repo_admin(&s, &auth, &owner, &repo).await?;
    Ok(Json(json!({"enabled": true, "allowed_actions": "all"})))
}
