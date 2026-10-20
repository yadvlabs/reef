//! GitHub REST JSON representations. Field names and shapes follow the OpenAPI schemas
//! `simple-user`, `public-user`, `private-user`, `organization-simple`, `organization-full`,
//! `repository`, `full-repository`, `short-branch`, `commit`, `content-file`, `content-directory`.

use crate::Config;
use base64::Engine;
use reef_core::time::{iso, iso_opt};
use reef_db::accounts::Account;
use reef_db::repos::{Permission, RepoWithOwner};
use reef_git::Commit;
use serde_json::{Value, json};

/// Opaque GraphQL node ids: `<prefix>_<base64url(decimal id)>`.
pub fn node_id(prefix: &str, id: i64) -> String {
    format!("{prefix}_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.to_string()))
}

pub fn decode_node_id(s: &str) -> Option<(String, i64)> {
    let (prefix, b64) = s.split_once('_')?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b64).ok()?;
    let id = std::str::from_utf8(&bytes).ok()?.parse().ok()?;
    Some((prefix.to_string(), id))
}

pub const NODE_USER: &str = "U";
pub const NODE_ORG: &str = "O";
pub const NODE_REPO: &str = "R";
pub const NODE_REF: &str = "REF";
pub const NODE_COMMIT: &str = "C";
pub const NODE_KEY: &str = "K";

pub fn avatar_url(cfg: &Config, a: &Account) -> String {
    match (&a.avatar_url, a.avatar_updated_at) {
        (_, Some(v)) => format!("{}/avatars/{}?v={v}", cfg.site_url(), a.id),
        (Some(u), None) => u.clone(),
        (None, None) => format!("{}/avatars/{}?v=4", cfg.site_url(), a.id),
    }
}

pub fn account_type(a: &Account) -> &'static str {
    if a.is_org() { "Organization" } else { "User" }
}

pub fn account_node_id(a: &Account) -> String {
    node_id(if a.is_org() { NODE_ORG } else { NODE_USER }, a.id)
}

pub fn simple_user(cfg: &Config, a: &Account) -> Value {
    let api = cfg.api_url();
    let site = cfg.site_url();
    let l = &a.login;
    json!({
        "login": l,
        "id": a.id,
        "node_id": account_node_id(a),
        "avatar_url": avatar_url(cfg, a),
        "gravatar_id": "",
        "url": format!("{api}/users/{l}"),
        "html_url": format!("{site}/{l}"),
        "followers_url": format!("{api}/users/{l}/followers"),
        "following_url": format!("{api}/users/{l}/following{{/other_user}}"),
        "gists_url": format!("{api}/users/{l}/gists{{/gist_id}}"),
        "starred_url": format!("{api}/users/{l}/starred{{/owner}}{{/repo}}"),
        "subscriptions_url": format!("{api}/users/{l}/subscriptions"),
        "organizations_url": format!("{api}/users/{l}/orgs"),
        "repos_url": format!("{api}/users/{l}/repos"),
        "events_url": format!("{api}/users/{l}/events{{/privacy}}"),
        "received_events_url": format!("{api}/users/{l}/received_events"),
        "type": account_type(a),
        "user_view_type": "public",
        "site_admin": a.is_admin(),
    })
}

/// `public-user`; `private` adds the fields only the user themself sees.
pub fn full_user(cfg: &Config, a: &Account, public_repos: i64, private: bool) -> Value {
    let mut v = simple_user(cfg, a);
    let o = v.as_object_mut().unwrap();
    o.insert("name".into(), json!(a.name));
    o.insert("company".into(), json!(a.company));
    o.insert("blog".into(), json!(a.blog.clone().unwrap_or_default()));
    o.insert("location".into(), json!(a.location));
    o.insert("email".into(), json!(if private { a.email.clone() } else { None }));
    o.insert("hireable".into(), Value::Null);
    o.insert("bio".into(), json!(a.bio));
    o.insert("twitter_username".into(), Value::Null);
    o.insert("notification_email".into(), json!(if private { a.email.clone() } else { None }));
    o.insert("public_repos".into(), json!(public_repos));
    o.insert("public_gists".into(), json!(0));
    o.insert("followers".into(), json!(0));
    o.insert("following".into(), json!(0));
    o.insert("created_at".into(), json!(iso(a.created_at)));
    o.insert("updated_at".into(), json!(iso(a.updated_at)));
    if private {
        o.insert("private_gists".into(), json!(0));
        o.insert("total_private_repos".into(), json!(0));
        o.insert("owned_private_repos".into(), json!(0));
        o.insert("disk_usage".into(), json!(0));
        o.insert("collaborators".into(), json!(0));
        o.insert("two_factor_authentication".into(), json!(false));
        o.insert("plan".into(), json!({"name": "enterprise", "space": 0, "private_repos": 9999, "collaborators": 0}));
        o.insert("user_view_type".into(), json!("private"));
    }
    v
}

