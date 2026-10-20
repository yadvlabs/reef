//! The server side of the stock `actions/runner` protocol (docs/02-runner-protocol.md):
//! registration, the VSS-style `_apis` under a tenant URL, session/message long polling, job
//! callbacks (timeline, logs, feed, events) and action download resolution.

use super::crypto;
use super::jwt::{self, Principal};
use super::logs::Files;
use super::schedule;
use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reef_core::time::iso;
use reef_core::{Error, Id};
use reef_db::actions::{Job, Runner};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::time::Duration;

const TOKEN_TTL_SECS: i64 = 24 * 3600;
const JOB_TOKEN_TTL_MS: i64 = 24 * 3_600_000;
const LOCK_MS: i64 = 10 * 60_000;
const LONG_POLL: Duration = Duration::from_secs(45);
const POLL_STEP: Duration = Duration::from_secs(2);

pub fn router() -> Router<AppState> {
    let apis = Router::new()
        .route("/_apis/connectionData", get(connection_data))
        .route("/_apis/", axum::routing::options(options_root))
        .route("/_apis", axum::routing::options(options_root))
        .route("/_apis/v1/auth", post(token_endpoint))
        .route("/_apis/v1/auth/", post(token_endpoint))
        .route("/_apis/distributedtask/pools", get(pools))
        .route("/_apis/distributedtask/pools/{pool}/agents", get(list_agents).post(create_agent))
        .route("/_apis/distributedtask/pools/{pool}/agents/{agent}", get(get_agent).put(replace_agent).delete(delete_agent))
        .route("/_apis/distributedtask/pools/{pool}/agents/{agent}/updates", axum::routing::put(agent_updates))
        .route("/_apis/distributedtask/pools/{pool}/sessions", post(create_session))
        .route("/_apis/distributedtask/pools/{pool}/sessions/{session}", axum::routing::delete(delete_session))
        .route("/_apis/distributedtask/pools/{pool}/messages", get(get_message))
        .route("/_apis/distributedtask/pools/{pool}/messages/{message}", axum::routing::delete(delete_message))
        .route("/_apis/distributedtask/pools/{pool}/jobrequests/{request}", get(get_request).patch(patch_request))
        .route("/_apis/distributedtask/packages/agent", get(packages))
        .route("/_apis/distributedtask/packages/agent/{platform}", get(packages))
        .route("/_apis/distributedtask/packages/agent/{platform}/{version}", get(packages))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/timelines", post(create_timeline))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/timelines/{timeline}", get(get_timeline))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/timelines/{timeline}/records", axum::routing::patch(patch_records))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/timelines/{timeline}/records/{record}/feed", post(feed))
        .route(
            "/_apis/distributedtask/hubs/{hub}/plans/{plan}/timelines/{timeline}/records/{record}/attachments/{kind}/{name}",
            axum::routing::put(attachment),
        )
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/logs", post(create_log))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/logs/{log}", post(append_log))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/events", post(events))
        .route("/_apis/distributedtask/hubs/{hub}/plans/{plan}/actiondownloadinfo", post(action_download_info));
    Router::new().nest("/_actions/{kind}/{scope}", apis).route("/_actions/proxy/{kind}/{owner}/{repo}/{*git_ref}", get(proxy_download))
}

// ----- tenant, auth, errors -----

#[derive(Clone)]
pub struct Tenant {
    pub kind: String,
    pub id: i64,
}

impl Tenant {
    fn parse(kind: &str, scope: &str) -> Result<Tenant, Response> {
        let id: i64 = scope.parse().map_err(|_| vss_error(StatusCode::NOT_FOUND, "TaskAgentPoolNotFoundException", "unknown tenant"))?;
        if kind != "repo" && kind != "org" {
            return Err(vss_error(StatusCode::NOT_FOUND, "TaskAgentPoolNotFoundException", "unknown tenant"));
        }
        Ok(Tenant { kind: kind.to_string(), id })
    }
}

pub fn tenant_url(s: &AppState, kind: &str, id: i64) -> String {
    format!("{}/_actions/{kind}/{id}", s.config.site_url())
}

/// The VSS error body the runner parses (`typeKey` drives its retry behaviour).
pub fn vss_error(status: StatusCode, type_key: &str, message: &str) -> Response {
    let body = json!({"$id": "1", "innerException": Value::Null, "message": message, "typeName": format!("GitHub.DistributedTask.WebApi.{type_key}, GitHub.DistributedTask.WebApi"), "typeKey": type_key, "errorCode": 0, "eventId": 3000});
    (status, Json(body)).into_response()
}

fn challenge() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer authorization_uri=/_apis/v1/auth")],
        Json(json!({"message": "authentication required"})),
    )
        .into_response()
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") { Some(rest.trim().to_string()) } else { None }
}

fn principal(s: &AppState, headers: &HeaderMap) -> Result<Principal, Response> {
    let Some(t) = bearer(headers) else { return Err(challenge()) };
    jwt::verify(&s.actions.keys.jwt, &t).map_err(|_| challenge())
}

/// Registration tokens and runner tokens may manage agents of their scope.
async fn scope_principal(s: &AppState, headers: &HeaderMap, t: &Tenant) -> Result<Principal, Response> {
    let p = principal(s, headers)?;
    match &p {
        Principal::Registration { scope_kind, scope_id } if *scope_kind == t.kind && *scope_id == t.id => Ok(p),
        Principal::Runner { runner_id } => {
            let r =
                s.db.runner_by_id(Id(*runner_id))
                    .await
                    .map_err(|_| vss_error(StatusCode::NOT_FOUND, "TaskAgentNotFoundException", "runner no longer exists"))?;
            if r.scope_kind == t.kind && r.scope_id == t.id { Ok(p) } else { Err(challenge()) }
        }
        Principal::Job { .. } => Ok(p),
        _ => Err(challenge()),
    }
}

