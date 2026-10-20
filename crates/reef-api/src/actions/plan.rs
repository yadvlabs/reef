//! Turn an event into workflow runs: read `.github/workflows/*` at the commit, match triggers,
//! and create the run with one job record per (job, matrix leg).

use super::event;
use crate::AppState;
use reef_actions::expr::{self, Status};
use reef_actions::trigger::Event;
use reef_actions::{Workflow, matrix, workflow};
use reef_core::{Error, Id, Result};
use reef_db::accounts::Account;
use reef_db::actions::{NewJob, NewRun, Run};
use reef_db::repos::RepoWithOwner;
use reef_git::{RepoReader, ops};
use serde_json::{Value, json};

pub const WORKFLOWS_DIR: &str = ".github/workflows";

/// What happened, with everything the trigger filters and the `github` context need.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub event: String,
    pub action: Option<String>,
    /// The ref the run checks out (`refs/heads/x`, `refs/tags/x`, or `refs/pull/N/merge`).
    pub git_ref: String,
    /// The ref branch filters apply to (for pull_request: the base branch).
    pub filter_ref: String,
    pub head_sha: String,
    pub head_branch: Option<String>,
    pub payload: Value,
    pub pull_number: Option<i64>,
    pub inputs: Option<Value>,
    pub changed_paths: Option<Vec<String>>,
    pub display_title: String,
    /// Only run this workflow file (workflow_dispatch).
    pub only_path: Option<String>,
}

/// Workflow files present at a commit: (path, contents).
pub async fn workflow_files(s: &AppState, rw: &RepoWithOwner, sha: &str) -> Result<Vec<(String, String)>> {
    let path = s.store.path(rw.repo.id());
    let sha = sha.to_string();
    tokio::task::spawn_blocking(move || {
        let r = RepoReader::open(&path)?;
        let entries = match r.tree(&sha, WORKFLOWS_DIR) {
            Ok(e) => e,
            Err(Error::NotFound) => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for e in entries {
            let lower = e.name.to_lowercase();
            if matches!(e.kind, reef_git::read::EntryKind::Blob | reef_git::read::EntryKind::Executable)
                && (lower.ends_with(".yml") || lower.ends_with(".yaml"))
                && let Ok(b) = r.blob(&e.id)
            {
                out.push((e.path.clone(), String::from_utf8_lossy(&b.data).into_owned()));
            }
        }
        out.sort();
        Ok(out)
    })
    .await
    .map_err(Error::internal)?
}

/// Keep the `workflows` table in step with the default branch.
pub async fn sync_workflows(s: &AppState, rw: &RepoWithOwner, sha: &str) -> Result<()> {
    let files = workflow_files(s, rw, sha).await?;
    let mut paths = Vec::new();
    for (path, yaml) in &files {
        let name = workflow::parse(yaml).ok().and_then(|w| w.name).unwrap_or_else(|| path.clone());
        s.db.upsert_workflow(rw.repo.id(), path, &name).await?;
        paths.push(path.clone());
    }
    s.db.retain_workflows(rw.repo.id(), &paths).await
}

/// Files changed between two commits (None when either side is unknown).
pub async fn changed_paths(s: &AppState, rw: &RepoWithOwner, before: &str, after: &str) -> Option<Vec<String>> {
    if before == reef_git::smart::RefUpdate::ZERO || after == reef_git::smart::RefUpdate::ZERO {
        return None;
    }
    let path = s.store.path(rw.repo.id());
    ops::diff_stat(s.store.git_binary(), &path, before, after).await.ok().map(|v| v.into_iter().map(|f| f.path).collect())
}

/// Create runs for every workflow the trigger matches. Returns the runs created.
pub async fn plan(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, trig: &Trigger) -> Result<Vec<Run>> {
    let files = workflow_files(s, rw, &trig.head_sha).await?;
    let mut runs = Vec::new();
    for (path, yaml) in files {
        if let Some(only) = &trig.only_path
            && only != &path
        {
            continue;
        }
        let parsed = workflow::parse(&yaml);
        let wf = match &parsed {
            Ok(wf) => wf,
            Err(e) => {
                // Surface broken files on pushes that touch them, like GitHub's "Invalid
                // workflow file" runs; otherwise ignore them quietly.
                let touched = trig.changed_paths.as_ref().map(|p| p.contains(&path)).unwrap_or(trig.only_path.is_some());
                if touched {
                    let wid = s.db.upsert_workflow(rw.repo.id(), &path, &path).await?;
                    let run = s.db.create_run(new_run(rw, wid.id(), &path, &path, &yaml, actor, trig, Some(&e.to_string()))).await?;
                    runs.push(run);
                }
                continue;
            }
        };
        let ev = Event {
            name: trig.event.clone(),
            action: trig.action.clone(),
            git_ref: trig.filter_ref.clone(),
            changed_paths: trig.changed_paths.clone(),
        };
        if !reef_actions::trigger::matches(wf, &ev) {
            continue;
        }
        let name = wf.name.clone().unwrap_or_else(|| path.clone());
        let wrow = s.db.upsert_workflow(rw.repo.id(), &path, &name).await?;
        if wrow.state == "disabled_manually" {
            continue;
        }
        let run = create_run_with_jobs(s, rw, actor, trig, wrow.id(), &path, &yaml, wf, &name).await?;
        runs.push(run);
    }
    Ok(runs)
}

#[allow(clippy::too_many_arguments)]
fn new_run<'a>(
    rw: &'a RepoWithOwner,
    workflow: Id,
    path: &'a str,
    name: &'a str,
    yaml: &'a str,
    actor: Option<&'a Account>,
    trig: &'a Trigger,
    error: Option<&'a str>,
) -> NewRun<'a> {
    let _ = rw;
    NewRun {
        repo: rw.repo.id(),
        workflow,
        name,
        display_title: &trig.display_title,
        event: &trig.event,
        head_sha: &trig.head_sha,
        head_branch: trig.head_branch.as_deref(),
        git_ref: &trig.git_ref,
        path,
        actor: actor.map(|a| a.id()),
        event_payload: "{}",
        workflow_yaml: yaml,
        pull_number: trig.pull_number,
        inputs: None,
        error,
    }
}