pub fn org_simple(cfg: &Config, a: &Account) -> Value {
    let api = cfg.api_url();
    let l = &a.login;
    json!({
        "login": l,
        "id": a.id,
        "node_id": account_node_id(a),
        "url": format!("{api}/orgs/{l}"),
        "repos_url": format!("{api}/orgs/{l}/repos"),
        "events_url": format!("{api}/orgs/{l}/events"),
        "hooks_url": format!("{api}/orgs/{l}/hooks"),
        "issues_url": format!("{api}/orgs/{l}/issues"),
        "members_url": format!("{api}/orgs/{l}/members{{/member}}"),
        "public_members_url": format!("{api}/orgs/{l}/public_members{{/member}}"),
        "avatar_url": avatar_url(cfg, a),
        "description": a.description,
    })
}

pub fn org_full(cfg: &Config, a: &Account, public_repos: i64, is_member_or_admin: bool) -> Value {
    let mut v = org_simple(cfg, a);
    let o = v.as_object_mut().unwrap();
    o.insert("name".into(), json!(a.name));
    o.insert("company".into(), json!(a.company));
    o.insert("blog".into(), json!(a.blog));
    o.insert("location".into(), json!(a.location));
    o.insert("email".into(), json!(a.billing_email));
    o.insert("twitter_username".into(), Value::Null);
    o.insert("is_verified".into(), json!(false));
    o.insert("has_organization_projects".into(), json!(true));
    o.insert("has_repository_projects".into(), json!(true));
    o.insert("public_repos".into(), json!(public_repos));
    o.insert("public_gists".into(), json!(0));
    o.insert("followers".into(), json!(0));
    o.insert("following".into(), json!(0));
    o.insert("html_url".into(), json!(format!("{}/{}", cfg.site_url(), a.login)));
    o.insert("created_at".into(), json!(iso(a.created_at)));
    o.insert("updated_at".into(), json!(iso(a.updated_at)));
    o.insert("archived_at".into(), Value::Null);
    o.insert("type".into(), json!("Organization"));
    if is_member_or_admin {
        o.insert("billing_email".into(), json!(a.billing_email));
        o.insert("default_repository_permission".into(), json!("read"));
        o.insert("members_can_create_repositories".into(), json!(true));
        o.insert("two_factor_requirement_enabled".into(), json!(false));
        o.insert("members_allowed_repository_creation_type".into(), json!("all"));
        o.insert("members_can_create_public_repositories".into(), json!(true));
        o.insert("members_can_create_private_repositories".into(), json!(true));
        o.insert("members_can_create_internal_repositories".into(), json!(false));
        o.insert("members_can_create_pages".into(), json!(true));
        o.insert("members_can_fork_private_repositories".into(), json!(false));
        o.insert("web_commit_signoff_required".into(), json!(false));
        o.insert("plan".into(), json!({"name": "enterprise", "space": 0, "private_repos": 9999, "filled_seats": 0, "seats": 0}));
    }
    v
}

pub fn clone_urls(cfg: &Config, rw: &RepoWithOwner) -> (String, String, String) {
    let full = rw.full_name();
    let https = format!("{}/{}.git", cfg.site_url(), full);
    let ssh = if cfg.ssh_port() == 22 {
        format!("git@{}:{}.git", cfg.ssh_host(), full)
    } else {
        format!("ssh://git@{}:{}/{}.git", cfg.ssh_host(), cfg.ssh_port(), full)
    };
    let git = format!("git://{}/{}.git", cfg.ssh_host(), full);
    (https, ssh, git)
}