async fn runner_principal(s: &AppState, headers: &HeaderMap, t: &Tenant) -> Result<Runner, Response> {
    match principal(s, headers)? {
        Principal::Runner { runner_id } => {
            let r =
                s.db.runner_by_id(Id(runner_id))
                    .await
                    .map_err(|_| vss_error(StatusCode::NOT_FOUND, "TaskAgentNotFoundException", "runner no longer exists"))?;
            if r.scope_kind == t.kind && r.scope_id == t.id { Ok(r) } else { Err(challenge()) }
        }
        _ => Err(challenge()),
    }
}

async fn job_principal(s: &AppState, headers: &HeaderMap, plan: &str) -> Result<Job, Response> {
    match principal(s, headers)? {
        Principal::Job { job_id } => {
            let j =
                s.db.job_by_id(Id(job_id))
                    .await
                    .map_err(|_| vss_error(StatusCode::NOT_FOUND, "TaskOrchestrationPlanNotFoundException", "job no longer exists"))?;
            if j.plan_id == plan {
                Ok(j)
            } else {
                Err(vss_error(StatusCode::NOT_FOUND, "TaskOrchestrationPlanNotFoundException", "plan mismatch"))
            }
        }
        _ => Err(challenge()),
    }
}

fn wrap(items: Vec<Value>) -> Value {
    json!({"count": items.len(), "value": items})
}

// ----- registration (GitHub API side) -----

#[derive(serde::Deserialize)]
pub struct Registration {
    url: String,
    runner_event: Option<String>,
}

/// `POST /api/v3/actions/runner-registration` with `Authorization: RemoteAuth <token>`.
pub async fn registration(State(s): State<AppState>, headers: HeaderMap, Json(b): Json<Registration>) -> ApiResult<Json<Value>> {
    let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
    let token =
        auth.strip_prefix("RemoteAuth ").map(str::trim).ok_or_else(|| ApiError(Error::Unauthorized("RemoteAuth token required".into())))?;
    let t = s.db.runner_token(token).await.map_err(|_| ApiError(Error::Unauthorized("invalid or expired registration token".into())))?;
    let expected = if b.runner_event.as_deref() == Some("remove") { "remove" } else { "register" };
    if t.purpose != expected {
        return Err(ApiError(Error::Unauthorized(format!("this token is for {}, not {expected}", t.purpose))));
    }
    // The URL the runner was configured with must name the same scope as the token.
    let path = b.url.split("://").nth(1).and_then(|r| r.split_once('/')).map(|(_, p)| p.trim_matches('/').to_string()).unwrap_or_default();
    let parts: Vec<&str> = path.split('/').collect();
    let matches = match (t.scope_kind.as_str(), parts.as_slice()) {
        ("repo", [o, r]) => s.db.repo_by_path(o, r).await.map(|rw| rw.repo.id == t.scope_id).unwrap_or(false),
        ("org", [o]) => s.db.account_by_login(o).await.map(|a| a.id == t.scope_id).unwrap_or(false),
        _ => false,
    };
    if !matches {
        return Err(ApiError(Error::Unauthorized("the URL does not match the scope of this registration token".into())));
    }
    let jwt = jwt::mint(
        &s.actions.keys.jwt,
        &Principal::Registration { scope_kind: t.scope_kind.clone(), scope_id: t.scope_id },
        3600,
        json!({}),
    );
    Ok(Json(
        json!({"url": tenant_url(&s, &t.scope_kind, t.scope_id), "token": jwt, "token_schema": "OAuthAccessToken", "use_v2_flow": false}),
    ))
}

// ----- connection data & locations -----

const LOCATIONS: &[(&str, &str, &str)] = &[
    ("a8c47e17-4d56-4a56-92bb-de7ea7dc65be", "pools", "_apis/distributedtask/pools/{poolId}"),
    ("e298ef32-5878-4cab-993c-043836571f42", "agents", "_apis/distributedtask/pools/{poolId}/agents/{agentId}"),
    ("134e239e-2df3-4794-a6f6-24f1f19ec8dc", "sessions", "_apis/distributedtask/pools/{poolId}/sessions/{sessionId}"),
    ("c3a054f6-7a8a-49c0-944e-3a8e5d7adfd7", "messages", "_apis/distributedtask/pools/{poolId}/messages/{messageId}"),
    ("fc825784-c92a-4299-9221-998a02d1b54f", "jobrequests", "_apis/distributedtask/pools/{poolId}/jobrequests/{requestId}"),
    ("8ffcd551-079c-493a-9c02-54346299d144", "packages", "_apis/distributedtask/packages/{packageType}/{platform}/{version}"),
    ("8cc1b02b-ae49-4516-b5ad-4f9b29967c30", "updates", "_apis/distributedtask/pools/{poolId}/agents/{agentId}/updates"),
    ("13b5d709-74aa-470b-a8e9-bf9f3ded3f18", "config", "_apis/distributedtask/runners/{agentId}/config/{configType}"),
    ("83597576-cc2c-453c-bea6-2882ae6a1653", "timelines", "_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}"),
    (
        "8893bc5b-35b2-4be7-83cb-99e683551db4",
        "records",
        "_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records",
    ),
    (
        "858983e4-19bd-4c5e-864c-507b59b58b12",
        "feed",
        "_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records/{recordId}/feed",
    ),
    ("46f5667d-263a-4684-91b1-dff7fdcf64e2", "logs", "_apis/distributedtask/hubs/{hubName}/plans/{planId}/logs/{logId}"),
    (
        "7898f959-9cdf-4096-b29e-7f293031629e",
        "attachments",
        "_apis/distributedtask/hubs/{hubName}/plans/{planId}/timelines/{timelineId}/records/{recordId}/attachments/{type}/{name}",
    ),
    ("557624af-b29e-4c20-8ab0-0399d2204f3f", "events", "_apis/distributedtask/hubs/{hubName}/plans/{planId}/events"),
    (
        "27d7f831-88c1-4719-8ca1-6a061dad90eb",
        "actiondownloadinfo",
        "_apis/distributedtask/hubs/{hubName}/plans/{planId}/actiondownloadinfo",
    ),
];

