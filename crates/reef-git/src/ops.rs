//! Repository operations done with the git binary: ranges, diffs, merges. Used by pull
//! requests. Everything is async and works on bare repositories without a worktree.

use reef_core::{Error, Result};
use std::path::Path;
use tokio::process::Command;

async fn git(git: &Path, repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    let out = Command::new(git).arg("-C").arg(repo).args(args).env("LANG", "C.UTF-8").output().await?;
    Ok(out)
}

async fn git_ok(bin: &Path, repo: &Path, args: &[&str]) -> Result<String> {
    let out = git(bin, repo, args).await?;
    if !out.status.success() {
        return Err(Error::msg(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub async fn rev_parse(bin: &Path, repo: &Path, rev: &str) -> Result<String> {
    git_ok(bin, repo, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .await
        .map(|s| s.trim().to_string())
        .map_err(|_| Error::NotFound)
}

pub async fn merge_base(bin: &Path, repo: &Path, a: &str, b: &str) -> Result<Option<String>> {
    let out = git(bin, repo, &["merge-base", a, b]).await?;
    if out.status.success() { Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string())) } else { Ok(None) }
}

pub async fn is_ancestor(bin: &Path, repo: &Path, a: &str, b: &str) -> Result<bool> {
    Ok(git(bin, repo, &["merge-base", "--is-ancestor", a, b]).await?.status.success())
}

/// Commits in `base..head`, oldest first.
pub async fn rev_list(bin: &Path, repo: &Path, base: &str, head: &str) -> Result<Vec<String>> {
    let out = git_ok(bin, repo, &["rev-list", "--reverse", &format!("{base}..{head}")]).await?;
    Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileStat {
    pub path: String,
    pub previous_path: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    /// "added" | "removed" | "modified" | "renamed" | "copied"
    pub status: String,
    pub binary: bool,
}

/// `git diff --numstat -M base...head` plus `--name-status` for change types.
pub async fn diff_stat(bin: &Path, repo: &Path, base: &str, head: &str) -> Result<Vec<FileStat>> {
    let range = format!("{base}...{head}");
    let numstat = git_ok(bin, repo, &["diff", "--numstat", "-M", "-z", &range]).await?;
    let status = git_ok(bin, repo, &["diff", "--name-status", "-M", "-z", &range]).await?;
    let mut kinds: std::collections::HashMap<String, (String, Option<String>)> = std::collections::HashMap::new();
    let mut parts = status.split('\0').filter(|s| !s.is_empty());
    while let Some(st) = parts.next() {
        let code = st.chars().next().unwrap_or('M');
        match code {
            'R' | 'C' => {
                let from = parts.next().unwrap_or("").to_string();
                let to = parts.next().unwrap_or("").to_string();
                kinds.insert(to, (if code == 'R' { "renamed" } else { "copied" }.into(), Some(from)));
            }
            _ => {
                let p = parts.next().unwrap_or("").to_string();
                let k = match code {
                    'A' => "added",
                    'D' => "removed",
                    _ => "modified",
                };
                kinds.insert(p, (k.into(), None));
            }
        }
    }
    let mut out = Vec::new();
    let mut fields = numstat.split('\0').filter(|s| !s.is_empty()).peekable();
    while let Some(line) = fields.next() {
        // "adds\tdels\tpath" or for renames "adds\tdels\t" then old, new as separate NUL fields
        let mut cols = line.splitn(3, '\t');
        let adds = cols.next().unwrap_or("-");
        let dels = cols.next().unwrap_or("-");
        let mut path = cols.next().unwrap_or("").to_string();
        let mut previous = None;
        if path.is_empty() {
            previous = fields.next().map(String::from);
            path = fields.next().unwrap_or("").to_string();
        }
        let binary = adds == "-";
        let (status, prev) = kinds.get(&path).cloned().unwrap_or(("modified".into(), previous.clone()));
        out.push(FileStat {
            path,
            previous_path: prev.or(previous),
            additions: adds.parse().unwrap_or(0),
            deletions: dels.parse().unwrap_or(0),
            status,
            binary,
        });
    }
    Ok(out)
}

/// Unified diff of `base...head` (or a single commit when `base` is None).
pub async fn diff_text(bin: &Path, repo: &Path, base: Option<&str>, head: &str, path: Option<&str>) -> Result<String> {
    let range = match base {
        Some(b) => format!("{b}...{head}"),
        None => format!("{head}^!"),
    };
    let mut args = vec!["diff", "--no-color", "-M", "--no-ext-diff", &range];
    if let Some(p) = path {
        args.push("--");
        args.push(p);
    }
    git_ok(bin, repo, &args).await
}

/// `git format-patch`-style patch text for a range.
pub async fn patch_text(bin: &Path, repo: &Path, base: &str, head: &str) -> Result<String> {
    git_ok(bin, repo, &["format-patch", "--stdout", &format!("{base}..{head}")]).await
}

/// Result of a merge-tree computation.
#[derive(Debug, Clone)]
pub struct MergeResult {
    pub tree: Option<String>,
    pub conflicts: Vec<String>,
}

/// Compute the merged tree of `base_ref` and `head` without touching a worktree
/// (`git merge-tree --write-tree`, git 2.38+).
pub async fn merge_tree(bin: &Path, repo: &Path, base: &str, head: &str) -> Result<MergeResult> {
    let out = git(bin, repo, &["merge-tree", "--write-tree", "--name-only", "--no-messages", base, head]).await?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = stdout.lines();
    let tree = lines.next().map(|l| l.trim().to_string()).filter(|l| l.len() == 40);
    match out.status.code() {
        Some(0) => Ok(MergeResult { tree, conflicts: vec![] }),
        Some(1) => Ok(MergeResult { tree: None, conflicts: lines.map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect() }),
        _ => Err(Error::msg(format!("git merge-tree failed: {}", String::from_utf8_lossy(&out.stderr).trim()))),
    }
}

#[derive(Clone, Copy)]
pub struct Identity<'a> {
    pub name: &'a str,
    pub email: &'a str,
}

/// Create a commit object for `tree` with the given parents. Returns the commit id.
pub async fn commit_tree(
    bin: &Path,
    repo: &Path,
    tree: &str,
    parents: &[&str],
    message: &str,
    author: Identity<'_>,
    committer: Identity<'_>,
) -> Result<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("-C").arg(repo).arg("commit-tree").arg(tree);
    for p in parents {
        cmd.arg("-p").arg(p);
    }
    cmd.arg("-m").arg(message);
    cmd.env("GIT_AUTHOR_NAME", author.name)
        .env("GIT_AUTHOR_EMAIL", author.email)
        .env("GIT_COMMITTER_NAME", committer.name)
        .env("GIT_COMMITTER_EMAIL", committer.email);
    let out = cmd.output().await?;
    if !out.status.success() {
        return Err(Error::msg(format!("git commit-tree: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Atomically move a ref from `old` (or unborn) to `new`.
pub async fn update_ref(bin: &Path, repo: &Path, name: &str, new: &str, old: Option<&str>, reason: &str) -> Result<()> {
    let mut args = vec!["update-ref", "-m", reason, name, new];
    if let Some(o) = old {
        args.push(o);
    }
    git_ok(bin, repo, &args).await.map(|_| ())
}

pub async fn delete_ref(bin: &Path, repo: &Path, name: &str) -> Result<()> {
    git_ok(bin, repo, &["update-ref", "-d", name]).await.map(|_| ())
}

/// The tree id of a commit.
pub async fn tree_of(bin: &Path, repo: &Path, commit: &str) -> Result<String> {
    git_ok(bin, repo, &["rev-parse", &format!("{commit}^{{tree}}")]).await.map(|s| s.trim().to_string())
}

/// Squash: one commit with `head`'s merged tree on top of `base`.
/// Rebase: replay each commit of `base..head` onto `base` with merge-tree.
/// Returns the new tip.
pub async fn rebase_onto(bin: &Path, repo: &Path, base: &str, head: &str, committer: Identity<'_>) -> Result<String> {
    let mut tip = base.to_string();
    for c in rev_list(bin, repo, base, head).await? {
        let parent = git_ok(bin, repo, &["rev-parse", &format!("{c}^")]).await?.trim().to_string();
        // Three-way merge of the commit's change onto the current tip.
        let out = git(bin, repo, &["merge-tree", "--write-tree", "--merge-base", &parent, &tip, &c]).await?;
        if !out.status.success() {
            return Err(Error::Conflict("rebase produced conflicts".into()));
        }
        let tree = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
        let raw = git_ok(bin, repo, &["log", "-1", "--format=%an%x00%ae%x00%B", &c]).await?;
        let mut parts = raw.splitn(3, '\0');
        let an = parts.next().unwrap_or("").to_string();
        let ae = parts.next().unwrap_or("").to_string();
        let msg = parts.next().unwrap_or("").trim_end().to_string();
        tip = commit_tree(
            bin,
            repo,
            &tree,
            &[&tip],
            &msg,
            Identity { name: &an, email: &ae },
            Identity { name: committer.name, email: committer.email },
        )
        .await?;
    }
    Ok(tip)
}

/// Read a blob at `rev:path`, for diff views that need the whole file.
pub async fn show_blob(bin: &Path, repo: &Path, rev: &str, path: &str) -> Result<Vec<u8>> {
    let out = git(bin, repo, &["show", &format!("{rev}:{path}")]).await?;
    if !out.status.success() {
        return Err(Error::NotFound);
    }
    Ok(out.stdout)
}

/// One file's section of a unified diff.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FilePatch {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    /// The hunks (everything after the `+++` line); empty for binary files.
    pub patch: String,
    pub binary: bool,
}

/// Split `git diff` output into per-file patches.
pub fn split_diff(text: &str) -> Vec<FilePatch> {
    let mut out: Vec<FilePatch> = Vec::new();
    let mut cur: Option<FilePatch> = None;
    let mut in_hunks = false;
    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            if let Some(c) = cur.take() {
                out.push(c);
            }
            cur = Some(FilePatch { old_path: None, new_path: None, patch: String::new(), binary: false });
            in_hunks = false;
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        if !in_hunks {
            if let Some(p) = line.strip_prefix("--- ") {
                let p = p.trim_end();
                c.old_path = if p == "/dev/null" { None } else { Some(strip_prefix(p)) };
            } else if let Some(p) = line.strip_prefix("+++ ") {
                let p = p.trim_end();
                c.new_path = if p == "/dev/null" { None } else { Some(strip_prefix(p)) };
                in_hunks = true;
            } else if line.starts_with("Binary files") || line.starts_with("GIT binary patch") {
                c.binary = true;
            } else if let Some(p) = line.strip_prefix("rename from ") {
                c.old_path = Some(p.trim_end().to_string());
            } else if let Some(p) = line.strip_prefix("rename to ") {
                c.new_path = Some(p.trim_end().to_string());
            }
        } else {
            c.patch.push_str(line);
        }
    }
    if let Some(c) = cur.take() {
        out.push(c);
    }
    out
}

fn strip_prefix(p: &str) -> String {
    let p = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(p);
    // Quoted paths with special characters.
    p.trim_matches('"').to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn splits_diffs() {
        let d = "diff --git a/x.txt b/x.txt\nindex 1..2 100644\n--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/new b/new\nnew file mode 100644\n--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+hi\n";
        let f = super::split_diff(d);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].new_path.as_deref(), Some("x.txt"));
        assert!(f[0].patch.starts_with("@@ -1 +1 @@"));
        assert_eq!(f[1].old_path, None);
    }
}

// ----- remotes: mirrors and imports -----

/// Credentials for a remote, passed to git through the environment (not the command line).
#[derive(Debug, Clone, Default)]
pub struct RemoteAuth {
    pub username: Option<String>,
    pub password: Option<String>,
}

impl RemoteAuth {
    fn apply(&self, cmd: &mut Command) {
        cmd.env("GIT_TERMINAL_PROMPT", "0").env("GIT_ASKPASS", "").env("SSH_ASKPASS", "");
        if self.username.is_some() || self.password.is_some() {
            let raw = format!("{}:{}", self.username.as_deref().unwrap_or(""), self.password.as_deref().unwrap_or(""));
            let header = format!("Authorization: Basic {}", b64(raw.as_bytes()));
            cmd.env("GIT_CONFIG_COUNT", "1").env("GIT_CONFIG_KEY_0", "http.extraHeader").env("GIT_CONFIG_VALUE_0", header);
        }
    }
}

/// Standard base64 (no dependency needed for one header).
fn b64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

async fn remote_cmd(
    bin: &Path,
    repo: Option<&Path>,
    args: &[&str],
    auth: &RemoteAuth,
    timeout: std::time::Duration,
) -> Result<std::process::Output> {
    let mut cmd = Command::new(bin);
    if let Some(r) = repo {
        cmd.arg("-C").arg(r);
    }
    cmd.args(args).env("LANG", "C.UTF-8").kill_on_drop(true);
    auth.apply(&mut cmd);
    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(out) => Ok(out?),
        Err(_) => Err(Error::msg(format!("git {} timed out after {}s", args.first().unwrap_or(&""), timeout.as_secs()))),
    }
}

fn remote_err(what: &str, out: &std::process::Output) -> Error {
    let msg = String::from_utf8_lossy(&out.stderr);
    // Keep the last meaningful line: git prints progress and the reason last.
    let line = msg.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("unknown error");
    Error::msg(format!("{what}: {line}"))
}

/// `refs/heads/*` and `refs/tags/*` of a bare repository as `(name, sha)`.
pub async fn all_refs(bin: &Path, repo: &Path) -> Result<Vec<(String, String)>> {
    let out = git_ok(bin, repo, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads/", "refs/tags/"]).await?;
    Ok(out.lines().filter_map(|l| l.split_once(' ').map(|(n, s)| (n.to_string(), s.to_string()))).collect())
}

/// Fetch every branch and tag from `url` into a bare repository, pruning what disappeared.
pub async fn fetch_mirror(bin: &Path, repo: &Path, url: &str, auth: &RemoteAuth, timeout: std::time::Duration) -> Result<()> {
    let out = remote_cmd(
        bin,
        Some(repo),
        &["fetch", "--prune", "--force", "--no-write-fetch-head", "--quiet", url, "+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"],
        auth,
        timeout,
    )
    .await?;
    if !out.status.success() {
        return Err(remote_err("fetch", &out));
    }
    Ok(())
}

/// Push every branch and tag to `url`, deleting what no longer exists here.
pub async fn push_mirror(bin: &Path, repo: &Path, url: &str, auth: &RemoteAuth, timeout: std::time::Duration) -> Result<()> {
    let out = remote_cmd(bin, Some(repo), &["push", "--mirror", "--quiet", url], auth, timeout).await?;
    if !out.status.success() {
        return Err(remote_err("push", &out));
    }
    Ok(())
}

/// The branch the remote's HEAD points at, if it advertises one.
pub async fn remote_head(bin: &Path, url: &str, auth: &RemoteAuth, timeout: std::time::Duration) -> Result<Option<String>> {
    let out = remote_cmd(bin, None, &["ls-remote", "--symref", url, "HEAD"], auth, timeout).await?;
    if !out.status.success() {
        return Err(remote_err("ls-remote", &out));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ref: ")
            && let Some((target, _)) = rest.split_once('\t')
        {
            return Ok(target.strip_prefix("refs/heads/").map(String::from));
        }
    }
    Ok(None)
}

/// Number of commits reachable from any ref.
pub async fn count_all_commits(bin: &Path, repo: &Path) -> Result<i64> {
    let out = git_ok(bin, repo, &["rev-list", "--all", "--count"]).await?;
    Ok(out.trim().parse().unwrap_or(0))
}
