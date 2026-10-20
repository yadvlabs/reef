//! Writing to a bare repository without a worktree: one commit that adds, updates, or
//! removes files on top of a branch tip. Used by the web editor and wikis. Everything goes
//! through git plumbing with a temporary index, so it is safe on shared storage.

use crate::ops::{Identity, commit_tree, update_ref};
use reef_core::{Error, Result};
use std::path::Path;
use tokio::process::Command;

pub struct FileChange {
    pub path: String,
    /// `None` deletes the file.
    pub content: Option<Vec<u8>>,
    pub executable: bool,
}

pub struct CommitRequest<'a> {
    pub branch: String,
    /// Expected current tip (`None` for an unborn branch). The commit fails if it moved.
    pub expected_head: Option<String>,
    pub message: String,
    pub author: Identity<'a>,
    pub committer: Identity<'a>,
    pub changes: Vec<FileChange>,
}

/// Apply `changes` as a new commit on `branch`. Returns the new commit id.
pub async fn commit_changes(git: &Path, repo: &Path, tmp_dir: &Path, req: CommitRequest<'_>) -> Result<String> {
    let full_ref = format!("refs/heads/{}", req.branch);
    tokio::fs::create_dir_all(tmp_dir).await?;
    let index = tmp_dir.join(format!("reef-index-{}-{}", std::process::id(), reef_core::token::random_hex(6)));
    let run = |args: Vec<String>, stdin: Option<Vec<u8>>| {
        let index = index.clone();
        let git = git.to_path_buf();
        let repo = repo.to_path_buf();
        async move {
            let mut cmd = Command::new(&git);
            cmd.arg("-C").arg(&repo).args(&args).env("GIT_INDEX_FILE", &index);
            cmd.stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
            cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn()?;
            if let Some(data) = stdin {
                use tokio::io::AsyncWriteExt;
                let mut w = child.stdin.take().unwrap();
                w.write_all(&data).await?;
                drop(w);
            }
            let out = child.wait_with_output().await?;
            if !out.status.success() {
                return Err(Error::msg(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
            }
            Ok::<String, Error>(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
    };
    let head = match run(vec!["rev-parse".into(), "--verify".into(), "-q".into(), full_ref.clone()], None).await {
        Ok(h) if !h.is_empty() => Some(h),
        _ => None,
    };
    if head != req.expected_head {
        let _ = std::fs::remove_file(&index);
        return Err(Error::Conflict("the branch moved since you started editing".into()));
    }
    // Start from the tip's tree (or empty).
    if let Some(h) = &head {
        run(vec!["read-tree".into(), h.clone()], None).await?;
    } else {
        run(vec!["read-tree".into(), "--empty".into()], None).await?;
    }
    for ch in &req.changes {
        if ch.path.is_empty() || ch.path.split('/').any(|seg| seg == ".." || seg.is_empty() || seg == ".git") {
            let _ = std::fs::remove_file(&index);
            return Err(Error::validation("invalid path"));
        }
        match &ch.content {
            Some(data) => {
                let oid = run(vec!["hash-object".into(), "-w".into(), "--stdin".into()], Some(data.clone())).await?;
                let mode = if ch.executable { "100755" } else { "100644" };
                run(vec!["update-index".into(), "--add".into(), "--cacheinfo".into(), format!("{mode},{oid},{}", ch.path)], None).await?;
            }
            None => {
                // `--force-remove` wants a work tree; an index-info line with mode 0 removes
                // the entry from the temporary index without one.
                let line = format!("0 {}\t{}\n", "0".repeat(40), ch.path);
                run(vec!["update-index".into(), "--index-info".into()], Some(line.into_bytes())).await?;
            }
        }
    }
    let tree = run(vec!["write-tree".into()], None).await?;
    let _ = std::fs::remove_file(&index);
    let parents: Vec<&str> = head.iter().map(|s| s.as_str()).collect();
    let commit = commit_tree(git, repo, &tree, &parents, &req.message, req.author, req.committer).await?;
    update_ref(git, repo, &full_ref, &commit, head.as_deref(), &req.message).await?;
    Ok(commit)
}
