//! GitHub REST: `/repos/{o}/{r}/hooks`, `/orgs/{org}/hooks`, deliveries and redelivery.

use super::{deliver_now, enqueue_one, payloads, ping_payload};
use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::rest::repos::load;
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use reef_core::time::iso;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::repos::RepoWithOwner;
use reef_db::webhooks::{Delivery, HookInput, Webhook};
use serde_json::{Value, json};

/// Event names offered by the UI and accepted by the API.
pub const EVENTS: &[&str] = &[
    "*",
    "push",
    "create",
    "delete",
    "fork",
    "gollum",
    "issue_comment",
    "issues",
    "pull_request",
    "pull_request_review",
    "pull_request_review_comment",
    "release",
    "repository",
    "star",
    "watch",
    "workflow_job",
    "workflow_run",
    "ping",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{owner}/{repo}/hooks", get(repo_list).post(repo_create))
        .route("/repos/{owner}/{repo}/hooks/{id}", get(repo_show).patch(repo_update).delete(repo_delete))
        .route("/repos/{owner}/{repo}/hooks/{id}/config", get(repo_config).patch(repo_update_config))
        .route("/repos/{owner}/{repo}/hooks/{id}/pings", post(repo_ping))
        .route("/repos/{owner}/{repo}/hooks/{id}/tests", post(repo_test))
        .route("/repos/{owner}/{repo}/hooks/{id}/deliveries", get(repo_deliveries))
        .route("/repos/{owner}/{repo}/hooks/{id}/deliveries/{delivery}", get(repo_delivery))
        .route("/repos/{owner}/{repo}/hooks/{id}/deliveries/{delivery}/attempts", post(repo_redeliver))
        .route("/orgs/{org}/hooks", get(org_list).post(org_create))
        .route("/orgs/{org}/hooks/{id}", get(org_show).patch(org_update).delete(org_delete))
        .route("/orgs/{org}/hooks/{id}/config", get(org_config).patch(org_update_config))
        .route("/orgs/{org}/hooks/{id}/pings", post(org_ping))
        .route("/orgs/{org}/hooks/{id}/deliveries", get(org_deliveries))
        .route("/orgs/{org}/hooks/{id}/deliveries/{delivery}", get(org_delivery))
        .route("/orgs/{org}/hooks/{id}/deliveries/{delivery}/attempts", post(org_redeliver))
}

/// Where a hook lives: the repository or organization, and the API prefix for its URLs.
enum Scope {
    Repo(RepoWithOwner),
    Org(Account),
}

impl Scope {
    fn kind(&self) -> &'static str {
        match self {
            Scope::Repo(_) => "repo",
            Scope::Org(_) => "org",
        }
    }
    fn owner_id(&self) -> Id {
        match self {
            Scope::Repo(rw) => rw.repo.id(),
            Scope::Org(a) => a.id(),
        }
    }
    fn api_prefix(&self, s: &AppState) -> String {
        match self {
            Scope::Repo(rw) => format!("{}/repos/{}/hooks", s.config.api_url(), rw.full_name()),
            Scope::Org(a) => format!("{}/orgs/{}/hooks", s.config.api_url(), a.login),
        }
    }
    fn type_name(&self) -> &'static str {
        match self {
            Scope::Repo(_) => "Repository",
            Scope::Org(_) => "Organization",
        }
    }
}

async fn repo_scope(s: &AppState, auth: &Auth, owner: &str, repo: &str) -> ApiResult<Scope> {
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin rights to Repository."));
    }
    Ok(Scope::Repo(rw))
}

async fn org_scope(s: &AppState, auth: &Auth, org: &str) -> ApiResult<Scope> {
    let u = auth.require()?;
    let a = s.db.account_by_login(org).await?;
    if !a.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    let owner = u.is_admin() || matches!(s.db.org_membership(a.id(), u.id()).await?, Some(m) if m.role == "admin");
    if !owner {
        return Err(crate::error::forbidden("Must be an organization owner."));
    }
    Ok(Scope::Org(a))
}

async fn hook_in(s: &AppState, scope: &Scope, id: i64) -> ApiResult<Webhook> {
    let h = s.db.hook_by_id(Id(id)).await?;
    if h.owner_kind != scope.kind() || h.owner_id != scope.owner_id().0 {
        return Err(ApiError(Error::NotFound));
    }
    Ok(h)
}

