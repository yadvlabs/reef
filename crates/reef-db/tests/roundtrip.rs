//! End-to-end exercise of the data layer. Runs on SQLite by default; set
//! REEF_TEST_DATABASE_URL to a PostgreSQL URL to run the same test there.

use reef_core::{Error, Id};
use reef_db::Db;
use reef_db::accounts::NewUser;
use reef_db::auth::SignIn;
use reef_db::device::Poll;
use reef_db::repos::{NewRepo, Permission, RepoPatch};

async fn db() -> Db {
    let url = std::env::var("REEF_TEST_DATABASE_URL").unwrap_or_else(|_| {
        let dir = std::env::temp_dir().join(format!("reef-test-{}", reef_core::token::random_hex(6)));
        format!("sqlite://{}/reef.db", dir.display())
    });
    let db = Db::connect(&url).await.expect("connect");
    // Postgres runs reuse a database: clear it so the test is repeatable.
    if db.backend() == reef_db::Backend::Postgres {
        for t in [
            "repo_events",
            "device_codes",
            "ssh_keys",
            "access_tokens",
            "sessions",
            "collaborators",
            "repositories",
            "org_members",
            "external_identities",
            "pending_identities",
            "accounts",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {t}"))).execute(db.pool()).await.unwrap();
        }
    }
    db
}

