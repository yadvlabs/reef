//! Workflow file model. GitHub's schema is polymorphic (strings or lists or maps in many
//! places), so we parse YAML into JSON and interpret the shapes we need server-side; anything
//! the runner evaluates itself (step inputs, env, `with`) stays as raw JSON.

use crate::{Error, Result};
use indexmap::IndexMap;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Workflow {
    pub name: Option<String>,
    pub run_name: Option<String>,
    /// Event name → trigger configuration (`Null` when the event has no filters).
    pub on: IndexMap<String, Value>,
    pub env: Value,
    pub defaults: Value,
    pub permissions: Value,
    pub concurrency: Value,
    pub jobs: IndexMap<String, Job>,
}

#[derive(Debug, Clone, Default)]
pub struct Job {
    pub id: String,
    pub name: Option<String>,
    pub needs: Vec<String>,
    pub if_: Option<String>,
    /// Raw `runs-on`: a string, a list, `{group, labels}`, or an expression.
    pub runs_on: Value,
    pub environment: Value,
    pub concurrency: Value,
    pub outputs: IndexMap<String, String>,
    pub env: Value,
    pub defaults: Value,
    pub steps: Vec<Step>,
    pub timeout_minutes: Value,
    pub strategy: Option<Strategy>,
    pub continue_on_error: Value,
    pub container: Value,
    pub services: Value,
    pub permissions: Value,
    /// Reusable workflow call (`uses: owner/repo/.github/workflows/x.yml@ref`).
    pub uses: Option<String>,
    pub with: Value,
    pub secrets: Value,
}

#[derive(Debug, Clone, Default)]
pub struct Strategy {
    pub matrix: Value,
    pub fail_fast: Value,
    pub max_parallel: Value,
}

#[derive(Debug, Clone)]
pub struct Step {
    pub id: Option<String>,
    pub if_: Option<String>,
    pub name: Option<String>,
    pub action: StepAction,
    pub with: Value,
    pub env: Value,
    pub shell: Option<String>,
    pub working_directory: Option<String>,
    pub continue_on_error: Value,
    pub timeout_minutes: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepAction {
    /// `uses: actions/checkout@v4`, `./local/action`, `docker://image:tag`.
    Uses(String),
    /// `run:` script.
    Run(String),
}

/// How a step's `uses:` resolves.
#[derive(Debug, Clone, PartialEq)]
pub enum UsesRef {
    /// `owner/repo[/path]@ref` on this or another GitHub-style host.
    Repository { name_with_owner: String, path: Option<String>, git_ref: String },
    /// `./path` inside the checked-out repository.
    Local(String),
    /// `docker://image:tag`.
    Docker(String),
}

pub fn parse_uses(s: &str) -> Result<UsesRef> {
    if let Some(img) = s.strip_prefix("docker://") {
        return Ok(UsesRef::Docker(img.to_string()));
    }
    if s.starts_with("./") || s.starts_with(".\\") {
        return Ok(UsesRef::Local(s.to_string()));
    }
    let (spec, git_ref) =
        s.rsplit_once('@').ok_or_else(|| Error::Invalid(format!("'{s}' must be owner/repo@ref, ./path or docker://image")))?;
    let mut parts = spec.splitn(3, '/');
    let (Some(owner), Some(repo)) = (parts.next(), parts.next()) else {
        return Err(Error::Invalid(format!("'{s}' is missing owner/repo")));
    };
    if owner.is_empty() || repo.is_empty() || git_ref.is_empty() {
        return Err(Error::Invalid(format!("'{s}' is not a valid action reference")));
    }
    Ok(UsesRef::Repository {
        name_with_owner: format!("{owner}/{repo}"),
        path: parts.next().filter(|p| !p.is_empty()).map(String::from),
        git_ref: git_ref.to_string(),
    })
}

fn yaml_to_json(v: serde_yaml::Value) -> Result<Value> {
    Ok(match v {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                Value::from(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_yaml::Value::String(s) => Value::String(s),
        serde_yaml::Value::Sequence(items) => Value::Array(items.into_iter().map(yaml_to_json).collect::<Result<_>>()?),
        serde_yaml::Value::Mapping(m) => {
            let mut out = serde_json::Map::new();
            for (k, v) in m {
                let key = match k {
                    serde_yaml::Value::String(s) => s,
                    serde_yaml::Value::Bool(true) => "on".to_string(), // YAML 1.1 readers turn `on:` into true
                    serde_yaml::Value::Bool(b) => b.to_string(),
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::Null => "null".to_string(),
                    other => return Err(Error::Invalid(format!("unsupported mapping key {other:?}"))),
                };
                out.insert(key, yaml_to_json(v)?);
            }
            Value::Object(out)
        }
        serde_yaml::Value::Tagged(t) => yaml_to_json(t.value)?,
    })
}

fn opt_string(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items.iter().filter_map(|i| opt_string(Some(i))).collect(),
        _ => vec![],
    }
}

fn take(v: &Value, key: &str) -> Value {
    v.get(key).cloned().unwrap_or(Value::Null)
}

/// Parse a workflow file. Errors name the offending part so they can be shown on the run.
pub fn parse(yaml: &str) -> Result<Workflow> {
    let raw: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|e| Error::Yaml(e.to_string()))?;
    let doc = yaml_to_json(raw)?;
    let Value::Object(root) = &doc else { return Err(Error::Invalid("workflow must be a mapping".into())) };
    let mut wf = Workflow { name: opt_string(root.get("name")), run_name: opt_string(root.get("run-name")), ..Default::default() };
    match root.get("on") {
        None => return Err(Error::Invalid("workflow has no 'on' triggers".into())),
        Some(Value::String(s)) => {
            wf.on.insert(s.clone(), Value::Null);
        }
        Some(Value::Array(items)) => {
            for i in items {
                if let Some(s) = opt_string(Some(i)) {
                    wf.on.insert(s, Value::Null);
                }
            }
        }
        Some(Value::Object(m)) => {
            for (k, v) in m {
                wf.on.insert(k.clone(), v.clone());
            }
        }
        Some(other) => return Err(Error::Invalid(format!("'on' has an unexpected shape: {other}"))),
    }
    wf.env = take(&doc, "env");
    wf.defaults = take(&doc, "defaults");
    wf.permissions = take(&doc, "permissions");
    wf.concurrency = take(&doc, "concurrency");
    let Some(Value::Object(jobs)) = root.get("jobs") else { return Err(Error::Invalid("workflow has no 'jobs'".into())) };
    if jobs.is_empty() {
        return Err(Error::Invalid("'jobs' is empty".into()));
    }
    for (id, j) in jobs {
        wf.jobs.insert(id.clone(), parse_job(id, j)?);
    }
    // Validate `needs` references and detect cycles.
    for job in wf.jobs.values() {
        for n in &job.needs {
            if !wf.jobs.contains_key(n) {
                return Err(Error::Invalid(format!("job '{}' needs unknown job '{n}'", job.id)));
            }
        }
    }
    topo_order(&wf)?;
    Ok(wf)
}