/// `repository` / `full-repository`. `perm` is the viewer's permission, used for the
/// `permissions` object; pass `None` for listings where GitHub omits it.
pub fn repo(cfg: &Config, rw: &RepoWithOwner, perm: Option<Permission>) -> Value {
    let api = cfg.api_url();
    let site = cfg.site_url();
    let r = &rw.repo;
    let full = rw.full_name();
    let u = format!("{api}/repos/{full}");
    let (https, ssh, git) = clone_urls(cfg, rw);
    let b = |i: i64| i != 0;
    let mut v = json!({
        "id": r.id,
        "node_id": node_id(NODE_REPO, r.id),
        "name": r.name,
        "full_name": full,
        "private": b(r.private),
        "owner": simple_user(cfg, &rw.owner),
        "html_url": format!("{site}/{full}"),
        "description": r.description,
        "fork": b(r.is_fork),
        "url": u,
        "forks_url": format!("{u}/forks"),
        "keys_url": format!("{u}/keys{{/key_id}}"),
        "collaborators_url": format!("{u}/collaborators{{/collaborator}}"),
        "teams_url": format!("{u}/teams"),
        "hooks_url": format!("{u}/hooks"),
        "issue_events_url": format!("{u}/issues/events{{/number}}"),
        "events_url": format!("{u}/events"),
        "assignees_url": format!("{u}/assignees{{/user}}"),
        "branches_url": format!("{u}/branches{{/branch}}"),
        "tags_url": format!("{u}/tags"),
        "blobs_url": format!("{u}/git/blobs{{/sha}}"),
        "git_tags_url": format!("{u}/git/tags{{/sha}}"),
        "git_refs_url": format!("{u}/git/refs{{/sha}}"),
        "trees_url": format!("{u}/git/trees{{/sha}}"),
        "statuses_url": format!("{u}/statuses/{{sha}}"),
        "languages_url": format!("{u}/languages"),
        "stargazers_url": format!("{u}/stargazers"),
        "contributors_url": format!("{u}/contributors"),
        "subscribers_url": format!("{u}/subscribers"),
        "subscription_url": format!("{u}/subscription"),
        "commits_url": format!("{u}/commits{{/sha}}"),
        "git_commits_url": format!("{u}/git/commits{{/sha}}"),
        "comments_url": format!("{u}/comments{{/number}}"),
        "issue_comment_url": format!("{u}/issues/comments{{/number}}"),
        "contents_url": format!("{u}/contents/{{+path}}"),
        "compare_url": format!("{u}/compare/{{base}}...{{head}}"),
        "merges_url": format!("{u}/merges"),
        "archive_url": format!("{u}/{{archive_format}}{{/ref}}"),
        "downloads_url": format!("{u}/downloads"),
        "issues_url": format!("{u}/issues{{/number}}"),
        "pulls_url": format!("{u}/pulls{{/number}}"),
        "milestones_url": format!("{u}/milestones{{/number}}"),
        "notifications_url": format!("{u}/notifications{{?since,all,participating}}"),
        "labels_url": format!("{u}/labels{{/name}}"),
        "releases_url": format!("{u}/releases{{/id}}"),
        "deployments_url": format!("{u}/deployments"),
        "created_at": iso(r.created_at),
        "updated_at": iso(r.updated_at),
        "pushed_at": iso_opt(r.pushed_at),
        "git_url": git,
        "ssh_url": ssh,
        "clone_url": https,
        "svn_url": format!("{site}/{full}"),
        "homepage": r.homepage,
        "size": r.size_kb,
        "stargazers_count": r.stars_count,
        "watchers_count": r.watchers_count,
        "language": r.language,
        "has_issues": b(r.has_issues),
        "has_projects": b(r.has_projects),
        "has_downloads": true,
        "has_wiki": b(r.has_wiki),
        "has_pages": false,
        "has_discussions": b(r.has_discussions),
        "forks_count": r.forks_count,
        "mirror_url": Value::Null,
        "archived": b(r.archived),
        "disabled": false,
        "open_issues_count": r.open_issues_count + r.open_pulls_count,
        // Reef extras: GitHub's open_issues_count includes pull requests; the tabs want them apart.
        "open_issues_only_count": r.open_issues_count,
        "open_pull_requests_count": r.open_pulls_count,
        "license": Value::Null,
        "allow_forking": true,
        "is_template": b(r.is_template),
        "web_commit_signoff_required": false,
        "topics": r.topics(),
        "visibility": if b(r.private) { "private" } else { "public" },
        "forks": r.forks_count,
        "open_issues": r.open_issues_count + r.open_pulls_count,
        "watchers": r.watchers_count,
        "default_branch": r.default_branch,
        "allow_squash_merge": b(r.allow_squash_merge),
        "allow_merge_commit": b(r.allow_merge_commit),
        "allow_rebase_merge": b(r.allow_rebase_merge),
        "allow_auto_merge": b(r.allow_auto_merge),
        "delete_branch_on_merge": b(r.delete_branch_on_merge),
        "allow_update_branch": false,
        "use_squash_pr_title_as_default": false,
        "squash_merge_commit_message": "COMMIT_MESSAGES",
        "squash_merge_commit_title": "COMMIT_OR_PR_TITLE",
        "merge_commit_message": "PR_TITLE",
        "merge_commit_title": "MERGE_MESSAGE",
        "network_count": r.forks_count,
        "subscribers_count": r.watchers_count,
    });
    if let Some(p) = perm {
        v.as_object_mut().unwrap().insert(
            "permissions".into(),
            json!({
                "admin": p >= Permission::Admin,
                "maintain": p >= Permission::Maintain,
                "push": p >= Permission::Write,
                "triage": p >= Permission::Triage,
                "pull": p >= Permission::Read,
            }),
        );
    }
    if rw.owner.is_org() {
        v.as_object_mut().unwrap().insert("organization".into(), simple_user(cfg, &rw.owner));
    }
    v
}

