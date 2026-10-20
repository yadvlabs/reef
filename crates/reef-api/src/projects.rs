//! Projects (boards): Reef's own JSON API used by the web UI. The GraphQL `projectsV2`
//! surface for `gh project` lives in `graphql::projects`.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::shapes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use reef_core::time::{iso, iso_opt};
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::projects::{Field, FieldOption, Item, Project, StatusUpdate, WORKFLOW_KINDS};
use serde_json::{Value, json};
use std::collections::HashMap;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{owner}", get(list).post(create))
        .route("/projects/{owner}/{number}", get(show).patch(update).delete(delete))
        .route("/projects/{owner}/{number}/items", post(add_item))
        .route("/projects/{owner}/{number}/items/{item}", axum::routing::patch(update_item).delete(delete_item))
        .route("/projects/{owner}/{number}/items/{item}/move", post(move_item))
        .route("/projects/{owner}/{number}/items/{item}/convert", post(convert_item))
        .route("/projects/{owner}/{number}/fields", post(create_field))
        .route("/projects/{owner}/{number}/fields/{field}", axum::routing::patch(update_field).delete(delete_field))
        .route("/projects/{owner}/{number}/views", post(create_view))
        .route("/projects/{owner}/{number}/views/{view}", axum::routing::patch(update_view).delete(delete_view))
        .route("/projects/{owner}/{number}/repos", post(link_repo))
        .route("/projects/{owner}/{number}/repos/{repo}", axum::routing::delete(unlink_repo))
        .route("/projects/{owner}/{number}/search", get(search_content))
        .route("/projects/{owner}/{number}/insights", get(insights))
        .route("/projects/{owner}/{number}/workflows", get(workflows))
        .route("/projects/{owner}/{number}/workflows/{kind}", axum::routing::put(put_workflow))
        .route("/projects/{owner}/{number}/archive", get(archive))
        .route("/projects/{owner}/{number}/status_updates", get(status_updates).post(create_status_update))
        .route("/projects/{owner}/{number}/status_updates/{id}", axum::routing::delete(delete_status_update))
        .route("/repos/{owner}/{repo}/projects", get(repo_projects))
        .route("/repos/{owner}/{repo}/issues/{number}/projects", get(issue_projects))
}

/// Projects an issue or pull request is in (with its field values), plus the projects the
/// viewer could add it to: the repository owner's and the viewer's own writable projects.
pub async fn issue_projects(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = crate::rest::repos::load(&s, &auth, &owner, &repo).await?;
    let issue = s.db.issue_by_number(rw.repo.id(), number).await?;
    let mut in_projects = Vec::new();
    let mut in_ids = Vec::new();
    for (p, it) in s.db.projects_of_issue(issue.id()).await? {
        let o = s.db.account_by_id(Id(p.owner_id)).await?;
        let a = access(&s, auth.user.as_ref(), &o, Some(&p)).await?;
        if a < Access::Read {
            continue;
        }
        in_ids.push(p.id);
        let fields: Vec<Value> = s.db.fields(p.id()).await?.iter().map(field_json).collect();
        let values: serde_json::Map<String, Value> =
            s.db.item_values(it.id()).await?.into_iter().map(|(f, v)| (f.to_string(), json!(v))).collect();
        in_projects.push(json!({"project": project_json(&s, &o, &p, a), "item_id": it.id, "fields": fields, "values": values}));
    }
    let mut available = Vec::new();
    if let Some(u) = &auth.user {
        let mut owners = vec![rw.owner.clone()];
        if rw.owner.id != u.id {
            owners.push(u.clone());
        }
        for o in owners {
            let a = access(&s, Some(u), &o, None).await?;
            if a < Access::Write {
                continue;
            }
            for p in s.db.projects(o.id(), false, false).await? {
                if !in_ids.contains(&p.id) {
                    available.push(project_json(&s, &o, &p, a));
                }
            }
        }
    }
    Ok(Json(json!({"projects": in_projects, "available": available})))
}

/// Projects linked to a repository, filtered to what the viewer may see.
pub async fn repo_projects(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = crate::rest::repos::load(&s, &auth, &owner, &repo).await?;
    let mut out = Vec::new();
    for p in s.db.projects_of_repo(rw.repo.id()).await? {
        let o = s.db.account_by_id(Id(p.owner_id)).await?;
        let a = access(&s, auth.user.as_ref(), &o, Some(&p)).await?;
        if a >= Access::Read {
            out.push(project_json(&s, &o, &p, a));
        }
    }
    Ok(Json(json!(out)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    None,
    Read,
    Write,
    Admin,
}

/// Who may do what on an owner's projects: the user themself and org admins are admin,
/// org members write, everyone else reads public projects.
pub async fn access(s: &AppState, viewer: Option<&Account>, owner: &Account, project: Option<&Project>) -> ApiResult<Access> {
    let public = project.map(|p| p.public != 0).unwrap_or(true);
    let Some(v) = viewer else { return Ok(if public { Access::Read } else { Access::None }) };
    if v.is_admin() || v.id == owner.id {
        return Ok(Access::Admin);
    }
    if owner.is_org()
        && let Some(m) = s.db.org_membership(owner.id(), v.id()).await?
    {
        return Ok(if m.role == "admin" { Access::Admin } else { Access::Write });
    }
    Ok(if public { Access::Read } else { Access::None })
}

pub async fn load(s: &AppState, auth: &Auth, owner: &str, number: i64, min: Access) -> ApiResult<(Account, Project, Access)> {
    let o = s.db.account_by_login(owner).await?;
    let p = s.db.project_by_number(o.id(), number).await?;
    let a = access(s, auth.user.as_ref(), &o, Some(&p)).await?;
    if a < min {
        return Err(if a == Access::None {
            ApiError(Error::NotFound)
        } else {
            crate::error::forbidden("You do not have permission to change this project")
        });
    }
    Ok((o, p, a))
}

pub fn project_json(s: &AppState, owner: &Account, p: &Project, access: Access) -> Value {
    json!({
        "id": p.id,
        "node_id": shapes::node_id("PVT", p.id),
        "number": p.number,
        "title": p.title,
        "short_description": p.short_description,
        "readme": p.readme,
        "public": p.public != 0,
        "closed": p.closed != 0,
        "owner": shapes::simple_user(&s.config, owner),
        "url": format!("{}/{}/projects/{}", s.config.site_url(), owner.login, p.number),
        "created_at": iso(p.created_at),
        "updated_at": iso(p.updated_at),
        "closed_at": iso_opt(p.closed_at),
        "permissions": {"read": access >= Access::Read, "write": access >= Access::Write, "admin": access >= Access::Admin},
    })
}

fn field_json(f: &Field) -> Value {
    json!({"id": f.id, "node_id": shapes::node_id("PVTF", f.id), "name": f.name, "data_type": f.data_type, "options": f.options()})
}

pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    Path(owner): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let o = s.db.account_by_login(&owner).await?;
    let a = access(&s, auth.user.as_ref(), &o, None).await?;
    let closed = q.get("state").map(|st| st == "closed" || st == "all").unwrap_or(false);
    let mut list = s.db.projects(o.id(), closed, a < Access::Write).await?;
    if q.get("state").map(|st| st == "closed").unwrap_or(false) {
        list.retain(|p| p.closed != 0);
    }
    let mut out = Vec::new();
    for p in &list {
        let items = s.db.items(p.id(), false).await?;
        let mut v = project_json(&s, &o, p, a);
        v.as_object_mut().unwrap().insert("items_count".into(), json!(items.len()));
        v.as_object_mut().unwrap().insert("status_update".into(), latest_status_json(&s, p).await);
        out.push(v);
    }
    Ok(Json(json!(out)))
}

#[derive(serde::Deserialize)]
pub struct NewProject {
    title: String,
    short_description: Option<String>,
    #[serde(default)]
    public: bool,
    repo: Option<String>,
}

pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    Path(owner): Path<String>,
    Json(b): Json<NewProject>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?;
    let o = s.db.account_by_login(&owner).await?;
    let a = access(&s, Some(u), &o, None).await?;
    if a < Access::Write {
        return Err(crate::error::forbidden("You cannot create projects for this owner"));
    }
    let p = s.db.create_project(o.id(), u.id(), &b.title, b.short_description.as_deref(), b.public).await?;
    if let Some(r) = b.repo
        && let Ok(rw) = s.db.repo_by_path(&owner, &r).await
    {
        s.db.link_repo(p.id(), rw.repo.id()).await?;
    }
    Ok((StatusCode::CREATED, Json(project_json(&s, &o, &p, a))))
}