fn hook_json(s: &AppState, scope: &Scope, h: &Webhook) -> Value {
    let u = format!("{}/{}", scope.api_prefix(s), h.id);
    let (code, status, message) = match h.last_status {
        Some(c) if (200..300).contains(&c) => (json!(c), "active", "OK"),
        Some(c) => (json!(c), "misconfigured", "Last delivery was not successful"),
        None if h.last_delivered_at.is_some() => (Value::Null, "misconfigured", "Could not connect"),
        None => (Value::Null, "unused", ""),
    };
    json!({
        "type": scope.type_name(),
        "id": h.id,
        "name": "web",
        "active": h.active != 0,
        "events": h.events(),
        "config": config_json(h),
        "updated_at": iso(h.updated_at),
        "created_at": iso(h.created_at),
        "url": u,
        "test_url": format!("{u}/test"),
        "ping_url": format!("{u}/pings"),
        "deliveries_url": format!("{u}/deliveries"),
        "last_response": {"code": code, "status": status, "message": message},
    })
}

fn config_json(h: &Webhook) -> Value {
    let mut c = json!({"url": h.url, "content_type": h.content_type, "insecure_ssl": if h.insecure_ssl != 0 { "1" } else { "0" }});
    if h.secret.as_deref().map(|x| !x.is_empty()).unwrap_or(false) {
        c["secret"] = json!("********");
    }
    c
}

fn delivery_json(s: &AppState, scope: &Scope, d: &Delivery, full: bool) -> Value {
    let mut v = json!({
        "id": d.id,
        "guid": d.guid,
        "delivered_at": d.delivered_at.map(iso).unwrap_or_else(|| iso(d.created_at)),
        "redelivery": d.redelivery_of.is_some(),
        "duration": d.duration_ms.map(|m| m as f64 / 1000.0),
        "status": match d.status.as_str() { "delivered" => "OK".to_string(), "pending" => if d.attempt == 0 { "Pending".into() } else { "Retrying".into() }, _ => d.response_body.as_deref().filter(|b| b.starts_with("delivery error")).map(|_| "Failed to connect".to_string()).unwrap_or_else(|| "Failed".into()) },
        "status_code": d.response_status,
        "event": d.event,
        "action": d.action,
        "installation_id": Value::Null,
        "repository_id": d.repo_id,
        "throttled_at": Value::Null,
        "url": format!("{}/{}/deliveries/{}", scope.api_prefix(s), d.hook_id, d.id),
        "attempt": d.attempt,
    });
    if full {
        let req_headers: Value = serde_json::from_str(&d.request_headers).unwrap_or(json!({}));
        let payload: Value = serde_json::from_str(&d.payload).unwrap_or(Value::Null);
        let res_headers: Value = d.response_headers.as_deref().and_then(|h| serde_json::from_str(h).ok()).unwrap_or(json!({}));
        v["request"] = json!({"headers": req_headers, "payload": payload});
        v["response"] = json!({"headers": res_headers, "payload": d.response_body});
    }
    v
}

#[derive(serde::Deserialize, Default)]
pub struct HookBody {
    #[allow(dead_code)]
    name: Option<String>,
    active: Option<bool>,
    events: Option<Vec<String>>,
    add_events: Option<Vec<String>>,
    remove_events: Option<Vec<String>>,
    config: Option<ConfigBody>,
}

#[derive(serde::Deserialize, Default)]
pub struct ConfigBody {
    url: Option<String>,
    content_type: Option<String>,
    secret: Option<String>,
    insecure_ssl: Option<Value>,
}

fn insecure(v: &Option<Value>) -> Option<bool> {
    match v {
        Some(Value::String(s)) => Some(s == "1" || s == "true"),
        Some(Value::Number(n)) => Some(n.as_i64() == Some(1)),
        Some(Value::Bool(b)) => Some(*b),
        _ => None,
    }
}

fn validate_events(events: &[String]) -> ApiResult<()> {
    for e in events {
        if !EVENTS.contains(&e.as_str()) {
            return Err(ApiError(Error::validation(format!("Unknown event: {e}"))));
        }
    }
    Ok(())
}

fn validate_url(url: &str) -> ApiResult<()> {
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() < 10 {
        return Err(ApiError(Error::validation("config.url must be an http(s) URL")));
    }
    Ok(())
}

