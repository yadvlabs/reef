use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use serde_json::{Value, json};

pub async fn current_user_orgs(State(s): State<AppState>, auth: Auth, p: Page) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let orgs = s.db.user_orgs(u.id()).await?;
    let all: Vec<Value> = orgs.iter().map(|o| shapes::org_simple(&s.config, o)).collect();
    let (items, h) = p.slice(&all, &format!("{}/user/orgs", s.config.api_url()));
    Ok((h, Json(json!(items))))
}

pub async fn user_orgs(State(s): State<AppState>, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let a = s.db.account_by_login(&login).await?;
    let orgs = s.db.user_orgs(a.id()).await?;
    Ok(Json(json!(orgs.iter().map(|o| shapes::org_simple(&s.config, o)).collect::<Vec<_>>())))
}

pub async fn list(State(s): State<AppState>, Query(q): Query<super::users::Since>) -> ApiResult<Json<Value>> {
    let _ = &q;
    let orgs = s.db.list_accounts("org", 0, 100).await?;
    Ok(Json(json!(orgs.iter().map(|o| shapes::org_simple(&s.config, o)).collect::<Vec<_>>())))
}

async fn org(s: &AppState, login: &str) -> ApiResult<Account> {
    let a = s.db.account_by_login(login).await?;
    if !a.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    Ok(a)
}

pub async fn show(State(s): State<AppState>, auth: Auth, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let o = org(&s, &login).await?;
    let member = match &auth.user {
        Some(u) => u.is_admin() || s.db.org_membership(o.id(), u.id()).await?.is_some(),
        None => false,
    };
    let n = s.db.count_repos_for_owner(o.id(), member).await?;
    Ok(Json(shapes::org_full(&s.config, &o, n, member)))
}

#[derive(serde::Deserialize)]
pub struct UpdateOrg {
    name: Option<String>,
    description: Option<String>,
    billing_email: Option<String>,
    blog: Option<String>,
    location: Option<String>,
    company: Option<String>,
}

pub async fn update(State(s): State<AppState>, auth: Auth, Path(login): Path<String>, Json(b): Json<UpdateOrg>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let o = org(&s, &login).await?;
    require_org_admin(&s, u, &o).await?;
    // Reuse the user patch for the shared profile columns; org-only columns are set directly.
    let a =
        s.db.update_user(
            o.id(),
            reef_db::accounts::UserPatch {
                name: b.name.map(Some),
                email: None,
                bio: None,
                company: b.company.map(Some),
                location: b.location.map(Some),
                blog: b.blog.map(Some),
            },
        )
        .await?;
    if b.description.is_some() || b.billing_email.is_some() {
        reef_db::sqlx::query(
            "UPDATE accounts SET description = COALESCE($1, description), billing_email = COALESCE($2, billing_email) WHERE id = $3",
        )
        .bind(b.description)
        .bind(b.billing_email)
        .bind(a.id)
        .execute(s.db.pool())
        .await
        .map_err(Error::internal)?;
    }
    let a = s.db.account_by_id(o.id()).await?;
    let n = s.db.count_repos_for_owner(a.id(), true).await?;
    Ok(Json(shapes::org_full(&s.config, &a, n, true)))
}

pub async fn require_org_admin(s: &AppState, u: &Account, o: &Account) -> ApiResult<()> {
    if u.is_admin() {
        return Ok(());
    }
    match s.db.org_membership(o.id(), u.id()).await? {
        Some(m) if m.role == "admin" => Ok(()),
        _ => Err(crate::error::forbidden("Must be an organization owner")),
    }
}

#[derive(serde::Deserialize)]
pub struct AdminCreateOrg {
    login: String,
    admin: Option<String>,
    profile_name: Option<String>,
}

/// `POST /admin/organizations` (site administrators). Reef also lets ordinary users
/// create organizations through its own API.
pub async fn admin_create(State(s): State<AppState>, auth: Auth, Json(b): Json<AdminCreateOrg>) -> ApiResult<impl IntoResponse> {
    let u = auth.require_admin()?;
    let owner = match b.admin {
        Some(l) => s.db.account_by_login(&l).await?,
        None => u.clone(),
    };
    let o = s.db.create_org(&b.login, owner.id(), b.profile_name.as_deref(), None).await?;
    Ok((StatusCode::CREATED, Json(shapes::org_simple(&s.config, &o))))
}

#[derive(serde::Deserialize)]
pub struct MembersQuery {
    role: Option<String>,
}

pub async fn members(
    State(s): State<AppState>,
    Path(login): Path<String>,
    page: Page,
    Query(q): Query<MembersQuery>,
) -> ApiResult<impl IntoResponse> {
    let o = org(&s, &login).await?;
    let role = match q.role.as_deref() {
        Some("admin") => Some("admin"),
        Some("member") => Some("member"),
        _ => None,
    };
    let ms = s.db.org_members(o.id(), role).await?;
    let all: Vec<Value> = ms.iter().map(|(a, _)| shapes::simple_user(&s.config, a)).collect();
    let (items, h) = page.slice(&all, &format!("{}/orgs/{}/members", s.config.api_url(), o.login));
    Ok((h, Json(json!(items))))
}

