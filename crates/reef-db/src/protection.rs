//! Branch protection rules.

use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::AssertSqlSafe;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Protection {
    pub id: i64,
    pub repo_id: i64,
    pub pattern: String,
    pub allow_force_pushes: i64,
    pub allow_deletions: i64,
    pub require_pull_request: i64,
    pub required_approving_review_count: i64,
    pub dismiss_stale_reviews: i64,
    pub require_code_owner_reviews: i64,
    pub enforce_admins: i64,
    pub required_status_checks: String,
    pub strict_status_checks: i64,
    pub require_linear_history: i64,
    pub require_signed_commits: i64,
    pub lock_branch: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Protection {
    pub fn matches(&self, branch: &str) -> bool {
        glob_match(&self.pattern, branch)
    }
    pub fn checks(&self) -> Vec<String> {
        serde_json::from_str(&self.required_status_checks).unwrap_or_default()
    }
}

/// Minimal glob: `*` matches any run of characters except `/`, `**` matches anything,
/// `?` one character. Good enough for GitHub-style patterns like `release/*`.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    fn rec(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') => {
                if p.get(1) == Some(&'*') {
                    let rest = &p[2..];
                    (0..=s.len()).any(|i| rec(rest, &s[i..]))
                } else {
                    let rest = &p[1..];
                    (0..=s.len()).take_while(|&i| i == 0 || s[i - 1] != '/').any(|i| rec(rest, &s[i..]))
                }
            }
            Some('?') => !s.is_empty() && s[0] != '/' && rec(&p[1..], &s[1..]),
            Some(c) => !s.is_empty() && s[0] == *c && rec(&p[1..], &s[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    rec(&p, &t)
}

#[derive(Debug, Clone, Default)]
pub struct ProtectionInput {
    pub pattern: String,
    pub allow_force_pushes: bool,
    pub allow_deletions: bool,
    pub require_pull_request: bool,
    pub required_approving_review_count: i64,
    pub dismiss_stale_reviews: bool,
    pub require_code_owner_reviews: bool,
    pub enforce_admins: bool,
    pub required_status_checks: Vec<String>,
    pub strict_status_checks: bool,
    pub require_linear_history: bool,
    pub require_signed_commits: bool,
    pub lock_branch: bool,
}

const COLS: &str = "id, repo_id, pattern, allow_force_pushes, allow_deletions, require_pull_request, required_approving_review_count, dismiss_stale_reviews, require_code_owner_reviews, enforce_admins, required_status_checks, strict_status_checks, require_linear_history, require_signed_commits, lock_branch, created_at, updated_at";

impl Db {
    pub async fn protections(&self, repo: Id) -> Result<Vec<Protection>> {
        sqlx::query_as::<_, Protection>(AssertSqlSafe(format!("SELECT {COLS} FROM branch_protections WHERE repo_id = $1 ORDER BY pattern")))
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn protection_by_pattern(&self, repo: Id, pattern: &str) -> Result<Protection> {
        sqlx::query_as::<_, Protection>(AssertSqlSafe(format!("SELECT {COLS} FROM branch_protections WHERE repo_id = $1 AND pattern = $2")))
            .bind(repo.0)
            .bind(pattern)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// The rules that apply to a branch, most specific (no wildcard) first.
    pub async fn protections_for_branch(&self, repo: Id, branch: &str) -> Result<Vec<Protection>> {
        let mut all: Vec<Protection> = self.protections(repo).await?.into_iter().filter(|p| p.matches(branch)).collect();
        all.sort_by_key(|p| (p.pattern.contains('*') || p.pattern.contains('?'), p.pattern.len()));
        Ok(all)
    }

    /// Create or replace the rule for `pattern`.
    pub async fn upsert_protection(&self, repo: Id, p: ProtectionInput) -> Result<Protection> {
        if p.pattern.trim().is_empty() || p.pattern.len() > 200 {
            return Err(Error::validation("pattern is required"));
        }
        let ts = now();
        let checks = serde_json::to_string(&p.required_status_checks).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "INSERT INTO branch_protections (id, repo_id, pattern, allow_force_pushes, allow_deletions, require_pull_request, required_approving_review_count, dismiss_stale_reviews, require_code_owner_reviews, enforce_admins, required_status_checks, strict_status_checks, require_linear_history, require_signed_commits, lock_branch, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $16) \
             ON CONFLICT (repo_id, pattern) DO UPDATE SET allow_force_pushes = $4, allow_deletions = $5, require_pull_request = $6, required_approving_review_count = $7, dismiss_stale_reviews = $8, require_code_owner_reviews = $9, enforce_admins = $10, required_status_checks = $11, strict_status_checks = $12, require_linear_history = $13, require_signed_commits = $14, lock_branch = $15, updated_at = $16",
        )
        .bind(reef_core::id::next().0)
        .bind(repo.0)
        .bind(p.pattern.trim())
        .bind(b(p.allow_force_pushes))
        .bind(b(p.allow_deletions))
        .bind(b(p.require_pull_request))
        .bind(p.required_approving_review_count.clamp(0, 6))
        .bind(b(p.dismiss_stale_reviews))
        .bind(b(p.require_code_owner_reviews))
        .bind(b(p.enforce_admins))
        .bind(checks)
        .bind(b(p.strict_status_checks))
        .bind(b(p.require_linear_history))
        .bind(b(p.require_signed_commits))
        .bind(b(p.lock_branch))
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        self.protection_by_pattern(repo, p.pattern.trim()).await
    }

    pub async fn delete_protection(&self, repo: Id, pattern: &str) -> Result<()> {
        let r = sqlx::query("DELETE FROM branch_protections WHERE repo_id = $1 AND pattern = $2")
            .bind(repo.0)
            .bind(pattern)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::glob_match;
    #[test]
    fn globs() {
        assert!(glob_match("main", "main"));
        assert!(!glob_match("main", "maint"));
        assert!(glob_match("release/*", "release/1.0"));
        assert!(!glob_match("release/*", "release/1.0/x"));
        assert!(glob_match("release/**", "release/1.0/x"));
        assert!(glob_match("*", "main"));
        assert!(!glob_match("*", "feature/x"));
        assert!(glob_match("v?", "v1"));
    }
}