/// The whole project for the board: fields, views, items with content and values.
pub async fn show(State(s): State<AppState>, auth: Auth, Path((owner, number)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let (o, p, a) = load(&s, &auth, &owner, number, Access::Read).await?;
    let fields = s.db.fields(p.id()).await?;
    let views = s.db.views(p.id()).await?;
    let items = s.db.items(p.id(), false).await?;
    let values = s.db.project_values(p.id()).await?;
    let mut by_item: HashMap<i64, serde_json::Map<String, Value>> = HashMap::new();
    for (item, field, value) in values {
        by_item.entry(item).or_default().insert(field.to_string(), json!(value));
    }
    let mut item_json = Vec::new();
    for it in &items {
        item_json.push(item_view(&s, &o, it, by_item.remove(&it.id).unwrap_or_default(), auth.user.as_ref()).await?);
    }
    let mut repos = Vec::new();
    for rid in s.db.linked_repos(p.id()).await? {
        if let Ok(rw) = s.db.repo_with_owner_by_id(Id(rid)).await {
            repos.push(json!({"id": rw.repo.id, "full_name": rw.full_name()}));
        }
    }
    let mut v = project_json(&s, &o, &p, a);
    let m = v.as_object_mut().unwrap();
    m.insert("fields".into(), json!(fields.iter().map(field_json).collect::<Vec<_>>()));
    m.insert("views".into(), json!(views.iter().map(view_json).collect::<Vec<_>>()));
    m.insert("items".into(), json!(item_json));
    m.insert("repositories".into(), json!(repos));
    m.insert("readme_html".into(), json!(crate::render::markdown(p.readme.as_deref().unwrap_or(""))));
    m.insert("status_update".into(), latest_status_json(&s, &p).await);
    m.insert("archived_count".into(), json!(s.db.archived_items(p.id()).await.map(|l| l.len()).unwrap_or(0)));
    Ok(Json(v))
}

async fn item_view(
    s: &AppState,
    owner: &Account,
    it: &Item,
    values: serde_json::Map<String, Value>,
    viewer: Option<&Account>,
) -> ApiResult<Value> {
    let content = match (it.content_type.as_str(), it.content_id) {
        ("draft", _) => json!({"type": "draft", "title": it.draft_title, "body": it.draft_body}),
        (_, Some(cid)) => match s.db.issue_full_by_id(Id(cid)).await {
            Ok(i) => {
                let rw = s.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await?;
                let perm = s.db.permission(viewer, &rw).await?;
                if !perm.can_read() {
                    json!({"type": "hidden"})
                } else {
                    let m = s.db.pull_meta(i.issue.id()).await.ok();
                    json!({
                        "type": if i.issue.is_pull() { "pull" } else { "issue" },
                        "id": i.issue.id, "number": i.issue.number, "title": i.issue.title, "state": i.issue.state,
                        "body": i.issue.body, "body_html": crate::render::markdown_in_repo(i.issue.body.as_deref().unwrap_or(""), &s.config, &rw),
                        "milestone": i.milestone.as_ref().map(|m| m.title.clone()),
                        "state_reason": i.issue.state_reason, "merged": m.map(|m| m.merged != 0).unwrap_or(false),
                        "repository": rw.full_name(),
                        "url": format!("{}/{}/{}/{}", s.config.site_url(), rw.full_name(), if i.issue.is_pull() { "pull" } else { "issues" }, i.issue.number),
                        "labels": i.labels.iter().map(|l| json!({"name": l.name, "color": l.color})).collect::<Vec<_>>(),
                        "assignees": i.assignees.iter().map(|a| shapes::simple_user(&s.config, a)).collect::<Vec<_>>(),
                        "author": shapes::simple_user(&s.config, &i.author),
                    })
                }
            }
            Err(_) => json!({"type": "deleted"}),
        },
        _ => json!({"type": "deleted"}),
    };
    let _ = owner;
    Ok(json!({
        "id": it.id, "node_id": shapes::node_id("PVTI", it.id), "position": it.position, "archived": it.archived != 0,
        "content": content, "values": values, "created_at": iso(it.created_at), "updated_at": iso(it.updated_at),
    }))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateProject {
    title: Option<String>,
    short_description: Option<Option<String>>,
    readme: Option<Option<String>>,
    public: Option<bool>,
    closed: Option<bool>,
}

pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Json(b): Json<UpdateProject>,
) -> ApiResult<Json<Value>> {
    let (o, p, a) = load(&s, &auth, &owner, number, Access::Admin).await?;
    let p =
        s.db.update_project(
            p.id(),
            b.title.as_deref(),
            b.short_description.as_ref().map(|d| d.as_deref()),
            b.readme.as_ref().map(|r| r.as_deref()),
            b.public,
            b.closed,
        )
        .await?;
    Ok(Json(project_json(&s, &o, &p, a)))
}

pub async fn delete(State(s): State<AppState>, auth: Auth, Path((owner, number)): Path<(String, i64)>) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Admin).await?;
    s.db.delete_project(p.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct AddItem {
    /// `owner/repo#123` or `#123` (repo from the project's links), or an issue id.
    issue: Option<String>,
    issue_id: Option<i64>,
    /// Draft item.
    title: Option<String>,
    body: Option<String>,
    /// Initial field values (field id -> value).
    values: Option<HashMap<String, Option<String>>>,
}

pub async fn add_item(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Json(b): Json<AddItem>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (o, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let u = auth.user.clone().unwrap();
    let item = if let Some(t) = b.title.filter(|t| !t.trim().is_empty()) {
        s.db.add_item(p.id(), "draft", None, Some(t.trim()), b.body.as_deref(), Some(u.id())).await?
    } else {
        let issue = match (b.issue_id, b.issue.as_deref()) {
            (Some(id), _) => s.db.issue_by_id(Id(id)).await?,
            (None, Some(r)) => {
                let (repo_part, num) = r.rsplit_once('#').ok_or_else(|| ApiError(Error::validation("use owner/repo#number")))?;
                let num: i64 = num.trim().parse().map_err(|_| ApiError(Error::validation("invalid issue number")))?;
                let (ro, rn) = if repo_part.is_empty() {
                    let linked = s.db.linked_repos(p.id()).await?;
                    let rid = linked.first().ok_or_else(|| ApiError(Error::validation("no linked repository; use owner/repo#number")))?;
                    let rw = s.db.repo_with_owner_by_id(Id(*rid)).await?;
                    (rw.owner.login.clone(), rw.repo.name.clone())
                } else {
                    let (a, c) = repo_part.split_once('/').ok_or_else(|| ApiError(Error::validation("use owner/repo#number")))?;
                    (a.to_string(), c.to_string())
                };
                let rw = s.db.repo_by_path(&ro, &rn).await?;
                if !s.db.permission(Some(&u), &rw).await?.can_read() {
                    return Err(ApiError(Error::NotFound));
                }
                s.db.issue_by_number(rw.repo.id(), num).await?
            }
            _ => return Err(ApiError(Error::validation("issue or title is required"))),
        };
        let rw = s.db.repo_with_owner_by_id(Id(issue.repo_id)).await?;
        if !s.db.permission(Some(&u), &rw).await?.can_read() {
            return Err(ApiError(Error::NotFound));
        }
        match s.db.items_for_content(Id(issue.id)).await?.into_iter().find(|it| it.project_id == p.id) {
            Some(existing) => existing,
            None => s.db.add_item(p.id(), if issue.is_pull() { "pull" } else { "issue" }, Some(issue.id), None, None, Some(u.id())).await?,
        }
    };
    if let Some(vals) = &b.values {
        for (fid, v) in vals {
            if let Ok(f) = fid.parse::<i64>()
                && let Ok(fld) = s.db.field_by_id(Id(f)).await
                && fld.project_id == p.id
            {
                s.db.set_item_value(item.id(), Id(f), v.as_deref()).await?;
            }
        }
    }
    s.db.default_status(p.id(), item.id()).await?;
    let values: serde_json::Map<String, Value> =
        s.db.item_values(item.id()).await?.into_iter().map(|(f, v)| (f.to_string(), json!(v))).collect();
    Ok((StatusCode::CREATED, Json(item_view(&s, &o, &item, values, Some(&u)).await?)))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateItem {
    title: Option<String>,
    body: Option<Option<String>>,
    /// field id -> value (null clears)
    values: Option<HashMap<String, Option<String>>>,
    archived: Option<bool>,
}

pub async fn update_item(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, item)): Path<(String, i64, i64)>,
    Json(b): Json<UpdateItem>,
) -> ApiResult<Json<Value>> {
    let (o, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let it = s.db.item_by_id(Id(item)).await?;
    if it.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    if it.content_type == "draft" && (b.title.is_some() || b.body.is_some()) {
        s.db.update_draft(it.id(), b.title.as_deref(), b.body.as_ref().map(|x| x.as_deref())).await?;
    }
    if let Some(vals) = b.values {
        let fields = s.db.fields(p.id()).await?;
        for (fid, val) in vals {
            let fid: i64 = fid.parse().map_err(|_| ApiError(Error::validation("bad field id")))?;
            let f = fields.iter().find(|f| f.id == fid).ok_or_else(|| ApiError(Error::validation("unknown field")))?;
            if let Some(v) = &val
                && f.data_type == "single_select"
                && !f.options().iter().any(|op| op.id == *v)
            {
                return Err(ApiError(Error::validation("unknown option")));
            }
            s.db.set_item_value(it.id(), Id(fid), val.as_deref()).await?;
            if let (Some(v), Some(cid)) = (&val, it.content_id)
                && f.data_type == "single_select"
                && f.name.eq_ignore_ascii_case("Status")
            {
                auto_close(&s, &p, Id(cid), v, auth.user.as_ref().unwrap()).await;
            }
        }
    }
    if let Some(a) = b.archived {
        s.db.set_item_archived(it.id(), a).await?;
    }
    let it = s.db.item_by_id(Id(item)).await?;
    let values: serde_json::Map<String, Value> =
        s.db.item_values(it.id()).await?.into_iter().map(|(f, v)| (f.to_string(), json!(v))).collect();
    Ok(Json(item_view(&s, &o, &it, values, auth.user.as_ref()).await?))
}

#[derive(serde::Deserialize, Default)]
pub struct MoveItem {
    after: Option<i64>,
}

#[derive(serde::Deserialize)]
pub struct ConvertItem {
    /// `owner/repo` to create the issue in.
    repo: String,
}

/// Convert a draft into an issue in a repository the viewer can write to.
pub async fn convert_item(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, item)): Path<(String, i64, i64)>,
    Json(b): Json<ConvertItem>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?.clone();
    let (o, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let it = s.db.item_by_id(Id(item)).await?;
    if it.project_id != p.id || it.content_type != "draft" {
        return Err(ApiError(Error::validation("only draft items can be converted")));
    }
    let (ro, rn) = b.repo.split_once('/').ok_or_else(|| ApiError(Error::validation("repo must be owner/name")))?;
    let rw = s.db.repo_by_path(ro, rn).await?;
    if s.db.permission(Some(&u), &rw).await? < reef_db::repos::Permission::Write {
        return Err(crate::error::forbidden("Must have write access to the repository to create an issue"));
    }
    let issue =
        s.db.create_issue(
            rw.repo.id(),
            &u,
            reef_db::issues::NewIssue {
                title: it.draft_title.clone().unwrap_or_else(|| "Untitled".into()),
                body: it.draft_body.clone(),
                is_pull: false,
                labels: vec![],
                assignees: vec![],
                milestone: None,
            },
        )
        .await?;
    let _ = s.db.record_event(rw.repo.id(), Some(u.id()), "issues.opened", &json!({"number": issue.issue.number})).await;
    let it = s.db.convert_draft(it.id(), issue.issue.id(), false).await?;
    let values: serde_json::Map<String, Value> =
        s.db.item_values(it.id()).await?.into_iter().map(|(f, v)| (f.to_string(), json!(v))).collect();
    Ok(Json(item_view(&s, &o, &it, values, Some(&u)).await?))
}

pub async fn move_item(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, item)): Path<(String, i64, i64)>,
    Json(b): Json<MoveItem>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let it = s.db.item_by_id(Id(item)).await?;
    if it.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.move_item(it.id(), b.after.map(Id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_item(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, item)): Path<(String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let it = s.db.item_by_id(Id(item)).await?;
    if it.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.delete_item(it.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct NewField {
    name: String,
    data_type: String,
    #[serde(default)]
    options: Vec<NewOption>,
}

#[derive(serde::Deserialize)]
pub struct NewOption {
    id: Option<String>,
    name: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    description: String,
}

fn options_of(v: &[NewOption]) -> Vec<FieldOption> {
    v.iter()
        .map(|o| FieldOption {
            id: o.id.clone().filter(|i| !i.is_empty()).unwrap_or_else(|| reef_core::token::random_hex(4)),
            name: o.name.trim().to_string(),
            color: o.color.clone(),
            description: o.description.clone(),
        })
        .collect()
}

pub async fn create_field(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Json(b): Json<NewField>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Admin).await?;
    let f = s.db.create_field(p.id(), &b.name, &b.data_type, &options_of(&b.options)).await?;
    Ok((StatusCode::CREATED, Json(field_json(&f))))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateField {
    name: Option<String>,
    options: Option<Vec<NewOption>>,
}

pub async fn update_field(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, field)): Path<(String, i64, i64)>,
    Json(b): Json<UpdateField>,
) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Admin).await?;
    let f = s.db.field_by_id(Id(field)).await?;
    if f.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    let opts = b.options.as_deref().map(options_of);
    let f = s.db.update_field(f.id(), b.name.as_deref(), opts.as_deref()).await?;
    Ok(Json(field_json(&f)))
}

