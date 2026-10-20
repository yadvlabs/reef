//! Build the `AgentJobRequestMessage` the runner executes, from a planned job and its
//! workflow definition. Encodings follow docs/02-runner-protocol.md section 6.

use super::event;
use crate::AppState;
use reef_actions::workflow::{Job as WfJob, StepAction, Workflow};
use reef_core::Result;
use reef_core::time::iso;
use reef_db::accounts::Account;
use reef_db::actions::{Job, Run};
use reef_db::repos::RepoWithOwner;
use serde_json::{Map, Value, json};

/// PipelineContextData encoding: dictionaries carry a type tag, scalars are bare JSON.
pub fn context_data(v: &Value) -> Value {
    match v {
        Value::Object(m) => json!({"t": 2, "d": m.iter().map(|(k, v)| json!({"k": k, "v": context_data(v)})).collect::<Vec<_>>()}),
        Value::Array(a) => json!({"t": 1, "a": a.iter().map(context_data).collect::<Vec<_>>()}),
        other => other.clone(),
    }
}

/// A single-quoted expression string literal.
fn expr_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// TemplateToken for a string that may contain `${{ }}`: a literal, a single expression, or
/// a `format('...', a, b)` expression for mixed text.
pub fn string_token(s: &str) -> Value {
    if !s.contains("${{") {
        return json!({"type": 0, "lit": s});
    }
    let mut parts: Vec<String> = Vec::new();
    let mut fmt = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${{") {
        fmt.push_str(&rest[..start].replace('{', "{{").replace('}', "}}"));
        let after = &rest[start + 3..];
        let Some(end) = after.find("}}") else {
            fmt.push_str(&after.replace('{', "{{").replace('}', "}}"));
            rest = "";
            break;
        };
        fmt.push_str(&format!("{{{}}}", parts.len()));
        parts.push(after[..end].trim().to_string());
        rest = &after[end + 2..];
    }
    fmt.push_str(&rest.replace('{', "{{").replace('}', "}}"));
    if parts.len() == 1 && fmt == "{0}" {
        return json!({"type": 3, "expr": parts[0]});
    }
    let mut args = vec![expr_string(&fmt)];
    args.extend(parts);
    json!({"type": 3, "expr": format!("format({})", args.join(", "))})
}

/// TemplateToken for any JSON value from the workflow file.
pub fn token(v: &Value) -> Value {
    match v {
        Value::Null => json!({"type": 7}),
        Value::Bool(b) => json!({"type": 5, "bool": b}),
        Value::Number(n) => json!({"type": 6, "num": n}),
        Value::String(s) => string_token(s),
        Value::Array(a) => json!({"type": 1, "seq": a.iter().map(token).collect::<Vec<_>>()}),
        Value::Object(m) => mapping(m),
    }
}

pub fn mapping(m: &Map<String, Value>) -> Value {
    json!({"type": 2, "map": m.iter().map(|(k, v)| json!({"key": {"type": 0, "lit": k}, "value": token(v)})).collect::<Vec<_>>()})
}

fn condition_of(if_: Option<&str>) -> String {
    match if_.map(str::trim).filter(|s| !s.is_empty()) {
        Some(c) => c.strip_prefix("${{").and_then(|s| s.strip_suffix("}}")).map(|s| s.trim().to_string()).unwrap_or_else(|| c.to_string()),
        None => "success()".into(),
    }
}

fn scalar_token(v: &Value) -> Option<Value> {
    if v.is_null() { None } else { Some(token(v)) }
}

/// Steps in the runner's shape.
pub fn steps(job: &WfJob) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for (i, st) in job.steps.iter().enumerate() {
        let id = reef_db::actions::uuid_v4();
        let name = st.id.clone().unwrap_or_else(|| if i == 0 { "__run".to_string() } else { format!("__run_{}", i + 1) });
        let (reference, inputs, display) = match &st.action {
            StepAction::Uses(u) => {
                let r = reef_actions::workflow::parse_uses(u).map_err(|e| reef_core::Error::validation(e.to_string()))?;
                let reference = match r {
                    reef_actions::workflow::UsesRef::Repository { name_with_owner, path, git_ref } => {
                        json!({"type": "repository", "name": name_with_owner, "ref": git_ref, "repositoryType": "GitHub", "path": path})
                    }
                    reef_actions::workflow::UsesRef::Docker(img) => json!({"type": "containerRegistry", "image": img}),
                    reef_actions::workflow::UsesRef::Local(p) => {
                        json!({"type": "repository", "name": "", "ref": "", "repositoryType": "GitHub", "path": p.trim_start_matches("./")})
                    }
                };
                let inputs = match &st.with {
                    Value::Object(m) => mapping(m),
                    _ => json!({"type": 2, "map": []}),
                };
                (reference, inputs, format!("Run {u}"))
            }
            StepAction::Run(script) => {
                let mut m = Map::new();
                m.insert("script".into(), Value::String(script.clone()));
                if let Some(sh) = &st.shell {
                    m.insert("shell".into(), Value::String(sh.clone()));
                }
                if let Some(wd) = &st.working_directory {
                    m.insert("workingDirectory".into(), Value::String(wd.clone()));
                }
                let first = script.lines().next().unwrap_or("").trim().to_string();
                (json!({"type": "script"}), mapping(&m), format!("Run {}", first))
            }
        };
        let display = st.name.clone().unwrap_or(display);
        let mut v = json!({
            "type": "action",
            "id": id,
            "name": name,
            "contextName": st.id,
            "condition": condition_of(st.if_.as_deref()),
            "enabled": true,
            "reference": reference,
            "inputs": inputs,
            "environment": match &st.env { Value::Object(m) => mapping(m), _ => Value::Null },
        });
        if reef_actions::expr::has_expression(&display) {
            v["displayNameToken"] = string_token(&display);
        } else {
            v["displayName"] = json!(display);
        }
        if let Some(t) = scalar_token(&st.continue_on_error) {
            v["continueOnError"] = t;
        }
        if let Some(t) = scalar_token(&st.timeout_minutes) {
            v["timeoutInMinutes"] = t;
        }
        out.push(v);
    }
    Ok(out)
}

