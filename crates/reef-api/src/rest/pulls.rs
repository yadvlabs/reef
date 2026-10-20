//! Pull requests: list, create, show, update, commits, files, merge, reviews, review
//! requests, compare, and commit statuses. Diff and patch media types are honoured.

use crate::AppState;
use crate::auth::Auth;
use crate::error::{ApiError, ApiResult};
use crate::pulls as svc;
use crate::rest::Page;
use crate::rest::issues::{issue_json, load_issue};
use crate::rest::repos::load;
use crate::shapes;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use reef_core::time::{iso, iso_opt};
use reef_core::{Error, Id};
use reef_db::accounts::Account;
use reef_db::issues::IssueFull;
use reef_db::pulls::PullMeta;
use reef_db::repos::{Permission, RepoWithOwner};
use reef_git::ops;
use serde_json::{Value, json};

fn media(headers: &HeaderMap) -> Option<&'static str> {
    let a = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("");
    if a.contains("vnd.github.diff") || a.contains("vnd.github.v3.diff") {
        Some("diff")
    } else if a.contains("vnd.github.patch") || a.contains("vnd.github.v3.patch") {
        Some("patch")
    } else {
        None
    }
}

fn wants_html(headers: &HeaderMap) -> bool {
    headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).map(|a| a.contains("html") || a.contains("full")).unwrap_or(false)
}

/// The REST `pull-request` shape, built on the issue shape.
pub async fn pull_json(
    s: &AppState,
    rw: &RepoWithOwner,
    i: &IssueFull,
    m: &PullMeta,
    viewer: Option<&Account>,
    html: bool,
) -> ApiResult<Value> {
    let mut v = issue_json(s, rw, i, viewer, html).await?;
    let o = v.as_object_mut().unwrap();
    let full = rw.full_name();
    let api = s.config.api_url();
    let site = s.config.site_url();
    let n = i.issue.number;
    let u = format!("{api}/repos/{full}/pulls/{n}");
    let repo_json = shapes::repo(&s.config, rw, None);
    let requested = s.db.review_requests(i.issue.id()).await?;
    let merged_by = match m.merged_by_id {
        Some(id) => s.db.account_by_id(Id(id)).await.ok(),
        None => None,
    };
    let reviews = s.db.reviews(i.issue.id()).await?;
    let review_comments = s.db.review_comments(i.issue.id()).await?;
    let mergeable_state = if m.draft != 0 {
        "draft"
    } else {
        match m.mergeable.as_str() {
            "mergeable" => "clean",
            "conflicting" => "dirty",
            _ => "unknown",
        }
    };
    let mut ins = |k: &str, val: Value| {
        o.insert(k.to_string(), val);
    };
    ins("url", json!(u));
    ins("html_url", json!(format!("{site}/{full}/pull/{n}")));
    ins("diff_url", json!(format!("{site}/{full}/pull/{n}.diff")));
    ins("patch_url", json!(format!("{site}/{full}/pull/{n}.patch")));
    ins("issue_url", json!(format!("{api}/repos/{full}/issues/{n}")));
    ins("commits_url", json!(format!("{u}/commits")));
    ins("review_comments_url", json!(format!("{u}/comments")));
    ins("review_comment_url", json!(format!("{api}/repos/{full}/pulls/comments{{/number}}")));
    ins("statuses_url", json!(format!("{api}/repos/{full}/statuses/{}", m.head_sha)));
    ins("merged_at", json!(iso_opt(m.merged_at)));
    ins("merge_commit_sha", json!(m.merge_commit_sha));
    ins("requested_reviewers", json!(requested.iter().map(|a| shapes::simple_user(&s.config, a)).collect::<Vec<_>>()));
    ins("requested_teams", json!([]));
    ins(
        "head",
        json!({"label": format!("{}:{}", rw.owner.login, m.head_ref), "ref": m.head_ref, "sha": m.head_sha, "user": shapes::simple_user(&s.config, &rw.owner), "repo": repo_json}),
    );
    ins(
        "base",
        json!({"label": format!("{}:{}", rw.owner.login, m.base_ref), "ref": m.base_ref, "sha": m.base_sha, "user": shapes::simple_user(&s.config, &rw.owner), "repo": repo_json}),
    );
    ins(
        "_links",
        json!({
            "self": {"href": u}, "html": {"href": format!("{site}/{full}/pull/{n}")}, "issue": {"href": format!("{api}/repos/{full}/issues/{n}")},
            "comments": {"href": format!("{api}/repos/{full}/issues/{n}/comments")}, "review_comments": {"href": format!("{u}/comments")},
            "review_comment": {"href": format!("{api}/repos/{full}/pulls/comments{{/number}}")}, "commits": {"href": format!("{u}/commits")}, "statuses": {"href": format!("{api}/repos/{full}/statuses/{}", m.head_sha)}
        }),
    );
    ins("auto_merge", Value::Null);
    ins("draft", json!(m.draft != 0));
    ins("merged", json!(m.merged != 0));
    ins(
        "mergeable",
        match m.mergeable.as_str() {
            "mergeable" => json!(true),
            "conflicting" => json!(false),
            _ => Value::Null,
        },
    );
    ins(
        "rebaseable",
        match m.mergeable.as_str() {
            "mergeable" => json!(true),
            "conflicting" => json!(false),
            _ => Value::Null,
        },
    );
    ins("mergeable_state", json!(mergeable_state));
    ins("merged_by", json!(merged_by.map(|a| shapes::simple_user(&s.config, &a))));
    ins("review_comments", json!(review_comments.len()));
    ins("maintainer_can_modify", json!(m.maintainer_can_modify != 0));
    ins("commits", json!(m.commits_count));
    ins("additions", json!(m.additions));
    ins("deletions", json!(m.deletions));
    ins("changed_files", json!(m.changed_files));
    ins("reviews_count", json!(reviews.len()));
    o.remove("pull_request");
    o.remove("state_reason");
    o.remove("closed_by");
    o.remove("timeline_url");
    o.remove("reactions");
    o.remove("comments_url");
    Ok(v)
}