pub async fn delete_field(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, field)): Path<(String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Admin).await?;
    let f = s.db.field_by_id(Id(field)).await?;
    if f.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.delete_field(f.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn view_json(vw: &reef_db::projects::View) -> Value {
    json!({"id": vw.id, "number": vw.number, "name": vw.name, "layout": vw.layout, "group_by_field_id": vw.group_by_field_id, "filter": vw.filter, "settings": vw.settings_json()})
}

#[derive(serde::Deserialize)]
pub struct NewView {
    name: String,
    #[serde(default = "board")]
    layout: String,
    group_by_field_id: Option<i64>,
    settings: Option<Value>,
}
fn board() -> String {
    "board".into()
}

pub async fn create_view(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Json(b): Json<NewView>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let settings = b.settings.unwrap_or_else(|| json!({"column_field_id": b.group_by_field_id}));
    let v = s.db.create_view_with(p.id(), &b.name, &b.layout, b.group_by_field_id, &settings).await?;
    Ok((StatusCode::CREATED, Json(view_json(&v))))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateView {
    name: Option<String>,
    layout: Option<String>,
    group_by_field_id: Option<Option<i64>>,
    filter: Option<Option<String>>,
    settings: Option<Value>,
}

pub async fn update_view(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, view)): Path<(String, i64, i64)>,
    Json(b): Json<UpdateView>,
) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let v = s.db.view_by_id(Id(view)).await?;
    if v.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    let v =
        s.db.update_view(
            v.id(),
            b.name.as_deref(),
            b.layout.as_deref(),
            b.group_by_field_id,
            b.filter.as_ref().map(|f| f.as_deref()),
            b.settings.as_ref(),
        )
        .await?;
    Ok(Json(view_json(&v)))
}

