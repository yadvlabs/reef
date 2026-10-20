//! Organization membership.

use crate::accounts::Account;
use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Membership {
    pub org_id: i64,
    pub user_id: i64,
    pub role: String,
    pub created_at: i64,
}

impl Db {
    pub async fn add_org_member(&self, org: Id, user: Id, role: &str) -> Result<()> {
        if !matches!(role, "admin" | "member") {
            return Err(Error::validation("role must be admin or member"));
        }
        sqlx::query(
            "INSERT INTO org_members (org_id, user_id, role, created_at) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (org_id, user_id) DO UPDATE SET role = $3",
        )
        .bind(org.0)
        .bind(user.0)
        .bind(role)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    pub async fn remove_org_member(&self, org: Id, user: Id) -> Result<()> {
        sqlx::query("DELETE FROM org_members WHERE org_id = $1 AND user_id = $2")
            .bind(org.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn org_membership(&self, org: Id, user: Id) -> Result<Option<Membership>> {
        sqlx::query_as::<_, Membership>("SELECT org_id, user_id, role, created_at FROM org_members WHERE org_id = $1 AND user_id = $2")
            .bind(org.0)
            .bind(user.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn org_members(&self, org: Id, role: Option<&str>) -> Result<Vec<(Account, String)>> {
        let rows = sqlx::query_as::<_, Membership>(
            "SELECT org_id, user_id, role, created_at FROM org_members WHERE org_id = $1 AND ($2 IS NULL OR role = $2) ORDER BY created_at",
        )
        .bind(org.0)
        .bind(role)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let ids: Vec<i64> = rows.iter().map(|m| m.user_id).collect();
        let accounts = self.accounts_by_ids(&ids).await?;
        Ok(rows.into_iter().filter_map(|m| accounts.iter().find(|a| a.id == m.user_id).cloned().map(|a| (a, m.role))).collect())
    }

    pub async fn user_orgs(&self, user: Id) -> Result<Vec<Account>> {
        let rows = sqlx::query_as::<_, Membership>(
            "SELECT org_id, user_id, role, created_at FROM org_members WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let ids: Vec<i64> = rows.iter().map(|m| m.org_id).collect();
        let mut accts = self.accounts_by_ids(&ids).await?;
        accts.sort_by_key(|a| a.login.to_lowercase());
        Ok(accts)
    }

    pub async fn count_org_members(&self, org: Id) -> Result<i64> {
        use sqlx::Row;
        let row = sqlx::query("SELECT COUNT(*) AS n FROM org_members WHERE org_id = $1")
            .bind(org.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        row.try_get::<i64, _>("n").map_err(map_err)
    }
}

impl Db {
    /// Remove an organization account and everything that hangs off it in the database.
    /// Repositories must already be deleted (the caller also removes their git storage).
    pub async fn delete_org(&self, org: Id) -> Result<()> {
        for sql in [
            "DELETE FROM org_members WHERE org_id = $1",
            "DELETE FROM pinned_repos WHERE owner_id = $1",
            "DELETE FROM external_identities WHERE user_id = $1",
            "DELETE FROM accounts WHERE id = $1 AND kind = 'org'",
        ] {
            sqlx::query(sql).bind(org.0).execute(&self.pool).await.map_err(map_err)?;
        }
        Ok(())
    }
}
