//! Issues, labels, milestones, comments, timeline events, and reactions.
//! Pull requests are issues with `is_pull = 1`; their extra data lives in `pulls`.

use crate::accounts::Account;
use crate::{Db, b, map_err, now};
use reef_core::{Error, Id, Result};
use sqlx::{AssertSqlSafe, Row};
use std::collections::HashMap;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Issue {
    pub id: i64,
    pub repo_id: i64,
    pub number: i64,
    pub is_pull: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub state_reason: Option<String>,
    pub author_id: i64,
    pub milestone_id: Option<i64>,
    pub locked: i64,
    pub lock_reason: Option<String>,
    pub comments_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: Option<i64>,
    pub closed_by_id: Option<i64>,
}

impl Issue {
    pub fn id(&self) -> Id {
        Id(self.id)
    }
    pub fn is_open(&self) -> bool {
        self.state == "open"
    }
    pub fn is_pull(&self) -> bool {
        self.is_pull != 0
    }
}

pub const ISSUE_COLS: &str = "id, repo_id, number, is_pull, title, body, state, state_reason, author_id, milestone_id, locked, lock_reason, comments_count, created_at, updated_at, closed_at, closed_by_id";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Label {
    pub id: i64,
    pub repo_id: i64,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    pub is_default: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

const LABEL_COLS: &str = "id, repo_id, name, color, description, is_default, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Milestone {
    pub id: i64,
    pub repo_id: i64,
    pub number: i64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub due_on: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub closed_at: Option<i64>,
}

const MILESTONE_COLS: &str = "id, repo_id, number, title, description, state, due_on, created_at, updated_at, closed_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Comment {
    pub id: i64,
    pub issue_id: i64,
    pub repo_id: i64,
    pub author_id: i64,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const COMMENT_COLS: &str = "id, issue_id, repo_id, author_id, body, created_at, updated_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct IssueEvent {
    pub id: i64,
    pub issue_id: i64,
    pub repo_id: i64,
    pub actor_id: Option<i64>,
    pub event: String,
    pub payload: String,
    pub created_at: i64,
}