pub async fn delete_view(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, view)): Path<(String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let v = s.db.view_by_id(Id(view)).await?;
    if v.project_id != p.id {
        return Err(ApiError(Error::NotFound));
    }
    s.db.delete_view(v.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct LinkRepo {
    repo: String,
}

pub async fn link_repo(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Json(b): Json<LinkRepo>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let (o, r) = b.repo.split_once('/').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((owner.clone(), b.repo.clone()));
    let rw = s.db.repo_by_path(&o, &r).await?;
    if !s.db.permission(auth.user.as_ref(), &rw).await?.can_write() {
        return Err(crate::error::forbidden("Must have write access to the repository"));
    }
    s.db.link_repo(p.id(), rw.repo.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unlink_repo(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, repo)): Path<(String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    s.db.unlink_repo(p.id(), Id(repo)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Search issues and pull requests the viewer can add: linked repositories first.
pub async fn search_content(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let text = q.get("q").cloned().unwrap_or_default();
    let (terms, quals) = crate::graphql::query::parse_search(&text);
    let (repo, mut f) =
        crate::rest::issues::filter_from_search(&s, &terms, &quals, None, None, auth.user.as_ref().map(|u| u.login.as_str())).await?;
    f.state = f.state.clone();
    let mut repo_ids: Vec<i64> = match &repo {
        Some(r) => vec![r.repo.id],
        None => s.db.linked_repos(p.id()).await?,
    };
    if repo_ids.is_empty() {
        repo_ids =
            s.db.repos_for_owner(&s.db.account_by_login(&owner).await?, auth.user.as_ref(), 1, 50)
                .await?
                .into_iter()
                .map(|r| r.repo.id)
                .collect();
    }
    let existing: std::collections::HashSet<i64> = s.db.items(p.id(), true).await?.into_iter().filter_map(|i| i.content_id).collect();
    let mut out = Vec::new();
    for rid in repo_ids.into_iter().take(10) {
        let rw = match s.db.repo_with_owner_by_id(Id(rid)).await {
            Ok(rw) => rw,
            Err(_) => continue,
        };
        let (items, _) = s.db.list_issues(Some(Id(rid)), &f, 1, 20).await?;
        for i in items {
            out.push(json!({"id": i.issue.id, "number": i.issue.number, "title": i.issue.title, "state": i.issue.state, "is_pull": i.issue.is_pull(), "repository": rw.full_name(), "added": existing.contains(&i.issue.id)}));
        }
    }
    Ok(Json(json!(out)))
}

// ----- status updates -----

async fn status_update_json(s: &AppState, u: &StatusUpdate) -> Value {
    let author = s.db.account_by_id(Id(u.author_id)).await.ok();
    json!({
        "id": u.id,
        "status": u.status,
        "start_date": u.start_date,
        "target_date": u.target_date,
        "body": u.body,
        "body_html": crate::render::markdown(u.body.as_deref().unwrap_or("")),
        "author": author.as_ref().map(|a| shapes::simple_user(&s.config, a)),
        "created_at": iso(u.created_at),
    })
}

async fn latest_status_json(s: &AppState, p: &Project) -> Value {
    match s.db.latest_status_update(p.id()).await {
        Ok(Some(u)) => status_update_json(s, &u).await,
        _ => Value::Null,
    }
}

pub async fn status_updates(State(s): State<AppState>, auth: Auth, Path((owner, number)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Read).await?;
    let mut out = Vec::new();
    for u in s.db.status_updates(p.id()).await? {
        out.push(status_update_json(&s, &u).await);
    }
    Ok(Json(json!(out)))
}

#[derive(serde::Deserialize)]
pub struct NewStatusUpdate {
    status: String,
    start_date: Option<String>,
    target_date: Option<String>,
    body: Option<String>,
}

pub async fn create_status_update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Json(b): Json<NewStatusUpdate>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?.clone();
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    let su =
        s.db.create_status_update(p.id(), u.id(), &b.status, b.start_date.as_deref(), b.target_date.as_deref(), b.body.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(status_update_json(&s, &su).await)))
}

pub async fn delete_status_update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, id)): Path<(String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Write).await?;
    s.db.delete_status_update(p.id(), Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- archive -----

pub async fn archive(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let (o, p, _) = load(&s, &auth, &owner, number, Access::Read).await?;
    let needle = q.get("q").map(|x| x.trim().to_lowercase()).unwrap_or_default();
    let mut out = Vec::new();
    for it in s.db.archived_items(p.id()).await? {
        let values: serde_json::Map<String, Value> =
            s.db.item_values(it.id()).await?.into_iter().map(|(f, v)| (f.to_string(), json!(v))).collect();
        let v = item_view(&s, &o, &it, values, auth.user.as_ref()).await?;
        if !needle.is_empty() {
            let title = v["content"]["title"].as_str().unwrap_or("").to_lowercase();
            let repo = v["content"]["repository"].as_str().unwrap_or("").to_lowercase();
            if !title.contains(&needle) && !repo.contains(&needle) {
                continue;
            }
        }
        out.push(v);
    }
    Ok(Json(json!(out)))
}

// ----- workflows -----

fn workflow_json(
    kind: &str,
    default_on: bool,
    default_status: Option<&str>,
    row: Option<&reef_db::projects::ProjectWorkflow>,
    status_field: Option<&Field>,
) -> Value {
    let (enabled, mut config) = match row {
        Some(w) => (w.enabled != 0, w.config_json()),
        None => (default_on, json!({})),
    };
    // Fill in the default status option so the UI shows what will happen.
    if let (Some(name), Some(f)) = (default_status, status_field)
        && config.get("status").and_then(|v| v.as_str()).filter(|v| !v.is_empty()).is_none()
        && let Some(o) = f.options().into_iter().find(|o| o.name.eq_ignore_ascii_case(name))
    {
        config["status"] = json!(o.id);
    }
    if kind == "auto_archive" && config.get("filter").is_none() {
        config["filter"] = json!("is:closed updated:<@today-2w");
    }
    if kind == "auto_add" && config.get("filter").is_none() {
        config["filter"] = json!("is:issue,pr is:open");
    }
    json!({"kind": kind, "enabled": enabled, "config": config, "configured": row.is_some()})
}

pub async fn workflows(State(s): State<AppState>, auth: Auth, Path((owner, number)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Read).await?;
    let rows = s.db.project_workflows(p.id()).await?;
    let status_field = s.db.status_field(p.id()).await?;
    let out: Vec<Value> = WORKFLOW_KINDS
        .iter()
        .map(|(k, on, st)| workflow_json(k, *on, *st, rows.iter().find(|w| w.kind == *k), status_field.as_ref()))
        .collect();
    Ok(Json(json!({"workflows": out, "status_field": status_field.as_ref().map(field_json)})))
}

#[derive(serde::Deserialize)]
pub struct PutWorkflow {
    enabled: bool,
    #[serde(default)]
    config: Value,
}

pub async fn put_workflow(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number, kind)): Path<(String, i64, String)>,
    Json(b): Json<PutWorkflow>,
) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Admin).await?;
    let config = if b.config.is_object() { b.config } else { json!({}) };
    if let Some(st) = config.get("status").and_then(|v| v.as_str()).filter(|v| !v.is_empty()) {
        let f = s.db.status_field(p.id()).await?.ok_or_else(|| ApiError(Error::validation("this project has no Status field")))?;
        if !f.options().iter().any(|o| o.id == st) {
            return Err(ApiError(Error::validation("unknown status option")));
        }
    }
    if kind == "auto_add" {
        if let Some(r) = config.get("repo").and_then(|v| v.as_str()).filter(|v| !v.is_empty()) {
            let (ro, rn) = r.split_once('/').ok_or_else(|| ApiError(Error::validation("repo must be owner/name")))?;
            let rw = s.db.repo_by_path(ro, rn).await?;
            if !s.db.permission(auth.user.as_ref(), &rw).await?.can_read() {
                return Err(ApiError(Error::NotFound));
            }
        } else if b.enabled {
            return Err(ApiError(Error::validation("auto-add needs a repository")));
        }
    }
    let w = s.db.set_project_workflow(p.id(), &kind, b.enabled, &config).await?;
    let (_, on, st) = WORKFLOW_KINDS.iter().find(|(k, _, _)| *k == kind).copied().unwrap_or((kind.as_str(), false, None));
    let status_field = s.db.status_field(p.id()).await?;
    Ok(Json(workflow_json(&kind, on, st, Some(&w), status_field.as_ref())))
}

