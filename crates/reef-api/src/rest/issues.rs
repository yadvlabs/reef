//! Issues, comments, timeline, assignees, and reactions.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::rest::Page;
use crate::rest::repos::load;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::issues::{Comment, IssueFilter, IssueFull, IssuePatch, NewIssue};
use reef_db::repos::{Permission, RepoWithOwner};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Does the caller want `body_html` (GitHub's `html+json` / `full+json` media types)? Reef
/// also honours `?html=1` for the web UI.
fn wants_html(headers: &HeaderMap) -> bool {
    headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).map(|a| a.contains("html") || a.contains("full")).unwrap_or(false)
}

pub async fn association(s: &AppState, rw: &RepoWithOwner, user: &Account) -> &'static str {
    let perm = s.db.permission(Some(user), rw).await.unwrap_or(Permission::None);
    let is_owner = user.id == rw.owner.id;
    let is_member = rw.owner.is_org() && s.db.org_membership(rw.owner.id(), user.id()).await.ok().flatten().is_some();
    shapes::author_association(perm, is_owner, is_member)
}

/// Render many issues, with reactions and milestone counts resolved in bulk.
pub async fn issues_json(
    s: &AppState,
    rw: &RepoWithOwner,
    items: &[IssueFull],
    viewer: Option<&Account>,
    html: bool,
) -> ApiResult<Vec<Value>> {
    let ids: Vec<i64> = items.iter().map(|i| i.issue.id).collect();
    let (counts, mine) = s.db.reaction_summary("issue", &ids, viewer.map(|v| v.id)).await?;
    let mut ms_counts: HashMap<i64, (i64, i64)> = HashMap::new();
    let mut assoc: HashMap<i64, &'static str> = HashMap::new();
    let mut out = Vec::with_capacity(items.len());
    for i in items {
        if let Some(m) = &i.milestone
            && !ms_counts.contains_key(&m.id)
        {
            ms_counts.insert(m.id, s.db.milestone_counts(Id(m.id)).await?);
        }
        if let std::collections::hash_map::Entry::Vacant(slot) = assoc.entry(i.author.id) {
            slot.insert(association(s, rw, &i.author).await);
        }
        let reactions = shapes::reactions_json(
            format!("{}/repos/{}/issues/{}/reactions", s.config.api_url(), rw.full_name(), i.issue.number),
            &counts,
            &mine,
            i.issue.id,
        );
        let body_html =
            if html { Some(crate::render::markdown_in_repo(i.issue.body.as_deref().unwrap_or(""), &s.config, rw)) } else { None };
        let mut v = shapes::issue(
            &s.config,
            rw,
            i,
            assoc[&i.author.id],
            reactions,
            i.milestone.as_ref().map(|m| ms_counts[&m.id]).unwrap_or((0, 0)),
            body_html,
        );
        if i.issue.is_pull() {
            decorate_pull(s, rw, i, &mut v).await;
        }
        out.push(v);
    }
    Ok(out)
}

/// Fill in the pull-request bits of an issue row that list views need: `merged_at`, `draft`,
/// the review decision and the combined state of checks on the head commit.
async fn decorate_pull(s: &AppState, rw: &RepoWithOwner, i: &IssueFull, v: &mut Value) {
    let Ok(m) = s.db.pull_meta(i.issue.id()).await else { return };
    let o = v.as_object_mut().unwrap();
    if let Some(pr) = o.get_mut("pull_request").and_then(|p| p.as_object_mut()) {
        pr.insert("merged_at".into(), json!(reef_core::time::iso_opt(m.merged_at)));
    }
    o.insert("draft".into(), json!(m.draft != 0));
    let latest = s.db.latest_reviews(i.issue.id()).await.unwrap_or_default();
    let decision = if latest.iter().any(|r| r.state == "changes_requested") {
        "CHANGES_REQUESTED"
    } else if latest.iter().any(|r| r.state == "approved") {
        "APPROVED"
    } else {
        "REVIEW_REQUIRED"
    };
    o.insert("review_decision".into(), json!(decision));
    let mut states: Vec<String> =
        s.db.latest_statuses(rw.repo.id(), &m.head_sha).await.unwrap_or_default().into_iter().map(|st| st.state).collect();
    for j in s.db.jobs_for_sha(rw.repo.id(), &m.head_sha).await.unwrap_or_default() {
        states.push(match (j.status.as_str(), j.conclusion.as_deref()) {
            ("completed", Some("success")) => "success".into(),
            ("completed", Some("skipped")) | ("completed", Some("neutral")) => "success".into(),
            ("completed", _) => "failure".into(),
            _ => "pending".into(),
        });
    }
    let checks = if states.is_empty() {
        Value::Null
    } else if states.iter().any(|x| x == "failure" || x == "error") {
        json!("failure")
    } else if states.iter().any(|x| x == "pending") {
        json!("pending")
    } else {
        json!("success")
    };
    o.insert("checks_state".into(), checks);
}

