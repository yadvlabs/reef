//! Git LFS objects (metadata; bytes live under `<data_dir>/lfs`) and file locks.

use crate::{Db, map_err, now};
use reef_core::{Id, Result};
use std::collections::HashSet;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LfsObject {
    pub repo_id: i64,
    pub oid: String,
    pub size: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LfsLock {
    pub id: i64,
    pub repo_id: i64,
    pub path: String,
    pub owner_id: i64,
    pub created_at: i64,
}

impl Db {
    pub async fn lfs_object(&self, repo: Id, oid: &str) -> Result<Option<LfsObject>> {
        sqlx::query_as::<_, LfsObject>("SELECT repo_id, oid, size, created_at FROM lfs_objects WHERE repo_id = $1 AND oid = $2")
            .bind(repo.0)
            .bind(oid)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Which of `oids` already exist for the repository (with their sizes).
    pub async fn lfs_objects_present(&self, repo: Id, oids: &[String]) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for chunk in oids.chunks(200) {
            let placeholders: Vec<String> = (0..chunk.len()).map(|i| format!("${}", i + 2)).collect();
            let sql = format!("SELECT oid FROM lfs_objects WHERE repo_id = $1 AND oid IN ({})", placeholders.join(", "));
            let mut q = sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql)).bind(repo.0);
            for o in chunk {
                q = q.bind(o);
            }
            for oid in q.fetch_all(&self.pool).await.map_err(map_err)? {
                out.insert(oid);
            }
        }
        Ok(out)
    }

    pub async fn insert_lfs_object(&self, repo: Id, oid: &str, size: i64) -> Result<()> {
        sqlx::query("INSERT INTO lfs_objects (repo_id, oid, size, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (repo_id, oid) DO UPDATE SET size = EXCLUDED.size")
            .bind(repo.0)
            .bind(oid)
            .bind(size)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// `(object count, total bytes)` for a repository.
    pub async fn lfs_stats(&self, repo: Id) -> Result<(i64, i64)> {
        let row: (i64, Option<i64>) = sqlx::query_as("SELECT COUNT(*), SUM(size) FROM lfs_objects WHERE repo_id = $1")
            .bind(repo.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((row.0, row.1.unwrap_or(0)))
    }

    pub async fn lfs_objects_of(&self, repo: Id, limit: i64) -> Result<Vec<LfsObject>> {
        sqlx::query_as::<_, LfsObject>(sqlx::AssertSqlSafe(format!(
            "SELECT repo_id, oid, size, created_at FROM lfs_objects WHERE repo_id = $1 ORDER BY created_at DESC LIMIT {limit}"
        )))
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn delete_lfs_objects(&self, repo: Id) -> Result<()> {
        sqlx::query("DELETE FROM lfs_objects WHERE repo_id = $1").bind(repo.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("DELETE FROM lfs_locks WHERE repo_id = $1").bind(repo.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    /// Repository ids that have LFS rows; used to sweep objects of deleted repositories.
    pub async fn lfs_repo_ids(&self) -> Result<Vec<i64>> {
        sqlx::query_scalar::<_, i64>("SELECT DISTINCT repo_id FROM lfs_objects").fetch_all(&self.pool).await.map_err(map_err)
    }

    pub async fn repo_exists(&self, repo: Id) -> Result<bool> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM repositories WHERE id = $1")
            .bind(repo.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(n > 0)
    }

    // ----- locks -----

    pub async fn create_lfs_lock(&self, repo: Id, path: &str, owner: Id) -> Result<LfsLock> {
        let id = reef_core::id::next();
        sqlx::query("INSERT INTO lfs_locks (id, repo_id, path, owner_id, created_at) VALUES ($1, $2, $3, $4, $5)")
            .bind(id.0)
            .bind(repo.0)
            .bind(path)
            .bind(owner.0)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.lfs_lock(repo, id).await?.ok_or(reef_core::Error::NotFound)
    }

    pub async fn lfs_lock(&self, repo: Id, id: Id) -> Result<Option<LfsLock>> {
        sqlx::query_as::<_, LfsLock>("SELECT id, repo_id, path, owner_id, created_at FROM lfs_locks WHERE repo_id = $1 AND id = $2")
            .bind(repo.0)
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn lfs_lock_by_path(&self, repo: Id, path: &str) -> Result<Option<LfsLock>> {
        sqlx::query_as::<_, LfsLock>("SELECT id, repo_id, path, owner_id, created_at FROM lfs_locks WHERE repo_id = $1 AND path = $2")
            .bind(repo.0)
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Locks of a repository, oldest first, optionally filtered by path; `after` is an id cursor.
    pub async fn lfs_locks(&self, repo: Id, path: Option<&str>, after: Option<i64>, limit: i64) -> Result<Vec<LfsLock>> {
        let limit = limit.clamp(1, 1000);
        let sql = format!(
            "SELECT id, repo_id, path, owner_id, created_at FROM lfs_locks WHERE repo_id = $1 AND ($2 IS NULL OR path = $2) AND ($3 IS NULL OR id > $3) ORDER BY id LIMIT {limit}"
        );
        sqlx::query_as::<_, LfsLock>(sqlx::AssertSqlSafe(sql))
            .bind(repo.0)
            .bind(path)
            .bind(after)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn delete_lfs_lock(&self, repo: Id, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM lfs_locks WHERE repo_id = $1 AND id = $2")
            .bind(repo.0)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 {
            return Err(reef_core::Error::NotFound);
        }
        Ok(())
    }
}