pub fn branch_short(cfg: &Config, rw: &RepoWithOwner, name: &str, sha: &str) -> Value {
    let u = format!("{}/repos/{}", cfg.api_url(), rw.full_name());
    json!({
        "name": name,
        "commit": {"sha": sha, "url": format!("{u}/commits/{sha}")},
        "protected": false,
    })
}

pub fn tag(cfg: &Config, rw: &RepoWithOwner, name: &str, sha: &str) -> Value {
    let full = rw.full_name();
    let u = format!("{}/repos/{}", cfg.api_url(), full);
    json!({
        "name": name,
        "commit": {"sha": sha, "url": format!("{u}/commits/{sha}")},
        "zipball_url": format!("{u}/zipball/refs/tags/{name}"),
        "tarball_url": format!("{u}/tarball/refs/tags/{name}"),
        "node_id": node_id(NODE_REF, 0),
    })
}

fn git_user(s: &reef_git::Signature) -> Value {
    json!({"name": s.name, "email": s.email, "date": iso(s.time_ms)})
}

/// `commit` (the REST "repository commit" shape). `author`/`committer` are matched Reef
/// accounts when the email is known.
pub fn commit(cfg: &Config, rw: &RepoWithOwner, c: &Commit, author: Option<&Account>, committer: Option<&Account>) -> Value {
    commit_with_verification(cfg, rw, c, author, committer, None)
}

/// GitHub's `verification` object for an unsigned commit.
pub fn unsigned_verification() -> Value {
    json!({"verified": false, "reason": "unsigned", "signature": Value::Null, "payload": Value::Null, "verified_at": Value::Null})
}

