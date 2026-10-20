//! Site administration (Reef API, `/api/reef/admin/...`): overview counts, users,
//! organizations, repositories, runners, and settings that override the config file.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use reef_core::time::iso;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/overview", get(overview))
        .route("/admin/users", get(users))
        .route("/admin/users/{login}", axum::routing::patch(patch_user))
        .route("/admin/orgs", get(orgs))
        .route("/admin/orgs/{login}", axum::routing::patch(patch_org))
        .route("/admin/repos", get(repos))
        .route("/admin/runners", get(runners))
        .route("/admin/runs", get(runs))
        .route("/admin/settings", get(settings).put(put_settings))
}

/// Settings the console may change; everything else stays in the config file.
pub const EDITABLE: &[(&str, &str)] = &[
    ("instance_name", "Name shown in the header and page titles"),
    ("allow_signup", "Allow anyone to create an account (true/false)"),
    ("announcement", "Banner shown to every signed-in user (markdown, empty to hide)"),
    ("default_branch", "Default branch name for new repositories"),
    ("actions_upstream", "Where actions not hosted here are downloaded from"),
    ("actions_retention_days", "Days to keep workflow logs and artifacts"),
];

fn require_admin(auth: &Auth) -> ApiResult<&Account> {
    auth.require_admin()
}

/// A setting with the console override applied.
pub async fn effective(s: &AppState, key: &str) -> String {
    if let Ok(Some(v)) = s.db.setting(key).await {
        return v;
    }
    match key {
        "instance_name" => s.config.instance_name.clone(),
        "allow_signup" => s.config.allow_signup.to_string(),
        "default_branch" => "main".into(),
        "actions_upstream" => s.config.actions.upstream.clone(),
        // The file's `[actions] retention`, in whole days, as the console edits it.
        "actions_retention_days" => (s.config.actions.retention().as_secs() / 86_400).to_string(),
        _ => String::new(),
    }
}

pub async fn overview(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let users = s.db.count_accounts("user").await?;
    let orgs = s.db.count_accounts("org").await?;
    let repos = s.db.count_repos().await?;
    let counts = json!({
        "users": users,
        "orgs": orgs,
        "repos": repos,
        "issues": s.db.count_rows("issues").await?,
        "runs": s.db.count_rows("workflow_runs").await?,
        "runners": s.db.count_rows("runners").await?,
        "projects": s.db.count_rows("projects").await?,
        "releases": s.db.count_rows("releases").await?,
    });
    let recent_users: Vec<Value> = s.db.list_accounts("user", 0, 8).await?.iter().rev().map(|a| user_row(&s, a)).collect();
    let recent_repos: Vec<Value> = s.db.all_repos(0, 8).await?.iter().map(|rw| repo_row(&s, rw)).collect();
    let runners = s.db.count_rows("runners").await?;
    Ok(Json(json!({
        "counts": counts,
        "recent_users": recent_users,
        "recent_repos": recent_repos,
        "runners_total": runners,
        "version": crate::VERSION,
        "database": s.db.backend().name(),
        "data_dir": s.config.data_dir.display().to_string(),
        "site_url": s.config.site_url(),
        "oidc": match s.config.oidc() { Some(g) => json!({"issuer": g.issuer, "client_id": g.client_id, "name": crate::oauth::provider_name(&s), "only": true}), None => Value::Null },
    })))
}

fn user_row(s: &AppState, a: &Account) -> Value {
    let mut v = shapes::simple_user(&s.config, a);
    v["name"] = json!(a.name);
    v["email"] = json!(a.email);
    v["site_admin"] = json!(a.is_admin());
    v["suspended"] = json!(a.suspended != 0);
    v["created_at"] = json!(iso(a.created_at));
    v
}

fn repo_row(s: &AppState, rw: &reef_db::repos::RepoWithOwner) -> Value {
    let mut v = shapes::repo(&s.config, rw, None);
    v["size_kb"] = json!(rw.repo.size_kb);
    v
}

#[derive(serde::Deserialize, Default)]
pub struct ListQuery {
    q: Option<String>,
    page: Option<i64>,
    per_page: Option<i64>,
}

