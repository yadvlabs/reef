//! Webhook-shaped event payloads (`github.event`) and the `github` context for a job.

use crate::AppState;
use crate::shapes;
use reef_core::time::iso;
use reef_db::accounts::Account;
use reef_db::actions::{Job, Run};
use reef_db::repos::RepoWithOwner;
use reef_git::read::Commit;
use serde_json::{Value, json};

fn commit_json(cfg: &crate::config::Config, rw: &RepoWithOwner, c: &Commit) -> Value {
    json!({
        "id": c.id,
        "tree_id": c.tree,
        "distinct": true,
        "message": c.message,
        "timestamp": iso(c.author.time_ms),
        "url": format!("{}/{}/{}/commit/{}", cfg.site_url(), rw.owner.login, rw.repo.name, c.id),
        "author": {"name": c.author.name, "email": c.author.email},
        "committer": {"name": c.committer.name, "email": c.committer.email},
        "added": [], "removed": [], "modified": []
    })
}

pub fn push_payload(
    s: &AppState,
    rw: &RepoWithOwner,
    actor: Option<&Account>,
    git_ref: &str,
    before: &str,
    after: &str,
    commits: &[Commit],
) -> Value {
    let cfg = &s.config;
    let created = before == reef_git::smart::RefUpdate::ZERO;
    let deleted = after == reef_git::smart::RefUpdate::ZERO;
    let head = commits.first().map(|c| commit_json(cfg, rw, c));
    let pusher = actor.map(|a| json!({"name": a.login, "email": a.email})).unwrap_or(json!({"name": "unknown"}));
    json!({
        "ref": git_ref,
        "before": before,
        "after": after,
        "created": created,
        "deleted": deleted,
        "forced": false,
        "base_ref": Value::Null,
        "compare": format!("{}/{}/{}/compare/{}...{}", cfg.site_url(), rw.owner.login, rw.repo.name, &before[..before.len().min(12)], &after[..after.len().min(12)]),
        "commits": commits.iter().rev().map(|c| commit_json(cfg, rw, c)).collect::<Vec<_>>(),
        "head_commit": head,
        "repository": shapes::repo(cfg, rw, None),
        "pusher": pusher,
        "sender": actor.map(|a| shapes::simple_user(cfg, a)).unwrap_or(Value::Null),
    })
}

pub fn pull_request_payload(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, action: &str, number: i64, pull: Value) -> Value {
    json!({
        "action": action,
        "number": number,
        "pull_request": pull,
        "repository": shapes::repo(&s.config, rw, None),
        "sender": actor.map(|a| shapes::simple_user(&s.config, a)).unwrap_or(Value::Null),
    })
}

pub fn dispatch_payload(
    s: &AppState,
    rw: &RepoWithOwner,
    actor: Option<&Account>,
    git_ref: &str,
    workflow_path: &str,
    inputs: &Value,
) -> Value {
    json!({
        "inputs": inputs,
        "ref": git_ref,
        "repository": shapes::repo(&s.config, rw, None),
        "sender": actor.map(|a| shapes::simple_user(&s.config, a)).unwrap_or(Value::Null),
        "workflow": workflow_path,
    })
}

/// The `github` context as the runner expects it (minus `token`, which the worker fills from
/// the `system.github.token` variable).
pub fn github_context(
    s: &AppState,
    rw: &RepoWithOwner,
    run: &Run,
    job: &Job,
    actor: Option<&Account>,
    triggering: Option<&Account>,
) -> Value {
    let cfg = &s.config;
    let site = cfg.site_url();
    let (ref_name, ref_type) = if let Some(b) = run.git_ref.strip_prefix("refs/heads/") {
        (b.to_string(), "branch")
    } else if let Some(t) = run.git_ref.strip_prefix("refs/tags/") {
        (t.to_string(), "tag")
    } else {
        (run.git_ref.clone(), "branch")
    };
    let payload: Value = serde_json::from_str(&run.event_payload).unwrap_or(json!({}));
    let (head_ref, base_ref) = match payload.get("pull_request") {
        Some(pr) => (pr.pointer("/head/ref").cloned().unwrap_or(Value::Null), pr.pointer("/base/ref").cloned().unwrap_or(Value::Null)),
        None => (json!(""), json!("")),
    };
    let actor_login = actor.map(|a| a.login.clone()).unwrap_or_else(|| "reef".into());
    let actor_id = actor.map(|a| a.id.to_string()).unwrap_or_default();
    let trig = triggering.or(actor);
    json!({
        "server_url": site,
        "api_url": cfg.api_url(),
        "graphql_url": format!("{site}/api/graphql"),
        "repository": format!("{}/{}", rw.owner.login, rw.repo.name),
        "repository_owner": rw.owner.login,
        "repository_owner_id": rw.owner.id.to_string(),
        "repository_id": rw.repo.id.to_string(),
        "repositoryUrl": format!("{site}/{}/{}.git", rw.owner.login, rw.repo.name),
        "run_id": run.id.to_string(),
        "run_number": run.run_number.to_string(),
        "run_attempt": run.run_attempt.to_string(),
        "retention_days": "90",
        "workflow": run.name,
        "workflow_ref": format!("{}/{}/{}@{}", rw.owner.login, rw.repo.name, run.path, run.git_ref),
        "workflow_sha": run.head_sha,
        "job": job.job_key,
        "sha": run.head_sha,
        "ref": run.git_ref,
        "ref_name": ref_name,
        "ref_type": ref_type,
        "ref_protected": false,
        "head_ref": head_ref,
        "base_ref": base_ref,
        "event_name": run.event,
        "event": payload,
        "event_path": "",
        "actor": actor_login,
        "actor_id": actor_id,
        "triggering_actor": trig.map(|a| a.login.clone()).unwrap_or_else(|| actor_login.clone()),
        "action": "",
        "action_path": "",
        "action_ref": "",
        "action_repository": "",
        "action_status": "",
        "workspace": "",
        "path": "",
        "env": "",
        "step_summary": "",
        "state": "",
        "output": "",
    })
}