pub async fn load_pull(s: &AppState, rw: &RepoWithOwner, number: i64) -> ApiResult<(IssueFull, PullMeta)> {
    let i = load_issue(s, rw, number).await?;
    if !i.issue.is_pull() {
        return Err(ApiError(Error::NotFound));
    }
    let m = svc::refresh_stats(s, rw, i.issue.id()).await.map_err(|_| ApiError(Error::NotFound))?;
    Ok((i, m))
}

#[derive(serde::Deserialize, Default)]
pub struct ListQuery {
    state: Option<String>,
    head: Option<String>,
    base: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
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
    let f = reef_db::issues::IssueFilter {
        state: q.state.clone().unwrap_or_else(|| "open".into()),
        is_pull: Some(true),
        sort: match q.sort.as_deref() {
            Some("updated") => "updated".into(),
            Some("popularity") => "comments".into(),
            _ => "created".into(),
        },
        ascending: q.direction.as_deref() == Some("asc"),
        ..Default::default()
    };
    let (items, _) = s.db.list_issues(Some(rw.repo.id()), &f, 1, 500).await?;
    let mut out = Vec::new();
    for i in items {
        let Ok(m) = s.db.pull_meta(i.issue.id()).await else { continue };
        if let Some(h) = &q.head {
            let want = h.split_once(':').map(|(_, b)| b).unwrap_or(h);
            if m.head_ref != want {
                continue;
            }
        }
        if let Some(b) = &q.base
            && &m.base_ref != b
        {
            continue;
        }
        out.push(pull_json(&s, &rw, &i, &m, auth.user.as_ref(), wants_html(&headers)).await?);
    }
    let (items, h) = page.slice(&out, &format!("{}/repos/{}/pulls", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

#[derive(serde::Deserialize)]
pub struct CreatePull {
    title: Option<String>,
    head: String,
    base: String,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    maintainer_can_modify: bool,
    issue: Option<i64>,
}

pub async fn create(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Json(b): Json<CreatePull>,
) -> ApiResult<impl IntoResponse> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_read() || rw.repo.archived != 0 {
        return Err(crate::error::forbidden("Repository is read-only"));
    }
    let title = match (b.title, b.issue) {
        (Some(t), _) => t,
        (None, Some(n)) => s.db.issue_by_number(rw.repo.id(), n).await?.title,
        (None, None) => return Err(ApiError(Error::field("PullRequest", "title", "missing_field", "title is required"))),
    };
    let head = b.head.split_once(':').map(|(_, r)| r.to_string()).unwrap_or(b.head);
    let (i, m) = svc::create(
        &s,
        &rw,
        u,
        svc::CreatePull { title, body: b.body, head, base: b.base, draft: b.draft, maintainer_can_modify: b.maintainer_can_modify },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(pull_json(&s, &rw, &i, &m, Some(u), wants_html(&headers)).await?)))
}

pub async fn show(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, m) = load_pull(&s, &rw, number).await?;
    let path = s.store.path(rw.repo.id());
    match media(&headers) {
        Some("diff") => {
            let d = ops::diff_text(s.store.git_binary(), &path, Some(&m.base_sha), &m.head_sha, None).await?;
            Ok(([(header::CONTENT_TYPE, "application/vnd.github.diff; charset=utf-8")], d).into_response())
        }
        Some("patch") => {
            let d = ops::patch_text(s.store.git_binary(), &path, &m.base_sha, &m.head_sha).await?;
            Ok(([(header::CONTENT_TYPE, "application/vnd.github.patch; charset=utf-8")], d).into_response())
        }
        _ => Ok(Json(pull_json(&s, &rw, &i, &m, auth.user.as_ref(), wants_html(&headers)).await?).into_response()),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct UpdatePull {
    title: Option<String>,
    body: Option<Option<String>>,
    state: Option<String>,
    base: Option<String>,
    maintainer_can_modify: Option<bool>,
    draft: Option<bool>,
}

pub async fn update(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<UpdatePull>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (i, m) = load_pull(&s, &rw, number).await?;
    if perm < Permission::Triage && u.id != i.author.id {
        return Err(crate::error::forbidden("You do not have permission to update this pull request"));
    }
    if m.merged != 0 && b.state.is_some() {
        return Err(ApiError(Error::Conflict("Merged pull requests cannot be reopened".into())));
    }
    if let Some(base) = &b.base {
        let path = s.store.path(rw.repo.id());
        let sha = ops::rev_parse(s.store.git_binary(), &path, &format!("refs/heads/{base}"))
            .await
            .map_err(|_| ApiError(Error::field("PullRequest", "base", "invalid", "base branch not found")))?;
        s.db.set_pull_base(i.issue.id(), base, &sha).await?;
    }
    if let Some(d) = b.draft {
        s.db.set_pull_draft(i.issue.id(), d).await?;
    }
    let _ = b.maintainer_can_modify;
    s.db.update_issue(
        i.issue.id(),
        u,
        reef_db::issues::IssuePatch {
            title: b.title,
            body: b.body,
            state: b.state.filter(|st| st == "open" || st == "closed"),
            ..Default::default()
        },
    )
    .await?;
    let (i, m) = load_pull(&s, &rw, number).await?;
    Ok(Json(pull_json(&s, &rw, &i, &m, Some(u), wants_html(&headers)).await?))
}

pub async fn commits(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (_, m) = load_pull(&s, &rw, number).await?;
    let path = s.store.path(rw.repo.id());
    let shas = ops::rev_list(s.store.git_binary(), &path, &m.base_sha, &m.head_sha).await?;
    let p2 = path.clone();
    let list = tokio::task::spawn_blocking(move || {
        let r = reef_git::RepoReader::open(&p2)?;
        shas.iter().map(|sha| r.commit(sha)).collect::<reef_core::Result<Vec<_>>>()
    })
    .await
    .map_err(Error::internal)??;
    let mut out = Vec::new();
    for c in &list {
        let author = crate::rest::contents::account_for_signature(&s, &c.author.name, &c.author.email).await;
        let committer = crate::rest::contents::account_for_signature(&s, &c.committer.name, &c.committer.email).await;
        let verification = crate::signing::verify_commit(&s, &rw, c).await;
        out.push(shapes::commit_with_verification(&s.config, &rw, c, author.as_ref(), committer.as_ref(), Some(verification)));
    }
    let (items, h) = page.slice(&out, &format!("{}/repos/{}/pulls/{number}/commits", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

/// `pull-request-file` objects: stats plus the per-file patch.
pub async fn files_json(s: &AppState, rw: &RepoWithOwner, base: &str, head: &str) -> ApiResult<Vec<Value>> {
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let stats = ops::diff_stat(git, &path, base, head).await?;
    let diff = ops::diff_text(git, &path, Some(base), head, None).await?;
    let patches = ops::split_diff(&diff);
    let full = rw.full_name();
    let api = s.config.api_url();
    let site = s.config.site_url();
    Ok(stats
        .iter()
        .map(|f| {
            let patch = patches
                .iter()
                .find(|p| p.new_path.as_deref() == Some(&f.path) || (p.new_path.is_none() && p.old_path.as_deref() == Some(&f.path)));
            json!({
                "sha": "",
                "filename": f.path,
                "status": f.status,
                "additions": f.additions,
                "deletions": f.deletions,
                "changes": f.additions + f.deletions,
                "blob_url": format!("{site}/{full}/blob/{head}/{}", f.path),
                "raw_url": format!("{site}/{full}/raw/{head}/{}", f.path),
                "contents_url": format!("{api}/repos/{full}/contents/{}?ref={head}", f.path),
                "patch": patch.filter(|p| !p.binary).map(|p| p.patch.trim_end().to_string()),
                "previous_filename": f.previous_path,
            })
        })
        .collect())
}

pub async fn files(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    page: Page,
) -> ApiResult<impl IntoResponse> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (_, m) = load_pull(&s, &rw, number).await?;
    let all = files_json(&s, &rw, &m.base_sha, &m.head_sha).await?;
    let (items, h) = page.slice(&all, &format!("{}/repos/{}/pulls/{number}/files", s.config.api_url(), rw.full_name()));
    Ok((h, Json(json!(items))))
}

pub async fn is_merged(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (_, m) = load_pull(&s, &rw, number).await?;
    Ok(if m.merged != 0 { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND })
}

#[derive(serde::Deserialize, Default)]
pub struct MergeBody {
    commit_title: Option<String>,
    commit_message: Option<String>,
    sha: Option<String>,
    merge_method: Option<String>,
}

pub async fn merge(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    body: Option<Json<MergeBody>>,
) -> ApiResult<Response> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access to merge"));
    }
    let (i, _) = load_pull(&s, &rw, number).await?;
    let b = body.map(|b| b.0).unwrap_or_default();
    let method = b
        .merge_method
        .as_deref()
        .map(svc::MergeMethod::parse)
        .unwrap_or(Some(svc::MergeMethod::Merge))
        .ok_or_else(|| ApiError(Error::validation("merge_method must be merge, squash, or rebase")))?;
    match svc::merge(
        &s,
        &rw,
        u,
        &i,
        svc::MergeOptions { method, commit_title: b.commit_title, commit_message: b.commit_message, expected_head: b.sha },
    )
    .await
    {
        Ok(sha) => Ok(Json(json!({"sha": sha, "merged": true, "message": "Pull Request successfully merged"})).into_response()),
        Err(Error::Conflict(msg)) => {
            Ok((StatusCode::METHOD_NOT_ALLOWED, Json(json!({"message": msg, "documentation_url": crate::error::DOCS}))).into_response())
        }
        Err(e) => Err(e.into()),
    }
}

// ----- reviews -----

fn review_json(
    s: &AppState,
    rw: &RepoWithOwner,
    number: i64,
    r: &reef_db::pulls::Review,
    author: Option<&Account>,
    association: &str,
    html: bool,
) -> Value {
    let full = rw.full_name();
    let mut v = json!({
        "id": r.id,
        "node_id": shapes::node_id("PRR", r.id),
        "user": author.map(|a| shapes::simple_user(&s.config, a)),
        "body": r.body.clone().unwrap_or_default(),
        "state": r.state.to_uppercase(),
        "html_url": format!("{}/{full}/pull/{number}#pullrequestreview-{}", s.config.site_url(), r.id),
        "pull_request_url": format!("{}/repos/{full}/pulls/{number}", s.config.api_url()),
        "_links": {"html": {"href": format!("{}/{full}/pull/{number}#pullrequestreview-{}", s.config.site_url(), r.id)}, "pull_request": {"href": format!("{}/repos/{full}/pulls/{number}", s.config.api_url())}},
        "submitted_at": iso_opt(r.submitted_at),
        "commit_id": r.commit_sha,
        "author_association": association,
    });
    if html {
        v.as_object_mut()
            .unwrap()
            .insert("body_html".into(), json!(crate::render::markdown_in_repo(r.body.as_deref().unwrap_or(""), &s.config, rw)));
    }
    v
}

pub async fn reviews(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    let mut out = Vec::new();
    let mut all = s.db.reviews(i.issue.id()).await?;
    if let Some(u) = auth.user.as_ref()
        && let Some(p) = s.db.pending_review(i.issue.id(), u.id()).await?
    {
        all.push(p);
    }
    for r in all {
        let a = s.db.account_by_id(Id(r.author_id)).await.ok();
        let assoc = match &a {
            Some(a) => crate::rest::issues::association(&s, &rw, a).await,
            None => "NONE",
        };
        out.push(review_json(&s, &rw, number, &r, a.as_ref(), assoc, wants_html(&headers)));
    }
    Ok(Json(json!(out)))
}

#[derive(serde::Deserialize, Default)]
pub struct NewReview {
    commit_id: Option<String>,
    body: Option<String>,
    event: Option<String>,
    #[serde(default)]
    comments: Vec<NewReviewComment>,
}

#[derive(serde::Deserialize)]
pub struct NewReviewComment {
    path: String,
    body: String,
    line: Option<i64>,
    start_line: Option<i64>,
    side: Option<String>,
    position: Option<i64>,
}

pub async fn create_review(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<NewReview>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_read() {
        return Err(crate::error::forbidden("Cannot review"));
    }
    let (i, m) = load_pull(&s, &rw, number).await?;
    let state = match b.event.as_deref() {
        Some("APPROVE") => "approved",
        Some("REQUEST_CHANGES") => "changes_requested",
        Some("COMMENT") => "commented",
        None => "pending",
        Some(_) => return Err(ApiError(Error::validation("event must be APPROVE, REQUEST_CHANGES, or COMMENT"))),
    };
    if state == "approved" && u.id == i.author.id {
        return Err(ApiError(Error::Validation {
            message: "Validation Failed".into(),
            errors: vec![reef_core::error::FieldError {
                resource: "PullRequestReview".into(),
                field: "user_id".into(),
                code: "custom".into(),
                message: Some("Can not approve your own pull request".into()),
            }],
        }));
    }
    if state == "changes_requested" && b.body.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(ApiError(Error::validation("A body is required when requesting changes")));
    }
    let sha = b.commit_id.clone().unwrap_or_else(|| m.head_sha.clone());
    let existing = if state == "pending" { s.db.pending_review(i.issue.id(), u.id()).await? } else { None };
    let r = match existing {
        Some(p) => match b.body.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(body) => s.db.update_review_body(Id(p.id), body).await?,
            None => p,
        },
        None => s.db.create_review(i.issue.id(), rw.repo.id(), u, state, b.body.as_deref(), Some(&sha)).await?,
    };
    if state != "pending" {
        let _ = s.db.notify_participants(i.issue.id(), Some(u.id()), "review").await;
    }
    for c in b.comments {
        let _ = c.position;
        s.db.create_review_comment(
            i.issue.id(),
            rw.repo.id(),
            Some(Id(r.id)),
            u,
            &c.body,
            &c.path,
            c.line,
            c.start_line,
            c.side.as_deref().unwrap_or("RIGHT"),
            &sha,
            None,
        )
        .await?;
    }
    if state != "pending" {
        s.db.add_issue_event(i.issue.id(), rw.repo.id(), Some(u.id()), "reviewed", &json!({"review_id": r.id, "state": state})).await?;
        crate::projects::on_review(&s, i.issue.id(), state).await;
    }
    let assoc = crate::rest::issues::association(&s, &rw, u).await;
    Ok(Json(review_json(&s, &rw, number, &r, Some(u), assoc, wants_html(&headers))))
}

