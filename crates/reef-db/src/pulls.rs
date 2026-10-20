//! Pull request metadata, reviews, review requests, review comments, and commit statuses.

use crate::accounts::Account;
use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::Row;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PullMeta {
    pub issue_id: i64,
    pub repo_id: i64,
    pub head_repo_id: i64,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub base_sha: String,
    pub draft: i64,
    pub merged: i64,
    pub merged_at: Option<i64>,
    pub merged_by_id: Option<i64>,
    pub merge_commit_sha: Option<String>,
    pub merge_method: Option<String>,
    pub mergeable: String,
    pub mergeable_sha: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub commits_count: i64,
    pub maintainer_can_modify: i64,
    pub auto_merge: Option<String>,
}

const PULL_COLS: &str = "issue_id, repo_id, head_repo_id, head_ref, head_sha, base_ref, base_sha, draft, merged, merged_at, merged_by_id, merge_commit_sha, merge_method, mergeable, mergeable_sha, additions, deletions, changed_files, commits_count, maintainer_can_modify, auto_merge";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Review {
    pub id: i64,
    pub pull_id: i64,
    pub repo_id: i64,
    pub author_id: i64,
    pub state: String,
    pub body: Option<String>,
    pub commit_sha: Option<String>,
    pub submitted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

const REVIEW_COLS: &str = "id, pull_id, repo_id, author_id, state, body, commit_sha, submitted_at, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReviewComment {
    pub id: i64,
    pub pull_id: i64,
    pub repo_id: i64,
    pub review_id: Option<i64>,
    pub author_id: i64,
    pub body: String,
    pub path: String,
    pub line: Option<i64>,
    pub start_line: Option<i64>,
    pub side: String,
    pub commit_sha: String,
    pub in_reply_to_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub resolved: i64,
    pub resolved_by_id: Option<i64>,
}

const RC_COLS: &str = "id, pull_id, repo_id, review_id, author_id, body, path, line, start_line, side, commit_sha, in_reply_to_id, created_at, updated_at, resolved, resolved_by_id";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommitStatus {
    pub id: i64,
    pub repo_id: i64,
    pub sha: String,
    pub state: String,
    pub context: String,
    pub description: Option<String>,
    pub target_url: Option<String>,
    pub creator_id: Option<i64>,
    pub created_at: i64,
}

pub struct NewPull {
    pub head_repo_id: i64,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub base_sha: String,
    pub draft: bool,
    pub maintainer_can_modify: bool,
}

pub struct PullStats {
    pub head_sha: String,
    pub base_sha: String,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub commits_count: i64,
    pub mergeable: &'static str,
}

