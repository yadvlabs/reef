//! GitHub Projects V2 surface: `projectV2`, `projectsV2`, items, fields, and the mutations
//! `gh project` uses.

use super::issues::{self as gi, IssueOrPullRequest};
use super::types::*;
use super::{Ctx, from_core, gql_error, not_found};
use crate::shapes;
use async_graphql::{Context, Enum, ID, InputObject, Object, Result, SimpleObject, Union};
use reef_core::Id;
use reef_core::time::{iso, iso_opt};
use reef_db::accounts::Account;
use reef_db::issues::IssueFull;
use reef_db::projects::{Field, Item, Project};
use reef_db::repos::Permission;

/// Built-in fields every project has, mirroring GitHub's system fields. Their ids are
/// `<project field node>_<key>` so they never collide with stored fields.
const SYSTEM_FIELDS: &[(&str, &str, ProjectV2FieldType)] = &[
    ("title", "Title", ProjectV2FieldType::Title),
    ("assignees", "Assignees", ProjectV2FieldType::Assignees),
    ("labels", "Labels", ProjectV2FieldType::Labels),
    ("milestone", "Milestone", ProjectV2FieldType::Milestone),
    ("repository", "Repository", ProjectV2FieldType::Repository),
];

fn system_field(project_id: i64, key: &str) -> ProjectV2FieldConfiguration {
    let (_, name, dt) = SYSTEM_FIELDS.iter().find(|(k, _, _)| *k == key).copied().unwrap_or(("title", "Title", ProjectV2FieldType::Title));
    ProjectV2FieldConfiguration::Field(ProjectV2Field {
        id: ID(format!("{}_{}", shapes::node_id(NODE_FIELD, project_id), key)),
        name: name.into(),
        data_type: dt,
    })
}