pub async fn issue_json(s: &AppState, rw: &RepoWithOwner, i: &IssueFull, viewer: Option<&Account>, html: bool) -> ApiResult<Value> {
    Ok(issues_json(s, rw, std::slice::from_ref(i), viewer, html).await?.pop().unwrap())
}

#[derive(serde::Deserialize, Default)]
pub struct ListQuery {
    milestone: Option<String>,
    state: Option<String>,
    assignee: Option<String>,
    creator: Option<String>,
    mentioned: Option<String>,
    labels: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
    since: Option<String>,
    q: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

impl ListQuery {
    fn filter(&self) -> IssueFilter {
        IssueFilter {
            state: self.state.clone().unwrap_or_else(|| "open".into()),
            is_pull: match self.kind.as_deref() {
                Some("issue") => Some(false),
                Some("pr") | Some("pull") => Some(true),
                _ => None,
            },
            labels: self
                .labels
                .as_deref()
                .map(|l| l.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                .unwrap_or_default(),
            assignee: self.assignee.clone(),
            creator: self.creator.clone(),
            mentioned: self.mentioned.clone(),
            milestone: self.milestone.clone(),
            since: self.since.as_deref().and_then(reef_core::time::parse_iso),
            sort: self.sort.clone().unwrap_or_else(|| "created".into()),
            ascending: self.direction.as_deref() == Some("asc"),
            terms: self.q.as_deref().map(|q| q.split_whitespace().map(String::from).collect()).unwrap_or_default(),
            ..Default::default()
        }
    }
}

pub async fn list(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
    Query(q): Query<ListQuery>,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (p, per) = page.clamp();
    let (items, total) = s.db.list_issues(Some(rw.repo.id()), &q.filter(), p, per).await?;
    let out = issues_json(&s, &rw, &items, auth.user.as_ref(), wants_html(&headers)).await?;
    let mut h = HeaderMap::new();
    h.insert("x-total-count", total.to_string().parse().unwrap());
    if (p * per) < total {
        h.insert(
            "link",
            format!("<{}/repos/{}/issues?page={}&per_page={per}>; rel=\"next\"", s.config.api_url(), rw.full_name(), p + 1)
                .parse()
                .unwrap(),
        );
    }
    Ok((h, Json(json!(out))))
}

#[derive(serde::Deserialize)]
pub struct CreateIssue {
    title: String,
    body: Option<String>,
    #[serde(default)]
    labels: Vec<Value>,
    #[serde(default)]
    assignees: Vec<String>,
    assignee: Option<String>,
    milestone: Option<Value>,
}

fn label_names(v: &[Value]) -> Vec<String> {
    v.iter().filter_map(|l| l.as_str().map(String::from).or_else(|| l.get("name").and_then(|n| n.as_str()).map(String::from))).collect()
}

async fn milestone_id(s: &AppState, rw: &RepoWithOwner, v: Option<&Value>) -> ApiResult<Option<Option<i64>>> {
    match v {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::Number(n)) => {
            let m =
                s.db.milestone_by_number(rw.repo.id(), n.as_i64().unwrap_or(0))
                    .await
                    .map_err(|_| ApiError(Error::field("Issue", "milestone", "invalid", "milestone not found")))?;
            Ok(Some(Some(m.id)))
        }
        Some(Value::String(t)) => {
            let m = match t.parse::<i64>() {
                Ok(n) => s.db.milestone_by_number(rw.repo.id(), n).await,
                Err(_) => s.db.milestone_by_title(rw.repo.id(), t).await,
            }
            .map_err(|_| ApiError(Error::field("Issue", "milestone", "invalid", "milestone not found")))?;
            Ok(Some(Some(m.id)))
        }
        _ => Err(ApiError(Error::field("Issue", "milestone", "invalid", "milestone is invalid"))),
    }
}

pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<CreateIssue>,
) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.has_issues == 0 {
        return Err(ApiError(Error::Forbidden("Issues are disabled for this repo".into())));
    }
    if rw.repo.archived != 0 {
        return Err(crate::error::forbidden("Repository was archived so is read-only."));
    }
    let can_triage = perm >= Permission::Triage;
    let mut assignees = b.assignees.clone();
    if let Some(a) = b.assignee {
        assignees.push(a);
    }
    let n = NewIssue {
        title: b.title,
        body: b.body,
        is_pull: false,
        labels: if can_triage { label_names(&b.labels) } else { vec![] },
        assignees: if can_triage { assignees } else { vec![] },
        milestone: if can_triage { milestone_id(&s, &rw, b.milestone.as_ref()).await?.flatten() } else { None },
    };
    let i = s.db.create_issue(rw.repo.id(), u, n).await?;
    s.db.record_event(rw.repo.id(), Some(u.id()), "issues.opened", &json!({"number": i.issue.number})).await?;
    Ok((StatusCode::CREATED, Json(issue_json(&s, &rw, &i, Some(u), wants_html(&headers)).await?)))
}