fn parse_job(id: &str, v: &Value) -> Result<Job> {
    let Value::Object(_) = v else { return Err(Error::Invalid(format!("job '{id}' must be a mapping"))) };
    let mut job = Job {
        id: id.to_string(),
        name: opt_string(v.get("name")),
        needs: string_list(v.get("needs")),
        if_: opt_string(v.get("if")),
        runs_on: take(v, "runs-on"),
        environment: take(v, "environment"),
        concurrency: take(v, "concurrency"),
        env: take(v, "env"),
        defaults: take(v, "defaults"),
        timeout_minutes: take(v, "timeout-minutes"),
        continue_on_error: take(v, "continue-on-error"),
        container: take(v, "container"),
        services: take(v, "services"),
        permissions: take(v, "permissions"),
        uses: opt_string(v.get("uses")),
        with: take(v, "with"),
        secrets: take(v, "secrets"),
        ..Default::default()
    };
    if let Some(Value::Object(outs)) = v.get("outputs") {
        for (k, o) in outs {
            job.outputs.insert(k.clone(), opt_string(Some(o)).unwrap_or_default());
        }
    }
    if let Some(s) = v.get("strategy") {
        job.strategy = Some(Strategy { matrix: take(s, "matrix"), fail_fast: take(s, "fail-fast"), max_parallel: take(s, "max-parallel") });
    }
    if job.uses.is_none() {
        if job.runs_on.is_null() {
            return Err(Error::Invalid(format!("job '{id}' has no 'runs-on'")));
        }
        let Some(Value::Array(steps)) = v.get("steps") else { return Err(Error::Invalid(format!("job '{id}' has no 'steps'"))) };
        for (i, s) in steps.iter().enumerate() {
            job.steps.push(parse_step(id, i, s)?);
        }
    }
    Ok(job)
}

fn parse_step(job: &str, i: usize, v: &Value) -> Result<Step> {
    let Value::Object(_) = v else { return Err(Error::Invalid(format!("job '{job}' step {} must be a mapping", i + 1))) };
    let action = match (opt_string(v.get("uses")), v.get("run")) {
        (Some(u), None) => StepAction::Uses(u),
        (None, Some(r)) => StepAction::Run(opt_string(Some(r)).unwrap_or_default()),
        (Some(_), Some(_)) => return Err(Error::Invalid(format!("job '{job}' step {} has both 'uses' and 'run'", i + 1))),
        (None, None) => return Err(Error::Invalid(format!("job '{job}' step {} needs 'uses' or 'run'", i + 1))),
    };
    Ok(Step {
        id: opt_string(v.get("id")),
        if_: opt_string(v.get("if")),
        name: opt_string(v.get("name")),
        action,
        with: take(v, "with"),
        env: take(v, "env"),
        shell: opt_string(v.get("shell")),
        working_directory: opt_string(v.get("working-directory")),
        continue_on_error: take(v, "continue-on-error"),
        timeout_minutes: take(v, "timeout-minutes"),
    })
}