/// Review workflows ("code changes requested", "code review approved"); call after a review
/// is submitted with `state` = `changes_requested` | `approved`.
pub async fn on_review(s: &AppState, issue: Id, state: &str) {
    if let Err(e) = s.db.review_automation(issue, state).await {
        tracing::warn!(error = %e, "project review workflow failed");
    }
}

/// "Auto-close issue": when an item's Status becomes the configured option, close the issue.
async fn auto_close(s: &AppState, p: &Project, issue: Id, option: &str, actor: &Account) {
    let Ok(Some(w)) = s.db.project_workflow(p.id(), "auto_close").await else { return };
    if w.enabled == 0 || w.status().as_deref() != Some(option) {
        return;
    }
    if let Ok(i) = s.db.issue_by_id(issue).await
        && i.is_open()
    {
        let _ = s.db.update_issue(issue, actor, reef_db::issues::IssuePatch { state: Some("closed".into()), ..Default::default() }).await;
    }
}

/// A project item as the auto-archive filter sees it.
struct ItemFacts {
    title: String,
    repository: String,
    number: i64,
    is_pull: bool,
    is_draft: bool,
    open: bool,
    merged: bool,
    labels: Vec<String>,
    assignees: Vec<String>,
    created_at: i64,
    updated_at: i64,
    status: Option<String>,
}

