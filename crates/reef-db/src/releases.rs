//! Releases: a tag with notes (and, later, assets).

use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::AssertSqlSafe;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Release {
    pub id: i64,
    pub repo_id: i64,
    pub tag_name: String,
    pub target_commitish: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub draft: i64,
    pub prerelease: i64,
    pub author_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub published_at: Option<i64>,
}

impl Release {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

const COLS: &str =
    "id, repo_id, tag_name, target_commitish, name, body, draft, prerelease, author_id, created_at, updated_at, published_at";

pub struct NewRelease<'a> {
    pub tag_name: &'a str,
    pub target_commitish: &'a str,
    pub name: Option<&'a str>,
    pub body: Option<&'a str>,
    pub draft: bool,
    pub prerelease: bool,
    pub author: Option<Id>,
}

#[derive(Default)]
pub struct ReleasePatch<'a> {
    pub tag_name: Option<&'a str>,
    pub target_commitish: Option<&'a str>,
    pub name: Option<Option<&'a str>>,
    pub body: Option<Option<&'a str>>,
    pub draft: Option<bool>,
    pub prerelease: Option<bool>,
}

impl Db {
    pub async fn create_release(&self, repo: Id, n: NewRelease<'_>) -> Result<Release> {
        let tag = n.tag_name.trim();
        if tag.is_empty() {
            return Err(Error::field("Release", "tag_name", "missing_field", "tag_name is required"));
        }
        reef_core::names::validate_branch(tag)
            .map_err(|_| Error::field("Release", "tag_name", "invalid", "tag_name is not a valid tag"))?;
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO releases (id, repo_id, tag_name, target_commitish, name, body, draft, prerelease, author_id, created_at, updated_at, published_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10, $11)")
            .bind(id.0)
            .bind(repo.0)
            .bind(tag)
            .bind(n.target_commitish)
            .bind(n.name)
            .bind(n.body)
            .bind(b(n.draft))
            .bind(b(n.prerelease))
            .bind(n.author.map(|a| a.0))
            .bind(ts)
            .bind(if n.draft { None } else { Some(ts) })
            .execute(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::Conflict(_) => Error::field("Release", "tag_name", "already_exists", "a release for this tag already exists"),
                o => o,
            })?;
        self.release_by_id(id).await
    }

    pub async fn release_by_id(&self, id: Id) -> Result<Release> {
        sqlx::query_as::<_, Release>(AssertSqlSafe(format!("SELECT {COLS} FROM releases WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn release_by_tag(&self, repo: Id, tag: &str) -> Result<Release> {
        sqlx::query_as::<_, Release>(AssertSqlSafe(format!("SELECT {COLS} FROM releases WHERE repo_id = $1 AND tag_name = $2")))
            .bind(repo.0)
            .bind(tag)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Newest published, non-prerelease release.
    pub async fn latest_release(&self, repo: Id) -> Result<Release> {
        sqlx::query_as::<_, Release>(AssertSqlSafe(format!("SELECT {COLS} FROM releases WHERE repo_id = $1 AND draft = 0 AND prerelease = 0 ORDER BY published_at DESC, created_at DESC LIMIT 1"))).bind(repo.0).fetch_one(&self.pool).await.map_err(map_err)
    }

    /// All releases, newest first; drafts only when `include_drafts`.
    pub async fn releases(&self, repo: Id, include_drafts: bool) -> Result<Vec<Release>> {
        let extra = if include_drafts { "" } else { "AND draft = 0" };
        sqlx::query_as::<_, Release>(AssertSqlSafe(format!(
            "SELECT {COLS} FROM releases WHERE repo_id = $1 {extra} ORDER BY created_at DESC"
        )))
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn update_release(&self, id: Id, p: ReleasePatch<'_>) -> Result<Release> {
        let cur = self.release_by_id(id).await?;
        let draft = p.draft.map(b).unwrap_or(cur.draft);
        let published = if cur.draft != 0 && draft == 0 { Some(now()) } else { cur.published_at };
        sqlx::query("UPDATE releases SET tag_name = $1, target_commitish = $2, name = $3, body = $4, draft = $5, prerelease = $6, updated_at = $7, published_at = $8 WHERE id = $9")
            .bind(p.tag_name.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&cur.tag_name))
            .bind(p.target_commitish.unwrap_or(&cur.target_commitish))
            .bind(match p.name { Some(n) => n.map(String::from), None => cur.name.clone() })
            .bind(match p.body { Some(n) => n.map(String::from), None => cur.body.clone() })
            .bind(draft)
            .bind(p.prerelease.map(b).unwrap_or(cur.prerelease))
            .bind(now())
            .bind(published)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.release_by_id(id).await
    }

    pub async fn delete_release(&self, id: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM releases WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }
}