#[derive(serde::Deserialize)]
pub struct SubmitReview {
    body: Option<String>,
    event: String,
}

pub async fn submit_review(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number, id)): Path<(String, String, i64, i64)>,
    Json(b): Json<SubmitReview>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    let r = s.db.review_by_id(Id(id)).await?;
    if r.author_id != u.id || r.pull_id != i.issue.id {
        return Err(ApiError(Error::NotFound));
    }
    let state = match b.event.as_str() {
        "APPROVE" => "approved",
        "REQUEST_CHANGES" => "changes_requested",
        _ => "commented",
    };
    let r = s.db.submit_review(Id(id), state, b.body.as_deref()).await?;
    s.db.add_issue_event(i.issue.id(), rw.repo.id(), Some(u.id()), "reviewed", &json!({"review_id": r.id, "state": state})).await?;
    crate::projects::on_review(&s, i.issue.id(), state).await;
    let assoc = crate::rest::issues::association(&s, &rw, u).await;
    Ok(Json(review_json(&s, &rw, number, &r, Some(u), assoc, wants_html(&headers))))
}

pub async fn dismiss_review(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number, id)): Path<(String, String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_admin() {
        return Err(crate::error::forbidden("Must have admin access to dismiss reviews"));
    }
    load_pull(&s, &rw, number).await?;
    let r = s.db.dismiss_review(Id(id)).await?;
    let a = s.db.account_by_id(Id(r.author_id)).await.ok();
    Ok(Json(review_json(&s, &rw, number, &r, a.as_ref(), "NONE", wants_html(&headers))))
}

