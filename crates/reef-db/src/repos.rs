//! Repositories, collaborators, and the permission model.

use crate::accounts::Account;
use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result, names};
use sqlx::AssertSqlSafe;
use sqlx::Row;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Repo {
    pub id: i64,
    pub owner_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub private: i64,
    pub default_branch: String,
    pub is_fork: i64,
    pub parent_id: Option<i64>,
    pub is_template: i64,
    pub archived: i64,
    pub has_issues: i64,
    pub has_projects: i64,
    pub has_wiki: i64,
    pub has_discussions: i64,
    pub allow_squash_merge: i64,
    pub allow_merge_commit: i64,
    pub allow_rebase_merge: i64,
    pub allow_auto_merge: i64,
    pub delete_branch_on_merge: i64,
    pub stars_count: i64,
    pub forks_count: i64,
    pub watchers_count: i64,
    pub open_issues_count: i64,
    pub open_pulls_count: i64,
    pub size_kb: i64,
    pub is_empty: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub pushed_at: Option<i64>,
    pub topics: String,
    pub language: Option<String>,
}

impl Repo {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
    pub fn is_private(&self) -> bool {
        self.private != 0
    }
    pub fn topics(&self) -> Vec<String> {
        serde_json::from_str(&self.topics).unwrap_or_default()
    }
}

pub const COLS: &str = "id, owner_id, name, description, homepage, private, default_branch, is_fork, parent_id, is_template, archived, has_issues, has_projects, has_wiki, has_discussions, allow_squash_merge, allow_merge_commit, allow_rebase_merge, allow_auto_merge, delete_branch_on_merge, stars_count, forks_count, watchers_count, open_issues_count, open_pulls_count, size_kb, is_empty, created_at, updated_at, pushed_at, topics, language";

/// A repository together with its owner, which every API representation needs.
#[derive(Debug, Clone)]
pub struct RepoWithOwner {
    pub repo: Repo,
    pub owner: Account,
}

impl RepoWithOwner {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner.login, self.repo.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    None,
    Read,
    Triage,
    Write,
    Maintain,
    Admin,
}

impl Permission {
    pub fn parse(s: &str) -> Option<Permission> {
        Some(match s {
            "read" | "pull" => Permission::Read,
            "triage" => Permission::Triage,
            "write" | "push" => Permission::Write,
            "maintain" => Permission::Maintain,
            "admin" => Permission::Admin,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::None => "none",
            Permission::Read => "read",
            Permission::Triage => "triage",
            Permission::Write => "write",
            Permission::Maintain => "maintain",
            Permission::Admin => "admin",
        }
    }
    /// GitHub's GraphQL `viewerPermission` values.
    pub fn graphql(self) -> &'static str {
        match self {
            Permission::None => "NONE",
            Permission::Read => "READ",
            Permission::Triage => "TRIAGE",
            Permission::Write => "WRITE",
            Permission::Maintain => "MAINTAIN",
            Permission::Admin => "ADMIN",
        }
    }
    pub fn can_read(self) -> bool {
        self >= Permission::Read
    }
    pub fn can_write(self) -> bool {
        self >= Permission::Write
    }
    pub fn can_admin(self) -> bool {
        self >= Permission::Admin
    }
}