pub async fn is_member(State(s): State<AppState>, Path((login, user)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let o = org(&s, &login).await?;
    let u = s.db.account_by_login(&user).await?;
    match s.db.org_membership(o.id(), u.id()).await? {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Ok(StatusCode::NOT_FOUND),
    }
}

fn membership_json(s: &AppState, o: &Account, u: &Account, role: &str) -> Value {
    json!({
        "url": format!("{}/orgs/{}/memberships/{}", s.config.api_url(), o.login, u.login),
        "state": "active",
        "role": role,
        "organization_url": format!("{}/orgs/{}", s.config.api_url(), o.login),
        "organization": shapes::org_simple(&s.config, o),
        "user": shapes::simple_user(&s.config, u),
    })
}

pub async fn membership(State(s): State<AppState>, Path((login, user)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let o = org(&s, &login).await?;
    let u = s.db.account_by_login(&user).await?;
    let m = s.db.org_membership(o.id(), u.id()).await?.ok_or(ApiError(Error::NotFound))?;
    Ok(Json(membership_json(&s, &o, &u, &m.role)))
}

pub async fn current_membership(State(s): State<AppState>, auth: Auth, Path(login): Path<String>) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let o = org(&s, &login).await?;
    let m = s.db.org_membership(o.id(), u.id()).await?.ok_or(ApiError(Error::NotFound))?;
    Ok(Json(membership_json(&s, &o, u, &m.role)))
}

pub async fn current_memberships(State(s): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let orgs = s.db.user_orgs(u.id()).await?;
    let mut out = Vec::new();
    for o in orgs {
        if let Some(m) = s.db.org_membership(o.id(), u.id()).await? {
            out.push(membership_json(&s, &o, u, &m.role));
        }
    }
    Ok(Json(json!(out)))
}

#[derive(serde::Deserialize, Default)]
pub struct SetMembership {
    role: Option<String>,
}

pub async fn set_membership(
    State(s): State<AppState>,
    auth: Auth,
    Path((login, user)): Path<(String, String)>,
    body: Option<Json<SetMembership>>,
) -> ApiResult<Json<Value>> {
    let me = auth.require()?;
    let o = org(&s, &login).await?;
    require_org_admin(&s, me, &o).await?;
    let u = s.db.account_by_login(&user).await?;
    if !u.is_user() {
        return Err(ApiError(Error::validation("not a user")));
    }
    let role = body.and_then(|b| b.0.role).unwrap_or_else(|| "member".into());
    s.db.add_org_member(o.id(), u.id(), &role).await?;
    Ok(Json(membership_json(&s, &o, &u, &role)))
}

pub async fn remove_member(State(s): State<AppState>, auth: Auth, Path((login, user)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let me = auth.require()?;
    let o = org(&s, &login).await?;
    let u = s.db.account_by_login(&user).await?;
    if u.id != me.id {
        require_org_admin(&s, me, &o).await?;
    }
    let admins = s.db.org_members(o.id(), Some("admin")).await?;
    if admins.len() == 1 && admins[0].0.id == u.id {
        return Err(ApiError(Error::validation("cannot remove the last owner of an organization")));
    }
    s.db.remove_org_member(o.id(), u.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub fn _unused(_: Id) {}

/// `DELETE /orgs/{org}`: owners and site administrators remove the organization, its
/// repositories (including git storage), projects and memberships.
pub async fn delete(State(s): State<AppState>, auth: Auth, Path(org): Path<String>) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let o = s.db.account_by_login(&org).await?;
    if !o.is_org() {
        return Err(ApiError(Error::NotFound));
    }
    require_org_admin(&s, u, &o).await?;
    if !auth.has_scope("admin:org") && !auth.has_scope("delete_repo") {
        return Err(crate::error::forbidden("Must have the admin:org scope to delete an organization."));
    }
    // Detached: a request cut short never leaves the organization half removed.
    let (s, u, o) = (s.clone(), u.clone(), o.clone());
    crate::timeout::detached(async move {
        // Site admins and owners see every repository of the org, so paging until empty deletes all.
        loop {
            let repos = s.db.repos_for_owner(&o, Some(&u), 1, 500).await?;
            if repos.is_empty() {
                break;
            }
            for rw in repos {
                match crate::rest::repos::delete_repository(&s, rw.repo.id()).await {
                    // The row is gone; leftover storage is only disk space.
                    Ok(()) => {}
                    Err(e) if s.db.repo_with_owner_by_id(rw.repo.id()).await.is_err() => {
                        tracing::warn!(repo = rw.repo.id, error = %e, "deleting an organization: removing repository storage failed");
                    }
                    Err(e) => return Err(ApiError(e)),
                }
            }
        }
        if s.db.count_repos_for_owner(o.id(), true).await? > 0 {
            return Err(ApiError(Error::validation("some repositories could not be deleted; the organization was left in place")));
        }
        for p in s.db.projects(o.id(), true, false).await.unwrap_or_default() {
            let _ = s.db.delete_project(p.id()).await;
        }
        s.db.delete_org(o.id()).await?;
        Ok::<(), ApiError>(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
