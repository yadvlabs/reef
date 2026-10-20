//! Notifications: one row per (user, subject); repeated activity on the same subject refreshes
//! the row instead of piling up, like GitHub's threads. Also the site settings the admin
//! console edits, and the cross-repository activity feed.

use crate::{Db, map_err, now};
use reef_core::{Id, Result};
use sqlx::{AssertSqlSafe, Row};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Notification {
    pub id: i64,
    pub user_id: i64,
    pub actor_id: Option<i64>,
    pub repo_id: Option<i64>,
    pub subject_type: String,
    pub subject_id: Option<i64>,
    pub reason: String,
    pub title: String,
    pub url: String,
    pub unread: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, user_id, actor_id, repo_id, subject_type, subject_id, reason, title, url, unread, created_at, updated_at";

pub struct Notify<'a> {
    pub user: Id,
    pub actor: Option<Id>,
    pub repo: Option<Id>,
    pub subject_type: &'a str,
    pub subject_id: Option<i64>,
    pub reason: &'a str,
    pub title: &'a str,
    pub url: &'a str,
}

impl Db {
    /// Create or refresh a notification. Nothing is recorded for the actor's own activity.
    pub async fn notify(&self, n: Notify<'_>) -> Result<()> {
        if n.actor == Some(n.user) {
            return Ok(());
        }
        let ts = now();
        if let Some(sid) = n.subject_id {
            // A personal reason (mention, assignment, review request) is not downgraded to a
            // generic one by later activity on the same thread.
            let personal = ["mention", "assign", "review_requested"];
            let reason_sql = if personal.contains(&n.reason) {
                "$2"
            } else {
                "CASE WHEN reason IN ('mention', 'assign', 'review_requested') AND unread = 1 THEN reason ELSE $2 END"
            };
            let updated = sqlx::query(AssertSqlSafe(format!("UPDATE notifications SET actor_id = $1, reason = {reason_sql}, title = $3, url = $4, unread = 1, updated_at = $5 WHERE user_id = $6 AND subject_type = $7 AND subject_id = $8")))
                .bind(n.actor.map(|a| a.0))
                .bind(n.reason)
                .bind(n.title)
                .bind(n.url)
                .bind(ts)
                .bind(n.user.0)
                .bind(n.subject_type)
                .bind(sid)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
            if updated.rows_affected() > 0 {
                return Ok(());
            }
        }
        sqlx::query("INSERT INTO notifications (id, user_id, actor_id, repo_id, subject_type, subject_id, reason, title, url, unread, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10, $10)")
            .bind(reef_core::id::next().0)
            .bind(n.user.0)
            .bind(n.actor.map(|a| a.0))
            .bind(n.repo.map(|r| r.0))
            .bind(n.subject_type)
            .bind(n.subject_id)
            .bind(n.reason)
            .bind(n.title)
            .bind(n.url)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Notify everyone taking part in an issue or pull request: author, assignees, commenters,
    /// people mentioned, requested reviewers. The actor is skipped.
    pub async fn notify_participants(&self, issue: Id, actor: Option<Id>, reason: &str) -> Result<()> {
        let i = self.issue_full_by_id(issue).await?;
        let rw = self.repo_with_owner_by_id(Id(i.issue.repo_id)).await?;
        let mut users: Vec<i64> = vec![i.author.id];
        users.extend(i.assignees.iter().map(|a| a.id));
        for c in self.comments(issue, None).await? {
            users.push(c.author_id);
        }
        for e in self.issue_events(issue).await? {
            if e.event == "mentioned"
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&e.payload)
                && let Some(uid) = v.get("user_id").and_then(|x| x.as_i64())
            {
                users.push(uid);
            }
        }
        if i.issue.is_pull() {
            for r in self.review_requests(issue).await.unwrap_or_default() {
                users.push(r.id);
            }
        }
        users.sort_unstable();
        users.dedup();
        let kind = if i.issue.is_pull() { "pull" } else { "issue" };
        let url = format!("/{}/{}/{}", rw.full_name(), if i.issue.is_pull() { "pull" } else { "issues" }, i.issue.number);
        let title = format!("{} · {}#{}", i.issue.title, rw.full_name(), i.issue.number);
        for u in users {
            self.notify(Notify {
                user: Id(u),
                actor,
                repo: Some(rw.repo.id()),
                subject_type: kind,
                subject_id: Some(issue.0),
                reason,
                title: &title,
                url: &url,
            })
            .await?;
        }
        Ok(())
    }

    /// Notify one person about an issue or pull request (mention, assignment, review request).
    pub async fn notify_user_about_issue(&self, user: Id, issue: Id, actor: Option<Id>, reason: &str) -> Result<()> {
        let i = self.issue_full_by_id(issue).await?;
        let rw = self.repo_with_owner_by_id(Id(i.issue.repo_id)).await?;
        let kind = if i.issue.is_pull() { "pull" } else { "issue" };
        let url = format!("/{}/{}/{}", rw.full_name(), if i.issue.is_pull() { "pull" } else { "issues" }, i.issue.number);
        let title = format!("{} · {}#{}", i.issue.title, rw.full_name(), i.issue.number);
        self.notify(Notify {
            user,
            actor,
            repo: Some(rw.repo.id()),
            subject_type: kind,
            subject_id: Some(issue.0),
            reason,
            title: &title,
            url: &url,
        })
        .await
    }