/// Review comments (line comments) of a pull request.
pub async fn review_comments(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, m) = load_pull(&s, &rw, number).await?;
    let comments = s.db.review_comments_visible(i.issue.id(), auth.user.as_ref().map(|u| u.id())).await?;
    let files = if comments.is_empty() { Vec::new() } else { files_json(&s, &rw, &m.base_sha, &m.head_sha).await.unwrap_or_default() };
    let mut pending_ids: Vec<i64> = Vec::new();
    if let Some(u) = auth.user.as_ref()
        && let Some(p) = s.db.pending_review(i.issue.id(), u.id()).await?
    {
        pending_ids.push(p.id);
    }
    let mut out = Vec::new();
    for c in comments {
        out.push(review_comment_json(&s, &rw, number, &c, &files, &pending_ids).await);
    }
    Ok(Json(json!(out)))
}

/// The hunk of `patch` that contains `line` on `side`, from its header through that line,
/// as GitHub's `diff_hunk`.
fn diff_hunk(patch: &str, line: Option<i64>, side: &str) -> String {
    let Some(target) = line else { return String::new() };
    let mut current: Vec<&str> = Vec::new();
    let (mut o, mut n) = (0i64, 0i64);
    for raw in patch.lines() {
        if raw.starts_with("@@") {
            current = vec![raw];
            let nums = raw.trim_start_matches("@@ ").split(" @@").next().unwrap_or("");
            let mut parts = nums.split(' ');
            o = parts.next().and_then(|p| p.trim_start_matches('-').split(',').next()).and_then(|x| x.parse().ok()).unwrap_or(0);
            n = parts.next().and_then(|p| p.trim_start_matches('+').split(',').next()).and_then(|x| x.parse().ok()).unwrap_or(0);
            continue;
        }
        if current.is_empty() {
            continue;
        }
        current.push(raw);
        let (this_old, this_new) = if raw.starts_with('+') {
            let v = (None, Some(n));
            n += 1;
            v
        } else if raw.starts_with('-') {
            let v = (Some(o), None);
            o += 1;
            v
        } else if raw.starts_with('\\') {
            (None, None)
        } else {
            let v = (Some(o), Some(n));
            o += 1;
            n += 1;
            v
        };
        let hit = if side == "LEFT" { this_old == Some(target) } else { this_new == Some(target) };
        if hit {
            let start = current.len().saturating_sub(4).max(1);
            let mut lines = vec![current[0]];
            lines.extend_from_slice(&current[start..]);
            return lines.join("\n");
        }
    }
    String::new()
}

