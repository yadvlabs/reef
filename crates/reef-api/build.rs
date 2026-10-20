//! Stamp the version string: `v0.1.0 (abc123def456)`, `-dirty` appended to the commit when the
//! tree had changes; `reef version` prints it as `reef <version> (<commit>)`.
//!
//! - `REEF_VERSION` (`make build VERSION=v…`) sets the version. It must be Cargo's version or a
//!   pre-release of it (`v0.1.0`, `v0.1.0-rc.1` for Cargo `0.1.0`); anything else fails the build.
//! - `REEF_COMMIT` sets the commit (`<sha>` or `<sha>-dirty`). The Makefile computes it on every
//!   `make` run and passes it to cargo, so the `-dirty` decision follows the working tree: a
//!   tree going clean or dirty changes the variable, and the stamp is redone
//!   (`rerun-if-env-changed`). Container images pass it as a build argument.
//! - Otherwise (plain `cargo build`) the commit comes from git, and the stamp is redone whenever
//!   `HEAD`, the branch it points to, `packed-refs` or the index change (the paths come from
//!   `git rev-parse --git-path`, so worktrees work; paths that do not exist are not watched),
//!   or anything under the tracked source directories does (`crates/*/src`, `bin/*/src`,
//!   `crates/*/migrations`, the manifests and `web/dist`), since editing a tracked file makes
//!   the tree dirty without touching the index.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    // No optional locks: `git status` must not refresh (write) the index of the tree it reports on.
    let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_OPTIONAL_LOCKS", "0").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn main() {
    println!("cargo:rerun-if-env-changed=REEF_COMMIT");
    println!("cargo:rerun-if-env-changed=REEF_VERSION");
    let cargo_version = env!("CARGO_PKG_VERSION");
    let version = match std::env::var("REEF_VERSION").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
        Some(v) => {
            let bare = v.strip_prefix('v').unwrap_or(&v);
            // A value already stamped ("v0.1.0 (abc)") is not a request: ignore it.
            if bare.contains(' ') {
                format!("v{cargo_version}")
            } else if bare == cargo_version || bare.strip_prefix(cargo_version).is_some_and(|rest| rest.starts_with('-')) {
                format!("v{bare}")
            } else {
                panic!("REEF_VERSION={v} does not match the Cargo version {cargo_version} (nor a pre-release of it)");
            }
        }
        None => format!("v{cargo_version}"),
    };
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));
    // `rust-embed` refuses to compile when its folder is missing, so a fresh clone builds before
    // `make web` has run (the server then shows a placeholder page instead of the UI).
    let _ = std::fs::create_dir_all(manifest.join("../../web/dist"));
    let commit = match std::env::var("REEF_COMMIT").ok().filter(|s| !s.trim().is_empty()) {
        Some(c) => c.trim().to_string(),
        None => {
            for name in ["HEAD", "packed-refs", "index"] {
                watch(&manifest, name);
            }
            if let Some(branch) = git(&manifest, &["symbolic-ref", "-q", "HEAD"]) {
                watch(&manifest, &branch);
            }
            watch_sources(&manifest);
            match git(&manifest, &["rev-parse", "--short=12", "HEAD"]) {
                Some(c) => {
                    let dirty = git(&manifest, &["status", "--porcelain", "--untracked-files=no"]).is_some();
                    if dirty { format!("{c}-dirty") } else { c }
                }
                None => "unknown".to_string(),
            }
        }
    };
    println!("cargo:rustc-env=REEF_VERSION={version} ({commit})");
    println!("cargo:rustc-env=REEF_BUILD_VERSION={version}");
    println!("cargo:rustc-env=REEF_BUILD_COMMIT={commit}");
}

/// `rerun-if-changed` on a file inside the git directory, when it exists.
fn watch(manifest: &Path, name: &str) {
    if let Some(p) = git(manifest, &["rev-parse", "--path-format=absolute", "--git-path", name]) {
        let p = PathBuf::from(p);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
}

/// `rerun-if-changed` on the workspace's tracked sources (cargo scans a directory recursively),
/// so an edit that makes the tree dirty re-evaluates `-dirty`.
fn watch_sources(manifest: &Path) {
    let Some(root) = git(manifest, &["rev-parse", "--show-toplevel"]).map(PathBuf::from) else { return };
    let mut paths = vec![root.join("Cargo.toml"), root.join("Cargo.lock"), root.join("web/dist")];
    for group in ["crates", "bin"] {
        let Ok(entries) = std::fs::read_dir(root.join(group)) else { continue };
        for e in entries.flatten() {
            for sub in ["src", "migrations", "Cargo.toml", "build.rs"] {
                paths.push(e.path().join(sub));
            }
        }
    }
    for p in paths.into_iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}
