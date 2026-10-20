//! Does an event trigger a workflow? Applies `branches`/`tags`/`paths` filters and activity
//! `types` the way GitHub documents them.

use crate::glob;
use crate::workflow::Workflow;
use serde_json::Value;

/// A thing that happened, in the terms workflows filter on.
#[derive(Debug, Clone, Default)]
pub struct Event {
    /// `push`, `pull_request`, `workflow_dispatch`, `issues`, ...
    pub name: String,
    /// Activity type for events that have one (`opened`, `synchronize`, `created`, ...).
    pub action: Option<String>,
    /// Full ref the filters apply to: for push the pushed ref; for pull_request the base ref.
    pub git_ref: String,
    /// Paths changed by the push / PR; `None` when unknown (then path filters pass).
    pub changed_paths: Option<Vec<String>>,
}

fn list(cfg: &Value, key: &str) -> Vec<String> {
    match cfg.get(key) {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items.iter().filter_map(|i| i.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

fn default_types(event: &str) -> &'static [&'static str] {
    match event {
        "pull_request" | "pull_request_target" => &["opened", "synchronize", "reopened"],
        "issues" => &[
            "opened",
            "edited",
            "deleted",
            "transferred",
            "pinned",
            "unpinned",
            "closed",
            "reopened",
            "assigned",
            "unassigned",
            "labeled",
            "unlabeled",
            "locked",
            "unlocked",
            "milestoned",
            "demilestoned",
        ],
        "issue_comment" => &["created", "edited", "deleted"],
        "release" => &["published", "unpublished", "created", "edited", "deleted", "prereleased", "released"],
        "pull_request_review" => &["submitted", "edited", "dismissed"],
        "pull_request_review_comment" => &["created", "edited", "deleted"],
        "check_run" => &["created", "rerequested", "completed", "requested_action"],
        "workflow_run" => &["requested", "completed", "in_progress"],
        _ => &[],
    }
}

/// True when the workflow's `on` matches the event.
pub fn matches(wf: &Workflow, ev: &Event) -> bool {
    let Some(cfg) = wf.on.get(&ev.name) else { return false };
    let cfg = if cfg.is_null() { &Value::Object(Default::default()) } else { cfg };

    // Activity types.
    if let Some(action) = &ev.action {
        let types = list(cfg, "types");
        if !types.is_empty() {
            if !types.iter().any(|t| t == action) {
                return false;
            }
        } else {
            let defaults = default_types(&ev.name);
            if !defaults.is_empty() && !defaults.contains(&action.as_str()) {
                return false;
            }
        }
    }

    // Branch / tag filters.
    let branches = list(cfg, "branches");
    let branches_ignore = list(cfg, "branches-ignore");
    let tags = list(cfg, "tags");
    let tags_ignore = list(cfg, "tags-ignore");
    if let Some(branch) = ev.git_ref.strip_prefix("refs/heads/") {
        if !branches.is_empty() && !glob::filter(&branches, branch) {
            return false;
        }
        if branches.is_empty() && !tags.is_empty() && ev.name == "push" {
            return false; // only tags configured: branch pushes don't trigger
        }
        if !branches_ignore.is_empty() && glob::filter(&branches_ignore, branch) {
            return false;
        }
    } else if let Some(tag) = ev.git_ref.strip_prefix("refs/tags/") {
        if !tags.is_empty() && !glob::filter(&tags, tag) {
            return false;
        }
        if tags.is_empty() && !branches.is_empty() && ev.name == "push" {
            return false; // only branches configured: tag pushes don't trigger
        }
        if !tags_ignore.is_empty() && glob::filter(&tags_ignore, tag) {
            return false;
        }
    }

    // Path filters.
    let paths = list(cfg, "paths");
    let paths_ignore = list(cfg, "paths-ignore");
    if let Some(changed) = ev.changed_paths.as_ref()
        && (!paths.is_empty() || !paths_ignore.is_empty())
    {
        if !paths.is_empty() && !changed.iter().any(|p| glob::filter(&paths, p)) {
            return false;
        }
        if !paths_ignore.is_empty() && paths.is_empty() && changed.iter().all(|p| glob::filter(&paths_ignore, p)) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::parse;

    fn wf(on: &str) -> Workflow {
        parse(&format!("on:\n{on}\njobs:\n  a:\n    runs-on: x\n    steps: [{{run: x}}]")).unwrap()
    }

    fn push(r: &str, paths: Option<Vec<&str>>) -> Event {
        Event {
            name: "push".into(),
            action: None,
            git_ref: r.into(),
            changed_paths: paths.map(|p| p.into_iter().map(String::from).collect()),
        }
    }

    #[test]
    fn push_filters() {
        let w = wf("  push:\n    branches: [main, 'release/**']\n    paths-ignore: ['docs/**']");
        assert!(matches(&w, &push("refs/heads/main", None)));
        assert!(matches(&w, &push("refs/heads/release/1.x", Some(vec!["src/a.rs", "docs/b.md"]))));
        assert!(!matches(&w, &push("refs/heads/dev", None)));
        assert!(!matches(&w, &push("refs/heads/main", Some(vec!["docs/b.md"]))));
        assert!(!matches(&w, &push("refs/tags/v1", None)));
        let t = wf("  push:\n    tags: ['v*']");
        assert!(matches(&t, &push("refs/tags/v1", None)));
        assert!(!matches(&t, &push("refs/heads/main", None)));
        let any = wf("  push:");
        assert!(matches(&any, &push("refs/tags/v1", None)));
        assert!(matches(&any, &push("refs/heads/x", None)));
        assert!(!matches(&any, &Event { name: "pull_request".into(), ..Default::default() }));
    }

    #[test]
    fn pr_types() {
        let w = wf("  pull_request:\n    types: [opened, labeled]\n    branches: [main]");
        let ev =
            |a: &str| Event { name: "pull_request".into(), action: Some(a.into()), git_ref: "refs/heads/main".into(), changed_paths: None };
        assert!(matches(&w, &ev("opened")));
        assert!(matches(&w, &ev("labeled")));
        assert!(!matches(&w, &ev("synchronize")));
        let d = wf("  pull_request:");
        assert!(matches(&d, &ev("synchronize")));
        assert!(!matches(&d, &ev("labeled")));
    }
}