fn instance_id(s: &AppState) -> String {
    let h = reef_core::token::hash(&format!("reef-instance:{}", hex::encode(s.actions.keys.jwt)));
    format!("{}-{}-4{}-8{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

async fn connection_data(State(s): State<AppState>, headers: HeaderMap, Path((kind, scope)): Path<(String, String)>) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    let defs: Vec<Value> = LOCATIONS
        .iter()
        .map(|(id, name, path)| {
            json!({
                "serviceType": "distributedtask", "identifier": id, "displayName": name, "relativePath": path,
                "relativeToSetting": "context", "description": "Resource Location", "serviceOwner": "00000000-0000-0000-0000-000000000000",
                "status": "active", "toolId": "Framework", "locationMappings": [], "resourceVersion": 1, "minVersion": "1.0", "maxVersion": "12.0", "releasedVersion": "1.0"
            })
        })
        .collect();
    let iid = instance_id(&s);
    Json(json!({
        "instanceId": iid,
        "deploymentId": iid,
        "deploymentType": "onPremises",
        "locationServiceData": {
            "serviceOwner": "00000000-0000-0000-0000-000000000000",
            "defaultAccessMappingMoniker": "PublicAccessMapping",
            "lastChangeId": 1,
            "lastChangeId64": 1,
            "clientCacheFresh": false,
            "clientCacheTimeToLive": 3600,
            "accessMappings": [{"moniker": "PublicAccessMapping", "displayName": "Public", "accessPoint": tenant_url(&s, &t.kind, t.id), "virtualDirectory": "", "serviceOwner": "00000000-0000-0000-0000-000000000000"}],
            "serviceDefinitions": defs
        },
        "webApplicationRelativeDirectory": ""
    }))
    .into_response()
}

async fn options_root(State(s): State<AppState>, headers: HeaderMap, Path((kind, scope)): Path<(String, String)>) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    let locs: Vec<Value> = LOCATIONS.iter().map(|(id, name, path)| json!({"id": id, "area": "distributedtask", "resourceName": name, "routeTemplate": path, "resourceVersion": 1, "minVersion": "1.0", "maxVersion": "12.0", "releasedVersion": "1.0"})).collect();
    Json(wrap(locs)).into_response()
}

// ----- OAuth token endpoint -----

#[derive(serde::Deserialize)]
pub struct TokenForm {
    grant_type: Option<String>,
    client_assertion_type: Option<String>,
    client_assertion: Option<String>,
}

async fn token_endpoint(
    State(s): State<AppState>,
    Path((kind, scope)): Path<(String, String)>,
    axum::extract::Form(f): axum::extract::Form<TokenForm>,
) -> Response {
    let oauth_err =
        |code: &str, desc: String| (StatusCode::BAD_REQUEST, Json(json!({"error": code, "error_description": desc}))).into_response();
    if f.grant_type.as_deref() != Some("client_credentials")
        || f.client_assertion_type.as_deref() != Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
    {
        return oauth_err("unsupported_grant_type", "expected client_credentials with a jwt-bearer assertion".into());
    }
    let Some(assertion) = f.client_assertion else { return oauth_err("invalid_request", "client_assertion is required".into()) };
    let Some((_, claims)) = jwt::peek(&assertion) else { return oauth_err("invalid_client", "malformed assertion".into()) };
    let Some(client_id) = claims.get("iss").and_then(|v| v.as_str()) else {
        return oauth_err("invalid_client", "assertion has no issuer".into());
    };
    let Ok(runner) = s.db.runner_by_client_id(client_id).await else { return oauth_err("invalid_client", "unknown client".into()) };
    if runner.scope_kind != kind || runner.scope_id.to_string() != scope {
        return oauth_err("invalid_client", "client belongs to another tenant".into());
    }
    let key: Value = serde_json::from_str(&runner.public_key).unwrap_or(Value::Null);
    let (e, n) = (key.get("exponent").and_then(|v| v.as_str()).unwrap_or(""), key.get("modulus").and_then(|v| v.as_str()).unwrap_or(""));
    let aud = format!("{}/_apis/v1/auth", tenant_url(&s, &kind, runner.scope_id));
    if let Err(err) = jwt::verify_client_assertion(&assertion, e, n, &aud) {
        return oauth_err("invalid_client", err.to_string());
    }
    let _ = s.db.touch_runner(runner.id(), None).await;
    let token = jwt::mint(&s.actions.keys.jwt, &Principal::Runner { runner_id: runner.id }, TOKEN_TTL_SECS, json!({}));
    Json(json!({"access_token": token, "token_type": "Bearer", "expires_in": TOKEN_TTL_SECS, "scope": "runner"})).into_response()
}

// ----- pools & agents -----

async fn pools(State(s): State<AppState>, headers: HeaderMap, Path((kind, scope)): Path<(String, String)>) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    Json(wrap(vec![json!({"id": 1, "name": "Default", "isHosted": false, "isInternal": true, "poolType": "automation", "size": 0, "scope": "00000000-0000-0000-0000-000000000000", "createdOn": "2026-01-01T00:00:00Z"})])).into_response()
}