/// Evaluate a GitHub-style project filter (`is:closed updated:<@today-2w label:bug -status:Done`)
/// against one item. Unknown qualifiers match everything.
fn item_matches(filter: &str, it: &ItemFacts, now: i64) -> bool {
    for raw in split_filter(filter) {
        let (neg, tok) = match raw.strip_prefix('-') {
            Some(t) => (true, t.to_string()),
            None => (false, raw.clone()),
        };
        let (key, value) = match tok.split_once(':') {
            Some((k, v)) => (Some(k.to_lowercase()), v.trim_matches('"').to_string()),
            None => (None, tok.clone()),
        };
        let v = value.to_lowercase();
        let ok = match key.as_deref() {
            None => it.title.to_lowercase().contains(&v) || it.repository.to_lowercase().contains(&v) || it.number.to_string() == v,
            Some("is") => v.split(',').any(|x| match x {
                "open" => it.open,
                "closed" => !it.open && !it.is_draft,
                "issue" => !it.is_pull && !it.is_draft,
                "pr" | "pull" => it.is_pull,
                "draft" => it.is_draft,
                "merged" => it.merged,
                _ => true,
            }),
            Some("label") => it.labels.iter().any(|l| l.eq_ignore_ascii_case(&value)),
            Some("assignee") => it.assignees.iter().any(|a| a.eq_ignore_ascii_case(&value)),
            Some("repo") | Some("repository") => {
                it.repository.eq_ignore_ascii_case(&value) || it.repository.to_lowercase().ends_with(&format!("/{v}"))
            }
            Some("status") => it.status.as_deref().map(|st| st.eq_ignore_ascii_case(&value)).unwrap_or(false),
            Some("no") => match v.as_str() {
                "status" => it.status.is_none(),
                "label" => it.labels.is_empty(),
                "assignee" => it.assignees.is_empty(),
                _ => true,
            },
            Some("updated") => date_cmp(&value, it.updated_at, now),
            Some("created") => date_cmp(&value, it.created_at, now),
            _ => true,
        };
        if neg == ok {
            return false;
        }
    }
    true
}