impl IssueEvent {
    pub fn payload(&self) -> serde_json::Value {
        serde_json::from_str(&self.payload).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Reaction {
    pub id: i64,
    pub subject_type: String,
    pub subject_id: i64,
    pub user_id: i64,
    pub content: String,
    pub created_at: i64,
}

pub const REACTIONS: &[&str] = &["+1", "-1", "laugh", "hooray", "confused", "heart", "rocket", "eyes"];

/// An issue with everything the API representations need.
#[derive(Debug, Clone)]
pub struct IssueFull {
    pub issue: Issue,
    pub author: Account,
    pub assignees: Vec<Account>,
    pub labels: Vec<Label>,
    pub milestone: Option<Milestone>,
    pub closed_by: Option<Account>,
}

#[derive(Debug, Clone, Default)]
pub struct IssueFilter {
    /// "open" | "closed" | "all"
    pub state: String,
    pub is_pull: Option<bool>,
    pub labels: Vec<String>,
    /// login, "none", or "*"
    pub assignee: Option<String>,
    pub creator: Option<String>,
    pub mentioned: Option<String>,
    /// number, "none", or "*"
    pub milestone: Option<String>,
    pub since: Option<i64>,
    /// "created" | "updated" | "comments"
    pub sort: String,
    pub ascending: bool,
    /// Free-text terms matched against title and body.
    pub terms: Vec<String>,
    /// Pull requests with a pending review request for this login.
    pub review_requested: Option<String>,
    /// Pull requests reviewed by this login.
    pub reviewed_by: Option<String>,
    /// Any of: author, assignee, mentioned, commenter, review requested.
    pub involves: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewIssue {
    pub title: String,
    pub body: Option<String>,
    pub is_pull: bool,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct IssuePatch {
    pub title: Option<String>,
    pub body: Option<Option<String>>,
    /// Some("open") | Some("closed")
    pub state: Option<String>,
    pub state_reason: Option<Option<String>>,
    /// Some(None) clears the milestone.
    pub milestone: Option<Option<i64>>,
    pub labels: Option<Vec<String>>,
    pub assignees: Option<Vec<String>>,
    /// When closing: the merge commit and pull request that closed the issue (for the
    /// "closed this as completed in #9 via abc1234" timeline entry).
    pub close_commit: Option<String>,
    pub close_pull: Option<i64>,
}

pub const DEFAULT_LABELS: &[(&str, &str, &str)] = &[
    ("bug", "d73a4a", "Something isn't working"),
    ("documentation", "0075ca", "Improvements or additions to documentation"),
    ("duplicate", "cfd3d7", "This issue or pull request already exists"),
    ("enhancement", "a2eeef", "New feature or request"),
    ("good first issue", "7057ff", "Good for newcomers"),
    ("help wanted", "008672", "Extra attention is needed"),
    ("invalid", "e4e669", "This doesn't seem right"),
    ("question", "d876e3", "Further information is requested"),
    ("wontfix", "ffffff", "This will not be worked on"),
];

fn valid_color(c: &str) -> Option<String> {
    let c = c.trim().trim_start_matches('#').to_lowercase();
    if c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()) { Some(c) } else { None }
}

impl Db {
    // ----- labels -----

    pub async fn ensure_default_labels(&self, repo: Id) -> Result<()> {
        for (name, color, desc) in DEFAULT_LABELS {
            let _ = self.create_label(repo, name, color, Some(desc), true).await;
        }
        Ok(())
    }

    pub async fn create_label(&self, repo: Id, name: &str, color: &str, description: Option<&str>, is_default: bool) -> Result<Label> {
        let name = name.trim();
        if name.is_empty() || name.len() > 50 {
            return Err(Error::field("Label", "name", "invalid", "name is invalid"));
        }
        let color = valid_color(color).ok_or_else(|| Error::field("Label", "color", "invalid", "color must be 6 hex digits"))?;
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO labels (id, repo_id, name, name_lower, color, description, is_default, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)")
            .bind(id.0)
            .bind(repo.0)
            .bind(name)
            .bind(name.to_lowercase())
            .bind(&color)
            .bind(description)
            .bind(b(is_default))
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::Conflict(_) => Error::field("Label", "name", "already_exists", "label already exists"),
                o => o,
            })?;
        self.label_by_id(id).await
    }

    pub async fn label_by_id(&self, id: Id) -> Result<Label> {
        sqlx::query_as::<_, Label>(AssertSqlSafe(format!("SELECT {LABEL_COLS} FROM labels WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn label_by_name(&self, repo: Id, name: &str) -> Result<Label> {
        sqlx::query_as::<_, Label>(AssertSqlSafe(format!("SELECT {LABEL_COLS} FROM labels WHERE repo_id = $1 AND name_lower = $2")))
            .bind(repo.0)
            .bind(name.trim().to_lowercase())
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn labels(&self, repo: Id) -> Result<Vec<Label>> {
        sqlx::query_as::<_, Label>(AssertSqlSafe(format!("SELECT {LABEL_COLS} FROM labels WHERE repo_id = $1 ORDER BY name_lower")))
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn update_label(&self, id: Id, name: Option<&str>, color: Option<&str>, description: Option<Option<&str>>) -> Result<Label> {
        let cur = self.label_by_id(id).await?;
        let name = name.map(str::trim).unwrap_or(&cur.name).to_string();
        let color = match color {
            Some(c) => valid_color(c).ok_or_else(|| Error::field("Label", "color", "invalid", "color must be 6 hex digits"))?,
            None => cur.color.clone(),
        };
        let description = match description {
            Some(d) => d.map(String::from),
            None => cur.description.clone(),
        };
        sqlx::query("UPDATE labels SET name = $1, name_lower = $2, color = $3, description = $4, updated_at = $5 WHERE id = $6")
            .bind(&name)
            .bind(name.to_lowercase())
            .bind(color)
            .bind(description)
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(|e| match map_err(e) {
                Error::Conflict(_) => Error::field("Label", "name", "already_exists", "label already exists"),
                o => o,
            })?;
        self.label_by_id(id).await
    }

    pub async fn delete_label(&self, id: Id) -> Result<()> {
        sqlx::query("DELETE FROM issue_labels WHERE label_id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        let r = sqlx::query("DELETE FROM labels WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    // ----- milestones -----

    pub async fn create_milestone(
        &self,
        repo: Id,
        title: &str,
        description: Option<&str>,
        due_on: Option<i64>,
        state: &str,
    ) -> Result<Milestone> {
        if title.trim().is_empty() {
            return Err(Error::field("Milestone", "title", "missing_field", "title is required"));
        }
        let number = self.next_number(repo, "next_milestone_number").await?;
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query("INSERT INTO milestones (id, repo_id, number, title, description, state, due_on, created_at, updated_at, closed_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $9)")
            .bind(id.0)
            .bind(repo.0)
            .bind(number)
            .bind(title.trim())
            .bind(description)
            .bind(if state == "closed" { "closed" } else { "open" })
            .bind(due_on)
            .bind(ts)
            .bind(if state == "closed" { Some(ts) } else { None })
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.milestone_by_id(id).await
    }

    pub async fn milestone_by_id(&self, id: Id) -> Result<Milestone> {
        sqlx::query_as::<_, Milestone>(AssertSqlSafe(format!("SELECT {MILESTONE_COLS} FROM milestones WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn milestone_by_number(&self, repo: Id, number: i64) -> Result<Milestone> {
        sqlx::query_as::<_, Milestone>(AssertSqlSafe(format!("SELECT {MILESTONE_COLS} FROM milestones WHERE repo_id = $1 AND number = $2")))
            .bind(repo.0)
            .bind(number)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn milestone_by_title(&self, repo: Id, title: &str) -> Result<Milestone> {
        sqlx::query_as::<_, Milestone>(AssertSqlSafe(format!("SELECT {MILESTONE_COLS} FROM milestones WHERE repo_id = $1 AND title = $2")))
            .bind(repo.0)
            .bind(title)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    /// `state`: "open" | "closed" | "all".
    pub async fn milestones(&self, repo: Id, state: &str) -> Result<Vec<Milestone>> {
        sqlx::query_as::<_, Milestone>(AssertSqlSafe(format!(
            "SELECT {MILESTONE_COLS} FROM milestones WHERE repo_id = $1 AND ($2 = 'all' OR state = $2) ORDER BY due_on, number"
        )))
        .bind(repo.0)
        .bind(state)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn update_milestone(
        &self,
        id: Id,
        title: Option<&str>,
        description: Option<Option<&str>>,
        due_on: Option<Option<i64>>,
        state: Option<&str>,
    ) -> Result<Milestone> {
        let cur = self.milestone_by_id(id).await?;
        let ts = now();
        let state = state.unwrap_or(&cur.state).to_string();
        let closed_at = if state == "closed" { cur.closed_at.or(Some(ts)) } else { None };
        sqlx::query(
            "UPDATE milestones SET title = $1, description = $2, due_on = $3, state = $4, closed_at = $5, updated_at = $6 WHERE id = $7",
        )
        .bind(title.map(str::trim).unwrap_or(&cur.title))
        .bind(match description {
            Some(d) => d.map(String::from),
            None => cur.description.clone(),
        })
        .bind(match due_on {
            Some(d) => d,
            None => cur.due_on,
        })
        .bind(&state)
        .bind(closed_at)
        .bind(ts)
        .bind(id.0)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        self.milestone_by_id(id).await
    }

    pub async fn delete_milestone(&self, id: Id) -> Result<()> {
        sqlx::query("UPDATE issues SET milestone_id = NULL WHERE milestone_id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        let r = sqlx::query("DELETE FROM milestones WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    /// (open, closed) issue counts of a milestone.
    pub async fn milestone_counts(&self, id: Id) -> Result<(i64, i64)> {
        let row = sqlx::query("SELECT SUM(CASE WHEN state = 'open' THEN 1 ELSE 0 END) AS o, SUM(CASE WHEN state = 'closed' THEN 1 ELSE 0 END) AS c FROM issues WHERE milestone_id = $1")
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        Ok((
            row.try_get::<Option<i64>, _>("o").map_err(map_err)?.unwrap_or(0),
            row.try_get::<Option<i64>, _>("c").map_err(map_err)?.unwrap_or(0),
        ))
    }

    // ----- numbering -----

    /// Atomically allocate the next issue or milestone number of a repository.
    async fn next_number(&self, repo: Id, column: &str) -> Result<i64> {
        let sql = format!("UPDATE repositories SET {column} = {column} + 1 WHERE id = $1 RETURNING {column} AS n");
        let row = sqlx::query(AssertSqlSafe(sql)).bind(repo.0).fetch_one(&self.pool).await.map_err(map_err)?;
        let n: i64 = row.try_get("n").map_err(map_err)?;
        Ok(n - 1)
    }

    // ----- issues -----

    pub async fn create_issue(&self, repo: Id, author: &Account, n: NewIssue) -> Result<IssueFull> {
        let title = n.title.trim();
        if title.is_empty() {
            return Err(Error::field("Issue", "title", "missing_field", "title is required"));
        }
        let number = self.next_number(repo, "next_issue_number").await?;
        let id = reef_core::id::next();
        let ts = now();
        sqlx::query(
            "INSERT INTO issues (id, repo_id, number, is_pull, title, body, state, author_id, milestone_id, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, 'open', $7, $8, $9, $9)",
        )
        .bind(id.0)
        .bind(repo.0)
        .bind(number)
        .bind(b(n.is_pull))
        .bind(title)
        .bind(n.body.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(author.id)
        .bind(n.milestone)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(map_err)?;
        if let Some(m) = n.milestone {
            self.add_issue_event(id, repo, Some(author.id()), "milestoned", &serde_json::json!({"milestone_id": m})).await?;
        }
        if !n.labels.is_empty() {
            self.set_issue_labels(id, repo, author, &n.labels).await?;
        }
        if !n.assignees.is_empty() {
            self.set_issue_assignees(id, repo, author, &n.assignees).await?;
        }
        self.record_mentions_by(id, repo, n.body.as_deref().unwrap_or(""), Some(author.id())).await?;
        self.recount_open_issues(repo).await?;
        self.issue_full_by_id(id).await
    }

    pub async fn issue_by_id(&self, id: Id) -> Result<Issue> {
        sqlx::query_as::<_, Issue>(AssertSqlSafe(format!("SELECT {ISSUE_COLS} FROM issues WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn issue_by_number(&self, repo: Id, number: i64) -> Result<Issue> {
        sqlx::query_as::<_, Issue>(AssertSqlSafe(format!("SELECT {ISSUE_COLS} FROM issues WHERE repo_id = $1 AND number = $2")))
            .bind(repo.0)
            .bind(number)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn issue_full_by_id(&self, id: Id) -> Result<IssueFull> {
        let i = self.issue_by_id(id).await?;
        Ok(self.hydrate_issues(vec![i]).await?.pop().unwrap())
    }

    pub async fn issue_full_by_number(&self, repo: Id, number: i64) -> Result<IssueFull> {
        let i = self.issue_by_number(repo, number).await?;
        Ok(self.hydrate_issues(vec![i]).await?.pop().unwrap())
    }

    /// Attach authors, assignees, labels, and milestones in a handful of queries.
    pub async fn hydrate_issues(&self, issues: Vec<Issue>) -> Result<Vec<IssueFull>> {
        if issues.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<i64> = issues.iter().map(|i| i.id).collect();
        let ph: Vec<String> = (1..=ids.len()).map(|i| format!("${i}")).collect();
        let list = ph.join(",");

        let mut q = sqlx::query(AssertSqlSafe(format!("SELECT issue_id, user_id FROM issue_assignees WHERE issue_id IN ({list})")));
        for id in &ids {
            q = q.bind(*id);
        }
        let assignee_rows = q.fetch_all(&self.pool).await.map_err(map_err)?;
        let mut q = sqlx::query(AssertSqlSafe(format!(
            "SELECT il.issue_id AS issue_id, l.id AS id FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE il.issue_id IN ({list}) ORDER BY l.name_lower"
        )));
        for id in &ids {
            q = q.bind(*id);
        }
        let label_rows = q.fetch_all(&self.pool).await.map_err(map_err)?;

        let mut account_ids: Vec<i64> = issues.iter().map(|i| i.author_id).collect();
        account_ids.extend(issues.iter().filter_map(|i| i.closed_by_id));
        account_ids.extend(assignee_rows.iter().map(|r| r.get::<i64, _>("user_id")));
        account_ids.sort_unstable();
        account_ids.dedup();
        let accounts: HashMap<i64, Account> = self.accounts_by_ids(&account_ids).await?.into_iter().map(|a| (a.id, a)).collect();

        let mut label_ids: Vec<i64> = label_rows.iter().map(|r| r.get::<i64, _>("id")).collect();
        label_ids.sort_unstable();
        label_ids.dedup();
        let labels: HashMap<i64, Label> = if label_ids.is_empty() {
            HashMap::new()
        } else {
            let ph: Vec<String> = (1..=label_ids.len()).map(|i| format!("${i}")).collect();
            let mut q =
                sqlx::query_as::<_, Label>(AssertSqlSafe(format!("SELECT {LABEL_COLS} FROM labels WHERE id IN ({})", ph.join(","))));
            for id in &label_ids {
                q = q.bind(*id);
            }
            q.fetch_all(&self.pool).await.map_err(map_err)?.into_iter().map(|l| (l.id, l)).collect()
        };
        let mut ms_ids: Vec<i64> = issues.iter().filter_map(|i| i.milestone_id).collect();
        ms_ids.sort_unstable();
        ms_ids.dedup();
        let milestones: HashMap<i64, Milestone> = if ms_ids.is_empty() {
            HashMap::new()
        } else {
            let ph: Vec<String> = (1..=ms_ids.len()).map(|i| format!("${i}")).collect();
            let mut q = sqlx::query_as::<_, Milestone>(AssertSqlSafe(format!(
                "SELECT {MILESTONE_COLS} FROM milestones WHERE id IN ({})",
                ph.join(",")
            )));
            for id in &ms_ids {
                q = q.bind(*id);
            }
            q.fetch_all(&self.pool).await.map_err(map_err)?.into_iter().map(|m| (m.id, m)).collect()
        };

        let ghost = |id: i64| Account {
            id,
            kind: "user".into(),
            login: "ghost".into(),
            name: None,
            email: None,
            bio: None,
            company: None,
            location: None,
            blog: None,
            avatar_url: None,
            is_admin: 0,
            suspended: 0,
            description: None,
            billing_email: None,
            created_at: 0,
            updated_at: 0,
            avatar_updated_at: None,
            external_org_id: None,
            job_repo: None,
            job_id: None,
        };
        Ok(issues
            .into_iter()
            .map(|issue| {
                let author = accounts.get(&issue.author_id).cloned().unwrap_or_else(|| ghost(issue.author_id));
                let assignees = assignee_rows
                    .iter()
                    .filter(|r| r.get::<i64, _>("issue_id") == issue.id)
                    .filter_map(|r| accounts.get(&r.get::<i64, _>("user_id")).cloned())
                    .collect();
                let labels = label_rows
                    .iter()
                    .filter(|r| r.get::<i64, _>("issue_id") == issue.id)
                    .filter_map(|r| labels.get(&r.get::<i64, _>("id")).cloned())
                    .collect();
                let milestone = issue.milestone_id.and_then(|m| milestones.get(&m).cloned());
                let closed_by = issue.closed_by_id.and_then(|c| accounts.get(&c).cloned());
                IssueFull { issue, author, assignees, labels, milestone, closed_by }
            })
            .collect())
    }

    /// Build the WHERE clause for a filter; returns (sql, binds).
    async fn filter_sql(&self, repo: Option<Id>, f: &IssueFilter) -> Result<(String, Vec<Bind>)> {
        let mut w: Vec<String> = Vec::new();
        let mut binds: Vec<Bind> = Vec::new();
        let push = |w: &mut Vec<String>, cond: &str, bind: Bind, binds: &mut Vec<Bind>| {
            binds.push(bind);
            w.push(cond.replace("?", &format!("${}", binds.len())));
        };
        if let Some(r) = repo {
            push(&mut w, "i.repo_id = ?", Bind::I(r.0), &mut binds);
        }
        match f.state.as_str() {
            "closed" => w.push("i.state = 'closed'".into()),
            "all" => {}
            _ => w.push("i.state = 'open'".into()),
        }
        if let Some(p) = f.is_pull {
            w.push(format!("i.is_pull = {}", if p { 1 } else { 0 }));
        }
        if let Some(since) = f.since {
            push(&mut w, "i.updated_at >= ?", Bind::I(since), &mut binds);
        }
        if let Some(c) = &f.creator {
            match self.account_by_login(c).await {
                Ok(a) => push(&mut w, "i.author_id = ?", Bind::I(a.id), &mut binds),
                Err(_) => w.push("1 = 0".into()),
            }
        }
        match f.assignee.as_deref() {
            Some("none") => w.push("NOT EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id)".into()),
            Some("*") => w.push("EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id)".into()),
            Some(login) => match self.account_by_login(login).await {
                Ok(a) => push(
                    &mut w,
                    "EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.user_id = ?)",
                    Bind::I(a.id),
                    &mut binds,
                ),
                Err(_) => w.push("1 = 0".into()),
            },
            None => {}
        }
        if let Some(m) = &f.mentioned {
            match self.account_by_login(m).await {
                Ok(a) => push(
                    &mut w,
                    "EXISTS (SELECT 1 FROM issue_events e WHERE e.issue_id = i.id AND e.event = 'mentioned' AND e.payload LIKE ?)",
                    Bind::S(format!("%\"user_id\":{}%", a.id)),
                    &mut binds,
                ),
                Err(_) => w.push("1 = 0".into()),
            }
        }
        match f.milestone.as_deref() {
            Some("none") => w.push("i.milestone_id IS NULL".into()),
            Some("*") => w.push("i.milestone_id IS NOT NULL".into()),
            Some(n) => {
                let ms = match n.parse::<i64>() {
                    Ok(num) if repo.is_some() => self.milestone_by_number(repo.unwrap(), num).await.ok(),
                    _ if repo.is_some() => self.milestone_by_title(repo.unwrap(), n).await.ok(),
                    _ => None,
                };
                match ms {
                    Some(m) => push(&mut w, "i.milestone_id = ?", Bind::I(m.id), &mut binds),
                    None => w.push("1 = 0".into()),
                }
            }
            None => {}
        }
        if let Some(r) = &f.review_requested {
            match self.account_by_login(r).await {
                Ok(a) => push(
                    &mut w,
                    "EXISTS (SELECT 1 FROM pull_review_requests rr WHERE rr.pull_id = i.id AND rr.user_id = ?)",
                    Bind::I(a.id),
                    &mut binds,
                ),
                Err(_) => w.push("1 = 0".into()),
            }
        }
        if let Some(r) = &f.reviewed_by {
            match self.account_by_login(r).await {
                Ok(a) => push(
                    &mut w,
                    "EXISTS (SELECT 1 FROM pull_reviews rv WHERE rv.pull_id = i.id AND rv.author_id = ?)",
                    Bind::I(a.id),
                    &mut binds,
                ),
                Err(_) => w.push("1 = 0".into()),
            }
        }
        if let Some(r) = &f.involves {
            match self.account_by_login(r).await {
                Ok(a) => {
                    binds.push(Bind::I(a.id));
                    let n = binds.len();
                    w.push(format!("(i.author_id = ${n} OR EXISTS (SELECT 1 FROM issue_assignees a WHERE a.issue_id = i.id AND a.user_id = ${n}) OR EXISTS (SELECT 1 FROM issue_comments c WHERE c.issue_id = i.id AND c.author_id = ${n}) OR EXISTS (SELECT 1 FROM pull_review_requests rr WHERE rr.pull_id = i.id AND rr.user_id = ${n}) OR EXISTS (SELECT 1 FROM issue_events e WHERE e.issue_id = i.id AND e.event = 'mentioned' AND e.payload LIKE '%\"user_id\":' || ${n} || '%'))"));
                }
                Err(_) => w.push("1 = 0".into()),
            }
        }
        for l in &f.labels {
            push(
                &mut w,
                "EXISTS (SELECT 1 FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE il.issue_id = i.id AND l.name_lower = ?)",
                Bind::S(l.trim().to_lowercase()),
                &mut binds,
            );
        }
        for t in &f.terms {
            let like = format!("%{}%", t.to_lowercase().replace('%', "\\%"));
            binds.push(Bind::S(like.clone()));
            let a = binds.len();
            binds.push(Bind::S(like));
            let bb = binds.len();
            w.push(format!("(LOWER(i.title) LIKE ${a} OR LOWER(COALESCE(i.body, '')) LIKE ${bb})"));
        }
        Ok((if w.is_empty() { "1 = 1".into() } else { w.join(" AND ") }, binds))
    }

    /// Issues matching a filter, paginated. Returns (page, total).
    pub async fn list_issues(&self, repo: Option<Id>, f: &IssueFilter, page: i64, per_page: i64) -> Result<(Vec<IssueFull>, i64)> {
        let (wh, binds) = self.filter_sql(repo, f).await?;
        let order = match f.sort.as_str() {
            "updated" => "i.updated_at",
            "comments" => "i.comments_count",
            _ => "i.created_at",
        };
        let dir = if f.ascending { "ASC" } else { "DESC" };
        let n = binds.len();
        let sql =
            format!("SELECT {ISSUE_COLS} FROM issues i WHERE {wh} ORDER BY {order} {dir}, i.id {dir} LIMIT ${} OFFSET ${}", n + 1, n + 2);
        let mut q = sqlx::query_as::<_, Issue>(AssertSqlSafe(sql));
        for bnd in &binds {
            q = match bnd {
                Bind::I(i) => q.bind(*i),
                Bind::S(s) => q.bind(s.clone()),
            };
        }
        let rows = q.bind(per_page).bind((page.max(1) - 1) * per_page).fetch_all(&self.pool).await.map_err(map_err)?;
        let mut cq = sqlx::query(AssertSqlSafe(format!("SELECT COUNT(*) AS n FROM issues i WHERE {wh}")));
        for bnd in &binds {
            cq = match bnd {
                Bind::I(i) => cq.bind(*i),
                Bind::S(s) => cq.bind(s.clone()),
            };
        }
        let total: i64 = cq.fetch_one(&self.pool).await.map_err(map_err)?.try_get("n").map_err(map_err)?;
        Ok((self.hydrate_issues(rows).await?, total))
    }

    pub async fn count_issues(&self, repo: Id, is_pull: bool, state: &str) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM issues WHERE repo_id = $1 AND is_pull = $2 AND ($3 = 'all' OR state = $3)")
            .bind(repo.0)
            .bind(b(is_pull))
            .bind(state)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)?;
        row.try_get("n").map_err(map_err)
    }

    async fn recount_open_issues(&self, repo: Id) -> Result<()> {
        sqlx::query("UPDATE repositories SET open_issues_count = (SELECT COUNT(*) FROM issues WHERE repo_id = $1 AND state = 'open' AND is_pull = 0), open_pulls_count = (SELECT COUNT(*) FROM issues WHERE repo_id = $1 AND state = 'open' AND is_pull = 1) WHERE id = $1")
            .bind(repo.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    pub async fn update_issue(&self, id: Id, actor: &Account, p: IssuePatch) -> Result<IssueFull> {
        let cur = self.issue_by_id(id).await?;
        let repo = Id(cur.repo_id);
        let ts = now();
        if let Some(t) = &p.title {
            let t = t.trim();
            if t.is_empty() {
                return Err(Error::field("Issue", "title", "missing_field", "title is required"));
            }
            if t != cur.title {
                sqlx::query("UPDATE issues SET title = $1, updated_at = $2 WHERE id = $3")
                    .bind(t)
                    .bind(ts)
                    .bind(id.0)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(id, repo, Some(actor.id()), "renamed", &serde_json::json!({"from": cur.title, "to": t})).await?;
            }
        }
        if let Some(body) = &p.body {
            let body = body.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
            sqlx::query("UPDATE issues SET body = $1, updated_at = $2 WHERE id = $3")
                .bind(&body)
                .bind(ts)
                .bind(id.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
            self.record_mentions_by(id, repo, body.as_deref().unwrap_or(""), Some(actor.id())).await?;
        }
        if let Some(m) = p.milestone
            && m != cur.milestone_id
        {
            if let Some(mid) = m {
                self.milestone_by_id(Id(mid)).await.map_err(|_| Error::validation("milestone not found"))?;
            }
            sqlx::query("UPDATE issues SET milestone_id = $1, updated_at = $2 WHERE id = $3")
                .bind(m)
                .bind(ts)
                .bind(id.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
            if let Some(old) = cur.milestone_id {
                self.add_issue_event(id, repo, Some(actor.id()), "demilestoned", &serde_json::json!({"milestone_id": old})).await?;
            }
            if let Some(new) = m {
                self.add_issue_event(id, repo, Some(actor.id()), "milestoned", &serde_json::json!({"milestone_id": new})).await?;
            }
        }
        if let Some(labels) = &p.labels {
            self.set_issue_labels(id, repo, actor, labels).await?;
        }
        if let Some(assignees) = &p.assignees {
            self.set_issue_assignees(id, repo, actor, assignees).await?;
        }
        if let Some(state) = &p.state {
            let reason = p.state_reason.clone().unwrap_or(None);
            if state == "closed" && cur.state != "closed" {
                let reason = reason.unwrap_or_else(|| "completed".into());
                sqlx::query("UPDATE issues SET state = 'closed', state_reason = $1, closed_at = $2, closed_by_id = $3, updated_at = $2 WHERE id = $4")
                    .bind(&reason)
                    .bind(ts)
                    .bind(actor.id)
                    .bind(id.0)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(
                    id,
                    repo,
                    Some(actor.id()),
                    "closed",
                    &serde_json::json!({"state_reason": reason, "commit_id": p.close_commit, "closing_pull": p.close_pull}),
                )
                .await?;
                let _ = self.status_automation(id, true).await;
                let _ = self.notify_participants(id, Some(actor.id()), "state_change").await;
            } else if state == "open" && cur.state != "open" {
                sqlx::query("UPDATE issues SET state = 'open', state_reason = 'reopened', closed_at = NULL, closed_by_id = NULL, updated_at = $1 WHERE id = $2")
                    .bind(ts)
                    .bind(id.0)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(id, repo, Some(actor.id()), "reopened", &serde_json::json!({})).await?;
                let _ = self.status_automation(id, false).await;
                let _ = self.notify_participants(id, Some(actor.id()), "state_change").await;
            } else if let Some(Some(r)) = &p.state_reason {
                sqlx::query("UPDATE issues SET state_reason = $1 WHERE id = $2")
                    .bind(r)
                    .bind(id.0)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
            }
            self.recount_open_issues(repo).await?;
        }
        self.issue_full_by_id(id).await
    }

    pub async fn set_issue_lock(&self, id: Id, actor: &Account, locked: bool, reason: Option<&str>) -> Result<()> {
        let cur = self.issue_by_id(id).await?;
        sqlx::query("UPDATE issues SET locked = $1, lock_reason = $2, updated_at = $3 WHERE id = $4")
            .bind(b(locked))
            .bind(if locked { reason } else { None })
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.add_issue_event(
            id,
            Id(cur.repo_id),
            Some(actor.id()),
            if locked { "locked" } else { "unlocked" },
            &serde_json::json!({"lock_reason": reason}),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_issue(&self, id: Id) -> Result<()> {
        let cur = self.issue_by_id(id).await?;
        for t in ["issue_labels", "issue_assignees", "issue_comments", "issue_events"] {
            sqlx::query(AssertSqlSafe(format!("DELETE FROM {t} WHERE issue_id = $1")))
                .bind(id.0)
                .execute(&self.pool)
                .await
                .map_err(map_err)?;
        }
        sqlx::query("DELETE FROM reactions WHERE subject_type = 'issue' AND subject_id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query("DELETE FROM issues WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        self.recount_open_issues(Id(cur.repo_id)).await
    }

    // ----- labels on issues -----

    pub async fn issue_labels(&self, id: Id) -> Result<Vec<Label>> {
        sqlx::query_as::<_, Label>(AssertSqlSafe("SELECT l.id, l.repo_id, l.name, l.color, l.description, l.is_default, l.created_at, l.updated_at FROM labels l JOIN issue_labels il ON il.label_id = l.id WHERE il.issue_id = $1 ORDER BY l.name_lower".to_string()))
        .bind(id.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Replace the label set. Unknown names are an error, as on GitHub.
    pub async fn set_issue_labels(&self, id: Id, repo: Id, actor: &Account, names: &[String]) -> Result<Vec<Label>> {
        let mut want: Vec<Label> = Vec::new();
        for n in names {
            let l = self
                .label_by_name(repo, n)
                .await
                .map_err(|_| Error::field("Label", "name", "invalid", format!("label \"{n}\" not found")))?;
            if !want.iter().any(|w| w.id == l.id) {
                want.push(l);
            }
        }
        let have = self.issue_labels(id).await?;
        for l in &have {
            if !want.iter().any(|w| w.id == l.id) {
                sqlx::query("DELETE FROM issue_labels WHERE issue_id = $1 AND label_id = $2")
                    .bind(id.0)
                    .bind(l.id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(
                    id,
                    repo,
                    Some(actor.id()),
                    "unlabeled",
                    &serde_json::json!({"label": {"name": l.name, "color": l.color}}),
                )
                .await?;
            }
        }
        for l in &want {
            if !have.iter().any(|h| h.id == l.id) {
                sqlx::query("INSERT INTO issue_labels (issue_id, label_id) VALUES ($1, $2)")
                    .bind(id.0)
                    .bind(l.id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(
                    id,
                    repo,
                    Some(actor.id()),
                    "labeled",
                    &serde_json::json!({"label": {"name": l.name, "color": l.color}}),
                )
                .await?;
            }
        }
        self.touch_issue(id).await?;
        self.issue_labels(id).await
    }

    pub async fn add_issue_labels(&self, id: Id, repo: Id, actor: &Account, names: &[String]) -> Result<Vec<Label>> {
        let mut all: Vec<String> = self.issue_labels(id).await?.into_iter().map(|l| l.name).collect();
        all.extend(names.iter().cloned());
        self.set_issue_labels(id, repo, actor, &all).await
    }

    pub async fn remove_issue_label(&self, id: Id, repo: Id, actor: &Account, name: &str) -> Result<Vec<Label>> {
        let keep: Vec<String> =
            self.issue_labels(id).await?.into_iter().filter(|l| !l.name.eq_ignore_ascii_case(name)).map(|l| l.name).collect();
        self.set_issue_labels(id, repo, actor, &keep).await
    }

    // ----- assignees -----

    pub async fn issue_assignees(&self, id: Id) -> Result<Vec<Account>> {
        let rows = sqlx::query("SELECT user_id FROM issue_assignees WHERE issue_id = $1")
            .bind(id.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)?;
        let ids: Vec<i64> = rows.iter().map(|r| r.get::<i64, _>("user_id")).collect();
        self.accounts_by_ids(&ids).await
    }

    pub async fn set_issue_assignees(&self, id: Id, repo: Id, actor: &Account, logins: &[String]) -> Result<Vec<Account>> {
        let mut want: Vec<Account> = Vec::new();
        for l in logins {
            let a = self
                .account_by_login(l)
                .await
                .map_err(|_| Error::field("Issue", "assignees", "invalid", format!("user \"{l}\" not found")))?;
            if a.is_user() && !want.iter().any(|w| w.id == a.id) {
                want.push(a);
            }
        }
        let have = self.issue_assignees(id).await?;
        for a in &have {
            if !want.iter().any(|w| w.id == a.id) {
                sqlx::query("DELETE FROM issue_assignees WHERE issue_id = $1 AND user_id = $2")
                    .bind(id.0)
                    .bind(a.id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(id, repo, Some(actor.id()), "unassigned", &serde_json::json!({"assignee_id": a.id})).await?;
            }
        }
        for a in &want {
            if !have.iter().any(|h| h.id == a.id) {
                sqlx::query("INSERT INTO issue_assignees (issue_id, user_id) VALUES ($1, $2)")
                    .bind(id.0)
                    .bind(a.id)
                    .execute(&self.pool)
                    .await
                    .map_err(map_err)?;
                self.add_issue_event(id, repo, Some(actor.id()), "assigned", &serde_json::json!({"assignee_id": a.id})).await?;
                let _ = self.notify_user_about_issue(a.id(), id, Some(actor.id()), "assign").await;
            }
        }
        self.touch_issue(id).await?;
        self.issue_assignees(id).await
    }

    pub async fn add_issue_assignees(&self, id: Id, repo: Id, actor: &Account, logins: &[String]) -> Result<Vec<Account>> {
        let mut all: Vec<String> = self.issue_assignees(id).await?.into_iter().map(|a| a.login).collect();
        all.extend(logins.iter().cloned());
        self.set_issue_assignees(id, repo, actor, &all).await
    }

    pub async fn remove_issue_assignees(&self, id: Id, repo: Id, actor: &Account, logins: &[String]) -> Result<Vec<Account>> {
        let keep: Vec<String> = self
            .issue_assignees(id)
            .await?
            .into_iter()
            .filter(|a| !logins.iter().any(|l| l.eq_ignore_ascii_case(&a.login)))
            .map(|a| a.login)
            .collect();
        self.set_issue_assignees(id, repo, actor, &keep).await
    }

    async fn touch_issue(&self, id: Id) -> Result<()> {
        sqlx::query("UPDATE issues SET updated_at = $1 WHERE id = $2").bind(now()).bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        Ok(())
    }

    // ----- comments -----

    pub async fn create_comment(&self, issue: Id, author: &Account, body: &str) -> Result<Comment> {
        let body = body.trim();
        if body.is_empty() {
            return Err(Error::field("IssueComment", "body", "missing_field", "body is required"));
        }
        let cur = self.issue_by_id(issue).await?;
        let id = reef_core::id::next();
        let ts = now();
        // The comment, its webhook outbox entry and the count commit together.
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        sqlx::query("INSERT INTO issue_comments (id, issue_id, repo_id, author_id, body, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $6)")
            .bind(id.0)
            .bind(issue.0)
            .bind(cur.repo_id)
            .bind(author.id)
            .bind(body)
            .bind(ts)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        crate::webhooks::enqueue_outbox(&mut *tx, crate::webhooks::OUTBOX_ISSUE_COMMENTS, id.0, ts).await?;
        sqlx::query("UPDATE issues SET comments_count = comments_count + 1, updated_at = $1 WHERE id = $2")
            .bind(ts)
            .bind(issue.0)
            .execute(&mut *tx)
            .await
            .map_err(map_err)?;
        tx.commit().await.map_err(map_err)?;
        self.record_mentions_by(issue, Id(cur.repo_id), body, Some(author.id())).await?;
        let _ = self.notify_participants(issue, Some(author.id()), "comment").await;
        self.comment_by_id(id).await
    }

    pub async fn comment_by_id(&self, id: Id) -> Result<Comment> {
        sqlx::query_as::<_, Comment>(AssertSqlSafe(format!("SELECT {COMMENT_COLS} FROM issue_comments WHERE id = $1")))
            .bind(id.0)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn comments(&self, issue: Id, since: Option<i64>) -> Result<Vec<Comment>> {
        sqlx::query_as::<_, Comment>(AssertSqlSafe(format!(
            "SELECT {COMMENT_COLS} FROM issue_comments WHERE issue_id = $1 AND ($2 IS NULL OR updated_at >= $2) ORDER BY id"
        )))
        .bind(issue.0)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn repo_comments(&self, repo: Id, since: Option<i64>) -> Result<Vec<Comment>> {
        sqlx::query_as::<_, Comment>(AssertSqlSafe(format!(
            "SELECT {COMMENT_COLS} FROM issue_comments WHERE repo_id = $1 AND ($2 IS NULL OR updated_at >= $2) ORDER BY id DESC LIMIT 200"
        )))
        .bind(repo.0)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    pub async fn update_comment(&self, id: Id, body: &str) -> Result<Comment> {
        let body = body.trim();
        if body.is_empty() {
            return Err(Error::field("IssueComment", "body", "missing_field", "body is required"));
        }
        sqlx::query("UPDATE issue_comments SET body = $1, updated_at = $2 WHERE id = $3")
            .bind(body)
            .bind(now())
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        self.comment_by_id(id).await
    }

    pub async fn delete_comment(&self, id: Id) -> Result<()> {
        let c = self.comment_by_id(id).await?;
        sqlx::query("DELETE FROM issue_comments WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("DELETE FROM reactions WHERE subject_type = 'issue_comment' AND subject_id = $1")
            .bind(id.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        sqlx::query("UPDATE issue_comments SET updated_at = updated_at WHERE 1 = 0").execute(&self.pool).await.map_err(map_err)?;
        sqlx::query("UPDATE issues SET comments_count = (SELECT COUNT(*) FROM issue_comments WHERE issue_id = $1) WHERE id = $1")
            .bind(c.issue_id)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        Ok(())
    }

    // ----- events -----

    pub async fn add_issue_event(
        &self,
        issue: Id,
        repo: Id,
        actor: Option<Id>,
        event: &str,
        payload: &serde_json::Value,
    ) -> Result<IssueEvent> {
        let id = reef_core::id::next();
        let ts = now();
        // The event and its webhook outbox entry commit together.
        let mut tx = self.pool.begin().await.map_err(map_err)?;
        sqlx::query(
            "INSERT INTO issue_events (id, issue_id, repo_id, actor_id, event, payload, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(id.0)
        .bind(issue.0)
        .bind(repo.0)
        .bind(actor.map(|a| a.0))
        .bind(event)
        .bind(payload.to_string())
        .bind(ts)
        .execute(&mut *tx)
        .await
        .map_err(map_err)?;
        crate::webhooks::enqueue_outbox(&mut *tx, crate::webhooks::OUTBOX_ISSUE_EVENTS, id.0, ts).await?;
        tx.commit().await.map_err(map_err)?;
        Ok(IssueEvent {
            id: id.0,
            issue_id: issue.0,
            repo_id: repo.0,
            actor_id: actor.map(|a| a.0),
            event: event.into(),
            payload: payload.to_string(),
            created_at: ts,
        })
    }

    pub async fn issue_events(&self, issue: Id) -> Result<Vec<IssueEvent>> {
        sqlx::query_as::<_, IssueEvent>(
            "SELECT id, issue_id, repo_id, actor_id, event, payload, created_at FROM issue_events WHERE issue_id = $1 ORDER BY id",
        )
        .bind(issue.0)
        .fetch_all(&self.pool)
        .await
        .map_err(map_err)
    }

    /// Record `@login` mentions found in `text` as `mentioned` events (once per user per issue).
    async fn record_mentions_by(&self, issue: Id, repo: Id, text: &str, actor: Option<Id>) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        for login in reef_core::names::mentions(text) {
            if !seen.insert(login.to_lowercase()) {
                continue;
            }
            if let Ok(a) = self.account_by_login(&login).await {
                let exists = sqlx::query("SELECT 1 AS x FROM issue_events WHERE issue_id = $1 AND event = 'mentioned' AND payload LIKE $2")
                    .bind(issue.0)
                    .bind(format!("%\"user_id\":{}%", a.id))
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(map_err)?;
                if exists.is_none() {
                    self.add_issue_event(issue, repo, None, "mentioned", &serde_json::json!({"user_id": a.id})).await?;
                    let _ = self.notify_user_about_issue(a.id(), issue, actor, "mention").await;
                }
            }
        }
        Ok(())
    }

    // ----- reactions -----

    pub async fn add_reaction(&self, subject_type: &str, subject_id: Id, user: Id, content: &str) -> Result<Reaction> {
        if !REACTIONS.contains(&content) {
            return Err(Error::field("Reaction", "content", "invalid", "content is invalid"));
        }
        let id = reef_core::id::next();
        let r = sqlx::query(
            "INSERT INTO reactions (id, subject_type, subject_id, user_id, content, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id.0)
        .bind(subject_type)
        .bind(subject_id.0)
        .bind(user.0)
        .bind(content)
        .bind(now())
        .execute(&self.pool)
        .await;
        match r.map_err(map_err) {
            Ok(_) => {}
            Err(Error::Conflict(_)) => {}
            Err(e) => return Err(e),
        }
        sqlx::query_as::<_, Reaction>("SELECT id, subject_type, subject_id, user_id, content, created_at FROM reactions WHERE subject_type = $1 AND subject_id = $2 AND user_id = $3 AND content = $4")
            .bind(subject_type)
            .bind(subject_id.0)
            .bind(user.0)
            .bind(content)
            .fetch_one(&self.pool)
            .await
            .map_err(map_err)
    }

    pub async fn delete_reaction(&self, id: Id, user: Id) -> Result<()> {
        let r = sqlx::query("DELETE FROM reactions WHERE id = $1 AND user_id = $2")
            .bind(id.0)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(map_err)?;
        if r.rows_affected() == 0 { Err(Error::NotFound) } else { Ok(()) }
    }

    pub async fn reactions(&self, subject_type: &str, subject_id: Id) -> Result<Vec<Reaction>> {
        sqlx::query_as::<_, Reaction>("SELECT id, subject_type, subject_id, user_id, content, created_at FROM reactions WHERE subject_type = $1 AND subject_id = $2 ORDER BY id")
            .bind(subject_type)
            .bind(subject_id.0)
            .fetch_all(&self.pool)
            .await
            .map_err(map_err)
    }

    /// Reaction counts by content for many subjects at once: (subject_id, content) -> count,
    /// plus the viewer's own reactions when `viewer` is given.
    pub async fn reaction_summary(
        &self,
        subject_type: &str,
        ids: &[i64],
        viewer: Option<i64>,
    ) -> Result<(HashMap<(i64, String), i64>, std::collections::HashSet<(i64, String)>)> {
        let mut counts = HashMap::new();
        let mut mine = std::collections::HashSet::new();
        if ids.is_empty() {
            return Ok((counts, mine));
        }
        let ph: Vec<String> = (2..=ids.len() + 1).map(|i| format!("${i}")).collect();
        let mut q = sqlx::query(AssertSqlSafe(format!(
            "SELECT subject_id, content, user_id FROM reactions WHERE subject_type = $1 AND subject_id IN ({})",
            ph.join(",")
        )));
        q = q.bind(subject_type);
        for id in ids {
            q = q.bind(*id);
        }
        for row in q.fetch_all(&self.pool).await.map_err(map_err)? {
            let sid: i64 = row.get("subject_id");
            let content: String = row.get("content");
            let uid: i64 = row.get("user_id");
            *counts.entry((sid, content.clone())).or_insert(0) += 1;
            if Some(uid) == viewer {
                mine.insert((sid, content));
            }
        }
        Ok((counts, mine))
    }
}

enum Bind {
    I(i64),
    S(String),
}
