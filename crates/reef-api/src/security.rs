//! Repository security tab: the SECURITY.md policy and a dependency inventory parsed from
//! manifest files (Reef API, `GET /api/reef/repos/{owner}/{repo}/security`).
//!
//! The inventory is a best-effort static read of the manifests on the default branch; it is
//! not a lockfile resolution and lists direct dependencies only.

use crate::AppState;
use crate::auth::Auth;
use crate::error::ApiResult;
use crate::rest::contents::with_repo;
use crate::rest::repos::load;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use reef_git::EntryKind;
use serde_json::{Value, json};

/// A read-only dependency scan over the tree; timed like any other route.
pub fn router() -> Router<AppState> {
    Router::new().route("/repos/{owner}/{repo}/security", get(security))
}

/// A parsed package requirement.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Package {
    pub name: String,
    pub requirement: String,
    pub kind: &'static str, // "runtime" | "dev"
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Manifest {
    pub manifest: String,
    pub ecosystem: &'static str,
    pub packages: Vec<Package>,
}

/// Directories never descended into when looking for manifests.
const SKIP_DIRS: &[&str] = &["node_modules", "vendor", "target", "dist", "build", ".git", "third_party", "__pycache__", ".venv", "venv"];

/// Which parser handles a file name, if any.
fn ecosystem_of(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "cargo.toml" => "cargo",
        "package.json" => "npm",
        "go.mod" => "go",
        "pyproject.toml" => "pip",
        "pom.xml" => "maven",
        "build.gradle" | "build.gradle.kts" => "gradle",
        "composer.json" => "composer",
        "gemfile" => "gem",
        _ => {
            if (lower.starts_with("requirements") && lower.ends_with(".txt")) || lower == "requirements.in" {
                "pip"
            } else if lower.ends_with(".csproj") || lower.ends_with(".fsproj") || lower.ends_with(".vbproj") {
                "nuget"
            } else {
                return None;
            }
        }
    })
}

fn is_policy(path: &str) -> bool {
    let lower = path.to_lowercase();
    matches!(lower.as_str(), "security.md" | "security" | "security.txt" | "security.rst" | "docs/security.md" | ".github/security.md")
}

