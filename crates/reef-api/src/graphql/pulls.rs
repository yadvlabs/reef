//! Pull request resolvers: metadata, mergeability, commits, files, reviews.

use super::issues::*;
use super::types::*;
use super::{Ctx, from_core};
use crate::shapes;
use async_graphql::{Context, ID, Result};
use reef_core::Id;
use reef_core::time::iso;
use reef_db::issues::IssueFull;
use reef_db::pulls::PullMeta;
use reef_db::repos::RepoWithOwner;

pub async fn pr_meta(ctx: &Context<'_>, i: &IssueFull) -> Option<PullMeta> {
    Ctx::from(ctx).db.pull_meta(i.issue.id()).await.ok()
}

pub async fn pr_state(ctx: &Context<'_>, i: &IssueFull) -> PullRequestState {
    if i.issue.is_open() {
        PullRequestState::Open
    } else if pr_meta(ctx, i).await.map(|m| m.merged != 0).unwrap_or(false) {
        PullRequestState::Merged
    } else {
        PullRequestState::Closed
    }
}

/// Refresh head/base shas, diff stats, and mergeability when they are stale. Cheap when
/// nothing moved (two rev-parse calls).
pub async fn refresh(s: &crate::AppState, rw: &RepoWithOwner, i: &IssueFull) -> reef_core::Result<PullMeta> {
    crate::pulls::refresh_stats(s, rw, i.issue.id()).await
}

pub async fn mergeable(ctx: &Context<'_>, rw: &RepoWithOwner, i: &IssueFull) -> MergeableState {
    let s = super::app(ctx);
    match refresh(&s, rw, i).await {
        Ok(m) => match m.mergeable.as_str() {
            "mergeable" => MergeableState::Mergeable,
            "conflicting" => MergeableState::Conflicting,
            _ => MergeableState::Unknown,
        },
        Err(_) => MergeableState::Unknown,
    }
}

pub async fn merge_state_status(ctx: &Context<'_>, rw: &RepoWithOwner, i: &IssueFull) -> MergeStateStatus {
    let c = Ctx::from(ctx);
    let Some(m) = pr_meta(ctx, i).await else { return MergeStateStatus::Unknown };
    if m.draft != 0 {
        return MergeStateStatus::Draft;
    }
    match mergeable(ctx, rw, i).await {
        MergeableState::Conflicting => return MergeStateStatus::Dirty,
        MergeableState::Unknown => return MergeStateStatus::Unknown,
        MergeableState::Mergeable => {}
    }
    let rules = c.db.protections_for_branch(rw.repo.id(), &m.base_ref).await.unwrap_or_default();
    if let Some(rule) = rules.first() {
        if rule.require_pull_request != 0
            && review_decision(ctx, rw, i).await != Some(PullRequestReviewDecision::Approved)
            && rule.required_approving_review_count > 0
        {
            return MergeStateStatus::Blocked;
        }
        let checks = rule.checks();
        if !checks.is_empty() {
            let statuses = c.db.latest_statuses(rw.repo.id(), &m.head_sha).await.unwrap_or_default();
            let all_ok = checks.iter().all(|ch| statuses.iter().any(|st| st.context == *ch && st.state == "success"));
            if !all_ok {
                return MergeStateStatus::Blocked;
            }
        }
        if rule.strict_status_checks != 0 {
            let path = c.store.path(rw.repo.id());
            if let Ok(false) = reef_git::ops::is_ancestor(c.store.git_binary(), &path, &m.base_sha, &m.head_sha).await {
                return MergeStateStatus::Behind;
            }
        }
    }
    let statuses = c.db.latest_statuses(rw.repo.id(), &m.head_sha).await.unwrap_or_default();
    if statuses.iter().any(|st| st.state == "failure" || st.state == "error") {
        return MergeStateStatus::Unstable;
    }
    MergeStateStatus::Clean
}

