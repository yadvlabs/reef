//! Read-only queries behind repository insights (pulse) and profile contribution graphs.

use crate::issues::{ISSUE_COLS, Issue};
use crate::{Db, map_err};
use reef_core::{Id, Result};
use sqlx::{AssertSqlSafe, FromRow, Row};

/// A pull request merged in a window, with the merge time.
#[derive(Debug, Clone)]
pub struct MergedPull {
    pub issue: Issue,
    pub merged_at: i64,
}

/// One issue or pull request authored by a user: `(repo_id, is_pull, created_at)`.
#[derive(Debug, Clone, Copy)]
pub struct AuthoredItem {
    pub repo_id: i64,
    pub is_pull: bool,
    pub created_at: i64,
}

impl Db {
    /// Issues (or pull requests) created in `repo` at or after `since` (ms).
    pub async fn issues_created_since(&self, repo: Id, since: i64, is_pull: bool) -> Result<Vec<Issue>> {
        sqlx::query_as::<_, Issue>(AssertSqlSafe(format!(
            "SELECT {ISSUE_COLS} FROM issues WHERE repo_id = $1 AND is_pull = $2 AND created_at >= $3 ORDER BY created_at DESC"
        )))
        .bind(repo.0)
        .bind(is_pull as i64)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Issues (or pull requests) closed in `repo` at or after `since` (ms).
    pub async fn issues_closed_since(&self, repo: Id, since: i64, is_pull: bool) -> Result<Vec<Issue>> {
        sqlx::query_as::<_, Issue>(AssertSqlSafe(format!("SELECT {ISSUE_COLS} FROM issues WHERE repo_id = $1 AND is_pull = $2 AND closed_at IS NOT NULL AND closed_at >= $3 ORDER BY closed_at DESC")))
            .bind(repo.0)
            .bind(is_pull as i64)
            .bind(since)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Pull requests merged in `repo` at or after `since` (ms).
    pub async fn pulls_merged_since(&self, repo: Id, since: i64) -> Result<Vec<MergedPull>> {
        let cols: Vec<String> = ISSUE_COLS.split(", ").map(|c| format!("i.{c}")).collect();
        let sql = format!(
            "SELECT {}, p.merged_at AS merged_at FROM issues i JOIN pulls p ON p.issue_id = i.id WHERE i.repo_id = $1 AND p.merged = 1 AND p.merged_at IS NOT NULL AND p.merged_at >= $2 ORDER BY p.merged_at DESC",
            cols.join(", ")
        );
        let rows = sqlx::query(AssertSqlSafe(sql)).bind(repo.0).bind(since).fetch_all(&self.pool).await.map_err(map_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let issue = Issue::from_row(&row).map_err(map_err)?;
            let merged_at: i64 = row.try_get("merged_at").map_err(map_err)?;
            out.push(MergedPull { issue, merged_at });
        }
        Ok(out)
    }

    /// Issues and pull requests authored by `author` created in `[since, until]` (ms).
    pub async fn issues_by_author_since(&self, author: Id, since: i64, until: i64) -> Result<Vec<AuthoredItem>> {
        let rows = sqlx::query("SELECT repo_id, is_pull, created_at FROM issues WHERE author_id = $1 AND created_at >= $2 AND created_at <= $3 ORDER BY created_at")
            .bind(author.0)
            .bind(since)
            .bind(until)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows
            .iter()
            .map(|r| AuthoredItem {
                repo_id: r.get::<i64, _>("repo_id"),
                is_pull: r.get::<i64, _>("is_pull") != 0,
                created_at: r.get::<i64, _>("created_at"),
            })
            .collect())
    }
}