/// Load an issue by number; pull requests are issues too (GitHub serves both here).
pub async fn load_issue(s: &AppState, rw: &RepoWithOwner, number: i64) -> ApiResult<IssueFull> {
    Ok(s.db.issue_full_by_number(rw.repo.id(), number).await?)
}

pub async fn show(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    if let (Some(u), true) = (&auth.user, wants_html(&headers)) {
        // Opening the page in the web UI reads the thread.
        let _ = s.db.mark_subject_read(u.id(), if i.issue.is_pull() { "pull" } else { "issue" }, i.issue.id).await;
    }
    Ok(Json(issue_json(&s, &rw, &i, auth.user.as_ref(), wants_html(&headers)).await?))
}

#[derive(serde::Deserialize, Default)]
pub struct UpdateIssue {
    title: Option<String>,
    body: Option<Option<String>>,
    state: Option<String>,
    state_reason: Option<Option<String>>,
    milestone: Option<Value>,
    labels: Option<Vec<Value>>,
    assignees: Option<Vec<String>>,
    assignee: Option<Option<String>>,
}

/// Can `user` change this issue? Triage permission, or the issue's author for title/body/close.
fn can_edit(perm: Permission, user: &Account, i: &IssueFull, metadata: bool) -> bool {
    if perm >= Permission::Triage {
        return true;
    }
    !metadata && user.id == i.author.id
}

pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<UpdateIssue>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    let metadata = b.milestone.is_some() || b.labels.is_some() || b.assignees.is_some() || b.assignee.is_some();
    if !can_edit(perm, u, &i, metadata) {
        return Err(crate::error::forbidden("You do not have permission to update this issue"));
    }
    if i.issue.locked != 0 && perm < Permission::Triage {
        return Err(crate::error::forbidden("This issue is locked"));
    }
    let mut assignees = b.assignees;
    if let Some(a) = b.assignee {
        assignees = Some(a.into_iter().collect());
    }
    let patch = IssuePatch {
        title: b.title,
        body: b.body,
        state: b.state.filter(|st| st == "open" || st == "closed"),
        state_reason: b.state_reason,
        milestone: milestone_id(&s, &rw, b.milestone.as_ref()).await?,
        labels: b.labels.as_deref().map(label_names),
        assignees,
        close_commit: None,
        close_pull: None,
    };
    let updated = s.db.update_issue(i.issue.id(), u, patch).await?;
    Ok(Json(issue_json(&s, &rw, &updated, Some(u), wants_html(&headers)).await?))
}

#[derive(serde::Deserialize, Default)]
pub struct LockBody {
    lock_reason: Option<String>,
}