pub async fn security(State(s): State<AppState>, auth: Auth, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (rw, _) = load(&s, &auth, &owner, &repo).await?;
    if rw.repo.is_empty != 0 {
        return Ok(Json(json!({"policy": Value::Null, "advisories": [], "dependencies": []})));
    }
    let branch = rw.repo.default_branch.clone();
    let (policy, manifests) = with_repo(&s, &rw, move |r| {
        let mut policy: Option<(String, String)> = None;
        let mut manifests: Vec<Manifest> = Vec::new();
        let mut stack = vec![(String::new(), 0usize)];
        let mut seen = 0usize;
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = r.tree(&branch, &dir) else { continue };
            for e in entries {
                seen += 1;
                if seen > 20_000 {
                    break;
                }
                match e.kind {
                    EntryKind::Tree if depth < 4 && !SKIP_DIRS.contains(&e.name.as_str()) => {
                        stack.push((e.path.clone(), depth + 1));
                    }
                    EntryKind::Blob | EntryKind::Executable => {
                        if policy.is_none()
                            && is_policy(&e.path)
                            && let Ok(b) = r.blob(&e.id)
                        {
                            policy = Some((e.path.clone(), String::from_utf8_lossy(&b.data).into_owned()));
                        }
                        if let Some(eco) = ecosystem_of(&e.name) {
                            let Ok(b) = r.blob(&e.id) else { continue };
                            if b.size > 2_000_000 {
                                continue;
                            }
                            let text = String::from_utf8_lossy(&b.data);
                            let packages = parse(eco, &e.name, &text);
                            if !packages.is_empty() || eco == "npm" || eco == "cargo" {
                                manifests.push(Manifest { manifest: e.path.clone(), ecosystem: eco, packages });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        manifests.sort_by(|a, b| a.manifest.cmp(&b.manifest));
        Ok((policy, manifests))
    })
    .await?;
    let policy_json = policy.map(|(path, text)| {
        let html = if path.to_lowercase().ends_with(".md") || !path.contains('.') {
            crate::render::markdown_in_repo(&text, &s.config, &rw)
        } else {
            format!("<pre>{}</pre>", ammonia::clean_text(&text))
        };
        json!({"path": path, "html": html})
    });
    let total: usize = manifests.iter().map(|m| m.packages.len()).sum();
    Ok(Json(json!({
        "policy": policy_json,
        "advisories": [],
        "dependencies": manifests,
        "total_dependencies": total,
    })))
}

// ----- parsers -----

pub fn parse(eco: &str, name: &str, text: &str) -> Vec<Package> {
    match eco {
        "cargo" => parse_cargo(text),
        "npm" => parse_json_deps(
            text,
            &[
                ("dependencies", "runtime"),
                ("peerDependencies", "runtime"),
                ("optionalDependencies", "runtime"),
                ("devDependencies", "dev"),
            ],
        ),
        "composer" => parse_json_deps(text, &[("require", "runtime"), ("require-dev", "dev")]),
        "go" => parse_go_mod(text),
        "pip" => {
            if name.eq_ignore_ascii_case("pyproject.toml") {
                parse_pyproject(text)
            } else {
                parse_requirements(
                    text,
                    if name.to_lowercase().contains("dev") || name.to_lowercase().contains("test") { "dev" } else { "runtime" },
                )
            }
        }
        "maven" => parse_pom(text),
        "gradle" => parse_gradle(text),
        "gem" => parse_gemfile(text),
        "nuget" => parse_csproj(text),
        _ => Vec::new(),
    }
}

fn pkg(name: &str, requirement: &str, kind: &'static str) -> Package {
    Package { name: name.trim().to_string(), requirement: requirement.trim().to_string(), kind }
}

/// Cargo.toml: `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, target-specific
/// tables, and `[workspace.dependencies]`. Inline tables use `version` (or `git`/`path`).
fn parse_cargo(text: &str) -> Vec<Package> {
    let Ok(doc) = text.parse::<toml::Table>() else { return Vec::new() };
    let mut out = Vec::new();
    let mut visit = |table: &toml::Table, kind: &'static str| {
        for (name, v) in table {
            let req = match v {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .or_else(|| t.get("git").and_then(|x| x.as_str()).map(|g| format!("git: {g}")))
                    .or_else(|| t.get("path").and_then(|x| x.as_str()).map(|p| format!("path: {p}")))
                    .or_else(|| t.get("workspace").and_then(|x| x.as_bool()).filter(|b| *b).map(|_| "workspace".to_string()))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            out.push(pkg(name, &req, kind));
        }
    };
    for (key, kind) in [("dependencies", "runtime"), ("build-dependencies", "runtime"), ("dev-dependencies", "dev")] {
        if let Some(t) = doc.get(key).and_then(|v| v.as_table()) {
            visit(t, kind);
        }
    }
    if let Some(ws) = doc.get("workspace").and_then(|v| v.as_table()).and_then(|w| w.get("dependencies")).and_then(|v| v.as_table()) {
        visit(ws, "runtime");
    }
    if let Some(targets) = doc.get("target").and_then(|v| v.as_table()) {
        for (_, t) in targets {
            if let Some(t) = t.as_table() {
                for (key, kind) in [("dependencies", "runtime"), ("build-dependencies", "runtime"), ("dev-dependencies", "dev")] {
                    if let Some(d) = t.get(key).and_then(|v| v.as_table()) {
                        visit(d, kind);
                    }
                }
            }
        }
    }
    out
}

fn parse_json_deps(text: &str, sections: &[(&str, &'static str)]) -> Vec<Package> {
    let Ok(doc) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut out = Vec::new();
    for (key, kind) in sections {
        if let Some(map) = doc.get(*key).and_then(|v| v.as_object()) {
            for (name, v) in map {
                if *key == "require" && (name == "php" || name.starts_with("ext-")) {
                    continue;
                }
                out.push(pkg(name, v.as_str().unwrap_or(""), kind));
            }
        }
    }
    out
}

fn parse_go_mod(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut in_block = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("require (") {
            in_block = true;
            continue;
        }
        if in_block && line == ")" {
            in_block = false;
            continue;
        }
        let spec = if in_block { Some(line) } else { line.strip_prefix("require ").map(str::trim) };
        if let Some(spec) = spec {
            let mut parts = spec.split_whitespace();
            if let (Some(name), Some(ver)) = (parts.next(), parts.next()) {
                let indirect = raw.contains("// indirect");
                out.push(pkg(name, ver, if indirect { "dev" } else { "runtime" }));
            }
        }
    }
    out
}

fn parse_requirements(text: &str, kind: &'static str) -> Vec<Package> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('-') || line.starts_with("http") {
            continue;
        }
        let spec = line.split(';').next().unwrap_or(line).trim();
        let idx = spec.find(|c: char| "=<>!~[ @".contains(c)).unwrap_or(spec.len());
        let (name, rest) = spec.split_at(idx);
        let req = rest.trim_start_matches([' ', '@']);
        if !name.is_empty() {
            out.push(pkg(name, req, kind));
        }
    }
    out
}

fn parse_pyproject(text: &str) -> Vec<Package> {
    let Ok(doc) = text.parse::<toml::Table>() else { return Vec::new() };
    let mut out = Vec::new();
    let split = |s: &str| -> (String, String) {
        let idx = s.find(|c: char| "=<>!~[ ;".contains(c)).unwrap_or(s.len());
        let (n, r) = s.split_at(idx);
        (n.trim().to_string(), r.split(';').next().unwrap_or("").trim().to_string())
    };
    if let Some(project) = doc.get("project").and_then(|v| v.as_table()) {
        if let Some(deps) = project.get("dependencies").and_then(|v| v.as_array()) {
            for d in deps.iter().filter_map(|v| v.as_str()) {
                let (n, r) = split(d);
                out.push(pkg(&n, &r, "runtime"));
            }
        }
        if let Some(groups) = project.get("optional-dependencies").and_then(|v| v.as_table()) {
            for (_, deps) in groups {
                for d in deps.as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
                    let (n, r) = split(d);
                    out.push(pkg(&n, &r, "dev"));
                }
            }
        }
    }
    // Poetry: [tool.poetry.dependencies] / [tool.poetry.group.*.dependencies]
    if let Some(poetry) = doc.get("tool").and_then(|v| v.get("poetry")).and_then(|v| v.as_table()) {
        if let Some(deps) = poetry.get("dependencies").and_then(|v| v.as_table()) {
            for (name, v) in deps {
                if name == "python" {
                    continue;
                }
                let req = v
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| v.get("version").and_then(|x| x.as_str()).map(str::to_string))
                    .unwrap_or_default();
                out.push(pkg(name, &req, "runtime"));
            }
        }
        if let Some(groups) = poetry.get("group").and_then(|v| v.as_table()) {
            for (_, g) in groups {
                if let Some(deps) = g.get("dependencies").and_then(|v| v.as_table()) {
                    for (name, v) in deps {
                        let req = v
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| v.get("version").and_then(|x| x.as_str()).map(str::to_string))
                            .unwrap_or_default();
                        out.push(pkg(name, &req, "dev"));
                    }
                }
            }
        }
        if let Some(deps) = poetry.get("dev-dependencies").and_then(|v| v.as_table()) {
            for (name, v) in deps {
                out.push(pkg(name, v.as_str().unwrap_or(""), "dev"));
            }
        }
    }
    out
}

/// Text between the first `<tag>` and `</tag>` in `s`.
fn xml_text<'a>(s: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let i = s.find(&open)? + open.len();
    let j = s[i..].find(&close)? + i;
    Some(s[i..j].trim())
}