    pub async fn notifications(&self, user: Id, unread_only: bool, limit: i64) -> Result<Vec<Notification>> {
        let extra = if unread_only { "AND unread = 1" } else { "" };
        sqlx::query_as::<_, Notification>(AssertSqlSafe(format!(
            "SELECT {COLS} FROM notifications WHERE user_id = $1 {extra} ORDER BY updated_at DESC LIMIT {limit}"
        )))
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn unread_count(&self, user: Id) -> Result<i64> {
        sqlx::query("SELECT COUNT(*) AS n FROM notifications WHERE user_id = $1 AND unread = 1")
            .bind(user.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("n")
            .map_err(map_err)
    }

    pub async fn mark_read(&self, user: Id, id: Option<Id>) -> Result<()> {
        match id {
            Some(i) => sqlx::query("UPDATE notifications SET unread = 0, updated_at = updated_at WHERE user_id = $1 AND id = $2")
                .bind(user.0)
                .bind(i.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?,
            None => sqlx::query("UPDATE notifications SET unread = 0 WHERE user_id = $1")
                .bind(user.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?,
        };
        Ok(())
    }

    /// Mark the thread about a subject as read (opening the issue page).
    pub async fn mark_subject_read(&self, user: Id, subject_type: &str, subject_id: i64) -> Result<()> {
        sqlx::query("UPDATE notifications SET unread = 0 WHERE user_id = $1 AND subject_type = $2 AND subject_id = $3")
            .bind(user.0)
            .bind(subject_type)
            .bind(subject_id)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn delete_notification(&self, user: Id, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM notifications WHERE user_id = $1 AND id = $2")
            .bind(user.0)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- site settings -----

    pub async fn setting(&self, key: &str) -> Result<Option<String>> {
        sqlx::query("SELECT value FROM site_settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?
            .map(|r| r.try_get::<String, _>("value"))
            .transpose()
            .map_err(map_err)
    }

    pub async fn settings(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT key, value FROM site_settings ORDER BY key").fetch_all(&self.pool).await.map_err(map_err)?;
        rows.iter()
            .map(|r| Ok((r.try_get::<String, _>("key").map_err(map_err)?, r.try_get::<String, _>("value").map_err(map_err)?)))
            .collect()
    }

    pub async fn set_setting(&self, key: &str, value: Option<&str>, by: Option<Id>) -> Result<()> {
        match value {
            None => {
                sqlx::query("DELETE FROM site_settings WHERE key = $1").bind(key).execute(&self.pool).await.map_err(map_err)?;
            }
            Some(v) => {
                sqlx::query("INSERT INTO site_settings (key, value, updated_at, updated_by) VALUES ($1, $2, $3, $4) ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = $3, updated_by = $4").bind(key).bind(v).bind(now()).bind(by.map(|b| b.0)).execute(&self.pool).await.map_err(map_err)?;
            }
        }
        Ok(())
    }

    // ----- activity feed -----

    /// Recent events across a set of repositories, newest first.
    pub async fn feed(&self, repos: &[i64], limit: i64) -> Result<Vec<crate::events::RepoEvent>> {
        if repos.is_empty() {
            return Ok(vec![]);
        }
        let list = repos.iter().take(500).map(|r| r.to_string()).collect::<Vec<_>>().join(",");
        sqlx::query_as::<_, crate::events::RepoEvent>(AssertSqlSafe(format!("SELECT id, repo_id, actor_id, kind, payload, created_at FROM repo_events WHERE repo_id IN ({list}) ORDER BY id DESC LIMIT {limit}"))).fetch_all(&self.pool).await.map_err(map_err)
    }

    /// Repository ids a user has a relationship with: owned, member-of-org, collaborator,
    /// starred, watched.
    pub async fn repos_of_interest(&self, user: Id) -> Result<Vec<i64>> {
        let mut ids: Vec<i64> = Vec::new();
        let rows = sqlx::query(
            "SELECT id FROM repositories WHERE owner_id = $1 \
             UNION SELECT r.id FROM repositories r JOIN org_members m ON m.org_id = r.owner_id WHERE m.user_id = $1 \
             UNION SELECT repo_id FROM collaborators WHERE user_id = $1 \
             UNION SELECT repo_id FROM stars WHERE user_id = $1 \
             UNION SELECT repo_id FROM watches WHERE user_id = $1 AND ignored = 0",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        for r in rows {
            ids.push(r.try_get::<i64, _>("id").map_err(map_err)?);
        }
        Ok(ids)
    }

    /// Everything public plus the user's own, for the admin overview and explore lists.
    pub async fn recent_public_repos(&self, limit: i64) -> Result<Vec<crate::repos::RepoWithOwner>> {
        let rows =
            sqlx::query(AssertSqlSafe(format!("SELECT id FROM repositories WHERE private = 0 ORDER BY updated_at DESC LIMIT {limit}")))
                .fetch_all(&self.pool)
                .await
                .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(r.try_get::<i64, _>("id").map_err(map_err)?)).await {
                out.push(rw);
            }
        }
        Ok(out)
    }

    /// All repositories, newest first (admin console).
    pub async fn all_repos(&self, offset: i64, limit: i64) -> Result<Vec<crate::repos::RepoWithOwner>> {
        let rows =
            sqlx::query(AssertSqlSafe(format!("SELECT id FROM repositories ORDER BY created_at DESC LIMIT {limit} OFFSET {offset}")))
                .fetch_all(&self.pool)
                .await
                .map_err(map_err)?;
        let mut out = Vec::new();
        for r in rows {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(r.try_get::<i64, _>("id").map_err(map_err)?)).await {
                out.push(rw);
            }
        }
        Ok(out)
    }

    pub async fn set_suspended(&self, id: Id, suspended: bool) -> Result<()> {
        sqlx::query("UPDATE accounts SET suspended = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(crate::b(suspended))
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if suspended {
            sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        }
        Ok(())
    }

    pub async fn count_rows(&self, table: &str) -> Result<i64> {
        if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Ok(0);
        }
        sqlx::query(AssertSqlSafe(format!("SELECT COUNT(*) AS n FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("n")
            .map_err(map_err)
    }
}