pub async fn users(State(s): State<AppState>, auth: Auth, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let mut all = s.db.list_accounts("user", 0, 10_000).await?;
    if let Some(needle) = q.q.as_deref().map(str::to_lowercase).filter(|x| !x.is_empty()) {
        all.retain(|a| {
            a.login.to_lowercase().contains(&needle)
                || a.email.as_deref().map(|e| e.to_lowercase().contains(&needle)).unwrap_or(false)
                || a.name.as_deref().map(|n| n.to_lowercase().contains(&needle)).unwrap_or(false)
        });
    }
    let total = all.len();
    let per = q.per_page.unwrap_or(50).clamp(1, 200) as usize;
    let page = q.page.unwrap_or(1).max(1) as usize;
    let mut rows = Vec::new();
    for a in all.iter().skip((page - 1) * per).take(per) {
        let mut v = user_row(&s, a);
        v["repos"] = json!(s.db.count_repos_for_owner(a.id(), true).await?);
        v["orgs"] = json!(s.db.user_orgs(a.id()).await?.iter().map(|o| o.login.clone()).collect::<Vec<_>>());
        rows.push(v);
    }
    Ok(Json(json!({"total": total, "users": rows})))
}

#[derive(serde::Deserialize, Default)]
pub struct UserPatch {
    site_admin: Option<bool>,
    suspended: Option<bool>,
    password: Option<String>,
    /// New login (rename).
    login: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub struct OrgPatch {
    login: Option<String>,
    name: Option<String>,
}

pub async fn patch_org(
    State(s): State<AppState>,
    auth: Auth,
    Path(login): Path<String>,
    Json(b): Json<OrgPatch>,
) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let o = s.db.account_by_login(&login).await?;
    if !o.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    if let Some(new) = b.login.as_deref().map(str::trim).filter(|l| !l.is_empty() && *l != o.login) {
        s.db.rename_account(o.id(), new).await?;
    }
    if let Some(n) = b.name {
        s.db.update_user(o.id(), reef_db::accounts::UserPatch { name: Some(Some(n)), ..Default::default() }).await?;
    }
    let o = s.db.account_by_id(o.id()).await?;
    let mut v = shapes::org_simple(&s.config, &o);
    v["name"] = json!(o.name);
    Ok(Json(v))
}

pub async fn patch_user(
    State(s): State<AppState>,
    auth: Auth,
    Path(login): Path<String>,
    Json(b): Json<UserPatch>,
) -> ApiResult<Json<Value>> {
    let me = require_admin(&auth)?.clone();
    let a = s.db.account_by_login(&login).await?;
    if !a.is_user() {
        return Err(ApiError(Error::NotFound));
    }
    if let Some(admin) = b.site_admin {
        if a.id == me.id && !admin {
            return Err(ApiError(Error::validation("you cannot remove your own administrator role")));
        }
        s.db.set_admin(a.id(), admin).await?;
    }
    if let Some(susp) = b.suspended {
        if a.id == me.id && susp {
            return Err(ApiError(Error::validation("you cannot suspend yourself")));
        }
        s.db.set_suspended(a.id(), susp).await?;
    }
    if let Some(pw) = b.password.filter(|p| !p.is_empty()) {
        s.db.set_password(a.id(), &pw).await?;
    }
    if let Some(new) = b.login.as_deref().map(str::trim).filter(|l| !l.is_empty() && *l != a.login) {
        s.db.rename_account(a.id(), new).await?;
    }
    Ok(Json(user_row(&s, &s.db.account_by_id(a.id()).await?)))
}

pub async fn orgs(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let mut rows = Vec::new();
    for o in s.db.list_accounts("org", 0, 10_000).await? {
        let members = s.db.org_members(o.id(), None).await?;
        let mut v = shapes::org_simple(&s.config, &o);
        v["name"] = json!(o.name);
        v["description"] = json!(o.description);
        v["created_at"] = json!(iso(o.created_at));
        v["members"] = json!(members.len());
        v["owners"] = json!(members.iter().filter(|(_, r)| r == "admin").map(|(a, _)| a.login.clone()).collect::<Vec<_>>());
        v["repos"] = json!(s.db.count_repos_for_owner(o.id(), true).await?);
        rows.push(v);
    }
    Ok(Json(json!({"total": rows.len(), "orgs": rows})))
}

