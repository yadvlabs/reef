//! Branch protection: GitHub's `/repos/{o}/{r}/branches/{b}/protection` family, plus a
//! Reef listing of all rules (GitHub exposes rules per branch only).

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::repos::load;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use reef_core::Error;
use reef_db::protection::{Protection, ProtectionInput};
use reef_db::repos::RepoWithOwner;
use serde_json::{Value, json};

pub fn protection_json(s: &AppState, rw: &RepoWithOwner, branch: &str, p: &Protection) -> Value {
    let u = format!("{}/repos/{}/branches/{}/protection", s.config.api_url(), rw.full_name(), branch);
    let b = |i: i64| i != 0;
    json!({
        "url": u,
        "pattern": p.pattern,
        "required_status_checks": {
            "url": format!("{u}/required_status_checks"),
            "strict": b(p.strict_status_checks),
            "contexts": p.checks(),
            "checks": p.checks().iter().map(|c| json!({"context": c, "app_id": Value::Null})).collect::<Vec<_>>(),
            "contexts_url": format!("{u}/required_status_checks/contexts"),
        },
        "required_pull_request_reviews": if b(p.require_pull_request) { json!({
            "url": format!("{u}/required_pull_request_reviews"),
            "dismiss_stale_reviews": b(p.dismiss_stale_reviews),
            "require_code_owner_reviews": b(p.require_code_owner_reviews),
            "required_approving_review_count": p.required_approving_review_count,
            "require_last_push_approval": false,
        }) } else { Value::Null },
        "enforce_admins": {"url": format!("{u}/enforce_admins"), "enabled": b(p.enforce_admins)},
        "required_linear_history": {"enabled": b(p.require_linear_history)},
        "allow_force_pushes": {"enabled": b(p.allow_force_pushes)},
        "allow_deletions": {"enabled": b(p.allow_deletions)},
        "block_creations": {"enabled": false},
        "required_conversation_resolution": {"enabled": false},
        "lock_branch": {"enabled": b(p.lock_branch)},
        "allow_fork_syncing": {"enabled": false},
        "required_signatures": {"url": format!("{u}/required_signatures"), "enabled": b(p.require_signed_commits)},
    })
}

async fn admin_repo(s: &AppState, auth: &Auth, owner: &str, repo: &str) -> ApiResult<RepoWithOwner> {
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    Ok(rw)
}

/// GitHub: `GET .../branches/{branch}/protection`. The branch name is matched against rules;
/// an unprotected branch is a 404 with GitHub's message.
pub async fn get(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, branch)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let rw = admin_repo(&s, &auth, &owner, &repo).await?;
    let rules = s.db.protections_for_branch(rw.repo.id(), &branch).await?;
    match rules.first() {
        Some(p) => Ok(Json(protection_json(&s, &rw, &branch, p))),
        None => Err(ApiError(Error::Validation { message: "Branch not protected".into(), errors: vec![] })),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct PutProtection {
    pattern: Option<String>,
    required_status_checks: Option<PutChecks>,
    enforce_admins: Option<bool>,
    required_pull_request_reviews: Option<PutReviews>,
    #[serde(default)]
    required_linear_history: bool,
    #[serde(default)]
    allow_force_pushes: Option<bool>,
    #[serde(default)]
    allow_deletions: bool,
    #[serde(default)]
    lock_branch: bool,
    #[serde(default)]
    required_signatures: bool,
}

#[derive(serde::Deserialize, Default)]
pub struct PutChecks {
    #[serde(default)]
    strict: bool,
    #[serde(default)]
    contexts: Vec<String>,
    #[serde(default)]
    checks: Vec<PutCheck>,
}

#[derive(serde::Deserialize)]
pub struct PutCheck {
    context: String,
}

#[derive(serde::Deserialize, Default)]
pub struct PutReviews {
    #[serde(default)]
    dismiss_stale_reviews: bool,
    #[serde(default)]
    require_code_owner_reviews: bool,
    #[serde(default)]
    required_approving_review_count: i64,
}

fn to_input(pattern: String, b: PutProtection) -> ProtectionInput {
    let mut checks: Vec<String> = Vec::new();
    let mut strict = false;
    if let Some(c) = b.required_status_checks {
        strict = c.strict;
        checks.extend(c.contexts);
        checks.extend(c.checks.into_iter().map(|c| c.context));
        checks.sort();
        checks.dedup();
    }
    let reviews = b.required_pull_request_reviews;
    ProtectionInput {
        pattern,
        allow_force_pushes: b.allow_force_pushes.unwrap_or(false),
        allow_deletions: b.allow_deletions,
        require_pull_request: reviews.is_some(),
        required_approving_review_count: reviews.as_ref().map(|r| r.required_approving_review_count).unwrap_or(0),
        dismiss_stale_reviews: reviews.as_ref().map(|r| r.dismiss_stale_reviews).unwrap_or(false),
        require_code_owner_reviews: reviews.as_ref().map(|r| r.require_code_owner_reviews).unwrap_or(false),
        enforce_admins: b.enforce_admins.unwrap_or(false),
        required_status_checks: checks,
        strict_status_checks: strict,
        require_linear_history: b.required_linear_history,
        require_signed_commits: b.required_signatures,
        lock_branch: b.lock_branch,
    }
}

/// GitHub: `PUT .../branches/{branch}/protection`. The branch name becomes the pattern.
pub async fn put(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, branch)): Path<(String, String, String)>,
    Json(b): Json<PutProtection>,
) -> ApiResult<Json<Value>> {
    let rw = admin_repo(&s, &auth, &owner, &repo).await?;
    let pattern = b.pattern.clone().unwrap_or_else(|| branch.clone());
    let p = s.db.upsert_protection(rw.repo.id(), to_input(pattern, b)).await?;
    Ok(Json(protection_json(&s, &rw, &branch, &p)))
}

