//! Repository events (pushes, ref creation, deletion). Consumed by the activity feed, and by
//! webhooks through the outbox entry recorded with each event.

use crate::{Db, map_err, now};
use reef_core::{Id, Result};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepoEvent {
    pub id: i64,
    pub repo_id: i64,
    pub actor_id: Option<i64>,
    pub kind: String,
    pub payload: String,
    pub created_at: i64,
}

impl Db {
    /// Record a repository event and queue it for the webhook poller in one transaction.
    pub async fn record_event(&self, repo: Id, actor: Option<Id>, kind: &str, payload: &serde_json::Value) -> Result<Id> {
        let id = reef_core::id::next();
        let ts = now();
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        sqlx::query("INSERT INTO repo_events (id, repo_id, actor_id, kind, payload, created_at) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(id.0)
            .bind(repo.0)
            .bind(actor.map(|a| a.0))
            .bind(kind)
            .bind(payload.to_string())
            .bind(ts)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        crate::webhooks::enqueue_outbox(&mut *tx, crate::webhooks::OUTBOX_REPO_EVENTS, id.0, ts).await?;
        tx.commit().await.map_err(map_err)?;
        Ok(id)
    }

    pub async fn repo_events(&self, repo: Id, limit: i64) -> Result<Vec<RepoEvent>> {
        sqlx::query_as::<_, RepoEvent>(
            "SELECT id, repo_id, actor_id, kind, payload, created_at FROM repo_events WHERE repo_id = $1 ORDER BY id DESC LIMIT $2",
        )
        .bind(repo.0)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }
}
