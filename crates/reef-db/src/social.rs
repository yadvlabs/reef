//! Stars, watching (subscriptions), and pinned repositories.

use crate::{Db, map_err, now};
use reef_core::{Id, Result};
use sqlx::{AssertSqlSafe, Row};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Watch {
    pub user_id: i64,
    pub repo_id: i64,
    pub ignored: i64,
    pub created_at: i64,
}

impl Db {
    // ----- stars -----

    pub async fn is_starred(&self, user: Id, repo: Id) -> Result<bool> {
        let n: i64 = sqlx::query("SELECT COUNT(*) AS n FROM stars WHERE user_id = $1 AND repo_id = $2")
            .bind(user.0)
            .bind(repo.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("n")
            .map_err(map_err)?;
        Ok(n > 0)
    }

    pub async fn star(&self, user: Id, repo: Id) -> Result<()> {
        let r =
            sqlx::query("INSERT INTO stars (user_id, repo_id, created_at) VALUES ($1, $2, $3) ON CONFLICT (user_id, repo_id) DO NOTHING")
                .bind(user.0)
                .bind(repo.0)
                .bind(now())
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        if r.rows_affected() > 0 {
            self.recount_stars(repo).await?;
        }
        Ok(())
    }

    pub async fn unstar(&self, user: Id, repo: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM stars WHERE user_id = $1 AND repo_id = $2")
            .bind(user.0)
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() > 0 {
            self.recount_stars(repo).await?;
        }
        Ok(())
    }

    async fn recount_stars(&self, repo: Id) -> Result<()> {
        sqlx::query("UPDATE repositories SET stars_count = (SELECT COUNT(*) FROM stars WHERE repo_id = $1) WHERE id = $1")
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Accounts that starred a repository, newest first.
    pub async fn stargazers(&self, repo: Id) -> Result<Vec<crate::accounts::Account>> {
        let ids: Vec<i64> = sqlx::query("SELECT user_id FROM stars WHERE repo_id = $1 ORDER BY created_at DESC")
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
            .iter()
            .map(|r| r.try_get::<i64, _>("user_id"))
            .collect::<std::result::Result<_, _>>()
            .map_err(map_err)?;
        let mut out = Vec::new();
        for id in ids {
            if let Ok(a) = self.account_by_id(Id(id)).await {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// Repositories a user starred, newest star first.
    pub async fn starred_repos(&self, user: Id) -> Result<Vec<crate::repos::RepoWithOwner>> {
        let ids: Vec<i64> = sqlx::query("SELECT repo_id FROM stars WHERE user_id = $1 ORDER BY created_at DESC")
            .bind(user.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
            .iter()
            .map(|r| r.try_get::<i64, _>("repo_id"))
            .collect::<std::result::Result<_, _>>()
            .map_err(map_err)?;
        let mut out = Vec::new();
        for id in ids {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(id)).await {
                out.push(rw);
            }
        }
        Ok(out)
    }

    // ----- watching -----

    pub async fn watch_state(&self, user: Id, repo: Id) -> Result<Option<Watch>> {
        sqlx::query_as::<_, Watch>("SELECT user_id, repo_id, ignored, created_at FROM watches WHERE user_id = $1 AND repo_id = $2")
            .bind(user.0)
            .bind(repo.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn set_watch(&self, user: Id, repo: Id, ignored: bool) -> Result<Watch> {
        sqlx::query("INSERT INTO watches (user_id, repo_id, ignored, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (user_id, repo_id) DO UPDATE SET ignored = $3")
            .bind(user.0)
            .bind(repo.0)
            .bind(crate::b(ignored))
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.recount_watchers(repo).await?;
        Ok(self.watch_state(user, repo).await?.expect("just inserted"))
    }

    pub async fn unwatch(&self, user: Id, repo: Id) -> Result<()> {
        sqlx::query("DELETE FROM watches WHERE user_id = $1 AND repo_id = $2")
            .bind(user.0)
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.recount_watchers(repo).await
    }

    async fn recount_watchers(&self, repo: Id) -> Result<()> {
        sqlx::query(
            "UPDATE repositories SET watchers_count = (SELECT COUNT(*) FROM watches WHERE repo_id = $1 AND ignored = 0) WHERE id = $1",
        )
        .bind(repo.0)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    pub async fn watchers(&self, repo: Id) -> Result<Vec<crate::accounts::Account>> {
        let ids: Vec<i64> = sqlx::query("SELECT user_id FROM watches WHERE repo_id = $1 AND ignored = 0 ORDER BY created_at DESC")
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
            .iter()
            .map(|r| r.try_get::<i64, _>("user_id"))
            .collect::<std::result::Result<_, _>>()
            .map_err(map_err)?;
        let mut out = Vec::new();
        for id in ids {
            if let Ok(a) = self.account_by_id(Id(id)).await {
                out.push(a);
            }
        }
        Ok(out)
    }

    pub async fn watched_repos(&self, user: Id) -> Result<Vec<crate::repos::RepoWithOwner>> {
        let ids: Vec<i64> = sqlx::query("SELECT repo_id FROM watches WHERE user_id = $1 AND ignored = 0 ORDER BY created_at DESC")
            .bind(user.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
            .iter()
            .map(|r| r.try_get::<i64, _>("repo_id"))
            .collect::<std::result::Result<_, _>>()
            .map_err(map_err)?;
        let mut out = Vec::new();
        for id in ids {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(id)).await {
                out.push(rw);
            }
        }
        Ok(out)
    }

    // ----- pins -----

    pub async fn pinned_repos(&self, owner: Id) -> Result<Vec<crate::repos::RepoWithOwner>> {
        let ids: Vec<i64> = sqlx::query("SELECT repo_id FROM pinned_repos WHERE owner_id = $1 ORDER BY position")
            .bind(owner.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
            .iter()
            .map(|r| r.try_get::<i64, _>("repo_id"))
            .collect::<std::result::Result<_, _>>()
            .map_err(map_err)?;
        let mut out = Vec::new();
        for id in ids {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(id)).await {
                out.push(rw);
            }
        }
        Ok(out)
    }

    /// Replace the pinned list (at most six, in order).
    pub async fn set_pinned_repos(&self, owner: Id, repos: &[Id]) -> Result<()> {
        sqlx::query("DELETE FROM pinned_repos WHERE owner_id = $1").bind(owner.0).execute(&self.pool).await.map_err(map_err)?;
        for (i, r) in repos.iter().take(6).enumerate() {
            sqlx::query("INSERT INTO pinned_repos (owner_id, repo_id, position) VALUES ($1, $2, $3)")
                .bind(owner.0)
                .bind(r.0)
                .bind(i as i64)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        Ok(())
    }

    // ----- forks -----

    pub async fn mark_fork(&self, repo: Id, parent: Id) -> Result<()> {
        sqlx::query("UPDATE repositories SET is_fork = 1, parent_id = $2 WHERE id = $1")
            .bind(repo.0)
            .bind(parent.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query("UPDATE repositories SET forks_count = (SELECT COUNT(*) FROM repositories WHERE parent_id = $1) WHERE id = $1")
            .bind(parent.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn forks_of(&self, parent: Id) -> Result<Vec<crate::repos::RepoWithOwner>> {
        let ids: Vec<i64> =
            sqlx::query(AssertSqlSafe("SELECT id FROM repositories WHERE parent_id = $1 ORDER BY created_at DESC".to_string()))
                .bind(parent.0)
                .fetch_all(&self.pool)
                .await
                .map_err(map_err)?
                .iter()
                .map(|r| r.try_get::<i64, _>("id"))
                .collect::<std::result::Result<_, _>>()
                .map_err(map_err)?;
        let mut out = Vec::new();
        for id in ids {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(id)).await {
                out.push(rw);
            }
        }
        Ok(out)
    }

    /// A user's existing fork of a repository, if any.
    pub async fn fork_by_owner(&self, parent: Id, owner: Id) -> Result<Option<crate::repos::RepoWithOwner>> {
        let id: Option<i64> = sqlx::query("SELECT id FROM repositories WHERE parent_id = $1 AND owner_id = $2")
            .bind(parent.0)
            .bind(owner.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?
            .map(|r| r.try_get::<i64, _>("id"))
            .transpose()
            .map_err(map_err)?;
        match id {
            Some(i) => Ok(self.repo_with_owner_by_id(Id(i)).await.ok()),
            None => Ok(None),
        }
    }

    // ----- avatars -----

    pub async fn touch_avatar(&self, id: Id, uploaded: bool) -> Result<()> {
        sqlx::query("UPDATE accounts SET avatar_updated_at = $2, avatar_url = NULL, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(if uploaded { Some(now()) } else { None })
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// External avatar (from the identity provider); ignored once a picture was uploaded.
    pub async fn set_external_avatar(&self, id: Id, url: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE accounts SET avatar_url = $2 WHERE id = $1 AND avatar_updated_at IS NULL")
            .bind(id.0)
            .bind(url)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }
}
