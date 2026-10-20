//! Pull request logic shared by REST, GraphQL, and the push path: creating, refreshing
//! stats and mergeability, merging, and closing-keyword parsing.

use crate::AppState;
use reef_core::{Error, Id, Result};
use reef_db::accounts::Account;
use reef_db::issues::{IssueFull, NewIssue};
use reef_db::pulls::{NewPull, PullMeta, PullStats};
use reef_db::repos::RepoWithOwner;
use reef_git::ops;

pub struct CreatePull {
    pub title: String,
    pub body: Option<String>,
    pub head: String,
    pub base: String,
    pub draft: bool,
    pub maintainer_can_modify: bool,
}

pub async fn create(s: &AppState, rw: &RepoWithOwner, author: &Account, p: CreatePull) -> Result<(IssueFull, PullMeta)> {
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let head_ref = p.head.trim().trim_start_matches("refs/heads/").to_string();
    let base_ref = p.base.trim().trim_start_matches("refs/heads/").to_string();
    if head_ref == base_ref {
        return Err(Error::field("PullRequest", "head", "invalid", "head and base must be different branches"));
    }
    let head_sha = ops::rev_parse(git, &path, &format!("refs/heads/{head_ref}"))
        .await
        .map_err(|_| Error::field("PullRequest", "head", "invalid", format!("branch \"{head_ref}\" not found")))?;
    let base_sha = ops::rev_parse(git, &path, &format!("refs/heads/{base_ref}"))
        .await
        .map_err(|_| Error::field("PullRequest", "base", "invalid", format!("branch \"{base_ref}\" not found")))?;
    if ops::is_ancestor(git, &path, &head_sha, &base_sha).await? {
        return Err(Error::Validation {
            message: "Validation Failed".into(),
            errors: vec![reef_core::error::FieldError {
                resource: "PullRequest".into(),
                field: "head".into(),
                code: "custom".into(),
                message: Some(format!("No commits between {base_ref} and {head_ref}")),
            }],
        });
    }
    for existing in s.db.open_pulls_for_head(rw.repo.id(), &head_ref).await? {
        if existing.base_ref == base_ref {
            let n = s.db.issue_by_id(Id(existing.issue_id)).await?.number;
            return Err(Error::Validation {
                message: "Validation Failed".into(),
                errors: vec![reef_core::error::FieldError {
                    resource: "PullRequest".into(),
                    field: "head".into(),
                    code: "custom".into(),
                    message: Some(format!("A pull request already exists for {}:{head_ref} (#{n})", rw.owner.login)),
                }],
            });
        }
    }
    let issue =
        s.db.create_issue(rw.repo.id(), author, NewIssue { title: p.title, body: p.body, is_pull: true, ..Default::default() }).await?;
    let meta =
        s.db.create_pull_meta(
            issue.issue.id(),
            rw.repo.id(),
            NewPull {
                head_repo_id: rw.repo.id,
                head_ref,
                head_sha,
                base_ref,
                base_sha,
                draft: p.draft,
                maintainer_can_modify: p.maintainer_can_modify,
            },
        )
        .await?;
    let meta = refresh_stats(s, rw, issue.issue.id()).await.unwrap_or(meta);
    s.db.record_event(rw.repo.id(), Some(author.id()), "pull_request.opened", &serde_json::json!({"number": issue.issue.number})).await?;
    // Issues this pull request will close get a "referenced" entry on their timeline.
    for n in closing_numbers(issue.issue.body.as_deref().unwrap_or("")) {
        if let Ok(target) = s.db.issue_by_number(rw.repo.id(), n).await
            && !target.is_pull()
            && target.id != issue.issue.id
        {
            let _ =
                s.db.add_issue_event(
                    target.id(),
                    rw.repo.id(),
                    Some(author.id()),
                    "cross-referenced",
                    &serde_json::json!({"source_number": issue.issue.number, "will_close": true}),
                )
                .await;
        }
    }
    if let Err(e) = crate::actions::on_pull_request(s, rw, Some(author), &issue, &meta, "opened").await {
        tracing::warn!(error = %e, "actions: planning runs for pull request failed");
    }
    Ok((issue, meta))
}