async fn review_comment_json(
    s: &AppState,
    rw: &RepoWithOwner,
    number: i64,
    c: &reef_db::pulls::ReviewComment,
    files: &[Value],
    pending_ids: &[i64],
) -> Value {
    let full = rw.full_name();
    let a = s.db.account_by_id(Id(c.author_id)).await.ok();
    let resolved_by = match c.resolved_by_id {
        Some(id) => s.db.account_by_id(Id(id)).await.ok(),
        None => None,
    };
    let patch = files.iter().find(|f| f["filename"].as_str() == Some(c.path.as_str())).and_then(|f| f["patch"].as_str()).unwrap_or("");
    let hunk = if c.in_reply_to_id.is_some() { String::new() } else { diff_hunk(patch, c.line, &c.side) };
    let assoc = match &a {
        Some(a) => crate::rest::issues::association(s, rw, a).await,
        None => "NONE",
    };
    json!({
        "id": c.id, "node_id": shapes::node_id("PRRC", c.id),
        "url": format!("{}/repos/{full}/pulls/comments/{}", s.config.api_url(), c.id),
        "pull_request_review_id": c.review_id, "diff_hunk": hunk, "path": c.path, "position": Value::Null, "original_position": Value::Null,
        "commit_id": c.commit_sha, "original_commit_id": c.commit_sha, "in_reply_to_id": c.in_reply_to_id,
        "user": a.map(|a| shapes::simple_user(&s.config, &a)), "body": c.body,
        "body_html": crate::render::markdown_in_repo(&c.body, &s.config, rw),
        "created_at": iso(c.created_at), "updated_at": iso(c.updated_at),
        "html_url": format!("{}/{full}/pull/{number}#discussion_r{}", s.config.site_url(), c.id),
        "pull_request_url": format!("{}/repos/{full}/pulls/{number}", s.config.api_url()),
        "author_association": assoc, "line": c.line, "start_line": c.start_line, "side": c.side, "start_side": c.start_line.map(|_| c.side.clone()),
        "subject_type": "line",
        "resolved": c.resolved != 0,
        "resolved_by": resolved_by.map(|a| shapes::simple_user(&s.config, &a)),
        "pending": c.review_id.map(|r| pending_ids.contains(&r)).unwrap_or(false),
    })
}

#[derive(serde::Deserialize)]
pub struct NewLineComment {
    body: String,
    commit_id: Option<String>,
    path: String,
    line: Option<i64>,
    start_line: Option<i64>,
    side: Option<String>,
    in_reply_to: Option<i64>,
}