fn agent_json(s: &AppState, r: &Runner) -> Value {
    let key: Value = serde_json::from_str(&r.public_key).unwrap_or(Value::Null);
    let labels: Vec<Value> = r.labels().iter().map(|l| json!({"id": 0, "name": l, "type": if matches!(l.as_str(), "self-hosted" | "Linux" | "Windows" | "macOS" | "X64" | "ARM64" | "ARM") { "system" } else { "user" }})).collect();
    let now = reef_core::time::now_ms();
    json!({
        "id": r.id,
        "name": r.name,
        "version": r.version,
        "osDescription": r.os,
        "enabled": true,
        "ephemeral": r.ephemeral != 0,
        "disableUpdate": r.disable_update != 0,
        "status": if r.busy != 0 { "busy" } else if r.online(now) { "online" } else { "offline" },
        "provisioningState": "Provisioned",
        "maxParallelism": 1,
        "createdOn": iso(r.created_at),
        "authorization": {"clientId": r.client_id, "authorizationUrl": format!("{}/_apis/v1/auth", tenant_url(s, &r.scope_kind, r.scope_id)), "publicKey": key},
        "labels": labels,
        "properties": {"RequireFipsCryptography": false}
    })
}

#[derive(serde::Deserialize, Default)]
pub struct AgentQuery {
    #[serde(rename = "agentName")]
    agent_name: Option<String>,
}

async fn list_agents(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool)): Path<(String, String, String)>,
    Query(q): Query<AgentQuery>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    let all = s.db.runners(&t.kind, Id(t.id)).await.unwrap_or_default();
    let items: Vec<Value> = all
        .iter()
        .filter(|r| q.agent_name.as_ref().map(|n| n.eq_ignore_ascii_case(&r.name)).unwrap_or(true))
        .map(|r| agent_json(&s, r))
        .collect();
    Json(wrap(items)).into_response()
}

fn labels_of(body: &Value) -> Vec<String> {
    let mut out: Vec<String> = body
        .get("labels")
        .and_then(|l| l.as_array())
        .map(|a| a.iter().filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(String::from)).collect())
        .unwrap_or_default();
    if out.is_empty() {
        out.push("self-hosted".into());
    }
    out
}

async fn create_agent(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    match scope_principal(&s, &headers, &t).await {
        Ok(Principal::Registration { .. }) => {}
        Ok(_) => return vss_error(StatusCode::FORBIDDEN, "AccessDeniedException", "a registration token is required to add runners"),
        Err(r) => return r,
    }
    let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    if let Ok(Some(existing)) = s.db.runner_by_name(&t.kind, Id(t.id), &name).await {
        // Same name without --replace: GitHub reports the conflict; runner.server replaces.
        // We replace when the caller re-registers, which keeps ephemeral fleets simple.
        return replace_inner(&s, existing, &body).await;
    }
    let key = body.pointer("/authorization/publicKey").cloned().unwrap_or(Value::Null);
    if key.get("modulus").is_none() {
        return vss_error(StatusCode::BAD_REQUEST, "InvalidOperationException", "authorization.publicKey is required");
    }
    let client_id = reef_db::actions::uuid_v4();
    match s
        .db
        .create_runner(
            &t.kind,
            Id(t.id),
            &name,
            body.get("osDescription").and_then(|v| v.as_str()),
            body.get("version").and_then(|v| v.as_str()),
            &labels_of(&body),
            body.get("ephemeral").and_then(|v| v.as_bool()).unwrap_or(false),
            body.get("disableUpdate").and_then(|v| v.as_bool()).unwrap_or(false),
            &client_id,
            &key.to_string(),
            &body.to_string(),
        )
        .await
    {
        Ok(r) => Json(agent_json(&s, &r)).into_response(),
        Err(e) => vss_error(StatusCode::BAD_REQUEST, "InvalidOperationException", &e.to_string()),
    }
}

async fn replace_inner(s: &AppState, existing: Runner, body: &Value) -> Response {
    let key = body
        .pointer("/authorization/publicKey")
        .cloned()
        .unwrap_or_else(|| serde_json::from_str(&existing.public_key).unwrap_or(Value::Null));
    match s
        .db
        .replace_runner(
            existing.id(),
            body.get("osDescription").and_then(|v| v.as_str()).or(existing.os.as_deref()),
            body.get("version").and_then(|v| v.as_str()).or(existing.version.as_deref()),
            &labels_of(body),
            body.get("ephemeral").and_then(|v| v.as_bool()).unwrap_or(existing.ephemeral != 0),
            body.get("disableUpdate").and_then(|v| v.as_bool()).unwrap_or(existing.disable_update != 0),
            &key.to_string(),
            &body.to_string(),
        )
        .await
    {
        Ok(r) => Json(agent_json(s, &r)).into_response(),
        Err(e) => vss_error(StatusCode::BAD_REQUEST, "InvalidOperationException", &e.to_string()),
    }
}

async fn replace_agent(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, agent)): Path<(String, String, String, i64)>,
    Json(body): Json<Value>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    match s.db.runner_by_id(Id(agent)).await {
        Ok(r) if r.scope_kind == t.kind && r.scope_id == t.id => replace_inner(&s, r, &body).await,
        _ => vss_error(StatusCode::NOT_FOUND, "TaskAgentNotFoundException", "runner not found"),
    }
}

async fn get_agent(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, agent)): Path<(String, String, String, i64)>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    match s.db.runner_by_id(Id(agent)).await {
        Ok(r) if r.scope_kind == t.kind && r.scope_id == t.id => Json(agent_json(&s, &r)).into_response(),
        _ => vss_error(StatusCode::NOT_FOUND, "TaskAgentNotFoundException", "runner not found"),
    }
}