pub async fn delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, branch)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let rw = admin_repo(&s, &auth, &owner, &repo).await?;
    // Delete the rule with this exact pattern, or the most specific rule matching the branch.
    let rules = s.db.protections_for_branch(rw.repo.id(), &branch).await?;
    let pattern =
        rules.iter().find(|r| r.pattern == branch).or(rules.first()).map(|r| r.pattern.clone()).ok_or(ApiError(Error::NotFound))?;
    s.db.delete_protection(rw.repo.id(), &pattern).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reef: `GET .../protection-rules` lists every rule of the repository.
pub async fn list(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let rw = admin_repo(&s, &auth, &owner, &repo).await?;
    let rules = s.db.protections(rw.repo.id()).await?;
    Ok(Json(json!(rules.iter().map(|p| protection_json(&s, &rw, &p.pattern, p)).collect::<Vec<_>>())))
}

/// Reef: `PUT .../protection-rules/{pattern}`; `DELETE` likewise. Patterns may contain `/`.
pub async fn put_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, pattern)): Path<(String, String, String)>,
    Json(b): Json<PutProtection>,
) -> ApiResult<Json<Value>> {
    let rw = admin_repo(&s, &auth, &owner, &repo).await?;
    let p = s.db.upsert_protection(rw.repo.id(), to_input(pattern.clone(), b)).await?;
    Ok(Json(protection_json(&s, &rw, &pattern, &p)))
}

pub async fn delete_rule(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, pattern)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let rw = admin_repo(&s, &auth, &owner, &repo).await?;
    s.db.delete_protection(rw.repo.id(), &pattern).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Split `name/protection` off a branch catch-all.
fn split_branch(branch: &str) -> (String, bool) {
    match branch.strip_suffix("/protection") {
        Some(b) => (b.to_string(), true),
        None => (branch.trim_end_matches('/').to_string(), false),
    }
}

pub async fn branch_get(
    state: State<AppState>,
    auth: Auth,
    Path((owner, repo, branch)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (name, prot) = split_branch(&branch);
    if prot {
        get(state, auth, Path((owner, repo, name))).await
    } else {
        crate::rest::contents::branch(state, auth, Path((owner, repo, name))).await
    }
}

pub async fn branch_put(
    state: State<AppState>,
    auth: Auth,
    Path((owner, repo, branch)): Path<(String, String, String)>,
    body: Json<PutProtection>,
) -> ApiResult<Json<Value>> {
    let (name, prot) = split_branch(&branch);
    if prot { put(state, auth, Path((owner, repo, name)), body).await } else { Err(ApiError(Error::NotFound)) }
}

pub async fn branch_delete(
    state: State<AppState>,
    auth: Auth,
    Path((owner, repo, branch)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let (name, prot) = split_branch(&branch);
    if prot { delete(state, auth, Path((owner, repo, name))).await } else { Err(ApiError(Error::NotFound)) }
}
