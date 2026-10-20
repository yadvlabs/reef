//! Issues, pull requests (the parts shared with issues), labels, milestones, comments,
//! reactions. Field names follow GitHub's schema; pull-request-only fields are stubs
//! until the pull request slice fills them in.

use super::pulls::mergeable;
use super::types::*;
use super::{Ctx, from_core, gql_error, not_found};
use crate::shapes;
use async_graphql::{Context, Enum, ID, InputObject, Object, Result, SimpleObject, Union};
use reef_core::Id;
use reef_core::time::{iso, iso_opt};
use reef_db::accounts::Account;
use reef_db::issues::{Comment, IssueFilter, IssueFull, Label as DbLabel, Milestone as DbMilestone};
use reef_db::repos::{Permission, RepoWithOwner};

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum IssueStateReason {
    Completed,
    NotPlanned,
    Reopened,
    Duplicate,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum IssueOrderField {
    CreatedAt,
    UpdatedAt,
    Comments,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum IssueCommentOrderField {
    UpdatedAt,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum CommentAuthorAssociation {
    Member,
    Owner,
    Mannequin,
    Collaborator,
    Contributor,
    FirstTimeContributor,
    FirstTimer,
    None,
}

impl CommentAuthorAssociation {
    #[allow(clippy::should_implement_trait)] // infallible: unknown names map to None
    pub fn from_str(s: &str) -> Self {
        match s {
            "OWNER" => Self::Owner,
            "MEMBER" => Self::Member,
            "COLLABORATOR" => Self::Collaborator,
            "CONTRIBUTOR" => Self::Contributor,
            _ => Self::None,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ReactionContent {
    ThumbsUp,
    ThumbsDown,
    Laugh,
    Hooray,
    Confused,
    Heart,
    Rocket,
    Eyes,
}

impl ReactionContent {
    pub fn key(self) -> &'static str {
        match self {
            Self::ThumbsUp => "+1",
            Self::ThumbsDown => "-1",
            Self::Laugh => "laugh",
            Self::Hooray => "hooray",
            Self::Confused => "confused",
            Self::Heart => "heart",
            Self::Rocket => "rocket",
            Self::Eyes => "eyes",
        }
    }
    pub const ALL: [ReactionContent; 8] =
        [Self::ThumbsUp, Self::ThumbsDown, Self::Laugh, Self::Hooray, Self::Confused, Self::Heart, Self::Rocket, Self::Eyes];
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum LockReason {
    OffTopic,
    TooHeated,
    Resolved,
    Spam,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PullRequestReviewState {
    Pending,
    Commented,
    Approved,
    ChangesRequested,
    Dismissed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PullRequestReviewDecision {
    ChangesRequested,
    Approved,
    ReviewRequired,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum MergeableState {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum MergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unknown,
    Unstable,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PatchStatus {
    Added,
    Deleted,
    Renamed,
    Copied,
    Modified,
    Changed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum StatusState {
    Expected,
    Error,
    Failure,
    Pending,
    Success,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum CheckStatusState {
    Queued,
    InProgress,
    Completed,
    Waiting,
    Pending,
    Requested,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum CheckConclusionState {
    ActionRequired,
    TimedOut,
    Cancelled,
    Failure,
    Success,
    Neutral,
    Skipped,
    StartupFailure,
    Stale,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ActorCapability {
    CanBeAssigned,
}

#[derive(InputObject)]
pub struct IssueOrder {
    pub field: IssueOrderField,
    pub direction: OrderDirection,
}

#[derive(InputObject, Default)]
pub struct IssueFilters {
    pub assignee: Option<String>,
    pub created_by: Option<String>,
    pub labels: Option<Vec<String>>,
    pub mentioned: Option<String>,
    pub milestone: Option<String>,
    pub milestone_number: Option<String>,
    pub since: Option<String>,
    pub states: Option<Vec<IssueState>>,
    pub viewer_subscribed: Option<bool>,
}

// ----- small objects -----

#[derive(SimpleObject)]
pub struct ReactingUserConnection {
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct ReactionGroup {
    pub content: ReactionContent,
    pub users: ReactingUserConnection,
    pub viewer_has_reacted: bool,
    pub created_at: Option<String>,
}

async fn reaction_groups(ctx: &Context<'_>, subject_type: &str, id: i64) -> Result<Vec<ReactionGroup>> {
    let c = Ctx::from(ctx);
    let (counts, mine) = c.db.reaction_summary(subject_type, &[id], c.viewer().map(|v| v.id)).await.map_err(from_core)?;
    Ok(ReactionContent::ALL
        .iter()
        .map(|r| ReactionGroup {
            content: *r,
            users: ReactingUserConnection { total_count: counts.get(&(id, r.key().to_string())).copied().unwrap_or(0) as i32 },
            viewer_has_reacted: mine.contains(&(id, r.key().to_string())),
            created_at: None,
        })
        .collect())
}

#[derive(Clone)]
pub struct Label {
    pub repo: RepoWithOwner,
    pub l: DbLabel,
}

#[Object]
impl Label {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_LABEL, self.l.id))
    }
    async fn name(&self) -> &str {
        &self.l.name
    }
    async fn color(&self) -> &str {
        &self.l.color
    }
    async fn description(&self) -> Option<&str> {
        self.l.description.as_deref()
    }
    async fn is_default(&self) -> bool {
        self.l.is_default != 0
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}/labels/{}", Ctx::from(ctx).config.site_url(), self.repo.full_name(), shapes::urlpath(&self.l.name))
    }
    async fn resource_path(&self) -> String {
        format!("/{}/labels/{}", self.repo.full_name(), shapes::urlpath(&self.l.name))
    }
    async fn created_at(&self) -> String {
        iso(self.l.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.l.updated_at)
    }
    async fn repository(&self) -> Repository {
        Repository(self.repo.clone())
    }
    async fn issues(&self, ctx: &Context<'_>, states: Option<Vec<IssueState>>) -> Result<CountConnection> {
        let c = Ctx::from(ctx);
        let f = IssueFilter {
            state: state_filter(states.as_deref()),
            is_pull: Some(false),
            labels: vec![self.l.name.clone()],
            ..Default::default()
        };
        let (_, n) = c.db.list_issues(Some(self.repo.repo.id()), &f, 1, 1).await.map_err(from_core)?;
        Ok(CountConnection { total_count: n as i32 })
    }
    async fn pull_requests(&self, ctx: &Context<'_>, states: Option<Vec<PullRequestState>>) -> Result<CountConnection> {
        let c = Ctx::from(ctx);
        let st = match states.as_deref() {
            Some(s) if s.contains(&PullRequestState::Open) && s.len() == 1 => "open",
            Some(s) if !s.contains(&PullRequestState::Open) => "closed",
            _ => "all",
        };
        let f = IssueFilter { state: st.into(), is_pull: Some(true), labels: vec![self.l.name.clone()], ..Default::default() };
        let (_, n) = c.db.list_issues(Some(self.repo.repo.id()), &f, 1, 1).await.map_err(from_core)?;
        Ok(CountConnection { total_count: n as i32 })
    }
}

#[derive(SimpleObject)]
pub struct LabelEdge {
    pub cursor: String,
    pub node: Label,
}

#[derive(SimpleObject)]
pub struct LabelConnection2 {
    pub nodes: Vec<Label>,
    pub edges: Vec<LabelEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(Clone)]
pub struct Milestone {
    pub repo: RepoWithOwner,
    pub m: DbMilestone,
}

#[Object]
impl Milestone {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_MILESTONE, self.m.id))
    }
    async fn number(&self) -> i32 {
        self.m.number as i32
    }
    async fn title(&self) -> &str {
        &self.m.title
    }
    async fn description(&self) -> Option<&str> {
        self.m.description.as_deref()
    }
    async fn due_on(&self) -> Option<String> {
        iso_opt(self.m.due_on)
    }
    async fn state(&self) -> MilestoneState {
        if self.m.state == "closed" { MilestoneState::Closed } else { MilestoneState::Open }
    }
    async fn closed(&self) -> bool {
        self.m.state == "closed"
    }
    async fn closed_at(&self) -> Option<String> {
        iso_opt(self.m.closed_at)
    }
    async fn created_at(&self) -> String {
        iso(self.m.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.m.updated_at)
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}/milestone/{}", Ctx::from(ctx).config.site_url(), self.repo.full_name(), self.m.number)
    }
    async fn resource_path(&self) -> String {
        format!("/{}/milestone/{}", self.repo.full_name(), self.m.number)
    }
    async fn progress_percentage(&self, ctx: &Context<'_>) -> Result<f64> {
        let (o, cl) = Ctx::from(ctx).db.milestone_counts(Id(self.m.id)).await.map_err(from_core)?;
        Ok(if o + cl == 0 { 0.0 } else { cl as f64 * 100.0 / (o + cl) as f64 })
    }
    async fn issues(&self, ctx: &Context<'_>, states: Option<Vec<IssueState>>) -> Result<CountConnection> {
        let c = Ctx::from(ctx);
        let f = IssueFilter {
            state: state_filter(states.as_deref()),
            is_pull: Some(false),
            milestone: Some(self.m.number.to_string()),
            ..Default::default()
        };
        let (_, n) = c.db.list_issues(Some(self.repo.repo.id()), &f, 1, 1).await.map_err(from_core)?;
        Ok(CountConnection { total_count: n as i32 })
    }
    async fn pull_requests(&self) -> CountConnection {
        CountConnection { total_count: 0 }
    }
    async fn repository(&self) -> Repository {
        Repository(self.repo.clone())
    }
}

#[derive(SimpleObject)]
pub struct MilestoneConnection2 {
    pub nodes: Vec<Milestone>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

pub fn state_filter(states: Option<&[IssueState]>) -> String {
    match states {
        None => "all".into(),
        Some(s) if s.contains(&IssueState::Open) && s.contains(&IssueState::Closed) => "all".into(),
        Some(s) if s.contains(&IssueState::Closed) => "closed".into(),
        Some([]) => "all".into(),
        Some(_) => "open".into(),
    }
}

// ----- comments -----

#[derive(Clone)]
pub struct IssueComment {
    pub repo: RepoWithOwner,
    pub issue: IssueFull,
    pub c: Comment,
}

#[Object]
impl IssueComment {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_COMMENT, self.c.id))
    }
    async fn database_id(&self) -> i64 {
        self.c.id
    }
    async fn body(&self) -> &str {
        &self.c.body
    }
    #[graphql(name = "bodyHTML")]
    async fn body_html(&self, ctx: &Context<'_>) -> String {
        crate::render::markdown_in_repo(&self.c.body, Ctx::from(ctx).config, &self.repo)
    }
    async fn body_text(&self) -> String {
        self.c.body.clone()
    }
    async fn author(&self, ctx: &Context<'_>) -> Result<Option<User>> {
        Ok(Ctx::from(ctx).db.account_by_id(Id(self.c.author_id)).await.ok().map(User))
    }
    async fn author_association(&self, ctx: &Context<'_>) -> Result<CommentAuthorAssociation> {
        let c = Ctx::from(ctx);
        let a = c.db.account_by_id(Id(self.c.author_id)).await.map_err(from_core)?;
        Ok(CommentAuthorAssociation::from_str(association(ctx, &self.repo, &a).await))
    }
    async fn created_at(&self) -> String {
        iso(self.c.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.c.updated_at)
    }
    async fn last_edited_at(&self) -> Option<String> {
        if self.c.updated_at > self.c.created_at { Some(iso(self.c.updated_at)) } else { None }
    }
    async fn includes_created_edit(&self) -> bool {
        self.c.updated_at > self.c.created_at
    }
    async fn is_minimized(&self) -> bool {
        false
    }
    async fn minimized_reason(&self) -> Option<String> {
        None
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!(
            "{}/{}/issues/{}#issuecomment-{}",
            Ctx::from(ctx).config.site_url(),
            self.repo.full_name(),
            self.issue.issue.number,
            self.c.id
        )
    }
    async fn resource_path(&self) -> String {
        format!("/{}/issues/{}#issuecomment-{}", self.repo.full_name(), self.issue.issue.number, self.c.id)
    }
    async fn viewer_did_author(&self, ctx: &Context<'_>) -> bool {
        Ctx::from(ctx).viewer().map(|v| v.id) == Some(self.c.author_id)
    }
    async fn viewer_can_update(&self, ctx: &Context<'_>) -> Result<bool> {
        let c = Ctx::from(ctx);
        let Some(v) = c.viewer() else { return Ok(false) };
        if v.id == self.c.author_id {
            return Ok(true);
        }
        Ok(c.db.permission(Some(v), &self.repo).await.map_err(from_core)? >= Permission::Write)
    }
    async fn viewer_can_delete(&self, ctx: &Context<'_>) -> Result<bool> {
        self.viewer_can_update(ctx).await
    }
    async fn viewer_can_react(&self) -> bool {
        true
    }
    async fn reaction_groups(&self, ctx: &Context<'_>) -> Result<Vec<ReactionGroup>> {
        reaction_groups(ctx, "issue_comment", self.c.id).await
    }
    async fn issue(&self) -> Issue {
        Issue { repo: self.repo.clone(), i: self.issue.clone() }
    }
    async fn repository(&self) -> Repository {
        Repository(self.repo.clone())
    }
}

#[derive(SimpleObject)]
pub struct IssueCommentEdge {
    pub cursor: String,
    pub node: IssueComment,
}

#[derive(SimpleObject)]
pub struct IssueCommentConnection {
    pub nodes: Vec<IssueComment>,
    pub edges: Vec<IssueCommentEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

pub async fn association(ctx: &Context<'_>, rw: &RepoWithOwner, a: &Account) -> &'static str {
    let c = Ctx::from(ctx);
    let perm = c.db.permission(Some(a), rw).await.unwrap_or(Permission::None);
    let is_member = rw.owner.is_org() && c.db.org_membership(rw.owner.id(), a.id()).await.ok().flatten().is_some();
    shapes::author_association(perm, a.id == rw.owner.id, is_member)
}

// ----- shared implementation for Issue and PullRequest -----

macro_rules! issue_like_impl {
    ($ty:ident, { $($extra:tt)* }) => {
        #[Object]
        impl $ty {
            $($extra)*
        async fn database_id(&self) -> i64 {
            self.i.issue.id
        }
        async fn number(&self) -> i32 {
            self.i.issue.number as i32
        }
        async fn title(&self) -> &str {
            &self.i.issue.title
        }
        #[graphql(name = "titleHTML")]
        async fn title_html(&self) -> String {
            ammonia::clean_text(&self.i.issue.title)
        }
        async fn body(&self) -> String {
            self.i.issue.body.clone().unwrap_or_default()
        }
        #[graphql(name = "bodyHTML")]
        async fn body_html(&self, ctx: &Context<'_>) -> String {
            crate::render::markdown_in_repo(self.i.issue.body.as_deref().unwrap_or(""), Ctx::from(ctx).config, &self.repo)
        }
        async fn body_text(&self) -> String {
            self.i.issue.body.clone().unwrap_or_default()
        }
        async fn closed(&self) -> bool {
            !self.i.issue.is_open()
        }
        async fn closed_at(&self) -> Option<String> {
            iso_opt(self.i.issue.closed_at)
        }
        async fn created_at(&self) -> String {
            iso(self.i.issue.created_at)
        }
        async fn updated_at(&self) -> String {
            iso(self.i.issue.updated_at)
        }
        async fn last_edited_at(&self) -> Option<String> {
            None
        }
        async fn locked(&self) -> bool {
            self.i.issue.locked != 0
        }
        async fn active_lock_reason(&self) -> Option<LockReason> {
            match self.i.issue.lock_reason.as_deref() {
                Some("off-topic") => Some(LockReason::OffTopic),
                Some("too heated") => Some(LockReason::TooHeated),
                Some("resolved") => Some(LockReason::Resolved),
                Some("spam") => Some(LockReason::Spam),
                _ => None,
            }
        }
        async fn author(&self) -> Option<User> {
            Some(User(self.i.author.clone()))
        }
        async fn author_association(&self, ctx: &Context<'_>) -> CommentAuthorAssociation {
            CommentAuthorAssociation::from_str(association(ctx, &self.repo, &self.i.author).await)
        }
        async fn editor(&self) -> Option<User> {
            None
        }
        async fn assignees(&self, first: Option<i32>, after: Option<String>) -> UserConnection {
            let (items, info, total) = page(self.i.assignees.clone(), first, after);
            UserConnection { nodes: items.into_iter().map(|(_, a)| User(a)).collect(), page_info: info, total_count: total as i32 }
        }
        async fn assigned_actors(&self, first: Option<i32>, after: Option<String>) -> AssigneeConnection {
            let (items, info, total) = page(self.i.assignees.clone(), first, after);
            AssigneeConnection { nodes: items.into_iter().map(|(_, a)| Assignee::User(User(a))).collect(), page_info: info, total_count: total as i32 }
        }
        async fn labels(&self, first: Option<i32>, after: Option<String>, _order_by: Option<LabelOrder>) -> LabelConnection2 {
            let (items, info, total) = page(self.i.labels.clone(), first, after);
            LabelConnection2 {
                nodes: items.iter().map(|(_, l)| Label { repo: self.repo.clone(), l: l.clone() }).collect(),
                edges: items.into_iter().map(|(i, l)| LabelEdge { cursor: cursor(i), node: Label { repo: self.repo.clone(), l } }).collect(),
                page_info: info,
                total_count: total as i32,
            }
        }
        async fn milestone(&self) -> Option<Milestone> {
            self.i.milestone.clone().map(|m| Milestone { repo: self.repo.clone(), m })
        }
        async fn comments(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>, last: Option<i32>, _order_by: Option<IssueCommentOrder>) -> Result<IssueCommentConnection> {
            let c = Ctx::from(ctx);
            let mut all = c.db.comments(self.i.issue.id(), None).await.map_err(from_core)?;
            let total = all.len();
            if let Some(l) = last {
                let l = l.max(0) as usize;
                if all.len() > l {
                    all = all.split_off(all.len() - l);
                }
                let nodes: Vec<IssueComment> = all.into_iter().map(|cm| IssueComment { repo: self.repo.clone(), issue: self.i.clone(), c: cm }).collect();
                return Ok(IssueCommentConnection {
                    edges: nodes.iter().enumerate().map(|(i, n)| IssueCommentEdge { cursor: cursor(i), node: n.clone() }).collect(),
                    nodes,
                    page_info: PageInfo { has_next_page: false, has_previous_page: total > l, start_cursor: None, end_cursor: None },
                    total_count: total as i32,
                });
            }
            let (items, info, total) = page(all, first, after);
            let nodes: Vec<IssueComment> = items.iter().map(|(_, cm)| IssueComment { repo: self.repo.clone(), issue: self.i.clone(), c: cm.clone() }).collect();
            Ok(IssueCommentConnection {
                edges: items.into_iter().map(|(i, cm)| IssueCommentEdge { cursor: cursor(i), node: IssueComment { repo: self.repo.clone(), issue: self.i.clone(), c: cm } }).collect(),
                nodes,
                page_info: info,
                total_count: total as i32,
            })
        }
        async fn reaction_groups(&self, ctx: &Context<'_>) -> Result<Vec<ReactionGroup>> {
            reaction_groups(ctx, "issue", self.i.issue.id).await
        }
        async fn repository(&self) -> Repository {
            Repository(self.repo.clone())
        }
        async fn project_cards(&self, _first: Option<i32>) -> ProjectCardConnection {
            ProjectCardConnection { nodes: vec![], total_count: 0 }
        }
        async fn project_items(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>, include_archived: Option<bool>) -> Result<super::projects::ProjectV2ItemConnection> {
            super::projects::content_items(ctx, self.i.issue.id, first, after, include_archived.unwrap_or(true)).await
        }
        async fn projects_v2(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<super::projects::ProjectV2Connection> {
            let items = super::projects::content_items(ctx, self.i.issue.id, None, None, true).await?;
            let mut projects: Vec<super::projects::ProjectV2Obj> = Vec::new();
            for it in items.nodes {
                if !projects.iter().any(|p| p.p.id == it.project.p.id) {
                    projects.push(it.project);
                }
            }
            let total = projects.len();
            let (sel, info, _) = page(projects, first, after);
            Ok(super::projects::ProjectV2Connection { nodes: sel.into_iter().map(|(_, p)| p).collect(), page_info: info, total_count: total as i32 })
        }
        async fn viewer_did_author(&self, ctx: &Context<'_>) -> bool {
            Ctx::from(ctx).viewer().map(|v| v.id) == Some(self.i.author.id)
        }
        async fn viewer_can_update(&self, ctx: &Context<'_>) -> Result<bool> {
            let c = Ctx::from(ctx);
            let Some(v) = c.viewer() else { return Ok(false) };
            if v.id == self.i.author.id {
                return Ok(true);
            }
            Ok(c.db.permission(Some(v), &self.repo).await.map_err(from_core)? >= Permission::Triage)
        }
        async fn viewer_can_react(&self) -> bool {
            true
        }
        async fn viewer_can_subscribe(&self) -> bool {
            true
        }
        async fn viewer_subscription(&self) -> Option<SubscriptionState> {
            Some(SubscriptionState::Unsubscribed)
        }
        async fn participants(&self, ctx: &Context<'_>, first: Option<i32>) -> Result<UserConnection> {
            let c = Ctx::from(ctx);
            let mut ids = vec![self.i.author.id];
            for cm in c.db.comments(self.i.issue.id(), None).await.map_err(from_core)? {
                if !ids.contains(&cm.author_id) {
                    ids.push(cm.author_id);
                }
            }
            let accts = c.db.accounts_by_ids(&ids).await.map_err(from_core)?;
            let (items, info, total) = page(accts, first, None);
            Ok(UserConnection { nodes: items.into_iter().map(|(_, a)| User(a)).collect(), page_info: info, total_count: total as i32 })
        }
        async fn timeline_items(&self, _first: Option<i32>) -> CountConnection {
            CountConnection { total_count: 0 }
        }
        async fn user_content_edits(&self, _first: Option<i32>) -> CountConnection {
            CountConnection { total_count: 0 }
        }
        }
    };
}

#[derive(SimpleObject)]
pub struct ProjectCardConnection {
    pub nodes: Vec<ProjectCard>,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct ProjectCard {
    pub id: ID,
    pub project: Project,
    pub column: Option<ProjectColumn>,
    pub note: Option<String>,
}

#[derive(SimpleObject)]
pub struct ProjectColumn {
    pub id: ID,
    pub name: String,
}

#[derive(InputObject)]
pub struct LabelOrder {
    pub field: LabelOrderField,
    pub direction: OrderDirection,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum LabelOrderField {
    Name,
    CreatedAt,
}

#[derive(InputObject)]
pub struct IssueCommentOrder {
    pub field: IssueCommentOrderField,
    pub direction: OrderDirection,
}

#[derive(InputObject)]
pub struct MilestoneOrder {
    pub field: MilestoneOrderField,
    pub direction: OrderDirection,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum MilestoneOrderField {
    DueDate,
    CreatedAt,
    UpdatedAt,
    Number,
}

#[derive(Union, Clone)]
pub enum Assignee {
    User(User),
    Bot(Bot),
}

#[derive(SimpleObject, Clone)]
pub struct Bot {
    pub id: ID,
    pub login: String,
}

#[derive(SimpleObject)]
pub struct AssigneeConnection {
    pub nodes: Vec<Assignee>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

// ----- Issue -----

#[derive(Clone)]
pub struct Issue {
    pub repo: RepoWithOwner,
    pub i: IssueFull,
}

issue_like_impl!(Issue, {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_ISSUE, self.i.issue.id))
    }
    async fn full_database_id(&self) -> String {
        self.i.issue.id.to_string()
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}/issues/{}", Ctx::from(ctx).config.site_url(), self.repo.full_name(), self.i.issue.number)
    }
    async fn resource_path(&self) -> String {
        format!("/{}/issues/{}", self.repo.full_name(), self.i.issue.number)
    }
    async fn state(&self) -> IssueState {
        if self.i.issue.is_open() { IssueState::Open } else { IssueState::Closed }
    }
    async fn state_reason(&self) -> Option<IssueStateReason> {
        match self.i.issue.state_reason.as_deref() {
            Some("completed") => Some(IssueStateReason::Completed),
            Some("not_planned") => Some(IssueStateReason::NotPlanned),
            Some("reopened") => Some(IssueStateReason::Reopened),
            Some("duplicate") => Some(IssueStateReason::Duplicate),
            _ => None,
        }
    }
    async fn is_pinned(&self) -> Option<bool> {
        Some(false)
    }
    async fn issue_type(&self) -> Option<IssueType> {
        None
    }
    async fn parent(&self) -> Option<Issue> {
        None
    }
    async fn sub_issues(&self, _first: Option<i32>) -> IssueConnection {
        IssueConnection {
            nodes: vec![],
            edges: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        }
    }
    async fn sub_issues_summary(&self) -> SubIssuesSummary {
        SubIssuesSummary { total: 0, completed: 0, percent_completed: 0 }
    }
    async fn blocked_by(&self, _first: Option<i32>) -> IssueConnection {
        IssueConnection {
            nodes: vec![],
            edges: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        }
    }
    async fn blocking(&self, _first: Option<i32>) -> IssueConnection {
        IssueConnection {
            nodes: vec![],
            edges: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        }
    }
    async fn closed_by_pull_requests_references(&self, _first: Option<i32>) -> PullRequestConnection {
        PullRequestConnection {
            nodes: vec![],
            edges: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        }
    }
    async fn tracked_issues_count(&self) -> i32 {
        0
    }
});

#[derive(SimpleObject)]
pub struct IssueType {
    pub id: ID,
    pub name: String,
    pub description: Option<String>,
    pub color: String,
}

#[derive(SimpleObject)]
pub struct SubIssuesSummary {
    pub total: i32,
    pub completed: i32,
    pub percent_completed: i32,
}

#[derive(SimpleObject)]
pub struct IssueEdge {
    pub cursor: String,
    pub node: Issue,
}

#[derive(SimpleObject)]
pub struct IssueConnection {
    pub nodes: Vec<Issue>,
    pub edges: Vec<IssueEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

// ----- PullRequest (issue-backed; PR-specific fields are filled by the pull request slice) -----

#[derive(Clone)]
pub struct PullRequest {
    pub repo: RepoWithOwner,
    pub i: IssueFull,
}

issue_like_impl!(PullRequest, {
    pub async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_PULL, self.i.issue.id))
    }
    async fn full_database_id(&self) -> String {
        self.i.issue.id.to_string()
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}/pull/{}", Ctx::from(ctx).config.site_url(), self.repo.full_name(), self.i.issue.number)
    }
    async fn resource_path(&self) -> String {
        format!("/{}/pull/{}", self.repo.full_name(), self.i.issue.number)
    }
    async fn state(&self, ctx: &Context<'_>) -> PullRequestState {
        crate::graphql::pulls::pr_state(ctx, &self.i).await
    }
    async fn is_draft(&self, ctx: &Context<'_>) -> bool {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.draft != 0).unwrap_or(false)
    }
    async fn merged(&self, ctx: &Context<'_>) -> bool {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.merged != 0).unwrap_or(false)
    }
    async fn merged_at(&self, ctx: &Context<'_>) -> Option<String> {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.and_then(|m| iso_opt(m.merged_at))
    }
    async fn merged_by(&self, ctx: &Context<'_>) -> Option<User> {
        let c = Ctx::from(ctx);
        let m = crate::graphql::pulls::pr_meta(ctx, &self.i).await?;
        c.db.account_by_id(Id(m.merged_by_id?)).await.ok().map(User)
    }
    async fn merge_commit(&self, ctx: &Context<'_>) -> Option<PrCommitRef> {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.and_then(|m| m.merge_commit_sha).map(|oid| PrCommitRef { oid })
    }
    async fn potential_merge_commit(&self) -> Option<PrCommitRef> {
        None
    }
    async fn base_ref_name(&self, ctx: &Context<'_>) -> String {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.base_ref).unwrap_or_default()
    }
    async fn base_ref_oid(&self, ctx: &Context<'_>) -> String {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.base_sha).unwrap_or_default()
    }
    async fn head_ref_name(&self, ctx: &Context<'_>) -> String {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.head_ref).unwrap_or_default()
    }
    async fn head_ref_oid(&self, ctx: &Context<'_>) -> String {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.head_sha).unwrap_or_default()
    }
    async fn base_ref(&self, ctx: &Context<'_>) -> Option<PrBaseRef> {
        let c = Ctx::from(ctx);
        let m = crate::graphql::pulls::pr_meta(ctx, &self.i).await?;
        let rules = c.db.protections_for_branch(self.repo.repo.id(), &m.base_ref).await.ok()?;
        Some(PrBaseRef {
            name: m.base_ref.clone(),
            branch_protection_rule: rules.first().map(|r| BranchProtectionRule {
                requires_strict_status_checks: r.strict_status_checks != 0,
                required_approving_review_count: r.required_approving_review_count as i32,
                pattern: r.pattern.clone(),
            }),
        })
    }
    async fn head_repository(&self) -> Option<Repository> {
        Some(Repository(self.repo.clone()))
    }
    async fn head_repository_owner(&self) -> Option<RepositoryOwner> {
        Some(RepositoryOwner::from_account(self.repo.owner.clone()))
    }
    async fn base_repository(&self) -> Option<Repository> {
        Some(Repository(self.repo.clone()))
    }
    async fn is_cross_repository(&self) -> bool {
        false
    }
    async fn maintainer_can_modify(&self) -> bool {
        false
    }
    async fn additions(&self, ctx: &Context<'_>) -> i32 {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.additions as i32).unwrap_or(0)
    }
    async fn deletions(&self, ctx: &Context<'_>) -> i32 {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.deletions as i32).unwrap_or(0)
    }
    async fn changed_files(&self, ctx: &Context<'_>) -> i32 {
        crate::graphql::pulls::pr_meta(ctx, &self.i).await.map(|m| m.changed_files as i32).unwrap_or(0)
    }
    async fn mergeable(&self, ctx: &Context<'_>) -> MergeableState {
        crate::graphql::pulls::mergeable(ctx, &self.repo, &self.i).await
    }
    async fn merge_state_status(&self, ctx: &Context<'_>) -> MergeStateStatus {
        crate::graphql::pulls::merge_state_status(ctx, &self.repo, &self.i).await
    }
    async fn review_decision(&self, ctx: &Context<'_>) -> Option<PullRequestReviewDecision> {
        crate::graphql::pulls::review_decision(ctx, &self.repo, &self.i).await
    }
    async fn is_in_merge_queue(&self) -> bool {
        false
    }
    async fn is_merge_queue_enabled(&self) -> bool {
        false
    }
    async fn can_be_rebased(&self, ctx: &Context<'_>) -> bool {
        matches!(mergeable(ctx, &self.repo, &self.i).await, MergeableState::Mergeable)
    }
    async fn locked_by_merge_queue(&self) -> bool {
        false
    }
    async fn merge_queue_entry(&self) -> Option<MergeQueueEntry> {
        None
    }
    async fn auto_merge_request(&self) -> Option<AutoMergeRequest> {
        None
    }
    async fn closing_issues_references(&self, ctx: &Context<'_>, _first: Option<i32>) -> Result<IssueConnection> {
        crate::graphql::pulls::closing_issues(ctx, &self.repo, &self.i).await
    }
    async fn commits(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        last: Option<i32>,
        after: Option<String>,
    ) -> Result<PullRequestCommitConnection> {
        crate::graphql::pulls::commits(ctx, &self.repo, &self.i, first, last, after).await
    }
    async fn files(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<PullRequestChangedFileConnection> {
        crate::graphql::pulls::files(ctx, &self.repo, &self.i, first, after).await
    }
    async fn reviews(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        states: Option<Vec<PullRequestReviewState>>,
    ) -> Result<PullRequestReviewConnection> {
        crate::graphql::pulls::reviews(ctx, &self.repo, &self.i, first, after, states, false).await
    }
    async fn latest_reviews(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<PullRequestReviewConnection> {
        crate::graphql::pulls::reviews(ctx, &self.repo, &self.i, first, after, None, true).await
    }
    async fn latest_opinionated_reviews(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        _writers_only: Option<bool>,
    ) -> Result<PullRequestReviewConnection> {
        crate::graphql::pulls::reviews(
            ctx,
            &self.repo,
            &self.i,
            first,
            after,
            Some(vec![PullRequestReviewState::Approved, PullRequestReviewState::ChangesRequested]),
            true,
        )
        .await
    }
    async fn review_requests(&self, ctx: &Context<'_>, _first: Option<i32>) -> Result<ReviewRequestConnection> {
        crate::graphql::pulls::review_requests(ctx, &self.repo, &self.i).await
    }
    async fn review_threads(&self, _first: Option<i32>) -> CountConnection {
        CountConnection { total_count: 0 }
    }
    async fn viewer_can_merge_as_admin(&self, ctx: &Context<'_>) -> Result<bool> {
        let c = Ctx::from(ctx);
        Ok(c.db.permission(c.viewer(), &self.repo).await.map_err(from_core)?.can_admin())
    }
    async fn viewer_merge_body_text(&self) -> String {
        String::new()
    }
    async fn viewer_merge_headline_text(&self) -> String {
        format!("Merge pull request #{} from {}", self.i.issue.number, self.repo.full_name())
    }
    async fn total_comments_count(&self) -> i32 {
        self.i.issue.comments_count as i32
    }
});

#[derive(SimpleObject)]
pub struct PrCommitRef {
    pub oid: String,
}

#[derive(SimpleObject)]
pub struct PrBaseRef {
    pub name: String,
    pub branch_protection_rule: Option<BranchProtectionRule>,
}

#[derive(SimpleObject)]
pub struct BranchProtectionRule {
    pub requires_strict_status_checks: bool,
    pub required_approving_review_count: i32,
    pub pattern: String,
}

#[derive(SimpleObject)]
pub struct MergeQueueEntry {
    pub id: ID,
    pub position: i32,
}

#[derive(SimpleObject)]
pub struct AutoMergeRequest {
    pub author_email: Option<String>,
    pub commit_body: Option<String>,
    pub commit_headline: Option<String>,
    pub merge_method: PullRequestMergeMethod,
    pub enabled_at: Option<String>,
    pub enabled_by: Option<User>,
}

#[derive(SimpleObject)]
pub struct PullRequestEdge {
    pub cursor: String,
    pub node: PullRequest,
}

#[derive(SimpleObject)]
pub struct PullRequestConnection {
    pub nodes: Vec<PullRequest>,
    pub edges: Vec<PullRequestEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct PullRequestCommit {
    pub id: ID,
    pub commit: Commit,
    pub url: String,
    pub resource_path: String,
}

#[derive(SimpleObject)]
pub struct PullRequestCommitConnection {
    pub nodes: Vec<PullRequestCommit>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct PullRequestChangedFile {
    pub path: String,
    pub additions: i32,
    pub deletions: i32,
    pub change_type: PatchStatus,
}

#[derive(SimpleObject)]
pub struct PullRequestChangedFileConnection {
    pub nodes: Vec<PullRequestChangedFile>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct PullRequestReview {
    pub id: ID,
    pub database_id: i64,
    pub author: Option<User>,
    pub author_association: CommentAuthorAssociation,
    pub body: String,
    #[graphql(name = "bodyHTML")]
    pub body_html: String,
    pub state: PullRequestReviewState,
    pub submitted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub url: String,
    pub commit: Option<PrCommitRef>,
    pub reaction_groups: Vec<ReactionGroup>,
    pub includes_created_edit: bool,
    pub viewer_did_author: bool,
}

#[derive(SimpleObject)]
pub struct PullRequestReviewConnection {
    pub nodes: Vec<PullRequestReview>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(Union, Clone)]
pub enum RequestedReviewer {
    User(User),
    Team(Team),
    Bot(Bot),
}

#[derive(SimpleObject, Clone)]
pub struct Team {
    pub id: ID,
    pub name: String,
    pub slug: String,
    pub organization: Organization,
}

#[derive(SimpleObject)]
pub struct ReviewRequest {
    pub id: ID,
    pub requested_reviewer: Option<RequestedReviewer>,
    pub as_code_owner: bool,
}

#[derive(SimpleObject)]
pub struct ReviewRequestConnection {
    pub nodes: Vec<ReviewRequest>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

/// `issueOrPullRequest(number)`.
#[derive(Union)]
pub enum IssueOrPullRequest {
    Issue(Issue),
    PullRequest(PullRequest),
}

pub fn wrap(repo: &RepoWithOwner, i: IssueFull) -> IssueOrPullRequest {
    if i.issue.is_pull() {
        IssueOrPullRequest::PullRequest(PullRequest { repo: repo.clone(), i })
    } else {
        IssueOrPullRequest::Issue(Issue { repo: repo.clone(), i })
    }
}

// ----- status checks (types gh selects on commits; populated by the checks slice) -----

#[derive(SimpleObject)]
#[graphql(complex)]
pub struct StatusContext {
    pub id: ID,
    pub context: String,
    pub state: StatusState,
    pub target_url: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
    #[graphql(skip)]
    pub repo_id: i64,
}

#[async_graphql::ComplexObject]
impl StatusContext {
    async fn is_required(&self, ctx: &Context<'_>, pull_request_id: Option<ID>, pull_request_number: Option<i32>) -> bool {
        required_context(ctx, self.repo_id, &self.context, pull_request_id, pull_request_number).await
    }
}

/// Is `context` a required status check on the pull request's base branch?
async fn required_context(
    ctx: &Context<'_>,
    repo_id: i64,
    context: &str,
    pull_request_id: Option<ID>,
    pull_request_number: Option<i32>,
) -> bool {
    let c = Ctx::from(ctx);
    let meta = match (pull_request_id, pull_request_number) {
        (Some(id), _) => match decode_issue_id(&id) {
            Ok(iid) => c.db.pull_meta(iid).await.ok(),
            Err(_) => None,
        },
        (None, Some(n)) => match c.db.issue_by_number(Id(repo_id), n as i64).await {
            Ok(i) => c.db.pull_meta(i.id()).await.ok(),
            Err(_) => None,
        },
        _ => None,
    };
    let Some(m) = meta else { return false };
    c.db.protections_for_branch(Id(repo_id), &m.base_ref)
        .await
        .unwrap_or_default()
        .first()
        .map(|r| r.checks().iter().any(|x| x == context))
        .unwrap_or(false)
}

#[derive(SimpleObject)]
#[graphql(complex)]
pub struct CheckRun {
    pub id: ID,
    pub name: String,
    pub status: CheckStatusState,
    pub conclusion: Option<CheckConclusionState>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub details_url: Option<String>,
    pub check_suite: Option<CheckSuite>,
    #[graphql(skip)]
    pub repo_id: i64,
}

#[async_graphql::ComplexObject]
impl CheckRun {
    async fn is_required(&self, ctx: &Context<'_>, pull_request_id: Option<ID>, pull_request_number: Option<i32>) -> bool {
        required_context(ctx, self.repo_id, &self.name, pull_request_id, pull_request_number).await
    }
}

#[derive(SimpleObject)]
pub struct CheckSuite {
    pub id: ID,
    pub workflow_run: Option<WorkflowRunRef>,
}

#[derive(SimpleObject)]
pub struct WorkflowRunRef {
    pub id: ID,
    pub workflow: WorkflowRef,
    pub url: String,
    pub event: String,
}

#[derive(SimpleObject)]
pub struct WorkflowRef {
    pub id: ID,
    pub name: String,
}

#[derive(Union)]
pub enum StatusCheckRollupContext {
    StatusContext(StatusContext),
    CheckRun(CheckRun),
}

#[derive(SimpleObject)]
pub struct CheckRunStateCount {
    pub state: CheckStatusState,
    pub count: i32,
}

#[derive(SimpleObject)]
pub struct StatusCheckRollupContextConnection {
    pub nodes: Vec<StatusCheckRollupContext>,
    pub page_info: PageInfo,
    pub total_count: i32,
    pub check_run_count: i32,
    pub status_context_count: i32,
    pub check_run_counts_by_state: Vec<CheckRunStateCount>,
}

pub struct StatusCheckRollup {
    pub id: ID,
    pub state: StatusState,
    pub contexts: StatusCheckRollupContextConnection,
}

#[Object]
impl StatusCheckRollup {
    async fn id(&self) -> &ID {
        &self.id
    }
    async fn state(&self) -> StatusState {
        self.state
    }
    async fn contexts(
        &self,
        _first: Option<i32>,
        _after: Option<String>,
        _last: Option<i32>,
        _before: Option<String>,
    ) -> &StatusCheckRollupContextConnection {
        &self.contexts
    }
}

// ----- helpers used by Repository and Query -----

pub fn issue_filter(
    states: Option<&[IssueState]>,
    labels: Option<&[String]>,
    filter_by: Option<&IssueFilters>,
    order_by: Option<&IssueOrder>,
    is_pull: Option<bool>,
) -> IssueFilter {
    let mut f = IssueFilter {
        state: state_filter(states),
        is_pull,
        labels: labels.map(|l| l.to_vec()).unwrap_or_default(),
        sort: "created".into(),
        ..Default::default()
    };
    if let Some(fb) = filter_by {
        if let Some(s) = &fb.states {
            f.state = state_filter(Some(s));
        }
        f.assignee = fb.assignee.clone();
        f.creator = fb.created_by.clone();
        f.mentioned = fb.mentioned.clone();
        f.milestone = fb.milestone_number.clone().or_else(|| fb.milestone.clone());
        if let Some(l) = &fb.labels {
            f.labels.extend(l.iter().cloned());
        }
        f.since = fb.since.as_deref().and_then(reef_core::time::parse_iso);
    }
    if let Some(o) = order_by {
        f.sort = match o.field {
            IssueOrderField::CreatedAt => "created",
            IssueOrderField::UpdatedAt => "updated",
            IssueOrderField::Comments => "comments",
        }
        .into();
        f.ascending = matches!(o.direction, OrderDirection::Asc);
    }
    f
}

/// Paginate a filter via the database (cursor = offset).
pub async fn issue_page(
    ctx: &Context<'_>,
    repo: Option<Id>,
    f: &IssueFilter,
    first: Option<i32>,
    after: Option<String>,
) -> Result<(Vec<(usize, IssueFull)>, PageInfo, i64)> {
    let c = Ctx::from(ctx);
    let n = first.unwrap_or(30).clamp(1, 100) as i64;
    let start = after.as_deref().map(|cu| parse_cursor(cu) + 1).unwrap_or(0) as i64;
    // Fetch by offset: page = start / n is not exact, so pull start..start+n directly.
    let (items, total) = c.db.list_issues(repo, f, 1, start + n).await.map_err(from_core)?;
    let items: Vec<(usize, IssueFull)> = items.into_iter().enumerate().skip(start as usize).collect();
    let end = items.last().map(|(i, _)| *i);
    let info = PageInfo {
        has_next_page: end.map(|e| (e as i64) + 1 < total).unwrap_or(false),
        has_previous_page: start > 0,
        start_cursor: items.first().map(|(i, _)| cursor(*i)),
        end_cursor: end.map(cursor),
    };
    Ok((items, info, total))
}

pub fn decode_id(id: &ID, prefix: &str) -> Result<Id> {
    match shapes::decode_node_id(&id.0) {
        Some((p, n)) if p == prefix => Ok(Id(n)),
        _ => Err(not_found("Could not resolve to a node with the global id of '".to_string() + &id.0 + "'.")),
    }
}

pub fn decode_issue_id(id: &ID) -> Result<Id> {
    match shapes::decode_node_id(&id.0) {
        Some((p, n)) if p == shapes::NODE_ISSUE || p == shapes::NODE_PULL => Ok(Id(n)),
        _ => Err(not_found("Could not resolve to a node with the global id of '".to_string() + &id.0 + "'.")),
    }
}

pub async fn load_issue(ctx: &Context<'_>, id: &ID) -> Result<(RepoWithOwner, IssueFull)> {
    let c = Ctx::from(ctx);
    let iid = decode_issue_id(id)?;
    let i = c.db.issue_full_by_id(iid).await.map_err(|_| not_found("Could not resolve to an Issue with the given id."))?;
    let rw = c.db.repo_with_owner_by_id(Id(i.issue.repo_id)).await.map_err(from_core)?;
    if !c.db.permission(c.viewer(), &rw).await.map_err(from_core)?.can_read() {
        return Err(not_found("Could not resolve to an Issue with the given id."));
    }
    Ok((rw, i))
}

pub async fn require_perm(ctx: &Context<'_>, rw: &RepoWithOwner, min: Permission, what: &str) -> Result<Account> {
    let c = Ctx::from(ctx);
    let Some(v) = c.viewer() else { return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated.")) };
    let p = c.db.permission(Some(v), rw).await.map_err(from_core)?;
    if p < min {
        return Err(super::forbidden(format!("You do not have permission to {what}.")));
    }
    Ok(v.clone())
}
