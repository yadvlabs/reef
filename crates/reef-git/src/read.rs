//! Read access to a repository with gix: refs, trees, blobs, and commit history.
//! All functions are synchronous; callers run them on a blocking thread.

use gix::bstr::ByteSlice;
use reef_core::{Error, Result};
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Milliseconds since the Unix epoch.
    pub time_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Commit {
    pub id: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
}

impl Commit {
    pub fn summary(&self) -> &str {
        self.message.lines().next().unwrap_or("")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Blob,
    Tree,
    Symlink,
    Submodule,
    Executable,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub id: String,
    pub kind: EntryKind,
    pub mode: u32,
}

#[derive(Debug, Clone)]
pub struct Blob {
    pub id: String,
    pub data: Vec<u8>,
    pub size: u64,
}

impl Blob {
    /// Git's own heuristic: a NUL in the first 8000 bytes means binary.
    pub fn is_binary(&self) -> bool {
        self.data.iter().take(8000).any(|b| *b == 0)
    }
}

/// A handle to one repository on disk.
pub struct RepoReader {
    repo: gix::Repository,
}

impl RepoReader {
    pub fn open(path: &Path) -> Result<RepoReader> {
        let repo = gix::open_opts(path, gix::open::Options::isolated()).map_err(|e| Error::msg(format!("open repository: {e}")))?;
        Ok(RepoReader { repo })
    }

    /// `true` when HEAD has no commit yet.
    pub fn is_empty(&self) -> bool {
        self.repo.head().map(|h| h.is_unborn()).unwrap_or(true)
    }

    /// The branch HEAD points at, if any.
    pub fn head_branch(&self) -> Option<String> {
        self.repo.head_name().ok().flatten().map(|n| n.shorten().to_string())
    }

    pub fn branches(&self) -> Result<Vec<(String, String)>> {
        self.refs_with_prefix("refs/heads/")
    }

    pub fn tags(&self) -> Result<Vec<(String, String)>> {
        self.refs_with_prefix("refs/tags/")
    }

    /// (short name, peeled commit id) pairs for refs under `prefix`, sorted by name.
    fn refs_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let platform = self.repo.references().map_err(|e| Error::msg(e.to_string()))?;
        let iter = platform.prefixed(prefix).map_err(|e| Error::msg(e.to_string()))?;
        let mut out = Vec::new();
        for r in iter {
            let mut r = r.map_err(|e| Error::msg(e.to_string()))?;
            let name = r.name().as_bstr().to_string();
            let id = r.peel_to_id().map_err(|e| Error::msg(e.to_string()))?;
            out.push((name.trim_start_matches(prefix).to_string(), id.to_string()));
        }
        out.sort();
        Ok(out)
    }

    /// Resolve a revision (branch, tag, or sha prefix) to a commit id.
    pub fn resolve(&self, rev: &str) -> Result<String> {
        if rev.is_empty() || rev.contains("..") || rev.starts_with('-') {
            return Err(Error::NotFound);
        }
        let id = self.repo.rev_parse_single(rev.as_bytes().as_bstr()).map_err(|_| Error::NotFound)?;
        let obj = id.object().map_err(|_| Error::NotFound)?;
        let commit = obj.peel_to_commit().map_err(|_| Error::NotFound)?;
        Ok(commit.id().to_string())
    }

    pub fn commit(&self, id: &str) -> Result<Commit> {
        let oid = gix::ObjectId::from_hex(id.as_bytes()).map_err(|_| Error::NotFound)?;
        let c = self.repo.find_commit(oid).map_err(|_| Error::NotFound)?;
        commit_from(&c)
    }

