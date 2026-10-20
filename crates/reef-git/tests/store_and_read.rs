use reef_core::Id;
use reef_git::{EntryKind, RepoReader, Store};
use std::process::Command;

fn sh(dir: &std::path::Path, args: &[&str]) {
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
}

#[tokio::test]
async fn init_push_and_read() {
    let tmp = std::env::temp_dir().join(format!("reef-git-{}", reef_core::token::random_hex(6)));
    let store = Store::new(tmp.join("repos"), "git");
    let id = Id(0x1234_5678_9abc_def0);
    let bare = store.init(id, "main").await.unwrap();
    assert!(store.exists(id));
    assert!(bare.ends_with("12/34/123456789abcdef0.git"));

    let r = RepoReader::open(&bare).unwrap();
    assert!(r.is_empty());
    assert_eq!(r.head_branch().as_deref(), Some("main"));

    // Make a working repo and push it in.
    let work = tmp.join("work");
    std::fs::create_dir_all(work.join("src")).unwrap();
    sh(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("README.md"), "# Hello\n\nreef test\n").unwrap();
    std::fs::write(work.join("src/main.rs"), "fn main() {}\n").unwrap();
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "first commit"]);
    std::fs::write(work.join("src/main.rs"), "fn main() { println!(\"hi\"); }\n").unwrap();
    sh(&work, &["commit", "-q", "-am", "second commit\n\nwith body"]);
    sh(&work, &["tag", "v0.1.0"]);
    sh(&work, &["push", "-q", bare.to_str().unwrap(), "main", "v0.1.0"]);

    let r = RepoReader::open(&bare).unwrap();
    assert!(!r.is_empty());
    let branches = r.branches().unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].0, "main");
    assert_eq!(r.tags().unwrap()[0].0, "v0.1.0");

    let head = r.resolve("main").unwrap();
    assert_eq!(head, branches[0].1);
    assert_eq!(r.resolve("v0.1.0").unwrap(), head);
    assert_eq!(r.resolve(&head[..7]).unwrap(), head);
    assert!(r.resolve("nope").is_err());

    let root = r.tree("main", "").unwrap();
    assert_eq!(root.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["src", "README.md"]);
    assert_eq!(root[0].kind, EntryKind::Tree);
    let src = r.tree("main", "src").unwrap();
    assert_eq!(src[0].path, "src/main.rs");
    let blob = r.blob_at("main", "src/main.rs").unwrap();
    assert!(String::from_utf8_lossy(&blob.data).contains("println"));
    assert!(!blob.is_binary());
    assert!(r.blob_at("main", "src").is_err());

    let log = r.log("main", None, 0, 10).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].summary(), "second commit");
    assert_eq!(log[0].message.trim(), "second commit\n\nwith body");
    assert_eq!(log[0].author.name, "Test");
    assert!(log[0].author.time_ms > 1_600_000_000_000);
    assert_eq!(log[0].parents.len(), 1);
    assert_eq!(log[1].parents.len(), 0);
    let only_readme = r.log("main", Some("README.md"), 0, 10).unwrap();
    assert_eq!(only_readme.len(), 1);
    assert_eq!(r.log("main", None, 1, 10).unwrap()[0].summary(), "first commit");
    assert_eq!(r.count_commits("main").unwrap(), 2);

    let (entry, blob) = r.readme("main").unwrap().unwrap();
    assert_eq!(entry.name, "README.md");
    assert!(blob.data.starts_with(b"# Hello"));

    assert!(store.size_kb(id).await.unwrap() >= 0);
    store.delete(id).await.unwrap();
    assert!(!store.exists(id));
    let _ = std::fs::remove_dir_all(&tmp);
}