pub async fn review_decision(ctx: &Context<'_>, rw: &RepoWithOwner, i: &IssueFull) -> Option<PullRequestReviewDecision> {
    let c = Ctx::from(ctx);
    let m = pr_meta(ctx, i).await?;
    let rules = c.db.protections_for_branch(rw.repo.id(), &m.base_ref).await.unwrap_or_default();
    let required = rules.first().filter(|r| r.require_pull_request != 0).map(|r| r.required_approving_review_count).unwrap_or(0);
    let latest = c.db.latest_reviews(i.issue.id()).await.unwrap_or_default();
    if latest.iter().any(|r| r.state == "changes_requested") {
        return Some(PullRequestReviewDecision::ChangesRequested);
    }
    let approvals = latest.iter().filter(|r| r.state == "approved").count() as i64;
    if required > 0 {
        return Some(if approvals >= required { PullRequestReviewDecision::Approved } else { PullRequestReviewDecision::ReviewRequired });
    }
    if approvals > 0 { Some(PullRequestReviewDecision::Approved) } else { None }
}

/// Issues referenced by closing keywords in the body ("closes #12", "fixes #3").
pub async fn closing_issues(ctx: &Context<'_>, rw: &RepoWithOwner, i: &IssueFull) -> Result<IssueConnection> {
    let c = Ctx::from(ctx);
    let mut nodes = Vec::new();
    for n in crate::pulls::closing_numbers(i.issue.body.as_deref().unwrap_or("")) {
        if let Ok(f) = c.db.issue_full_by_number(rw.repo.id(), n).await
            && !f.issue.is_pull()
        {
            nodes.push(Issue { repo: rw.clone(), i: f });
        }
    }
    let total = nodes.len() as i32;
    Ok(IssueConnection {
        edges: nodes.iter().enumerate().map(|(k, n)| IssueEdge { cursor: cursor(k), node: n.clone() }).collect(),
        nodes,
        page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
        total_count: total,
    })
}

pub async fn commits(
    ctx: &Context<'_>,
    rw: &RepoWithOwner,
    i: &IssueFull,
    first: Option<i32>,
    last: Option<i32>,
    after: Option<String>,
) -> Result<PullRequestCommitConnection> {
    let c = Ctx::from(ctx);
    let Some(m) = pr_meta(ctx, i).await else {
        return Ok(PullRequestCommitConnection {
            nodes: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        });
    };
    let path = c.store.path(rw.repo.id());
    let shas = reef_git::ops::rev_list(c.store.git_binary(), &path, &m.base_sha, &m.head_sha).await.unwrap_or_default();
    let total = shas.len();
    let selected: Vec<String> = match last {
        Some(l) => shas.iter().rev().take(l.max(0) as usize).rev().cloned().collect(),
        None => shas.clone(),
    };
    let (items, info, _) = page(selected, first, after);
    let p2 = path.clone();
    let ids: Vec<String> = items.iter().map(|(_, s)| s.clone()).collect();
    let commits = tokio::task::spawn_blocking(move || {
        let r = reef_git::RepoReader::open(&p2)?;
        ids.iter().map(|id| r.commit(id)).collect::<reef_core::Result<Vec<_>>>()
    })
    .await
    .map_err(|e| from_core(reef_core::Error::internal(e)))?
    .map_err(from_core)?;
    let site = c.config.site_url();
    let full = rw.full_name();
    Ok(PullRequestCommitConnection {
        nodes: commits
            .into_iter()
            .map(|cm| PullRequestCommit {
                id: ID(format!("PRC_{}", cm.id)),
                url: format!("{site}/{full}/pull/{}/commits/{}", i.issue.number, cm.id),
                resource_path: format!("/{full}/pull/{}/commits/{}", i.issue.number, cm.id),
                commit: Commit { repo: rw.clone(), c: cm },
            })
            .collect(),
        page_info: info,
        total_count: total as i32,
    })
}

