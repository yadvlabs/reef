//! Moves runs forward: releases jobs whose `needs` are met, evaluates job `if`s, settles run
//! status, and handles completion, cancellation and re-runs. Every step is idempotent so any
//! host may call it at any time.

use super::plan;
use crate::AppState;
use reef_actions::expr::{self, Status};
use reef_actions::workflow;
use reef_core::{Id, Result};
use reef_db::accounts::Account;
use reef_db::actions::{Job, Run};
use serde_json::{Value, json};
use std::collections::HashMap;

fn aggregate(legs: &[&Job]) -> &'static str {
    let c: Vec<&str> = legs.iter().map(|j| j.conclusion.as_deref().unwrap_or("")).collect();
    if c.iter().any(|x| *x == "failure" || *x == "timed_out") {
        "failure"
    } else if c.contains(&"cancelled") {
        "cancelled"
    } else if !c.is_empty() && c.iter().all(|x| *x == "skipped") {
        "skipped"
    } else {
        "success"
    }
}

/// Release waiting jobs whose dependencies finished, then settle the run's own status.
pub async fn advance_run(s: &AppState, run_id: Id) -> Result<()> {
    let run = s.db.run_by_id(run_id).await?;
    if run.status == "completed" {
        return Ok(());
    }
    let rw = s.db.repo_with_owner_by_id(Id(run.repo_id)).await?;
    let jobs = s.db.jobs_of_run(run_id, Some(run.run_attempt)).await?;
    let mut by_key: HashMap<&str, Vec<&Job>> = HashMap::new();
    for j in &jobs {
        by_key.entry(j.job_key.as_str()).or_default().push(j);
    }
    let waiting: Vec<&Job> = jobs.iter().filter(|j| j.status == "waiting").collect();
    if !waiting.is_empty() {
        let wf = workflow::parse(&run.workflow_yaml).ok();
        let actor = match run.actor_id {
            Some(a) => s.db.account_by_id(Id(a)).await.ok(),
            None => None,
        };
        let base = plan::base_context(s, &rw, &run, actor.as_ref()).await?;
        for job in waiting {
            let deps = job.needs();
            let done = deps.iter().all(|k| by_key.get(k.as_str()).map(|legs| legs.iter().all(|l| l.done())).unwrap_or(true));
            if !done {
                continue;
            }
            // Build `needs` and the status the condition sees.
            let mut needs = serde_json::Map::new();
            let (mut any_fail, mut any_cancel, mut any_skip) = (false, false, false);
            for k in &deps {
                let legs = by_key.get(k.as_str()).cloned().unwrap_or_default();
                let result = aggregate(&legs);
                match result {
                    "failure" => any_fail = true,
                    "cancelled" => any_cancel = true,
                    "skipped" => any_skip = true,
                    _ => {}
                }
                let mut outputs = serde_json::Map::new();
                for l in legs {
                    if let Value::Object(o) = l.outputs_json() {
                        outputs.extend(o);
                    }
                }
                needs.insert(k.clone(), json!({"result": result, "outputs": outputs}));
            }
            let status = Status { success: !any_fail && !any_cancel && !any_skip, failure: any_fail, cancelled: any_cancel };
            let mut ctx = base.clone();
            ctx["needs"] = Value::Object(needs);
            ctx["matrix"] = job.matrix_json();
            ctx["strategy"] = json!({"fail-fast": true, "max-parallel": 0, "job-index": 0, "job-total": by_key.get(job.job_key.as_str()).map(|l| l.len()).unwrap_or(1)});
            let def = wf.as_ref().and_then(|w| w.jobs.get(&job.job_key));
            let cond = def.and_then(|d| d.if_.clone()).unwrap_or_default();
            let cancelled_run = jobs.iter().any(|j| j.conclusion.as_deref() == Some("cancelled") && j.cancel_requested != 0);
            let should_run = if cancelled_run { Ok(false) } else { expr::condition(&cond, &ctx, status) };
            match should_run {
                Ok(true) => {
                    let runs_on = def.map(|d| d.runs_on.clone()).unwrap_or(Value::Null);
                    match workflow::runs_on_labels(&runs_on, &ctx) {
                        Ok(labels) => s.db.queue_job(job.id(), &labels).await?,
                        Err(e) => {
                            tracing::warn!(job = job.id, error = %e, "runs-on could not be resolved");
                            s.db.complete_job(job.id(), "failure", None).await?;
                        }
                    }
                }
                Ok(false) => s.db.complete_job(job.id(), "skipped", None).await?,
                Err(e) => {
                    tracing::warn!(job = job.id, error = %e, "job condition failed to evaluate");
                    s.db.complete_job(job.id(), "failure", None).await?;
                }
            }
        }
    }
    settle_run(s, run_id).await
}

/// Derive the run's status/conclusion from its jobs.
pub async fn settle_run(s: &AppState, run_id: Id) -> Result<()> {
    let run = s.db.run_by_id(run_id).await?;
    let jobs = s.db.jobs_of_run(run_id, Some(run.run_attempt)).await?;
    if jobs.is_empty() {
        return Ok(());
    }
    let any = |st: &str| jobs.iter().any(|j| j.status == st);
    if any("in_progress") {
        if run.status != "in_progress" {
            s.db.set_run_status(run_id, "in_progress", None).await?;
        }
        return Ok(());
    }
    if any("queued") || any("waiting") {
        // Nothing running, some pending: queued unless we had already started.
        if run.status == "completed" {
            s.db.set_run_status(run_id, "queued", None).await?;
        }
        return Ok(());
    }
    let all: Vec<&Job> = jobs.iter().collect();
    let conclusion = aggregate(&all);
    if run.status != "completed" || run.conclusion.as_deref() != Some(conclusion) {
        s.db.set_run_status(run_id, "completed", Some(conclusion)).await?;
        on_run_completed(s, &run, conclusion).await;
    }
    Ok(())
}

