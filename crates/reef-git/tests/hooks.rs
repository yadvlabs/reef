//! The pre-receive hook's linear-history check: merge commits are found on updated branches
//! (`old..new`) and on new branches (what they add over the default branch, or their whole
//! history when there is no default branch to compare with).

use reef_core::Id;
use reef_git::Store;
use reef_git::hooks::{ZERO_SHA, default_branch_ref, introduces_merge_commits};
use std::path::Path;
use std::process::Command;

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(work: &Path, name: &str) {
    std::fs::write(work.join(name), name).unwrap();
    sh(work, &["add", "."]);
    sh(work, &["commit", "-q", "-m", name]);
}

#[tokio::test]
async fn merge_commits_are_found_on_new_and_updated_branches() {
    let tmp = std::env::temp_dir().join(format!("reef-hooks-{}", reef_core::token::random_hex(6)));
    let store = Store::new(tmp.join("repos"), "git");
    let bare = store.init(Id(0x1000), "main").await.unwrap();
    let work = tmp.join("work");
    std::fs::create_dir_all(&work).unwrap();
    sh(&work, &["init", "-q", "-b", "main"]);
    commit(&work, "a");
    commit(&work, "b");
    let main = sh(&work, &["rev-parse", "HEAD"]);
    // feature: cut from main, one commit, then a merge of a topic branch.
    sh(&work, &["checkout", "-q", "-b", "feature"]);
    commit(&work, "c");
    sh(&work, &["checkout", "-q", "-b", "topic"]);
    commit(&work, "d");
    sh(&work, &["checkout", "-q", "feature"]);
    sh(&work, &["merge", "-q", "--no-ff", "-m", "merge topic", "topic"]);
    let feature = sh(&work, &["rev-parse", "HEAD"]);
    // linear: cut from main, one commit.
    sh(&work, &["checkout", "-q", "-b", "linear", &main]);
    commit(&work, "e");
    let linear = sh(&work, &["rev-parse", "HEAD"]);
    sh(&work, &["push", "-q", bare.to_str().unwrap(), "main", "feature", "linear"]);

    let git = Path::new("git");
    assert_eq!(default_branch_ref(git, &bare).await.unwrap().as_deref(), Some("refs/heads/main"));
    // New branches are judged on what they add over the default branch.
    assert!(!introduces_merge_commits(git, &bare, ZERO_SHA, &main).await.unwrap(), "the default branch itself is linear");
    assert!(introduces_merge_commits(git, &bare, ZERO_SHA, &feature).await.unwrap(), "a new branch carrying a merge is caught");
    assert!(!introduces_merge_commits(git, &bare, ZERO_SHA, &linear).await.unwrap(), "a new linear branch passes");
    // Updates are judged on old..new.
    assert!(introduces_merge_commits(git, &bare, &main, &feature).await.unwrap());
    assert!(!introduces_merge_commits(git, &bare, &main, &linear).await.unwrap());
    assert!(!introduces_merge_commits(git, &bare, &linear, &linear).await.unwrap(), "an empty range has no merges");

    // Without a default branch to compare with (HEAD unborn), the whole history is judged.
    let bare2 = store.init(Id(0x2000), "trunk").await.unwrap();
    sh(&work, &["push", "-q", bare2.to_str().unwrap(), "feature", "linear"]);
    assert_eq!(default_branch_ref(git, &bare2).await.unwrap().as_deref(), Some("refs/heads/trunk"));
    assert!(introduces_merge_commits(git, &bare2, ZERO_SHA, &feature).await.unwrap(), "whole history: the merge counts");
    assert!(!introduces_merge_commits(git, &bare2, ZERO_SHA, &linear).await.unwrap());
    assert!(introduces_merge_commits(git, &bare2, ZERO_SHA, "nonsense").await.is_err(), "git errors surface");
    std::fs::remove_dir_all(&tmp).ok();
}