pub async fn files(
    ctx: &Context<'_>,
    rw: &RepoWithOwner,
    i: &IssueFull,
    first: Option<i32>,
    after: Option<String>,
) -> Result<PullRequestChangedFileConnection> {
    let c = Ctx::from(ctx);
    let Some(m) = pr_meta(ctx, i).await else {
        return Ok(PullRequestChangedFileConnection {
            nodes: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        });
    };
    let path = c.store.path(rw.repo.id());
    let stats = reef_git::ops::diff_stat(c.store.git_binary(), &path, &m.base_sha, &m.head_sha).await.unwrap_or_default();
    let (items, info, total) = page(stats, first, after);
    Ok(PullRequestChangedFileConnection {
        nodes: items
            .into_iter()
            .map(|(_, f)| PullRequestChangedFile {
                path: f.path,
                additions: f.additions as i32,
                deletions: f.deletions as i32,
                change_type: match f.status.as_str() {
                    "added" => PatchStatus::Added,
                    "removed" => PatchStatus::Deleted,
                    "renamed" => PatchStatus::Renamed,
                    "copied" => PatchStatus::Copied,
                    _ => PatchStatus::Modified,
                },
            })
            .collect(),
        page_info: info,
        total_count: total as i32,
    })
}

pub async fn reviews(
    ctx: &Context<'_>,
    rw: &RepoWithOwner,
    i: &IssueFull,
    first: Option<i32>,
    after: Option<String>,
    states: Option<Vec<PullRequestReviewState>>,
    latest: bool,
) -> Result<PullRequestReviewConnection> {
    let c = Ctx::from(ctx);
    let rows = if latest { c.db.latest_reviews(i.issue.id()).await } else { c.db.reviews(i.issue.id()).await }.map_err(from_core)?;
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| match &states {
            Some(st) => st.contains(&review_state(&r.state)),
            None => true,
        })
        .collect();
    let (items, info, total) = page(rows, first, after);
    let mut nodes = Vec::new();
    for (_, r) in items {
        let author = c.db.account_by_id(Id(r.author_id)).await.ok();
        let assoc = match &author {
            Some(a) => CommentAuthorAssociation::from_str(association(ctx, rw, a).await),
            None => CommentAuthorAssociation::None,
        };
        let groups = super::issues::ReactionContent::ALL
            .iter()
            .map(|rc| ReactionGroup {
                content: *rc,
                users: ReactingUserConnection { total_count: 0 },
                viewer_has_reacted: false,
                created_at: None,
            })
            .collect();
        nodes.push(PullRequestReview {
            id: ID(shapes::node_id("PRR", r.id)),
            database_id: r.id,
            viewer_did_author: c.viewer().map(|v| v.id) == Some(r.author_id),
            author: author.map(User),
            author_association: assoc,
            body_html: crate::render::markdown_in_repo(r.body.as_deref().unwrap_or(""), c.config, rw),
            body: r.body.clone().unwrap_or_default(),
            state: review_state(&r.state),
            submitted_at: r.submitted_at.map(iso),
            created_at: iso(r.created_at),
            updated_at: iso(r.updated_at),
            url: format!("{}/{}/pull/{}#pullrequestreview-{}", c.config.site_url(), rw.full_name(), i.issue.number, r.id),
            commit: r.commit_sha.clone().map(|oid| PrCommitRef { oid }),
            reaction_groups: groups,
            includes_created_edit: r.updated_at > r.created_at,
        });
    }
    Ok(PullRequestReviewConnection { nodes, page_info: info, total_count: total as i32 })
}

pub fn review_state(s: &str) -> PullRequestReviewState {
    match s {
        "approved" => PullRequestReviewState::Approved,
        "changes_requested" => PullRequestReviewState::ChangesRequested,
        "dismissed" => PullRequestReviewState::Dismissed,
        "pending" => PullRequestReviewState::Pending,
        _ => PullRequestReviewState::Commented,
    }
}

pub async fn review_requests(ctx: &Context<'_>, _rw: &RepoWithOwner, i: &IssueFull) -> Result<ReviewRequestConnection> {
    let c = Ctx::from(ctx);
    let users = c.db.review_requests(i.issue.id()).await.map_err(from_core)?;
    let total = users.len() as i32;
    Ok(ReviewRequestConnection {
        nodes: users
            .into_iter()
            .map(|u| ReviewRequest {
                id: ID(format!("RR_{}_{}", i.issue.id, u.id)),
                requested_reviewer: Some(RequestedReviewer::User(User(u))),
                as_code_owner: false,
            })
            .collect(),
        page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
        total_count: total,
    })
}
