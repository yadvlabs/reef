//! GitHub-shaped webhook payloads built from Reef's recorded events.

use crate::AppState;
use crate::shapes;
use reef_core::Id;
use reef_core::time::iso;
use reef_db::accounts::Account;
use reef_db::repos::RepoWithOwner;
use serde_json::{Value, json};
use std::collections::HashMap;

/// `(event name, action, payload)` triples ready to enqueue.
pub type Emitted = Vec<(String, Option<String>, Value)>;

const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

/// The fields every payload carries: `repository`, `organization` (for org-owned repos) and
/// `sender` (the actor, or the repository owner when the event had none).
pub fn base(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>) -> Value {
    let mut v = json!({
        "repository": shapes::repo(&s.config, rw, None),
        "sender": shapes::simple_user(&s.config, actor.unwrap_or(&rw.owner)),
    });
    if rw.owner.is_org() {
        v["organization"] = shapes::org_simple(&s.config, &rw.owner);
    }
    v
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn emit(out: &mut Emitted, event: &str, action: Option<&str>, mut body: Value) {
    if let Some(a) = action {
        body["action"] = json!(a);
    }
    out.push((event.to_string(), action.map(str::to_string), body));
}

/// Events derived from a `repo_events` row.
pub async fn from_repo_event(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, kind: &str, p: &Value) -> Emitted {
    let mut out = Emitted::new();
    let b = base(s, rw, actor);
    match kind {
        "push" | "create" | "delete" => {
            let r = p["ref"].as_str().unwrap_or("");
            let before = p["before"].as_str().unwrap_or(ZERO_SHA);
            let after = p["after"].as_str().unwrap_or(ZERO_SHA);
            if let Some(push) = push_payload(s, rw, actor, r, before, after).await {
                emit(&mut out, "push", None, with(b.clone(), push));
            }
            let ref_type = if r.starts_with("refs/tags/") { "tag" } else { "branch" };
            let short = r.trim_start_matches("refs/heads/").trim_start_matches("refs/tags/");
            if kind == "create" {
                emit(
                    &mut out,
                    "create",
                    None,
                    with(
                        b.clone(),
                        json!({"ref": short, "ref_type": ref_type, "master_branch": rw.repo.default_branch, "description": rw.repo.description, "pusher_type": "user"}),
                    ),
                );
            }
            if kind == "delete" {
                emit(&mut out, "delete", None, with(b.clone(), json!({"ref": short, "ref_type": ref_type, "pusher_type": "user"})));
            }
            // A push to a branch with open pull requests is a `synchronize` for each of them.
            if kind == "push"
                && ref_type == "branch"
                && let Ok(pulls) = s.db.open_pulls_for_head(rw.repo.id(), short).await
            {
                for m in pulls {
                    if let Some(pr) = pull_json(s, rw, Id(m.issue_id)).await {
                        let n = pr["number"].clone();
                        emit(
                            &mut out,
                            "pull_request",
                            Some("synchronize"),
                            with(b.clone(), json!({"number": n, "before": before, "after": after, "pull_request": pr})),
                        );
                    }
                }
            }
        }
        "issues.opened" => {
            if let Some(n) = p["number"].as_i64()
                && let Ok(i) = s.db.issue_by_number(rw.repo.id(), n).await
                && let Some(issue) = issue_json(s, rw, i.id()).await
            {
                emit(&mut out, "issues", Some("opened"), with(b, json!({"issue": issue})));
            }
        }
        "pull_request.opened" => {
            if let Some(n) = p["number"].as_i64()
                && let Ok(i) = s.db.issue_by_number(rw.repo.id(), n).await
                && let Some(pr) = pull_json(s, rw, i.id()).await
            {
                emit(&mut out, "pull_request", Some("opened"), with(b, json!({"number": n, "pull_request": pr})));
            }
        }
        "repository.created" => emit(&mut out, "repository", Some("created"), b),
        "watch" => {
            emit(&mut out, "watch", Some("started"), b.clone());
            emit(&mut out, "star", Some("created"), with(b, json!({"starred_at": iso(reef_core::time::now_ms())})));
        }
        "fork" => {
            let forkee = p["forkee"].as_str().unwrap_or("");
            let forkee_json = match forkee.split_once('/') {
                Some((o, n)) => s.db.repo_by_path(o, n).await.ok().map(|f| shapes::repo(&s.config, &f, None)),
                None => None,
            };
            emit(&mut out, "fork", None, with(b, json!({"forkee": forkee_json})));
        }
        "release.published" => {
            if let Some(id) = p["id"].as_i64()
                && let Ok(r) = s.db.release_by_id(Id(id)).await
            {
                let author = match r.author_id {
                    Some(a) => s.db.account_by_id(Id(a)).await.ok(),
                    None => None,
                };
                let rj = crate::rest::releases::release_json(s, rw, &r, author.as_ref(), None);
                emit(&mut out, "release", Some("published"), with(b.clone(), json!({"release": rj.clone()})));
                emit(&mut out, "release", Some("released"), with(b, json!({"release": rj})));
            }
        }
        "workflow_run.completed" => {
            if let Some(id) = p["run_id"].as_i64()
                && let Ok(run) = s.db.run_by_id(Id(id)).await
            {
                let mut cache: HashMap<i64, Option<Account>> = HashMap::new();
                let rj = crate::actions::rest::run_json(s, rw, &run, &mut cache).await;
                let wf = s.db.workflow_by_id(Id(run.workflow_id)).await.ok().map(|w| json!({"id": w.id, "node_id": shapes::node_id("W_", w.id), "name": w.name, "path": w.path, "state": w.state, "url": format!("{}/repos/{}/actions/workflows/{}", s.config.api_url(), rw.full_name(), w.id), "html_url": format!("{}/{}/blob/{}/{}", s.config.site_url(), rw.full_name(), rw.repo.default_branch, w.path), "badge_url": format!("{}/{}/actions/workflows/{}/badge.svg", s.config.site_url(), rw.full_name(), w.path.rsplit('/').next().unwrap_or(""))}));
                emit(&mut out, "workflow_run", Some("completed"), with(b.clone(), json!({"workflow_run": rj, "workflow": wf})));
                if let Ok(jobs) = s.db.jobs_of_run(run.id(), Some(run.run_attempt)).await {
                    for j in jobs {
                        let jj = crate::actions::rest::job_json(s, rw, &run, &j);
                        emit(&mut out, "workflow_job", Some("completed"), with(b.clone(), json!({"workflow_job": jj})));
                    }
                }
            }
        }
        "wiki" => {
            let page = p["page"].as_str().unwrap_or("");
            let name = page.trim_end_matches(".md");
            emit(
                &mut out,
                "gollum",
                None,
                with(
                    b,
                    json!({"pages": [{"page_name": name, "title": name.replace('-', " "), "summary": Value::Null, "action": "edited", "sha": p["commit"], "html_url": format!("{}/{}/wiki/{}", s.config.site_url(), rw.full_name(), name)}]}),
                ),
            );
        }
        _ => {}
    }
    out
}

/// Events derived from an `issue_events` row.
pub async fn from_issue_event(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, issue: Id, event: &str, p: &Value) -> Emitted {
    let mut out = Emitted::new();
    let Ok(i) = s.db.issue_by_id(issue).await else { return out };
    let is_pull = i.is_pull();
    let b = base(s, rw, actor);
    // The subject document, keyed as GitHub does: `issue` or `pull_request`.
    let subject = async {
        if is_pull {
            pull_json(s, rw, issue).await.map(|v| ("pull_request", v))
        } else {
            issue_json(s, rw, issue).await.map(|v| ("issue", v))
        }
    };
    let ev = if is_pull { "pull_request" } else { "issues" };
    let number = i.number;
    let simple = |out: &mut Emitted, action: &str, extra: Value, subj: Option<(&str, Value)>| {
        if let Some((key, v)) = subj {
            let mut body = with(b.clone(), extra);
            body[key] = v;
            if is_pull {
                body["number"] = json!(number);
            }
            emit(out, ev, Some(action), body);
        }
    };
    match event {
        "closed" => {
            let subj = subject.await;
            simple(&mut out, "closed", json!({}), subj);
        }
        "merged" => {
            // GitHub sends `closed` with `pull_request.merged = true`; the merge itself is
            // the preceding row's business, so nothing extra here.
            let subj = subject.await;
            simple(&mut out, "closed", json!({}), subj);
        }
        "reopened" => {
            let subj = subject.await;
            simple(&mut out, "reopened", json!({}), subj);
        }
        "renamed" => {
            let subj = subject.await;
            simple(&mut out, "edited", json!({"changes": {"title": {"from": p["from"]}}}), subj);
        }
        "labeled" | "unlabeled" => {
            let name = p["label"]["name"].as_str().unwrap_or("");
            let label = match s.db.label_by_name(rw.repo.id(), name).await {
                Ok(l) => shapes::label(&s.config, rw, &l),
                Err(_) => json!({"name": name, "color": p["label"]["color"]}),
            };
            let subj = subject.await;
            simple(&mut out, event, json!({"label": label}), subj);
        }
        "assigned" | "unassigned" => {
            let assignee = match p["assignee_id"].as_i64() {
                Some(id) => s.db.account_by_id(Id(id)).await.ok().map(|a| shapes::simple_user(&s.config, &a)),
                None => None,
            };
            let subj = subject.await;
            simple(&mut out, event, json!({"assignee": assignee}), subj);
        }
        "milestoned" | "demilestoned" => {
            let ms = match p["milestone_id"].as_i64() {
                Some(id) => match s.db.milestone_by_id(Id(id)).await {
                    Ok(m) => {
                        let (open, closed) = s.db.milestone_counts(Id(m.id)).await.unwrap_or((0, 0));
                        Some(shapes::milestone(&s.config, rw, &m, open, closed, None))
                    }
                    Err(_) => None,
                },
                None => None,
            };
            let subj = subject.await;
            simple(&mut out, event, json!({"milestone": ms}), subj);
        }
        "locked" | "unlocked" => {
            let subj = subject.await;
            simple(&mut out, event, json!({}), subj);
        }
        "ready_for_review" => {
            let subj = subject.await;
            simple(&mut out, "ready_for_review", json!({}), subj);
        }
        "convert_to_draft" => {
            let subj = subject.await;
            simple(&mut out, "converted_to_draft", json!({}), subj);
        }
        "review_requested" => {
            let login = p["requested_reviewer"].as_str().unwrap_or("");
            let reviewer = s.db.account_by_login(login).await.ok().map(|a| shapes::simple_user(&s.config, &a));
            let subj = subject.await;
            simple(&mut out, "review_requested", json!({"requested_reviewer": reviewer}), subj);
        }
        "reviewed" => {
            if let (Some(rid), Some(pr)) = (p["review_id"].as_i64(), pull_json(s, rw, issue).await)
                && let Ok(r) = s.db.review_by_id(Id(rid)).await
                && r.state != "PENDING"
            {
                let author = s.db.account_by_id(Id(r.author_id)).await.ok();
                let review = json!({
                    "id": r.id,
                    "node_id": shapes::node_id("PRR_", r.id),
                    "user": author.as_ref().map(|a| shapes::simple_user(&s.config, a)),
                    "body": r.body,
                    "commit_id": r.commit_sha,
                    "submitted_at": r.submitted_at.map(iso),
                    "state": r.state.to_lowercase(),
                    "html_url": format!("{}/{}/pull/{}#pullrequestreview-{}", s.config.site_url(), rw.full_name(), number, r.id),
                    "pull_request_url": format!("{}/repos/{}/pulls/{}", s.config.api_url(), rw.full_name(), number),
                    "author_association": "NONE",
                });
                emit(&mut out, "pull_request_review", Some("submitted"), with(b.clone(), json!({"review": review, "pull_request": pr})));
            }
        }
        "review_commented" => {
            if let (Some(cid), Some(pr)) = (p["comment_id"].as_i64(), pull_json(s, rw, issue).await)
                && let Ok(c) = s.db.review_comment_by_id(Id(cid)).await
            {
                let author = s.db.account_by_id(Id(c.author_id)).await.ok();
                let comment = json!({
                    "id": c.id,
                    "node_id": shapes::node_id("PRRC_", c.id),
                    "pull_request_review_id": c.review_id,
                    "user": author.as_ref().map(|a| shapes::simple_user(&s.config, a)),
                    "body": c.body,
                    "path": c.path,
                    "line": c.line,
                    "start_line": c.start_line,
                    "side": c.side,
                    "commit_id": c.commit_sha,
                    "in_reply_to_id": c.in_reply_to_id,
                    "created_at": iso(c.created_at),
                    "updated_at": iso(c.updated_at),
                    "html_url": format!("{}/{}/pull/{}#discussion_r{}", s.config.site_url(), rw.full_name(), number, c.id),
                    "pull_request_url": format!("{}/repos/{}/pulls/{}", s.config.api_url(), rw.full_name(), number),
                    "author_association": "NONE",
                });
                emit(
                    &mut out,
                    "pull_request_review_comment",
                    Some("created"),
                    with(b.clone(), json!({"comment": comment, "pull_request": pr})),
                );
            }
        }
        _ => {}
    }
    out
}

/// `issue_comment` payload for a new comment.
pub async fn issue_comment(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, issue: Id, comment: Id) -> Option<Value> {
    let c = s.db.comment_by_id(comment).await.ok()?;
    let i = s.db.issue_by_id(issue).await.ok()?;
    let author = s.db.account_by_id(Id(c.author_id)).await.ok()?;
    let issue_v = if i.is_pull() { pull_json(s, rw, issue).await? } else { issue_json(s, rw, issue).await? };
    let comments = crate::rest::issues::comments_json(s, rw, i.number, std::slice::from_ref(&c), None, false).await.ok()?;
    let cj = comments.into_iter().next()?;
    let _ = author;
    Some(with(base(s, rw, actor), json!({"action": "created", "issue": issue_v, "comment": cj})))
}

async fn issue_json(s: &AppState, rw: &RepoWithOwner, issue: Id) -> Option<Value> {
    let i = s.db.issue_full_by_id(issue).await.ok()?;
    crate::rest::issues::issue_json(s, rw, &i, None, false).await.ok()
}

async fn pull_json(s: &AppState, rw: &RepoWithOwner, issue: Id) -> Option<Value> {
    let i = s.db.issue_full_by_id(issue).await.ok()?;
    let m = s.db.pull_meta(issue).await.ok()?;
    crate::rest::pulls::pull_json(s, rw, &i, &m, None, false).await.ok()
}

/// The `push` payload: commits between `before` and `after` (at most 20, oldest first, as
/// GitHub does), `head_commit`, and the pusher.
pub async fn push_payload(s: &AppState, rw: &RepoWithOwner, actor: Option<&Account>, r: &str, before: &str, after: &str) -> Option<Value> {
    let created = before == ZERO_SHA;
    let deleted = after == ZERO_SHA;
    let git = s.store.git_binary();
    let path = s.store.path(rw.repo.id());
    let shas: Vec<String> = if deleted {
        vec![]
    } else if created {
        crate::rest::contents::with_repo(s, rw, {
            let after = after.to_string();
            move |repo| repo.log(&after, None, 0, 20).map(|v| v.into_iter().rev().map(|c| c.id).collect())
        })
        .await
        .unwrap_or_default()
    } else {
        let mut list = reef_git::ops::rev_list(git, &path, before, after).await.unwrap_or_default();
        if list.len() > 20 {
            list = list.split_off(list.len() - 20);
        }
        list
    };
    let mut commits = Vec::new();
    for sha in &shas {
        let sha2 = sha.clone();
        let Ok(c) = crate::rest::contents::with_repo(s, rw, move |repo| repo.commit(&sha2)).await else { continue };
        let author_login = crate::rest::contents::account_for_signature(s, &c.author.name, &c.author.email).await.map(|a| a.login);
        let committer_login = crate::rest::contents::account_for_signature(s, &c.committer.name, &c.committer.email).await.map(|a| a.login);
        commits.push(json!({
            "id": c.id,
            "tree_id": c.tree,
            "distinct": true,
            "message": c.message.trim_end(),
            "timestamp": iso(c.author.time_ms),
            "url": format!("{}/{}/commit/{}", s.config.site_url(), rw.full_name(), c.id),
            "author": {"name": c.author.name, "email": c.author.email, "username": author_login},
            "committer": {"name": c.committer.name, "email": c.committer.email, "username": committer_login},
            "added": [], "removed": [], "modified": [],
        }));
    }
    let head_commit = commits.last().cloned();
    let pusher = actor
        .map(|a| json!({"name": a.login, "email": a.email}))
        .unwrap_or_else(|| json!({"name": rw.owner.login, "email": rw.owner.email}));
    let short = r.trim_start_matches("refs/heads/").trim_start_matches("refs/tags/");
    Some(json!({
        "ref": r,
        "before": before,
        "after": after,
        "created": created,
        "deleted": deleted,
        "forced": false,
        "base_ref": Value::Null,
        "compare": if created || deleted { format!("{}/{}/commits/{}", s.config.site_url(), rw.full_name(), short) } else { format!("{}/{}/compare/{}...{}", s.config.site_url(), rw.full_name(), &before[..12.min(before.len())], &after[..12.min(after.len())]) },
        "commits": commits,
        "head_commit": head_commit,
        "pusher": pusher,
    }))
}