/// Commit JSON with a precomputed signature `verification` (see `crate::signing`).
pub fn commit_with_verification(
    cfg: &Config,
    rw: &RepoWithOwner,
    c: &Commit,
    author: Option<&Account>,
    committer: Option<&Account>,
    verification: Option<Value>,
) -> Value {
    let verification = verification.unwrap_or_else(unsigned_verification);
    let full = rw.full_name();
    let api = cfg.api_url();
    let site = cfg.site_url();
    let u = format!("{api}/repos/{full}");
    json!({
        "sha": c.id,
        "node_id": node_id(NODE_COMMIT, 0),
        "commit": {
            "author": git_user(&c.author),
            "committer": git_user(&c.committer),
            "message": c.message.trim_end(),
            "tree": {"sha": c.tree, "url": format!("{u}/git/trees/{}", c.tree)},
            "url": format!("{u}/git/commits/{}", c.id),
            "comment_count": 0,
            "verification": verification,
        },
        "url": format!("{u}/commits/{}", c.id),
        "html_url": format!("{site}/{full}/commit/{}", c.id),
        "comments_url": format!("{u}/commits/{}/comments", c.id),
        "author": author.map(|a| simple_user(cfg, a)),
        "committer": committer.map(|a| simple_user(cfg, a)),
        "parents": c.parents.iter().map(|p| json!({
            "sha": p,
            "url": format!("{u}/commits/{p}"),
            "html_url": format!("{site}/{full}/commit/{p}"),
        })).collect::<Vec<_>>(),
    })
}

pub fn content_links(cfg: &Config, rw: &RepoWithOwner, path: &str, sha: &str, rev: &str, kind: &str) -> Value {
    let full = rw.full_name();
    let api = cfg.api_url();
    let site = cfg.site_url();
    let git_kind = if kind == "dir" { "trees" } else { "blobs" };
    json!({
        "self": format!("{api}/repos/{full}/contents/{path}?ref={rev}"),
        "git": format!("{api}/repos/{full}/git/{git_kind}/{sha}"),
        "html": format!("{site}/{full}/{}/{rev}/{path}", if kind == "dir" { "tree" } else { "blob" }),
    })
}

pub fn content_entry(cfg: &Config, rw: &RepoWithOwner, e: &reef_git::Entry, rev: &str, size: u64) -> Value {
    let full = rw.full_name();
    let api = cfg.api_url();
    let site = cfg.site_url();
    let kind = match e.kind {
        reef_git::EntryKind::Tree => "dir",
        reef_git::EntryKind::Symlink => "symlink",
        reef_git::EntryKind::Submodule => "submodule",
        _ => "file",
    };
    let git_kind = if kind == "dir" { "trees" } else { "blobs" };
    json!({
        "type": kind,
        "size": if kind == "dir" { 0 } else { size },
        "name": e.name,
        "path": e.path,
        "sha": e.id,
        "url": format!("{api}/repos/{full}/contents/{}?ref={rev}", e.path),
        "git_url": format!("{api}/repos/{full}/git/{git_kind}/{}", e.id),
        "html_url": format!("{site}/{full}/{}/{rev}/{}", if kind == "dir" { "tree" } else { "blob" }, e.path),
        "download_url": if kind == "dir" { Value::Null } else { json!(format!("{site}/{full}/raw/{rev}/{}", e.path)) },
        "_links": content_links(cfg, rw, &e.path, &e.id, rev, kind),
    })
}

pub fn content_file(cfg: &Config, rw: &RepoWithOwner, e: &reef_git::Entry, blob: &reef_git::Blob, rev: &str) -> Value {
    let mut v = content_entry(cfg, rw, e, rev, blob.size);
    let o = v.as_object_mut().unwrap();
    o.insert("encoding".into(), json!("base64"));
    o.insert("content".into(), json!(base64_lines(&blob.data)));
    v
}

/// GitHub wraps base64 content at 60 characters per line.
fn base64_lines(data: &[u8]) -> String {
    let s = base64::engine::general_purpose::STANDARD.encode(data);
    let mut out = String::with_capacity(s.len() + s.len() / 60 + 1);
    for chunk in s.as_bytes().chunks(60) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out
}

pub fn ssh_key(cfg: &Config, k: &reef_db::auth::SshKey) -> Value {
    json!({
        "id": k.id,
        "key": k.public_key,
        "title": k.title,
        "url": format!("{}/user/keys/{}", cfg.api_url(), k.id),
        "created_at": iso(k.created_at),
        "verified": true,
        "read_only": false,
        "last_used": iso_opt(k.last_used_at),
    })
}

pub fn token(t: &reef_db::auth::AccessToken, plain: Option<&str>) -> Value {
    json!({
        "id": t.id,
        "name": t.name,
        "kind": t.kind,
        "token": plain,
        "token_last4": t.token_last4,
        "scopes": t.scope_list(),
        "created_at": iso(t.created_at),
        "expires_at": iso_opt(t.expires_at),
        "last_used_at": iso_opt(t.last_used_at),
    })
}

