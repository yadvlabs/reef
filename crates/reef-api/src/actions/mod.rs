//! GitHub Actions: workflow runs planned from repository events, executed by stock
//! `actions/runner` processes speaking their native protocol, exposed through GitHub's REST API
//! and Reef's web UI.

pub mod crypto;
pub mod event;
pub mod jwt;
pub mod logs;
pub mod message;
pub mod plan;
pub mod rest;
pub mod retention;
pub mod runner;
pub mod schedule;
pub mod secrets;

/// Shared Actions services: the file store and the instance keys.
pub struct ActionsState {
    pub files: logs::Files,
    pub keys: secrets::Keys,
}

impl ActionsState {
    pub fn load(data_dir: &std::path::Path) -> Result<ActionsState> {
        Ok(ActionsState { files: logs::Files::new(data_dir), keys: secrets::Keys::load_or_create(data_dir)? })
    }
}

use crate::AppState;
use reef_core::Result;
use reef_db::accounts::Account;
use reef_db::repos::RepoWithOwner;
use reef_git::smart::RefUpdate;

/// A ref was pushed: sync workflow definitions, start `push` runs, and `pull_request`
/// `synchronize` runs for open PRs from that branch.
pub async fn on_push(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, u: &RefUpdate) -> Result<()> {
    if u.is_delete() {
        return Ok(());
    }
    if u.branch() == Some(rw.repo.default_branch.as_str())
        && let Err(e) = plan::sync_workflows(s, rw, &u.new).await
    {
        tracing::warn!(error = %e, "workflow sync failed");
    }
    let commits = plan::push_commits(s, rw, &u.old, &u.new).await;
    let title = commits.first().map(|c| c.summary().to_string()).unwrap_or_else(|| u.name.clone());
    let payload = event::push_payload(s, rw, actor, &u.name, &u.old, &u.new, &commits);
    let trig = plan::Trigger {
        event: "push".into(),
        action: None,
        git_ref: u.name.clone(),
        filter_ref: u.name.clone(),
        head_sha: u.new.clone(),
        head_branch: u.branch().map(String::from),
        payload,
        pull_number: None,
        inputs: None,
        changed_paths: plan::changed_paths(s, rw, &u.old, &u.new).await,
        display_title: title,
        only_path: None,
    };
    if let Err(e) = plan::plan(s, rw, actor, &trig).await {
        tracing::warn!(error = %e, "planning push runs failed");
    }
    if let Some(branch) = u.branch() {
        for m in s.db.open_pulls_for_head(rw.repo.id(), branch).await.unwrap_or_default() {
            if let Ok(i) = s.db.issue_full_by_id(reef_core::Id(m.issue_id)).await
                && let Err(e) = on_pull_request(s, rw, actor, &i, &m, "synchronize").await
            {
                tracing::warn!(error = %e, "planning pull_request runs failed");
            }
        }
    }
    Ok(())
}

/// A pull request event (`opened`, `synchronize`, `reopened`, `closed`, ...).
pub async fn on_pull_request(
    s: &AppState,
    rw: &RepoWithOwner,
    actor: Option<&Account>,
    i: &reef_db::issues::IssueFull,
    m: &reef_db::pulls::PullMeta,
    action: &str,
) -> Result<()> {
    let pull = crate::rest::pulls::pull_json(s, rw, i, m, actor, false).await.map_err(|e| e.0)?;
    let payload = event::pull_request_payload(s, rw, actor, action, i.issue.number, pull);
    let trig = plan::Trigger {
        event: "pull_request".into(),
        action: Some(action.to_string()),
        // GitHub checks out the merge ref; we run on the head commit and expose it as such.
        git_ref: format!("refs/pull/{}/merge", i.issue.number),
        filter_ref: format!("refs/heads/{}", m.base_ref),
        head_sha: m.head_sha.clone(),
        head_branch: Some(m.head_ref.clone()),
        payload,
        pull_number: Some(i.issue.number),
        inputs: None,
        changed_paths: plan::changed_paths(s, rw, &m.base_sha, &m.head_sha).await,
        display_title: i.issue.title.clone(),
        only_path: None,
    };
    plan::plan(s, rw, actor, &trig).await.map(|_| ())
}

/// `workflow_dispatch`: run one workflow file at a ref with inputs.
pub async fn dispatch(
    s: &AppState,
    rw: &RepoWithOwner,
    actor: &Account,
    path: &str,
    git_ref: &str,
    inputs: serde_json::Value,
) -> Result<Vec<reef_db::actions::Run>> {
    let repo_path = s.store.path(rw.repo.id());
    let full_ref = if git_ref.starts_with("refs/") { git_ref.to_string() } else { format!("refs/heads/{git_ref}") };
    let sha = reef_git::ops::rev_parse(s.store.git_binary(), &repo_path, &full_ref)
        .await
        .map_err(|_| reef_core::Error::field("WorkflowDispatch", "ref", "invalid", "No ref found"))?;
    let payload = event::dispatch_payload(s, rw, Some(actor), &full_ref, path, &inputs);
    let trig = plan::Trigger {
        event: "workflow_dispatch".into(),
        action: None,
        git_ref: full_ref.clone(),
        filter_ref: full_ref.clone(),
        head_sha: sha,
        head_branch: full_ref.strip_prefix("refs/heads/").map(String::from),
        payload,
        pull_number: None,
        inputs: Some(inputs),
        changed_paths: None,
        display_title: String::new(),
        only_path: Some(path.to_string()),
    };
    plan::plan(s, rw, Some(actor), &trig).await
}

/// Periodic maintenance run by one task per host: expired locks, stale tokens.
pub async fn tick(s: &AppState) {
    if let Err(e) = schedule::reap_stale_jobs(s).await {
        tracing::warn!(error = %e, "reaping stale jobs failed");
    }
    let _ = s.db.purge_runner_tokens().await;
}