async fn delete_agent(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, agent)): Path<(String, String, String, i64)>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    match s.db.runner_by_id(Id(agent)).await {
        Ok(r) if r.scope_kind == t.kind && r.scope_id == t.id => {
            for j in s.db.jobs_of_runner(r.id()).await.unwrap_or_default() {
                let _ = s.db.requeue_job(j.id()).await;
            }
            let _ = s.db.delete_runner(r.id()).await;
            StatusCode::NO_CONTENT.into_response()
        }
        _ => vss_error(StatusCode::NOT_FOUND, "TaskAgentNotFoundException", "runner not found"),
    }
}

async fn agent_updates(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, agent)): Path<(String, String, String, i64)>,
) -> Response {
    get_agent(State(s), headers, Path((kind, scope, _pool, agent))).await
}

async fn packages(State(_s): State<AppState>) -> Response {
    Json(wrap(vec![])).into_response()
}

// ----- sessions & messages -----

async fn create_session(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let runner = match runner_principal(&s, &headers, &t).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let key = crypto::new_key();
    let pk: Value = serde_json::from_str(&runner.public_key).unwrap_or(Value::Null);
    let wrapped = match crypto::wrap_key(
        &key,
        pk.get("exponent").and_then(|v| v.as_str()).unwrap_or(""),
        pk.get("modulus").and_then(|v| v.as_str()).unwrap_or(""),
    ) {
        Ok(w) => w,
        Err(e) => return vss_error(StatusCode::BAD_REQUEST, "InvalidOperationException", &e.to_string()),
    };
    let sid = reef_db::actions::uuid_v4();
    let owner = body.get("ownerName").and_then(|v| v.as_str());
    if let Err(e) =
        s.db.create_session_for_runner(&sid, runner.id(), &base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key), owner)
            .await
    {
        return vss_error(StatusCode::INTERNAL_SERVER_ERROR, "InvalidOperationException", &e.to_string());
    }
    let _ = s.db.touch_runner(runner.id(), Some(false)).await;
    Json(json!({
        "sessionId": sid,
        "ownerName": owner,
        "agent": {"id": runner.id, "name": runner.name, "version": runner.version, "osDescription": runner.os, "status": 0},
        "useFipsEncryption": true,
        "encryptionKey": {"encrypted": true, "value": wrapped}
    }))
    .into_response()
}

async fn delete_session(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, session)): Path<(String, String, String, String)>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let runner = match runner_principal(&s, &headers, &t).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let _ = s.db.delete_runner_session(&session).await;
    if runner.ephemeral != 0 && s.db.jobs_of_runner(runner.id()).await.map(|j| j.is_empty()).unwrap_or(true) {
        // An ephemeral runner leaving after its job is gone for good.
        let done = s.db.runner_by_id(runner.id()).await.map(|r| r.busy == 0).unwrap_or(true);
        if done {
            let _ = s.db.delete_runner(runner.id()).await;
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(serde::Deserialize, Default)]
pub struct MessageQuery {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    status: Option<String>,
}

/// Long-poll for work: a cancellation for a job this runner holds, or the oldest queued job
/// whose labels the runner satisfies.
async fn get_message(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool)): Path<(String, String, String)>,
    Query(q): Query<MessageQuery>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    let runner = match runner_principal(&s, &headers, &t).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    let Some(sid) = q.session_id else { return vss_error(StatusCode::BAD_REQUEST, "InvalidOperationException", "sessionId is required") };
    let session = match s.db.runner_session(&sid).await {
        Ok(sess) if sess.runner_id == runner.id => sess,
        _ => return vss_error(StatusCode::FORBIDDEN, "TaskAgentSessionExpiredException", "session expired"),
    };
    let key: Vec<u8> = match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &session.encryption_key) {
        Ok(k) if k.len() == 32 => k,
        _ => return vss_error(StatusCode::FORBIDDEN, "TaskAgentSessionExpiredException", "session key invalid"),
    };
    let key: [u8; 32] = key.try_into().unwrap();
    let busy = q.status.as_deref().map(|st| st.eq_ignore_ascii_case("busy")).unwrap_or(false);
    let deadline = tokio::time::Instant::now() + LONG_POLL;
    loop {
        let _ = s.db.touch_session(&sid).await;
        let _ = s.db.touch_runner(runner.id(), Some(busy)).await;
        // 1. Cancellations for jobs this runner is running.
        for j in s.db.jobs_of_runner(runner.id()).await.unwrap_or_default() {
            if j.cancel_requested == 1 {
                let _ = s.db.mark_cancel_sent(j.id()).await;
                let body = json!({"jobId": j.job_guid, "timeout": "00:05:00"}).to_string();
                let (iv, ct) = crypto::encrypt_body(&key, body.as_bytes());
                return Json(
                    json!({"messageId": reef_core::id::next().0 & 0x7fff_ffff, "messageType": "JobCancellation", "iv": iv, "body": ct}),
                )
                .into_response();
            }
        }
        // 2. New work, unless the runner is busy (one job at a time) or ephemeral and used.
        if !busy && (runner.ephemeral == 0 || !s.db.jobs_of_runner(runner.id()).await.map(|j| !j.is_empty()).unwrap_or(false)) {
            match next_job(&s, &runner, &t).await {
                Ok(Some(msg)) => {
                    let body = msg.to_string();
                    let (iv, ct) = crypto::encrypt_body(&key, body.as_bytes());
                    let mid = msg.get("requestId").and_then(|r| r.as_i64()).unwrap_or(0);
                    return Json(json!({"messageId": mid, "messageType": "PipelineAgentJobRequest", "iv": iv, "body": ct})).into_response();
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "dispatching job failed"),
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return StatusCode::NO_CONTENT.into_response();
        }
        tokio::time::sleep(POLL_STEP).await;
    }
}