pub async fn create_review_comment(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<NewLineComment>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, m) = load_pull(&s, &rw, number).await?;
    let parent = match b.in_reply_to {
        Some(id) => Some(s.db.review_comment_by_id(Id(id)).await?),
        None => None,
    };
    let sha = b.commit_id.or_else(|| parent.as_ref().map(|p| p.commit_sha.clone())).unwrap_or_else(|| m.head_sha.clone());
    let path = parent.as_ref().map(|p| p.path.clone()).unwrap_or(b.path);
    let line = parent.as_ref().map(|p| p.line).unwrap_or(b.line);
    let side = parent.as_ref().map(|p| p.side.clone()).or(b.side).unwrap_or_else(|| "RIGHT".into());
    let root = parent.as_ref().map(|p| p.in_reply_to_id.unwrap_or(p.id));
    let c = s.db.create_review_comment(i.issue.id(), rw.repo.id(), None, u, &b.body, &path, line, b.start_line, &side, &sha, root).await?;
    s.db.add_issue_event(
        i.issue.id(),
        rw.repo.id(),
        Some(u.id()),
        "review_commented",
        &json!({"comment_id": c.id, "path": c.path, "line": c.line}),
    )
    .await?;
    let _ = s.db.notify_participants(i.issue.id(), Some(u.id()), "comment").await;
    let files = files_json(&s, &rw, &m.base_sha, &m.head_sha).await.unwrap_or_default();
    Ok((StatusCode::CREATED, Json(review_comment_json(&s, &rw, number, &c, &files, &[]).await)))
}

/// Pull number and comment for `/pulls/comments/{id}` routes (the comment carries its pull).
async fn load_review_comment(s: &AppState, rw: &RepoWithOwner, id: i64) -> ApiResult<(reef_db::pulls::ReviewComment, i64)> {
    let c = s.db.review_comment_by_id(Id(id)).await?;
    if c.repo_id != rw.repo.id {
        return Err(ApiError(Error::NotFound));
    }
    let issue = s.db.issue_by_id(Id(c.pull_id)).await?;
    Ok((c, issue.number))
}

pub async fn review_comment_show(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (c, number) = load_review_comment(&s, &rw, id).await?;
    Ok(Json(review_comment_json(&s, &rw, number, &c, &[], &[]).await))
}

#[derive(serde::Deserialize)]
pub struct EditBody {
    body: String,
}

pub async fn review_comment_update(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Json(b): Json<EditBody>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (c, number) = load_review_comment(&s, &rw, id).await?;
    if c.author_id != u.id && !perm.can_write() {
        return Err(crate::error::forbidden("Cannot edit this comment"));
    }
    let c = s.db.update_review_comment(Id(id), &b.body).await?;
    Ok(Json(review_comment_json(&s, &rw, number, &c, &[], &[]).await))
}

pub async fn review_comment_delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (c, _) = load_review_comment(&s, &rw, id).await?;
    if c.author_id != u.id && !perm.can_write() {
        return Err(crate::error::forbidden("Cannot delete this comment"));
    }
    s.db.delete_review_comment(Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reef extension: resolve a review thread (`PUT /pulls/comments/{id}/resolve`).
pub async fn resolve_thread(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (c, number) = load_review_comment(&s, &rw, id).await?;
    let root = Id(c.in_reply_to_id.unwrap_or(c.id));
    s.db.set_thread_resolved(root, Some(u.id())).await?;
    let c = s.db.review_comment_by_id(root).await?;
    Ok(Json(review_comment_json(&s, &rw, number, &c, &[], &[]).await))
}

pub async fn unresolve_thread(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (c, number) = load_review_comment(&s, &rw, id).await?;
    let root = Id(c.in_reply_to_id.unwrap_or(c.id));
    s.db.set_thread_resolved(root, None).await?;
    let c = s.db.review_comment_by_id(root).await?;
    Ok(Json(review_comment_json(&s, &rw, number, &c, &[], &[]).await))
}

pub async fn review_show(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number, id)): Path<(String, String, i64, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    let r = s.db.review_by_id(Id(id)).await?;
    if r.pull_id != i.issue.id || (r.state == "pending" && auth.user.as_ref().map(|u| u.id) != Some(r.author_id)) {
        return Err(ApiError(Error::NotFound));
    }
    let a = s.db.account_by_id(Id(r.author_id)).await.ok();
    Ok(Json(review_json(&s, &rw, number, &r, a.as_ref(), "NONE", wants_html(&headers))))
}

/// `PUT /pulls/{n}/reviews/{id}`: update the summary body of the viewer's review.
pub async fn review_update(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number, id)): Path<(String, String, i64, i64)>,
    Json(b): Json<EditBody>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    let r = s.db.review_by_id(Id(id)).await?;
    if r.pull_id != i.issue.id || r.author_id != u.id {
        return Err(ApiError(Error::NotFound));
    }
    let r = s.db.update_review_body(Id(id), &b.body).await?;
    let assoc = crate::rest::issues::association(&s, &rw, u).await;
    Ok(Json(review_json(&s, &rw, number, &r, Some(u), assoc, wants_html(&headers))))
}