/// The URL must also point at a public address (no loopback, link-local, private ranges),
/// unless `[network] allow_private_targets` is on.
async fn validate_target(s: &AppState, url: &str) -> ApiResult<()> {
    crate::outbound::check_url(s, url).await.map_err(|r| ApiError(Error::validation(format!("config.url: {r}"))))
}

fn input_from(existing: Option<&Webhook>, b: &HookBody) -> ApiResult<HookInput> {
    let cfg = b.config.as_ref();
    let mut input = match existing {
        Some(h) => HookInput {
            url: h.url.clone(),
            content_type: h.content_type.clone(),
            secret: h.secret.clone(),
            events: h.events(),
            active: h.active != 0,
            insecure_ssl: h.insecure_ssl != 0,
        },
        None => HookInput {
            url: String::new(),
            content_type: "json".into(),
            secret: None,
            events: vec!["push".into()],
            active: true,
            insecure_ssl: false,
        },
    };
    if let Some(c) = cfg {
        if let Some(u) = &c.url {
            input.url = u.trim().to_string();
        }
        if let Some(ct) = &c.content_type {
            input.content_type = if ct == "form" { "form".into() } else { "json".into() };
        }
        if let Some(sec) = &c.secret {
            // "********" means "keep"; empty clears.
            if sec != "********" {
                input.secret = if sec.is_empty() { None } else { Some(sec.clone()) };
            }
        }
        if let Some(i) = insecure(&c.insecure_ssl) {
            input.insecure_ssl = i;
        }
    }
    if let Some(a) = b.active {
        input.active = a;
    }
    if let Some(e) = &b.events {
        input.events = e.clone();
    }
    if let Some(add) = &b.add_events {
        for e in add {
            if !input.events.contains(e) {
                input.events.push(e.clone());
            }
        }
    }
    if let Some(rm) = &b.remove_events {
        input.events.retain(|e| !rm.contains(e));
    }
    if input.events.is_empty() {
        input.events = vec!["push".into()];
    }
    validate_url(&input.url)?;
    validate_events(&input.events)?;
    Ok(input)
}

fn ping_docs(s: &AppState, scope: &Scope, hook: &Webhook, actor: &Account) -> Value {
    let hj = hook_json(s, scope, hook);
    let (repo, org) = match scope {
        Scope::Repo(rw) => {
            (Some(shapes::repo(&s.config, rw, None)), if rw.owner.is_org() { Some(shapes::org_simple(&s.config, &rw.owner)) } else { None })
        }
        Scope::Org(a) => (None, Some(shapes::org_simple(&s.config, a))),
    };
    ping_payload(s, hook, hj, repo, org, shapes::simple_user(&s.config, actor))
}

async fn queue_ping(s: &AppState, scope: &Scope, hook: &Webhook, actor: &Account) -> ApiResult<Delivery> {
    let body = ping_docs(s, scope, hook, actor);
    let repo = match scope {
        Scope::Repo(rw) => Some(rw.repo.id()),
        Scope::Org(_) => None,
    };
    let d = enqueue_one(s, hook, "ping", None, &body, repo, None).await?;
    deliver_now(s, hook, d.id()).await;
    Ok(s.db.delivery_by_id(d.id()).await?)
}

// ----- shared handlers -----

async fn list(s: &AppState, scope: Scope, page: Page) -> ApiResult<axum::response::Response> {
    let hooks = s.db.hooks_for(scope.kind(), scope.owner_id()).await?;
    let all: Vec<Value> = hooks.iter().map(|h| hook_json(s, &scope, h)).collect();
    let (items, h) = page.slice(&all, &scope.api_prefix(s));
    Ok((h, Json(json!(items))).into_response())
}

async fn create(s: &AppState, scope: Scope, actor: &Account, b: HookBody) -> ApiResult<axum::response::Response> {
    let input = input_from(None, &b)?;
    validate_target(s, &input.url).await?;
    let h = s.db.create_hook(scope.kind(), scope.owner_id(), &input).await?;
    // GitHub pings a hook on creation.
    let _ = queue_ping(s, &scope, &h, actor).await;
    let h = s.db.hook_by_id(h.id()).await?;
    Ok((StatusCode::CREATED, Json(hook_json(s, &scope, &h))).into_response())
}

async fn update(s: &AppState, scope: Scope, id: i64, b: HookBody) -> ApiResult<Json<Value>> {
    let h = hook_in(s, &scope, id).await?;
    let input = input_from(Some(&h), &b)?;
    validate_target(s, &input.url).await?;
    let h = s.db.update_hook(h.id(), &input).await?;
    Ok(Json(hook_json(s, &scope, &h)))
}