// ----- issues -----

use reef_db::issues::{Comment, IssueEvent, IssueFull, Label, Milestone};

pub const NODE_ISSUE: &str = "I";
pub const NODE_PULL: &str = "PR";
pub const NODE_LABEL: &str = "LA";
pub const NODE_MILESTONE: &str = "MI";
pub const NODE_COMMENT: &str = "IC";

pub fn label(cfg: &Config, rw: &RepoWithOwner, l: &Label) -> Value {
    json!({
        "id": l.id,
        "node_id": node_id(NODE_LABEL, l.id),
        "url": format!("{}/repos/{}/labels/{}", cfg.api_url(), rw.full_name(), urlpath(&l.name)),
        "name": l.name,
        "color": l.color,
        "default": l.is_default != 0,
        "description": l.description,
    })
}

pub fn milestone(cfg: &Config, rw: &RepoWithOwner, m: &Milestone, open: i64, closed: i64, creator: Option<&Account>) -> Value {
    let full = rw.full_name();
    json!({
        "url": format!("{}/repos/{full}/milestones/{}", cfg.api_url(), m.number),
        "html_url": format!("{}/{full}/milestone/{}", cfg.site_url(), m.number),
        "labels_url": format!("{}/repos/{full}/milestones/{}/labels", cfg.api_url(), m.number),
        "id": m.id,
        "node_id": node_id(NODE_MILESTONE, m.id),
        "number": m.number,
        "state": m.state,
        "title": m.title,
        "description": m.description,
        "creator": creator.map(|c| simple_user(cfg, c)),
        "open_issues": open,
        "closed_issues": closed,
        "created_at": iso(m.created_at),
        "updated_at": iso(m.updated_at),
        "closed_at": iso_opt(m.closed_at),
        "due_on": iso_opt(m.due_on),
    })
}

/// Reaction summary in GitHub's `reactions` object shape.
pub fn reactions_json(
    url: String,
    counts: &std::collections::HashMap<(i64, String), i64>,
    mine: &std::collections::HashSet<(i64, String)>,
    subject: i64,
) -> Value {
    let get = |k: &str| counts.get(&(subject, k.to_string())).copied().unwrap_or(0);
    let total: i64 = reef_db::issues::REACTIONS.iter().map(|k| get(k)).sum();
    // `viewer_reactions` is a Reef extra so the UI can toggle without listing reactions.
    let viewer: Vec<&str> = reef_db::issues::REACTIONS.iter().copied().filter(|k| mine.contains(&(subject, k.to_string()))).collect();
    json!({
        "url": url,
        "total_count": total,
        "+1": get("+1"), "-1": get("-1"), "laugh": get("laugh"), "hooray": get("hooray"),
        "confused": get("confused"), "heart": get("heart"), "rocket": get("rocket"), "eyes": get("eyes"),
        "viewer_reactions": viewer,
    })
}

pub fn author_association(perm: Permission, is_owner: bool, is_org_member: bool) -> &'static str {
    if is_owner {
        "OWNER"
    } else if is_org_member {
        "MEMBER"
    } else if perm >= Permission::Triage {
        "COLLABORATOR"
    } else {
        "NONE"
    }
}