/// Find, claim and build the next job for a runner.
async fn next_job(s: &AppState, runner: &Runner, t: &Tenant) -> reef_core::Result<Option<Value>> {
    let labels: Vec<String> = runner.labels().iter().map(|l| l.to_lowercase()).collect();
    let mut owners: HashMap<i64, reef_db::repos::RepoWithOwner> = HashMap::new();
    for job in s.db.queued_jobs(200).await? {
        let rw = match owners.get(&job.repo_id) {
            Some(rw) => rw.clone(),
            None => {
                let Ok(rw) = s.db.repo_with_owner_by_id(Id(job.repo_id)).await else { continue };
                owners.insert(job.repo_id, rw.clone());
                rw
            }
        };
        let in_scope = match t.kind.as_str() {
            "repo" => rw.repo.id == t.id,
            _ => rw.owner.id == t.id,
        };
        if !in_scope {
            continue;
        }
        let wanted: Vec<String> = job.labels().iter().map(|l| l.to_lowercase()).collect();
        if wanted.is_empty() || !wanted.iter().all(|w| labels.contains(w)) {
            continue;
        }
        let request_id = reef_core::id::next().0 & 0x7fff_ffff_ffff;
        let msg = build_job_message(s, &rw, &job, runner, t, request_id).await?;
        if s.db.assign_job(job.id(), runner, request_id, &reef_db::actions::uuid_v4(), LOCK_MS, &msg.to_string()).await? {
            let _ = s.db.touch_runner(runner.id(), Some(true)).await;
            schedule::settle_run(s, Id(job.run_id)).await?;
            return Ok(Some(msg));
        }
    }
    Ok(None)
}

async fn build_job_message(
    s: &AppState,
    rw: &reef_db::repos::RepoWithOwner,
    job: &Job,
    runner: &Runner,
    t: &Tenant,
    request_id: i64,
) -> reef_core::Result<Value> {
    let run = s.db.run_by_id(Id(job.run_id)).await?;
    let wf = reef_actions::workflow::parse(&run.workflow_yaml).map_err(|e| Error::validation(e.to_string()))?;
    let def = wf.jobs.get(&job.job_key).ok_or_else(|| Error::validation(format!("job '{}' vanished from the workflow", job.job_key)))?;
    let actor = match run.actor_id {
        Some(a) => s.db.account_by_id(Id(a)).await.ok(),
        None => None,
    };
    // needs context from completed dependencies
    let siblings = s.db.jobs_of_run(run.id(), Some(run.run_attempt)).await?;
    let mut needs = Map::new();
    for k in job.needs() {
        let legs: Vec<&Job> = siblings.iter().filter(|j| j.job_key == k).collect();
        let result = if legs.iter().any(|j| matches!(j.conclusion.as_deref(), Some("failure") | Some("timed_out"))) {
            "failure"
        } else if legs.iter().any(|j| j.conclusion.as_deref() == Some("cancelled")) {
            "cancelled"
        } else if !legs.is_empty() && legs.iter().all(|j| j.conclusion.as_deref() == Some("skipped")) {
            "skipped"
        } else {
            "success"
        };
        let mut outputs = Map::new();
        for l in legs {
            if let Value::Object(o) = l.outputs_json() {
                outputs.extend(o);
            }
        }
        needs.insert(k, json!({"result": result, "outputs": outputs}));
    }
    // vars and secrets: org first, repo overrides
    let mut vars = Map::new();
    let mut secrets = Map::new();
    if rw.owner.is_org() {
        for v in s.db.variables("org", rw.owner.id()).await? {
            vars.insert(v.name, Value::String(v.value));
        }
        for sec in s.db.secrets("org", rw.owner.id()).await? {
            if let Ok(v) = s.actions.keys.open(&sec.value_enc) {
                secrets.insert(sec.name, Value::String(v));
            }
        }
    }
    for v in s.db.variables("repo", rw.repo.id()).await? {
        vars.insert(v.name, Value::String(v.value));
    }
    for sec in s.db.secrets("repo", rw.repo.id()).await? {
        if let Ok(v) = s.actions.keys.open(&sec.value_enc) {
            secrets.insert(sec.name, Value::String(v));
        }
    }
    let github_token = s.db.create_job_token(job.id(), rw.repo.id(), r#"{"contents":"write","issues":"write","pull_requests":"write","packages":"write","actions":"write","checks":"write","statuses":"write"}"#, JOB_TOKEN_TTL_MS).await?;
    let access_token = jwt::mint(
        &s.actions.keys.jwt,
        &Principal::Job { job_id: job.id },
        JOB_TOKEN_TTL_MS / 1000,
        json!({"orch_id": format!("{}.{}", job.plan_id, job.job_key)}),
    );
    let tenant = tenant_url(s, &t.kind, t.id);
    super::message::build(
        s,
        super::message::MessageInput {
            rw,
            run: &run,
            job,
            wf: &wf,
            def,
            actor: actor.as_ref(),
            tenant_url: &tenant,
            access_token: &access_token,
            github_token: &github_token,
            request_id,
            needs: Value::Object(needs),
            vars,
            secrets,
            runner_group: "Default",
            cache_url: None,
        },
    )
    .map(|mut m| {
        let _ = runner;
        // The runner renews with this request id; keep it on the message too.
        m["requestId"] = json!(request_id);
        m
    })
}

async fn delete_message(State(_s): State<AppState>) -> Response {
    StatusCode::NO_CONTENT.into_response()
}

// ----- job requests -----

fn request_json(j: &Job) -> Value {
    json!({
        "requestId": j.request_id,
        "lockedUntil": j.locked_until.map(iso),
        "planId": j.plan_id,
        "jobId": j.job_guid,
        "result": match (j.status.as_str(), j.conclusion.as_deref()) {
            ("completed", Some("success")) => json!("succeeded"),
            ("completed", Some("cancelled")) => json!("canceled"),
            ("completed", Some("skipped")) => json!("skipped"),
            ("completed", _) => json!("failed"),
            _ => Value::Null,
        },
        "reservedAgent": j.runner_id.map(|id| json!({"id": id, "name": j.runner_name})),
        "receiveTime": j.started_at.map(iso),
        "finishTime": j.completed_at.map(iso),
    })
}

async fn get_request(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, request)): Path<(String, String, String, i64)>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    match s.db.job_by_request(request).await {
        Ok(j) => Json(request_json(&j)).into_response(),
        Err(_) => vss_error(StatusCode::NOT_FOUND, "TaskAgentJobNotFoundException", "job request not found"),
    }
}

