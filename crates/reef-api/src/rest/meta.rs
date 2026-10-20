use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult, DOCS};
use crate::ratelimit::RateKey;
use axum::Json;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::json;

/// `GET /`: the endpoint directory. `gh` calls this to read `X-OAuth-Scopes`.
pub async fn root(State(s): State<AppState>, auth: Auth) -> impl IntoResponse {
    let api = s.config.api_url();
    let mut h = HeaderMap::new();
    h.insert("x-oauth-scopes", auth.scopes_header().parse().unwrap());
    h.insert("x-accepted-oauth-scopes", "".parse().unwrap());
    let body = json!({
        "current_user_url": format!("{api}/user"),
        "current_user_authorizations_html_url": format!("{}/settings/tokens", s.config.site_url()),
        "authorizations_url": format!("{api}/authorizations"),
        "code_search_url": format!("{api}/search/code?q={{query}}{{&page,per_page,sort,order}}"),
        "commit_search_url": format!("{api}/search/commits?q={{query}}{{&page,per_page,sort,order}}"),
        "emails_url": format!("{api}/user/emails"),
        "emojis_url": format!("{api}/emojis"),
        "events_url": format!("{api}/events"),
        "feeds_url": format!("{api}/feeds"),
        "followers_url": format!("{api}/user/followers"),
        "following_url": format!("{api}/user/following{{/target}}"),
        "gists_url": format!("{api}/gists{{/gist_id}}"),
        "hub_url": format!("{api}/hub"),
        "issue_search_url": format!("{api}/search/issues?q={{query}}{{&page,per_page,sort,order}}"),
        "issues_url": format!("{api}/issues"),
        "keys_url": format!("{api}/user/keys"),
        "label_search_url": format!("{api}/search/labels?q={{query}}&repository_id={{repository_id}}{{&page,per_page}}"),
        "notifications_url": format!("{api}/notifications"),
        "organization_url": format!("{api}/orgs/{{org}}"),
        "organization_repositories_url": format!("{api}/orgs/{{org}}/repos{{?type,page,per_page,sort}}"),
        "organization_teams_url": format!("{api}/orgs/{{org}}/teams"),
        "public_gists_url": format!("{api}/gists/public"),
        "rate_limit_url": format!("{api}/rate_limit"),
        "repository_url": format!("{api}/repos/{{owner}}/{{repo}}"),
        "repository_search_url": format!("{api}/search/repositories?q={{query}}{{&page,per_page,sort,order}}"),
        "current_user_repositories_url": format!("{api}/user/repos{{?type,page,per_page,sort}}"),
        "starred_url": format!("{api}/user/starred{{/owner}}{{/repo}}"),
        "starred_gists_url": format!("{api}/gists/starred"),
        "topic_search_url": format!("{api}/search/topics?q={{query}}{{&page,per_page}}"),
        "user_url": format!("{api}/users/{{user}}"),
        "user_organizations_url": format!("{api}/user/orgs"),
        "user_repositories_url": format!("{api}/users/{{user}}/repos{{?type,page,per_page,sort}}"),
        "user_search_url": format!("{api}/search/users?q={{query}}{{&page,per_page,sort,order}}"),
    });
    (h, Json(body))
}

pub async fn meta(State(s): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "verifiable_password_authentication": true,
        // `installed_version` stays bare semver: `gh` and octokit parse it as a GHES version.
        "installed_version": env!("CARGO_PKG_VERSION"),
        "version": crate::VERSION,
        "reef": {"name": s.config.instance_name, "version": crate::VERSION},
        "ssh_key_fingerprints": {},
        "ssh_keys": [],
        "hooks": [], "github_enterprise_importer": [], "web": [], "api": [], "git": [], "packages": [], "pages": [], "importer": [], "actions": [], "actions_macos": [], "dependabot": [], "copilot": [],
        "domains": {}
    }))
}

pub async fn versions() -> Json<serde_json::Value> {
    Json(json!(["2022-11-28", "2026-03-10"]))
}

/// `GET /rate_limit`: the caller's budget as the limiter sees it (reading it does not spend
/// a request). Every resource shares the one `core` budget. When rate limiting is off for
/// this caller the answer is 404, as GitHub Enterprise Server answers with it disabled.
pub async fn rate_limit(State(s): State<AppState>, key: Option<Extension<RateKey>>) -> Result<Json<serde_json::Value>, ApiError> {
    let core = key.and_then(|k| s.limiter.status(k.0.0).status()).ok_or_else(|| ApiError(reef_core::Error::NotFound))?;
    let core = core.json();
    Ok(Json(json!({
        "resources": {
            "core": core, "search": core, "graphql": core, "integration_manifest": core, "code_search": core,
        },
        "rate": core
    })))
}

pub async fn zen() -> &'static str {
    "Keep it logically awesome."
}

pub async fn octocat() -> &'static str {
    "               MMM.           .MMM\n               MMMMMMMMMMMMMMMMMMM\n               MMMMMMMMMMMMMMMMMMM      ____________\n              MMMMMMMMMMMMMMMMMMMMM    |            |\n             MMMMMMMMMMMMMMMMMMMMMMM   | Reef, on. |\n            MMMMMMMMMMMMMMMMMMMMMMMM   |____________|\n"
}

pub async fn emojis() -> Json<serde_json::Value> {
    Json(json!({}))
}

pub async fn gitignore_templates() -> Json<serde_json::Value> {
    Json(json!([]))
}

pub async fn licenses() -> Json<serde_json::Value> {
    Json(json!([]))
}

pub async fn fallback() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found", "documentation_url": DOCS, "status": "404"})))
}

/// Helper for handlers that want an explicit 404 with the standard body.
pub fn nf<T>() -> ApiResult<T> {
    Err(ApiError(reef_core::Error::NotFound))
}