    /// Commits reachable from `rev`, newest first, skipping `skip` and returning at most `limit`.
    /// When `path` is given, only commits that touch it are returned.
    pub fn log(&self, rev: &str, path: Option<&str>, skip: usize, limit: usize) -> Result<Vec<Commit>> {
        let start = self.resolve(rev)?;
        let oid = gix::ObjectId::from_hex(start.as_bytes()).map_err(|_| Error::NotFound)?;
        let walk = self
            .repo
            .rev_walk([oid])
            .sorting(gix::revision::walk::Sorting::ByCommitTime(Default::default()))
            .all()
            .map_err(|e| Error::msg(e.to_string()))?;
        let mut out = Vec::new();
        let mut seen = 0usize;
        for info in walk {
            let info = info.map_err(|e| Error::msg(e.to_string()))?;
            let c = info.object().map_err(|e| Error::msg(e.to_string()))?;
            if let Some(p) = path
                && !self.commit_touches(&c, p)?
            {
                continue;
            }
            if seen < skip {
                seen += 1;
                continue;
            }
            out.push(commit_from(&c)?);
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Does this commit change `path` relative to its first parent?
    fn commit_touches(&self, c: &gix::Commit<'_>, path: &str) -> Result<bool> {
        let tree = c.tree().map_err(|e| Error::msg(e.to_string()))?;
        let here = tree.lookup_entry_by_path(path).map_err(|e| Error::msg(e.to_string()))?.map(|e| e.oid().to_owned());
        let parent = c.parent_ids().next();
        let there = match parent {
            Some(pid) => {
                let pc = self.repo.find_commit(pid).map_err(|e| Error::msg(e.to_string()))?;
                let pt = pc.tree().map_err(|e| Error::msg(e.to_string()))?;
                pt.lookup_entry_by_path(path).map_err(|e| Error::msg(e.to_string()))?.map(|e| e.oid().to_owned())
            }
            None => None,
        };
        Ok(here != there)
    }

    /// The entries of the tree at `path` (empty string for the root) as of `rev`.
    pub fn tree(&self, rev: &str, path: &str) -> Result<Vec<Entry>> {
        let cid = self.resolve(rev)?;
        let oid = gix::ObjectId::from_hex(cid.as_bytes()).map_err(|_| Error::NotFound)?;
        let commit = self.repo.find_commit(oid).map_err(|_| Error::NotFound)?;
        let root = commit.tree().map_err(|e| Error::msg(e.to_string()))?;
        let tree = if path.is_empty() {
            root
        } else {
            let entry = root.lookup_entry_by_path(path).map_err(|e| Error::msg(e.to_string()))?.ok_or(Error::NotFound)?;
            if !entry.mode().is_tree() {
                return Err(Error::NotFound);
            }
            self.repo.find_tree(entry.oid()).map_err(|_| Error::NotFound)?
        };
        let mut out = Vec::new();
        for e in tree.iter() {
            let e = e.map_err(|e| Error::msg(e.to_string()))?;
            let name = e.filename().to_string();
            let mode = e.mode();
            let kind = if mode.is_tree() {
                EntryKind::Tree
            } else if mode.is_link() {
                EntryKind::Symlink
            } else if mode.is_commit() {
                EntryKind::Submodule
            } else if mode.is_executable() {
                EntryKind::Executable
            } else {
                EntryKind::Blob
            };
            let full = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
            out.push(Entry { name, path: full, id: e.oid().to_string(), kind, mode: mode.value() as u32 });
        }
        // GitHub order: directories first, then files, each alphabetical.
        out.sort_by(|a, b| {
            let ad = a.kind == EntryKind::Tree;
            let bd = b.kind == EntryKind::Tree;
            bd.cmp(&ad).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    /// The entry at `path` as of `rev`, or `NotFound`.
    pub fn entry(&self, rev: &str, path: &str) -> Result<Entry> {
        let (dir, name) = match path.rsplit_once('/') {
            Some((d, n)) => (d, n),
            None => ("", path),
        };
        self.tree(rev, dir)?.into_iter().find(|e| e.name == name).ok_or(Error::NotFound)
    }

    pub fn blob_at(&self, rev: &str, path: &str) -> Result<Blob> {
        let cid = self.resolve(rev)?;
        let oid = gix::ObjectId::from_hex(cid.as_bytes()).map_err(|_| Error::NotFound)?;
        let commit = self.repo.find_commit(oid).map_err(|_| Error::NotFound)?;
        let root = commit.tree().map_err(|e| Error::msg(e.to_string()))?;
        let entry = root.lookup_entry_by_path(path).map_err(|e| Error::msg(e.to_string()))?.ok_or(Error::NotFound)?;
        if entry.mode().is_tree() {
            return Err(Error::NotFound);
        }
        self.blob(&entry.oid().to_string())
    }

    pub fn blob(&self, id: &str) -> Result<Blob> {
        let oid = gix::ObjectId::from_hex(id.as_bytes()).map_err(|_| Error::NotFound)?;
        let obj = self.repo.find_object(oid).map_err(|_| Error::NotFound)?;
        if obj.kind != gix::object::Kind::Blob {
            return Err(Error::NotFound);
        }
        let data = obj.data.clone();
        Ok(Blob { id: id.to_string(), size: data.len() as u64, data })
    }

    /// Find a README in the root tree, GitHub style (README.md, README, readme.txt, ...).
    pub fn readme(&self, rev: &str) -> Result<Option<(Entry, Blob)>> {
        let entries = self.tree(rev, "")?;
        let pick = entries.into_iter().find(|e| {
            let n = e.name.to_lowercase();
            e.kind == EntryKind::Blob
                && (n == "readme.md" || n == "readme" || n == "readme.txt" || n == "readme.markdown" || n == "readme.rst")
        });
        match pick {
            Some(e) => {
                let b = self.blob(&e.id)?;
                Ok(Some((e, b)))
            }
            None => Ok(None),
        }
    }

    /// Number of commits reachable from `rev`.
    /// The license file at the root, if any (LICENSE, LICENSE.md, LICENSE.txt, COPYING, UNLICENSE).
    pub fn license(&self, rev: &str) -> Result<Option<(Entry, Blob)>> {
        let entries = self.tree(rev, "")?;
        let pick = entries.into_iter().find(|e| {
            let n = e.name.to_lowercase();
            e.kind == EntryKind::Blob
                && (n == "license"
                    || n == "license.md"
                    || n == "license.txt"
                    || n == "licence"
                    || n == "licence.md"
                    || n == "copying"
                    || n == "copying.md"
                    || n == "unlicense"
                    || n == "license.rst")
        });
        match pick {
            Some(e) => {
                let b = self.blob(&e.id)?;
                Ok(Some((e, b)))
            }
            None => Ok(None),
        }
    }

    pub fn count_commits(&self, rev: &str) -> Result<usize> {
        let start = self.resolve(rev)?;
        let oid = gix::ObjectId::from_hex(start.as_bytes()).map_err(|_| Error::NotFound)?;
        let walk = self.repo.rev_walk([oid]).all().map_err(|e| Error::msg(e.to_string()))?;
        Ok(walk.count())
    }
}

fn commit_from(c: &gix::Commit<'_>) -> Result<Commit> {
    let decoded = c.decode().map_err(|e| Error::msg(e.to_string()))?;
    let sig = |s: gix::actor::SignatureRef<'_>| -> Signature {
        let t = s.time().ok();
        Signature { name: s.name.to_string(), email: s.email.to_string(), time_ms: t.map(|t| t.seconds * 1000).unwrap_or(0) }
    };
    Ok(Commit {
        id: c.id().to_string(),
        tree: decoded.tree().to_string(),
        parents: decoded.parents().map(|p| p.to_string()).collect(),
        author: sig(decoded.author().map_err(|e| Error::msg(e.to_string()))?),
        committer: sig(decoded.committer().map_err(|e| Error::msg(e.to_string()))?),
        message: decoded.message.to_str_lossy().into_owned(),
    })
}
