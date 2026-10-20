//! REST search: repositories and users. Substring matching over what the caller can see;
//! a real index comes with the search crate later.

use crate::AppState;
use crate::auth::Auth;
use crate::error::ApiResult;
use crate::graphql::query::parse_search;
use crate::rest::Page;
use crate::shapes;
use axum::Json;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde_json::{Value, json};

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SearchQueryPublic {
    #[serde(default)]
    pub q: String,
    pub sort: Option<String>,
    pub order: Option<String>,
}

pub async fn repositories(State(s): State<AppState>, auth: Auth, page: Page, Query(q): Query<SearchQuery>) -> ApiResult<impl IntoResponse> {
    let (terms, filters) = parse_search(&q.q);
    let owner = filters.iter().find_map(|(k, v)| if k == "user" || k == "org" || k == "owner" { Some(v.clone()) } else { None });
    let candidates = match owner {
        Some(o) => match s.db.account_by_login(&o).await {
            Ok(a) => s.db.repos_for_owner(&a, auth.user.as_ref(), 1, 10_000).await?,
            Err(_) => vec![],
        },
        None => {
            let mut all = s.db.public_repos(0, 1000).await?;
            if let Some(u) = &auth.user {
                for rw in s.db.repos_for_user(u, "owner,collaborator,organization_member").await? {
                    if !all.iter().any(|r| r.repo.id == rw.repo.id) {
                        all.push(rw);
                    }
                }
            }
            all
        }
    };
    let mut hits: Vec<_> = candidates
        .into_iter()
        .filter(|rw| {
            let hay = format!("{} {}", rw.full_name(), rw.repo.description.clone().unwrap_or_default()).to_lowercase();
            terms.iter().all(|t| hay.contains(&t.to_lowercase()))
                && filters.iter().all(|(k, v)| match (k.as_str(), v.as_str()) {
                    ("is", "public") => !rw.repo.is_private(),
                    ("is", "private") => rw.repo.is_private(),
                    ("archived", v) => (v == "true") == (rw.repo.archived != 0),
                    ("fork", "only") => rw.repo.is_fork != 0,
                    ("fork", "false") => rw.repo.is_fork == 0,
                    ("in", _) | ("language", _) | ("topic", _) | ("stars", _) | ("size", _) | ("pushed", _) | ("created", _) => true,
                    _ => true,
                })
        })
        .collect();
    let desc = q.order.as_deref() != Some("asc");
    match q.sort.as_deref() {
        Some("stars") => hits.sort_by_key(|r| r.repo.stars_count),
        Some("forks") => hits.sort_by_key(|r| r.repo.forks_count),
        Some("updated") => hits.sort_by_key(|r| r.repo.updated_at),
        _ => hits.sort_by_key(|r| r.repo.pushed_at.unwrap_or(r.repo.created_at)),
    }
    if desc {
        hits.reverse();
    }
    let total = hits.len();
    let all: Vec<Value> = hits
        .iter()
        .map(|r| {
            let mut v = shapes::repo(&s.config, r, None);
            v.as_object_mut().unwrap().insert("score".into(), json!(1.0));
            v
        })
        .collect();
    let (items, h) = page.slice(&all, &format!("{}/search/repositories?q={}", s.config.api_url(), urlenc(&q.q)));
    Ok((h, Json(json!({"total_count": total, "incomplete_results": false, "items": items}))))
}

pub async fn users(State(s): State<AppState>, page: Page, Query(q): Query<SearchQuery>) -> ApiResult<impl IntoResponse> {
    let (terms, filters) = parse_search(&q.q);
    let kind = filters.iter().find_map(|(k, v)| if k == "type" { Some(v.to_lowercase()) } else { None });
    let mut all = Vec::new();
    for k in ["user", "org"] {
        if kind.as_deref().map(|t| (t == "org") == (k == "org")).unwrap_or(true) {
            all.extend(s.db.list_accounts(k, 0, 1000).await?);
        }
    }
    let hits: Vec<_> = all
        .into_iter()
        .filter(|a| {
            let hay = format!("{} {}", a.login, a.name.clone().unwrap_or_default()).to_lowercase();
            terms.iter().all(|t| hay.contains(&t.to_lowercase()))
        })
        .collect();
    let total = hits.len();
    let all: Vec<Value> = hits
        .iter()
        .map(|a| {
            let mut v = shapes::simple_user(&s.config, a);
            v.as_object_mut().unwrap().insert("score".into(), json!(1.0));
            v
        })
        .collect();
    let (items, h) = page.slice(&all, &format!("{}/search/users?q={}", s.config.api_url(), urlenc(&q.q)));
    Ok((h, Json(json!({"total_count": total, "incomplete_results": false, "items": items}))))
}

pub async fn empty(Query(_q): Query<SearchQuery>) -> Json<Value> {
    Json(json!({"total_count": 0, "incomplete_results": false, "items": []}))
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