#[derive(Debug, Default, Clone)]
pub struct NewRepo {
    pub name: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub private: bool,
    pub default_branch: Option<String>,
    pub has_issues: Option<bool>,
    pub has_projects: Option<bool>,
    pub has_wiki: Option<bool>,
    pub is_template: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RepoPatch {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub homepage: Option<Option<String>>,
    pub private: Option<bool>,
    pub default_branch: Option<String>,
    pub has_issues: Option<bool>,
    pub has_projects: Option<bool>,
    pub has_wiki: Option<bool>,
    pub archived: Option<bool>,
    pub is_template: Option<bool>,
    pub allow_squash_merge: Option<bool>,
    pub allow_merge_commit: Option<bool>,
    pub allow_rebase_merge: Option<bool>,
    pub allow_auto_merge: Option<bool>,
    pub delete_branch_on_merge: Option<bool>,
    pub topics: Option<Vec<String>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Collaborator {
    pub repo_id: i64,
    pub user_id: i64,
    pub permission: String,
    pub created_at: i64,
}

impl Db {
    pub async fn create_repo(&self, owner: Id, r: NewRepo) -> Result<Repo> {
        names::validate_repo_name(&r.name)?;
        if let Some(b) = &r.default_branch {
            names::validate_branch(b)?;
        }
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query(
            "INSERT INTO repositories (id, owner_id, name, name_lower, description, homepage, private, default_branch, is_template, has_issues, has_projects, has_wiki, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)",
        )
        .bind(id.0)
        .bind(owner.0)
        .bind(&r.name)
        .bind(r.name.to_lowercase())
        .bind(&r.description)
        .bind(&r.homepage)
        .bind(b(r.private))
        .bind(r.default_branch.as_deref().unwrap_or("main"))
        .bind(b(r.is_template))
        .bind(b(r.has_issues.unwrap_or(true)))
        .bind(b(r.has_projects.unwrap_or(true)))
        .bind(b(r.has_wiki.unwrap_or(true)))
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(|e| match map_err(e) {
            Error::Conflict(_) => Error::field("Repository", "name", "custom", "name already exists on this account"),
            o => o,
        })?;
        self.repo_by_id(id).await
    }

    pub async fn repo_by_id(&self, id: Id) -> Result<Repo> {
        sqlx::query_as::<_, Repo>(AssertSqlSafe(format!("SELECT {COLS} FROM repositories WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn repo_with_owner_by_id(&self, id: Id) -> Result<RepoWithOwner> {
        let repo = self.repo_by_id(id).await?;
        let owner = self.account_by_id(Id(repo.owner_id)).await?;
        Ok(RepoWithOwner { repo, owner })
    }

    /// Look up `owner/name`, case-insensitively, trimming a trailing `.git`.
    pub async fn repo_by_path(&self, owner: &str, name: &str) -> Result<RepoWithOwner> {
        let name = name.strip_suffix(".git").unwrap_or(name);
        let owner = self.account_by_login(owner).await?;
        let repo =
            sqlx::query_as::<_, Repo>(AssertSqlSafe(format!("SELECT {COLS} FROM repositories WHERE owner_id = $1 AND name_lower = $2")))
                .bind(owner.id)
                .bind(name.to_lowercase())
                .fetch_one(&self.pool)
                .await
                .map_err(map_err)?;
        Ok(RepoWithOwner { repo, owner })
    }

    pub async fn update_repo(&self, id: Id, p: RepoPatch) -> Result<Repo> {
        if let Some(n) = &p.name {
            names::validate_repo_name(n)?;
        }
        if let Some(b) = &p.default_branch {
            names::validate_branch(b)?;
        }
        let mut sets: Vec<String> = Vec::new();
        let mut q_args: Vec<Arg> = Vec::new();
        macro_rules! set {
            ($col:expr, $val:expr) => {{
                q_args.push($val);
                sets.push(format!("{} = ${}", $col, q_args.len()));
            }};
        }
        if let Some(n) = &p.name {
            set!("name", Arg::S(Some(n.clone())));
            set!("name_lower", Arg::S(Some(n.to_lowercase())));
        }
        if let Some(v) = p.description {
            set!("description", Arg::S(v));
        }
        if let Some(v) = p.homepage {
            set!("homepage", Arg::S(v));
        }
        if let Some(v) = p.private {
            set!("private", Arg::I(b(v)));
        }
        if let Some(v) = p.default_branch {
            set!("default_branch", Arg::S(Some(v)));
        }
        if let Some(v) = p.has_issues {
            set!("has_issues", Arg::I(b(v)));
        }
        if let Some(v) = p.has_projects {
            set!("has_projects", Arg::I(b(v)));
        }
        if let Some(v) = p.has_wiki {
            set!("has_wiki", Arg::I(b(v)));
        }
        if let Some(v) = p.archived {
            set!("archived", Arg::I(b(v)));
        }
        if let Some(v) = p.is_template {
            set!("is_template", Arg::I(b(v)));
        }
        if let Some(v) = p.allow_squash_merge {
            set!("allow_squash_merge", Arg::I(b(v)));
        }
        if let Some(v) = p.allow_merge_commit {
            set!("allow_merge_commit", Arg::I(b(v)));
        }
        if let Some(v) = p.allow_rebase_merge {
            set!("allow_rebase_merge", Arg::I(b(v)));
        }
        if let Some(v) = p.allow_auto_merge {
            set!("allow_auto_merge", Arg::I(b(v)));
        }
        if let Some(v) = p.delete_branch_on_merge {
            set!("delete_branch_on_merge", Arg::I(b(v)));
        }
        if let Some(v) = &p.topics {
            set!("topics", Arg::S(Some(serde_json::to_string(v).unwrap_or_else(|_| "[]".into()))));
        }
        if sets.is_empty() {
            return self.repo_by_id(id).await;
        }
        let n = q_args.len();
        let sql = format!("UPDATE repositories SET {}, updated_at = ${} WHERE id = ${}", sets.join(", "), n + 1, n + 2);
        let mut q = sqlx::query(AssertSqlSafe(sql.clone()));
        for a in q_args {
            q = match a {
                Arg::S(s) => q.bind(s),
                Arg::I(i) => q.bind(i),
            };
        }
        q.bind(now()).bind(id.0).execute(&self.pool).await.map_err(|e| match map_err(e) {
            Error::Conflict(_) => Error::field("Repository", "name", "custom", "name already exists on this account"),
            o => o,
        })?;
        self.repo_by_id(id).await
    }

    /// Record a push: bump `pushed_at`, mark non-empty, and update size.
    /// Primary language shown in repository listings (recomputed after pushes).
    pub async fn set_language(&self, id: Id, language: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE repositories SET language = $1 WHERE id = $2")
            .bind(language)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Set the size shown for a repository (git objects plus LFS bytes).
    pub async fn set_repo_size_kb(&self, id: Id, size_kb: i64) -> Result<()> {
        sqlx::query("UPDATE repositories SET size_kb = $1 WHERE id = $2")
            .bind(size_kb)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn touch_repo_push(&self, id: Id, size_kb: Option<i64>, is_empty: bool) -> Result<()> {
        let ts = now();
        sqlx::query(
            "UPDATE repositories SET pushed_at = $1, updated_at = $1, is_empty = $2, size_kb = COALESCE($3, size_kb) WHERE id = $4",
        )
        .bind(ts)
        .bind(b(is_empty))
        .bind(size_kb)
        .bind(id.0)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    pub async fn delete_repo(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM collaborators WHERE repo_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("DELETE FROM repo_events WHERE repo_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        let r = sqlx::query("DELETE FROM repositories WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    /// Repositories owned by an account, filtered to what `viewer` may see.
    pub async fn repos_for_owner(&self, owner: &Account, viewer: Option<&Account>, page: i64, per_page: i64) -> Result<Vec<RepoWithOwner>> {
        let all = sqlx::query_as::<_, Repo>(AssertSqlSafe(format!(
            "SELECT {COLS} FROM repositories WHERE owner_id = $1 ORDER BY updated_at DESC"
        )))
        .bind(owner.id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out = Vec::new();
        for repo in all {
            let rw = RepoWithOwner { repo, owner: owner.clone() };
            if self.permission(viewer, &rw).await?.can_read() {
                out.push(rw);
            }
        }
        let start = ((page.max(1) - 1) * per_page) as usize;
        Ok(out.into_iter().skip(start).take(per_page as usize).collect())
    }

    /// Everything the user can see: owned, organization, and collaborator repositories.
    pub async fn repos_for_user(&self, user: &Account, affiliation: &str) -> Result<Vec<RepoWithOwner>> {
        let want_owner = affiliation.contains("owner");
        let want_collab = affiliation.contains("collaborator");
        let want_org = affiliation.contains("organization_member");
        let mut ids: Vec<i64> = Vec::new();
        if want_owner {
            let rows = sqlx::query("SELECT id FROM repositories WHERE owner_id = $1")
                .bind(user.id)
                .fetch_all(&self.pool)
                .await
                .map_err(map_err)?;
            ids.extend(rows.iter().map(|r| r.get::<i64, _>("id")));
        }
        if want_collab {
            let rows = sqlx::query("SELECT repo_id FROM collaborators WHERE user_id = $1")
                .bind(user.id)
                .fetch_all(&self.pool)
                .await
                .map_err(map_err)?;
            ids.extend(rows.iter().map(|r| r.get::<i64, _>("repo_id")));
        }
        if want_org {
            let rows =
                sqlx::query("SELECT r.id AS id FROM repositories r JOIN org_members m ON m.org_id = r.owner_id WHERE m.user_id = $1")
                    .bind(user.id)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(map_err)?;
            ids.extend(rows.iter().map(|r| r.get::<i64, _>("id")));
        }
        ids.sort_unstable();
        ids.dedup();
        let mut out = Vec::new();
        for id in ids {
            if let Ok(rw) = self.repo_with_owner_by_id(Id(id)).await {
                out.push(rw);
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.repo.updated_at));
        Ok(out)
    }

    /// Public repositories, newest first, for the explore page and `GET /repositories`.
    pub async fn public_repos(&self, since: i64, limit: i64) -> Result<Vec<RepoWithOwner>> {
        let repos = sqlx::query_as::<_, Repo>(AssertSqlSafe(format!(
            "SELECT {COLS} FROM repositories WHERE private = 0 AND id > $1 ORDER BY id LIMIT $2"
        )))
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let owner_ids: Vec<i64> = repos.iter().map(|r| r.owner_id).collect();
        let owners = self.accounts_by_ids(&owner_ids).await?;
        Ok(repos
            .into_iter()
            .filter_map(|repo| owners.iter().find(|o| o.id == repo.owner_id).cloned().map(|owner| RepoWithOwner { repo, owner }))
            .collect())
    }

    pub async fn count_repos(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM repositories").fetch_one(&self.pool).await.map_err(map_err)?;
        row.try_get::<i64, _>("n").map_err(map_err)
    }

    pub async fn count_repos_for_owner(&self, owner: Id, include_private: bool) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM repositories WHERE owner_id = $1 AND ($2 = 1 OR private = 0)")
            .bind(owner.0)
            .bind(b(include_private))
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        row.try_get::<i64, _>("n").map_err(map_err)
    }

    // ----- collaborators -----

    pub async fn set_collaborator(&self, repo: Id, user: Id, permission: Permission) -> Result<()> {
        sqlx::query(
            "INSERT INTO collaborators (repo_id, user_id, permission, created_at) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (repo_id, user_id) DO UPDATE SET permission = $3",
        )
        .bind(repo.0)
        .bind(user.0)
        .bind(permission.as_str())
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        Ok(())
    }

    pub async fn remove_collaborator(&self, repo: Id, user: Id) -> Result<()> {
        sqlx::query("DELETE FROM collaborators WHERE repo_id = $1 AND user_id = $2")
            .bind(repo.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn collaborators(&self, repo: Id) -> Result<Vec<(Account, Permission)>> {
        let rows = sqlx::query_as::<_, Collaborator>(
            "SELECT repo_id, user_id, permission, created_at FROM collaborators WHERE repo_id = $1 ORDER BY created_at",
        )
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let ids: Vec<i64> = rows.iter().map(|c| c.user_id).collect();
        let accts = self.accounts_by_ids(&ids).await?;
        Ok(rows
            .into_iter()
            .filter_map(|c| {
                accts.iter().find(|a| a.id == c.user_id).cloned().map(|a| (a, Permission::parse(&c.permission).unwrap_or(Permission::Read)))
            })
            .collect())
    }

    /// The effective permission of `viewer` (or an anonymous visitor) on a repository.
    pub async fn permission(&self, viewer: Option<&Account>, rw: &RepoWithOwner) -> Result<Permission> {
        let base = if rw.repo.is_private() { Permission::None } else { Permission::Read };
        let Some(v) = viewer else { return Ok(base) };
        // A job token acts only within its own repository.
        if let Some(jr) = v.job_repo {
            return Ok(if jr == rw.repo.id { Permission::Write } else { base });
        }
        if v.is_admin() || v.id == rw.owner.id {
            return Ok(Permission::Admin);
        }
        let mut best = base;
        if rw.owner.is_org()
            && let Some(m) = self.org_membership(Id(rw.owner.id), Id(v.id)).await?
        {
            best = best.max(if m.role == "admin" { Permission::Admin } else { Permission::Read });
        }
        let c = sqlx::query("SELECT permission FROM collaborators WHERE repo_id = $1 AND user_id = $2")
            .bind(rw.repo.id)
            .bind(v.id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)?;
        if let Some(row) = c {
            let p: String = row.get("permission");
            best = best.max(Permission::parse(&p).unwrap_or(Permission::Read));
        }
        Ok(best)
    }

    /// May `viewer` create repositories under `owner`?
    pub async fn can_create_in(&self, viewer: &Account, owner: &Account) -> Result<bool> {
        if viewer.is_admin() || viewer.id == owner.id {
            return Ok(true);
        }
        if owner.is_org() {
            // Every member may create repositories in the organization, as GitHub's default allows.
            return Ok(self.org_membership(Id(owner.id), Id(viewer.id)).await?.is_some());
        }
        Ok(false)
    }
}

enum Arg {
    S(Option<String>),
    I(i64),
}