/// `DELETE /pulls/{n}/reviews/{id}`: discard the viewer's pending review and its comments.
pub async fn review_delete(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number, id)): Path<(String, String, i64, i64)>,
) -> ApiResult<StatusCode> {
    let u = auth.require()?;
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    let r = s.db.review_by_id(Id(id)).await?;
    if r.pull_id != i.issue.id || r.author_id != u.id {
        return Err(ApiError(Error::NotFound));
    }
    if r.state != "pending" {
        return Err(ApiError(Error::validation("Only pending reviews can be deleted")));
    }
    s.db.delete_review(Id(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ----- review requests -----

#[derive(serde::Deserialize, Default)]
pub struct ReviewersBody {
    #[serde(default)]
    reviewers: Vec<String>,
    #[serde(default)]
    team_reviewers: Vec<String>,
}

pub async fn requested_reviewers(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, number)): Path<(String, String, i64)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    let users = s.db.review_requests(i.issue.id()).await?;
    Ok(Json(json!({"users": users.iter().map(|a| shapes::simple_user(&s.config, a)).collect::<Vec<_>>(), "teams": []})))
}

pub async fn request_reviewers(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<ReviewersBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    if perm < Permission::Triage && u.id != i.author.id {
        return Err(crate::error::forbidden("Cannot request reviews"));
    }
    if b.reviewers.iter().any(|r| r.eq_ignore_ascii_case(&i.author.login)) {
        return Err(ApiError(Error::validation("Review cannot be requested from pull request author.")));
    }
    let _ = b.team_reviewers;
    s.db.request_reviewers(i.issue.id(), &b.reviewers).await?;
    for login in &b.reviewers {
        if let Ok(a) = s.db.account_by_login(login).await {
            let _ = s.db.notify_user_about_issue(a.id(), i.issue.id(), Some(u.id()), "review_requested").await;
        }
    }
    for r in &b.reviewers {
        s.db.add_issue_event(i.issue.id(), rw.repo.id(), Some(u.id()), "review_requested", &json!({"requested_reviewer": r})).await?;
    }
    let (i, m) = load_pull(&s, &rw, number).await?;
    Ok((StatusCode::CREATED, Json(pull_json(&s, &rw, &i, &m, Some(u), wants_html(&headers)).await?)))
}

pub async fn remove_reviewers(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Json(b): Json<ReviewersBody>,
) -> ApiResult<Json<Value>> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    let (i, _) = load_pull(&s, &rw, number).await?;
    if perm < Permission::Triage && u.id != i.author.id {
        return Err(crate::error::forbidden("Cannot change review requests"));
    }
    s.db.remove_review_requests(i.issue.id(), &b.reviewers).await?;
    let (i, m) = load_pull(&s, &rw, number).await?;
    Ok(Json(pull_json(&s, &rw, &i, &m, Some(u), wants_html(&headers)).await?))
}

// ----- compare -----

pub async fn compare(
    State(s): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((owner, repo, basehead)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (base, head) = basehead
        .split_once("...")
        .or_else(|| basehead.split_once(".."))
        .ok_or_else(|| ApiError(Error::validation("compare needs base...head")))?;
    let head = head.split_once(':').map(|(_, h)| h).unwrap_or(head);
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let base_sha = ops::rev_parse(git, &path, base).await?;
    let head_sha = ops::rev_parse(git, &path, head).await?;
    match media(&headers) {
        Some("diff") => {
            let d = ops::diff_text(git, &path, Some(&base_sha), &head_sha, None).await?;
            return Ok(([(header::CONTENT_TYPE, "application/vnd.github.diff; charset=utf-8")], d).into_response());
        }
        Some("patch") => {
            let d = ops::patch_text(git, &path, &base_sha, &head_sha).await?;
            return Ok(([(header::CONTENT_TYPE, "application/vnd.github.patch; charset=utf-8")], d).into_response());
        }
        _ => {}
    }
    let merge_base = ops::merge_base(git, &path, &base_sha, &head_sha).await?.unwrap_or_else(|| base_sha.clone());
    let ahead = ops::rev_list(git, &path, &base_sha, &head_sha).await?;
    let behind = ops::rev_list(git, &path, &head_sha, &base_sha).await?;
    let status = if ahead.is_empty() && behind.is_empty() {
        "identical"
    } else if behind.is_empty() {
        "ahead"
    } else if ahead.is_empty() {
        "behind"
    } else {
        "diverged"
    };
    let p2 = path.clone();
    let ids = ahead.clone();
    let (base_c, mb_c, commits) = tokio::task::spawn_blocking(move || {
        let r = reef_git::RepoReader::open(&p2)?;
        Ok::<_, Error>((
            r.commit(&base_sha)?,
            r.commit(&merge_base)?,
            ids.iter().map(|i| r.commit(i)).collect::<reef_core::Result<Vec<_>>>()?,
        ))
    })
    .await
    .map_err(Error::internal)??;
    let files = files_json(&s, &rw, &mb_c.id, &head_sha).await?;
    let full = rw.full_name();
    let api = s.config.api_url();
    let site = s.config.site_url();
    let mut commit_json = Vec::new();
    for c in &commits {
        let a = crate::rest::contents::account_for_signature(&s, &c.author.name, &c.author.email).await;
        commit_json.push(shapes::commit(&s.config, &rw, c, a.as_ref(), None));
    }
    Ok(Json(json!({
        "url": format!("{api}/repos/{full}/compare/{base}...{head}"),
        "html_url": format!("{site}/{full}/compare/{base}...{head}"),
        "permalink_url": format!("{site}/{full}/compare/{}...{}", &base_c.id[..7], &head_sha[..7]),
        "diff_url": format!("{site}/{full}/compare/{base}...{head}.diff"),
        "patch_url": format!("{site}/{full}/compare/{base}...{head}.patch"),
        "base_commit": shapes::commit(&s.config, &rw, &base_c, None, None),
        "merge_base_commit": shapes::commit(&s.config, &rw, &mb_c, None, None),
        "status": status,
        "ahead_by": ahead.len(),
        "behind_by": behind.len(),
        "total_commits": ahead.len(),
        "commits": commit_json,
        "files": files,
    }))
    .into_response())
}

// ----- commit statuses -----

fn status_json(s: &AppState, rw: &RepoWithOwner, st: &reef_db::pulls::CommitStatus, creator: Option<&Account>) -> Value {
    json!({
        "id": st.id, "node_id": shapes::node_id("SC", st.id),
        "url": format!("{}/repos/{}/statuses/{}", s.config.api_url(), rw.full_name(), st.sha),
        "state": st.state, "description": st.description, "target_url": st.target_url, "context": st.context,
        "created_at": iso(st.created_at), "updated_at": iso(st.created_at),
        "creator": creator.map(|c| shapes::simple_user(&s.config, c)),
    })
}

#[derive(serde::Deserialize)]
pub struct NewStatus {
    state: String,
    target_url: Option<String>,
    description: Option<String>,
    context: Option<String>,
}

pub async fn create_status(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, sha)): Path<(String, String, String)>,
    Json(b): Json<NewStatus>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let u = auth.require()?;
    let (rw, perm) = load(&s, &auth, &owner, &repo).await?;
    if !perm.can_write() {
        return Err(crate::error::forbidden("Must have push access to create statuses"));
    }
    let path = s.store.path(rw.repo.id());
    let sha = ops::rev_parse(s.store.git_binary(), &path, &sha).await?;
    let st =
        s.db.create_status(
            rw.repo.id(),
            &sha,
            &b.state,
            b.context.as_deref().unwrap_or("default"),
            b.description.as_deref(),
            b.target_url.as_deref(),
            Some(u.id()),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(status_json(&s, &rw, &st, Some(u)))))
}