impl Db {
    pub async fn create_pull_meta(&self, issue: Id, repo: Id, p: NewPull) -> Result<PullMeta> {
        sqlx::query(
            "INSERT INTO pulls (issue_id, repo_id, head_repo_id, head_ref, head_sha, base_ref, base_sha, draft, maintainer_can_modify) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(issue.0)
        .bind(repo.0)
        .bind(p.head_repo_id)
        .bind(&p.head_ref)
        .bind(&p.head_sha)
        .bind(&p.base_ref)
        .bind(&p.base_sha)
        .bind(b(p.draft))
        .bind(b(p.maintainer_can_modify))
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        self.pull_meta(issue).await
    }

    pub async fn pull_meta(&self, issue: Id) -> Result<PullMeta> {
        sqlx::query_as::<_, PullMeta>(sqlx::AssertSqlSafe(format!("SELECT {PULL_COLS} FROM pulls WHERE issue_id = $1")))
            .bind(issue.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Open pull requests whose head is `branch` in this repository.
    pub async fn open_pulls_for_head(&self, repo: Id, branch: &str) -> Result<Vec<PullMeta>> {
        sqlx::query_as::<_, PullMeta>(sqlx::AssertSqlSafe("SELECT p.issue_id, p.repo_id, p.head_repo_id, p.head_ref, p.head_sha, p.base_ref, p.base_sha, p.draft, p.merged, p.merged_at, p.merged_by_id, p.merge_commit_sha, p.merge_method, p.mergeable, p.mergeable_sha, p.additions, p.deletions, p.changed_files, p.commits_count, p.maintainer_can_modify, p.auto_merge \
             FROM pulls p JOIN issues i ON i.id = p.issue_id WHERE p.head_repo_id = $1 AND p.head_ref = $2 AND i.state = 'open'".to_string()))
        .bind(repo.0)
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Open pull requests targeting `branch`.
    pub async fn open_pulls_for_base(&self, repo: Id, branch: &str) -> Result<Vec<PullMeta>> {
        sqlx::query_as::<_, PullMeta>(sqlx::AssertSqlSafe("SELECT p.issue_id, p.repo_id, p.head_repo_id, p.head_ref, p.head_sha, p.base_ref, p.base_sha, p.draft, p.merged, p.merged_at, p.merged_by_id, p.merge_commit_sha, p.merge_method, p.mergeable, p.mergeable_sha, p.additions, p.deletions, p.changed_files, p.commits_count, p.maintainer_can_modify, p.auto_merge \
             FROM pulls p JOIN issues i ON i.id = p.issue_id WHERE p.repo_id = $1 AND p.base_ref = $2 AND i.state = 'open'".to_string()))
        .bind(repo.0)
        .bind(branch)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn update_pull_stats(&self, issue: Id, st: PullStats) -> Result<()> {
        sqlx::query("UPDATE pulls SET head_sha = $1, base_sha = $2, additions = $3, deletions = $4, changed_files = $5, commits_count = $6, mergeable = $7, mergeable_sha = $8 WHERE issue_id = $9")
            .bind(&st.head_sha)
            .bind(&st.base_sha)
            .bind(st.additions)
            .bind(st.deletions)
            .bind(st.changed_files)
            .bind(st.commits_count)
            .bind(st.mergeable)
            .bind(format!("{}+{}", st.head_sha, st.base_sha))
            .bind(issue.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn set_pull_draft(&self, issue: Id, draft: bool) -> Result<()> {
        sqlx::query("UPDATE pulls SET draft = $1 WHERE issue_id = $2")
            .bind(b(draft))
            .bind(issue.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn set_pull_base(&self, issue: Id, base_ref: &str, base_sha: &str) -> Result<()> {
        sqlx::query("UPDATE pulls SET base_ref = $1, base_sha = $2, mergeable = 'unknown' WHERE issue_id = $3")
            .bind(base_ref)
            .bind(base_sha)
            .bind(issue.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn mark_pull_merged(&self, issue: Id, by: &Account, merge_commit: &str, method: &str) -> Result<()> {
        let ts = now();
        sqlx::query(
            "UPDATE pulls SET merged = 1, merged_at = $1, merged_by_id = $2, merge_commit_sha = $3, merge_method = $4 WHERE issue_id = $5",
        )
        .bind(ts)
        .bind(by.id)
        .bind(merge_commit)
        .bind(method)
        .bind(issue.0)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    // ----- reviews -----

    pub async fn create_review(
        &self,
        pull: Id,
        repo: Id,
        author: &Account,
        state: &str,
        body: Option<&str>,
        commit_sha: Option<&str>,
    ) -> Result<Review> {
        if !matches!(state, "pending" | "commented" | "approved" | "changes_requested") {
            return Err(Error::validation("invalid review state"));
        }
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO pull_reviews (id, pull_id, repo_id, author_id, state, body, commit_sha, submitted_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)")
            .bind(id.0)
            .bind(pull.0)
            .bind(repo.0)
            .bind(author.id)
            .bind(state)
            .bind(body.map(str::trim).filter(|s| !s.is_empty()))
            .bind(commit_sha)
            .bind(if state == "pending" { None } else { Some(ts) })
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        // A submitted review satisfies the author's review request.
        if state != "pending" {
            sqlx::query("DELETE FROM pull_review_requests WHERE pull_id = $1 AND user_id = $2")
                .bind(pull.0)
                .bind(author.id)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        self.review_by_id(id).await
    }

    pub async fn review_by_id(&self, id: Id) -> Result<Review> {
        sqlx::query_as::<_, Review>(sqlx::AssertSqlSafe(format!("SELECT {REVIEW_COLS} FROM pull_reviews WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn reviews(&self, pull: Id) -> Result<Vec<Review>> {
        sqlx::query_as::<_, Review>(sqlx::AssertSqlSafe(format!(
            "SELECT {REVIEW_COLS} FROM pull_reviews WHERE pull_id = $1 AND state <> 'pending' ORDER BY id"
        )))
        .bind(pull.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn submit_review(&self, id: Id, state: &str, body: Option<&str>) -> Result<Review> {
        let ts = now();
        sqlx::query("UPDATE pull_reviews SET state = $1, body = COALESCE($2, body), submitted_at = $3, updated_at = $3 WHERE id = $4")
            .bind(state)
            .bind(body)
            .bind(ts)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.review_by_id(id).await
    }

    /// The author's not-yet-submitted review, if any.
    pub async fn pending_review(&self, pull: Id, author: Id) -> Result<Option<Review>> {
        sqlx::query_as::<_, Review>(sqlx::AssertSqlSafe(format!(
            "SELECT {REVIEW_COLS} FROM pull_reviews WHERE pull_id = $1 AND author_id = $2 AND state = 'pending' ORDER BY id DESC"
        )))
        .bind(pull.0)
        .bind(author.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn update_review_body(&self, id: Id, body: &str) -> Result<Review> {
        sqlx::query("UPDATE pull_reviews SET body = $1, updated_at = $2 WHERE id = $3")
            .bind(body.trim())
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.review_by_id(id).await
    }

    /// Delete a review together with its line comments (used for pending reviews).
    pub async fn delete_review(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM review_comments WHERE review_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("DELETE FROM pull_reviews WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    pub async fn dismiss_review(&self, id: Id) -> Result<Review> {
        sqlx::query("UPDATE pull_reviews SET state = 'dismissed', updated_at = $1 WHERE id = $2")
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.review_by_id(id).await
    }

    /// Latest non-dismissed review per author (what "review decision" is computed from).
    pub async fn latest_reviews(&self, pull: Id) -> Result<Vec<Review>> {
        let all = self.reviews(pull).await?;
        let mut latest: Vec<Review> = Vec::new();
        for r in all.into_iter().rev() {
            if r.state == "commented" || r.state == "dismissed" {
                continue;
            }
            if !latest.iter().any(|l| l.author_id == r.author_id) {
                latest.push(r);
            }
        }
        latest.reverse();
        Ok(latest)
    }

    // ----- review requests -----

    pub async fn request_reviewers(&self, pull: Id, logins: &[String]) -> Result<Vec<Account>> {
        for l in logins {
            let a = self
                .account_by_login(l)
                .await
                .map_err(|_| Error::field("PullRequest", "reviewers", "invalid", format!("user \"{l}\" not found")))?;
            let _ = sqlx::query("INSERT INTO pull_review_requests (pull_id, user_id, created_at) VALUES ($1, $2, $3)")
                .bind(pull.0)
                .bind(a.id)
                .bind(now())
                .execute(&self.pool)
                .await;
        }
        self.review_requests(pull).await
    }

    pub async fn remove_review_requests(&self, pull: Id, logins: &[String]) -> Result<Vec<Account>> {
        for l in logins {
            if let Ok(a) = self.account_by_login(l).await {
                sqlx::query("DELETE FROM pull_review_requests WHERE pull_id = $1 AND user_id = $2")
                    .bind(pull.0)
                    .bind(a.id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
            }
        }
        self.review_requests(pull).await
    }

    pub async fn review_requests(&self, pull: Id) -> Result<Vec<Account>> {
        let rows = sqlx::query("SELECT user_id FROM pull_review_requests WHERE pull_id = $1 ORDER BY created_at")
            .bind(pull.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        let ids: Vec<i64> = rows.iter().map(|r| r.get::<i64, _>("user_id")).collect();
        self.accounts_by_ids(&ids).await
    }

    // ----- review comments -----

    #[allow(clippy::too_many_arguments)]
    pub async fn create_review_comment(
        &self,
        pull: Id,
        repo: Id,
        review: Option<Id>,
        author: &Account,
        body: &str,
        path: &str,
        line: Option<i64>,
        start_line: Option<i64>,
        side: &str,
        commit_sha: &str,
        in_reply_to: Option<i64>,
    ) -> Result<ReviewComment> {
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO review_comments (id, pull_id, repo_id, review_id, author_id, body, path, line, start_line, side, commit_sha, in_reply_to_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)")
            .bind(id.0)
            .bind(pull.0)
            .bind(repo.0)
            .bind(review.map(|r| r.0))
            .bind(author.id)
            .bind(body.trim())
            .bind(path)
            .bind(line)
            .bind(start_line)
            .bind(if side == "LEFT" { "LEFT" } else { "RIGHT" })
            .bind(commit_sha)
            .bind(in_reply_to)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.review_comment_by_id(id).await
    }

    pub async fn review_comment_by_id(&self, id: Id) -> Result<ReviewComment> {
        sqlx::query_as::<_, ReviewComment>(sqlx::AssertSqlSafe(format!("SELECT {RC_COLS} FROM review_comments WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn review_comments(&self, pull: Id) -> Result<Vec<ReviewComment>> {
        sqlx::query_as::<_, ReviewComment>(sqlx::AssertSqlSafe(format!(
            "SELECT {RC_COLS} FROM review_comments WHERE pull_id = $1 ORDER BY id"
        )))
        .bind(pull.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Line comments visible to `viewer`: everything submitted, plus the viewer's own pending ones.
    pub async fn review_comments_visible(&self, pull: Id, viewer: Option<Id>) -> Result<Vec<ReviewComment>> {
        sqlx::query_as::<_, ReviewComment>(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM review_comments c WHERE c.pull_id = $1 AND (c.review_id IS NULL OR c.author_id = $2 OR NOT EXISTS (SELECT 1 FROM pull_reviews r WHERE r.id = c.review_id AND r.state = 'pending')) ORDER BY c.id",
            RC_COLS.split(", ").map(|c| format!("c.{c}")).collect::<Vec<_>>().join(", ")
        )))
        .bind(pull.0)
        .bind(viewer.map(|v| v.0).unwrap_or(0))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Mark a thread (the root comment and its replies) resolved or unresolved.
    pub async fn set_thread_resolved(&self, root: Id, by: Option<Id>) -> Result<()> {
        sqlx::query("UPDATE review_comments SET resolved = $1, resolved_by_id = $2, updated_at = $3 WHERE id = $4 OR in_reply_to_id = $4")
            .bind(b(by.is_some()))
            .bind(by.map(|b| b.0))
            .bind(now())
            .bind(root.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn update_review_comment(&self, id: Id, body: &str) -> Result<ReviewComment> {
        sqlx::query("UPDATE review_comments SET body = $1, updated_at = $2 WHERE id = $3")
            .bind(body.trim())
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.review_comment_by_id(id).await
    }

    pub async fn delete_review_comment(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM review_comments WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- commit statuses -----

    #[allow(clippy::too_many_arguments)]
    pub async fn create_status(
        &self,
        repo: Id,
        sha: &str,
        state: &str,
        context: &str,
        description: Option<&str>,
        target_url: Option<&str>,
        creator: Option<Id>,
    ) -> Result<CommitStatus> {
        if !matches!(state, "error" | "failure" | "pending" | "success") {
            return Err(Error::field("Status", "state", "invalid", "state must be error, failure, pending, or success"));
        }
        let id = reef_core::id::next();
        sqlx::query("INSERT INTO commit_statuses (id, repo_id, sha, state, context, description, target_url, creator_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
            .bind(id.0)
            .bind(repo.0)
            .bind(sha)
            .bind(state)
            .bind(context)
            .bind(description)
            .bind(target_url)
            .bind(creator.map(|c| c.0))
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query_as::<_, CommitStatus>(
            "SELECT id, repo_id, sha, state, context, description, target_url, creator_id, created_at FROM commit_statuses WHERE id = $1",
        )
        .bind(id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)
    }

    /// All statuses for a commit, newest first.
    pub async fn statuses(&self, repo: Id, sha: &str) -> Result<Vec<CommitStatus>> {
        sqlx::query_as::<_, CommitStatus>("SELECT id, repo_id, sha, state, context, description, target_url, creator_id, created_at FROM commit_statuses WHERE repo_id = $1 AND sha = $2 ORDER BY id DESC")
            .bind(repo.0)
            .bind(sha)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Latest status per context.
    pub async fn latest_statuses(&self, repo: Id, sha: &str) -> Result<Vec<CommitStatus>> {
        let all = self.statuses(repo, sha).await?;
        let mut seen = std::collections::HashSet::new();
        Ok(all.into_iter().filter(|s| seen.insert(s.context.clone())).collect())
    }
}