async fn update_config(s: &AppState, scope: Scope, id: i64, c: ConfigBody) -> ApiResult<Json<Value>> {
    let h = hook_in(s, &scope, id).await?;
    let input = input_from(Some(&h), &HookBody { config: Some(c), ..Default::default() })?;
    validate_target(s, &input.url).await?;
    let h = s.db.update_hook(h.id(), &input).await?;
    Ok(Json(config_json(&h)))
}

#[derive(serde::Deserialize, Default)]
pub struct DeliveriesQuery {
    per_page: Option<i64>,
}

async fn deliveries(s: &AppState, scope: Scope, id: i64, q: DeliveriesQuery) -> ApiResult<Json<Value>> {
    let h = hook_in(s, &scope, id).await?;
    let list = s.db.deliveries_for_hook(h.id(), q.per_page.unwrap_or(30).clamp(1, 100)).await?;
    Ok(Json(json!(list.iter().map(|d| delivery_json(s, &scope, d, false)).collect::<Vec<_>>())))
}

async fn delivery(s: &AppState, scope: Scope, id: i64, delivery: i64) -> ApiResult<Json<Value>> {
    let h = hook_in(s, &scope, id).await?;
    let d = s.db.delivery_by_id(Id(delivery)).await?;
    if d.hook_id != h.id {
        return Err(ApiError(Error::NotFound));
    }
    Ok(Json(delivery_json(s, &scope, &d, true)))
}

async fn redeliver(s: &AppState, scope: Scope, id: i64, delivery: i64) -> ApiResult<axum::response::Response> {
    let h = hook_in(s, &scope, id).await?;
    let d = s.db.delivery_by_id(Id(delivery)).await?;
    if d.hook_id != h.id {
        return Err(ApiError(Error::NotFound));
    }
    let payload: Value = serde_json::from_str(&d.payload).unwrap_or(Value::Null);
    let nd = enqueue_one(s, &h, &d.event, d.action.as_deref(), &payload, d.repo_id.map(Id), Some(d.id())).await?;
    deliver_now(s, &h, nd.id()).await;
    let nd = s.db.delivery_by_id(nd.id()).await?;
    Ok((StatusCode::ACCEPTED, Json(delivery_json(s, &scope, &nd, true))).into_response())
}

// ----- repository hooks -----

pub async fn repo_list(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
) -> ApiResult<axum::response::Response> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    list(&s, scope, page).await
}

pub async fn repo_create(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<HookBody>,
) -> ApiResult<axum::response::Response> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    let actor = auth.require()?.clone();
    create(&s, scope, &actor, b).await
}

pub async fn repo_show(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    let h = hook_in(&s, &scope, id).await?;
    Ok(Json(hook_json(&s, &scope, &h)))
}

pub async fn repo_update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Json(b): Json<HookBody>,
) -> ApiResult<Json<Value>> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    update(&s, scope, id, b).await
}

pub async fn repo_delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    let h = hook_in(&s, &scope, id).await?;
    s.db.delete_hook(h.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn repo_config(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    let h = hook_in(&s, &scope, id).await?;
    Ok(Json(config_json(&h)))
}

pub async fn repo_update_config(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Json(c): Json<ConfigBody>,
) -> ApiResult<Json<Value>> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    update_config(&s, scope, id, c).await
}