async fn on_run_completed(s: &AppState, run: &Run, conclusion: &str) {
    let _ =
        s.db.record_event(
            Id(run.repo_id),
            None,
            "workflow_run.completed",
            &json!({"run_id": run.id, "conclusion": conclusion, "name": run.name}),
        )
        .await;
    if (conclusion == "failure" || conclusion == "startup_failure")
        && let (Some(uid), Ok(rw)) = (run.triggering_actor_id.or(run.actor_id), s.db.repo_with_owner_by_id(Id(run.repo_id)).await)
    {
        let _ =
            s.db.notify(reef_db::notifications::Notify {
                user: Id(uid),
                actor: None,
                repo: Some(rw.repo.id()),
                subject_type: "run",
                subject_id: Some(run.id),
                reason: "ci_activity",
                title: &format!("{} workflow run failed for {} · {}", run.name, run.display_title, rw.full_name()),
                url: &format!("/{}/actions/runs/{}", rw.full_name(), run.id),
            })
            .await;
    }
}

/// A job finished (as reported by the runner, or decided by us).
pub async fn complete_job(s: &AppState, job: &Job, conclusion: &str, outputs: Option<&Value>) -> Result<()> {
    let out = outputs.map(|o| serde_json::to_string(o).unwrap_or_default());
    s.db.complete_job(job.id(), conclusion, out.as_deref()).await?;
    s.db.revoke_job_tokens(job.id()).await?;
    if let Some(rid) = job.runner_id
        && let Ok(r) = s.db.runner_by_id(Id(rid)).await
    {
        if r.ephemeral != 0 {
            let _ = s.db.delete_runner(r.id()).await;
        } else {
            let _ = s.db.touch_runner(r.id(), Some(false)).await;
        }
    }
    // fail-fast: a failed matrix leg cancels its queued siblings.
    if conclusion == "failure"
        && let Ok(run) = s.db.run_by_id(Id(job.run_id)).await
    {
        let fail_fast = workflow::parse(&run.workflow_yaml)
            .ok()
            .and_then(|w| w.jobs.get(&job.job_key).and_then(|j| j.strategy.as_ref().map(|st| !matches!(st.fail_fast, Value::Bool(false)))))
            .unwrap_or(true);
        if fail_fast {
            for sib in s.db.jobs_of_run(run.id(), Some(run.run_attempt)).await.unwrap_or_default() {
                if sib.job_key == job.job_key && sib.id != job.id {
                    match sib.status.as_str() {
                        "queued" | "waiting" => {
                            let _ = s.db.complete_job(sib.id(), "cancelled", None).await;
                        }
                        "in_progress" => {
                            let _ = s.db.request_job_cancel(sib.id()).await;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    advance_run(s, Id(job.run_id)).await
}

pub async fn cancel_run(s: &AppState, run: &Run) -> Result<()> {
    for j in s.db.jobs_of_run(run.id(), Some(run.run_attempt)).await? {
        match j.status.as_str() {
            "queued" | "waiting" => s.db.complete_job(j.id(), "cancelled", None).await?,
            "in_progress" => s.db.request_job_cancel(j.id()).await?,
            _ => {}
        }
    }
    advance_run(s, run.id()).await
}

/// Re-run a whole run, or only its failed jobs (successful ones are carried forward).
pub async fn rerun(s: &AppState, run: &Run, actor: Option<&Account>, failed_only: bool) -> Result<Run> {
    if run.status != "completed" {
        return Err(reef_core::Error::Conflict("run is still in progress".into()));
    }
    let rw = s.db.repo_with_owner_by_id(Id(run.repo_id)).await?;
    let previous = s.db.jobs_of_run(run.id(), Some(run.run_attempt)).await?;
    let run = s.db.start_rerun(run.id(), actor.map(|a| a.id())).await?;
    let wf = match workflow::parse(&run.workflow_yaml) {
        Ok(w) => w,
        Err(e) => {
            s.db.fail_run_startup(run.id(), &e.to_string()).await?;
            return s.db.run_by_id(run.id()).await;
        }
    };
    let ctx = plan::base_context(s, &rw, &run, actor).await?;
    let keep = if failed_only { Some(previous.as_slice()) } else { None };
    if let Err(e) = plan::create_jobs(s, &rw, &run, &wf, &ctx, run.run_attempt, keep).await {
        s.db.fail_run_startup(run.id(), &e.to_string()).await?;
        return s.db.run_by_id(run.id()).await;
    }
    advance_run(s, run.id()).await?;
    s.db.run_by_id(run.id()).await
}

/// Jobs whose runner stopped renewing: put them back in the queue, or fail them if they
/// have been requeued before (the runner may be broken).
pub async fn reap_stale_jobs(s: &AppState) -> Result<()> {
    for j in s.db.stale_jobs().await? {
        tracing::warn!(job = j.id, runner = ?j.runner_name, "job lock expired; requeueing");
        if let Some(rid) = j.runner_id {
            let _ = s.db.touch_runner(Id(rid), Some(false)).await;
        }
        s.db.requeue_job(j.id()).await?;
        settle_run(s, Id(j.run_id)).await?;
    }
    Ok(())
}