pub async fn lock(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    body: Option<Json<LockBody>>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Triage {
        return Err(crate::error::forbidden("Must have triage access to lock"));
    }
    let i = load_issue(&s, &rw, number).await?;
    let reason = body.and_then(|b| b.0.lock_reason).filter(|r| matches!(r.as_str(), "off-topic" | "too heated" | "resolved" | "spam"));
    s.db.set_issue_lock(i.issue.id(), u, true, reason.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unlock(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if perm < Permission::Triage {
        return Err(crate::error::forbidden("Must have triage access to unlock"));
    }
    let i = load_issue(&s, &rw, number).await?;
    s.db.set_issue_lock(i.issue.id(), u, false, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin access to delete issues"));
    }
    let i = load_issue(&s, &rw, number).await?;
    s.db.delete_issue(i.issue.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- comments -----

pub async fn comments_json(
    s: &AppState,
    rw: &RepoWithOwner,
    issue_number: i64,
    comments: &[Comment],
    viewer: Option<&Account>,
    html: bool,
) -> ApiResult<Vec<Value>> {
    let ids: Vec<i64> = comments.iter().map(|c| c.id).collect();
    let (counts, mine) = s.db.reaction_summary("issue_comment", &ids, viewer.map(|v| v.id)).await?;
    let mut author_ids: Vec<i64> = comments.iter().map(|c| c.author_id).collect();
    author_ids.sort_unstable();
    author_ids.dedup();
    let authors: HashMap<i64, Account> = s.db.accounts_by_ids(&author_ids).await?.into_iter().map(|a| (a.id, a)).collect();
    let mut assoc: HashMap<i64, &'static str> = HashMap::new();
    let mut out = Vec::new();
    for c in comments {
        let Some(author) = authors.get(&c.author_id) else { continue };
        if let std::collections::hash_map::Entry::Vacant(slot) = assoc.entry(author.id) {
            slot.insert(association(s, rw, author).await);
        }
        let reactions = shapes::reactions_json(
            format!("{}/repos/{}/issues/comments/{}/reactions", s.config.api_url(), rw.full_name(), c.id),
            &counts,
            &mine,
            c.id,
        );
        let body_html = if html { Some(crate::render::markdown_in_repo(&c.body, &s.config, rw)) } else { None };
        out.push(shapes::comment(&s.config, rw, c, author, issue_number, assoc[&author.id], reactions, body_html));
    }
    Ok(out)
}

#[derive(serde::Deserialize, Default)]
pub struct SinceQuery {
    since: Option<String>,
}

pub async fn list_comments(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    page: Page,
    Query(q): Query<SinceQuery>,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    let all = s.db.comments(i.issue.id(), q.since.as_deref().and_then(reef_core::time::parse_iso)).await?;
    let out = comments_json(&s, &rw, number, &all, auth.user.as_ref(), wants_html(&headers)).await?;
    let (items, h) = page.slice(&out, &format!("{}/repos/{}/issues/{number}/comments", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

pub async fn list_repo_comments(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    page: Page,
    Query(q): Query<SinceQuery>,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let all = s.db.repo_comments(rw.repo.id(), q.since.as_deref().and_then(reef_core::time::parse_iso)).await?;
    let mut out = Vec::new();
    for c in &all {
        let issue = s.db.issue_by_id(Id(c.issue_id)).await?;
        out.extend(comments_json(&s, &rw, issue.number, std::slice::from_ref(c), auth.user.as_ref(), wants_html(&headers)).await?);
    }
    let (items, h) = page.slice(&out, &format!("{}/repos/{}/issues/comments", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

#[derive(serde::Deserialize)]
pub struct CommentBody {
    body: String,
}

pub async fn create_comment(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<CommentBody>,
) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    if i.issue.locked != 0 && perm < Permission::Triage {
        return Err(crate::error::forbidden("This issue is locked"));
    }
    if rw.repo.archived != 0 {
        return Err(crate::error::forbidden("Repository was archived so is read-only."));
    }
    let c = s.db.create_comment(i.issue.id(), u, &b.body).await?;
    let out = comments_json(&s, &rw, number, std::slice::from_ref(&c), Some(u), wants_html(&headers)).await?;
    Ok((StatusCode::CREATED, Json(out.into_iter().next().unwrap())))
}

async fn load_comment(s: &AppState, rw: &RepoWithOwner, id: i64) -> ApiResult<(Comment, i64)> {
    let c = s.db.comment_by_id(Id(id)).await?;
    if c.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let issue = s.db.issue_by_id(Id(c.issue_id)).await?;
    Ok((c, issue.number))
}

pub async fn show_comment(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (c, number) = load_comment(&s, &rw, id).await?;
    let out = comments_json(&s, &rw, number, std::slice::from_ref(&c), auth.user.as_ref(), wants_html(&headers)).await?;
    Ok(Json(out.into_iter().next().unwrap()))
}

pub async fn update_comment(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Json(b): Json<CommentBody>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (c, number) = load_comment(&s, &rw, id).await?;
    if c.author_id != u.id && perm < Permission::Write {
        return Err(crate::error::forbidden("You cannot edit this comment"));
    }
    let c = s.db.update_comment(Id(id), &b.body).await?;
    let out = comments_json(&s, &rw, number, std::slice::from_ref(&c), Some(u), wants_html(&headers)).await?;
    Ok(Json(out.into_iter().next().unwrap()))
}

pub async fn delete_comment(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (c, _) = load_comment(&s, &rw, id).await?;
    if c.author_id != u.id && perm < Permission::Write {
        return Err(crate::error::forbidden("You cannot delete this comment"));
    }
    s.db.delete_comment(Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- events and timeline -----

pub async fn events_json(s: &AppState, rw: &RepoWithOwner, events: &[reef_db::issues::IssueEvent]) -> ApiResult<Vec<Value>> {
    let mut ids: Vec<i64> = events.iter().filter_map(|e| e.actor_id).collect();
    for e in events {
        if let Some(a) = e.payload().get("assignee_id").and_then(|v| v.as_i64()) {
            ids.push(a);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    let accounts: HashMap<i64, Account> = s.db.accounts_by_ids(&ids).await?.into_iter().map(|a| (a.id, a)).collect();
    let mut out = Vec::new();
    for e in events {
        if e.event == "mentioned" {
            continue;
        }
        let p = e.payload();
        let mut extra = json!({});
        match e.event.as_str() {
            "labeled" | "unlabeled" => extra = json!({"label": p.get("label").cloned().unwrap_or(Value::Null)}),
            "assigned" | "unassigned" => {
                let a = p.get("assignee_id").and_then(|v| v.as_i64()).and_then(|id| accounts.get(&id));
                extra = json!({"assignee": a.map(|a| shapes::simple_user(&s.config, a)), "assigner": e.actor_id.and_then(|id| accounts.get(&id)).map(|a| shapes::simple_user(&s.config, a))});
            }
            "milestoned" | "demilestoned" => {
                let title = match p.get("milestone_id").and_then(|v| v.as_i64()) {
                    Some(id) => s.db.milestone_by_id(Id(id)).await.ok().map(|m| m.title),
                    None => None,
                };
                extra = json!({"milestone": {"title": title}});
            }
            "renamed" => extra = json!({"rename": {"from": p.get("from"), "to": p.get("to")}}),
            "closed" => {
                let commit = p.get("commit_id").and_then(|v| v.as_str()).map(str::to_string);
                let pull = p.get("closing_pull").and_then(|v| v.as_i64());
                let closing = match pull {
                    Some(n) => s.db.issue_by_number(rw.repo.id(), n).await.ok().map(|i| json!({"number": i.number, "title": i.title, "state": i.state, "is_pull": i.is_pull(), "html_url": format!("{}/{}/pull/{}", s.config.site_url(), rw.full_name(), i.number)})),
                    None => None,
                };
                extra = json!({
                    "state_reason": p.get("state_reason"),
                    "commit_id": commit,
                    "commit_url": commit.as_ref().map(|c| format!("{}/repos/{}/commits/{c}", s.config.api_url(), rw.full_name())),
                    "closing_pull": closing,
                });
            }
            "merged" => {
                let commit = p.get("commit_id").and_then(|v| v.as_str()).map(str::to_string);
                extra = json!({
                    "commit_id": commit,
                    "commit_url": commit.as_ref().map(|c| format!("{}/repos/{}/commits/{c}", s.config.api_url(), rw.full_name())),
                    "merge_method": p.get("method"),
                    "base_ref": p.get("base_ref"),
                    "head_ref": p.get("head_ref"),
                });
            }
            "cross-referenced" => {
                let src = match p.get("source_number").and_then(|v| v.as_i64()) {
                    Some(n) => s.db.issue_by_number(rw.repo.id(), n).await.ok(),
                    None => None,
                };
                let merged = match &src {
                    Some(i) if i.is_pull() => s.db.pull_meta(i.id()).await.ok().map(|m| m.merged != 0).unwrap_or(false),
                    _ => false,
                };
                extra = json!({
                    "source": src.map(|i| json!({"type": "issue", "issue": {"number": i.number, "title": i.title, "state": i.state, "merged": merged, "pull_request": if i.is_pull() { json!({"merged": merged}) } else { Value::Null }, "html_url": format!("{}/{}/{}/{}", s.config.site_url(), rw.full_name(), if i.is_pull() { "pull" } else { "issues" }, i.number)}})),
                    "will_close": p.get("will_close").and_then(|v| v.as_bool()).unwrap_or(false),
                });
            }
            "locked" | "unlocked" => extra = json!({"lock_reason": p.get("lock_reason")}),
            _ => {}
        }
        out.push(shapes::issue_event(&s.config, rw, e, e.actor_id.and_then(|id| accounts.get(&id)), extra));
    }
    Ok(out)
}

pub async fn events(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    let ev = s.db.issue_events(i.issue.id()).await?;
    let out = events_json(&s, &rw, &ev).await?;
    let (items, h) = page.slice(&out, &format!("{}/repos/{}/issues/{number}/events", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

/// Events and comments interleaved by time, as GitHub's timeline API does.
pub async fn timeline(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    let ev = s.db.issue_events(i.issue.id()).await?;
    let cm = s.db.comments(i.issue.id(), None).await?;
    let mut items: Vec<(i64, Value)> = Vec::new();
    for v in events_json(&s, &rw, &ev).await? {
        let ts = v.get("created_at").and_then(|c| c.as_str()).and_then(reef_core::time::parse_iso).unwrap_or(0);
        items.push((ts, v));
    }
    for mut v in comments_json(&s, &rw, number, &cm, auth.user.as_ref(), wants_html(&headers)).await? {
        let ts = v.get("created_at").and_then(|c| c.as_str()).and_then(reef_core::time::parse_iso).unwrap_or(0);
        v.as_object_mut().unwrap().insert("event".into(), json!("commented"));
        let actor = v.get("user").cloned().unwrap_or(Value::Null);
        v.as_object_mut().unwrap().insert("actor".into(), actor);
        items.push((ts, v));
    }
    items.sort_by_key(|(t, v)| (*t, v.get("id").and_then(|i| i.as_i64()).unwrap_or(0)));
    let out: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
    let (page_items, h) = page.slice(&out, &format!("{}/repos/{}/issues/{number}/timeline", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(page_items))))
}

// ----- labels on an issue -----

pub async fn issue_labels(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    Ok(Json(json!(i.labels.iter().map(|l| shapes::label(&s.config, &rw, l)).collect::<Vec<_>>())))
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum LabelsBody {
    Wrapped { labels: Vec<Value> },
    List(Vec<Value>),
}

impl LabelsBody {
    fn names(&self) -> Vec<String> {
        match self {
            LabelsBody::Wrapped { labels } => label_names(labels),
            LabelsBody::List(l) => label_names(l),
        }
    }
}

async fn triage_issue(s: &AppState, auth: &Auth, owner: &str, repo: &str, number: i64) -> ApiResult<(RepoWithOwner, IssueFull, Account)> {
    let u = auth.require()?.clone();
    let (rw, perm) = load(s, auth, owner, repo).await?;
    if perm < Permission::Triage {
        return Err(crate::error::forbidden("Must have triage access"));
    }
    let i = load_issue(s, &rw, number).await?;
    Ok((rw, i, u))
}

pub async fn add_labels(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<LabelsBody>,
) -> ApiResult<Json<Value>> {
    let (rw, i, u) = triage_issue(&s, &auth, &owner, &repo, number).await?;
    let labels = s.db.add_issue_labels(i.issue.id(), rw.repo.id(), &u, &b.names()).await?;
    Ok(Json(json!(labels.iter().map(|l| shapes::label(&s.config, &rw, l)).collect::<Vec<_>>())))
}

pub async fn set_labels(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    body: Option<Json<LabelsBody>>,
) -> ApiResult<Json<Value>> {
    let (rw, i, u) = triage_issue(&s, &auth, &owner, &repo, number).await?;
    let names = body.map(|b| b.0.names()).unwrap_or_default();
    let labels = s.db.set_issue_labels(i.issue.id(), rw.repo.id(), &u, &names).await?;
    Ok(Json(json!(labels.iter().map(|l| shapes::label(&s.config, &rw, l)).collect::<Vec<_>>())))
}

pub async fn clear_labels(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let (rw, i, u) = triage_issue(&s, &auth, &owner, &repo, number).await?;
    s.db.set_issue_labels(i.issue.id(), rw.repo.id(), &u, &[]).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_label(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number, name)): Path<(String, String, i64, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, i, u) = triage_issue(&s, &auth, &owner, &repo, number).await?;
    let labels = s.db.remove_issue_label(i.issue.id(), rw.repo.id(), &u, &name).await?;
    Ok(Json(json!(labels.iter().map(|l| shapes::label(&s.config, &rw, l)).collect::<Vec<_>>())))
}

// ----- assignees -----

#[derive(serde::Deserialize, Default)]
pub struct AssigneesBody {
    #[serde(default)]
    assignees: Vec<String>,
}

pub async fn add_assignees(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<AssigneesBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let (rw, i, u) = triage_issue(&s, &auth, &owner, &repo, number).await?;
    s.db.add_issue_assignees(i.issue.id(), rw.repo.id(), &u, &b.assignees).await?;
    let i = s.db.issue_full_by_id(i.issue.id()).await?;
    Ok((StatusCode::CREATED, Json(issue_json(&s, &rw, &i, Some(&u), wants_html(&headers)).await?)))
}

pub async fn remove_assignees(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<AssigneesBody>,
) -> ApiResult<Json<Value>> {
    let (rw, i, u) = triage_issue(&s, &auth, &owner, &repo, number).await?;
    s.db.remove_issue_assignees(i.issue.id(), rw.repo.id(), &u, &b.assignees).await?;
    let i = s.db.issue_full_by_id(i.issue.id()).await?;
    Ok(Json(issue_json(&s, &rw, &i, Some(&u), wants_html(&headers)).await?))
}

pub async fn assignable_accounts(s: &AppState, rw: &RepoWithOwner) -> ApiResult<Vec<Account>> {
    let mut users: Vec<Account> = Vec::new();
    if rw.owner.is_user() {
        users.push(rw.owner.clone());
    } else {
        users.extend(s.db.org_members(rw.owner.id(), None).await?.into_iter().map(|(a, _)| a));
    }
    for (a, _) in s.db.collaborators(rw.repo.id()).await? {
        if !users.iter().any(|u| u.id == a.id) {
            users.push(a);
        }
    }
    Ok(users)
}

pub async fn assignees(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let users = assignable_accounts(&s, &rw).await?;
    Ok(Json(json!(users.iter().map(|a| shapes::simple_user(&s.config, a)).collect::<Vec<_>>())))
}

pub async fn check_assignee(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, login)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let users = assignable_accounts(&s, &rw).await?;
    Ok(if users.iter().any(|u| u.login.eq_ignore_ascii_case(&login)) { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND })
}

// ----- reactions -----

fn reaction_json(s: &AppState, r: &reef_db::issues::Reaction, user: &Account) -> Value {
    json!({"id": r.id, "node_id": shapes::node_id("RE", r.id), "user": shapes::simple_user(&s.config, user), "content": r.content, "created_at": reef_core::time::iso(r.created_at)})
}

async fn reactions_list(s: &AppState, subject_type: &str, subject: Id) -> ApiResult<Vec<Value>> {
    let rs = s.db.reactions(subject_type, subject).await?;
    let ids: Vec<i64> = rs.iter().map(|r| r.user_id).collect();
    let users: HashMap<i64, Account> = s.db.accounts_by_ids(&ids).await?.into_iter().map(|a| (a.id, a)).collect();
    Ok(rs.iter().filter_map(|r| users.get(&r.user_id).map(|u| reaction_json(s, r, u))).collect())
}

#[derive(serde::Deserialize)]
pub struct ReactionBody {
    content: String,
}

pub async fn issue_reactions(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    Ok(Json(json!(reactions_list(&s, "issue", i.issue.id()).await?)))
}

pub async fn add_issue_reaction(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<ReactionBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let i = load_issue(&s, &rw, number).await?;
    let r = s.db.add_reaction("issue", i.issue.id(), u.id(), &b.content).await?;
    Ok((StatusCode::CREATED, Json(reaction_json(&s, &r, u))))
}

pub async fn delete_issue_reaction(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, _number, id)): Path<(String, String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    load(&s, &auth, &owner, &repo).await?;
    s.db.delete_reaction(Id(id), u.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn comment_reactions(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    load_comment(&s, &rw, id).await?;
    Ok(Json(json!(reactions_list(&s, "issue_comment", Id(id)).await?)))
}

pub async fn add_comment_reaction(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Json(b): Json<ReactionBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    load_comment(&s, &rw, id).await?;
    let r = s.db.add_reaction("issue_comment", Id(id), u.id(), &b.content).await?;
    Ok((StatusCode::CREATED, Json(reaction_json(&s, &r, u))))
}

pub async fn delete_comment_reaction(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, _cid, id)): Path<(String, String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    load(&s, &auth, &owner, &repo).await?;
    s.db.delete_reaction(Id(id), u.id()).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- cross-repository listings -----

#[derive(serde::Deserialize, Default)]
pub struct UserIssuesQuery {
    filter: Option<String>,
    state: Option<String>,
    labels: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
}

/// `GET /issues` and `GET /user/issues`: the viewer's issues across repositories they can see.
pub async fn user_issues(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    page: Page,
    Query(q): Query<UserIssuesQuery>,
) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let mut f = IssueFilter {
        state: q.state.clone().unwrap_or_else(|| "open".into()),
        labels: q.labels.as_deref().map(|l| l.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default(),
        sort: q.sort.clone().unwrap_or_else(|| "created".into()),
        ascending: q.direction.as_deref() == Some("asc"),
        ..Default::default()
    };
    match q.filter.as_deref().unwrap_or("assigned") {
        "created" => f.creator = Some(u.login.clone()),
        "mentioned" => f.mentioned = Some(u.login.clone()),
        "all" | "subscribed" | "repos" => {}
        _ => f.assignee = Some(u.login.clone()),
    }
    let (p, per) = page.clamp();
    let (items, _) = s.db.list_issues(None, &f, p, per * 4).await?;
    let mut out = Vec::new();
    let mut repos: HashMap<i64, Option<RepoWithOwner>> = HashMap::new();
    for i in items {
        let rw = match repos.get(&i.issue.repo_id) {
            Some(r) => r.clone(),
            None => {
                let r = match s.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await {
                    Ok(rw) => match s.db.permission(Some(u), &rw).await? {
                        p if p.can_read() => Some(rw),
                        _ => None,
                    },
                    Err(_) => None,
                };
                repos.insert(i.issue.repo_id, r.clone());
                r
            }
        };
        if let Some(rw) = rw {
            let mut v = issue_json(&s, &rw, &i, Some(u), wants_html(&headers)).await?;
            v.as_object_mut().unwrap().insert("repository".into(), shapes::repo(&s.config, &rw, None));
            out.push(v);
            if out.len() as i64 >= per {
                break;
            }
        }
    }
    Ok(Json(json!(out)))
}

/// `GET /search/issues`: qualifiers `repo:`, `is:`, `state:`, `type:`, `label:`, `author:`,
/// `assignee:`, `mentions:`, `milestone:`, `sort:`.
pub async fn search(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    page: Page,
    Query(q): Query<super::search::SearchQueryPublic>,
) -> ApiResult<Json<Value>> {
    let (terms, quals) = crate::graphql::query::parse_search(&q.q);
    let (repo, f) =
        filter_from_search(&s, &terms, &quals, q.sort.as_deref(), q.order.as_deref(), auth.user.as_ref().map(|u| u.login.as_str())).await?;
    let (p, per) = page.clamp();
    let (items, total) = s.db.list_issues(repo.as_ref().map(|r| r.repo.id()), &f, p, per).await?;
    let mut out = Vec::new();
    for i in items {
        let rw = match &repo {
            Some(r) => r.clone(),
            None => match s.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await {
                Ok(rw) if s.db.permission(auth.user.as_ref(), &rw).await?.can_read() => rw,
                _ => continue,
            },
        };
        let mut v = issue_json(&s, &rw, &i, auth.user.as_ref(), wants_html(&headers)).await?;
        v.as_object_mut().unwrap().insert("score".into(), json!(1.0));
        out.push(v);
    }
    Ok(Json(json!({"total_count": total, "incomplete_results": false, "items": out})))
}

/// Translate GitHub search qualifiers into an issue filter. Shared with GraphQL search.
pub async fn filter_from_search(
    s: &AppState,
    terms: &[String],
    quals: &[(String, String)],
    sort: Option<&str>,
    order: Option<&str>,
    viewer: Option<&str>,
) -> ApiResult<(Option<RepoWithOwner>, IssueFilter)> {
    let mut f = IssueFilter { state: "all".into(), terms: terms.to_vec(), sort: "created".into(), ..Default::default() };
    let mut repo = None;
    let me = |v: &str| if v == "@me" { viewer.unwrap_or("@me").to_string() } else { v.to_string() };
    for (k, v) in quals {
        let v = &me(v);
        match k.as_str() {
            "review-requested" | "user-review-requested" => f.review_requested = Some(v.clone()),
            "reviewed-by" => f.reviewed_by = Some(v.clone()),
            "involves" => f.involves = Some(v.clone()),
            "archived" | "org" | "user" | "in" | "created" | "updated" | "closed" | "merged" => {}
            "repo" => {
                if let Some((o, r)) = v.split_once('/') {
                    repo = s.db.repo_by_path(o, r).await.ok();
                    if repo.is_none() {
                        f.terms.push("\u{0}no-such-repo".into());
                    }
                }
            }
            "is" | "type" => match v.as_str() {
                "issue" => f.is_pull = Some(false),
                "pr" | "pull-request" => f.is_pull = Some(true),
                "open" => f.state = "open".into(),
                "closed" => f.state = "closed".into(),
                _ => {}
            },
            "state" => f.state = v.clone(),
            "label" => f.labels.push(v.clone()),
            "author" => f.creator = Some(v.clone()),
            "assignee" => f.assignee = Some(if v == "none" { "none".into() } else { v.clone() }),
            "mentions" => f.mentioned = Some(v.clone()),
            "milestone" => f.milestone = Some(v.clone()),
            "no" if v == "assignee" => f.assignee = Some("none".into()),
            "no" if v == "milestone" => f.milestone = Some("none".into()),
            "sort" => f.sort = v.split('-').next().unwrap_or("created").to_string(),
            _ => {}
        }
    }
    if let Some(sort) = sort {
        f.sort = match sort {
            "updated" => "updated".into(),
            "comments" => "comments".into(),
            _ => "created".into(),
        };
    }
    f.ascending = order == Some("asc");
    Ok((repo, f))
}

#[derive(serde::Deserialize)]
pub struct MarkdownBody {
    text: String,
    #[serde(default)]
    mode: Option<String>,
    context: Option<String>,
}

/// `POST /markdown`: render GitHub Flavored Markdown to HTML.
pub async fn markdown(State(s): State<AppState>, auth: Auth, Json(b): Json<MarkdownBody>) -> ApiResult<impl IntoResponse> {
    let html = match b.context.as_deref().and_then(|c| c.split_once('/')) {
        Some((o, r)) if b.mode.as_deref() == Some("gfm") => match load(&s, &auth, o, r).await {
            Ok((rw, _)) => crate::render::markdown_in_repo(&b.text, &s.config, &rw),
            Err(_) => crate::render::markdown(&b.text),
        },
        _ => crate::render::markdown(&b.text),
    };
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html))
}
