use reef_core::{Error, Id, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// The on-disk home of every repository. Layout: `<root>/<aa>/<bb>/<id>.git` where `aa`
/// and `bb` are the first two byte pairs of the id in hex.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    git: PathBuf,
    hooks_dir: Option<PathBuf>,
    tmp_dir: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>, git_binary: impl Into<PathBuf>) -> Store {
        let root: PathBuf = root.into();
        // Absolute, so spawned git processes with a different working directory find it.
        let root = if root.is_absolute() { root } else { std::env::current_dir().map(|c| c.join(&root)).unwrap_or(root) };
        Store { root, git: git_binary.into(), hooks_dir: None, tmp_dir: std::env::temp_dir() }
    }

    /// Scratch directory for temporary indexes, signature checks and initial commits. The
    /// server points it under its state directory (never `/tmp`).
    pub fn with_tmp_dir(mut self, dir: PathBuf) -> Store {
        self.tmp_dir = dir;
        self
    }

    pub fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    /// Enable server-side hooks: spawned git processes get `core.hooksPath=<dir>`.
    pub fn with_hooks(mut self, dir: PathBuf) -> Store {
        self.hooks_dir = Some(dir);
        self
    }

    pub fn hooks_dir(&self) -> Option<&Path> {
        self.hooks_dir.as_deref()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn git_binary(&self) -> &Path {
        &self.git
    }

    pub fn path(&self, id: Id) -> PathBuf {
        let hex = format!("{:016x}", id.0 as u64);
        self.root.join(&hex[..2]).join(&hex[2..4]).join(format!("{hex}.git"))
    }

    /// The wiki of a repository: a sibling bare repository, `<id>.wiki.git`.
    pub fn wiki_path(&self, id: Id) -> PathBuf {
        let hex = format!("{:016x}", id.0 as u64);
        self.root.join(&hex[..2]).join(&hex[2..4]).join(format!("{hex}.wiki.git"))
    }

    /// Create the wiki repository if it does not exist yet.
    pub async fn ensure_wiki(&self, id: Id, default_branch: &str) -> Result<PathBuf> {
        let path = self.wiki_path(id);
        if !path.join("HEAD").is_file() {
            tokio::fs::create_dir_all(path.parent().unwrap()).await?;
            let out = Command::new(&self.git)
                .args(["init", "--bare", "-q", "--initial-branch", default_branch])
                .arg(&path)
                .stdin(Stdio::null())
                .output()
                .await?;
            if !out.status.success() {
                return Err(Error::msg(format!("git init failed: {}", String::from_utf8_lossy(&out.stderr))));
            }
        }
        Ok(path)
    }

    pub fn exists(&self, id: Id) -> bool {
        self.path(id).join("HEAD").is_file()
    }

    /// Create an empty bare repository with `HEAD` pointing at `default_branch`.
    pub async fn init(&self, id: Id, default_branch: &str) -> Result<PathBuf> {
        let path = self.path(id);
        if path.exists() {
            return Err(Error::Conflict("repository storage already exists".into()));
        }
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let out = Command::new(&self.git)
            .args(["init", "--bare", "-q", "--initial-branch", default_branch])
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .await?;
        if !out.status.success() {
            return Err(Error::msg(format!("git init failed: {}", String::from_utf8_lossy(&out.stderr))));
        }
        // Server-side defaults: no auto-gc mid-push, keep packed refs tidy, protocol v2.
        for (k, v) in [
            ("receive.denyNonFastForwards", "false"),
            ("gc.auto", "0"),
            ("uploadpack.allowFilter", "true"),
            ("core.logAllRefUpdates", "true"),
        ] {
            self.config(&path, k, v).await?;
        }
        Ok(path)
    }

    pub async fn config(&self, path: &Path, key: &str, value: &str) -> Result<()> {
        let out = Command::new(&self.git).arg("-C").arg(path).args(["config", key, value]).output().await?;
        if !out.status.success() {
            return Err(Error::msg(format!("git config failed: {}", String::from_utf8_lossy(&out.stderr))));
        }
        Ok(())
    }

    /// Point HEAD at a branch (used when the first push creates a branch other than the default).
    pub async fn set_head(&self, path: &Path, branch: &str) -> Result<()> {
        let out =
            Command::new(&self.git).arg("-C").arg(path).args(["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")]).output().await?;
        if !out.status.success() {
            return Err(Error::msg(format!("git symbolic-ref failed: {}", String::from_utf8_lossy(&out.stderr))));
        }
        Ok(())
    }

    /// Remove a repository from disk. The directory is renamed first so a crash mid-delete
    /// never leaves a half-repository at the live path.
    pub async fn delete(&self, id: Id) -> Result<()> {
        let path = self.path(id);
        if !path.exists() {
            return Ok(());
        }
        let tomb = path.with_extension(format!("deleted-{}", reef_core::time::now_ms()));
        tokio::fs::rename(&path, &tomb).await?;
        tokio::fs::remove_dir_all(&tomb).await?;
        let wiki = self.wiki_path(id);
        if wiki.exists() {
            let _ = tokio::fs::remove_dir_all(&wiki).await;
        }
        Ok(())
    }

    /// Approximate size on disk in kilobytes.
    pub async fn size_kb(&self, id: Id) -> Result<i64> {
        let path = self.path(id);
        tokio::task::spawn_blocking(move || dir_size(&path, false).map(|b| (b / 1024) as i64)).await.map_err(Error::internal)?
    }

    /// Run `git gc` in the background-friendly quiet mode.
    pub async fn gc(&self, id: Id) -> Result<()> {
        let out = Command::new(&self.git).arg("-C").arg(self.path(id)).args(["gc", "-q"]).output().await?;
        if !out.status.success() {
            return Err(Error::msg(format!("git gc failed: {}", String::from_utf8_lossy(&out.stderr))));
        }
        Ok(())
    }
}

/// Bytes of everything under `path`, recursively. Strict (`lenient = false`): the first entry
/// that cannot be read is an error, so a repository's size is exact or absent. Lenient: a
/// missing root is 0 and entries that vanish or cannot be read during the walk (a tree other
/// instances are changing, such as the Actions logs the metrics walk covers) are skipped.
pub fn dir_size(path: &Path, lenient: bool) -> Result<u64> {
    let rd = match std::fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) if lenient => return Ok(0),
        Err(e) => return Err(e.into()),
    };
    let mut total = 0;
    for entry in rd {
        let (entry, meta) = match entry.and_then(|e| e.metadata().map(|m| (e, m))) {
            Ok(x) => x,
            Err(_) if lenient => continue,
            Err(e) => return Err(e.into()),
        };
        if meta.is_dir() {
            total += dir_size(&entry.path(), lenient)?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod dir_size_tests {
    use super::dir_size;

    #[test]
    fn sums_recursively_and_is_lenient_on_request() {
        let dir = std::env::temp_dir().join(format!("reef-dirsize-{}", reef_core::token::random_hex(6)));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/one.log"), b"12345").unwrap();
        std::fs::write(dir.join("a/b/two.log"), b"123").unwrap();
        assert_eq!(dir_size(&dir, false).unwrap(), 8);
        assert_eq!(dir_size(&dir, true).unwrap(), 8);
        assert!(dir_size(&dir.join("nope"), false).is_err(), "strict: a missing tree is an error");
        assert_eq!(dir_size(&dir.join("nope"), true).unwrap(), 0, "lenient: a missing tree is empty");
        std::fs::remove_dir_all(&dir).ok();
    }
}