/// Renew (every 60 s) or, for plan versions below 8, finish.
async fn patch_request(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((kind, scope, _pool, request)): Path<(String, String, String, i64)>,
    body: Option<Json<Value>>,
) -> Response {
    let t = match Tenant::parse(&kind, &scope) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(r) = scope_principal(&s, &headers, &t).await {
        return r;
    }
    let Ok(job) = s.db.job_by_request(request).await else {
        return vss_error(StatusCode::NOT_FOUND, "TaskAgentJobNotFoundException", "job request not found");
    };
    if job.status == "completed" {
        return Json(request_json(&job)).into_response();
    }
    let b = body.map(|b| b.0).unwrap_or(Value::Null);
    if let Some(result) = b.get("result").and_then(|r| r.as_str()) {
        let conclusion = match result {
            "succeeded" | "succeededWithIssues" => "success",
            "canceled" => "cancelled",
            "skipped" => "skipped",
            _ => "failure",
        };
        let _ = schedule::complete_job(&s, &job, conclusion, None).await;
    } else {
        let _ = s.db.renew_job_lock(job.id(), LOCK_MS).await;
    }
    match s.db.job_by_id(job.id()).await {
        Ok(j) => Json(request_json(&j)).into_response(),
        Err(_) => vss_error(StatusCode::NOT_FOUND, "TaskAgentJobNotFoundException", "job request not found"),
    }
}

// ----- timeline, logs, feed, events -----

async fn create_timeline(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan)): Path<(String, String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(r) = job_principal(&s, &headers, &plan).await {
        return r;
    }
    let mut v = body;
    if v.get("id").is_none() {
        v["id"] = json!(reef_db::actions::uuid_v4());
    }
    v["records"] = json!([]);
    Json(v).into_response()
}

async fn get_timeline(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan, timeline)): Path<(String, String, String, String, String)>,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    Json(json!({"id": timeline, "changeId": 1, "location": Value::Null, "records": job.timeline_json()})).into_response()
}

/// Merge partial record updates by id into the job's timeline document.
async fn patch_records(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan, _timeline)): Path<(String, String, String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    let incoming: Vec<Value> = body.get("value").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut records = job.timeline_json();
    for rec in incoming {
        let Some(id) = rec.get("id").and_then(|i| i.as_str()).map(String::from) else { continue };
        match records.iter_mut().find(|r| r.get("id").and_then(|i| i.as_str()) == Some(id.as_str())) {
            Some(existing) => {
                if let (Value::Object(e), Value::Object(n)) = (existing, &rec) {
                    for (k, v) in n {
                        if !v.is_null() {
                            e.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            None => records.push(rec),
        }
    }
    records.sort_by(|a, b| {
        let oa = a.get("order").and_then(|o| o.as_i64()).unwrap_or(i64::MAX);
        let ob = b.get("order").and_then(|o| o.as_i64()).unwrap_or(i64::MAX);
        oa.cmp(&ob)
    });
    let _ = s.db.set_job_timeline(job.id(), &Value::Array(records.clone()).to_string()).await;
    let _ = s.db.renew_job_lock(job.id(), LOCK_MS).await;
    Json(wrap(records)).into_response()
}

async fn feed(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan, _timeline, _record)): Path<(String, String, String, String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    let step = body.get("stepId").and_then(|v| v.as_str()).unwrap_or("job").to_string();
    let lines: Vec<String> = body
        .get("value")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if !lines.is_empty() {
        let mut text = lines.join("\n");
        text.push('\n');
        let rel = format!("logs/{}/{}/live-{}.log", job.run_id, job.id, step);
        let _ = s.actions.files.append(&rel, text.as_bytes()).await;
    }
    StatusCode::OK.into_response()
}

async fn attachment(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan, timeline, record, kind, name)): Path<(String, String, String, String, String, String, String, String)>,
    body: Bytes,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    let safe =
        |x: &str| x.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect::<String>();
    let rel = format!("attachments/{}/{}/{}/{}", job.run_id, job.id, safe(&kind), safe(&name));
    let _ = s.actions.files.write(&rel, &body).await;
    Json(json!({"type": kind, "name": name, "timelineId": timeline, "recordId": record, "createdOn": iso(reef_core::time::now_ms())}))
        .into_response()
}

async fn create_log(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan)): Path<(String, String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    let path = body.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
    // Reserve the id first so the file name can use it.
    let log = match s.db.create_job_log(job.id(), "").await {
        Ok(l) => l,
        Err(e) => return vss_error(StatusCode::INTERNAL_SERVER_ERROR, "InvalidOperationException", &e.to_string()),
    };
    let rel = Files::log_rel(job.run_id, job.id, log.id);
    let _ = sqlx_set_log_path(&s, log.id, &rel).await;
    Json(json!({"id": log.id, "location": Value::Null, "path": path, "lineCount": 0, "indexLocation": Value::Null, "createdOn": iso(log.created_at), "lastChangedOn": iso(log.created_at)})).into_response()
}