/// Recompute head/base shas, diff stats, and mergeability if anything moved.
pub async fn refresh_stats(s: &AppState, rw: &RepoWithOwner, issue: Id) -> Result<PullMeta> {
    let m = s.db.pull_meta(issue).await?;
    if m.merged != 0 {
        return Ok(m);
    }
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let head_sha = match ops::rev_parse(git, &path, &format!("refs/heads/{}", m.head_ref)).await {
        Ok(sha) => sha,
        Err(_) => m.head_sha.clone(), // branch deleted: keep the last known head
    };
    let base_sha = ops::rev_parse(git, &path, &format!("refs/heads/{}", m.base_ref)).await.unwrap_or_else(|_| m.base_sha.clone());
    let key = format!("{head_sha}+{base_sha}");
    if m.mergeable_sha.as_deref() == Some(&key) {
        return Ok(m);
    }
    let stats = ops::diff_stat(git, &path, &base_sha, &head_sha).await.unwrap_or_default();
    let commits = ops::rev_list(git, &path, &base_sha, &head_sha).await.unwrap_or_default();
    let mergeable = match ops::merge_tree(git, &path, &base_sha, &head_sha).await {
        Ok(r) if r.tree.is_some() => "mergeable",
        Ok(_) => "conflicting",
        Err(_) => "unknown",
    };
    s.db.update_pull_stats(
        issue,
        PullStats {
            head_sha,
            base_sha,
            additions: stats.iter().map(|f| f.additions).sum(),
            deletions: stats.iter().map(|f| f.deletions).sum(),
            changed_files: stats.len() as i64,
            commits_count: commits.len() as i64,
            mergeable,
        },
    )
    .await?;
    s.db.pull_meta(issue).await
}

