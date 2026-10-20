//! Projects: boards and tables of issues, pull requests, and draft items.

use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::{AssertSqlSafe, Row};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Project {
    pub id: i64,
    pub owner_id: i64,
    pub number: i64,
    pub title: String,
    pub short_description: Option<String>,
    pub readme: Option<String>,
    pub public: i64,
    pub closed: i64,
    pub creator_id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: Option<i64>,
}

impl Project {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

impl Item {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

impl Field {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

impl View {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
}

const PROJECT_COLS: &str =
    "id, owner_id, number, title, short_description, readme, public, closed, creator_id, created_at, updated_at, closed_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Field {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub data_type: String,
    pub options: String,
    pub position: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldOption {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub description: String,
}

impl Field {
    pub fn options(&self) -> Vec<FieldOption> {
        serde_json::from_str(&self.options).unwrap_or_default()
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Item {
    pub id: i64,
    pub project_id: i64,
    pub content_type: String,
    pub content_id: Option<i64>,
    pub draft_title: Option<String>,
    pub draft_body: Option<String>,
    pub position: i64,
    pub archived: i64,
    pub creator_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

const ITEM_COLS: &str =
    "id, project_id, content_type, content_id, draft_title, draft_body, position, archived, creator_id, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct View {
    pub id: i64,
    pub project_id: i64,
    pub number: i64,
    pub name: String,
    pub layout: String,
    pub group_by_field_id: Option<i64>,
    pub filter: Option<String>,
    pub position: i64,
    pub created_at: i64,
    pub settings: String,
}

impl View {
    pub fn settings_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.settings).ok().filter(|v: &serde_json::Value| v.is_object()).unwrap_or_else(|| serde_json::json!({}))
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProjectWorkflow {
    pub id: i64,
    pub project_id: i64,
    pub kind: String,
    pub enabled: i64,
    pub config: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl ProjectWorkflow {
    pub fn config_json(&self) -> serde_json::Value {
        serde_json::from_str(&self.config).ok().filter(|v: &serde_json::Value| v.is_object()).unwrap_or_else(|| serde_json::json!({}))
    }
    /// The status option this workflow sets, if configured.
    pub fn status(&self) -> Option<String> {
        self.config_json().get("status").and_then(|v| v.as_str()).map(String::from).filter(|s| !s.is_empty())
    }
}

/// Every workflow kind, in GitHub's order. `(kind, enabled by default, default status option name)`.
pub const WORKFLOW_KINDS: &[(&str, bool, Option<&str>)] = &[
    ("item_added", true, Some("Todo")),
    ("item_reopened", true, Some("Todo")),
    ("item_closed", true, Some("Done")),
    ("changes_requested", false, Some("In Progress")),
    ("review_approved", false, Some("Done")),
    ("pull_merged", true, Some("Done")),
    ("auto_archive", false, None),
    ("auto_add", false, None),
    ("auto_close", false, Some("Done")),
];

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StatusUpdate {
    pub id: i64,
    pub project_id: i64,
    pub author_id: i64,
    pub status: String,
    pub start_date: Option<String>,
    pub target_date: Option<String>,
    pub body: Option<String>,
    pub created_at: i64,
}

pub const STATUS_UPDATE_STATES: &[&str] = &["INACTIVE", "ON_TRACK", "AT_RISK", "OFF_TRACK", "COMPLETE"];

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ItemHistory {
    pub id: i64,
    pub project_id: i64,
    pub item_id: i64,
    pub kind: String,
    pub field_id: Option<i64>,
    pub value: Option<String>,
    pub prev_value: Option<String>,
    pub created_at: i64,
}

pub const STATUS_OPTIONS: &[(&str, &str, &str)] = &[
    ("Todo", "GRAY", "This item hasn't been started"),
    ("In Progress", "YELLOW", "This is actively being worked on"),
    ("Done", "GREEN", "This has been completed"),
];

impl Db {
    pub async fn create_project(&self, owner: Id, creator: Id, title: &str, description: Option<&str>, public: bool) -> Result<Project> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::field("Project", "title", "missing_field", "title is required"));
        }
        let row = sqlx::query(
            "UPDATE accounts SET next_project_number = next_project_number + 1 WHERE id = $1 RETURNING next_project_number AS n",
        )
        .bind(owner.0)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        let number: i64 = row.try_get::<i64, _>("n").map_err(map_err)? - 1;
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO projects (id, owner_id, number, title, short_description, public, creator_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)")
            .bind(id.0)
            .bind(owner.0)
            .bind(number)
            .bind(title)
            .bind(description)
            .bind(b(public))
            .bind(creator.0)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        // Built-in fields: Title (text, implicit), Status (single select).
        let options: Vec<FieldOption> = STATUS_OPTIONS
            .iter()
            .map(|(n, c, d)| FieldOption {
                id: reef_core::token::random_hex(4),
                name: n.to_string(),
                color: c.to_string(),
                description: d.to_string(),
            })
            .collect();
        let status = self.create_field(id, "Status", "single_select", &options).await?;
        self.create_view(id, "Board", "board", Some(status.id)).await?;
        self.create_view(id, "Table", "table", None).await?;
        self.project_by_id(id).await
    }

