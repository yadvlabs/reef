//! Webhooks: hook definitions for repositories and organizations, the delivery log with
//! retry bookkeeping, and the outbox of recorded events the poller turns into deliveries.

use crate::{Backend, Db, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::{AssertSqlSafe, Row};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Webhook {
    pub id: i64,
    pub owner_kind: String,
    pub owner_id: i64,
    pub url: String,
    pub content_type: String,
    pub secret: Option<String>,
    pub events: String,
    pub active: i64,
    pub insecure_ssl: i64,
    pub last_status: Option<i64>,
    pub last_delivered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

const HOOK_COLS: &str = "id, owner_kind, owner_id, url, content_type, secret, events, active, insecure_ssl, last_status, last_delivered_at, created_at, updated_at";

impl Webhook {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
    pub fn events(&self) -> Vec<String> {
        serde_json::from_str(&self.events).unwrap_or_default()
    }
    pub fn is_active(&self) -> bool {
        self.active != 0
    }
    /// Whether the hook subscribes to `event` (`*` subscribes to everything).
    pub fn wants(&self, event: &str) -> bool {
        self.events().iter().any(|e| e == "*" || e == event)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HookInput {
    pub url: String,
    pub content_type: String,
    pub secret: Option<String>,
    pub events: Vec<String>,
    pub active: bool,
    pub insecure_ssl: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Delivery {
    pub id: i64,
    pub hook_id: i64,
    pub guid: String,
    pub event: String,
    pub action: Option<String>,
    pub payload: String,
    pub request_headers: String,
    pub response_status: Option<i64>,
    pub response_headers: Option<String>,
    pub response_body: Option<String>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub attempt: i64,
    pub next_attempt_at: Option<i64>,
    pub lock_until: Option<i64>,
    pub redelivery_of: Option<i64>,
    pub repo_id: Option<i64>,
    pub delivered_at: Option<i64>,
    pub created_at: i64,
}

const DELIVERY_COLS: &str = "id, hook_id, guid, event, action, payload, request_headers, response_status, response_headers, response_body, duration_ms, status, attempt, next_attempt_at, lock_until, redelivery_of, repo_id, delivered_at, created_at";

impl Delivery {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

/// A delivery to record. The poller builds these for claimed outbox entries and commits them
/// together with the entries' removal (`commit_outbox`), so an event is either fully turned
/// into deliveries or left for the next pass: nothing is lost when a process dies half way.
#[derive(Debug, Clone)]
pub struct NewDelivery {
    pub hook: Id,
    pub guid: String,
    pub event: String,
    pub action: Option<String>,
    pub payload: String,
    pub request_headers: String,
    pub repo: Option<Id>,
    pub redelivery_of: Option<Id>,
}

async fn insert_delivery<'e, E>(exec: E, id: Id, t: i64, d: &NewDelivery) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Any>,
{
    sqlx::query("INSERT INTO webhook_deliveries (id, hook_id, guid, event, action, payload, request_headers, status, attempt, next_attempt_at, redelivery_of, repo_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', 0, $8, $9, $10, $8)")
        .bind(id.0)
        .bind(d.hook.0)
        .bind(&d.guid)
        .bind(&d.event)
        .bind(d.action.as_deref())
        .bind(&d.payload)
        .bind(&d.request_headers)
        .bind(t)
        .bind(d.redelivery_of.map(|r| r.0))
        .bind(d.repo.map(|r| r.0))
        .execute(exec)
        .await
        .map_err(map_err)?;
    Ok(())
}

/// A row of `repo_events` read by the poller.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PolledRepoEvent {
    pub id: i64,
    pub repo_id: i64,
    pub actor_id: Option<i64>,
    pub kind: String,
    pub payload: String,
    pub created_at: i64,
}

/// A row of `issue_events` read by the poller.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PolledIssueEvent {
    pub id: i64,
    pub issue_id: i64,
    pub repo_id: i64,
    pub actor_id: Option<i64>,
    pub event: String,
    pub payload: String,
    pub created_at: i64,
}

/// A row of `issue_comments` read by the poller.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PolledComment {
    pub id: i64,
    pub issue_id: i64,
    pub repo_id: i64,
    pub author_id: i64,
    pub created_at: i64,
}

/// The event tables the outbox refers to (`webhook_outbox.source`).
pub const OUTBOX_REPO_EVENTS: &str = "repo_events";
pub const OUTBOX_ISSUE_EVENTS: &str = "issue_events";
pub const OUTBOX_ISSUE_COMMENTS: &str = "issue_comments";

/// One recorded event waiting to be turned into deliveries.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OutboxEntry {
    pub id: i64,
    /// `OUTBOX_REPO_EVENTS`, `OUTBOX_ISSUE_EVENTS` or `OUTBOX_ISSUE_COMMENTS`.
    pub source: String,
    pub event_id: i64,
    pub created_at: i64,
}

/// Outbox entries claimed by one poller pass; `token` identifies the claim to
/// `commit_outbox` and `release_outbox`.
#[derive(Debug, Clone)]
pub struct OutboxClaim {
    pub token: String,
    pub entries: Vec<OutboxEntry>,
}

/// Queue a just-recorded event for the webhook poller. Call it on the transaction that
/// inserts the event, so the entry commits exactly when the event does.
pub async fn enqueue_outbox<'e, E>(exec: E, source: &str, event_id: i64, at: i64) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Any>,
{
    sqlx::query("INSERT INTO webhook_outbox (id, source, event_id, created_at) VALUES ($1, $2, $3, $4)")
        .bind(reef_core::id::next().0)
        .bind(source)
        .bind(event_id)
        .bind(at)
        .execute(exec)
        .await
        .map_err(map_err)?;
    Ok(())
}

fn json_events(events: &[String]) -> String {
    serde_json::to_string(events).unwrap_or_else(|_| "[]".into())
}

impl Db {
    // ----- hooks -----

    pub async fn create_hook(&self, owner_kind: &str, owner_id: Id, h: &HookInput) -> Result<Webhook> {
        let id = reef_core::id::next();
        let t = now();
        sqlx::query("INSERT INTO webhooks (id, owner_kind, owner_id, url, content_type, secret, events, active, insecure_ssl, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)")
            .bind(id.0)
            .bind(owner_kind)
            .bind(owner_id.0)
            .bind(&h.url)
            .bind(&h.content_type)
            .bind(h.secret.as_deref())
            .bind(json_events(&h.events))
            .bind(h.active as i64)
            .bind(h.insecure_ssl as i64)
            .bind(t)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.hook_by_id(id).await
    }

    pub async fn update_hook(&self, id: Id, h: &HookInput) -> Result<Webhook> {
        sqlx::query("UPDATE webhooks SET url = $2, content_type = $3, secret = $4, events = $5, active = $6, insecure_ssl = $7, updated_at = $8 WHERE id = $1")
            .bind(id.0)
            .bind(&h.url)
            .bind(&h.content_type)
            .bind(h.secret.as_deref())
            .bind(json_events(&h.events))
            .bind(h.active as i64)
            .bind(h.insecure_ssl as i64)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.hook_by_id(id).await
    }

    pub async fn delete_hook(&self, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM webhooks WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        sqlx::query("DELETE FROM webhook_deliveries WHERE hook_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    pub async fn hook_by_id(&self, id: Id) -> Result<Webhook> {
        sqlx::query_as::<_, Webhook>(AssertSqlSafe(format!("SELECT {HOOK_COLS} FROM webhooks WHERE id = $1")))
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?
            .ok_or(Error::NotFound)
    }

    /// Hooks defined directly on a repository or organization.
    pub async fn hooks_for(&self, owner_kind: &str, owner_id: Id) -> Result<Vec<Webhook>> {
        sqlx::query_as::<_, Webhook>(AssertSqlSafe(format!(
            "SELECT {HOOK_COLS} FROM webhooks WHERE owner_kind = $1 AND owner_id = $2 ORDER BY id"
        )))
        .bind(owner_kind)
        .bind(owner_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Active hooks that receive a repository's events: its own plus its organization's.
    pub async fn active_hooks_for_repo(&self, repo: Id, org: Option<Id>) -> Result<Vec<Webhook>> {
        let mut out = sqlx::query_as::<_, Webhook>(AssertSqlSafe(format!(
            "SELECT {HOOK_COLS} FROM webhooks WHERE owner_kind = 'repo' AND owner_id = $1 AND active <> 0 ORDER BY id"
        )))
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        if let Some(o) = org {
            let more = sqlx::query_as::<_, Webhook>(AssertSqlSafe(format!(
                "SELECT {HOOK_COLS} FROM webhooks WHERE owner_kind = 'org' AND owner_id = $1 AND active <> 0 ORDER BY id"
            )))
            .bind(o.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
            out.extend(more);
        }
        Ok(out)
    }

    pub async fn set_hook_last(&self, id: Id, status: Option<i64>, at: i64) -> Result<()> {
        sqlx::query("UPDATE webhooks SET last_status = $2, last_delivered_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(status)
            .bind(at)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- deliveries -----

    #[allow(clippy::too_many_arguments)]
    pub async fn create_delivery(
        &self,
        hook: Id,
        guid: &str,
        event: &str,
        action: Option<&str>,
        payload: &str,
        request_headers: &str,
        repo: Option<Id>,
        redelivery_of: Option<Id>,
    ) -> Result<Delivery> {
        let id = reef_core::id::next();
        let d = NewDelivery {
            hook,
            guid: guid.into(),
            event: event.into(),
            action: action.map(String::from),
            payload: payload.into(),
            request_headers: request_headers.into(),
            repo,
            redelivery_of,
        };
        insert_delivery(&self.pool, id, now(), &d).await?;
        self.delivery_by_id(id).await
    }

    pub async fn delivery_by_id(&self, id: Id) -> Result<Delivery> {
        sqlx::query_as::<_, Delivery>(AssertSqlSafe(format!("SELECT {DELIVERY_COLS} FROM webhook_deliveries WHERE id = $1")))
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?
            .ok_or(Error::NotFound)
    }

    pub async fn deliveries_for_hook(&self, hook: Id, limit: i64) -> Result<Vec<Delivery>> {
        sqlx::query_as::<_, Delivery>(AssertSqlSafe(format!(
            "SELECT {DELIVERY_COLS} FROM webhook_deliveries WHERE hook_id = $1 ORDER BY id DESC LIMIT {}",
            limit.clamp(1, 500)
        )))
        .bind(hook.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Claim due deliveries for this process for `lock_ms`. Several instances sharing the
    /// database never post the same delivery twice: on PostgreSQL the due rows are selected
    /// `FOR UPDATE SKIP LOCKED` and updated in one statement; on SQLite (one writer) each
    /// candidate is taken with a guarded `UPDATE` that fails when someone else got there.
    /// A claim that outlives its lease must be renewed (`renew_delivery_lock`) or it becomes
    /// claimable again.
    pub async fn claim_deliveries(&self, limit: i64, lock_ms: i64) -> Result<Vec<Delivery>> {
        let t = now();
        let limit = limit.clamp(1, 200);
        let due = "status = 'pending' AND (next_attempt_at IS NULL OR next_attempt_at <= $1) AND (lock_until IS NULL OR lock_until < $1)";
        if self.backend == Backend::Postgres {
            return sqlx::query_as::<_, Delivery>(AssertSqlSafe(format!(
                "UPDATE webhook_deliveries SET lock_until = $2, attempt = attempt + 1 WHERE id IN (SELECT id FROM webhook_deliveries WHERE {due} ORDER BY id LIMIT {limit} FOR UPDATE SKIP LOCKED) RETURNING {DELIVERY_COLS}"
            )))
            .bind(t)
            .bind(t + lock_ms)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err);
        }
        let candidates = sqlx::query_as::<_, Delivery>(AssertSqlSafe(format!(
            "SELECT {DELIVERY_COLS} FROM webhook_deliveries WHERE {due} ORDER BY id LIMIT {limit}"
        )))
        .bind(t)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut claimed = Vec::new();
        for d in candidates {
            let r = sqlx::query("UPDATE webhook_deliveries SET lock_until = $2, attempt = attempt + 1 WHERE id = $1 AND status = 'pending' AND (lock_until IS NULL OR lock_until < $3)")
                .bind(d.id)
                .bind(t + lock_ms)
                .bind(t)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
            if r.rows_affected() == 1 {
                claimed.push(Delivery { attempt: d.attempt + 1, lock_until: Some(t + lock_ms), ..d });
            }
        }
        Ok(claimed)
    }

    /// Claim one delivery by id, the way `claim_deliveries` claims a batch: only when it is
    /// still pending and nobody holds its lease (the retry schedule is not consulted, so a
    /// ping, test or redelivery goes out now). `None` when it is delivered, failed, or leased
    /// by another instance, which will post it.
    pub async fn claim_delivery(&self, id: Id, lock_ms: i64) -> Result<Option<Delivery>> {
        let t = now();
        let r = sqlx::query("UPDATE webhook_deliveries SET lock_until = $2, attempt = attempt + 1 WHERE id = $1 AND status = 'pending' AND (lock_until IS NULL OR lock_until < $3)")
            .bind(id.0)
            .bind(t + lock_ms)
            .bind(t)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() != 1 {
            return Ok(None);
        }
        let d = self.delivery_by_id(id).await?;
        // Only ours if the lease is still the one just taken.
        Ok((d.lock_until == Some(t + lock_ms)).then_some(d))
    }

    /// Extend the lease on a claimed delivery by `lock_ms` from now. `held` is the
    /// `lock_until` the claim returned: when it no longer matches, the lease expired and
    /// another instance took the row, so the caller must not post it. Returns the new
    /// `lock_until`, or `None` when the claim is lost.
    pub async fn renew_delivery_lock(&self, id: Id, held: i64, lock_ms: i64) -> Result<Option<i64>> {
        let until = now() + lock_ms;
        let r = sqlx::query("UPDATE webhook_deliveries SET lock_until = $3 WHERE id = $1 AND status = 'pending' AND lock_until = $2")
            .bind(id.0)
            .bind(held)
            .bind(until)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((r.rows_affected() == 1).then_some(until))
    }

    /// Deliveries waiting to be posted (queued, or retrying after a failure).
    pub async fn count_pending_deliveries(&self) -> Result<i64> {
        sqlx::query("SELECT COUNT(*) AS n FROM webhook_deliveries WHERE status = 'pending'")
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("n")
            .map_err(map_err)
    }

    /// Record the outcome of an attempt. `next_attempt_at = Some(..)` keeps it pending.
    #[allow(clippy::too_many_arguments)]
    pub async fn finish_delivery(
        &self,
        id: Id,
        status: &str,
        response_status: Option<i64>,
        response_headers: Option<&str>,
        response_body: Option<&str>,
        duration_ms: i64,
        next_attempt_at: Option<i64>,
    ) -> Result<()> {
        let t = now();
        sqlx::query("UPDATE webhook_deliveries SET status = $2, response_status = $3, response_headers = $4, response_body = $5, duration_ms = $6, next_attempt_at = $7, lock_until = NULL, delivered_at = $8 WHERE id = $1")
            .bind(id.0)
            .bind(status)
            .bind(response_status)
            .bind(response_headers)
            .bind(response_body)
            .bind(duration_ms)
            .bind(next_attempt_at)
            .bind(t)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn purge_deliveries_before(&self, before: i64) -> Result<u64> {
        Ok(sqlx::query("DELETE FROM webhook_deliveries WHERE created_at < $1 AND status <> 'pending'")
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(map_err)?
            .rows_affected())
    }

    // ----- outbox -----

    /// Claim up to `limit` outbox entries, oldest first, for `lease_ms`. Several instances
    /// never hold the same entry: on PostgreSQL the free rows are selected `FOR UPDATE SKIP
    /// LOCKED` and updated in one statement; on SQLite each candidate is taken with a guarded
    /// `UPDATE`. An entry whose lease expired (its poller died) is claimable again.
    pub async fn claim_outbox(&self, limit: i64, lease_ms: i64) -> Result<OutboxClaim> {
        let t = now();
        let token = reef_core::token::random_hex(16);
        let limit = limit.clamp(1, 1000);
        let free = "lock_until IS NULL OR lock_until < $3";
        let mut entries = if self.backend == Backend::Postgres {
            sqlx::query_as::<_, OutboxEntry>(AssertSqlSafe(format!(
                "UPDATE webhook_outbox SET claim = $1, lock_until = $2 WHERE id IN (SELECT id FROM webhook_outbox WHERE {free} ORDER BY id LIMIT {limit} FOR UPDATE SKIP LOCKED) RETURNING id, source, event_id, created_at"
            )))
            .bind(&token)
            .bind(t + lease_ms)
            .bind(t)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?
        } else {
            let candidates = sqlx::query_as::<_, OutboxEntry>(AssertSqlSafe(format!(
                "SELECT id, source, event_id, created_at FROM webhook_outbox WHERE lock_until IS NULL OR lock_until < $1 ORDER BY id LIMIT {limit}"
            )))
            .bind(t)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
            let mut claimed = Vec::new();
            for e in candidates {
                let r =
                    sqlx::query(AssertSqlSafe(format!("UPDATE webhook_outbox SET claim = $1, lock_until = $2 WHERE id = $4 AND ({free})")))
                        .bind(&token)
                        .bind(t + lease_ms)
                        .bind(t)
                        .bind(e.id)
                        .execute(&self.pool)
                        .await
                        .map_err(map_err)?;
                if r.rows_affected() == 1 {
                    claimed.push(e);
                }
            }
            claimed
        };
        entries.sort_by_key(|e| e.id);
        Ok(OutboxClaim { token, entries })
    }

    /// Record the deliveries built for claimed outbox entries and remove those entries, in one
    /// transaction. Each entry is removed only while `claim` still holds it; the deliveries of
    /// an entry that was lost (the lease expired and a sibling re-claimed it) are not recorded,
    /// so every event becomes deliveries exactly once. Returns how many entries were committed.
    pub async fn commit_outbox(&self, claim: &str, built: &[(i64, Vec<NewDelivery>)]) -> Result<usize> {
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        let t = now();
        let mut committed = 0;
        for (entry, deliveries) in built {
            let r = sqlx::query("DELETE FROM webhook_outbox WHERE id = $1 AND claim = $2")
                .bind(entry)
                .bind(claim)
                .execute(&mut *tx)
                .await
                .map_err(map_err)?;
            if r.rows_affected() != 1 {
                continue;
            }
            committed += 1;
            for d in deliveries {
                insert_delivery(&mut *tx, reef_core::id::next(), t, d).await?;
            }
        }
        tx.commit().await.map_err(map_err)?;
        Ok(committed)
    }

    /// Give up a claim without committing (building failed): the entries are free again.
    pub async fn release_outbox(&self, claim: &str) -> Result<()> {
        sqlx::query("UPDATE webhook_outbox SET claim = NULL, lock_until = NULL WHERE claim = $1")
            .bind(claim)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Drop unclaimed outbox entries recorded before `before` (epoch ms): the poller calls this
    /// while no active hook exists, so nothing piles up and enabling a hook never replays
    /// history.
    pub async fn purge_outbox_before(&self, before: i64) -> Result<u64> {
        Ok(sqlx::query("DELETE FROM webhook_outbox WHERE created_at < $1 AND (lock_until IS NULL OR lock_until < $2)")
            .bind(before)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?
            .rows_affected())
    }

    /// Outbox entries waiting (claimed or not).
    pub async fn count_outbox(&self) -> Result<i64> {
        sqlx::query("SELECT COUNT(*) AS n FROM webhook_outbox").fetch_one(&self.pool).await.map_err(map_err)?.try_get("n").map_err(map_err)
    }

    pub async fn polled_repo_event(&self, id: i64) -> Result<Option<PolledRepoEvent>> {
        sqlx::query_as::<_, PolledRepoEvent>("SELECT id, repo_id, actor_id, kind, payload, created_at FROM repo_events WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn polled_issue_event(&self, id: i64) -> Result<Option<PolledIssueEvent>> {
        sqlx::query_as::<_, PolledIssueEvent>(
            "SELECT id, issue_id, repo_id, actor_id, event, payload, created_at FROM issue_events WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn polled_comment(&self, id: i64) -> Result<Option<PolledComment>> {
        sqlx::query_as::<_, PolledComment>("SELECT id, issue_id, repo_id, author_id, created_at FROM issue_comments WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Whether any active hook exists at all (lets the poller skip work on quiet instances).
    pub async fn any_active_hook(&self) -> Result<bool> {
        let n: i64 = sqlx::query("SELECT COUNT(*) AS n FROM webhooks WHERE active <> 0")
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("n")
            .map_err(map_err)?;
        Ok(n > 0)
    }
}
