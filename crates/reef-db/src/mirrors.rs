//! Pull mirrors (periodic fetch from another host), push mirrors (replicate every push)
//! and source imports into empty repositories.

use crate::{Db, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::Row;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MirrorConfig {
    pub repo_id: i64,
    pub pull_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub interval_minutes: i64,
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
    pub next_sync_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Lease held by the instance syncing it (epoch ms); `None` or in the past = free.
    pub sync_lock_until: Option<i64>,
    /// A "sync now" arrived while a sync was running: the holder syncs again when it is done.
    pub sync_requested: i64,
}

const MIRROR_COLS: &str = "repo_id, pull_url, username, password, interval_minutes, last_synced_at, last_error, next_sync_at, created_at, updated_at, sync_lock_until, sync_requested";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PushMirror {
    pub id: i64,
    pub repo_id: i64,
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub pending: i64,
    pub last_attempt_at: Option<i64>,
    pub last_pushed_at: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
    /// Lease held by the instance pushing it (epoch ms); `None` or in the past = free.
    pub lock_until: Option<i64>,
    /// Bumped by every "needs a push" mark; a push clears `pending` only for the generation
    /// it claimed (see `PushClaim`).
    pub pending_gen: i64,
}

const PUSH_COLS: &str =
    "id, repo_id, url, username, password, pending, last_attempt_at, last_pushed_at, last_error, created_at, lock_until, pending_gen";

/// A won push-mirror lease: the lease value (for renewal and release) and the pending
/// generation it covers. The push starts after the claim, so it carries every change marked
/// up to `generation`; a mark that arrives later bumps the generation and keeps the mirror pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushClaim {
    pub lease: i64,
    pub generation: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepoImport {
    pub repo_id: i64,
    pub vcs: String,
    pub vcs_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    /// `importing` | `complete` | `error` | `cancelled`
    pub status: String,
    pub message: Option<String>,
    pub commit_count: Option<i64>,
    pub actor_id: Option<i64>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

const IMPORT_COLS: &str = "repo_id, vcs, vcs_url, username, password, status, message, commit_count, actor_id, started_at, finished_at";

pub struct MirrorPatch<'a> {
    pub pull_url: &'a str,
    pub username: Option<&'a str>,
    /// `None` keeps the stored password.
    pub password: Option<Option<&'a str>>,
    pub interval_minutes: i64,
}

impl Db {
    pub async fn mirror_config(&self, repo: Id) -> Result<Option<MirrorConfig>> {
        sqlx::query_as::<_, MirrorConfig>(sqlx::AssertSqlSafe(format!("SELECT {MIRROR_COLS} FROM mirror_config WHERE repo_id = $1")))
            .bind(repo.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn set_mirror_config(&self, repo: Id, p: MirrorPatch<'_>) -> Result<MirrorConfig> {
        let ts = now();
        let interval = p.interval_minutes.clamp(1, 60 * 24 * 30);
        let existing = self.mirror_config(repo).await?;
        let password = match p.password {
            Some(pw) => pw.map(String::from),
            None => existing.as_ref().and_then(|e| e.password.clone()),
        };
        if existing.is_some() {
            // A change while a sync holds the lease is remembered like a "sync now": the holder
            // (which fetched from the old source) syncs again as soon as it is done, instead of
            // rescheduling on the interval.
            sqlx::query("UPDATE mirror_config SET pull_url = $1, username = $2, password = $3, interval_minutes = $4, next_sync_at = $5, updated_at = $5, sync_requested = 1 WHERE repo_id = $6")
                .bind(p.pull_url)
                .bind(p.username)
                .bind(password)
                .bind(interval)
                .bind(ts)
                .bind(repo.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        } else {
            sqlx::query("INSERT INTO mirror_config (repo_id, pull_url, username, password, interval_minutes, next_sync_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6, $6)")
                .bind(repo.0)
                .bind(p.pull_url)
                .bind(p.username)
                .bind(password)
                .bind(interval)
                .bind(ts)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        self.mirror_config(repo).await?.ok_or(Error::NotFound)
    }

    pub async fn delete_mirror_config(&self, repo: Id) -> Result<()> {
        sqlx::query("DELETE FROM mirror_config WHERE repo_id = $1").bind(repo.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    /// Take the lease on a due mirror for `lease_ms`. Exactly one caller wins while the lease
    /// is held, whichever instance it runs on. Returns the lease (its expiry) the winner holds:
    /// renewing and releasing need it back, so a holder whose lease expired and was taken over
    /// can neither extend nor release the new holder's.
    pub async fn claim_mirror(&self, repo: Id, lease_ms: i64) -> Result<Option<i64>> {
        let ts = now();
        let until = ts + lease_ms;
        // The sync starting now honours every "sync now" made before it.
        let r = sqlx::query("UPDATE mirror_config SET sync_lock_until = $2, sync_requested = 0 WHERE repo_id = $1 AND (next_sync_at IS NULL OR next_sync_at <= $3) AND (sync_lock_until IS NULL OR sync_lock_until < $3)")
            .bind(repo.0)
            .bind(until)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((r.rows_affected() == 1).then_some(until))
    }

    /// Extend the lease `held` by `lease_ms` from now. `None` when it is no longer ours.
    pub async fn renew_mirror_lease(&self, repo: Id, held: i64, lease_ms: i64) -> Result<Option<i64>> {
        let until = now() + lease_ms;
        let r = sqlx::query("UPDATE mirror_config SET sync_lock_until = $3 WHERE repo_id = $1 AND sync_lock_until = $2")
            .bind(repo.0)
            .bind(held)
            .bind(until)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((r.rows_affected() == 1).then_some(until))
    }

    /// Record the outcome of a sync, schedule the next one and release the lease `held`.
    /// False (nothing recorded) when the lease is no longer ours. A "sync now" that arrived
    /// while the sync ran (`sync_requested`) keeps the mirror due immediately, so the holder
    /// (or any instance's next tick) syncs again.
    pub async fn mirror_synced(&self, repo: Id, held: i64, error: Option<&str>) -> Result<bool> {
        let ts = now();
        // Failed syncs retry sooner (10 minutes) than the configured interval.
        let r = sqlx::query("UPDATE mirror_config SET last_synced_at = CASE WHEN $1 IS NULL THEN $2 ELSE last_synced_at END, last_error = $1, next_sync_at = CASE WHEN sync_requested <> 0 THEN 0 ELSE $2 + CASE WHEN $1 IS NULL THEN interval_minutes * 60000 ELSE 600000 END END, updated_at = $2, sync_lock_until = NULL WHERE repo_id = $3 AND sync_lock_until = $4")
            .bind(error)
            .bind(ts)
            .bind(repo.0)
            .bind(held)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(r.rows_affected() == 1)
    }

    /// Request an immediate sync. The request is remembered (`sync_requested`) until a sync
    /// starts after it, so one arriving while another instance syncs is not lost.
    pub async fn mirror_sync_now(&self, repo: Id) -> Result<()> {
        sqlx::query("UPDATE mirror_config SET next_sync_at = $1, sync_requested = 1 WHERE repo_id = $2")
            .bind(0i64)
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Mirrors due for a sync that no instance holds a lease on.
    pub async fn due_mirrors(&self, limit: i64) -> Result<Vec<MirrorConfig>> {
        sqlx::query_as::<_, MirrorConfig>(sqlx::AssertSqlSafe(format!(
            "SELECT {MIRROR_COLS} FROM mirror_config WHERE (next_sync_at IS NULL OR next_sync_at <= $1) AND (sync_lock_until IS NULL OR sync_lock_until < $1) ORDER BY next_sync_at LIMIT {limit}"
        )))
        .bind(now())
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    // ----- push mirrors -----

    pub async fn push_mirrors(&self, repo: Id) -> Result<Vec<PushMirror>> {
        sqlx::query_as::<_, PushMirror>(sqlx::AssertSqlSafe(format!("SELECT {PUSH_COLS} FROM push_mirrors WHERE repo_id = $1 ORDER BY id")))
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn push_mirror(&self, repo: Id, id: Id) -> Result<PushMirror> {
        sqlx::query_as::<_, PushMirror>(sqlx::AssertSqlSafe(format!("SELECT {PUSH_COLS} FROM push_mirrors WHERE repo_id = $1 AND id = $2")))
            .bind(repo.0)
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn create_push_mirror(&self, repo: Id, url: &str, username: Option<&str>, password: Option<&str>) -> Result<PushMirror> {
        let id = reef_core::id::next();
        sqlx::query(
            "INSERT INTO push_mirrors (id, repo_id, url, username, password, pending, created_at) VALUES ($1, $2, $3, $4, $5, 1, $6)",
        )
        .bind(id.0)
        .bind(repo.0)
        .bind(url)
        .bind(username)
        .bind(password)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        self.push_mirror(repo, id).await
    }

    pub async fn delete_push_mirror(&self, repo: Id, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM push_mirrors WHERE repo_id = $1 AND id = $2")
            .bind(repo.0)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    /// Mark every push mirror of a repository as needing a push.
    pub async fn push_mirrors_mark_pending(&self, repo: Id) -> Result<()> {
        sqlx::query("UPDATE push_mirrors SET pending = 1, pending_gen = pending_gen + 1 WHERE repo_id = $1")
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn push_mirror_mark_pending(&self, repo: Id, id: Id) -> Result<()> {
        sqlx::query("UPDATE push_mirrors SET pending = 1, pending_gen = pending_gen + 1 WHERE repo_id = $1 AND id = $2")
            .bind(repo.0)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Take the lease on a pending push mirror for `lease_ms`; one instance pushes at a time.
    /// Returns the lease the winner holds (see `claim_mirror`) and the pending generation it
    /// covers. A caller that loses (the lease is held) leaves `pending` set: the holder, or
    /// the next tick, pushes again.
    pub async fn claim_push_mirror(&self, id: Id, lease_ms: i64) -> Result<Option<PushClaim>> {
        let ts = now();
        let until = ts + lease_ms;
        let r = sqlx::query(
            "UPDATE push_mirrors SET lock_until = $2 WHERE id = $1 AND pending = 1 AND (lock_until IS NULL OR lock_until < $3)",
        )
        .bind(id.0)
        .bind(until)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        if r.rows_affected() != 1 {
            return Ok(None);
        }
        // Read after the claim, before the push starts: marks up to here are in the push.
        let generation: Option<i64> = sqlx::query("SELECT pending_gen FROM push_mirrors WHERE id = $1 AND lock_until = $2")
            .bind(id.0)
            .bind(until)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?
            .map(|r| r.try_get("pending_gen"))
            .transpose()
            .map_err(map_err)?;
        Ok(generation.map(|generation| PushClaim { lease: until, generation }))
    }

    /// Extend the push-mirror lease `held` by `lease_ms` from now. `None` when it is no longer ours.
    pub async fn renew_push_mirror_lease(&self, id: Id, held: i64, lease_ms: i64) -> Result<Option<i64>> {
        let until = now() + lease_ms;
        let r = sqlx::query("UPDATE push_mirrors SET lock_until = $3 WHERE id = $1 AND lock_until = $2")
            .bind(id.0)
            .bind(held)
            .bind(until)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((r.rows_affected() == 1).then_some(until))
    }

    /// Record the outcome of a push and release the lease `held`. False when it is no longer
    /// ours. A successful push clears `pending` only if no mark newer than `generation` (the
    /// generation its claim covered) arrived while it ran; otherwise the mirror stays pending
    /// and is pushed again.
    pub async fn push_mirror_result(&self, id: Id, held: i64, generation: i64, error: Option<&str>) -> Result<bool> {
        let ts = now();
        let r = sqlx::query("UPDATE push_mirrors SET pending = CASE WHEN $1 IS NULL AND pending_gen = $5 THEN 0 ELSE pending END, last_attempt_at = $2, last_pushed_at = CASE WHEN $1 IS NULL THEN $2 ELSE last_pushed_at END, last_error = $1, lock_until = NULL WHERE id = $3 AND lock_until = $4")
            .bind(error)
            .bind(ts)
            .bind(id.0)
            .bind(held)
            .bind(generation)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(r.rows_affected() == 1)
    }

    /// Pending push mirrors that no instance holds a lease on and that are due: never tried,
    /// marked again after a successful push (no error), or failed longer than
    /// `retry_after_ms` ago.
    pub async fn pending_push_mirrors(&self, retry_after_ms: i64, limit: i64) -> Result<Vec<PushMirror>> {
        let ts = now();
        sqlx::query_as::<_, PushMirror>(sqlx::AssertSqlSafe(format!("SELECT {PUSH_COLS} FROM push_mirrors WHERE pending = 1 AND (last_attempt_at IS NULL OR last_attempt_at <= $1 OR last_error IS NULL) AND (lock_until IS NULL OR lock_until < $2) ORDER BY id LIMIT {limit}")))
            .bind(ts - retry_after_ms)
            .bind(ts)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    // ----- imports -----

    pub async fn repo_import(&self, repo: Id) -> Result<Option<RepoImport>> {
        sqlx::query_as::<_, RepoImport>(sqlx::AssertSqlSafe(format!("SELECT {IMPORT_COLS} FROM repo_imports WHERE repo_id = $1")))
            .bind(repo.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn start_repo_import(
        &self,
        repo: Id,
        vcs: &str,
        vcs_url: &str,
        username: Option<&str>,
        password: Option<&str>,
        actor: Option<Id>,
    ) -> Result<RepoImport> {
        let ts = now();
        sqlx::query("DELETE FROM repo_imports WHERE repo_id = $1").bind(repo.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("INSERT INTO repo_imports (repo_id, vcs, vcs_url, username, password, status, actor_id, started_at) VALUES ($1, $2, $3, $4, $5, 'importing', $6, $7)")
            .bind(repo.0)
            .bind(vcs)
            .bind(vcs_url)
            .bind(username)
            .bind(password)
            .bind(actor.map(|a| a.0))
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.repo_import(repo).await?.ok_or(Error::NotFound)
    }

    pub async fn finish_repo_import(&self, repo: Id, status: &str, message: Option<&str>, commit_count: Option<i64>) -> Result<()> {
        sqlx::query(
            "UPDATE repo_imports SET status = $1, message = $2, commit_count = $3, finished_at = $4, password = NULL WHERE repo_id = $5",
        )
        .bind(status)
        .bind(message)
        .bind(commit_count)
        .bind(now())
        .bind(repo.0)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    pub async fn delete_repo_import(&self, repo: Id) -> Result<()> {
        sqlx::query("DELETE FROM repo_imports WHERE repo_id = $1").bind(repo.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    /// Imports still marked as running that started before `older_than_ms` ago (stale after a restart).
    pub async fn stale_repo_imports(&self, older_than_ms: i64) -> Result<Vec<RepoImport>> {
        sqlx::query_as::<_, RepoImport>(sqlx::AssertSqlSafe(format!(
            "SELECT {IMPORT_COLS} FROM repo_imports WHERE status = 'importing' AND started_at <= $1 ORDER BY started_at LIMIT 10"
        )))
        .bind(now() - older_than_ms)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }
}