    pub async fn project_by_id(&self, id: Id) -> Result<Project> {
        sqlx::query_as::<_, Project>(AssertSqlSafe(format!("SELECT {PROJECT_COLS} FROM projects WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn project_by_number(&self, owner: Id, number: i64) -> Result<Project> {
        sqlx::query_as::<_, Project>(AssertSqlSafe(format!("SELECT {PROJECT_COLS} FROM projects WHERE owner_id = $1 AND number = $2")))
            .bind(owner.0)
            .bind(number)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Projects of an owner. `include_closed` adds closed ones; `public_only` hides private.
    pub async fn projects(&self, owner: Id, include_closed: bool, public_only: bool) -> Result<Vec<Project>> {
        sqlx::query_as::<_, Project>(AssertSqlSafe(format!(
            "SELECT {PROJECT_COLS} FROM projects WHERE owner_id = $1 AND ($2 = 1 OR closed = 0) AND ($3 = 0 OR public = 1) ORDER BY number DESC"
        )))
        .bind(owner.0)
        .bind(b(include_closed))
        .bind(b(public_only))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn update_project(
        &self,
        id: Id,
        title: Option<&str>,
        description: Option<Option<&str>>,
        readme: Option<Option<&str>>,
        public: Option<bool>,
        closed: Option<bool>,
    ) -> Result<Project> {
        let cur = self.project_by_id(id).await?;
        let ts = now();
        let closed_v = closed.unwrap_or(cur.closed != 0);
        sqlx::query("UPDATE projects SET title = $1, short_description = $2, readme = $3, public = $4, closed = $5, closed_at = $6, updated_at = $7 WHERE id = $8")
            .bind(title.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&cur.title))
            .bind(match description { Some(d) => d.map(String::from), None => cur.short_description.clone() })
            .bind(match readme { Some(r) => r.map(String::from), None => cur.readme.clone() })
            .bind(b(public.unwrap_or(cur.public != 0)))
            .bind(b(closed_v))
            .bind(if closed_v { cur.closed_at.or(Some(ts)) } else { None })
            .bind(ts)
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.project_by_id(id).await
    }

    pub async fn delete_project(&self, id: Id) -> Result<()> {
        for t in ["project_item_values"] {
            sqlx::query(AssertSqlSafe(format!("DELETE FROM {t} WHERE item_id IN (SELECT id FROM project_items WHERE project_id = $1)")))
                .bind(id.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        for t in [
            "project_items",
            "project_fields",
            "project_views",
            "project_repositories",
            "project_workflows",
            "project_status_updates",
            "project_item_history",
        ] {
            sqlx::query(AssertSqlSafe(format!("DELETE FROM {t} WHERE project_id = $1")))
                .bind(id.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        sqlx::query("DELETE FROM projects WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- fields -----

    pub async fn create_field(&self, project: Id, name: &str, data_type: &str, options: &[FieldOption]) -> Result<Field> {
        if !matches!(data_type, "single_select" | "text" | "number" | "date" | "iteration") {
            return Err(Error::validation("invalid field type"));
        }
        let id = reef_core::id::next();
        let pos: i64 = sqlx::query("SELECT COALESCE(MAX(position), 0) + 1 AS p FROM project_fields WHERE project_id = $1")
            .bind(project.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("p")
            .map_err(map_err)?;
        sqlx::query("INSERT INTO project_fields (id, project_id, name, data_type, options, position, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(id.0)
            .bind(project.0)
            .bind(name.trim())
            .bind(data_type)
            .bind(serde_json::to_string(options).unwrap_or_else(|_| "[]".into()))
            .bind(pos)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.field_by_id(id).await
    }

    pub async fn field_by_id(&self, id: Id) -> Result<Field> {
        sqlx::query_as::<_, Field>(
            "SELECT id, project_id, name, data_type, options, position, created_at FROM project_fields WHERE id = $1",
        )
        .bind(id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn fields(&self, project: Id) -> Result<Vec<Field>> {
        sqlx::query_as::<_, Field>("SELECT id, project_id, name, data_type, options, position, created_at FROM project_fields WHERE project_id = $1 ORDER BY position, id")
            .bind(project.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn update_field(&self, id: Id, name: Option<&str>, options: Option<&[FieldOption]>) -> Result<Field> {
        let cur = self.field_by_id(id).await?;
        sqlx::query("UPDATE project_fields SET name = $1, options = $2 WHERE id = $3")
            .bind(name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&cur.name))
            .bind(match options {
                Some(o) => serde_json::to_string(o).unwrap_or_else(|_| "[]".into()),
                None => cur.options.clone(),
            })
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.field_by_id(id).await
    }

    pub async fn delete_field(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM project_item_values WHERE field_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("UPDATE project_views SET group_by_field_id = NULL WHERE group_by_field_id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query("DELETE FROM project_fields WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- items -----

    pub async fn add_item(
        &self,
        project: Id,
        content_type: &str,
        content_id: Option<i64>,
        draft_title: Option<&str>,
        draft_body: Option<&str>,
        creator: Option<Id>,
    ) -> Result<Item> {
        let id = reef_core::id::next();
        let ts = now();
        let pos: i64 = sqlx::query("SELECT COALESCE(MAX(position), 0) + 1024 AS p FROM project_items WHERE project_id = $1")
            .bind(project.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?
            .try_get("p")
            .map_err(map_err)?;
        let r = sqlx::query("INSERT INTO project_items (id, project_id, content_type, content_id, draft_title, draft_body, position, creator_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)")
            .bind(id.0)
            .bind(project.0)
            .bind(content_type)
            .bind(content_id)
            .bind(draft_title)
            .bind(draft_body)
            .bind(pos)
            .bind(creator.map(|c| c.0))
            .bind(ts)
            .execute(&self.pool)
            .await;
        match r.map_err(map_err) {
            Ok(_) => {}
            Err(Error::Conflict(_)) => {
                // Already in the project: return the existing item.
                return sqlx::query_as::<_, Item>(AssertSqlSafe(format!(
                    "SELECT {ITEM_COLS} FROM project_items WHERE project_id = $1 AND content_type = $2 AND content_id = $3"
                )))
                .bind(project.0)
                .bind(content_type)
                .bind(content_id)
                .fetch_one(&self.pool)
                .await
                .map_err(map_err);
            }
            Err(e) => return Err(e),
        }
        sqlx::query("UPDATE projects SET updated_at = $1 WHERE id = $2")
            .bind(ts)
            .bind(project.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        let _ = self.record_item_history(project, id, "added", None, None, None).await;
        self.item_by_id(id).await
    }

    pub async fn item_by_id(&self, id: Id) -> Result<Item> {
        sqlx::query_as::<_, Item>(AssertSqlSafe(format!("SELECT {ITEM_COLS} FROM project_items WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn items(&self, project: Id, include_archived: bool) -> Result<Vec<Item>> {
        sqlx::query_as::<_, Item>(AssertSqlSafe(format!(
            "SELECT {ITEM_COLS} FROM project_items WHERE project_id = $1 AND ($2 = 1 OR archived = 0) ORDER BY position, id"
        )))
        .bind(project.0)
        .bind(b(include_archived))
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Projects containing an issue.
    pub async fn projects_of_issue(&self, issue: Id) -> Result<Vec<(Project, Item)>> {
        let items = sqlx::query_as::<_, Item>(AssertSqlSafe(format!(
            "SELECT {ITEM_COLS} FROM project_items WHERE content_id = $1 AND content_type IN ('issue', 'pull') AND archived = 0"
        )))
        .bind(issue.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out = Vec::new();
        for it in items {
            if let Ok(p) = self.project_by_id(Id(it.project_id)).await {
                out.push((p, it));
            }
        }
        Ok(out)
    }

    /// Items (in any project, archived included) whose content is the given issue or pull.
    pub async fn items_for_content(&self, issue: Id) -> Result<Vec<Item>> {
        sqlx::query_as::<_, Item>(AssertSqlSafe(format!(
            "SELECT {ITEM_COLS} FROM project_items WHERE content_id = $1 AND content_type IN ('issue', 'pull') ORDER BY created_at"
        )))
        .bind(issue.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// The project's `Status` field (the built-in single select), if it still exists.
    pub async fn status_field(&self, project: Id) -> Result<Option<Field>> {
        Ok(self.fields(project).await?.into_iter().find(|f| f.data_type == "single_select" && f.name.eq_ignore_ascii_case("Status")))
    }

    /// Workflow "item added to project": Status = the configured option (first option by
    /// default) when the item has no status yet.
    pub async fn default_status(&self, project: Id, item: Id) -> Result<()> {
        let Some(f) = self.status_field(project).await? else { return Ok(()) };
        let has = self.item_values(item).await?.iter().any(|(fid, v)| *fid == f.id && v.is_some());
        if has {
            return Ok(());
        }
        let wf = self.project_workflow(project, "item_added").await?;
        let enabled = wf.as_ref().map(|w| w.enabled != 0).unwrap_or(true);
        if !enabled {
            return Ok(());
        }
        let opts = f.options();
        let target =
            wf.and_then(|w| w.status()).filter(|id| opts.iter().any(|o| o.id == *id)).or_else(|| opts.first().map(|o| o.id.clone()));
        if let Some(id) = target {
            self.set_item_value(item, f.id(), Some(&id)).await?;
        }
        Ok(())
    }

    /// Workflows "item closed", "pull request merged" and "item reopened", applied to every
    /// project the issue or pull request is in. Each project's configured option wins; when a
    /// project has no row the GitHub defaults apply (Done / Todo, only if such an option exists).
    pub async fn status_automation(&self, issue: Id, closed: bool) -> Result<()> {
        let merged: bool = if closed {
            sqlx::query("SELECT merged FROM pulls WHERE issue_id = $1")
                .bind(issue.0)
                .fetch_optional(&self.pool)
                .await
                .map_err(map_err)?
                .map(|r| r.get::<i64, _>("merged") != 0)
                .unwrap_or(false)
        } else {
            false
        };
        let kind = if !closed {
            "item_reopened"
        } else if merged {
            "pull_merged"
        } else {
            "item_closed"
        };
        for (p, it) in self.projects_of_issue(issue).await? {
            self.apply_status_workflow(&p, &it, kind).await?;
        }
        Ok(())
    }

    /// Apply one status-setting workflow (`item_closed`, `pull_merged`, `item_reopened`,
    /// `changes_requested`, `review_approved`) to one item.
    pub async fn apply_status_workflow(&self, p: &Project, it: &Item, kind: &str) -> Result<()> {
        let Some(f) = self.status_field(p.id()).await? else { return Ok(()) };
        let (_, default_on, default_name) = WORKFLOW_KINDS.iter().find(|(k, _, _)| *k == kind).copied().unwrap_or((kind, false, None));
        let wf = self.project_workflow(p.id(), kind).await?;
        let enabled = wf.as_ref().map(|w| w.enabled != 0).unwrap_or(default_on);
        if !enabled {
            return Ok(());
        }
        let opts = f.options();
        let target = wf
            .and_then(|w| w.status())
            .filter(|id| opts.iter().any(|o| o.id == *id))
            .or_else(|| default_name.and_then(|n| opts.iter().find(|o| o.name.eq_ignore_ascii_case(n)).map(|o| o.id.clone())));
        if let Some(id) = target {
            self.set_item_value(it.id(), f.id(), Some(&id)).await?;
        }
        Ok(())
    }

    /// Review workflows: `state` is `changes_requested` or `approved`.
    pub async fn review_automation(&self, issue: Id, state: &str) -> Result<()> {
        let kind = match state {
            "changes_requested" => "changes_requested",
            "approved" => "review_approved",
            _ => return Ok(()),
        };
        for (p, it) in self.projects_of_issue(issue).await? {
            self.apply_status_workflow(&p, &it, kind).await?;
        }
        Ok(())
    }

    // ----- workflows -----

    pub async fn project_workflows(&self, project: Id) -> Result<Vec<ProjectWorkflow>> {
        sqlx::query_as::<_, ProjectWorkflow>(
            "SELECT id, project_id, kind, enabled, config, created_at, updated_at FROM project_workflows WHERE project_id = $1",
        )
        .bind(project.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn project_workflow(&self, project: Id, kind: &str) -> Result<Option<ProjectWorkflow>> {
        sqlx::query_as::<_, ProjectWorkflow>("SELECT id, project_id, kind, enabled, config, created_at, updated_at FROM project_workflows WHERE project_id = $1 AND kind = $2")
            .bind(project.0)
            .bind(kind)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn set_project_workflow(
        &self,
        project: Id,
        kind: &str,
        enabled: bool,
        config: &serde_json::Value,
    ) -> Result<ProjectWorkflow> {
        if !WORKFLOW_KINDS.iter().any(|(k, _, _)| *k == kind) {
            return Err(Error::validation(format!("unknown workflow {kind}")));
        }
        let ts = now();
        sqlx::query("INSERT INTO project_workflows (id, project_id, kind, enabled, config, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT (project_id, kind) DO UPDATE SET enabled = $4, config = $5, updated_at = $6")
            .bind(reef_core::id::next().0)
            .bind(project.0)
            .bind(kind)
            .bind(b(enabled))
            .bind(config.to_string())
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.project_workflow(project, kind).await?.ok_or(Error::NotFound)
    }

    /// Projects that have a given workflow enabled (for the scheduler).
    pub async fn projects_with_workflow(&self, kind: &str) -> Result<Vec<(Project, ProjectWorkflow)>> {
        let rows = sqlx::query_as::<_, ProjectWorkflow>(
            "SELECT id, project_id, kind, enabled, config, created_at, updated_at FROM project_workflows WHERE kind = $1 AND enabled = 1",
        )
        .bind(kind)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)?;
        let mut out = Vec::new();
        for w in rows {
            if let Ok(p) = self.project_by_id(Id(w.project_id)).await
                && p.closed == 0
            {
                out.push((p, w));
            }
        }
        Ok(out)
    }

    // ----- status updates -----

    pub async fn status_updates(&self, project: Id) -> Result<Vec<StatusUpdate>> {
        sqlx::query_as::<_, StatusUpdate>("SELECT id, project_id, author_id, status, start_date, target_date, body, created_at FROM project_status_updates WHERE project_id = $1 ORDER BY created_at DESC")
            .bind(project.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn latest_status_update(&self, project: Id) -> Result<Option<StatusUpdate>> {
        sqlx::query_as::<_, StatusUpdate>("SELECT id, project_id, author_id, status, start_date, target_date, body, created_at FROM project_status_updates WHERE project_id = $1 ORDER BY created_at DESC LIMIT 1")
            .bind(project.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn create_status_update(
        &self,
        project: Id,
        author: Id,
        status: &str,
        start_date: Option<&str>,
        target_date: Option<&str>,
        body: Option<&str>,
    ) -> Result<StatusUpdate> {
        if !STATUS_UPDATE_STATES.contains(&status) {
            return Err(Error::validation("status must be INACTIVE, ON_TRACK, AT_RISK, OFF_TRACK or COMPLETE"));
        }
        let id = reef_core::id::next();
        sqlx::query("INSERT INTO project_status_updates (id, project_id, author_id, status, start_date, target_date, body, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(id.0)
            .bind(project.0)
            .bind(author.0)
            .bind(status)
            .bind(start_date.map(str::trim).filter(|d| !d.is_empty()))
            .bind(target_date.map(str::trim).filter(|d| !d.is_empty()))
            .bind(body.map(str::trim).filter(|b| !b.is_empty()))
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query("UPDATE projects SET updated_at = $1 WHERE id = $2")
            .bind(now())
            .bind(project.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query_as::<_, StatusUpdate>(
            "SELECT id, project_id, author_id, status, start_date, target_date, body, created_at FROM project_status_updates WHERE id = $1",
        )
        .bind(id.0)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn delete_status_update(&self, project: Id, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM project_status_updates WHERE id = $1 AND project_id = $2")
            .bind(id.0)
            .bind(project.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- history -----

    pub async fn record_item_history(
        &self,
        project: Id,
        item: Id,
        kind: &str,
        field: Option<Id>,
        value: Option<&str>,
        prev: Option<&str>,
    ) -> Result<()> {
        sqlx::query("INSERT INTO project_item_history (id, project_id, item_id, kind, field_id, value, prev_value, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
            .bind(reef_core::id::next().0)
            .bind(project.0)
            .bind(item.0)
            .bind(kind)
            .bind(field.map(|f| f.0))
            .bind(value)
            .bind(prev)
            .bind(now())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn item_history(&self, project: Id, since: i64) -> Result<Vec<ItemHistory>> {
        sqlx::query_as::<_, ItemHistory>("SELECT id, project_id, item_id, kind, field_id, value, prev_value, created_at FROM project_item_history WHERE project_id = $1 AND created_at >= $2 ORDER BY created_at ASC")
            .bind(project.0)
            .bind(since)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Archived items of a project, newest first.
    pub async fn archived_items(&self, project: Id) -> Result<Vec<Item>> {
        sqlx::query_as::<_, Item>(AssertSqlSafe(format!(
            "SELECT {ITEM_COLS} FROM project_items WHERE project_id = $1 AND archived = 1 ORDER BY updated_at DESC"
        )))
        .bind(project.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Turn a draft into a real issue's item (the issue was just created from the draft).
    pub async fn convert_draft(&self, id: Id, issue: Id, is_pull: bool) -> Result<Item> {
        sqlx::query("UPDATE project_items SET content_type = $1, content_id = $2, draft_title = NULL, draft_body = NULL, updated_at = $3 WHERE id = $4 AND content_type = 'draft'")
            .bind(if is_pull { "pull" } else { "issue" })
            .bind(issue.0)
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.item_by_id(id).await
    }

    pub async fn update_draft(&self, id: Id, title: Option<&str>, body: Option<Option<&str>>) -> Result<Item> {
        let cur = self.item_by_id(id).await?;
        sqlx::query("UPDATE project_items SET draft_title = $1, draft_body = $2, updated_at = $3 WHERE id = $4")
            .bind(title.map(str::trim).filter(|t| !t.is_empty()).map(String::from).or(cur.draft_title.clone()))
            .bind(match body {
                Some(b) => b.map(String::from),
                None => cur.draft_body.clone(),
            })
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.item_by_id(id).await
    }

    pub async fn set_item_archived(&self, id: Id, archived: bool) -> Result<()> {
        let it = self.item_by_id(id).await?;
        sqlx::query("UPDATE project_items SET archived = $1, updated_at = $2 WHERE id = $3")
            .bind(b(archived))
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if (it.archived != 0) != archived {
            let _ =
                self.record_item_history(Id(it.project_id), id, if archived { "archived" } else { "unarchived" }, None, None, None).await;
        }
        Ok(())
    }

    pub async fn delete_item(&self, id: Id) -> Result<()> {
        if let Ok(it) = self.item_by_id(id).await {
            let _ = self.record_item_history(Id(it.project_id), id, "removed", None, None, None).await;
        }
        sqlx::query("DELETE FROM project_item_values WHERE item_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("DELETE FROM project_items WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    /// Move an item: new position (sortable) within the project.
    pub async fn move_item(&self, id: Id, after: Option<Id>) -> Result<()> {
        let it = self.item_by_id(id).await?;
        let items = self.items(Id(it.project_id), true).await?;
        let pos = match after {
            None => items.iter().filter(|x| x.id != it.id).map(|x| x.position).min().unwrap_or(1024) - 1024,
            Some(a) => {
                let idx = items.iter().position(|x| x.id == a.0).ok_or(Error::NotFound)?;
                let prev = items[idx].position;
                let next = items.iter().skip(idx + 1).find(|x| x.id != it.id).map(|x| x.position);
                match next {
                    Some(n) if n - prev > 1 => (prev + n) / 2,
                    Some(_) => {
                        // No room: renumber everything with a stride.
                        let mut p = 0;
                        for x in &items {
                            if x.id == it.id {
                                continue;
                            }
                            p += 1024;
                            sqlx::query("UPDATE project_items SET position = $1 WHERE id = $2")
                                .bind(p)
                                .bind(x.id)
                                .execute(&self.pool)
                                .await
                                .map_err(map_err)?;
                            if x.id == a.0 {
                                p += 1024;
                                sqlx::query("UPDATE project_items SET position = $1 WHERE id = $2")
                                    .bind(p)
                                    .bind(it.id)
                                    .execute(&self.pool)
                                    .await
                                    .map_err(map_err)?;
                            }
                        }
                        return Ok(());
                    }
                    None => prev + 1024,
                }
            }
        };
        sqlx::query("UPDATE project_items SET position = $1, updated_at = $2 WHERE id = $3")
            .bind(pos)
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- values -----

    pub async fn item_values(&self, item: Id) -> Result<Vec<(i64, Option<String>)>> {
        let rows = sqlx::query("SELECT field_id, value FROM project_item_values WHERE item_id = $1")
            .bind(item.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows.iter().map(|r| (r.get("field_id"), r.get("value"))).collect())
    }

    /// All values of a project as (item_id, field_id, value).
    pub async fn project_values(&self, project: Id) -> Result<Vec<(i64, i64, Option<String>)>> {
        let rows = sqlx::query("SELECT v.item_id, v.field_id, v.value FROM project_item_values v JOIN project_items i ON i.id = v.item_id WHERE i.project_id = $1")
            .bind(project.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows.iter().map(|r| (r.get("item_id"), r.get("field_id"), r.get("value"))).collect())
    }

    pub async fn set_item_value(&self, item: Id, field: Id, value: Option<&str>) -> Result<()> {
        let before = self.item_values(item).await?.into_iter().find(|(f, _)| *f == field.0).and_then(|(_, v)| v);
        if before.as_deref() != value
            && let Ok(it) = self.item_by_id(item).await
        {
            let _ = self.record_item_history(Id(it.project_id), item, "value", Some(field), value, before.as_deref()).await;
        }
        match value {
            Some(v) => {
                sqlx::query("INSERT INTO project_item_values (item_id, field_id, value) VALUES ($1, $2, $3) ON CONFLICT (item_id, field_id) DO UPDATE SET value = $3")
                    .bind(item.0)
                    .bind(field.0)
                    .bind(v)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
            }
            None => {
                sqlx::query("DELETE FROM project_item_values WHERE item_id = $1 AND field_id = $2")
                    .bind(item.0)
                    .bind(field.0)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
            }
        }
        sqlx::query("UPDATE project_items SET updated_at = $1 WHERE id = $2")
            .bind(now())
            .bind(item.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- views -----

    pub async fn create_view(&self, project: Id, name: &str, layout: &str, group_by: Option<i64>) -> Result<View> {
        self.create_view_with(project, name, layout, group_by, &serde_json::json!({"column_field_id": group_by})).await
    }

    pub async fn create_view_with(
        &self,
        project: Id,
        name: &str,
        layout: &str,
        group_by: Option<i64>,
        settings: &serde_json::Value,
    ) -> Result<View> {
        let id = reef_core::id::next();
        let row = sqlx::query(
            "SELECT COALESCE(MAX(number), 0) + 1 AS n, COALESCE(MAX(position), 0) + 1 AS p FROM project_views WHERE project_id = $1",
        )
        .bind(project.0)
        .fetch_one(&self.pool)
        .await
        .map_err(map_err)?;
        let n: i64 = row.try_get("n").map_err(map_err)?;
        let p: i64 = row.try_get("p").map_err(map_err)?;
        sqlx::query("INSERT INTO project_views (id, project_id, number, name, layout, group_by_field_id, position, created_at, settings) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
            .bind(id.0)
            .bind(project.0)
            .bind(n)
            .bind(name)
            .bind(if layout == "table" { "table" } else if layout == "roadmap" { "roadmap" } else { "board" })
            .bind(group_by)
            .bind(p)
            .bind(now())
            .bind(settings.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.view_by_id(id).await
    }

    pub async fn view_by_id(&self, id: Id) -> Result<View> {
        sqlx::query_as::<_, View>("SELECT id, project_id, number, name, layout, group_by_field_id, filter, position, created_at, settings FROM project_views WHERE id = $1").bind(id.0).fetch_one(&self.pool).await.map_err(map_err)
    }

    pub async fn views(&self, project: Id) -> Result<Vec<View>> {
        sqlx::query_as::<_, View>("SELECT id, project_id, number, name, layout, group_by_field_id, filter, position, created_at, settings FROM project_views WHERE project_id = $1 ORDER BY position, number")
            .bind(project.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// `settings` is merged key-by-key into the stored document (null removes a key).
    pub async fn update_view(
        &self,
        id: Id,
        name: Option<&str>,
        layout: Option<&str>,
        group_by: Option<Option<i64>>,
        filter: Option<Option<&str>>,
        settings: Option<&serde_json::Value>,
    ) -> Result<View> {
        let cur = self.view_by_id(id).await?;
        let mut doc = cur.settings_json();
        if let (Some(serde_json::Value::Object(patch)), Some(d)) = (settings, doc.as_object_mut()) {
            for (k, v) in patch {
                if v.is_null() {
                    d.remove(k);
                } else {
                    d.insert(k.clone(), v.clone());
                }
            }
        }
        sqlx::query("UPDATE project_views SET name = $1, layout = $2, group_by_field_id = $3, filter = $4, settings = $6 WHERE id = $5")
            .bind(name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&cur.name))
            .bind(layout.unwrap_or(&cur.layout))
            .bind(match group_by {
                Some(g) => g,
                None => cur.group_by_field_id,
            })
            .bind(match filter {
                Some(f) => f.map(String::from),
                None => cur.filter.clone(),
            })
            .bind(id.0)
            .bind(doc.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.view_by_id(id).await
    }

    pub async fn delete_view(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM project_views WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- linked repositories -----

    pub async fn link_repo(&self, project: Id, repo: Id) -> Result<()> {
        let _ = sqlx::query("INSERT INTO project_repositories (project_id, repo_id) VALUES ($1, $2)")
            .bind(project.0)
            .bind(repo.0)
            .execute(&self.pool)
            .await;
        Ok(())
    }

    pub async fn unlink_repo(&self, project: Id, repo: Id) -> Result<()> {
        sqlx::query("DELETE FROM project_repositories WHERE project_id = $1 AND repo_id = $2")
            .bind(project.0)
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn projects_of_repo(&self, repo: Id) -> Result<Vec<Project>> {
        sqlx::query_as::<_, Project>(AssertSqlSafe(format!(
            "SELECT p.{} FROM projects p JOIN project_repositories pr ON pr.project_id = p.id WHERE pr.repo_id = $1 ORDER BY p.number DESC",
            PROJECT_COLS.replace(", ", ", p.")
        )))
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn linked_repos(&self, project: Id) -> Result<Vec<i64>> {
        let rows = sqlx::query("SELECT repo_id FROM project_repositories WHERE project_id = $1")
            .bind(project.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(rows.iter().map(|r| r.get("repo_id")).collect())
    }
}