async fn sqlx_set_log_path(s: &AppState, id: i64, rel: &str) -> reef_core::Result<()> {
    s.db.set_log_path(id, rel).await
}

async fn append_log(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan, log)): Path<(String, String, String, String, i64)>,
    body: Bytes,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    let Ok(l) = s.db.job_log(log).await else { return vss_error(StatusCode::NOT_FOUND, "InvalidOperationException", "log not found") };
    if l.job_id != job.id {
        return vss_error(StatusCode::NOT_FOUND, "InvalidOperationException", "log not found");
    }
    let lines = body.iter().filter(|b| **b == b'\n').count() as i64;
    let _ = s.actions.files.append(&l.path, &body).await;
    let _ = s.db.add_log_lines(log, lines).await;
    Json(json!({"id": log, "location": Value::Null, "path": l.path, "lineCount": l.line_count + lines})).into_response()
}

/// `JobCompleted`: the authoritative end of a job.
async fn events(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan)): Path<(String, String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    if body.get("name").and_then(|n| n.as_str()) != Some("JobCompleted") {
        return StatusCode::OK.into_response();
    }
    let conclusion = match body.get("result").and_then(|r| r.as_str()).unwrap_or("failed") {
        "succeeded" | "succeededWithIssues" => "success",
        "canceled" => "cancelled",
        "skipped" => "skipped",
        _ => "failure",
    };
    let mut outputs = Map::new();
    if let Some(Value::Object(o)) = body.get("outputs") {
        for (k, v) in o {
            outputs.insert(k.clone(), v.get("value").cloned().unwrap_or(Value::Null));
        }
    }
    if let Err(e) = schedule::complete_job(&s, &job, conclusion, Some(&Value::Object(outputs))).await {
        tracing::warn!(error = %e, job = job.id, "completing job failed");
    }
    StatusCode::OK.into_response()
}

// ----- action downloads -----

async fn action_download_info(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((_k, _sc, _hub, plan)): Path<(String, String, String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let job = match job_principal(&s, &headers, &plan).await {
        Ok(j) => j,
        Err(r) => return r,
    };
    let token = match s.db.create_job_token(job.id(), Id(job.repo_id), r#"{"contents":"read"}"#, JOB_TOKEN_TTL_MS).await {
        Ok(t) => t,
        Err(e) => return vss_error(StatusCode::INTERNAL_SERVER_ERROR, "InvalidOperationException", &e.to_string()),
    };
    let mut out = Map::new();
    for a in body.get("actions").and_then(|a| a.as_array()).cloned().unwrap_or_default() {
        let nwo = a.get("nameWithOwner").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let git_ref = a.get("ref").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let key = format!("{nwo}@{git_ref}");
        let (owner, name) = nwo.split_once('/').unwrap_or((&nwo, ""));
        let mut info = json!({"nameWithOwner": nwo, "ref": git_ref, "resolvedNameWithOwner": nwo, "resolvedSha": git_ref, "authentication": {"token": token}, "packageDetails": Value::Null});
        match s.db.repo_by_path(owner, name).await {
            Ok(rw) => {
                let path = s.store.path(rw.repo.id());
                let sha = reef_git::ops::rev_parse(s.store.git_binary(), &path, &format!("refs/tags/{git_ref}"))
                    .await
                    .or(reef_git::ops::rev_parse(s.store.git_binary(), &path, &format!("refs/heads/{git_ref}")).await)
                    .or(reef_git::ops::rev_parse(s.store.git_binary(), &path, &git_ref).await);
                match sha {
                    Ok(sha) => {
                        info["resolvedSha"] = json!(sha);
                        info["tarballUrl"] = json!(format!("{}/repos/{}/tarball/{sha}", s.config.api_url(), rw.full_name()));
                        info["zipballUrl"] = json!(format!("{}/repos/{}/zipball/{sha}", s.config.api_url(), rw.full_name()));
                    }
                    Err(_) => {
                        return vss_error(
                            StatusCode::BAD_REQUEST,
                            "UnresolvableActionDownloadInfoException",
                            &format!("{key}: ref not found on this Reef"),
                        );
                    }
                }
            }
            Err(_) => {
                let enc = |x: &str| x.split('/').map(crate::shapes::urlpath).collect::<Vec<_>>().join("/");
                info["tarballUrl"] =
                    json!(format!("{}/_actions/proxy/tarball/{}/{}/{}", s.config.site_url(), enc(owner), enc(name), enc(&git_ref)));
                info["zipballUrl"] =
                    json!(format!("{}/_actions/proxy/zipball/{}/{}/{}", s.config.site_url(), enc(owner), enc(name), enc(&git_ref)));
            }
        }
        out.insert(key, info);
    }
    Json(json!({"actions": out})).into_response()
}

/// Stream an action archive from the upstream host (github.com by default) for runners; the
/// runner authenticates with its job token.
async fn proxy_download(
    State(s): State<AppState>,
    auth: Auth,
    Path((kind, owner, repo, git_ref)): Path<(String, String, String, String)>,
) -> Response {
    if auth.user.as_ref().and_then(|u| u.job_repo).is_none() {
        return (StatusCode::UNAUTHORIZED, "job token required").into_response();
    }
    let format = if kind == "zipball" { "zip" } else { "tar.gz" };
    let url = format!("{}/{owner}/{repo}/{format}/{git_ref}", s.config.actions.upstream.trim_end_matches('/'));
    let client = match reqwest::Client::builder().user_agent("reef-actions").build() {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let ct =
                resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
            let stream = resp.bytes_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct)
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Ok(resp) => (StatusCode::BAD_GATEWAY, format!("upstream returned {} for {url}", resp.status())).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("upstream request failed: {e}")).into_response(),
    }
}