/// The REST `issue` shape. `milestone_counts` is (open, closed) for the issue's milestone.
pub fn issue(
    cfg: &Config,
    rw: &RepoWithOwner,
    i: &IssueFull,
    association: &str,
    reactions: Value,
    milestone_counts: (i64, i64),
    body_html: Option<String>,
) -> Value {
    let full = rw.full_name();
    let api = cfg.api_url();
    let site = cfg.site_url();
    let n = i.issue.number;
    let u = format!("{api}/repos/{full}/issues/{n}");
    let mut v = json!({
        "id": i.issue.id,
        "node_id": node_id(if i.issue.is_pull() { NODE_PULL } else { NODE_ISSUE }, i.issue.id),
        "url": u,
        "repository_url": format!("{api}/repos/{full}"),
        "labels_url": format!("{u}/labels{{/name}}"),
        "comments_url": format!("{u}/comments"),
        "events_url": format!("{u}/events"),
        "html_url": format!("{site}/{full}/{}/{n}", if i.issue.is_pull() { "pull" } else { "issues" }),
        "number": n,
        "state": i.issue.state,
        "state_reason": i.issue.state_reason,
        "title": i.issue.title,
        "body": i.issue.body,
        "user": simple_user(cfg, &i.author),
        "labels": i.labels.iter().map(|l| label(cfg, rw, l)).collect::<Vec<_>>(),
        "assignee": i.assignees.first().map(|a| simple_user(cfg, a)),
        "assignees": i.assignees.iter().map(|a| simple_user(cfg, a)).collect::<Vec<_>>(),
        "milestone": i.milestone.as_ref().map(|m| milestone(cfg, rw, m, milestone_counts.0, milestone_counts.1, None)),
        "locked": i.issue.locked != 0,
        "active_lock_reason": i.issue.lock_reason,
        "comments": i.issue.comments_count,
        "closed_at": iso_opt(i.issue.closed_at),
        "created_at": iso(i.issue.created_at),
        "updated_at": iso(i.issue.updated_at),
        "closed_by": i.closed_by.as_ref().map(|a| simple_user(cfg, a)),
        "author_association": association,
        "reactions": reactions,
        "timeline_url": format!("{u}/timeline"),
        "performed_via_github_app": Value::Null,
        "draft": false,
    });
    if i.issue.is_pull() {
        v.as_object_mut().unwrap().insert(
            "pull_request".into(),
            json!({
                "url": format!("{api}/repos/{full}/pulls/{n}"),
                "html_url": format!("{site}/{full}/pull/{n}"),
                "diff_url": format!("{site}/{full}/pull/{n}.diff"),
                "patch_url": format!("{site}/{full}/pull/{n}.patch"),
                "merged_at": Value::Null,
            }),
        );
    }
    if let Some(h) = body_html {
        v.as_object_mut().unwrap().insert("body_html".into(), json!(h));
    }
    v
}

#[allow(clippy::too_many_arguments)]
pub fn comment(
    cfg: &Config,
    rw: &RepoWithOwner,
    c: &Comment,
    author: &Account,
    issue_number: i64,
    association: &str,
    reactions: Value,
    body_html: Option<String>,
) -> Value {
    let full = rw.full_name();
    let api = cfg.api_url();
    let mut v = json!({
        "id": c.id,
        "node_id": node_id(NODE_COMMENT, c.id),
        "url": format!("{api}/repos/{full}/issues/comments/{}", c.id),
        "html_url": format!("{}/{full}/issues/{issue_number}#issuecomment-{}", cfg.site_url(), c.id),
        "body": c.body,
        "user": simple_user(cfg, author),
        "created_at": iso(c.created_at),
        "updated_at": iso(c.updated_at),
        "issue_url": format!("{api}/repos/{full}/issues/{issue_number}"),
        "author_association": association,
        "reactions": reactions,
        "performed_via_github_app": Value::Null,
    });
    if let Some(h) = body_html {
        v.as_object_mut().unwrap().insert("body_html".into(), json!(h));
    }
    v
}

/// An `issue-event` (also used inside the timeline). `extra` carries the event-specific
/// keys (label, assignee, milestone, rename) already resolved by the caller.
pub fn issue_event(cfg: &Config, rw: &RepoWithOwner, e: &IssueEvent, actor: Option<&Account>, extra: Value) -> Value {
    let mut v = json!({
        "id": e.id,
        "node_id": node_id("IE", e.id),
        "url": format!("{}/repos/{}/issues/events/{}", cfg.api_url(), rw.full_name(), e.id),
        "actor": actor.map(|a| simple_user(cfg, a)),
        "event": e.event,
        "commit_id": Value::Null,
        "commit_url": Value::Null,
        "created_at": iso(e.created_at),
        "performed_via_github_app": Value::Null,
    });
    if let Value::Object(m) = extra {
        for (k, val) in m {
            v.as_object_mut().unwrap().insert(k, val);
        }
    }
    v
}

pub fn urlpath(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