fn parse_pom(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<dependency>") {
        let Some(j) = rest[i..].find("</dependency>") else { break };
        let block = &rest[i..i + j];
        if let (Some(g), Some(a)) = (xml_text(block, "groupId"), xml_text(block, "artifactId")) {
            let v = xml_text(block, "version").unwrap_or("");
            let scope = xml_text(block, "scope").unwrap_or("compile");
            out.push(pkg(&format!("{g}:{a}"), v, if scope == "test" || scope == "provided" { "dev" } else { "runtime" }));
        }
        rest = &rest[i + j + "</dependency>".len()..];
    }
    out
}

fn parse_gradle(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        let Some(cfg) = line.split([' ', '(']).next() else { continue };
        let kind = match cfg {
            "implementation" | "api" | "compile" | "runtimeOnly" | "compileOnly" | "kapt" | "annotationProcessor" => "runtime",
            "testImplementation" | "testCompile" | "testRuntimeOnly" | "androidTestImplementation" => "dev",
            _ => continue,
        };
        // implementation("group:name:version") or implementation 'group:name:version'
        let Some(q) = line.find(['"', '\'']) else { continue };
        let quote = line.as_bytes()[q] as char;
        let Some(e) = line[q + 1..].find(quote) else { continue };
        let coord = &line[q + 1..q + 1 + e];
        let mut parts = coord.split(':');
        if let (Some(g), Some(n)) = (parts.next(), parts.next()) {
            out.push(pkg(&format!("{g}:{n}"), parts.next().unwrap_or(""), kind));
        }
    }
    out
}