/// Base expression context shared by every job of a run (no `needs`/`matrix` yet).
pub async fn base_context(s: &AppState, rw: &RepoWithOwner, run: &Run, actor: Option<&Account>) -> Result<Value> {
    let inputs: Value = run.inputs.as_deref().and_then(|i| serde_json::from_str(i).ok()).unwrap_or(json!({}));
    let mut vars = serde_json::Map::new();
    if rw.owner.is_org() {
        for v in s.db.variables("org", rw.owner.id()).await? {
            vars.insert(v.name, Value::String(v.value));
        }
    }
    for v in s.db.variables("repo", rw.repo.id()).await? {
        vars.insert(v.name, Value::String(v.value));
    }
    let placeholder = reef_db::actions::Job {
        id: 0,
        run_id: run.id,
        repo_id: run.repo_id,
        run_attempt: run.run_attempt,
        job_key: String::new(),
        name: String::new(),
        matrix: None,
        needs: "[]".into(),
        labels: "[]".into(),
        status: String::new(),
        conclusion: None,
        runner_id: None,
        runner_name: None,
        request_id: None,
        lock_token: None,
        locked_until: None,
        cancel_requested: 0,
        plan_id: String::new(),
        timeline_id: String::new(),
        job_guid: String::new(),
        message: None,
        timeline: None,
        outputs: None,
        order_index: 0,
        created_at: 0,
        updated_at: 0,
        started_at: None,
        completed_at: None,
    };
    Ok(json!({
        "github": event::github_context(s, rw, run, &placeholder, actor, None),
        "inputs": inputs,
        "vars": Value::Object(vars),
        "env": {},
    }))
}

#[allow(clippy::too_many_arguments)]
async fn create_run_with_jobs(
    s: &AppState,
    rw: &RepoWithOwner,
    actor: Option<&Account>,
    trig: &Trigger,
    workflow: Id,
    path: &str,
    yaml: &str,
    wf: &Workflow,
    name: &str,
) -> Result<Run> {
    let payload = serde_json::to_string(&trig.payload).unwrap_or_else(|_| "{}".into());
    let inputs = trig.inputs.as_ref().map(|i| serde_json::to_string(i).unwrap_or_default());
    let mut n = new_run(rw, workflow, path, name, yaml, actor, trig, None);
    n.event_payload = &payload;
    n.inputs = inputs.as_deref();
    let run = s.db.create_run(n).await?;
    let ctx = base_context(s, rw, &run, actor).await?;
    // run-name
    if let Some(rn) = &wf.run_name
        && let Ok(title) = expr::interpolate(rn, &ctx, Status::all_ok())
        && !title.trim().is_empty()
    {
        s.db.set_run_title(run.id(), title.trim()).await?;
    }
    match create_jobs(s, rw, &run, wf, &ctx, run.run_attempt, None).await {
        Ok(()) => {}
        Err(e) => {
            s.db.fail_run_startup(run.id(), &e.to_string()).await?;
            return s.db.run_by_id(run.id()).await;
        }
    }
    super::schedule::advance_run(s, run.id()).await?;
    s.db.run_by_id(run.id()).await
}