fn split_filter(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in q.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `<@today-2w`, `>2026-01-01`, `2026-01-01..2026-02-01`, `>=@today-1m`.
fn date_cmp(expr: &str, ts: i64, now: i64) -> bool {
    let (op, rest) = if let Some(r) = expr.strip_prefix(">=") {
        (">=", r)
    } else if let Some(r) = expr.strip_prefix("<=") {
        ("<=", r)
    } else if let Some(r) = expr.strip_prefix('>') {
        (">", r)
    } else if let Some(r) = expr.strip_prefix('<') {
        ("<", r)
    } else if let Some((a, b)) = expr.split_once("..") {
        return match (parse_date_expr(a, now), parse_date_expr(b, now)) {
            (Some(x), Some(y)) => ts >= x && ts < y + 86_400_000,
            _ => true,
        };
    } else {
        ("=", expr)
    };
    let Some(t) = parse_date_expr(rest, now) else { return true };
    match op {
        ">" => ts > t + 86_400_000,
        ">=" => ts >= t,
        "<" => ts < t,
        "<=" => ts < t + 86_400_000,
        _ => ts >= t && ts < t + 86_400_000,
    }
}

/// `@today`, `@today-2w`, `@today-3d`, `@today-1m`, `@today+1y`, or `YYYY-MM-DD`; midnight UTC in ms.
fn parse_date_expr(e: &str, now: i64) -> Option<i64> {
    let e = e.trim();
    const DAY: i64 = 86_400_000;
    if let Some(rest) = e.strip_prefix("@today") {
        let today = now - now.rem_euclid(DAY);
        if rest.is_empty() {
            return Some(today);
        }
        let (sign, num) = match rest.chars().next()? {
            '-' => (-1, &rest[1..]),
            '+' => (1, &rest[1..]),
            _ => return None,
        };
        let unit = num.chars().last()?;
        let n: i64 = num[..num.len() - 1].parse().ok()?;
        let days = match unit {
            'd' => n,
            'w' => n * 7,
            'm' => n * 30,
            'y' => n * 365,
            _ => return None,
        };
        return Some(today + sign * days * DAY);
    }
    if e.len() == 10 {
        return reef_core::time::parse_iso(&format!("{e}T00:00:00Z"));
    }
    reef_core::time::parse_iso(e)
}

async fn facts(s: &AppState, p: &Project, it: &Item, status_field: Option<&Field>) -> ItemFacts {
    let values = s.db.item_values(it.id()).await.unwrap_or_default();
    let status = status_field.and_then(|f| {
        values
            .iter()
            .find(|(fid, _)| *fid == f.id)
            .and_then(|(_, v)| v.clone())
            .and_then(|id| f.options().into_iter().find(|o| o.id == id).map(|o| o.name))
    });
    let _ = p;
    match (it.content_type.as_str(), it.content_id) {
        ("draft", _) | (_, None) => ItemFacts {
            title: it.draft_title.clone().unwrap_or_default(),
            repository: String::new(),
            number: 0,
            is_pull: false,
            is_draft: true,
            open: true,
            merged: false,
            labels: vec![],
            assignees: vec![],
            created_at: it.created_at,
            updated_at: it.updated_at,
            status,
        },
        (_, Some(cid)) => match s.db.issue_full_by_id(Id(cid)).await {
            Ok(i) => {
                let repo = s.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await.map(|rw| rw.full_name()).unwrap_or_default();
                let merged = s.db.pull_meta(i.issue.id()).await.map(|m| m.merged != 0).unwrap_or(false);
                ItemFacts {
                    title: i.issue.title.clone(),
                    repository: repo,
                    number: i.issue.number,
                    is_pull: i.issue.is_pull(),
                    is_draft: false,
                    open: i.issue.is_open(),
                    merged,
                    labels: i.labels.iter().map(|l| l.name.clone()).collect(),
                    assignees: i.assignees.iter().map(|a| a.login.clone()).collect(),
                    created_at: i.issue.created_at,
                    updated_at: i.issue.updated_at.max(it.updated_at),
                    status,
                }
            }
            Err(_) => ItemFacts {
                title: String::new(),
                repository: String::new(),
                number: 0,
                is_pull: false,
                is_draft: false,
                open: false,
                merged: false,
                labels: vec![],
                assignees: vec![],
                created_at: it.created_at,
                updated_at: it.updated_at,
                status,
            },
        },
    }
}

/// Scheduled workflows: "auto-add to project" and "auto-archive items". Call periodically
/// (the server's 30s tick).
pub async fn run_scheduled_workflows(s: &AppState) {
    let now = reef_core::time::now_ms();
    // auto-archive
    if let Ok(list) = s.db.projects_with_workflow("auto_archive").await {
        for (p, w) in list {
            let filter = w.config_json().get("filter").and_then(|v| v.as_str()).unwrap_or("is:closed updated:<@today-2w").to_string();
            let status_field = s.db.status_field(p.id()).await.ok().flatten();
            let Ok(items) = s.db.items(p.id(), false).await else { continue };
            for it in items {
                let f = facts(s, &p, &it, status_field.as_ref()).await;
                if item_matches(&filter, &f, now) {
                    let _ = s.db.set_item_archived(it.id(), true).await;
                }
            }
        }
    }
    // auto-add
    if let Ok(list) = s.db.projects_with_workflow("auto_add").await {
        for (p, w) in list {
            let cfg = w.config_json();
            let Some(repo) = cfg.get("repo").and_then(|v| v.as_str()) else { continue };
            let Some((ro, rn)) = repo.split_once('/') else { continue };
            let Ok(rw) = s.db.repo_by_path(ro, rn).await else { continue };
            let filter = cfg.get("filter").and_then(|v| v.as_str()).unwrap_or("is:open").to_string();
            let is_kinds: Vec<&str> = filter.split_whitespace().filter_map(|t| t.strip_prefix("is:")).flat_map(|v| v.split(',')).collect();
            let want_issue = is_kinds.contains(&"issue") || !(is_kinds.contains(&"pr") || is_kinds.contains(&"pull"));
            let want_pull = is_kinds.contains(&"pr") || is_kinds.contains(&"pull") || !is_kinds.contains(&"issue");
            let (terms, quals) = crate::graphql::query::parse_search(
                &filter.replace("is:issue,pr", "").replace("is:pr,issue", "").replace("is:issue", "").replace("is:pr", ""),
            );
            let Ok((_, f)) = crate::rest::issues::filter_from_search(s, &terms, &quals, None, None, None).await else { continue };
            let Ok((issues, _)) = s.db.list_issues(Some(rw.repo.id()), &f, 1, 200).await else { continue };
            let existing: std::collections::HashSet<i64> =
                s.db.items(p.id(), true).await.unwrap_or_default().into_iter().filter_map(|i| i.content_id).collect();
            for i in issues {
                if existing.contains(&i.issue.id) || (i.issue.is_pull() && !want_pull) || (!i.issue.is_pull() && !want_issue) {
                    continue;
                }
                if let Ok(item) =
                    s.db.add_item(p.id(), if i.issue.is_pull() { "pull" } else { "issue" }, Some(i.issue.id), None, None, None).await
                {
                    let _ = s.db.default_status(p.id(), item.id()).await;
                }
            }
        }
    }
}

// ----- insights -----

/// Charts for the Insights page: items over time (per status), current items per status, and
/// items per value of any field.
pub async fn insights(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, number)): Path<(String, i64)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let (_, p, _) = load(&s, &auth, &owner, number, Access::Read).await?;
    const DAY: i64 = 86_400_000;
    let now = reef_core::time::now_ms();
    let today = now - now.rem_euclid(DAY);
    let days: i64 = match q.get("range").map(String::as_str) {
        Some("2w") => 14,
        Some("1m") => 30,
        Some("6m") => 180,
        Some("1y") => 365,
        _ => 90,
    };
    let since = today - (days - 1) * DAY;
    let fields = s.db.fields(p.id()).await?;
    let status_field = fields.iter().find(|f| f.data_type == "single_select" && f.name.eq_ignore_ascii_case("Status")).cloned();
    let items = s.db.items(p.id(), true).await?;
    let values = s.db.project_values(p.id()).await?;
    let history = s.db.item_history(p.id(), 0).await?;
    let current_status = |item: i64| -> Option<String> {
        status_field.as_ref().and_then(|f| values.iter().find(|(i, fid, _)| *i == item && *fid == f.id).and_then(|(_, _, v)| v.clone()))
    };
    let status_at = |item: &Item, at: i64| -> Option<String> {
        let Some(f) = &status_field else { return None };
        let rows: Vec<&reef_db::projects::ItemHistory> =
            history.iter().filter(|h| h.item_id == item.id && h.kind == "value" && h.field_id == Some(f.id)).collect();
        match rows.iter().rev().find(|h| h.created_at <= at) {
            Some(h) => h.value.clone(),
            None => match rows.first() {
                Some(first) => first.prev_value.clone(),
                None => current_status(item.id),
            },
        }
    };
    let archived_at = |item: &Item, at: i64| -> bool {
        let rows: Vec<&reef_db::projects::ItemHistory> =
            history.iter().filter(|h| h.item_id == item.id && (h.kind == "archived" || h.kind == "unarchived")).collect();
        match rows.iter().rev().find(|h| h.created_at <= at) {
            Some(h) => h.kind == "archived",
            None => match rows.first() {
                Some(first) => first.kind == "unarchived",
                None => item.archived != 0,
            },
        }
    };
    let opts = status_field.as_ref().map(|f| f.options()).unwrap_or_default();
    let mut series: Vec<Value> = Vec::new();
    let mut d = since;
    while d <= today {
        let end = d + DAY - 1;
        let mut counts: Vec<i64> = vec![0; opts.len() + 1];
        for it in &items {
            if it.created_at > end || archived_at(it, end) {
                continue;
            }
            let st = status_at(it, end);
            let idx = st.and_then(|id| opts.iter().position(|o| o.id == id)).map(|i| i + 1).unwrap_or(0);
            counts[idx] += 1;
        }
        series.push(json!({"date": iso(d)[..10], "counts": counts}));
        d += DAY;
    }
    let mut status_now: Vec<i64> = vec![0; opts.len() + 1];
    for it in items.iter().filter(|i| i.archived == 0) {
        let idx = current_status(it.id).and_then(|id| opts.iter().position(|o| o.id == id)).map(|i| i + 1).unwrap_or(0);
        status_now[idx] += 1;
    }
    let buckets: Vec<Value> = std::iter::once(json!({"name": "No status", "color": "GRAY", "count": status_now[0]}))
        .chain(opts.iter().enumerate().map(|(i, o)| json!({"name": o.name, "color": o.color, "count": status_now[i + 1]})))
        .collect();
    // per-field chart
    let field_key = q.get("field").cloned().unwrap_or_default();
    let mut field_chart = Value::Null;
    if !field_key.is_empty() {
        let mut counts: Vec<(String, String, i64)> = Vec::new();
        let mut bump = |name: String, color: &str| {
            if let Some(c) = counts.iter_mut().find(|c| c.0 == name) {
                c.2 += 1;
            } else {
                counts.push((name, color.to_string(), 1));
            }
        };
        let custom = field_key.parse::<i64>().ok().and_then(|id| fields.iter().find(|f| f.id == id));
        let mut name = field_key.clone();
        for it in items.iter().filter(|i| i.archived == 0) {
            if let Some(f) = custom {
                name = f.name.clone();
                let v = values.iter().find(|(i, fid, _)| *i == it.id && *fid == f.id).and_then(|(_, _, v)| v.clone());
                match (f.data_type.as_str(), v) {
                    ("single_select", v) => match v.and_then(|id| f.options().into_iter().find(|o| o.id == id)) {
                        Some(o) => bump(o.name, &o.color),
                        None => bump(format!("No {}", f.name), "GRAY"),
                    },
                    ("date", Some(v)) => bump(v.chars().take(7).collect(), "BLUE"),
                    (_, Some(v)) => bump(v, "BLUE"),
                    (_, None) => bump(format!("No {}", f.name), "GRAY"),
                }
            } else {
                let facts = facts(&s, &p, it, status_field.as_ref()).await;
                match field_key.as_str() {
                    "assignees" => {
                        name = "Assignees".into();
                        if facts.assignees.is_empty() {
                            bump("Unassigned".into(), "GRAY");
                        }
                        for a in facts.assignees {
                            bump(a, "BLUE");
                        }
                    }
                    "labels" => {
                        name = "Labels".into();
                        if facts.labels.is_empty() {
                            bump("No labels".into(), "GRAY");
                        }
                        for l in facts.labels {
                            bump(l, "BLUE");
                        }
                    }
                    "repository" => {
                        name = "Repository".into();
                        bump(if facts.repository.is_empty() { "Draft".into() } else { facts.repository }, "BLUE");
                    }
                    "type" => {
                        name = "Type".into();
                        bump(
                            if facts.is_draft {
                                "Draft"
                            } else if facts.is_pull {
                                "Pull request"
                            } else {
                                "Issue"
                            }
                            .into(),
                            "BLUE",
                        );
                    }
                    _ => {
                        name = "State".into();
                        bump(
                            if facts.merged {
                                "Merged"
                            } else if facts.open {
                                "Open"
                            } else {
                                "Closed"
                            }
                            .into(),
                            if facts.merged {
                                "PURPLE"
                            } else if facts.open {
                                "GREEN"
                            } else {
                                "RED"
                            },
                        );
                    }
                }
            }
        }
        counts.sort_by_key(|c| std::cmp::Reverse(c.2));
        field_chart =
            json!({"name": name, "buckets": counts.iter().map(|(n, c, k)| json!({"name": n, "color": c, "count": k})).collect::<Vec<_>>()});
    }
    let total_open = items.iter().filter(|i| i.archived == 0).count();
    Ok(Json(json!({
        "since": iso(since)[..10], "until": iso(today)[..10], "days": days,
        "status_options": std::iter::once(json!({"id": null, "name": "No status", "color": "GRAY"})).chain(opts.iter().map(|o| json!({"id": o.id, "name": o.name, "color": o.color}))).collect::<Vec<_>>(),
        "burnup": series,
        "status": {"field": status_field.as_ref().map(field_json), "buckets": buckets, "total": total_open},
        "field": field_chart,
        "fields": fields.iter().map(|f| json!({"key": f.id.to_string(), "name": f.name})).chain([("assignees", "Assignees"), ("labels", "Labels"), ("repository", "Repository"), ("type", "Type"), ("state", "State")].iter().map(|(k, n)| json!({"key": k, "name": n}))).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts_at(updated_days_ago: i64, open: bool, now: i64) -> ItemFacts {
        ItemFacts {
            title: "Fix login".into(),
            repository: "acme/app".into(),
            number: 7,
            is_pull: false,
            is_draft: false,
            open,
            merged: false,
            labels: vec!["bug".into()],
            assignees: vec![],
            created_at: now - 40 * 86_400_000,
            updated_at: now - updated_days_ago * 86_400_000,
            status: Some("Done".into()),
        }
    }

    #[test]
    fn auto_archive_filter_matches_stale_closed_items() {
        let now: i64 = 1_790_000_000_000;
        assert!(item_matches("is:closed updated:<@today-2w", &facts_at(20, false, now), now));
        assert!(!item_matches("is:closed updated:<@today-2w", &facts_at(3, false, now), now));
        assert!(!item_matches("is:closed updated:<@today-2w", &facts_at(20, true, now), now));
        assert!(item_matches("label:bug status:Done -is:pr", &facts_at(1, true, now), now));
        assert!(!item_matches("-label:bug", &facts_at(1, true, now), now));
        assert!(item_matches("\"fix login\" repo:app", &facts_at(1, true, now), now));
        assert!(item_matches("no:assignee is:issue,pr", &facts_at(1, true, now), now));
    }

    #[test]
    fn date_expressions() {
        let now: i64 = 1_790_000_000_000;
        let today = now - now.rem_euclid(86_400_000);
        assert_eq!(parse_date_expr("@today", now), Some(today));
        assert_eq!(parse_date_expr("@today-1w", now), Some(today - 7 * 86_400_000));
        assert_eq!(parse_date_expr("2026-01-01", now), reef_core::time::parse_iso("2026-01-01T00:00:00Z"));
        assert!(date_cmp(">2026-01-01", now, now));
        assert!(!date_cmp("<2026-01-01", now, now));
    }
}
