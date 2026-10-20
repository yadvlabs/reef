//! Commits made without a work tree (web editor, wikis): add, then delete.

use reef_core::Id;
use reef_git::ops::Identity;
use reef_git::write::{CommitRequest, FileChange, commit_changes};
use reef_git::{RepoReader, Store};

fn names(bare: &std::path::Path) -> Vec<String> {
    RepoReader::open(bare).unwrap().tree("HEAD", "").unwrap().into_iter().map(|e| e.name).collect()
}

#[tokio::test]
async fn add_then_delete_in_a_bare_repository() {
    let tmp = std::env::temp_dir().join(format!("reef-write-{}", reef_core::token::random_hex(6)));
    let store = Store::new(tmp.join("repos"), "git");
    let bare = store.init(Id(0x0abc), "main").await.unwrap();
    let git = std::path::Path::new("git");
    let who = || Identity { name: "Test", email: "t@example.com" };
    let change = |path: &str, content: Option<&str>| FileChange {
        path: path.into(),
        content: content.map(|c| c.as_bytes().to_vec()),
        executable: false,
    };
    let first = commit_changes(
        git,
        &bare,
        &tmp,
        CommitRequest {
            branch: "main".into(),
            expected_head: None,
            message: "add".into(),
            author: who(),
            committer: who(),
            changes: vec![change("Home.md", Some("# Home\n")), change("Other.md", Some("# Other\n"))],
        },
    )
    .await
    .unwrap();
    assert_eq!(names(&bare), ["Home.md", "Other.md"]);
    commit_changes(
        git,
        &bare,
        &tmp,
        CommitRequest {
            branch: "main".into(),
            expected_head: Some(first),
            message: "delete".into(),
            author: who(),
            committer: who(),
            changes: vec![change("Other.md", None)],
        },
    )
    .await
    .unwrap();
    assert_eq!(names(&bare), ["Home.md"]);
    let _ = std::fs::remove_dir_all(&tmp);
}