/// Create the job records of an attempt. `keep` carries completed jobs from a previous
/// attempt to copy instead of re-running (re-run failed jobs).
pub async fn create_jobs(
    s: &AppState,
    rw: &RepoWithOwner,
    run: &Run,
    wf: &Workflow,
    ctx: &Value,
    attempt: i64,
    keep: Option<&[reef_db::actions::Job]>,
) -> Result<()> {
    let order = workflow::topo_order(wf).map_err(|e| Error::validation(e.to_string()))?;
    let mut index = 0i64;
    for job in order {
        if let Some(u) = &job.uses {
            return Err(Error::validation(format!("job '{}' calls reusable workflow '{u}', which Reef does not support yet", job.id)));
        }
        let legs: Vec<Option<matrix::Combination>> = match &job.strategy {
            Some(st) if !st.matrix.is_null() => {
                let m = expr::evaluate_value(&st.matrix, ctx, Status::all_ok())
                    .map_err(|e| Error::validation(format!("job '{}': {e}", job.id)))?;
                let combos = matrix::expand(&m).map_err(|e| Error::validation(format!("job '{}': {e}", job.id)))?;
                if combos.is_empty() {
                    return Err(Error::validation(format!("job '{}': matrix expands to nothing", job.id)));
                }
                combos.into_iter().map(Some).collect()
            }
            _ => vec![None],
        };
        for leg in legs {
            let mut c = ctx.clone();
            if let Some(m) = &leg {
                c["matrix"] = json!(m);
            }
            let name = match &job.name {
                Some(n) => expr::interpolate(n, &c, Status::all_ok()).unwrap_or_else(|_| n.clone()),
                None => match &leg {
                    Some(m) => format!("{} ({})", job.id, matrix::leg_name(m)),
                    None => job.id.clone(),
                },
            };
            let matrix_json = leg.as_ref().map(|m| serde_json::to_string(m).unwrap_or_default());
            let prev = keep.and_then(|k| {
                k.iter().find(|j| {
                    j.job_key == job.id && j.matrix.as_deref() == matrix_json.as_deref() && j.conclusion.as_deref() == Some("success")
                })
            });
            let created =
                s.db.create_job(
                    NewJob {
                        run: run.id(),
                        repo: rw.repo.id(),
                        job_key: &job.id,
                        name: &name,
                        matrix: matrix_json.as_deref(),
                        needs: &job.needs,
                        labels: &[],
                        status: "waiting",
                        order_index: index,
                    },
                    attempt,
                )
                .await?;
            if let Some(p) = prev {
                // Carry the earlier success forward untouched.
                s.db.complete_job(created.id(), "success", p.outputs.as_deref()).await?;
                if let Some(t) = &p.timeline {
                    s.db.set_job_timeline(created.id(), t).await?;
                }
            }
            index += 1;
        }
    }
    Ok(())
}

/// Read the commits a push added (newest first, capped), for the payload.
pub async fn push_commits(s: &AppState, rw: &RepoWithOwner, before: &str, after: &str) -> Vec<reef_git::read::Commit> {
    let path = s.store.path(rw.repo.id());
    let shas: Vec<String> = if before == reef_git::smart::RefUpdate::ZERO {
        vec![after.to_string()]
    } else {
        ops::rev_list(s.store.git_binary(), &path, before, after).await.unwrap_or_else(|_| vec![after.to_string()])
    };
    let shas: Vec<String> = shas.into_iter().take(20).collect();
    tokio::task::spawn_blocking(move || {
        let r = RepoReader::open(&path).ok()?;
        Some(shas.iter().filter_map(|sha| r.commit(sha).ok()).collect::<Vec<_>>())
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}