fn parse_gemfile(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut dev_depth = 0usize;
    let mut depth_stack: Vec<bool> = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with("group ") {
            let dev = line.contains(":test") || line.contains(":development");
            depth_stack.push(dev);
            if dev {
                dev_depth += 1;
            }
            continue;
        }
        if line == "end" {
            if let Some(dev) = depth_stack.pop()
                && dev
            {
                dev_depth -= 1;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("gem ") {
            let mut items = rest.split(',').map(|s| s.trim().trim_matches(|c| c == '"' || c == '\''));
            let Some(name) = items.next() else { continue };
            let req: Vec<&str> = items.filter(|s| !s.is_empty() && !s.contains(':') && !s.contains("=>")).collect();
            let dev = dev_depth > 0 || rest.contains(":test") || rest.contains(":development");
            out.push(pkg(name, &req.join(", "), if dev { "dev" } else { "runtime" }));
        }
    }
    out
}

fn parse_csproj(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<PackageReference") {
        let Some(j) = rest[i..].find('>') else { break };
        let tag = &rest[i..i + j];
        let attr = |name: &str| -> Option<String> {
            let key = format!("{name}=\"");
            let k = tag.find(&key)? + key.len();
            let e = tag[k..].find('"')? + k;
            Some(tag[k..e].to_string())
        };
        if let Some(name) = attr("Include") {
            out.push(pkg(&name, &attr("Version").unwrap_or_default(), "runtime"));
        }
        rest = &rest[i + j..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_and_npm() {
        let c = parse(
            "cargo",
            "Cargo.toml",
            "[dependencies]\nserde = \"1\"\ntokio = { version = \"1.2\", features = [\"full\"] }\n[dev-dependencies]\ninsta = \"1\"\n",
        );
        assert_eq!(c.len(), 3);
        assert_eq!(c[1].requirement, "1.2");
        assert_eq!(c[2].kind, "dev");
        let n = parse("npm", "package.json", r#"{"dependencies":{"react":"^19"},"devDependencies":{"vite":"^7"}}"#);
        assert_eq!(n.len(), 2);
        assert_eq!(n[1].kind, "dev");
    }

    #[test]
    fn go_pip_gradle() {
        let g = parse("go", "go.mod", "module x\n\nrequire (\n\tgithub.com/a/b v1.2.3\n\tgolang.org/x/net v0.1.0 // indirect\n)\n");
        assert_eq!(g.len(), 2);
        assert_eq!(g[1].kind, "dev");
        let p = parse("pip", "requirements.txt", "requests>=2.0 # http\nflask==3.0.0\n-r other.txt\n");
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].requirement, ">=2.0");
        let gr = parse("gradle", "build.gradle.kts", "implementation(\"org.a:b:1.0\")\ntestImplementation 'junit:junit:4.13'\n");
        assert_eq!(gr.len(), 2);
        assert_eq!(gr[0].name, "org.a:b");
    }
}