pub struct MessageInput<'a> {
    pub rw: &'a RepoWithOwner,
    pub run: &'a Run,
    pub job: &'a Job,
    pub wf: &'a Workflow,
    pub def: &'a WfJob,
    pub actor: Option<&'a Account>,
    /// Base URL for job callbacks (the tenant URL).
    pub tenant_url: &'a str,
    /// JWT presented on callbacks (SystemVssConnection.AccessToken).
    pub access_token: &'a str,
    /// The job's GITHUB_TOKEN.
    pub github_token: &'a str,
    pub request_id: i64,
    pub needs: Value,
    pub vars: Map<String, Value>,
    pub secrets: Map<String, Value>,
    pub runner_group: &'a str,
    pub cache_url: Option<&'a str>,
}

/// The complete job message (still unencrypted JSON).
pub fn build(s: &AppState, m: MessageInput<'_>) -> Result<Value> {
    let github = event::github_context(s, m.rw, m.run, m.job, m.actor, None);
    let matrix = m.job.matrix_json();
    let legs = 1;
    let strategy = json!({"fail-fast": m.def.strategy.as_ref().map(|st| !matches!(st.fail_fast, Value::Bool(false))).unwrap_or(true), "max-parallel": legs, "job-index": 0, "job-total": legs});
    let inputs: Value = m.run.inputs.as_deref().and_then(|i| serde_json::from_str(i).ok()).unwrap_or(json!({}));

    let mut context_data_map = Map::new();
    context_data_map.insert("github".into(), context_data(&github));
    context_data_map.insert("needs".into(), context_data(&m.needs));
    context_data_map.insert("strategy".into(), context_data(&strategy));
    context_data_map.insert("matrix".into(), if matrix.is_null() { Value::Null } else { context_data(&matrix) });
    context_data_map.insert("inputs".into(), context_data(&inputs));
    context_data_map.insert("vars".into(), context_data(&Value::Object(m.vars.clone())));

    let mut variables = Map::new();
    let var = |v: &str| json!({"value": v});
    variables.insert("system.github.token".into(), json!({"value": m.github_token, "isSecret": true}));
    variables.insert("github_token".into(), json!({"value": m.github_token, "isSecret": true}));
    variables.insert("system.github.job".into(), var(&m.job.job_key));
    variables.insert("system.workflowFilePath".into(), var(&m.run.path));
    variables.insert("system.workflowFileFullPath".into(), var(&format!("{}/{}@{}", m.rw.full_name(), m.run.path, m.run.git_ref)));
    variables.insert("system.phaseDisplayName".into(), var(&m.job.name));
    variables.insert("system.runnerGroupName".into(), var(m.runner_group));
    for flag in [
        "DistributedTask.NewActionMetadata",
        "DistributedTask.EnableCompositeActions",
        "DistributedTask.EnhancedAnnotations",
        "DistributedTask.UploadStepSummary",
        "DistributedTask.AddWarningToNode12Action",
        "DistributedTask.AllowRunnerContainerHooks",
        "DistributedTask.DeprecateStepOutputCommands",
        "DistributedTask.ForceGithubJavascriptActionsToNode16",
    ] {
        variables.insert(flag.into(), var("true"));
    }
    for (k, v) in &m.secrets {
        variables.insert(k.clone(), json!({"value": v.as_str().unwrap_or(""), "isSecret": true}));
    }

    let mut env_list = Vec::new();
    if let Value::Object(e) = &m.wf.env {
        env_list.push(mapping(e));
    }
    if let Value::Object(e) = &m.def.env {
        env_list.push(mapping(e));
    }
    let mut defaults = Vec::new();
    if let Value::Object(d) = &m.wf.defaults {
        defaults.push(mapping(d));
    }
    if let Value::Object(d) = &m.def.defaults {
        defaults.push(mapping(d));
    }
    let outputs = if m.def.outputs.is_empty() {
        Value::Null
    } else {
        mapping(&m.def.outputs.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect())
    };
    let actions_environment = match &m.def.environment {
        Value::String(n) => json!({"name": n, "url": Value::Null}),
        Value::Object(o) => json!({"name": o.get("name").and_then(|n| n.as_str()).unwrap_or(""), "url": o.get("url").map(token)}),
        _ => Value::Null,
    };
    let mut data = json!({"CacheServerUrl": format!("{}/", m.tenant_url.trim_end_matches('/'))});
    if let Some(c) = m.cache_url {
        data["CacheServerUrl"] = json!(c);
    }

    Ok(json!({
        "messageType": "PipelineAgentJobRequest",
        "plan": {
            "scopeIdentifier": "00000000-0000-0000-0000-000000000000",
            "planType": "Actions",
            "version": 12,
            "planId": m.job.plan_id,
            "planGroup": "Actions",
            "owner": {"id": 0, "name": "Reef"},
            "definition": Value::Null, "artifactUri": Value::Null, "artifactLocation": Value::Null
        },
        "timeline": {"id": m.job.timeline_id, "changeId": 1, "location": Value::Null},
        "jobId": m.job.job_guid,
        "jobDisplayName": m.job.name,
        "jobName": m.job.job_key,
        "requestId": m.request_id,
        "lockedUntil": iso(reef_core::time::now_ms() + 10 * 60_000),
        "resources": {
            "endpoints": [{
                "id": "00000000-0000-0000-0000-000000000000",
                "name": "SystemVssConnection",
                "type": "SystemVssConnection",
                "url": m.tenant_url,
                "authorization": {"scheme": "OAuth", "parameters": {"AccessToken": m.access_token}},
                "data": data,
                "isShared": false,
                "isReady": true
            }],
            "containers": [],
            "repositories": []
        },
        "variables": variables,
        "mask": [],
        "contextData": context_data_map,
        "workspace": {},
        "jobContainer": if m.def.container.is_null() { Value::Null } else { token(&m.def.container) },
        "jobServiceContainers": if m.def.services.is_null() { Value::Null } else { token(&m.def.services) },
        "jobOutputs": outputs,
        "defaults": defaults,
        "environmentVariables": env_list,
        "fileTable": [m.run.path],
        "actionsEnvironment": actions_environment,
        "steps": steps(m.def)?,
        "snapshot": Value::Null,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(string_token("plain"), json!({"type": 0, "lit": "plain"}));
        assert_eq!(string_token("${{ github.sha }}"), json!({"type": 3, "expr": "github.sha"}));
        assert_eq!(string_token("v-${{ matrix.os }}-{x}"), json!({"type": 3, "expr": "format('v-{0}-{{x}}', matrix.os)"}));
        assert_eq!(
            context_data(&json!({"a": [1, "x"], "b": true})),
            json!({"t": 2, "d": [{"k": "a", "v": {"t": 1, "a": [1, "x"]}}, {"k": "b", "v": true}]})
        );
        assert_eq!(condition_of(Some("${{ always() }}")), "always()");
        assert_eq!(condition_of(None), "success()");
    }
}