pub const NODE_PROJECT: &str = "PVT";
pub const NODE_FIELD: &str = "PVTF";
pub const NODE_ITEM: &str = "PVTI";
pub const NODE_DRAFT: &str = "DI";

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ProjectV2FieldType {
    Assignees,
    LinkedPullRequests,
    Reviewers,
    Labels,
    Milestone,
    Repository,
    Title,
    Text,
    SingleSelect,
    Number,
    Date,
    Iteration,
    TrackedBy,
    Tracks,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ProjectV2CustomFieldType {
    Text,
    SingleSelect,
    Number,
    Date,
    Iteration,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ProjectV2SingleSelectFieldOptionColor {
    Gray,
    Blue,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    Purple,
}

fn color_of(s: &str) -> ProjectV2SingleSelectFieldOptionColor {
    match s {
        "BLUE" => ProjectV2SingleSelectFieldOptionColor::Blue,
        "GREEN" => ProjectV2SingleSelectFieldOptionColor::Green,
        "YELLOW" => ProjectV2SingleSelectFieldOptionColor::Yellow,
        "ORANGE" => ProjectV2SingleSelectFieldOptionColor::Orange,
        "RED" => ProjectV2SingleSelectFieldOptionColor::Red,
        "PINK" => ProjectV2SingleSelectFieldOptionColor::Pink,
        "PURPLE" => ProjectV2SingleSelectFieldOptionColor::Purple,
        _ => ProjectV2SingleSelectFieldOptionColor::Gray,
    }
}

// ----- project -----

#[derive(Clone)]
pub struct ProjectV2Obj {
    pub owner: Account,
    pub p: Project,
}

#[Object(name = "ProjectV2")]
impl ProjectV2Obj {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(NODE_PROJECT, self.p.id))
    }
    async fn database_id(&self) -> i64 {
        self.p.id
    }
    async fn number(&self) -> i32 {
        self.p.number as i32
    }
    async fn title(&self) -> &str {
        &self.p.title
    }
    async fn short_description(&self) -> Option<&str> {
        self.p.short_description.as_deref()
    }
    async fn readme(&self) -> Option<&str> {
        self.p.readme.as_deref()
    }
    async fn public(&self) -> bool {
        self.p.public != 0
    }
    async fn closed(&self) -> bool {
        self.p.closed != 0
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}/projects/{}", Ctx::from(ctx).config.site_url(), self.owner.login, self.p.number)
    }
    async fn resource_path(&self) -> String {
        format!("/{}/projects/{}", self.owner.login, self.p.number)
    }
    async fn created_at(&self) -> String {
        iso(self.p.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.p.updated_at)
    }
    async fn closed_at(&self) -> Option<String> {
        iso_opt(self.p.closed_at)
    }
    async fn owner(&self) -> ProjectV2Owner {
        if self.owner.is_org() {
            ProjectV2Owner::Organization(Organization(self.owner.clone()))
        } else {
            ProjectV2Owner::User(User(self.owner.clone()))
        }
    }
    async fn creator(&self, ctx: &Context<'_>) -> Option<User> {
        Ctx::from(ctx).db.account_by_id(Id(self.p.creator_id)).await.ok().map(User)
    }
    async fn viewer_can_update(&self, ctx: &Context<'_>) -> Result<bool> {
        let s = super::app(ctx);
        let c = Ctx::from(ctx);
        Ok(crate::projects::access(&s, c.viewer(), &self.owner, Some(&self.p)).await.map_err(|e| from_core(e.0))?
            >= crate::projects::Access::Write)
    }
    async fn template(&self) -> bool {
        false
    }
    async fn fields(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<ProjectV2FieldConnection> {
        let c = Ctx::from(ctx);
        let fields = c.db.fields(self.p.id()).await.map_err(from_core)?;
        let mut all: Vec<ProjectV2FieldConfiguration> = vec![system_field(self.p.id, "title"), system_field(self.p.id, "assignees")];
        all.extend(fields.iter().map(field_config));
        all.extend(["labels", "milestone", "repository"].into_iter().map(|k| system_field(self.p.id, k)));
        let total = all.len();
        let (items, info, _) = page(all, first, after);
        Ok(ProjectV2FieldConnection { nodes: items.into_iter().map(|(_, f)| f).collect(), page_info: info, total_count: total as i32 })
    }
    async fn field(&self, ctx: &Context<'_>, name: String) -> Result<Option<ProjectV2FieldConfiguration>> {
        let c = Ctx::from(ctx);
        if let Some((k, _, _)) = SYSTEM_FIELDS.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(&name)) {
            return Ok(Some(system_field(self.p.id, k)));
        }
        Ok(c.db.fields(self.p.id()).await.map_err(from_core)?.iter().find(|f| f.name.eq_ignore_ascii_case(&name)).map(field_config))
    }
    async fn items(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        query: Option<String>,
    ) -> Result<ProjectV2ItemConnection> {
        let c = Ctx::from(ctx);
        let mut items = c.db.items(self.p.id(), false).await.map_err(from_core)?;
        if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
            let q = q.to_lowercase();
            let mut keep = Vec::new();
            for it in items {
                let title = match (it.content_type.as_str(), it.content_id) {
                    ("draft", _) => it.draft_title.clone().unwrap_or_default(),
                    (_, Some(cid)) => c.db.issue_by_id(Id(cid)).await.map(|i| i.title).unwrap_or_default(),
                    _ => String::new(),
                };
                if title.to_lowercase().contains(&q) || q.contains("is:") {
                    keep.push(it);
                }
            }
            items = keep;
        }
        let total = items.len();
        let (sel, info, _) = page(items, first, after);
        Ok(ProjectV2ItemConnection {
            nodes: sel.into_iter().map(|(_, it)| ProjectV2ItemObj { project: self.clone(), it }).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn repositories(&self, ctx: &Context<'_>, first: Option<i32>) -> Result<RepositoryConnection> {
        let c = Ctx::from(ctx);
        let mut repos = Vec::new();
        for rid in c.db.linked_repos(self.p.id()).await.map_err(from_core)? {
            if let Ok(rw) = c.db.repo_with_owner_by_id(Id(rid)).await
                && c.db.permission(c.viewer(), &rw).await.map_err(from_core)?.can_read()
            {
                repos.push(rw);
            }
        }
        let (items, info, total) = page(repos, first, None);
        Ok(RepositoryConnection {
            nodes: items.iter().map(|(_, r)| Repository(r.clone())).collect(),
            edges: items.into_iter().map(|(i, r)| RepositoryEdge { cursor: cursor(i), node: Repository(r) }).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn views(&self, ctx: &Context<'_>, first: Option<i32>) -> Result<ProjectV2ViewConnection> {
        let c = Ctx::from(ctx);
        let views = c.db.views(self.p.id()).await.map_err(from_core)?;
        let total = views.len();
        let (items, info, _) = page(views, first, None);
        Ok(ProjectV2ViewConnection {
            nodes: items
                .into_iter()
                .map(|(_, v)| ProjectV2View {
                    id: ID(shapes::node_id("PVTV", v.id)),
                    number: v.number as i32,
                    name: v.name,
                    layout: match v.layout.as_str() {
                        "table" => ProjectV2ViewLayout::TableLayout,
                        "roadmap" => ProjectV2ViewLayout::RoadmapLayout,
                        _ => ProjectV2ViewLayout::BoardLayout,
                    },
                })
                .collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
}

#[derive(Union, Clone)]
pub enum ProjectV2Owner {
    User(User),
    Organization(Organization),
}

#[derive(SimpleObject)]
pub struct ProjectV2Connection {
    pub nodes: Vec<ProjectV2Obj>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ProjectV2ViewLayout {
    BoardLayout,
    TableLayout,
    RoadmapLayout,
}

#[derive(SimpleObject)]
pub struct ProjectV2View {
    pub id: ID,
    pub number: i32,
    pub name: String,
    pub layout: ProjectV2ViewLayout,
}

#[derive(SimpleObject)]
pub struct ProjectV2ViewConnection {
    pub nodes: Vec<ProjectV2View>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

// ----- fields -----

#[derive(SimpleObject, Clone)]
pub struct ProjectV2Field {
    pub id: ID,
    pub name: String,
    pub data_type: ProjectV2FieldType,
}

#[derive(SimpleObject, Clone)]
pub struct ProjectV2SingleSelectFieldOption {
    pub id: String,
    pub name: String,
    pub color: ProjectV2SingleSelectFieldOptionColor,
    pub description: String,
}

#[derive(SimpleObject, Clone)]
pub struct ProjectV2SingleSelectField {
    pub id: ID,
    pub name: String,
    pub data_type: ProjectV2FieldType,
    pub options: Vec<ProjectV2SingleSelectFieldOption>,
}

#[derive(SimpleObject, Clone)]
pub struct ProjectV2IterationField {
    pub id: ID,
    pub name: String,
    pub data_type: ProjectV2FieldType,
}

#[derive(Union, Clone)]
pub enum ProjectV2FieldConfiguration {
    Field(ProjectV2Field),
    SingleSelectField(ProjectV2SingleSelectField),
    IterationField(ProjectV2IterationField),
}

#[derive(SimpleObject)]
pub struct ProjectV2FieldConnection {
    pub nodes: Vec<ProjectV2FieldConfiguration>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

fn field_config(f: &Field) -> ProjectV2FieldConfiguration {
    let id = ID(shapes::node_id(NODE_FIELD, f.id));
    match f.data_type.as_str() {
        "single_select" => ProjectV2FieldConfiguration::SingleSelectField(ProjectV2SingleSelectField {
            id,
            name: f.name.clone(),
            data_type: ProjectV2FieldType::SingleSelect,
            options: f
                .options()
                .into_iter()
                .map(|o| ProjectV2SingleSelectFieldOption { id: o.id, name: o.name, color: color_of(&o.color), description: o.description })
                .collect(),
        }),
        "iteration" => ProjectV2FieldConfiguration::IterationField(ProjectV2IterationField {
            id,
            name: f.name.clone(),
            data_type: ProjectV2FieldType::Iteration,
        }),
        other => ProjectV2FieldConfiguration::Field(ProjectV2Field {
            id,
            name: f.name.clone(),
            data_type: match other {
                "number" => ProjectV2FieldType::Number,
                "date" => ProjectV2FieldType::Date,
                _ => ProjectV2FieldType::Text,
            },
        }),
    }
}

// ----- items -----

#[derive(Clone)]
pub struct ProjectV2ItemObj {
    pub project: ProjectV2Obj,
    pub it: Item,
}

#[derive(SimpleObject, Clone)]
pub struct DraftIssue {
    pub id: ID,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Union, Clone)]
pub enum ProjectV2ItemContent {
    DraftIssue(DraftIssue),
    Issue(gi::Issue),
    PullRequest(gi::PullRequest),
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ProjectV2ItemType {
    Issue,
    PullRequest,
    DraftIssue,
    Redacted,
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldTextValue {
    pub id: ID,
    pub text: Option<String>,
    pub field: ProjectV2FieldConfiguration,
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldNumberValue {
    pub id: ID,
    pub number: Option<f64>,
    pub field: ProjectV2FieldConfiguration,
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldDateValue {
    pub id: ID,
    pub date: Option<String>,
    pub field: ProjectV2FieldConfiguration,
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldSingleSelectValue {
    pub id: ID,
    pub name: Option<String>,
    pub option_id: Option<String>,
    pub field: ProjectV2FieldConfiguration,
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldIterationValue {
    pub id: ID,
    pub title: String,
    pub start_date: String,
    pub duration: i32,
    pub iteration_id: String,
    pub field: ProjectV2FieldConfiguration,
}

pub struct ProjectV2ItemFieldLabelValue {
    pub id: ID,
    pub labels: Vec<gi::Label>,
    pub field: ProjectV2FieldConfiguration,
}

#[Object]
impl ProjectV2ItemFieldLabelValue {
    async fn id(&self) -> &ID {
        &self.id
    }
    async fn field(&self) -> ProjectV2FieldConfiguration {
        self.field.clone()
    }
    async fn labels(&self, first: Option<i32>, after: Option<String>) -> gi::LabelConnection2 {
        let (items, info, total) = page(self.labels.clone(), first, after);
        gi::LabelConnection2 {
            nodes: items.iter().map(|(_, l)| l.clone()).collect(),
            edges: items.into_iter().map(|(i, l)| gi::LabelEdge { cursor: cursor(i), node: l }).collect(),
            page_info: info,
            total_count: total as i32,
        }
    }
}

pub struct ProjectV2ItemFieldUserValue {
    pub id: ID,
    pub users: Vec<User>,
    pub field: ProjectV2FieldConfiguration,
}

#[Object]
impl ProjectV2ItemFieldUserValue {
    async fn id(&self) -> &ID {
        &self.id
    }
    async fn field(&self) -> ProjectV2FieldConfiguration {
        self.field.clone()
    }
    async fn users(&self, first: Option<i32>, after: Option<String>) -> UserConnection {
        let (items, info, total) = page(self.users.clone(), first, after);
        UserConnection { nodes: items.into_iter().map(|(_, u)| u).collect(), page_info: info, total_count: total as i32 }
    }
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldMilestoneValue {
    pub id: ID,
    pub milestone: Option<gi::Milestone>,
    pub field: ProjectV2FieldConfiguration,
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldRepositoryValue {
    pub id: ID,
    pub repository: Option<Repository>,
    pub field: ProjectV2FieldConfiguration,
}

pub struct ProjectV2ItemFieldPullRequestValue {
    pub id: ID,
    pub pull_requests: Vec<gi::PullRequest>,
    pub field: ProjectV2FieldConfiguration,
}

#[Object]
impl ProjectV2ItemFieldPullRequestValue {
    async fn id(&self) -> &ID {
        &self.id
    }
    async fn field(&self) -> ProjectV2FieldConfiguration {
        self.field.clone()
    }
    async fn pull_requests(&self, first: Option<i32>, after: Option<String>) -> gi::PullRequestConnection {
        let (items, info, total) = page(self.pull_requests.clone(), first, after);
        gi::PullRequestConnection {
            nodes: items.iter().map(|(_, p)| p.clone()).collect(),
            edges: items.into_iter().map(|(i, p)| gi::PullRequestEdge { cursor: cursor(i), node: p }).collect(),
            page_info: info,
            total_count: total as i32,
        }
    }
}

#[derive(SimpleObject)]
pub struct RequestedReviewerConnection {
    pub nodes: Vec<gi::RequestedReviewer>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

pub struct ProjectV2ItemFieldReviewerValue {
    pub id: ID,
    pub reviewers: Vec<gi::RequestedReviewer>,
    pub field: ProjectV2FieldConfiguration,
}

#[Object]
impl ProjectV2ItemFieldReviewerValue {
    async fn id(&self) -> &ID {
        &self.id
    }
    async fn field(&self) -> ProjectV2FieldConfiguration {
        self.field.clone()
    }
    async fn reviewers(&self, first: Option<i32>, after: Option<String>) -> RequestedReviewerConnection {
        let (items, info, total) = page(self.reviewers.clone(), first, after);
        RequestedReviewerConnection { nodes: items.into_iter().map(|(_, r)| r).collect(), page_info: info, total_count: total as i32 }
    }
}

#[derive(Union)]
pub enum ProjectV2ItemFieldValue {
    Text(ProjectV2ItemFieldTextValue),
    Number(ProjectV2ItemFieldNumberValue),
    Date(ProjectV2ItemFieldDateValue),
    SingleSelect(ProjectV2ItemFieldSingleSelectValue),
    Iteration(ProjectV2ItemFieldIterationValue),
    Label(ProjectV2ItemFieldLabelValue),
    User(ProjectV2ItemFieldUserValue),
    Milestone(ProjectV2ItemFieldMilestoneValue),
    Repository(ProjectV2ItemFieldRepositoryValue),
    PullRequest(ProjectV2ItemFieldPullRequestValue),
    Reviewer(ProjectV2ItemFieldReviewerValue),
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemFieldValueConnection {
    pub nodes: Vec<ProjectV2ItemFieldValue>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[Object(name = "ProjectV2Item")]
impl ProjectV2ItemObj {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(NODE_ITEM, self.it.id))
    }
    async fn database_id(&self) -> i64 {
        self.it.id
    }
    async fn is_archived(&self) -> bool {
        self.it.archived != 0
    }
    async fn created_at(&self) -> String {
        iso(self.it.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.it.updated_at)
    }
    async fn project(&self) -> ProjectV2Obj {
        self.project.clone()
    }
    #[graphql(name = "type")]
    async fn kind(&self) -> ProjectV2ItemType {
        match self.it.content_type.as_str() {
            "draft" => ProjectV2ItemType::DraftIssue,
            "pull" => ProjectV2ItemType::PullRequest,
            _ => ProjectV2ItemType::Issue,
        }
    }
    async fn content(&self, ctx: &Context<'_>) -> Result<Option<ProjectV2ItemContent>> {
        let c = Ctx::from(ctx);
        match (self.it.content_type.as_str(), self.it.content_id) {
            ("draft", _) => Ok(Some(ProjectV2ItemContent::DraftIssue(DraftIssue {
                id: ID(shapes::node_id(NODE_DRAFT, self.it.id)),
                title: self.it.draft_title.clone().unwrap_or_default(),
                body: self.it.draft_body.clone().unwrap_or_default(),
                created_at: iso(self.it.created_at),
                updated_at: iso(self.it.updated_at),
            }))),
            (_, Some(cid)) => {
                let Ok(i) = c.db.issue_full_by_id(Id(cid)).await else { return Ok(None) };
                let rw = c.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await.map_err(from_core)?;
                if !c.db.permission(c.viewer(), &rw).await.map_err(from_core)?.can_read() {
                    return Ok(None);
                }
                Ok(Some(match gi::wrap(&rw, i) {
                    IssueOrPullRequest::Issue(i) => ProjectV2ItemContent::Issue(i),
                    IssueOrPullRequest::PullRequest(p) => ProjectV2ItemContent::PullRequest(p),
                }))
            }
            _ => Ok(None),
        }
    }
    async fn field_values(&self, ctx: &Context<'_>, first: Option<i32>) -> Result<ProjectV2ItemFieldValueConnection> {
        let c = Ctx::from(ctx);
        let fields = c.db.fields(self.project.p.id()).await.map_err(from_core)?;
        let values = c.db.item_values(self.it.id()).await.map_err(from_core)?;
        let mut nodes = Vec::new();
        let pid = self.project.p.id;
        let vid = |k: &str| ID(format!("PVTFV_{}_{}", self.it.id, k));
        // System fields: Title is a text value; issue items also carry assignees, labels,
        // milestone and repository, like GitHub.
        let content: Option<(reef_db::repos::RepoWithOwner, IssueFull)> = match (self.it.content_type.as_str(), self.it.content_id) {
            ("draft", _) => None,
            (_, Some(cid)) => match c.db.issue_full_by_id(Id(cid)).await {
                Ok(i) => c.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await.ok().map(|rw| (rw, i)),
                Err(_) => None,
            },
            _ => None,
        };
        let title = match &content {
            Some((_, i)) => Some(i.issue.title.clone()),
            None => self.it.draft_title.clone(),
        };
        nodes.push(ProjectV2ItemFieldValue::Text(ProjectV2ItemFieldTextValue {
            id: vid("title"),
            text: title,
            field: system_field(pid, "title"),
        }));
        if let Some((rw, i)) = &content {
            if !i.assignees.is_empty() {
                nodes.push(ProjectV2ItemFieldValue::User(ProjectV2ItemFieldUserValue {
                    id: vid("assignees"),
                    users: i.assignees.iter().cloned().map(User).collect(),
                    field: system_field(pid, "assignees"),
                }));
            }
            if !i.labels.is_empty() {
                nodes.push(ProjectV2ItemFieldValue::Label(ProjectV2ItemFieldLabelValue {
                    id: vid("labels"),
                    labels: i.labels.iter().map(|l| gi::Label { repo: rw.clone(), l: l.clone() }).collect(),
                    field: system_field(pid, "labels"),
                }));
            }
            if let Some(m) = &i.milestone {
                nodes.push(ProjectV2ItemFieldValue::Milestone(ProjectV2ItemFieldMilestoneValue {
                    id: vid("milestone"),
                    milestone: Some(gi::Milestone { repo: rw.clone(), m: m.clone() }),
                    field: system_field(pid, "milestone"),
                }));
            }
            nodes.push(ProjectV2ItemFieldValue::Repository(ProjectV2ItemFieldRepositoryValue {
                id: vid("repository"),
                repository: Some(Repository(rw.clone())),
                field: system_field(pid, "repository"),
            }));
        }
        for (fid, val) in values {
            let Some(f) = fields.iter().find(|f| f.id == fid) else { continue };
            let Some(v) = val else { continue };
            let id = ID(format!("PVTFV_{}_{}", self.it.id, f.id));
            let cfg = field_config(f);
            nodes.push(match f.data_type.as_str() {
                "single_select" => ProjectV2ItemFieldValue::SingleSelect(ProjectV2ItemFieldSingleSelectValue {
                    id,
                    name: f.options().into_iter().find(|o| o.id == v).map(|o| o.name),
                    option_id: Some(v),
                    field: cfg,
                }),
                "number" => ProjectV2ItemFieldValue::Number(ProjectV2ItemFieldNumberValue { id, number: v.parse().ok(), field: cfg }),
                "date" => ProjectV2ItemFieldValue::Date(ProjectV2ItemFieldDateValue { id, date: Some(v), field: cfg }),
                _ => ProjectV2ItemFieldValue::Text(ProjectV2ItemFieldTextValue { id, text: Some(v), field: cfg }),
            });
        }
        let total = nodes.len();
        let n = first.unwrap_or(100).max(0) as usize;
        nodes.truncate(n);
        Ok(ProjectV2ItemFieldValueConnection {
            nodes,
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: total as i32,
        })
    }
    async fn field_value_by_name(&self, ctx: &Context<'_>, name: String) -> Result<Option<ProjectV2ItemFieldValue>> {
        let all = self.field_values(ctx, Some(100)).await?;
        Ok(all.nodes.into_iter().find(|v| {
            let f = match v {
                ProjectV2ItemFieldValue::Text(t) => &t.field,
                ProjectV2ItemFieldValue::Number(t) => &t.field,
                ProjectV2ItemFieldValue::Date(t) => &t.field,
                ProjectV2ItemFieldValue::SingleSelect(t) => &t.field,
                ProjectV2ItemFieldValue::Iteration(t) => &t.field,
                ProjectV2ItemFieldValue::Label(t) => &t.field,
                ProjectV2ItemFieldValue::User(t) => &t.field,
                ProjectV2ItemFieldValue::Milestone(t) => &t.field,
                ProjectV2ItemFieldValue::Repository(t) => &t.field,
                ProjectV2ItemFieldValue::PullRequest(t) => &t.field,
                ProjectV2ItemFieldValue::Reviewer(t) => &t.field,
            };
            cfg_name(f).eq_ignore_ascii_case(&name)
        }))
    }
}

fn cfg_name(c: &ProjectV2FieldConfiguration) -> &str {
    match c {
        ProjectV2FieldConfiguration::Field(f) => &f.name,
        ProjectV2FieldConfiguration::SingleSelectField(f) => &f.name,
        ProjectV2FieldConfiguration::IterationField(f) => &f.name,
    }
}

#[derive(SimpleObject)]
pub struct ProjectV2ItemConnection {
    pub nodes: Vec<ProjectV2ItemObj>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

/// Project items that reference the given issue or pull request.
pub async fn content_items(
    ctx: &Context<'_>,
    issue_id: i64,
    first: Option<i32>,
    after: Option<String>,
    include_archived: bool,
) -> Result<ProjectV2ItemConnection> {
    let s = super::app(ctx);
    let c = Ctx::from(ctx);
    let mut out = Vec::new();
    for it in c.db.items_for_content(Id(issue_id)).await.map_err(from_core)? {
        if !include_archived && it.archived != 0 {
            continue;
        }
        let Ok(p) = c.db.project_by_id(Id(it.project_id)).await else { continue };
        let owner = c.db.account_by_id(Id(p.owner_id)).await.map_err(from_core)?;
        if crate::projects::access(&s, c.viewer(), &owner, Some(&p)).await.map_err(|e| from_core(e.0))? >= crate::projects::Access::Read {
            out.push(ProjectV2ItemObj { project: ProjectV2Obj { owner, p }, it });
        }
    }
    let total = out.len();
    let (sel, info, _) = page(out, first, after);
    Ok(ProjectV2ItemConnection { nodes: sel.into_iter().map(|(_, it)| it).collect(), page_info: info, total_count: total as i32 })
}

// ----- helpers for owners -----

pub async fn owner_projects(
    ctx: &Context<'_>,
    owner: &Account,
    first: Option<i32>,
    after: Option<String>,
    query: Option<String>,
) -> Result<ProjectV2Connection> {
    let s = super::app(ctx);
    let c = Ctx::from(ctx);
    let a = crate::projects::access(&s, c.viewer(), owner, None).await.map_err(|e| from_core(e.0))?;
    let mut list = c.db.projects(owner.id(), true, a < crate::projects::Access::Write).await.map_err(from_core)?;
    if let Some(q) = query {
        let q = q.to_lowercase();
        if q.contains("is:open") {
            list.retain(|p| p.closed == 0);
        }
        if q.contains("is:closed") {
            list.retain(|p| p.closed != 0);
        }
        let words: Vec<&str> = q.split_whitespace().filter(|w| !w.contains(':')).collect();
        list.retain(|p| words.iter().all(|w| p.title.to_lowercase().contains(w)));
    }
    let total = list.len();
    let (items, info, _) = page(list, first, after);
    Ok(ProjectV2Connection {
        nodes: items.into_iter().map(|(_, p)| ProjectV2Obj { owner: owner.clone(), p }).collect(),
        page_info: info,
        total_count: total as i32,
    })
}

pub async fn owner_project(ctx: &Context<'_>, owner: &Account, number: i32) -> Result<Option<ProjectV2Obj>> {
    let s = super::app(ctx);
    let c = Ctx::from(ctx);
    match c.db.project_by_number(owner.id(), number as i64).await {
        Ok(p) => {
            let a = crate::projects::access(&s, c.viewer(), owner, Some(&p)).await.map_err(|e| from_core(e.0))?;
            if a >= crate::projects::Access::Read {
                Ok(Some(ProjectV2Obj { owner: owner.clone(), p }))
            } else {
                Err(not_found(format!("Could not resolve to a ProjectV2 with the number {number}.")))
            }
        }
        Err(_) => Err(not_found(format!("Could not resolve to a ProjectV2 with the number {number}."))),
    }
}

async fn load_project(ctx: &Context<'_>, id: &ID, min: crate::projects::Access) -> Result<ProjectV2Obj> {
    let s = super::app(ctx);
    let c = Ctx::from(ctx);
    let pid = gi::decode_id(id, NODE_PROJECT)?;
    let p = c.db.project_by_id(pid).await.map_err(|_| not_found("Could not resolve to a ProjectV2."))?;
    let owner = c.db.account_by_id(Id(p.owner_id)).await.map_err(from_core)?;
    let a = crate::projects::access(&s, c.viewer(), &owner, Some(&p)).await.map_err(|e| from_core(e.0))?;
    if a < min {
        return Err(if a == crate::projects::Access::None {
            not_found("Could not resolve to a ProjectV2.")
        } else {
            super::forbidden("You do not have permission to change this project.")
        });
    }
    Ok(ProjectV2Obj { owner, p })
}

// ----- mutations -----

#[derive(InputObject)]
#[graphql(name = "CreateProjectV2Input")]
pub struct CreateProjectV2Input {
    pub owner_id: ID,
    pub title: String,
    pub repository_id: Option<ID>,
    pub team_id: Option<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct ProjectV2Payload {
    pub project_v2: Option<ProjectV2Obj>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UpdateProjectV2Input")]
pub struct UpdateProjectV2Input {
    pub project_id: ID,
    pub title: Option<String>,
    pub short_description: Option<String>,
    pub readme: Option<String>,
    pub public: Option<bool>,
    pub closed: Option<bool>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DeleteProjectV2Input")]
pub struct DeleteProjectV2Input {
    pub project_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "AddProjectV2ItemByIdInput")]
pub struct AddProjectV2ItemByIdInput {
    pub project_id: ID,
    pub content_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct AddProjectV2ItemByIdPayload {
    pub item: Option<ProjectV2ItemObj>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "AddProjectV2DraftIssueInput")]
pub struct AddProjectV2DraftIssueInput {
    pub project_id: ID,
    pub title: String,
    pub body: Option<String>,
    pub assignee_ids: Option<Vec<ID>>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct AddProjectV2DraftIssuePayload {
    pub project_item: Option<ProjectV2ItemObj>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UpdateProjectV2DraftIssueInput")]
pub struct UpdateProjectV2DraftIssueInput {
    pub draft_issue_id: ID,
    pub title: Option<String>,
    pub body: Option<String>,
    pub assignee_ids: Option<Vec<ID>>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UpdateProjectV2DraftIssuePayload {
    pub draft_issue: Option<DraftIssue>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ProjectV2FieldValue")]
pub struct ProjectV2FieldValueInput {
    pub text: Option<String>,
    pub number: Option<f64>,
    pub date: Option<String>,
    pub single_select_option_id: Option<String>,
    pub iteration_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UpdateProjectV2ItemFieldValueInput")]
pub struct UpdateProjectV2ItemFieldValueInput {
    pub project_id: ID,
    pub item_id: ID,
    pub field_id: ID,
    pub value: ProjectV2FieldValueInput,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UpdateProjectV2ItemFieldValuePayload {
    pub project_v2_item: Option<ProjectV2ItemObj>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ClearProjectV2ItemFieldValueInput")]
pub struct ClearProjectV2ItemFieldValueInput {
    pub project_id: ID,
    pub item_id: ID,
    pub field_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DeleteProjectV2ItemInput")]
pub struct DeleteProjectV2ItemInput {
    pub project_id: ID,
    pub item_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct DeleteProjectV2ItemPayload {
    pub deleted_item_id: Option<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ArchiveProjectV2ItemInput")]
pub struct ArchiveProjectV2ItemInput {
    pub project_id: ID,
    pub item_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct ArchiveProjectV2ItemPayload {
    pub item: Option<ProjectV2ItemObj>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ProjectV2SingleSelectFieldOptionInput")]
pub struct ProjectV2SingleSelectFieldOptionInput {
    pub name: String,
    pub color: ProjectV2SingleSelectFieldOptionColor,
    pub description: String,
}

#[derive(InputObject)]
#[graphql(name = "CreateProjectV2FieldInput")]
pub struct CreateProjectV2FieldInput {
    pub project_id: ID,
    pub data_type: ProjectV2CustomFieldType,
    pub name: String,
    pub single_select_options: Option<Vec<ProjectV2SingleSelectFieldOptionInput>>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct CreateProjectV2FieldPayload {
    pub project_v2_field: Option<ProjectV2FieldConfiguration>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DeleteProjectV2FieldInput")]
pub struct DeleteProjectV2FieldInput {
    pub field_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "LinkProjectV2ToRepositoryInput")]
pub struct LinkProjectV2ToRepositoryInput {
    pub project_id: ID,
    pub repository_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UnlinkProjectV2FromRepositoryInput")]
pub struct UnlinkProjectV2FromRepositoryInput {
    pub project_id: ID,
    pub repository_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UnlinkProjectV2FromRepositoryPayload {
    pub repository: Option<Repository>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UnarchiveProjectV2ItemInput")]
pub struct UnarchiveProjectV2ItemInput {
    pub project_id: ID,
    pub item_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UnarchiveProjectV2ItemPayload {
    pub item: Option<ProjectV2ItemObj>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct LinkProjectV2ToRepositoryPayload {
    pub repository: Option<Repository>,
    pub client_mutation_id: Option<String>,
}

#[derive(Default)]
pub struct ProjectMutations;

fn color_name(c: ProjectV2SingleSelectFieldOptionColor) -> &'static str {
    match c {
        ProjectV2SingleSelectFieldOptionColor::Gray => "GRAY",
        ProjectV2SingleSelectFieldOptionColor::Blue => "BLUE",
        ProjectV2SingleSelectFieldOptionColor::Green => "GREEN",
        ProjectV2SingleSelectFieldOptionColor::Yellow => "YELLOW",
        ProjectV2SingleSelectFieldOptionColor::Orange => "ORANGE",
        ProjectV2SingleSelectFieldOptionColor::Red => "RED",
        ProjectV2SingleSelectFieldOptionColor::Pink => "PINK",
        ProjectV2SingleSelectFieldOptionColor::Purple => "PURPLE",
    }
}

#[Object]
impl ProjectMutations {
    #[graphql(name = "createProjectV2")]
    async fn create_project_v2(&self, ctx: &Context<'_>, input: CreateProjectV2Input) -> Result<ProjectV2Payload> {
        let s = super::app(ctx);
        let c = Ctx::from(ctx);
        let Some(viewer) = c.viewer().cloned() else {
            return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated."));
        };
        let (_, num) = shapes::decode_node_id(&input.owner_id.0).ok_or_else(|| not_found("Could not resolve to a ProjectV2Owner."))?;
        let owner = c.db.account_by_id(Id(num)).await.map_err(|_| not_found("Could not resolve to a ProjectV2Owner."))?;
        if crate::projects::access(&s, Some(&viewer), &owner, None).await.map_err(|e| from_core(e.0))? < crate::projects::Access::Write {
            return Err(super::forbidden("You cannot create projects for this owner."));
        }
        let p = c.db.create_project(owner.id(), viewer.id(), &input.title, None, false).await.map_err(from_core)?;
        if let Some(rid) = input.repository_id
            && let Ok(id) = gi::decode_id(&rid, shapes::NODE_REPO)
        {
            let _ = c.db.link_repo(p.id(), id).await;
        }
        Ok(ProjectV2Payload { project_v2: Some(ProjectV2Obj { owner, p }), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "updateProjectV2")]
    async fn update_project_v2(&self, ctx: &Context<'_>, input: UpdateProjectV2Input) -> Result<ProjectV2Payload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Admin).await?;
        let p =
            c.db.update_project(
                po.p.id(),
                input.title.as_deref(),
                input.short_description.as_deref().map(Some),
                input.readme.as_deref().map(Some),
                input.public,
                input.closed,
            )
            .await
            .map_err(from_core)?;
        Ok(ProjectV2Payload { project_v2: Some(ProjectV2Obj { owner: po.owner, p }), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "deleteProjectV2")]
    async fn delete_project_v2(&self, ctx: &Context<'_>, input: DeleteProjectV2Input) -> Result<ProjectV2Payload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Admin).await?;
        c.db.delete_project(po.p.id()).await.map_err(from_core)?;
        Ok(ProjectV2Payload { project_v2: Some(po), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "addProjectV2ItemById")]
    async fn add_project_v2_item_by_id(&self, ctx: &Context<'_>, input: AddProjectV2ItemByIdInput) -> Result<AddProjectV2ItemByIdPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let (rw, i) = gi::load_issue(ctx, &input.content_id).await?;
        let _ = rw;
        // GitHub returns the existing item when the content is already in the project.
        let existing = c.db.items_for_content(Id(i.issue.id)).await.map_err(from_core)?.into_iter().find(|it| it.project_id == po.p.id);
        let it = match existing {
            Some(it) => it,
            None => {
                let it =
                    c.db.add_item(
                        po.p.id(),
                        if i.issue.is_pull() { "pull" } else { "issue" },
                        Some(i.issue.id),
                        None,
                        None,
                        c.viewer().map(|v| v.id()),
                    )
                    .await
                    .map_err(from_core)?;
                c.db.default_status(po.p.id(), it.id()).await.map_err(from_core)?;
                it
            }
        };
        Ok(AddProjectV2ItemByIdPayload { item: Some(ProjectV2ItemObj { project: po, it }), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "addProjectV2DraftIssue")]
    async fn add_project_v2_draft_issue(
        &self,
        ctx: &Context<'_>,
        input: AddProjectV2DraftIssueInput,
    ) -> Result<AddProjectV2DraftIssuePayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let it =
            c.db.add_item(po.p.id(), "draft", None, Some(input.title.trim()), input.body.as_deref(), c.viewer().map(|v| v.id()))
                .await
                .map_err(from_core)?;
        c.db.default_status(po.p.id(), it.id()).await.map_err(from_core)?;
        Ok(AddProjectV2DraftIssuePayload {
            project_item: Some(ProjectV2ItemObj { project: po, it }),
            client_mutation_id: input.client_mutation_id,
        })
    }

    #[graphql(name = "updateProjectV2DraftIssue")]
    async fn update_project_v2_draft_issue(
        &self,
        ctx: &Context<'_>,
        input: UpdateProjectV2DraftIssueInput,
    ) -> Result<UpdateProjectV2DraftIssuePayload> {
        let c = Ctx::from(ctx);
        let iid = gi::decode_id(&input.draft_issue_id, NODE_DRAFT)?;
        let it = c.db.item_by_id(iid).await.map_err(|_| not_found("Could not resolve to a DraftIssue."))?;
        let p = c.db.project_by_id(Id(it.project_id)).await.map_err(from_core)?;
        load_project(ctx, &ID(shapes::node_id(NODE_PROJECT, p.id)), crate::projects::Access::Write).await?;
        let it = c.db.update_draft(iid, input.title.as_deref(), input.body.as_deref().map(Some)).await.map_err(from_core)?;
        Ok(UpdateProjectV2DraftIssuePayload {
            draft_issue: Some(DraftIssue {
                id: ID(shapes::node_id(NODE_DRAFT, it.id)),
                title: it.draft_title.clone().unwrap_or_default(),
                body: it.draft_body.clone().unwrap_or_default(),
                created_at: iso(it.created_at),
                updated_at: iso(it.updated_at),
            }),
            client_mutation_id: input.client_mutation_id,
        })
    }

    #[graphql(name = "updateProjectV2ItemFieldValue")]
    async fn update_project_v2_item_field_value(
        &self,
        ctx: &Context<'_>,
        input: UpdateProjectV2ItemFieldValueInput,
    ) -> Result<UpdateProjectV2ItemFieldValuePayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let iid = gi::decode_id(&input.item_id, NODE_ITEM)?;
        let it = c.db.item_by_id(iid).await.map_err(|_| not_found("Could not resolve to a ProjectV2Item."))?;
        if it.project_id != po.p.id {
            return Err(not_found("Could not resolve to a ProjectV2Item."));
        }
        if input.field_id.0.ends_with("_title") {
            if it.content_type == "draft" {
                c.db.update_draft(iid, input.value.text.as_deref(), None).await.map_err(from_core)?;
            }
            return Ok(UpdateProjectV2ItemFieldValuePayload {
                project_v2_item: Some(ProjectV2ItemObj { project: po, it }),
                client_mutation_id: input.client_mutation_id,
            });
        }
        let fid = gi::decode_id(&input.field_id, NODE_FIELD)?;
        let f = c.db.field_by_id(fid).await.map_err(|_| not_found("Could not resolve to a ProjectV2Field."))?;
        let value = match f.data_type.as_str() {
            "single_select" => {
                let opt = input
                    .value
                    .single_select_option_id
                    .ok_or_else(|| gql_error("UNPROCESSABLE", "singleSelectOptionId is required for a single select field"))?;
                if !f.options().iter().any(|o| o.id == opt) {
                    return Err(gql_error("UNPROCESSABLE", "unknown option"));
                }
                opt
            }
            "number" => input.value.number.map(|n| n.to_string()).ok_or_else(|| gql_error("UNPROCESSABLE", "number is required"))?,
            "date" => input.value.date.ok_or_else(|| gql_error("UNPROCESSABLE", "date is required"))?,
            _ => input.value.text.ok_or_else(|| gql_error("UNPROCESSABLE", "text is required"))?,
        };
        c.db.set_item_value(iid, fid, Some(&value)).await.map_err(from_core)?;
        let it = c.db.item_by_id(iid).await.map_err(from_core)?;
        Ok(UpdateProjectV2ItemFieldValuePayload {
            project_v2_item: Some(ProjectV2ItemObj { project: po, it }),
            client_mutation_id: input.client_mutation_id,
        })
    }

    #[graphql(name = "clearProjectV2ItemFieldValue")]
    async fn clear_project_v2_item_field_value(
        &self,
        ctx: &Context<'_>,
        input: ClearProjectV2ItemFieldValueInput,
    ) -> Result<UpdateProjectV2ItemFieldValuePayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let iid = gi::decode_id(&input.item_id, NODE_ITEM)?;
        let fid = gi::decode_id(&input.field_id, NODE_FIELD)?;
        c.db.set_item_value(iid, fid, None).await.map_err(from_core)?;
        let it = c.db.item_by_id(iid).await.map_err(from_core)?;
        Ok(UpdateProjectV2ItemFieldValuePayload {
            project_v2_item: Some(ProjectV2ItemObj { project: po, it }),
            client_mutation_id: input.client_mutation_id,
        })
    }

    #[graphql(name = "deleteProjectV2Item")]
    async fn delete_project_v2_item(&self, ctx: &Context<'_>, input: DeleteProjectV2ItemInput) -> Result<DeleteProjectV2ItemPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let iid = gi::decode_id(&input.item_id, NODE_ITEM)?;
        let it = c.db.item_by_id(iid).await.map_err(|_| not_found("Could not resolve to a ProjectV2Item."))?;
        if it.project_id != po.p.id {
            return Err(not_found("Could not resolve to a ProjectV2Item."));
        }
        c.db.delete_item(iid).await.map_err(from_core)?;
        Ok(DeleteProjectV2ItemPayload { deleted_item_id: Some(input.item_id), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "archiveProjectV2Item")]
    async fn archive_project_v2_item(&self, ctx: &Context<'_>, input: ArchiveProjectV2ItemInput) -> Result<ArchiveProjectV2ItemPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let iid = gi::decode_id(&input.item_id, NODE_ITEM)?;
        c.db.set_item_archived(iid, true).await.map_err(from_core)?;
        let it = c.db.item_by_id(iid).await.map_err(from_core)?;
        Ok(ArchiveProjectV2ItemPayload { item: Some(ProjectV2ItemObj { project: po, it }), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "unarchiveProjectV2Item")]
    async fn unarchive_project_v2_item(
        &self,
        ctx: &Context<'_>,
        input: UnarchiveProjectV2ItemInput,
    ) -> Result<UnarchiveProjectV2ItemPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let iid = gi::decode_id(&input.item_id, NODE_ITEM)?;
        c.db.set_item_archived(iid, false).await.map_err(from_core)?;
        let it = c.db.item_by_id(iid).await.map_err(from_core)?;
        Ok(UnarchiveProjectV2ItemPayload { item: Some(ProjectV2ItemObj { project: po, it }), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "createProjectV2Field")]
    async fn create_project_v2_field(&self, ctx: &Context<'_>, input: CreateProjectV2FieldInput) -> Result<CreateProjectV2FieldPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Admin).await?;
        let dt = match input.data_type {
            ProjectV2CustomFieldType::Text => "text",
            ProjectV2CustomFieldType::SingleSelect => "single_select",
            ProjectV2CustomFieldType::Number => "number",
            ProjectV2CustomFieldType::Date => "date",
            ProjectV2CustomFieldType::Iteration => "iteration",
        };
        let opts: Vec<reef_db::projects::FieldOption> = input
            .single_select_options
            .unwrap_or_default()
            .into_iter()
            .map(|o| reef_db::projects::FieldOption {
                id: reef_core::token::random_hex(4),
                name: o.name,
                color: color_name(o.color).into(),
                description: o.description,
            })
            .collect();
        let f = c.db.create_field(po.p.id(), &input.name, dt, &opts).await.map_err(from_core)?;
        Ok(CreateProjectV2FieldPayload { project_v2_field: Some(field_config(&f)), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "deleteProjectV2Field")]
    async fn delete_project_v2_field(&self, ctx: &Context<'_>, input: DeleteProjectV2FieldInput) -> Result<CreateProjectV2FieldPayload> {
        let c = Ctx::from(ctx);
        let fid = gi::decode_id(&input.field_id, NODE_FIELD)?;
        let f = c.db.field_by_id(fid).await.map_err(|_| not_found("Could not resolve to a ProjectV2Field."))?;
        load_project(ctx, &ID(shapes::node_id(NODE_PROJECT, f.project_id)), crate::projects::Access::Admin).await?;
        c.db.delete_field(fid).await.map_err(from_core)?;
        Ok(CreateProjectV2FieldPayload { project_v2_field: Some(field_config(&f)), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "linkProjectV2ToRepository")]
    async fn link_project_v2_to_repository(
        &self,
        ctx: &Context<'_>,
        input: LinkProjectV2ToRepositoryInput,
    ) -> Result<LinkProjectV2ToRepositoryPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let rid = gi::decode_id(&input.repository_id, shapes::NODE_REPO)?;
        let rw = c.db.repo_with_owner_by_id(rid).await.map_err(|_| not_found("Could not resolve to a Repository."))?;
        if c.db.permission(c.viewer(), &rw).await.map_err(from_core)? < Permission::Write {
            return Err(super::forbidden("Must have write access to the repository."));
        }
        c.db.link_repo(po.p.id(), rid).await.map_err(from_core)?;
        Ok(LinkProjectV2ToRepositoryPayload { repository: Some(Repository(rw)), client_mutation_id: input.client_mutation_id })
    }

    #[graphql(name = "unlinkProjectV2FromRepository")]
    async fn unlink_project_v2_from_repository(
        &self,
        ctx: &Context<'_>,
        input: UnlinkProjectV2FromRepositoryInput,
    ) -> Result<UnlinkProjectV2FromRepositoryPayload> {
        let c = Ctx::from(ctx);
        let po = load_project(ctx, &input.project_id, crate::projects::Access::Write).await?;
        let rid = gi::decode_id(&input.repository_id, shapes::NODE_REPO)?;
        c.db.unlink_repo(po.p.id(), rid).await.map_err(from_core)?;
        let rw = c.db.repo_with_owner_by_id(rid).await.ok();
        Ok(UnlinkProjectV2FromRepositoryPayload { repository: rw.map(Repository), client_mutation_id: input.client_mutation_id })
    }
}
