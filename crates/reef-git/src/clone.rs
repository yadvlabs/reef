//! Forking: copy a bare repository into a new bare repository (objects, refs, HEAD).

use reef_core::Error;
use reef_core::Result;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// `git clone --bare` from one local repository into another. The origin remote is removed
/// so the fork stands on its own.
pub async fn fork_bare(git: &Path, src: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(Error::Conflict("repository storage already exists".into()));
    }
    tokio::fs::create_dir_all(dst.parent().unwrap()).await?;
    let out = Command::new(git).args(["clone", "--bare", "-q"]).arg(src).arg(dst).stdin(Stdio::null()).output().await?;
    if !out.status.success() {
        return Err(Error::msg(format!("git clone failed: {}", String::from_utf8_lossy(&out.stderr))));
    }
    let _ = Command::new(git).arg("-C").arg(dst).args(["remote", "remove", "origin"]).stdin(Stdio::null()).output().await;
    Ok(())
}