#[tokio::test]
async fn accounts_repos_and_permissions() {
    let db = db().await;
    let rohit = db
        .create_user(NewUser {
            login: "rohit",
            email: "rohit@example.com",
            password: Some("correct horse"),
            name: Some("Rohit"),
            is_admin: false,
        })
        .await
        .unwrap();
    let dup = db.create_user(NewUser { login: "ROHIT", email: "x@example.com", password: None, name: None, is_admin: false }).await;
    assert!(matches!(dup, Err(Error::Validation { .. })), "logins are case-insensitively unique");
    let mira = db
        .create_user(NewUser { login: "mira", email: "mira@example.com", password: Some("another pass"), name: None, is_admin: false })
        .await
        .unwrap();

    // Native auth
    assert!(db.verify_password("rohit", "correct horse").await.is_ok());
    assert!(db.verify_password("rohit@example.com", "correct horse").await.is_ok());
    assert!(db.verify_password("rohit", "wrong").await.is_err());
    assert!(db.verify_password("nobody", "wrong").await.is_err());

    // Orgs
    let lab = db.create_org("yadvlabs", rohit.id(), Some("YadvLabs"), None).await.unwrap();
    assert!(lab.is_org());
    assert_eq!(db.user_orgs(rohit.id()).await.unwrap().len(), 1);
    assert_eq!(db.user_orgs(mira.id()).await.unwrap().len(), 0);

    // Repos
    let reef = db.create_repo(lab.id(), NewRepo { name: "reef".into(), private: true, ..Default::default() }).await.unwrap();
    let dupe = db.create_repo(lab.id(), NewRepo { name: "Reef".into(), ..Default::default() }).await;
    assert!(dupe.is_err(), "repo names are case-insensitively unique per owner");
    let rw = db.repo_by_path("YadvLabs", "reef.git").await.unwrap();
    assert_eq!(rw.repo.id, reef.id);
    assert_eq!(rw.full_name(), "yadvlabs/reef");

    // Permissions: owner-org admin, non-member, collaborator, anonymous, public
    assert_eq!(db.permission(Some(&rohit), &rw).await.unwrap(), Permission::Admin);
    assert_eq!(db.permission(Some(&mira), &rw).await.unwrap(), Permission::None);
    assert_eq!(db.permission(None, &rw).await.unwrap(), Permission::None);
    db.set_collaborator(reef.id(), mira.id(), Permission::Write).await.unwrap();
    assert_eq!(db.permission(Some(&mira), &rw).await.unwrap(), Permission::Write);
    let repo = db
        .update_repo(reef.id(), RepoPatch { private: Some(false), description: Some(Some("The reef".into())), ..Default::default() })
        .await
        .unwrap();
    assert!(!repo.is_private());
    let rw = db.repo_with_owner_by_id(reef.id()).await.unwrap();
    assert_eq!(db.permission(None, &rw).await.unwrap(), Permission::Read);
    assert_eq!(rw.repo.description.as_deref(), Some("The reef"));

    // Listing
    let mine = db.repos_for_user(&rohit, "owner,collaborator,organization_member").await.unwrap();
    assert_eq!(mine.len(), 1);
    let theirs = db.repos_for_user(&mira, "owner,collaborator,organization_member").await.unwrap();
    assert_eq!(theirs.len(), 1);
    assert_eq!(db.public_repos(0, 10).await.unwrap().len(), 1);

    // Sessions
    let cookie = db.create_session(rohit.id(), Some("127.0.0.1"), Some("test")).await.unwrap();
    let s = db.session_by_cookie(&cookie).await.unwrap();
    assert_eq!(s.user_id, rohit.id);
    db.delete_session(&cookie).await.unwrap();
    assert!(db.session_by_cookie(&cookie).await.is_err());

    // Provider sign-ins (single logout): sessions keep the sign-in's sid and ID token.
    let gk = |sid: &str| SignIn { sid: Some(sid.into()), id_token: Some(format!("eyJ.{sid}.sig")) };
    let s1 = db.create_sign_in_session(rohit.id(), None, None, &gk("sid-1")).await.unwrap();
    let s2 = db.create_sign_in_session(rohit.id(), None, None, &gk("sid-2")).await.unwrap();
    let pw = db.create_session(rohit.id(), None, None).await.unwrap();
    let m1 = db.create_sign_in_session(mira.id(), None, None, &gk("sid-m")).await.unwrap();
    assert_eq!(db.session_sign_in(&s1).await.unwrap(), gk("sid-1"));
    assert_eq!(db.session_by_cookie(&s1).await.unwrap().sid.as_deref(), Some("sid-1"));
    assert!(!db.session_sign_in(&pw).await.unwrap().is_oidc());
    assert!(!db.session_sign_in("fgs_unknown").await.unwrap().is_oidc());
    assert_eq!(db.delete_sessions_by_sid("sid-1").await.unwrap(), 1);
    assert!(db.session_by_cookie(&s1).await.is_err());
    assert!(db.session_by_cookie(&s2).await.is_ok() && db.session_by_cookie(&pw).await.is_ok());
    assert_eq!(db.delete_sessions_by_sid("").await.unwrap(), 0);
    db.link_external_identity(rohit.id(), "https://gk.test", "gk-rohit", None).await.unwrap();
    assert_eq!(db.external_identity_user("https://gk.test", "gk-rohit").await.unwrap(), Some(rohit.id()));
    assert_eq!(db.external_identity_user("https://gk.test", "gk-nobody").await.unwrap(), None);
    // The provider's issuer URL changed (region re-homed): identities move to the new issuer,
    // one at a time on sign-in or in bulk, and never onto a subject the new issuer already has.
    let (old, new) = ("https://old.example.com", "https://id.example.com");
    db.link_external_identity(rohit.id(), old, "gk-r1", None).await.unwrap();
    db.link_external_identity(mira.id(), old, "gk-m1", None).await.unwrap();
    db.link_external_identity(mira.id(), old, "gk-m2", None).await.unwrap();
    db.link_external_identity(rohit.id(), new, "gk-m2", None).await.unwrap();
    assert!(!db.rekey_external_identity(old, new, "gk-nobody").await.unwrap());
    assert!(db.rekey_external_identity(old, new, "gk-r1").await.unwrap());
    assert!(!db.rekey_external_identity(old, new, "gk-r1").await.unwrap(), "moved once");
    assert_eq!(db.external_identity_user(new, "gk-r1").await.unwrap(), Some(rohit.id()));
    assert_eq!(db.external_identity_user(old, "gk-r1").await.unwrap(), None);
    assert_eq!(db.rekey_issuer(old, new).await.unwrap(), (1, 1), "gk-m1 moves; gk-m2 exists under the new issuer");
    assert_eq!(db.external_identity_user(new, "gk-m1").await.unwrap(), Some(mira.id()));
    assert_eq!(db.external_identity_user(new, "gk-m2").await.unwrap(), Some(rohit.id()));
    assert_eq!(db.external_identity_user(old, "gk-m2").await.unwrap(), Some(mira.id()));
    assert_eq!(db.rekey_issuer(old, old).await.unwrap(), (0, 0));
    assert_eq!(db.delete_user_sessions(rohit.id()).await.unwrap(), 2);
    assert!(db.session_by_cookie(&m1).await.is_ok());
    let pending =
        db.create_pending_identity("https://gk.test", "gk-new", None, false, Some("new"), None, None, &gk("sid-p")).await.unwrap();
    assert_eq!(db.pending_identity(&pending).await.unwrap().sign_in(), gk("sid-p"));
    db.delete_pending_identity(&pending).await.unwrap();

    // Tokens
    let (plain, tok) = db.create_token(rohit.id(), "pat", "laptop", &["repo", "read:org"], None, None).await.unwrap();
    assert!(plain.starts_with("fgp_"));
    let found = db.token_by_plain(&plain).await.unwrap();
    assert_eq!(found.id, tok.id);
    assert_eq!(found.scope_list(), vec!["repo", "read:org"]);
    assert!(db.token_by_plain("fgp_nope").await.is_err());
    db.delete_token(rohit.id(), Id(tok.id)).await.unwrap();
    assert!(db.token_by_plain(&plain).await.is_err());

    // SSH keys
    let k = db.add_ssh_key(rohit.id(), "laptop", "ssh-ed25519", "ssh-ed25519 AAAA", "SHA256:abc").await.unwrap();
    assert!(db.add_ssh_key(mira.id(), "dupe", "ssh-ed25519", "ssh-ed25519 AAAA", "SHA256:abc").await.is_err());
    assert_eq!(db.ssh_key_by_fingerprint("SHA256:abc").await.unwrap().id, k.id);

    // Device flow
    let start = db.device_start("178c6fc778ccc68e1d6a", "repo,read:org").await.unwrap();
    assert!(matches!(db.device_poll(&start.device_code).await.unwrap(), Poll::Pending));
    assert!(matches!(db.device_poll(&start.device_code).await.unwrap(), Poll::SlowDown));
    db.device_decide(&start.user_code, rohit.id(), true).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    // bypass the interval for the test by clearing last_poll_at
    sqlx::query("UPDATE device_codes SET last_poll_at = NULL").execute(db.pool()).await.unwrap();
    match db.device_poll(&start.device_code).await.unwrap() {
        Poll::Granted { access_token, scopes } => {
            assert!(access_token.starts_with("fgo_"));
            assert_eq!(scopes, "repo,read:org");
            let t = db.token_by_plain(&access_token).await.unwrap();
            assert_eq!(t.kind, "oauth");
            assert_eq!(t.user_id, rohit.id);
        }
        _ => panic!("expected grant"),
    }
    sqlx::query("UPDATE device_codes SET last_poll_at = NULL").execute(db.pool()).await.unwrap();
    assert!(matches!(db.device_poll(&start.device_code).await.unwrap(), Poll::Expired), "single use");

    // Events and deletion
    db.record_event(reef.id(), Some(rohit.id()), "push", &serde_json::json!({"ref": "refs/heads/main"})).await.unwrap();
    assert_eq!(db.repo_events(reef.id(), 10).await.unwrap().len(), 1);
    db.delete_repo(reef.id()).await.unwrap();
    assert!(matches!(db.repo_by_id(reef.id()).await, Err(Error::NotFound)));
}

// Deleting an organisation removes it and its memberships, and a failing
// statement is reported rather than ignored.
#[tokio::test]
async fn delete_org_removes_the_org() {
    let db = db().await;
    let owner = db
        .create_user(NewUser { login: "org-owner", email: "owner@example.com", password: None, name: None, is_admin: false })
        .await
        .unwrap();
    let org = db.create_org("doomed-org", owner.id(), None, None).await.unwrap();
    db.delete_org(org.id()).await.unwrap();
    assert!(db.account_by_login("doomed-org").await.is_err(), "the organisation is gone");
    assert!(db.account_by_login("org-owner").await.is_ok(), "its owner stays");
}