/// Refresh every open pull request touching a pushed branch (head or base).
pub async fn on_branch_updated(s: &AppState, rw: &RepoWithOwner, branch: &str) -> Result<()> {
    let mut ids: Vec<i64> = s.db.open_pulls_for_head(rw.repo.id(), branch).await?.into_iter().map(|m| m.issue_id).collect();
    ids.extend(s.db.open_pulls_for_base(rw.repo.id(), branch).await?.into_iter().map(|m| m.issue_id));
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        if let Err(e) = refresh_stats(s, rw, Id(id)).await {
            tracing::warn!(error = %e, pull = id, "refreshing pull request failed");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn parse(s: &str) -> Option<MergeMethod> {
        match s.to_ascii_lowercase().as_str() {
            "merge" => Some(MergeMethod::Merge),
            "squash" => Some(MergeMethod::Squash),
            "rebase" => Some(MergeMethod::Rebase),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }
}

pub struct MergeOptions {
    pub method: MergeMethod,
    pub commit_title: Option<String>,
    pub commit_message: Option<String>,
    /// Expected head sha (GitHub's `sha` field); the merge fails if the head moved.
    pub expected_head: Option<String>,
}

/// Merge a pull request into its base branch. Returns the merge commit id. REST and GraphQL
/// both merge through here; the work runs detached (`timeout::detached`), so a request that
/// times out or hangs up never leaves the base branch moved without the pull request marked
/// merged.
pub async fn merge(s: &AppState, rw: &RepoWithOwner, actor: &Account, issue: &IssueFull, o: MergeOptions) -> Result<String> {
    let (s, rw, actor, issue) = (s.clone(), rw.clone(), actor.clone(), issue.clone());
    crate::timeout::detached(async move { merge_attached(&s, &rw, &actor, &issue, o).await }).await
}

async fn merge_attached(s: &AppState, rw: &RepoWithOwner, actor: &Account, issue: &IssueFull, o: MergeOptions) -> Result<String> {
    if !issue.issue.is_open() {
        return Err(Error::Conflict("Pull request is not open".into()));
    }
    let m = refresh_stats(s, rw, issue.issue.id()).await?;
    if let Some(exp) = &o.expected_head
        && exp != &m.head_sha
    {
        return Err(Error::Conflict("Head branch was modified. Review and try the merge again.".into()));
    }
    if m.draft != 0 {
        return Err(Error::Conflict("Pull request is a draft".into()));
    }
    let allowed = match o.method {
        MergeMethod::Merge => rw.repo.allow_merge_commit != 0,
        MergeMethod::Squash => rw.repo.allow_squash_merge != 0,
        MergeMethod::Rebase => rw.repo.allow_rebase_merge != 0,
    };
    if !allowed {
        return Err(Error::Conflict(format!("{} merges are not allowed on this repository", o.method.as_str())));
    }
    // Branch protection on the base branch.
    let rules = s.db.protections_for_branch(rw.repo.id(), &m.base_ref).await?;
    if let Some(rule) = rules.first()
        && !(actor.is_admin() && rule.enforce_admins == 0)
    {
        if rule.require_pull_request != 0 && rule.required_approving_review_count > 0 {
            let latest = s.db.latest_reviews(issue.issue.id()).await?;
            if latest.iter().any(|r| r.state == "changes_requested") {
                return Err(Error::Conflict("Changes have been requested on this pull request".into()));
            }
            let approvals = latest.iter().filter(|r| r.state == "approved").count() as i64;
            if approvals < rule.required_approving_review_count {
                return Err(Error::Conflict(format!("At least {} approving review(s) required", rule.required_approving_review_count)));
            }
        }
        let checks = rule.checks();
        if !checks.is_empty() {
            let statuses = s.db.latest_statuses(rw.repo.id(), &m.head_sha).await?;
            for ch in &checks {
                if !statuses.iter().any(|st| st.context == *ch && st.state == "success") {
                    return Err(Error::Conflict(format!("Required status check \"{ch}\" is not successful")));
                }
            }
        }
        if rule.require_linear_history != 0 && o.method == MergeMethod::Merge {
            return Err(Error::Conflict("Merge commits are not allowed on this branch (linear history required)".into()));
        }
        if rule.lock_branch != 0 {
            return Err(Error::Conflict("The base branch is locked".into()));
        }
    }
    let path = s.store.path(rw.repo.id());
    let git = s.store.git_binary();
    let name = actor.name.clone().unwrap_or_else(|| actor.login.clone());
    let email = actor.email.clone().unwrap_or_else(|| format!("{}@users.noreply.{}", actor.login, s.config.site_host()));
    let committer = ops::Identity { name: &name, email: &email };
    let base_sha = ops::rev_parse(git, &path, &format!("refs/heads/{}", m.base_ref)).await?;
    let new_tip = match o.method {
        MergeMethod::Merge => {
            let r = ops::merge_tree(git, &path, &base_sha, &m.head_sha).await?;
            let tree = r.tree.ok_or_else(|| Error::Conflict(format!("Merge conflict in: {}", r.conflicts.join(", "))))?;
            let title = o
                .commit_title
                .clone()
                .unwrap_or_else(|| format!("Merge pull request #{} from {}/{}", issue.issue.number, rw.owner.login, m.head_ref));
            let msg = match o.commit_message.as_deref().filter(|b| !b.trim().is_empty()) {
                Some(b) => format!("{title}\n\n{b}"),
                None => format!("{title}\n\n{}", issue.issue.title),
            };
            ops::commit_tree(git, &path, &tree, &[&base_sha, &m.head_sha], &msg, committer, committer).await?
        }
        MergeMethod::Squash => {
            let r = ops::merge_tree(git, &path, &base_sha, &m.head_sha).await?;
            let tree = r.tree.ok_or_else(|| Error::Conflict(format!("Merge conflict in: {}", r.conflicts.join(", "))))?;
            let title = o.commit_title.clone().unwrap_or_else(|| format!("{} (#{})", issue.issue.title, issue.issue.number));
            let body = match o.commit_message.as_deref().filter(|b| !b.trim().is_empty()) {
                Some(b) => b.to_string(),
                None => issue.issue.body.clone().unwrap_or_default(),
            };
            let msg = if body.trim().is_empty() { title } else { format!("{title}\n\n{body}") };
            let author_name = issue.author.name.clone().unwrap_or_else(|| issue.author.login.clone());
            let author_email =
                issue.author.email.clone().unwrap_or_else(|| format!("{}@users.noreply.{}", issue.author.login, s.config.site_host()));
            ops::commit_tree(git, &path, &tree, &[&base_sha], &msg, ops::Identity { name: &author_name, email: &author_email }, committer)
                .await?
        }
        MergeMethod::Rebase => ops::rebase_onto(git, &path, &base_sha, &m.head_sha, committer).await.map_err(|e| match e {
            Error::Conflict(_) => Error::Conflict("Rebase would produce conflicts; merge or squash instead".into()),
            o => o,
        })?,
    };
    ops::update_ref(
        git,
        &path,
        &format!("refs/heads/{}", m.base_ref),
        &new_tip,
        Some(&base_sha),
        &format!("merge pull request #{}", issue.issue.number),
    )
    .await?;
    s.db.mark_pull_merged(issue.issue.id(), actor, &new_tip, o.method.as_str()).await?;
    s.db.update_issue(
        issue.issue.id(),
        actor,
        reef_db::issues::IssuePatch { state: Some("closed".into()), state_reason: Some(Some("completed".into())), ..Default::default() },
    )
    .await?;
    s.db.add_issue_event(
        issue.issue.id(),
        rw.repo.id(),
        Some(actor.id()),
        "merged",
        &serde_json::json!({"commit_id": new_tip, "method": o.method.as_str(), "base_ref": m.base_ref, "head_ref": m.head_ref}),
    )
    .await?;
    let _ = s.db.notify_participants(issue.issue.id(), Some(actor.id()), "state_change").await;
    // Close referenced issues, recording how they were closed.
    for n in closing_numbers(issue.issue.body.as_deref().unwrap_or("")) {
        if let Ok(target) = s.db.issue_by_number(rw.repo.id(), n).await
            && !target.is_pull()
            && target.is_open()
        {
            let _ =
                s.db.update_issue(
                    target.id(),
                    actor,
                    reef_db::issues::IssuePatch {
                        state: Some("closed".into()),
                        state_reason: Some(Some("completed".into())),
                        close_commit: Some(new_tip.clone()),
                        close_pull: Some(issue.issue.number),
                        ..Default::default()
                    },
                )
                .await;
        }
    }
    if rw.repo.delete_branch_on_merge != 0 && m.head_repo_id == rw.repo.id {
        let others = s.db.open_pulls_for_base(rw.repo.id(), &m.head_ref).await.unwrap_or_default();
        if others.is_empty() && m.head_ref != rw.repo.default_branch {
            let _ = ops::delete_ref(git, &path, &format!("refs/heads/{}", m.head_ref)).await;
        }
    }
    let size = s.store.size_kb(rw.repo.id()).await.ok();
    s.db.touch_repo_push(rw.repo.id(), size, false).await?;
    on_branch_updated(s, rw, &m.base_ref).await?;
    Ok(new_tip)
}

/// Issue numbers referenced by GitHub's closing keywords in `text`, in order of appearance.
pub fn closing_numbers(text: &str) -> Vec<i64> {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let mut found: Vec<(usize, i64)> = Vec::new();
    for kw in ["closes", "closed", "close", "fixes", "fixed", "fix", "resolves", "resolved", "resolve"] {
        let mut start = 0;
        while let Some(rel) = lower[start..].find(kw) {
            let pos = start + rel;
            let end = pos + kw.len();
            let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
            let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
            if before_ok && after_ok {
                let after_trim = lower[end..].trim_start_matches([':', ' ']);
                if let Some(rest) = after_trim.strip_prefix('#') {
                    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = digits.parse::<i64>() {
                        found.push((pos, n));
                    }
                }
            }
            start = end;
        }
    }
    found.sort_by_key(|(p, _)| *p);
    let mut out = Vec::new();
    for (_, n) in found {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn closing_keywords() {
        assert_eq!(super::closing_numbers("Fixes #12, closes: #3 and resolves #12"), vec![12, 3]);
        assert_eq!(super::closing_numbers("prefixes #9"), Vec::<i64>::new());
    }
}