pub async fn repos(State(s): State<AppState>, auth: Auth, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let per = q.per_page.unwrap_or(50).clamp(1, 200);
    let page = q.page.unwrap_or(1).max(1);
    let mut list = s.db.all_repos((page - 1) * per, per).await?;
    if let Some(needle) = q.q.as_deref().map(str::to_lowercase).filter(|x| !x.is_empty()) {
        list = s.db.all_repos(0, 10_000).await?.into_iter().filter(|rw| rw.full_name().to_lowercase().contains(&needle)).collect();
    }
    Ok(Json(json!({"total": s.db.count_repos().await?, "repos": list.iter().map(|rw| repo_row(&s, rw)).collect::<Vec<_>>()})))
}

pub async fn runners(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let mut rows = Vec::new();
    let now = reef_core::time::now_ms();
    for o in s.db.list_accounts("org", 0, 10_000).await? {
        for r in s.db.runners("org", o.id()).await? {
            rows.push(json!({"id": r.id, "name": r.name, "scope": o.login, "scope_kind": "org", "os": r.os, "version": r.version, "labels": r.labels(), "online": r.online(now), "busy": r.busy != 0, "ephemeral": r.ephemeral != 0, "last_seen_at": r.last_seen_at.map(iso)}));
        }
    }
    for rw in s.db.all_repos(0, 10_000).await? {
        for r in s.db.runners("repo", rw.repo.id()).await? {
            rows.push(json!({"id": r.id, "name": r.name, "scope": rw.full_name(), "scope_kind": "repo", "os": r.os, "version": r.version, "labels": r.labels(), "online": r.online(now), "busy": r.busy != 0, "ephemeral": r.ephemeral != 0, "last_seen_at": r.last_seen_at.map(iso)}));
        }
    }
    let queued = s.db.queued_jobs(200).await?.len();
    Ok(Json(json!({"runners": rows, "queued_jobs": queued})))
}

pub async fn runs(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let mut rows = Vec::new();
    let mut cache = HashMap::new();
    for rw in s.db.all_repos(0, 10_000).await? {
        let (list, _) = s.db.runs(rw.repo.id(), &Default::default(), 5, 0).await?;
        for r in &list {
            rows.push(crate::actions::rest::run_json(&s, &rw, r, &mut cache).await);
        }
    }
    rows.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
    rows.truncate(50);
    Ok(Json(json!({"runs": rows})))
}

pub async fn settings(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    require_admin(&auth)?;
    let stored: HashMap<String, String> = s.db.settings().await?.into_iter().collect();
    let mut out = Vec::new();
    for (k, desc) in EDITABLE {
        out.push(json!({"key": k, "description": desc, "value": effective(&s, k).await, "overridden": stored.contains_key(*k)}));
    }
    Ok(Json(json!({"settings": out})))
}

#[derive(serde::Deserialize)]
pub struct PutSettings {
    /// key -> value; null resets to the config file's value.
    values: HashMap<String, Option<String>>,
}

pub async fn put_settings(State(s): State<AppState>, auth: Auth, Json(b): Json<PutSettings>) -> ApiResult<Json<Value>> {
    let me = require_admin(&auth)?.clone();
    for (k, v) in &b.values {
        if !EDITABLE.iter().any(|(e, _)| e == k) {
            return Err(ApiError(Error::validation(format!("unknown setting {k}"))));
        }
        if k == "allow_signup"
            && let Some(v) = v
            && v != "true"
            && v != "false"
        {
            return Err(ApiError(Error::validation("allow_signup must be true or false")));
        }
        s.db.set_setting(k, v.as_deref().map(str::trim).filter(|x| !x.is_empty()), Some(me.id())).await?;
    }
    settings(State(s), auth).await
}

pub fn _unused(_: StatusCode, _: Id) {}
