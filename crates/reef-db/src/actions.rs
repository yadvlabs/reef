//! Actions storage: workflows, runs, jobs, runners, sessions, logs, artifacts, secrets.

use crate::{Backend, Db, b, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::{AssertSqlSafe, Row};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Runner {
    pub id: i64,
    pub scope_kind: String,
    pub scope_id: i64,
    pub name: String,
    pub os: Option<String>,
    pub version: Option<String>,
    pub labels: String,
    pub ephemeral: i64,
    pub disable_update: i64,
    pub client_id: String,
    pub public_key: String,
    pub agent_json: Option<String>,
    pub busy: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_seen_at: Option<i64>,
}

impl Runner {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
    pub fn labels(&self) -> Vec<String> {
        serde_json::from_str(&self.labels).unwrap_or_default()
    }
    /// A runner is online while it holds a session that polled recently.
    pub fn online(&self, now_ms: i64) -> bool {
        self.last_seen_at.map(|t| now_ms - t < RUNNER_ONLINE_WINDOW_MS).unwrap_or(false)
    }
}

pub const RUNNER_ONLINE_WINDOW_MS: i64 = 2 * 60_000;
const RUNNER_COLS: &str = "id, scope_kind, scope_id, name, os, version, labels, ephemeral, disable_update, client_id, public_key, agent_json, busy, created_at, updated_at, last_seen_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RunnerToken {
    pub token_hash: String,
    pub scope_kind: String,
    pub scope_id: i64,
    pub purpose: String,
    pub created_by: Option<i64>,
    pub expires_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub session_id: String,
    pub runner_id: i64,
    pub encryption_key: String,
    pub owner_name: Option<String>,
    pub created_at: i64,
    pub last_poll_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Workflow {
    pub id: i64,
    pub repo_id: i64,
    pub path: String,
    pub name: String,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Workflow {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

const WORKFLOW_COLS: &str = "id, repo_id, path, name, state, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Run {
    pub id: i64,
    pub repo_id: i64,
    pub workflow_id: i64,
    pub run_number: i64,
    pub run_attempt: i64,
    pub name: String,
    pub display_title: String,
    pub event: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub head_sha: String,
    pub head_branch: Option<String>,
    pub git_ref: String,
    pub path: String,
    pub actor_id: Option<i64>,
    pub triggering_actor_id: Option<i64>,
    pub event_payload: String,
    pub workflow_yaml: String,
    pub pull_number: Option<i64>,
    pub inputs: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

impl Run {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

/// The statement behind `Db::runs_completed_before` (binds: cutoff, cursor time, cursor id);
/// public so the plan can be checked against the index.
pub fn runs_completed_before_sql(limit: i64) -> String {
    format!(
        "SELECT {RUN_COLS} FROM workflow_runs WHERE status = 'completed' AND {RUN_FINISHED} < $1 AND {RUN_FINISHED} >= $2 AND ({RUN_FINISHED} > $2 OR id > $3) ORDER BY {RUN_FINISHED}, id LIMIT {}",
        limit.clamp(1, 1000)
    )
}

/// When a run last finished, as indexed by `workflow_runs_finished` (migration 0004).
pub const RUN_FINISHED: &str = "COALESCE(completed_at, updated_at)";
const RUN_COLS: &str = "id, repo_id, workflow_id, run_number, run_attempt, name, display_title, event, status, conclusion, head_sha, head_branch, git_ref, path, actor_id, triggering_actor_id, event_payload, workflow_yaml, pull_number, inputs, error, created_at, updated_at, started_at, completed_at";

pub struct NewRun<'a> {
    pub repo: Id,
    pub workflow: Id,
    pub name: &'a str,
    pub display_title: &'a str,
    pub event: &'a str,
    pub head_sha: &'a str,
    pub head_branch: Option<&'a str>,
    pub git_ref: &'a str,
    pub path: &'a str,
    pub actor: Option<Id>,
    pub event_payload: &'a str,
    pub workflow_yaml: &'a str,
    pub pull_number: Option<i64>,
    pub inputs: Option<&'a str>,
    pub error: Option<&'a str>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Job {
    pub id: i64,
    pub run_id: i64,
    pub repo_id: i64,
    pub run_attempt: i64,
    pub job_key: String,
    pub name: String,
    pub matrix: Option<String>,
    pub needs: String,
    pub labels: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub runner_id: Option<i64>,
    pub runner_name: Option<String>,
    pub request_id: Option<i64>,
    pub lock_token: Option<String>,
    pub locked_until: Option<i64>,
    pub cancel_requested: i64,
    pub plan_id: String,
    pub timeline_id: String,
    pub job_guid: String,
    pub message: Option<String>,
    pub timeline: Option<String>,
    pub outputs: Option<String>,
    pub order_index: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

impl Job {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
    pub fn needs(&self) -> Vec<String> {
        serde_json::from_str(&self.needs).unwrap_or_default()
    }
    pub fn labels(&self) -> Vec<String> {
        serde_json::from_str(&self.labels).unwrap_or_default()
    }
    pub fn matrix_json(&self) -> serde_json::Value {
        self.matrix.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or(serde_json::Value::Null)
    }
    pub fn outputs_json(&self) -> serde_json::Value {
        self.outputs.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or_else(|| serde_json::json!({}))
    }
    pub fn timeline_json(&self) -> Vec<serde_json::Value> {
        self.timeline.as_deref().and_then(|m| serde_json::from_str(m).ok()).unwrap_or_default()
    }
    pub fn done(&self) -> bool {
        self.status == "completed"
    }
}

/// Job counts by state (see `count_jobs_by_state`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobCounts {
    pub queued: i64,
    pub running: i64,
    pub completed: i64,
    pub failed: i64,
}

const JOB_COLS: &str = "id, run_id, repo_id, run_attempt, job_key, name, matrix, needs, labels, status, conclusion, runner_id, runner_name, request_id, lock_token, locked_until, cancel_requested, plan_id, timeline_id, job_guid, message, timeline, outputs, order_index, created_at, updated_at, started_at, completed_at";

pub struct NewJob<'a> {
    pub run: Id,
    pub repo: Id,
    pub job_key: &'a str,
    pub name: &'a str,
    pub matrix: Option<&'a str>,
    pub needs: &'a [String],
    pub labels: &'a [String],
    /// `waiting` when it has unmet needs, else `queued`.
    pub status: &'a str,
    pub order_index: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct JobLog {
    pub id: i64,
    pub job_id: i64,
    pub path: String,
    pub line_count: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct JobToken {
    pub token_hash: String,
    pub job_id: i64,
    pub repo_id: i64,
    pub permissions: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Artifact {
    pub id: i64,
    pub run_id: i64,
    pub repo_id: i64,
    pub name: String,
    pub size: i64,
    pub path: Option<String>,
    pub expired: i64,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Secret {
    pub scope_kind: String,
    pub scope_id: i64,
    pub name: String,
    pub value_enc: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Variable {
    pub scope_kind: String,
    pub scope_id: i64,
    pub name: String,
    pub value: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Default, Clone)]
pub struct RunFilter {
    pub workflow: Option<Id>,
    pub actor: Option<Id>,
    pub branch: Option<String>,
    pub event: Option<String>,
    pub status: Option<String>,
    pub head_sha: Option<String>,
    pub created_after: Option<i64>,
}

fn json_vec(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

impl Db {
    // ----- runners -----

    #[allow(clippy::too_many_arguments)]
    pub async fn create_runner(
        &self,
        scope_kind: &str,
        scope_id: Id,
        name: &str,
        os: Option<&str>,
        version: Option<&str>,
        labels: &[String],
        ephemeral: bool,
        disable_update: bool,
        client_id: &str,
        public_key: &str,
        agent_json: &str,
    ) -> Result<Runner> {
        let name = name.trim();
        if name.is_empty() || name.len() > 64 {
            return Err(Error::validation("runner name must be 1-64 characters"));
        }
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO runners (id, scope_kind, scope_id, name, os, version, labels, ephemeral, disable_update, client_id, public_key, agent_json, busy, created_at, updated_at, last_seen_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 0, $13, $13, NULL)")
            .bind(id.0)
            .bind(scope_kind)
            .bind(scope_id.0)
            .bind(name)
            .bind(os)
            .bind(version)
            .bind(json_vec(labels))
            .bind(b(ephemeral))
            .bind(b(disable_update))
            .bind(client_id)
            .bind(public_key)
            .bind(agent_json)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.runner_by_id(id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_runner(
        &self,
        id: Id,
        os: Option<&str>,
        version: Option<&str>,
        labels: &[String],
        ephemeral: bool,
        disable_update: bool,
        public_key: &str,
        agent_json: &str,
    ) -> Result<Runner> {
        sqlx::query("UPDATE runners SET os = $2, version = $3, labels = $4, ephemeral = $5, disable_update = $6, public_key = $7, agent_json = $8, updated_at = $9 WHERE id = $1")
            .bind(id.0)
            .bind(os)
            .bind(version)
            .bind(json_vec(labels))
            .bind(b(ephemeral))
            .bind(b(disable_update))
            .bind(public_key)
            .bind(agent_json)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.runner_by_id(id).await
    }

    pub async fn runner_by_id(&self, id: Id) -> Result<Runner> {
        sqlx::query_as::<_, Runner>(AssertSqlSafe(format!("SELECT {RUNNER_COLS} FROM runners WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn runner_by_client_id(&self, client_id: &str) -> Result<Runner> {
        sqlx::query_as::<_, Runner>(AssertSqlSafe(format!("SELECT {RUNNER_COLS} FROM runners WHERE client_id = $1")))
            .bind(client_id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn runner_by_name(&self, scope_kind: &str, scope_id: Id, name: &str) -> Result<Option<Runner>> {
        sqlx::query_as::<_, Runner>(AssertSqlSafe(format!(
            "SELECT {RUNNER_COLS} FROM runners WHERE scope_kind = $1 AND scope_id = $2 AND name = $3"
        )))
        .bind(scope_kind)
        .bind(scope_id.0)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn runners(&self, scope_kind: &str, scope_id: Id) -> Result<Vec<Runner>> {
        sqlx::query_as::<_, Runner>(AssertSqlSafe(format!(
            "SELECT {RUNNER_COLS} FROM runners WHERE scope_kind = $1 AND scope_id = $2 ORDER BY name"
        )))
        .bind(scope_kind)
        .bind(scope_id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn delete_runner(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM runner_sessions WHERE runner_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        let r = sqlx::query("DELETE FROM runners WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    pub async fn touch_runner(&self, id: Id, busy: Option<bool>) -> Result<()> {
        match busy {
            Some(bz) => sqlx::query("UPDATE runners SET last_seen_at = $2, busy = $3 WHERE id = $1")
                .bind(id.0)
                .bind(now())
                .bind(b(bz))
                .execute(&self.pool)
                .await
                .map_err(map_err)?,
            None => sqlx::query("UPDATE runners SET last_seen_at = $2 WHERE id = $1")
                .bind(id.0)
                .bind(now())
                .execute(&self.pool)
                .await
                .map_err(map_err)?,
        };
        Ok(())
    }

    // ----- registration tokens -----

    pub async fn create_runner_token(
        &self,
        scope_kind: &str,
        scope_id: Id,
        purpose: &str,
        created_by: Option<Id>,
        ttl_ms: i64,
    ) -> Result<(String, i64)> {
        let plain = reef_core::token::mint("");
        let plain = format!("A{}", plain.to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).take(28).collect::<String>());
        let expires = now() + ttl_ms;
        sqlx::query(
            "INSERT INTO runner_tokens (token_hash, scope_kind, scope_id, purpose, created_by, expires_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(reef_core::token::hash(&plain))
        .bind(scope_kind)
        .bind(scope_id.0)
        .bind(purpose)
        .bind(created_by.map(|i| i.0))
        .bind(expires)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok((plain, expires))
    }

    /// Look up a registration token; expired ones read as missing.
    pub async fn runner_token(&self, plain: &str) -> Result<RunnerToken> {
        let t = sqlx::query_as::<_, RunnerToken>(
            "SELECT token_hash, scope_kind, scope_id, purpose, created_by, expires_at FROM runner_tokens WHERE token_hash = $1",
        )
        .bind(reef_core::token::hash(plain))
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        if t.expires_at < now() {
            return Err(Error::NotFound);
        }
        Ok(t)
    }

    pub async fn purge_runner_tokens(&self) -> Result<()> {
        sqlx::query("DELETE FROM runner_tokens WHERE expires_at < $1").bind(now()).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- sessions -----

    pub async fn create_session_for_runner(
        &self,
        session_id: &str,
        runner: Id,
        encryption_key: &str,
        owner_name: Option<&str>,
    ) -> Result<Session> {
        // One session per runner: a new one replaces a stale one (the runner restarted).
        sqlx::query("DELETE FROM runner_sessions WHERE runner_id = $1").bind(runner.0).execute(&self.pool).await.map_err(map_err)?;
        let ts = now();
        sqlx::query("INSERT INTO runner_sessions (session_id, runner_id, encryption_key, owner_name, created_at, last_poll_at) VALUES ($1, $2, $3, $4, $5, $5)")
            .bind(session_id)
            .bind(runner.0)
            .bind(encryption_key)
            .bind(owner_name)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.runner_session(session_id).await
    }

    pub async fn runner_session(&self, session_id: &str) -> Result<Session> {
        sqlx::query_as::<_, Session>(
            "SELECT session_id, runner_id, encryption_key, owner_name, created_at, last_poll_at FROM runner_sessions WHERE session_id = $1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn touch_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE runner_sessions SET last_poll_at = $2 WHERE session_id = $1")
            .bind(session_id)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn delete_runner_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM runner_sessions WHERE session_id = $1").bind(session_id).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- workflows -----

    pub async fn upsert_workflow(&self, repo: Id, path: &str, name: &str) -> Result<Workflow> {
        let ts = now();
        if let Some(w) = self.workflow_by_path(repo, path).await? {
            if w.name != name || w.state == "deleted" {
                let state = if w.state == "deleted" { "active" } else { w.state.as_str() };
                sqlx::query("UPDATE workflows SET name = $2, state = $3, updated_at = $4 WHERE id = $1")
                    .bind(w.id)
                    .bind(name)
                    .bind(state)
                    .bind(ts)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
            }
            return self.workflow_by_id(Id(w.id)).await;
        }
        let id = reef_core::id::next();
        sqlx::query(
            "INSERT INTO workflows (id, repo_id, path, name, state, created_at, updated_at) VALUES ($1, $2, $3, $4, 'active', $5, $5)",
        )
        .bind(id.0)
        .bind(repo.0)
        .bind(path)
        .bind(name)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        self.workflow_by_id(id).await
    }

    pub async fn workflow_by_id(&self, id: Id) -> Result<Workflow> {
        sqlx::query_as::<_, Workflow>(AssertSqlSafe(format!("SELECT {WORKFLOW_COLS} FROM workflows WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn workflow_by_path(&self, repo: Id, path: &str) -> Result<Option<Workflow>> {
        sqlx::query_as::<_, Workflow>(AssertSqlSafe(format!("SELECT {WORKFLOW_COLS} FROM workflows WHERE repo_id = $1 AND path = $2")))
            .bind(repo.0)
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Resolve `ci.yml`, `.github/workflows/ci.yml`, or a numeric id.
    pub async fn workflow_by_ref(&self, repo: Id, r: &str) -> Result<Workflow> {
        if let Ok(n) = r.parse::<i64>() {
            let w = self.workflow_by_id(Id(n)).await?;
            if w.repo_id != repo.0 {
                return Err(Error::NotFound);
            }
            return Ok(w);
        }
        let path = if r.contains('/') { r.to_string() } else { format!(".github/workflows/{r}") };
        self.workflow_by_path(repo, &path).await?.ok_or(Error::NotFound)
    }

    pub async fn workflows(&self, repo: Id) -> Result<Vec<Workflow>> {
        sqlx::query_as::<_, Workflow>(AssertSqlSafe(format!(
            "SELECT {WORKFLOW_COLS} FROM workflows WHERE repo_id = $1 AND state <> 'deleted' ORDER BY name"
        )))
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn set_workflow_state(&self, id: Id, state: &str) -> Result<()> {
        sqlx::query("UPDATE workflows SET state = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(state)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Mark workflows no longer present in the default branch as deleted.
    pub async fn retain_workflows(&self, repo: Id, paths: &[String]) -> Result<()> {
        for w in self.workflows(repo).await? {
            if !paths.contains(&w.path) {
                self.set_workflow_state(Id(w.id), "deleted").await?;
            }
        }
        Ok(())
    }

    // ----- runs -----

    pub async fn create_run(&self, n: NewRun<'_>) -> Result<Run> {
        let row = sqlx::query("UPDATE repositories SET next_run_number = next_run_number + 1 WHERE id = $1 RETURNING next_run_number AS n")
            .bind(n.repo.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        let number: i64 = row.try_get::<i64, _>("n").map_err(map_err)? - 1;
        let id = reef_core::id::next();
        let ts = now();
        let (status, conclusion, completed) =
            if n.error.is_some() { ("completed", Some("startup_failure"), Some(ts)) } else { ("queued", None, None) };
        sqlx::query("INSERT INTO workflow_runs (id, repo_id, workflow_id, run_number, run_attempt, name, display_title, event, status, conclusion, head_sha, head_branch, git_ref, path, actor_id, triggering_actor_id, event_payload, workflow_yaml, pull_number, inputs, error, created_at, updated_at, started_at, completed_at) VALUES ($1, $2, $3, $4, 1, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14, $15, $16, $17, $18, $19, $20, $20, NULL, $21)")
            .bind(id.0)
            .bind(n.repo.0)
            .bind(n.workflow.0)
            .bind(number)
            .bind(n.name)
            .bind(n.display_title)
            .bind(n.event)
            .bind(status)
            .bind(conclusion)
            .bind(n.head_sha)
            .bind(n.head_branch)
            .bind(n.git_ref)
            .bind(n.path)
            .bind(n.actor.map(|a| a.0))
            .bind(n.event_payload)
            .bind(n.workflow_yaml)
            .bind(n.pull_number)
            .bind(n.inputs)
            .bind(n.error)
            .bind(ts)
            .bind(completed)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.run_by_id(id).await
    }

    pub async fn run_by_id(&self, id: Id) -> Result<Run> {
        sqlx::query_as::<_, Run>(AssertSqlSafe(format!("SELECT {RUN_COLS} FROM workflow_runs WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn runs(&self, repo: Id, f: &RunFilter, limit: i64, offset: i64) -> Result<(Vec<Run>, i64)> {
        let mut where_ = vec!["repo_id = $1".to_string()];
        let mut binds: Vec<Bind> = vec![Bind::I(repo.0)];
        let push = |cond: &str, v: Bind, binds: &mut Vec<Bind>, where_: &mut Vec<String>| {
            binds.push(v);
            where_.push(cond.replace("?", &format!("${}", binds.len())));
        };
        if let Some(w) = f.workflow {
            push("workflow_id = ?", Bind::I(w.0), &mut binds, &mut where_);
        }
        if let Some(a) = f.actor {
            push("actor_id = ?", Bind::I(a.0), &mut binds, &mut where_);
        }
        if let Some(bname) = &f.branch {
            push("head_branch = ?", Bind::S(bname.clone()), &mut binds, &mut where_);
        }
        if let Some(e) = &f.event {
            push("event = ?", Bind::S(e.clone()), &mut binds, &mut where_);
        }
        if let Some(s) = &f.status {
            // GitHub accepts either a status or a conclusion here.
            push("(status = ? OR conclusion = ?)", Bind::S(s.clone()), &mut binds, &mut where_);
            let n = binds.len();
            let last = where_.last_mut().unwrap();
            *last = last.replacen('?', &format!("${n}"), 1);
        }
        if let Some(sha) = &f.head_sha {
            push("head_sha = ?", Bind::S(sha.clone()), &mut binds, &mut where_);
        }
        if let Some(t) = f.created_after {
            push("created_at >= ?", Bind::I(t), &mut binds, &mut where_);
        }
        let w = where_.join(" AND ");
        let mut q = sqlx::query_as::<_, Run>(AssertSqlSafe(format!(
            "SELECT {RUN_COLS} FROM workflow_runs WHERE {w} ORDER BY created_at DESC, id DESC LIMIT {limit} OFFSET {offset}"
        )));
        let mut c = sqlx::query(AssertSqlSafe(format!("SELECT COUNT(*) AS n FROM workflow_runs WHERE {w}")));
        for bnd in &binds {
            match bnd {
                Bind::I(i) => {
                    q = q.bind(*i);
                    c = c.bind(*i);
                }
                Bind::S(s) => {
                    q = q.bind(s.clone());
                    c = c.bind(s.clone());
                }
            }
        }
        let rows = q.fetch_all(&self.pool).await.map_err(map_err)?;
        let total: i64 = c.fetch_one(&self.pool).await.map_err(map_err)?.try_get("n").map_err(map_err)?;
        Ok((rows, total))
    }

    pub async fn runs_for_sha(&self, repo: Id, sha: &str) -> Result<Vec<Run>> {
        sqlx::query_as::<_, Run>(AssertSqlSafe(format!(
            "SELECT {RUN_COLS} FROM workflow_runs WHERE repo_id = $1 AND head_sha = $2 ORDER BY created_at DESC"
        )))
        .bind(repo.0)
        .bind(sha)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn set_run_status(&self, id: Id, status: &str, conclusion: Option<&str>) -> Result<()> {
        let ts = now();
        match status {
            "in_progress" => sqlx::query("UPDATE workflow_runs SET status = $2, conclusion = NULL, updated_at = $3, started_at = COALESCE(started_at, $3) WHERE id = $1").bind(id.0).bind(status).bind(ts).execute(&self.pool).await.map_err(map_err)?,
            "completed" => sqlx::query("UPDATE workflow_runs SET status = $2, conclusion = $3, updated_at = $4, started_at = COALESCE(started_at, $4), completed_at = $4 WHERE id = $1").bind(id.0).bind(status).bind(conclusion).bind(ts).execute(&self.pool).await.map_err(map_err)?,
            _ => sqlx::query("UPDATE workflow_runs SET status = $2, conclusion = $3, updated_at = $4 WHERE id = $1").bind(id.0).bind(status).bind(conclusion).bind(ts).execute(&self.pool).await.map_err(map_err)?,
        };
        Ok(())
    }

    /// Prepare a re-run: bump the attempt, reset status, and drop the previous attempt's jobs
    /// (or only failed ones when `failed_only`). Returns the job keys that must be recreated.
    pub async fn start_rerun(&self, id: Id, actor: Option<Id>) -> Result<Run> {
        let ts = now();
        sqlx::query("UPDATE workflow_runs SET run_attempt = run_attempt + 1, status = 'queued', conclusion = NULL, error = NULL, triggering_actor_id = COALESCE($2, triggering_actor_id), updated_at = $3, started_at = NULL, completed_at = NULL WHERE id = $1")
            .bind(id.0)
            .bind(actor.map(|a| a.0))
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.run_by_id(id).await
    }

    pub async fn set_run_title(&self, id: Id, title: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_runs SET display_title = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(title)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// The run could not start (bad file, unsupported feature).
    pub async fn fail_run_startup(&self, id: Id, error: &str) -> Result<()> {
        let ts = now();
        sqlx::query("UPDATE workflow_runs SET status = 'completed', conclusion = 'startup_failure', error = $2, updated_at = $3, completed_at = $3 WHERE id = $1").bind(id.0).bind(error).bind(ts).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("UPDATE workflow_jobs SET status = 'completed', conclusion = 'cancelled', updated_at = $2, completed_at = $2 WHERE run_id = $1 AND status <> 'completed'").bind(id.0).bind(ts).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    pub async fn delete_run(&self, id: Id) -> Result<()> {
        for j in self.jobs_of_run(id, None).await? {
            sqlx::query("DELETE FROM job_logs WHERE job_id = $1").bind(j.id).execute(&self.pool).await.map_err(map_err)?;
            sqlx::query("DELETE FROM job_tokens WHERE job_id = $1").bind(j.id).execute(&self.pool).await.map_err(map_err)?;
        }
        sqlx::query("DELETE FROM workflow_jobs WHERE run_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("DELETE FROM artifacts WHERE run_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        let r = sqlx::query("DELETE FROM workflow_runs WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    // ----- jobs -----

    pub async fn create_job(&self, n: NewJob<'_>, run_attempt: i64) -> Result<Job> {
        let id = reef_core::id::next();
        let ts = now();
        let guid = || uuid_v4();
        sqlx::query("INSERT INTO workflow_jobs (id, run_id, repo_id, run_attempt, job_key, name, matrix, needs, labels, status, conclusion, plan_id, timeline_id, job_guid, order_index, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NULL, $11, $12, $13, $14, $15, $15)")
            .bind(id.0)
            .bind(n.run.0)
            .bind(n.repo.0)
            .bind(run_attempt)
            .bind(n.job_key)
            .bind(n.name)
            .bind(n.matrix)
            .bind(json_vec(n.needs))
            .bind(json_vec(n.labels))
            .bind(n.status)
            .bind(guid())
            .bind(guid())
            .bind(guid())
            .bind(n.order_index)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.job_by_id(id).await
    }

    pub async fn job_by_id(&self, id: Id) -> Result<Job> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!("SELECT {JOB_COLS} FROM workflow_jobs WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn job_by_request(&self, request_id: i64) -> Result<Job> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!("SELECT {JOB_COLS} FROM workflow_jobs WHERE request_id = $1")))
            .bind(request_id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn job_by_plan(&self, plan_id: &str) -> Result<Job> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!("SELECT {JOB_COLS} FROM workflow_jobs WHERE plan_id = $1")))
            .bind(plan_id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Jobs of a run, latest attempt by default.
    pub async fn jobs_of_run(&self, run: Id, attempt: Option<i64>) -> Result<Vec<Job>> {
        match attempt {
            Some(a) => sqlx::query_as::<_, Job>(AssertSqlSafe(format!(
                "SELECT {JOB_COLS} FROM workflow_jobs WHERE run_id = $1 AND run_attempt = $2 ORDER BY order_index, id"
            )))
            .bind(run.0)
            .bind(a)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err),
            None => sqlx::query_as::<_, Job>(AssertSqlSafe(format!(
                "SELECT {JOB_COLS} FROM workflow_jobs WHERE run_id = $1 ORDER BY order_index, id"
            )))
            .bind(run.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err),
        }
    }

    pub async fn jobs_for_sha(&self, repo: Id, sha: &str) -> Result<Vec<Job>> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!("SELECT j.* FROM (SELECT {JOB_COLS} FROM workflow_jobs) j JOIN workflow_runs r ON r.id = j.run_id WHERE r.repo_id = $1 AND r.head_sha = $2 AND j.run_attempt = r.run_attempt ORDER BY j.created_at")))
            .bind(repo.0)
            .bind(sha)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Queued jobs across the instance whose labels a runner might satisfy, oldest first.
    pub async fn queued_jobs(&self, limit: i64) -> Result<Vec<Job>> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!(
            "SELECT {JOB_COLS} FROM workflow_jobs WHERE status = 'queued' ORDER BY created_at LIMIT {limit}"
        )))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn waiting_jobs_of_run(&self, run: Id, attempt: i64) -> Result<Vec<Job>> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!(
            "SELECT {JOB_COLS} FROM workflow_jobs WHERE run_id = $1 AND run_attempt = $2 AND status = 'waiting' ORDER BY order_index"
        )))
        .bind(run.0)
        .bind(attempt)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Jobs that a runner holds but that have gone quiet (lock expired).
    pub async fn stale_jobs(&self) -> Result<Vec<Job>> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!(
            "SELECT {JOB_COLS} FROM workflow_jobs WHERE status = 'in_progress' AND locked_until IS NOT NULL AND locked_until < $1"
        )))
        .bind(now())
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Move a waiting job to the queue with its resolved labels.
    pub async fn queue_job(&self, id: Id, labels: &[String]) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET status = 'queued', labels = $2, updated_at = $3 WHERE id = $1 AND status = 'waiting'")
            .bind(id.0)
            .bind(json_vec(labels))
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Atomically hand a queued job to a runner. Returns false if someone else got it first.
    /// One guarded `UPDATE ... WHERE status = 'queued'` on both engines; on PostgreSQL the
    /// row is selected `FOR UPDATE SKIP LOCKED` first, so a claim in flight on another
    /// instance is skipped rather than waited for.
    pub async fn assign_job(
        &self,
        id: Id,
        runner: &Runner,
        request_id: i64,
        lock_token: &str,
        lock_ms: i64,
        message: &str,
    ) -> Result<bool> {
        let ts = now();
        let target = match self.backend {
            Backend::Postgres => "id = (SELECT id FROM workflow_jobs WHERE id = $1 AND status = 'queued' FOR UPDATE SKIP LOCKED)",
            Backend::Sqlite => "id = $1 AND status = 'queued'",
        };
        let r = sqlx::query(AssertSqlSafe(format!("UPDATE workflow_jobs SET status = 'in_progress', runner_id = $2, runner_name = $3, request_id = $4, lock_token = $5, locked_until = $6, message = $7, updated_at = $8, started_at = $8 WHERE {target}")))
            .bind(id.0)
            .bind(runner.id)
            .bind(&runner.name)
            .bind(request_id)
            .bind(lock_token)
            .bind(ts + lock_ms)
            .bind(message)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn renew_job_lock(&self, id: Id, lock_ms: i64) -> Result<()> {
        let ts = now();
        sqlx::query("UPDATE workflow_jobs SET locked_until = $2, updated_at = $3 WHERE id = $1 AND status = 'in_progress'")
            .bind(id.0)
            .bind(ts + lock_ms)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Put an in-progress job back in the queue (runner vanished).
    pub async fn requeue_job(&self, id: Id) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET status = 'queued', runner_id = NULL, runner_name = NULL, request_id = NULL, lock_token = NULL, locked_until = NULL, message = NULL, updated_at = $2, started_at = NULL WHERE id = $1 AND status = 'in_progress'").bind(id.0).bind(now()).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    pub async fn complete_job(&self, id: Id, conclusion: &str, outputs: Option<&str>) -> Result<()> {
        let ts = now();
        sqlx::query("UPDATE workflow_jobs SET status = 'completed', conclusion = $2, outputs = COALESCE($3, outputs), locked_until = NULL, updated_at = $4, started_at = COALESCE(started_at, $4), completed_at = $4 WHERE id = $1 AND status <> 'completed'")
            .bind(id.0)
            .bind(conclusion)
            .bind(outputs)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn request_job_cancel(&self, id: Id) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET cancel_requested = 1, updated_at = $2 WHERE id = $1 AND status = 'in_progress'")
            .bind(id.0)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// The cancellation was delivered to the runner; do not send it again.
    pub async fn mark_cancel_sent(&self, id: Id) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET cancel_requested = 2 WHERE id = $1 AND cancel_requested = 1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// In-progress jobs held by a runner.
    pub async fn jobs_of_runner(&self, runner: Id) -> Result<Vec<Job>> {
        sqlx::query_as::<_, Job>(AssertSqlSafe(format!(
            "SELECT {JOB_COLS} FROM workflow_jobs WHERE runner_id = $1 AND status = 'in_progress' ORDER BY started_at"
        )))
        .bind(runner.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn set_job_timeline(&self, id: Id, timeline: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET timeline = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(timeline)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn set_job_outputs(&self, id: Id, outputs: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET outputs = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(outputs)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn set_job_name(&self, id: Id, name: &str) -> Result<()> {
        sqlx::query("UPDATE workflow_jobs SET name = $2, updated_at = $3 WHERE id = $1")
            .bind(id.0)
            .bind(name)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Jobs per state, for `/metrics`: (queued, running, completed, failed). `waiting`
    /// jobs (dependencies pending) count as queued; failed is the subset of completed jobs
    /// whose conclusion is `failure` or `timed_out`.
    pub async fn count_jobs_by_state(&self) -> Result<JobCounts> {
        let rows = sqlx::query("SELECT status, conclusion, COUNT(*) AS n FROM workflow_jobs GROUP BY status, conclusion")
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        let mut c = JobCounts::default();
        for r in rows {
            let status: String = r.try_get("status").map_err(map_err)?;
            let conclusion: Option<String> = r.try_get("conclusion").map_err(map_err)?;
            let n: i64 = r.try_get("n").map_err(map_err)?;
            match status.as_str() {
                "waiting" | "queued" => c.queued += n,
                "in_progress" => c.running += n,
                "completed" => {
                    c.completed += n;
                    if matches!(conclusion.as_deref(), Some("failure") | Some("timed_out")) {
                        c.failed += n;
                    }
                }
                _ => {}
            }
        }
        Ok(c)
    }

    /// Runners that polled within the online window.
    pub async fn count_online_runners(&self) -> Result<i64> {
        sqlx::query("SELECT COUNT(*) AS n FROM runners WHERE last_seen_at IS NOT NULL AND last_seen_at > $1")
            .bind(now() - RUNNER_ONLINE_WINDOW_MS)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("n")
            .map_err(map_err)
    }

    /// Completed runs that finished before `before` (epoch ms), in finishing order, after the
    /// cursor `after` (`(finished, id)` of the last row a pass already looked at; start with
    /// `(i64::MIN, i64::MIN)`): what the retention job removes. A run is aged by when it last
    /// completed (`completed_at`, else `updated_at`), not when it was created, so a run
    /// re-run inside the window is kept.
    ///
    /// The query is shaped for the partial expression index `workflow_runs_finished`
    /// (`(COALESCE(completed_at, updated_at), id) WHERE status = 'completed'`, migration 0004):
    /// the same expression bounded on both sides (the cutoff above, the cursor's time below) and
    /// the matching `ORDER BY` let SQLite and PostgreSQL range-scan it instead of every run.
    pub async fn runs_completed_before(&self, before: i64, after: (i64, i64), limit: i64) -> Result<Vec<Run>> {
        sqlx::query_as::<_, Run>(AssertSqlSafe(runs_completed_before_sql(limit)))
            .bind(before)
            .bind(after.0)
            .bind(after.1)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Artifacts past their `expires_at` that still have a file and are not waiting out a
    /// `retry_after` (see `defer_artifact_expiry`), in expiry order after the cursor `after`
    /// (`(expires_at, id)` of the last row a pass looked at; start with `(i64::MIN, i64::MIN)`).
    pub async fn artifacts_expired_at(&self, at: i64, after: (i64, i64), limit: i64) -> Result<Vec<Artifact>> {
        sqlx::query_as::<_, Artifact>(AssertSqlSafe(format!(
            "SELECT id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at FROM artifacts WHERE expired = 0 AND expires_at IS NOT NULL AND expires_at < $1 AND (retry_after IS NULL OR retry_after <= $1) AND (expires_at > $2 OR (expires_at = $2 AND id > $3)) ORDER BY expires_at, id LIMIT {}",
            limit.clamp(1, 1000)
        )))
        .bind(at)
        .bind(after.0)
        .bind(after.1)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Expiring an artifact failed (its file could not be removed): leave it until `until`
    /// (epoch ms) so the retention job's head moves past it.
    pub async fn defer_artifact_expiry(&self, id: Id, until: i64) -> Result<()> {
        sqlx::query("UPDATE artifacts SET retry_after = $2 WHERE id = $1")
            .bind(id.0)
            .bind(until)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// The artifact's file was removed: keep the row (GitHub lists expired artifacts) but
    /// mark it expired and drop its path and size.
    pub async fn expire_artifact(&self, id: Id) -> Result<()> {
        sqlx::query("UPDATE artifacts SET expired = 1, path = NULL, size = 0, updated_at = $2 WHERE id = $1")
            .bind(id.0)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- logs -----

    pub async fn create_job_log(&self, job: Id, path: &str) -> Result<JobLog> {
        // Log ids are small integers in the runner protocol; a per-instance sequence is fine.
        let id = reef_core::id::next().0 & 0x7fff_ffff;
        sqlx::query("INSERT INTO job_logs (id, job_id, path, line_count, created_at) VALUES ($1, $2, $3, 0, $4)")
            .bind(id)
            .bind(job.0)
            .bind(path)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.job_log(id).await
    }

    pub async fn set_log_path(&self, id: i64, path: &str) -> Result<()> {
        sqlx::query("UPDATE job_logs SET path = $2 WHERE id = $1").bind(id).bind(path).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    pub async fn job_log(&self, id: i64) -> Result<JobLog> {
        sqlx::query_as::<_, JobLog>("SELECT id, job_id, path, line_count, created_at FROM job_logs WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn job_logs(&self, job: Id) -> Result<Vec<JobLog>> {
        sqlx::query_as::<_, JobLog>("SELECT id, job_id, path, line_count, created_at FROM job_logs WHERE job_id = $1 ORDER BY id")
            .bind(job.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn add_log_lines(&self, id: i64, lines: i64) -> Result<()> {
        sqlx::query("UPDATE job_logs SET line_count = line_count + $2 WHERE id = $1")
            .bind(id)
            .bind(lines)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- job tokens -----

    pub async fn create_job_token(&self, job: Id, repo: Id, permissions: &str, ttl_ms: i64) -> Result<String> {
        let plain = reef_core::token::mint(JOB_TOKEN_PREFIX);
        sqlx::query("INSERT INTO job_tokens (token_hash, job_id, repo_id, permissions, expires_at) VALUES ($1, $2, $3, $4, $5)")
            .bind(reef_core::token::hash(&plain))
            .bind(job.0)
            .bind(repo.0)
            .bind(permissions)
            .bind(now() + ttl_ms)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(plain)
    }

    pub async fn job_token(&self, plain: &str) -> Result<JobToken> {
        let t = sqlx::query_as::<_, JobToken>(
            "SELECT token_hash, job_id, repo_id, permissions, expires_at FROM job_tokens WHERE token_hash = $1",
        )
        .bind(reef_core::token::hash(plain))
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        if t.expires_at < now() {
            return Err(Error::Unauthorized("token expired".into()));
        }
        Ok(t)
    }

    pub async fn revoke_job_tokens(&self, job: Id) -> Result<()> {
        sqlx::query("DELETE FROM job_tokens WHERE job_id = $1").bind(job.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- artifacts -----

    pub async fn create_artifact(&self, run: Id, repo: Id, name: &str, retention_days: i64) -> Result<Artifact> {
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO artifacts (id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at) VALUES ($1, $2, $3, $4, 0, NULL, 0, $5, $6, $6)")
            .bind(id.0)
            .bind(run.0)
            .bind(repo.0)
            .bind(name)
            .bind(ts + retention_days.max(1) * 86_400_000)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.artifact_by_id(id).await
    }

    pub async fn finalize_artifact(&self, id: Id, path: &str, size: i64) -> Result<()> {
        sqlx::query("UPDATE artifacts SET path = $2, size = $3, updated_at = $4 WHERE id = $1")
            .bind(id.0)
            .bind(path)
            .bind(size)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn artifact_by_id(&self, id: Id) -> Result<Artifact> {
        sqlx::query_as::<_, Artifact>(
            "SELECT id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at FROM artifacts WHERE id = $1",
        )
        .bind(id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn artifact_by_name(&self, run: Id, name: &str) -> Result<Option<Artifact>> {
        sqlx::query_as::<_, Artifact>("SELECT id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at FROM artifacts WHERE run_id = $1 AND name = $2").bind(run.0).bind(name).fetch_optional(&self.pool).await.map_err(map_err)
    }

    pub async fn artifacts_of_run(&self, run: Id) -> Result<Vec<Artifact>> {
        sqlx::query_as::<_, Artifact>("SELECT id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at FROM artifacts WHERE run_id = $1 AND path IS NOT NULL ORDER BY name").bind(run.0).fetch_all(&self.pool).await.map_err(map_err)
    }

    pub async fn artifacts_of_repo(&self, repo: Id, name: Option<&str>, limit: i64, offset: i64) -> Result<(Vec<Artifact>, i64)> {
        let (rows, total) = match name {
            Some(n) => (
                sqlx::query_as::<_, Artifact>(AssertSqlSafe(format!("SELECT id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at FROM artifacts WHERE repo_id = $1 AND name = $2 AND path IS NOT NULL ORDER BY created_at DESC LIMIT {limit} OFFSET {offset}"))).bind(repo.0).bind(n).fetch_all(&self.pool).await.map_err(map_err)?,
                sqlx::query("SELECT COUNT(*) AS n FROM artifacts WHERE repo_id = $1 AND name = $2 AND path IS NOT NULL").bind(repo.0).bind(n).fetch_one(&self.pool).await.map_err(map_err)?.try_get::<i64, _>("n").map_err(map_err)?,
            ),
            None => (
                sqlx::query_as::<_, Artifact>(AssertSqlSafe(format!("SELECT id, run_id, repo_id, name, size, path, expired, expires_at, created_at, updated_at FROM artifacts WHERE repo_id = $1 AND path IS NOT NULL ORDER BY created_at DESC LIMIT {limit} OFFSET {offset}"))).bind(repo.0).fetch_all(&self.pool).await.map_err(map_err)?,
                sqlx::query("SELECT COUNT(*) AS n FROM artifacts WHERE repo_id = $1 AND path IS NOT NULL").bind(repo.0).fetch_one(&self.pool).await.map_err(map_err)?.try_get::<i64, _>("n").map_err(map_err)?,
            ),
        };
        Ok((rows, total))
    }

    pub async fn delete_artifact(&self, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM artifacts WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    // ----- secrets & variables -----

    pub async fn set_secret(&self, scope_kind: &str, scope_id: Id, name: &str, value_enc: &str) -> Result<()> {
        let name = name.trim().to_uppercase();
        validate_var_name(&name)?;
        let ts = now();
        let r =
            sqlx::query("UPDATE actions_secrets SET value_enc = $4, updated_at = $5 WHERE scope_kind = $1 AND scope_id = $2 AND name = $3")
                .bind(scope_kind)
                .bind(scope_id.0)
                .bind(&name)
                .bind(value_enc)
                .bind(ts)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        if r.rows_affected() == 0 {
            sqlx::query("INSERT INTO actions_secrets (scope_kind, scope_id, name, value_enc, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5)").bind(scope_kind).bind(scope_id.0).bind(&name).bind(value_enc).bind(ts).execute(&self.pool).await.map_err(map_err)?;
        }
        Ok(())
    }

    pub async fn secrets(&self, scope_kind: &str, scope_id: Id) -> Result<Vec<Secret>> {
        sqlx::query_as::<_, Secret>("SELECT scope_kind, scope_id, name, value_enc, created_at, updated_at FROM actions_secrets WHERE scope_kind = $1 AND scope_id = $2 ORDER BY name").bind(scope_kind).bind(scope_id.0).fetch_all(&self.pool).await.map_err(map_err)
    }

    pub async fn secret(&self, scope_kind: &str, scope_id: Id, name: &str) -> Result<Secret> {
        sqlx::query_as::<_, Secret>("SELECT scope_kind, scope_id, name, value_enc, created_at, updated_at FROM actions_secrets WHERE scope_kind = $1 AND scope_id = $2 AND name = $3").bind(scope_kind).bind(scope_id.0).bind(name.to_uppercase()).fetch_one(&self.pool).await.map_err(map_err)
    }

    pub async fn delete_secret(&self, scope_kind: &str, scope_id: Id, name: &str) -> Result<()> {
        let r = sqlx::query("DELETE FROM actions_secrets WHERE scope_kind = $1 AND scope_id = $2 AND name = $3")
            .bind(scope_kind)
            .bind(scope_id.0)
            .bind(name.to_uppercase())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    pub async fn set_variable(&self, scope_kind: &str, scope_id: Id, name: &str, value: &str) -> Result<()> {
        let name = name.trim().to_uppercase();
        validate_var_name(&name)?;
        let ts = now();
        let r =
            sqlx::query("UPDATE actions_variables SET value = $4, updated_at = $5 WHERE scope_kind = $1 AND scope_id = $2 AND name = $3")
                .bind(scope_kind)
                .bind(scope_id.0)
                .bind(&name)
                .bind(value)
                .bind(ts)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        if r.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO actions_variables (scope_kind, scope_id, name, value, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $5)",
            )
            .bind(scope_kind)
            .bind(scope_id.0)
            .bind(&name)
            .bind(value)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        }
        Ok(())
    }

    pub async fn variables(&self, scope_kind: &str, scope_id: Id) -> Result<Vec<Variable>> {
        sqlx::query_as::<_, Variable>("SELECT scope_kind, scope_id, name, value, created_at, updated_at FROM actions_variables WHERE scope_kind = $1 AND scope_id = $2 ORDER BY name").bind(scope_kind).bind(scope_id.0).fetch_all(&self.pool).await.map_err(map_err)
    }

    pub async fn variable(&self, scope_kind: &str, scope_id: Id, name: &str) -> Result<Variable> {
        sqlx::query_as::<_, Variable>("SELECT scope_kind, scope_id, name, value, created_at, updated_at FROM actions_variables WHERE scope_kind = $1 AND scope_id = $2 AND name = $3").bind(scope_kind).bind(scope_id.0).bind(name.to_uppercase()).fetch_one(&self.pool).await.map_err(map_err)
    }

    pub async fn delete_variable(&self, scope_kind: &str, scope_id: Id, name: &str) -> Result<()> {
        let r = sqlx::query("DELETE FROM actions_variables WHERE scope_kind = $1 AND scope_id = $2 AND name = $3")
            .bind(scope_kind)
            .bind(scope_id.0)
            .bind(name.to_uppercase())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }
}

pub const JOB_TOKEN_PREFIX: &str = "fgj_";

enum Bind {
    I(i64),
    S(String),
}

fn validate_var_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && !name.starts_with("GITHUB_");
    if ok {
        Ok(())
    } else {
        Err(Error::validation("names may contain letters, digits and underscores, must not start with a digit or GITHUB_"))
    }
}

/// A random version-4 UUID in the canonical lowercase form.
pub fn uuid_v4() -> String {
    let h = reef_core::token::random_hex(16);
    let b: Vec<char> = h.chars().collect();
    let mut s: String = b.iter().collect();
    // Set version (4) and variant (8..b) nibbles.
    s.replace_range(12..13, "4");
    let v = match b[16] {
        '0'..='3' => '8',
        '4'..='7' => '9',
        '8'..='b' => 'a',
        _ => 'b',
    };
    s.replace_range(16..17, &v.to_string());
    format!("{}-{}-{}-{}-{}", &s[0..8], &s[8..12], &s[12..16], &s[16..20], &s[20..32])
}