/// Jobs in dependency order; errors on cycles.
pub fn topo_order(wf: &Workflow) -> Result<Vec<&Job>> {
    let mut out: Vec<&Job> = Vec::new();
    let mut state: IndexMap<&str, u8> = IndexMap::new(); // 1 visiting, 2 done
    fn visit<'a>(wf: &'a Workflow, id: &'a str, state: &mut IndexMap<&'a str, u8>, out: &mut Vec<&'a Job>) -> Result<()> {
        match state.get(id) {
            Some(2) => return Ok(()),
            Some(1) => return Err(Error::Invalid(format!("jobs form a dependency cycle through '{id}'"))),
            _ => {}
        }
        state.insert(id, 1);
        let job = &wf.jobs[id];
        for n in &job.needs {
            visit(wf, n, state, out)?;
        }
        state.insert(id, 2);
        out.push(job);
        Ok(())
    }
    for id in wf.jobs.keys() {
        visit(wf, id, &mut state, &mut out)?;
    }
    Ok(out)
}

/// Resolve `runs-on` to runner labels given the evaluation context.
pub fn runs_on_labels(runs_on: &Value, ctx: &Value) -> Result<Vec<String>> {
    let v = crate::expr::evaluate_value(runs_on, ctx, crate::expr::Status::all_ok())?;
    let labels: Vec<String> = match &v {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items.iter().map(crate::expr::to_string).collect(),
        Value::Object(m) => match m.get("labels") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(items)) => items.iter().map(crate::expr::to_string).collect(),
            _ => vec![],
        },
        _ => vec![],
    };
    let labels: Vec<String> = labels.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    if labels.is_empty() {
        return Err(Error::Invalid("runs-on resolved to no labels".into()));
    }
    Ok(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WF: &str = r#"
name: CI
on:
  push:
    branches: [main, 'release/**']
    paths-ignore: ['docs/**']
  pull_request:
  workflow_dispatch:
    inputs:
      debug:
        type: boolean
        default: false
env:
  CARGO_TERM_COLOR: always
jobs:
  build:
    name: Build (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v4
      - name: Test
        run: cargo test
        env:
          RUST_BACKTRACE: 1
  deploy:
    needs: [build]
    if: github.ref == 'refs/heads/main'
    runs-on: [self-hosted, linux]
    steps:
      - run: ./deploy.sh
"#;

    #[test]
    fn parses() {
        let wf = parse(WF).unwrap();
        assert_eq!(wf.name.as_deref(), Some("CI"));
        assert_eq!(wf.on.len(), 3);
        assert!(wf.on["pull_request"].is_null());
        assert_eq!(wf.jobs.len(), 2);
        let b = &wf.jobs["build"];
        assert_eq!(b.steps.len(), 2);
        assert_eq!(b.steps[0].action, StepAction::Uses("actions/checkout@v4".into()));
        assert!(b.strategy.is_some());
        let d = &wf.jobs["deploy"];
        assert_eq!(d.needs, vec!["build"]);
        assert_eq!(runs_on_labels(&d.runs_on, &Value::Null).unwrap(), vec!["self-hosted", "linux"]);
        let ctx = serde_json::json!({"matrix": {"os": "macos-latest"}});
        assert_eq!(runs_on_labels(&b.runs_on, &ctx).unwrap(), vec!["macos-latest"]);
        let order: Vec<&str> = topo_order(&wf).unwrap().iter().map(|j| j.id.as_str()).collect();
        assert_eq!(order, vec!["build", "deploy"]);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(parse("name: x\njobs: {}").is_err());
        assert!(parse("on: push\njobs:\n  a:\n    runs-on: x\n    needs: b\n    steps: [{run: x}]\n  b:\n    runs-on: x\n    needs: a\n    steps: [{run: x}]").is_err());
        assert!(parse("on: push\njobs:\n  a:\n    steps: [{run: x}]").is_err());
    }

    #[test]
    fn uses_refs() {
        assert_eq!(
            parse_uses("actions/checkout@v4").unwrap(),
            UsesRef::Repository { name_with_owner: "actions/checkout".into(), path: None, git_ref: "v4".into() }
        );
        assert_eq!(
            parse_uses("octo/tools/sub/dir@main").unwrap(),
            UsesRef::Repository { name_with_owner: "octo/tools".into(), path: Some("sub/dir".into()), git_ref: "main".into() }
        );
        assert_eq!(parse_uses("./.github/actions/x").unwrap(), UsesRef::Local("./.github/actions/x".into()));
        assert_eq!(parse_uses("docker://alpine:3").unwrap(), UsesRef::Docker("alpine:3".into()));
        assert!(parse_uses("nope").is_err());
    }
}