pub async fn repo_ping(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    let actor = auth.require()?.clone();
    let h = hook_in(&s, &scope, id).await?;
    queue_ping(&s, &scope, &h, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GitHub's "test" sends a `push` for the latest commit on the default branch.
pub async fn repo_test(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    let actor = auth.require()?.clone();
    let h = hook_in(&s, &scope, id).await?;
    let Scope::Repo(rw) = &scope else { return Err(ApiError(Error::NotFound)) };
    if rw.repo.is_empty != 0 {
        return Err(ApiError(Error::validation("The repository has no commits to test with.")));
    }
    let branch = rw.repo.default_branch.clone();
    let head = crate::rest::contents::with_repo(&s, rw, move |r| r.resolve(&branch)).await?;
    let parent = crate::rest::contents::with_repo(&s, rw, {
        let head = head.clone();
        move |r| r.commit(&head).map(|c| c.parents.first().cloned())
    })
    .await?;
    let r = format!("refs/heads/{}", rw.repo.default_branch);
    let before = parent.unwrap_or_else(|| "0000000000000000000000000000000000000000".into());
    let Some(push) = payloads::push_payload(&s, rw, Some(&actor), &r, &before, &head).await else { return Err(ApiError(Error::NotFound)) };
    let mut body = payloads::base(&s, rw, Some(&actor));
    if let (Some(b), Some(p)) = (body.as_object_mut(), push.as_object()) {
        for (k, v) in p {
            b.insert(k.clone(), v.clone());
        }
    }
    let d = enqueue_one(&s, &h, "push", None, &body, Some(rw.repo.id()), None).await?;
    deliver_now(&s, &h, d.id()).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn repo_deliveries(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Query(q): Query<DeliveriesQuery>,
) -> ApiResult<Json<Value>> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    deliveries(&s, scope, id, q).await
}

pub async fn repo_delivery(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id, delivery_id)): Path<(String, String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    delivery(&s, scope, id, delivery_id).await
}

pub async fn repo_redeliver(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id, delivery_id)): Path<(String, String, i64, i64)>,
) -> ApiResult<axum::response::Response> {
    let scope = repo_scope(&s, &auth, &owner, &repo).await?;
    redeliver(&s, scope, id, delivery_id).await
}

// ----- organization hooks -----

pub async fn org_list(State(s): State<AppState>, auth: Auth, Path(org): Path<String>, page: Page) -> ApiResult<axum::response::Response> {
    let scope = org_scope(&s, &auth, &org).await?;
    list(&s, scope, page).await
}

pub async fn org_create(
    State(s): State<AppState>,
    auth: Auth,
    Path(org): Path<String>,
    Json(b): Json<HookBody>,
) -> ApiResult<axum::response::Response> {
    let scope = org_scope(&s, &auth, &org).await?;
    let actor = auth.require()?.clone();
    create(&s, scope, &actor, b).await
}

pub async fn org_show(State(s): State<AppState>, auth: Auth, Path((org, id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let scope = org_scope(&s, &auth, &org).await?;
    let h = hook_in(&s, &scope, id).await?;
    Ok(Json(hook_json(&s, &scope, &h)))
}

pub async fn org_update(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, id)): Path<(String, i64)>,
    Json(b): Json<HookBody>,
) -> ApiResult<Json<Value>> {
    let scope = org_scope(&s, &auth, &org).await?;
    update(&s, scope, id, b).await
}

pub async fn org_delete(State(s): State<AppState>, auth: Auth, Path((org, id)): Path<(String, i64)>) -> ApiResult<StatusCode> {
    let scope = org_scope(&s, &auth, &org).await?;
    let h = hook_in(&s, &scope, id).await?;
    s.db.delete_hook(h.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn org_config(State(s): State<AppState>, auth: Auth, Path((org, id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let scope = org_scope(&s, &auth, &org).await?;
    let h = hook_in(&s, &scope, id).await?;
    Ok(Json(config_json(&h)))
}

pub async fn org_update_config(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, id)): Path<(String, i64)>,
    Json(c): Json<ConfigBody>,
) -> ApiResult<Json<Value>> {
    let scope = org_scope(&s, &auth, &org).await?;
    update_config(&s, scope, id, c).await
}

pub async fn org_ping(State(s): State<AppState>, auth: Auth, Path((org, id)): Path<(String, i64)>) -> ApiResult<StatusCode> {
    let scope = org_scope(&s, &auth, &org).await?;
    let actor = auth.require()?.clone();
    let h = hook_in(&s, &scope, id).await?;
    queue_ping(&s, &scope, &h, &actor).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn org_deliveries(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, id)): Path<(String, i64)>,
    Query(q): Query<DeliveriesQuery>,
) -> ApiResult<Json<Value>> {
    let scope = org_scope(&s, &auth, &org).await?;
    deliveries(&s, scope, id, q).await
}

pub async fn org_delivery(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, id, delivery_id)): Path<(String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    let scope = org_scope(&s, &auth, &org).await?;
    delivery(&s, scope, id, delivery_id).await
}

pub async fn org_redeliver(
    State(s): State<AppState>,
    auth: Auth,
    Path((org, id, delivery_id)): Path<(String, i64, i64)>,
) -> ApiResult<axum::response::Response> {
    let scope = org_scope(&s, &auth, &org).await?;
    redeliver(&s, scope, id, delivery_id).await
}