pub async fn list_statuses(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, r)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let path = s.store.path(rw.repo.id());
    let sha = ops::rev_parse(s.store.git_binary(), &path, &r).await?;
    let mut out = Vec::new();
    for st in s.db.statuses(rw.repo.id(), &sha).await? {
        let c = match st.creator_id {
            Some(id) => s.db.account_by_id(Id(id)).await.ok(),
            None => None,
        };
        out.push(status_json(&s, &rw, &st, c.as_ref()));
    }
    Ok(Json(json!(out)))
}

/// Combined status for a ref.
pub async fn combined_status(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, r)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let path = s.store.path(rw.repo.id());
    let sha = ops::rev_parse(s.store.git_binary(), &path, &r).await?;
    let latest = s.db.latest_statuses(rw.repo.id(), &sha).await?;
    let state = if latest.is_empty() {
        "pending"
    } else if latest.iter().any(|st| st.state == "failure" || st.state == "error") {
        "failure"
    } else if latest.iter().all(|st| st.state == "success") {
        "success"
    } else {
        "pending"
    };
    let mut statuses = Vec::new();
    for st in &latest {
        statuses.push(status_json(&s, &rw, st, None));
    }
    Ok(Json(json!({
        "state": state, "statuses": statuses, "sha": sha, "total_count": latest.len(),
        "repository": shapes::repo(&s.config, &rw, None),
        "commit_url": format!("{}/repos/{}/commits/{sha}", s.config.api_url(), rw.full_name()),
        "url": format!("{}/repos/{}/commits/{sha}/status", s.config.api_url(), rw.full_name()),
    })))
}

pub async fn check_runs(
    State(s): State<AppState>,
    auth: Auth,
    Path((owner, repo, _r)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    load(&s, &auth, &owner, &repo).await?;
    Ok(Json(json!({"total_count": 0, "check_runs": []})))
}

/// Commit diff/patch media types on `GET /repos/{o}/{r}/commits/{sha}`.
pub async fn commit_diff(s: &AppState, rw: &RepoWithOwner, sha: &str, kind: &str) -> ApiResult<Response> {
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let sha = ops::rev_parse(git, &path, sha).await?;
    if kind == "patch" {
        let d = ops::patch_text(git, &path, &format!("{sha}^"), &sha).await.map_err(|_| ApiError(Error::NotFound))?;
        return Ok(([(header::CONTENT_TYPE, "application/vnd.github.patch; charset=utf-8")], d).into_response());
    }
    let d = ops::diff_text(git, &path, None, &sha, None).await?;
    Ok(([(header::CONTENT_TYPE, "application/vnd.github.diff; charset=utf-8")], d).into_response())
}

/// `/{owner}/{repo}/commit/{sha}.patch` and `.diff`, as on github.com. The router cannot
/// match a suffix inside a segment, so plain `/commit/{sha}` falls through to the web app.
pub async fn commit_patch_url(
    State(s): State<AppState>,
    auth: Auth,
    uri: axum::http::Uri,
    Path((owner, repo, sha)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let Some((sha, kind)) = sha.rsplit_once('.').filter(|(_, k)| *k == "patch" || *k == "diff") else {
        return Ok(crate::web::spa(uri).await);
    };
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let mut res = commit_diff(&s, &rw, sha, kind).await?;
    res.headers_mut().insert(header::CONTENT_TYPE, "text/plain; charset=utf-8".parse().unwrap());
    Ok(res)
}

/// `/{owner}/{repo}/pull/{number}.patch` and `.diff`; plain `/pull/{number}` is the web app.
pub async fn pull_patch_url(
    State(s): State<AppState>,
    auth: Auth,
    uri: axum::http::Uri,
    Path((owner, repo, number)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let Some((n, kind)) = number.rsplit_once('.').filter(|(_, k)| *k == "patch" || *k == "diff") else {
        return Ok(crate::web::spa(uri).await);
    };
    let Ok(number) = n.parse::<i64>() else {
        return Ok(crate::web::spa(uri).await);
    };
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    let (_, m) = load_pull(&s, &rw, number).await?;
    let git = s.store.git_binary();
    let path = s.store.path(rw.repo.id());
    let d = if kind == "patch" {
        ops::patch_text(git, &path, &m.base_sha, &m.head_sha).await?
    } else {
        ops::diff_text(git, &path, Some(&m.base_sha), &m.head_sha, None).await?
    };
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], d).into_response())
}

pub fn media_public(headers: &HeaderMap) -> Option<&'static str> {
    media(headers)
}

pub fn _unused(_: Option<Query<()>>) {}
