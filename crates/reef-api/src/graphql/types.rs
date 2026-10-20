//! GraphQL object types. Names and fields mirror GitHub's schema.

use super::{Ctx, from_core, not_found};
use crate::shapes;
use async_graphql::{ComplexObject, Context, Enum, ID, Interface, Object, Result, SimpleObject};
use base64::Engine;
use reef_core::time::{iso, iso_opt};
use reef_db::accounts::Account;
use reef_db::repos::{Permission, RepoWithOwner};

// ----- enums -----

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "RepositoryPermission")]
pub enum RepositoryPermission {
    Admin,
    Maintain,
    Write,
    Triage,
    Read,
}

impl RepositoryPermission {
    pub fn from(p: Permission) -> Option<Self> {
        Some(match p {
            Permission::Admin => Self::Admin,
            Permission::Maintain => Self::Maintain,
            Permission::Write => Self::Write,
            Permission::Triage => Self::Triage,
            Permission::Read => Self::Read,
            Permission::None => return None,
        })
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RepositoryVisibility {
    Private,
    Public,
    Internal,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RepositoryPrivacy {
    Public,
    Private,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RepositoryAffiliation {
    Owner,
    Collaborator,
    OrganizationMember,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RepositoryOrderField {
    CreatedAt,
    UpdatedAt,
    PushedAt,
    Name,
    Stargazers,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ReleaseOrderField {
    CreatedAt,
    Name,
}

#[derive(async_graphql::InputObject)]
pub struct ReleaseOrder {
    pub field: ReleaseOrderField,
    pub direction: OrderDirection,
}

#[derive(SimpleObject)]
pub struct Release {
    pub id: ID,
    pub database_id: i64,
    pub name: Option<String>,
    pub tag_name: String,
    pub description: Option<String>,
    pub is_draft: bool,
    pub is_prerelease: bool,
    pub is_latest: bool,
    pub created_at: String,
    pub published_at: Option<String>,
    pub url: String,
    pub author: Option<User>,
    pub tag_commit: Option<Commit>,
}

#[derive(SimpleObject)]
pub struct ReleaseConnection {
    pub nodes: Vec<Release>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PullRequestMergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum SubscriptionState {
    Unsubscribed,
    Subscribed,
    Ignored,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum SearchType {
    Issue,
    Repository,
    User,
    Discussion,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum MilestoneState {
    Open,
    Closed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ProjectState {
    Open,
    Closed,
}

#[derive(async_graphql::InputObject)]
pub struct RepositoryOrder {
    pub field: RepositoryOrderField,
    pub direction: OrderDirection,
}

// ----- shared bits -----

#[derive(SimpleObject, Clone)]
pub struct PageInfo {
    pub has_next_page: bool,
    pub has_previous_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

/// GitHub's `URI` scalar; plain string on the wire.
#[derive(Clone, Debug)]
pub struct Uri(pub String);

#[async_graphql::Scalar(name = "URI")]
impl async_graphql::ScalarType for Uri {
    fn parse(value: async_graphql::Value) -> async_graphql::InputValueResult<Self> {
        match value {
            async_graphql::Value::String(s) => Ok(Uri(s)),
            _ => Err(async_graphql::InputValueError::expected_type(value)),
        }
    }
    fn to_value(&self) -> async_graphql::Value {
        async_graphql::Value::String(self.0.clone())
    }
}

pub fn cursor(i: usize) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("cursor:{i}"))
}

pub fn parse_cursor(c: &str) -> usize {
    base64::engine::general_purpose::STANDARD
        .decode(c)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.strip_prefix("cursor:").and_then(|n| n.parse().ok()))
        .unwrap_or(0)
}

/// Apply `first`/`after` to a materialised list.
pub fn page<T: Clone>(all: Vec<T>, first: Option<i32>, after: Option<String>) -> (Vec<(usize, T)>, PageInfo, usize) {
    let total = all.len();
    let start = after.as_deref().map(|c| parse_cursor(c) + 1).unwrap_or(0);
    let n = first.unwrap_or(30).clamp(0, 100) as usize;
    let items: Vec<(usize, T)> = all.into_iter().enumerate().skip(start).take(n).collect();
    let end = items.last().map(|(i, _)| *i);
    let info = PageInfo {
        has_next_page: end.map(|e| e + 1 < total).unwrap_or(false),
        has_previous_page: start > 0,
        start_cursor: items.first().map(|(i, _)| cursor(*i)),
        end_cursor: end.map(cursor),
    };
    (items, info, total)
}

#[derive(SimpleObject)]
pub struct CountConnection {
    pub total_count: i32,
}

#[derive(SimpleObject)]
#[graphql(name = "Language")]
pub struct Language {
    pub name: String,
    pub color: Option<String>,
}

#[derive(SimpleObject)]
pub struct LanguageEdge {
    pub size: i32,
    pub node: Language,
}

#[derive(SimpleObject)]
pub struct LanguageConnection {
    pub edges: Vec<LanguageEdge>,
    pub nodes: Vec<Language>,
    pub total_count: i32,
    pub total_size: i32,
}

#[derive(SimpleObject)]
pub struct License {
    pub key: String,
    pub name: String,
    pub nickname: Option<String>,
    pub spdx_id: Option<String>,
    pub url: Option<String>,
}

#[derive(SimpleObject)]
pub struct Topic {
    pub name: String,
}

#[derive(SimpleObject)]
pub struct RepositoryTopic {
    pub topic: Topic,
    pub url: String,
}

#[derive(SimpleObject)]
pub struct RepositoryTopicConnection {
    pub nodes: Vec<RepositoryTopic>,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct IssueTemplate {
    pub name: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub about: Option<String>,
}

#[derive(SimpleObject)]
pub struct PullRequestTemplate {
    pub filename: String,
    pub body: Option<String>,
}

#[derive(SimpleObject)]
pub struct CodeOfConduct {
    pub key: String,
    pub name: String,
    pub url: Option<String>,
}

#[derive(SimpleObject)]
pub struct RepositoryContactLink {
    pub about: String,
    pub name: String,
    pub url: String,
}

#[derive(SimpleObject)]
pub struct FundingLink {
    pub platform: String,
    pub url: String,
}

#[derive(SimpleObject)]
pub struct Project {
    pub id: ID,
    pub name: String,
    pub number: i32,
    pub body: Option<String>,
    pub resource_path: String,
    pub state: ProjectState,
}

#[derive(SimpleObject)]
pub struct ProjectConnection {
    pub nodes: Vec<Project>,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct RateLimit {
    pub limit: i32,
    pub cost: i32,
    pub remaining: i32,
    pub used: i32,
    pub reset_at: String,
    pub node_count: i32,
}

// ----- git objects -----

#[derive(Clone)]
pub struct GitActor {
    pub name: String,
    pub email: String,
    pub date: String,
}

#[Object]
impl GitActor {
    async fn name(&self) -> &str {
        &self.name
    }
    async fn email(&self) -> &str {
        &self.email
    }
    async fn date(&self) -> &str {
        &self.date
    }
    async fn avatar_url(&self, ctx: &Context<'_>, #[graphql(default = 0)] _size: i32) -> String {
        let c = Ctx::from(ctx);
        match c.db.user_by_login_or_email(&self.email).await {
            Ok(a) => shapes::avatar_url(c.config, &a),
            Err(_) => format!("{}/avatars/0", c.config.site_url()),
        }
    }
    async fn user(&self, ctx: &Context<'_>) -> Option<User> {
        let c = Ctx::from(ctx);
        c.db.user_by_login_or_email(&self.email).await.ok().map(User)
    }
}

#[derive(Clone)]
pub struct Commit {
    pub repo: RepoWithOwner,
    pub c: reef_git::Commit,
}

#[Object]
impl Commit {
    async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_COMMIT, 0) + ":" + &self.c.id)
    }
    async fn oid(&self) -> &str {
        &self.c.id
    }
    async fn abbreviated_oid(&self) -> String {
        self.c.id[..7].to_string()
    }
    async fn message(&self) -> String {
        self.c.message.trim_end().to_string()
    }
    async fn message_headline(&self) -> String {
        self.c.summary().to_string()
    }
    async fn message_body(&self) -> String {
        self.c.message.trim_end().split_once('\n').map(|x| x.1).unwrap_or("").trim_start().to_string()
    }
    async fn committed_date(&self) -> String {
        iso(self.c.committer.time_ms)
    }
    async fn authored_date(&self) -> String {
        iso(self.c.author.time_ms)
    }
    async fn author(&self) -> GitActor {
        GitActor { name: self.c.author.name.clone(), email: self.c.author.email.clone(), date: iso(self.c.author.time_ms) }
    }
    async fn committer(&self) -> GitActor {
        GitActor { name: self.c.committer.name.clone(), email: self.c.committer.email.clone(), date: iso(self.c.committer.time_ms) }
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        let c = Ctx::from(ctx);
        format!("{}/{}/commit/{}", c.config.site_url(), self.repo.full_name(), self.c.id)
    }
    async fn tree_entry_count(&self) -> i32 {
        0
    }
    async fn authors(&self, _first: Option<i32>) -> GitActorConnection {
        GitActorConnection {
            nodes: vec![GitActor {
                name: self.c.author.name.clone(),
                email: self.c.author.email.clone(),
                date: iso(self.c.author.time_ms),
            }],
            total_count: 1,
        }
    }
    async fn additions(&self) -> i32 {
        0
    }
    async fn deletions(&self) -> i32 {
        0
    }
    async fn changed_files_if_available(&self) -> Option<i32> {
        None
    }
    async fn parents(&self, _first: Option<i32>) -> CountConnection {
        CountConnection { total_count: self.c.parents.len() as i32 }
    }
    /// Commit statuses (and later check runs) for this commit, as GitHub's rollup.
    async fn status_check_rollup(&self, ctx: &Context<'_>) -> Result<Option<super::issues::StatusCheckRollup>> {
        use super::issues::{StatusCheckRollup, StatusCheckRollupContext, StatusCheckRollupContextConnection, StatusContext, StatusState};
        let c = Ctx::from(ctx);
        let statuses = c.db.latest_statuses(self.repo.repo.id(), &self.c.id).await.map_err(from_core)?;
        if statuses.is_empty() {
            return Ok(None);
        }
        let map = |s: &str| match s {
            "success" => StatusState::Success,
            "failure" => StatusState::Failure,
            "error" => StatusState::Error,
            _ => StatusState::Pending,
        };
        let state = if statuses.iter().any(|s| s.state == "failure" || s.state == "error") {
            StatusState::Failure
        } else if statuses.iter().all(|s| s.state == "success") {
            StatusState::Success
        } else {
            StatusState::Pending
        };
        let nodes: Vec<StatusCheckRollupContext> = statuses
            .iter()
            .map(|s| {
                StatusCheckRollupContext::StatusContext(StatusContext {
                    id: ID(shapes::node_id("SC", s.id)),
                    context: s.context.clone(),
                    state: map(&s.state),
                    target_url: s.target_url.clone(),
                    description: s.description.clone(),
                    created_at: iso(s.created_at),
                    repo_id: self.repo.repo.id,
                })
            })
            .collect();
        let n = nodes.len() as i32;
        Ok(Some(StatusCheckRollup {
            id: ID(format!("SCR_{}", self.c.id)),
            state,
            contexts: StatusCheckRollupContextConnection {
                nodes,
                page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
                total_count: n,
                check_run_count: 0,
                status_context_count: n,
                check_run_counts_by_state: vec![],
            },
        }))
    }
    async fn status(&self, ctx: &Context<'_>) -> Result<Option<CommitStatusSummary>> {
        let c = Ctx::from(ctx);
        let statuses = c.db.latest_statuses(self.repo.repo.id(), &self.c.id).await.map_err(from_core)?;
        if statuses.is_empty() {
            return Ok(None);
        }
        let state = if statuses.iter().any(|s| s.state == "failure" || s.state == "error") {
            "FAILURE"
        } else if statuses.iter().all(|s| s.state == "success") {
            "SUCCESS"
        } else {
            "PENDING"
        };
        Ok(Some(CommitStatusSummary { state: state.into() }))
    }
    async fn history(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        path: Option<String>,
    ) -> Result<CommitHistoryConnection> {
        let c = Ctx::from(ctx);
        let path_repo = c.store.path(self.repo.repo.id());
        let start = self.c.id.clone();
        let list = tokio::task::spawn_blocking(move || {
            let r = reef_git::RepoReader::open(&path_repo)?;
            r.log(&start, path.as_deref(), 0, 500)
        })
        .await
        .map_err(|e| from_core(reef_core::Error::internal(e)))?
        .map_err(from_core)?;
        let (items, info, total) = page(list, first, after);
        Ok(CommitHistoryConnection {
            nodes: items.into_iter().map(|(_, c)| Commit { repo: self.repo.clone(), c }).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
}

#[derive(SimpleObject)]
pub struct GitActorConnection {
    pub nodes: Vec<GitActor>,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct CommitStatusSummary {
    pub state: String,
}

#[derive(SimpleObject)]
pub struct CommitHistoryConnection {
    pub nodes: Vec<Commit>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(Clone)]
pub struct Ref {
    pub repo: RepoWithOwner,
    pub name: String,
    pub prefix: String,
    pub sha: String,
}

#[Object]
impl Ref {
    async fn id(&self) -> ID {
        ID(format!("{}:{}{}", shapes::node_id(shapes::NODE_REF, self.repo.repo.id), self.prefix, self.name))
    }
    async fn name(&self) -> &str {
        &self.name
    }
    async fn prefix(&self) -> &str {
        &self.prefix
    }
    async fn target(&self, ctx: &Context<'_>) -> Result<Option<Commit>> {
        let c = Ctx::from(ctx);
        let path = c.store.path(self.repo.repo.id());
        let sha = self.sha.clone();
        let commit = tokio::task::spawn_blocking(move || reef_git::RepoReader::open(&path)?.commit(&sha))
            .await
            .map_err(|e| from_core(reef_core::Error::internal(e)))?
            .map_err(from_core)?;
        Ok(Some(Commit { repo: self.repo.clone(), c: commit }))
    }
    async fn repository(&self) -> Repository {
        Repository(self.repo.clone())
    }
}

// ----- owners -----

#[derive(Clone)]
pub struct User(pub Account);

#[derive(Clone)]
pub struct Organization(pub Account);

#[derive(Interface, Clone)]
#[allow(clippy::duplicated_attributes)] // async-graphql's `field(...)` list trips the lint
#[graphql(
    field(name = "id", ty = "ID"),
    field(name = "login", ty = "String"),
    field(name = "url", ty = "String"),
    field(name = "resource_path", ty = "String"),
    field(name = "avatar_url", ty = "String", arg(name = "size", ty = "Option<i32>")),
    field(
        name = "repositories",
        ty = "RepositoryConnection",
        arg(name = "first", ty = "Option<i32>"),
        arg(name = "after", ty = "Option<String>"),
        arg(name = "privacy", ty = "Option<RepositoryPrivacy>"),
        arg(name = "is_fork", ty = "Option<bool>"),
        arg(name = "is_archived", ty = "Option<bool>"),
        arg(name = "owner_affiliations", ty = "Option<Vec<RepositoryAffiliation>>"),
        arg(name = "affiliations", ty = "Option<Vec<RepositoryAffiliation>>"),
        arg(name = "order_by", ty = "Option<RepositoryOrder>")
    ),
    field(name = "repository", ty = "Option<Repository>", arg(name = "name", ty = "String"))
)]
pub enum RepositoryOwner {
    User(User),
    Organization(Organization),
}

impl RepositoryOwner {
    pub fn from_account(a: Account) -> RepositoryOwner {
        if a.is_org() { RepositoryOwner::Organization(Organization(a)) } else { RepositoryOwner::User(User(a)) }
    }
}

#[derive(Interface, Clone)]
#[graphql(field(name = "id", ty = "ID"))]
pub enum Node {
    User(User),
    Organization(Organization),
    Repository(Repository),
    Issue(super::issues::Issue),
    PullRequest(super::issues::PullRequest),
    Label(super::issues::Label),
    Milestone(super::issues::Milestone),
    IssueComment(super::issues::IssueComment),
}

#[allow(clippy::too_many_arguments)]
async fn owner_repositories(
    ctx: &Context<'_>,
    owner: &Account,
    first: Option<i32>,
    after: Option<String>,
    privacy: Option<RepositoryPrivacy>,
    is_fork: Option<bool>,
    is_archived: Option<bool>,
    affiliations: Option<Vec<RepositoryAffiliation>>,
    order_by: Option<RepositoryOrder>,
) -> Result<RepositoryConnection> {
    let c = Ctx::from(ctx);
    let mut list = if let Some(aff) =
        affiliations.filter(|a| !a.is_empty()).filter(|_| owner.is_user() && c.viewer().map(|v| v.id) == Some(owner.id))
    {
        // The viewer asking about themselves may include collaborator and org repositories.
        let names: Vec<&str> = aff
            .iter()
            .map(|a| match a {
                RepositoryAffiliation::Owner => "owner",
                RepositoryAffiliation::Collaborator => "collaborator",
                RepositoryAffiliation::OrganizationMember => "organization_member",
            })
            .collect();
        c.db.repos_for_user(owner, &names.join(",")).await.map_err(from_core)?
    } else {
        c.db.repos_for_owner(owner, c.viewer(), 1, 10_000).await.map_err(from_core)?
    };
    if let Some(p) = privacy {
        list.retain(|r| r.repo.is_private() == matches!(p, RepositoryPrivacy::Private));
    }
    if let Some(f) = is_fork {
        list.retain(|r| (r.repo.is_fork != 0) == f);
    }
    if let Some(a) = is_archived {
        list.retain(|r| (r.repo.archived != 0) == a);
    }
    let (field, dir) = order_by.map(|o| (o.field, o.direction)).unwrap_or((RepositoryOrderField::PushedAt, OrderDirection::Desc));
    match field {
        RepositoryOrderField::CreatedAt => list.sort_by_key(|r| r.repo.created_at),
        RepositoryOrderField::UpdatedAt => list.sort_by_key(|r| r.repo.updated_at),
        RepositoryOrderField::PushedAt => list.sort_by_key(|r| r.repo.pushed_at.unwrap_or(r.repo.created_at)),
        RepositoryOrderField::Name => list.sort_by_key(|a| a.repo.name.to_lowercase()),
        RepositoryOrderField::Stargazers => list.sort_by_key(|r| r.repo.stars_count),
    }
    if matches!(dir, OrderDirection::Desc) {
        list.reverse();
    }
    let (items, info, total) = page(list, first, after);
    Ok(RepositoryConnection {
        nodes: items.iter().map(|(_, r)| Repository(r.clone())).collect(),
        edges: items.into_iter().map(|(i, r)| RepositoryEdge { cursor: cursor(i), node: Repository(r) }).collect(),
        page_info: info,
        total_count: total as i32,
    })
}

#[derive(SimpleObject)]
pub struct RepositoryEdge {
    pub cursor: String,
    pub node: Repository,
}

#[derive(SimpleObject)]
pub struct RepositoryConnection {
    pub nodes: Vec<Repository>,
    pub edges: Vec<RepositoryEdge>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct OrganizationConnection {
    pub nodes: Vec<Organization>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[derive(SimpleObject)]
pub struct UserConnection {
    pub nodes: Vec<User>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

#[Object]
impl User {
    async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_USER, self.0.id))
    }
    async fn database_id(&self) -> i64 {
        self.0.id
    }
    async fn login(&self) -> String {
        self.0.login.clone()
    }
    async fn name(&self) -> Option<String> {
        self.0.name.clone()
    }
    async fn email(&self, ctx: &Context<'_>) -> String {
        // GitHub exposes the public email only; Reef shows it to the user themself.
        let c = Ctx::from(ctx);
        if c.viewer().map(|v| v.id) == Some(self.0.id) { self.0.email.clone().unwrap_or_default() } else { String::new() }
    }
    async fn bio(&self) -> Option<String> {
        self.0.bio.clone()
    }
    async fn company(&self) -> Option<String> {
        self.0.company.clone()
    }
    async fn location(&self) -> Option<String> {
        self.0.location.clone()
    }
    async fn website_url(&self) -> Option<String> {
        self.0.blog.clone()
    }
    async fn avatar_url(&self, ctx: &Context<'_>, _size: Option<i32>) -> String {
        shapes::avatar_url(Ctx::from(ctx).config, &self.0)
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}", Ctx::from(ctx).config.site_url(), self.0.login)
    }
    async fn resource_path(&self) -> String {
        format!("/{}", self.0.login)
    }
    async fn created_at(&self) -> String {
        iso(self.0.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.0.updated_at)
    }
    async fn is_site_admin(&self) -> bool {
        self.0.is_admin()
    }
    async fn is_viewer(&self, ctx: &Context<'_>) -> bool {
        Ctx::from(ctx).viewer().map(|v| v.id) == Some(self.0.id)
    }
    async fn is_employee(&self) -> bool {
        false
    }
    #[graphql(name = "__typename_hint", visible = false)]
    async fn typename_hint(&self) -> &str {
        "User"
    }
    async fn organizations(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<OrganizationConnection> {
        let c = Ctx::from(ctx);
        let orgs = c.db.user_orgs(self.0.id()).await.map_err(from_core)?;
        let (items, info, total) = page(orgs, first, after);
        Ok(OrganizationConnection {
            nodes: items.into_iter().map(|(_, a)| Organization(a)).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    #[allow(clippy::too_many_arguments)]
    async fn repositories(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        privacy: Option<RepositoryPrivacy>,
        is_fork: Option<bool>,
        is_archived: Option<bool>,
        owner_affiliations: Option<Vec<RepositoryAffiliation>>,
        affiliations: Option<Vec<RepositoryAffiliation>>,
        order_by: Option<RepositoryOrder>,
    ) -> Result<RepositoryConnection> {
        owner_repositories(ctx, &self.0, first, after, privacy, is_fork, is_archived, affiliations.or(owner_affiliations), order_by).await
    }
    async fn repository(&self, ctx: &Context<'_>, name: String) -> Result<Option<Repository>> {
        repo_of_owner(ctx, &self.0, &name).await
    }
    #[graphql(name = "projectV2")]
    async fn project_v2(&self, ctx: &Context<'_>, number: i32) -> Result<Option<super::projects::ProjectV2Obj>> {
        super::projects::owner_project(ctx, &self.0, number).await
    }
    #[graphql(name = "projectsV2")]
    async fn projects_v2(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        query: Option<String>,
    ) -> Result<super::projects::ProjectV2Connection> {
        super::projects::owner_projects(ctx, &self.0, first, after, query).await
    }
    async fn followers(&self) -> CountConnection {
        CountConnection { total_count: 0 }
    }
    async fn following(&self) -> CountConnection {
        CountConnection { total_count: 0 }
    }
    async fn starred_repositories(&self) -> CountConnection {
        CountConnection { total_count: 0 }
    }
}

#[Object]
impl Organization {
    async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_ORG, self.0.id))
    }
    async fn database_id(&self) -> i64 {
        self.0.id
    }
    async fn login(&self) -> String {
        self.0.login.clone()
    }
    async fn name(&self) -> Option<String> {
        self.0.name.clone()
    }
    async fn description(&self) -> Option<String> {
        self.0.description.clone()
    }
    async fn email(&self) -> Option<String> {
        self.0.billing_email.clone()
    }
    async fn location(&self) -> Option<String> {
        self.0.location.clone()
    }
    async fn website_url(&self) -> Option<String> {
        self.0.blog.clone()
    }
    async fn avatar_url(&self, ctx: &Context<'_>, _size: Option<i32>) -> String {
        shapes::avatar_url(Ctx::from(ctx).config, &self.0)
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}", Ctx::from(ctx).config.site_url(), self.0.login)
    }
    async fn resource_path(&self) -> String {
        format!("/{}", self.0.login)
    }
    async fn created_at(&self) -> String {
        iso(self.0.created_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.0.updated_at)
    }
    async fn is_verified(&self) -> bool {
        false
    }
    async fn viewer_can_administer(&self, ctx: &Context<'_>) -> Result<bool> {
        let c = Ctx::from(ctx);
        let Some(v) = c.viewer() else { return Ok(false) };
        if v.is_admin() {
            return Ok(true);
        }
        Ok(c.db.org_membership(self.0.id(), v.id()).await.map_err(from_core)?.map(|m| m.role == "admin").unwrap_or(false))
    }
    async fn viewer_can_create_repositories(&self, ctx: &Context<'_>) -> Result<bool> {
        let c = Ctx::from(ctx);
        let Some(v) = c.viewer() else { return Ok(false) };
        c.db.can_create_in(v, &self.0).await.map_err(from_core)
    }
    async fn viewer_is_a_member(&self, ctx: &Context<'_>) -> Result<bool> {
        let c = Ctx::from(ctx);
        let Some(v) = c.viewer() else { return Ok(false) };
        Ok(c.db.org_membership(self.0.id(), v.id()).await.map_err(from_core)?.is_some())
    }
    async fn members_with_role(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<UserConnection> {
        let c = Ctx::from(ctx);
        let ms = c.db.org_members(self.0.id(), None).await.map_err(from_core)?;
        let (items, info, total) = page(ms.into_iter().map(|(a, _)| a).collect(), first, after);
        Ok(UserConnection { nodes: items.into_iter().map(|(_, a)| User(a)).collect(), page_info: info, total_count: total as i32 })
    }
    #[allow(clippy::too_many_arguments)]
    async fn repositories(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        privacy: Option<RepositoryPrivacy>,
        is_fork: Option<bool>,
        is_archived: Option<bool>,
        owner_affiliations: Option<Vec<RepositoryAffiliation>>,
        affiliations: Option<Vec<RepositoryAffiliation>>,
        order_by: Option<RepositoryOrder>,
    ) -> Result<RepositoryConnection> {
        owner_repositories(ctx, &self.0, first, after, privacy, is_fork, is_archived, affiliations.or(owner_affiliations), order_by).await
    }
    async fn repository(&self, ctx: &Context<'_>, name: String) -> Result<Option<Repository>> {
        repo_of_owner(ctx, &self.0, &name).await
    }
    async fn teams(&self) -> CountConnection {
        CountConnection { total_count: 0 }
    }
    #[graphql(name = "projectV2")]
    async fn project_v2(&self, ctx: &Context<'_>, number: i32) -> Result<Option<super::projects::ProjectV2Obj>> {
        super::projects::owner_project(ctx, &self.0, number).await
    }
    #[graphql(name = "projectsV2")]
    async fn projects_v2(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        query: Option<String>,
    ) -> Result<super::projects::ProjectV2Connection> {
        super::projects::owner_projects(ctx, &self.0, first, after, query).await
    }
}

async fn repo_of_owner(ctx: &Context<'_>, owner: &Account, name: &str) -> Result<Option<Repository>> {
    let c = Ctx::from(ctx);
    match c.db.repo_by_path(&owner.login, name).await {
        Ok(rw) => {
            let perm = c.db.permission(c.viewer(), &rw).await.map_err(from_core)?;
            if perm.can_read() { Ok(Some(Repository(rw))) } else { Ok(None) }
        }
        Err(reef_core::Error::NotFound) => Ok(None),
        Err(e) => Err(from_core(e)),
    }
}

// ----- repository -----

#[derive(Clone)]
pub struct Repository(pub RepoWithOwner);

impl Repository {
    fn b(v: i64) -> bool {
        v != 0
    }
    async fn perm(&self, ctx: &Context<'_>) -> Result<Permission> {
        let c = Ctx::from(ctx);
        c.db.permission(c.viewer(), &self.0).await.map_err(from_core)
    }
}

#[Object]
impl Repository {
    async fn id(&self) -> ID {
        ID(shapes::node_id(shapes::NODE_REPO, self.0.repo.id))
    }
    async fn database_id(&self) -> i64 {
        self.0.repo.id
    }
    async fn full_database_id(&self) -> String {
        self.0.repo.id.to_string()
    }
    async fn name(&self) -> String {
        self.0.repo.name.clone()
    }
    async fn name_with_owner(&self) -> String {
        self.0.full_name()
    }
    async fn owner(&self) -> RepositoryOwner {
        RepositoryOwner::from_account(self.0.owner.clone())
    }
    async fn parent(&self, ctx: &Context<'_>) -> Result<Option<Repository>> {
        let Some(pid) = self.0.repo.parent_id else { return Ok(None) };
        let c = Ctx::from(ctx);
        match c.db.repo_with_owner_by_id(reef_core::Id(pid)).await {
            Ok(rw) => Ok(Some(Repository(rw))),
            Err(_) => Ok(None),
        }
    }
    async fn template_repository(&self) -> Option<Repository> {
        None
    }
    async fn description(&self) -> Option<String> {
        self.0.repo.description.clone()
    }
    async fn short_description_html(&self) -> String {
        ammonia::clean_text(self.0.repo.description.as_deref().unwrap_or(""))
    }
    async fn homepage_url(&self) -> Option<String> {
        self.0.repo.homepage.clone()
    }
    async fn open_graph_image_url(&self, ctx: &Context<'_>) -> String {
        shapes::avatar_url(Ctx::from(ctx).config, &self.0.owner)
    }
    async fn uses_custom_open_graph_image(&self) -> bool {
        false
    }
    async fn url(&self, ctx: &Context<'_>) -> String {
        format!("{}/{}", Ctx::from(ctx).config.site_url(), self.0.full_name())
    }
    async fn resource_path(&self) -> String {
        format!("/{}", self.0.full_name())
    }
    async fn ssh_url(&self, ctx: &Context<'_>) -> String {
        shapes::clone_urls(Ctx::from(ctx).config, &self.0).1
    }
    async fn mirror_url(&self) -> Option<String> {
        None
    }
    async fn security_policy_url(&self) -> Option<String> {
        None
    }
    async fn created_at(&self) -> String {
        iso(self.0.repo.created_at)
    }
    async fn pushed_at(&self) -> Option<String> {
        iso_opt(self.0.repo.pushed_at)
    }
    async fn updated_at(&self) -> String {
        iso(self.0.repo.updated_at)
    }
    async fn archived_at(&self) -> Option<String> {
        None
    }
    async fn is_blank_issues_enabled(&self) -> bool {
        true
    }
    async fn is_security_policy_enabled(&self) -> bool {
        false
    }
    async fn has_issues_enabled(&self) -> bool {
        Self::b(self.0.repo.has_issues)
    }
    async fn has_projects_enabled(&self) -> bool {
        Self::b(self.0.repo.has_projects)
    }
    async fn has_wiki_enabled(&self) -> bool {
        Self::b(self.0.repo.has_wiki)
    }
    async fn has_discussions_enabled(&self) -> bool {
        Self::b(self.0.repo.has_discussions)
    }
    async fn merge_commit_allowed(&self) -> bool {
        Self::b(self.0.repo.allow_merge_commit)
    }
    async fn squash_merge_allowed(&self) -> bool {
        Self::b(self.0.repo.allow_squash_merge)
    }
    async fn rebase_merge_allowed(&self) -> bool {
        Self::b(self.0.repo.allow_rebase_merge)
    }
    async fn auto_merge_allowed(&self) -> bool {
        Self::b(self.0.repo.allow_auto_merge)
    }
    async fn delete_branch_on_merge(&self) -> bool {
        Self::b(self.0.repo.delete_branch_on_merge)
    }
    async fn fork_count(&self) -> i32 {
        self.0.repo.forks_count as i32
    }
    async fn stargazer_count(&self) -> i32 {
        self.0.repo.stars_count as i32
    }
    async fn watchers(&self) -> CountConnection {
        CountConnection { total_count: self.0.repo.watchers_count as i32 }
    }
    async fn stargazers(&self) -> CountConnection {
        CountConnection { total_count: self.0.repo.stars_count as i32 }
    }
    #[allow(clippy::too_many_arguments)]
    async fn issues(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        states: Option<Vec<IssueState>>,
        labels: Option<Vec<String>>,
        filter_by: Option<super::issues::IssueFilters>,
        order_by: Option<super::issues::IssueOrder>,
    ) -> Result<super::issues::IssueConnection> {
        let f = super::issues::issue_filter(states.as_deref(), labels.as_deref(), filter_by.as_ref(), order_by.as_ref(), Some(false));
        let (items, info, total) = super::issues::issue_page(ctx, Some(self.0.repo.id()), &f, first, after).await?;
        Ok(super::issues::IssueConnection {
            nodes: items.iter().map(|(_, i)| super::issues::Issue { repo: self.0.clone(), i: i.clone() }).collect(),
            edges: items
                .into_iter()
                .map(|(k, i)| super::issues::IssueEdge { cursor: cursor(k), node: super::issues::Issue { repo: self.0.clone(), i } })
                .collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn issue(&self, ctx: &Context<'_>, number: i32) -> Result<Option<super::issues::Issue>> {
        let c = Ctx::from(ctx);
        match c.db.issue_full_by_number(self.0.repo.id(), number as i64).await {
            Ok(i) if !i.issue.is_pull() => Ok(Some(super::issues::Issue { repo: self.0.clone(), i })),
            _ => Err(not_found(format!("Could not resolve to an Issue with the number of {number}."))),
        }
    }
    async fn issue_or_pull_request(&self, ctx: &Context<'_>, number: i32) -> Result<Option<super::issues::IssueOrPullRequest>> {
        let c = Ctx::from(ctx);
        match c.db.issue_full_by_number(self.0.repo.id(), number as i64).await {
            Ok(i) => Ok(Some(super::issues::wrap(&self.0, i))),
            Err(_) => Err(not_found(format!("Could not resolve to an issue or pull request with the number of {number}."))),
        }
    }
    async fn pull_request(&self, ctx: &Context<'_>, number: i32) -> Result<Option<super::issues::PullRequest>> {
        let c = Ctx::from(ctx);
        match c.db.issue_full_by_number(self.0.repo.id(), number as i64).await {
            Ok(i) if i.issue.is_pull() => Ok(Some(super::issues::PullRequest { repo: self.0.clone(), i })),
            _ => Err(not_found(format!("Could not resolve to a PullRequest with the number of {number}."))),
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn pull_requests(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        states: Option<Vec<PullRequestState>>,
        labels: Option<Vec<String>>,
        head_ref_name: Option<String>,
        base_ref_name: Option<String>,
        order_by: Option<super::issues::IssueOrder>,
    ) -> Result<super::issues::PullRequestConnection> {
        let c = Ctx::from(ctx);
        let state = match states.as_deref() {
            None => "all".to_string(),
            Some(s) if s.contains(&PullRequestState::Open) && s.len() == 1 => "open".into(),
            Some(s) if !s.contains(&PullRequestState::Open) => "closed".into(),
            _ => "all".into(),
        };
        let mut f = super::issues::issue_filter(None, labels.as_deref(), None, order_by.as_ref(), Some(true));
        f.state = state;
        let (items, _, _) = super::issues::issue_page(ctx, Some(self.0.repo.id()), &f, Some(500), None).await?;
        let mut selected = Vec::new();
        for (_, i) in items {
            let m = c.db.pull_meta(i.issue.id()).await.ok();
            if let Some(h) = &head_ref_name
                && m.as_ref().map(|m| &m.head_ref) != Some(h)
            {
                continue;
            }
            if let Some(b) = &base_ref_name
                && m.as_ref().map(|m| &m.base_ref) != Some(b)
            {
                continue;
            }
            if let Some(st) = &states {
                let merged = m.as_ref().map(|m| m.merged != 0).unwrap_or(false);
                let cur = if i.issue.is_open() {
                    PullRequestState::Open
                } else if merged {
                    PullRequestState::Merged
                } else {
                    PullRequestState::Closed
                };
                if !st.contains(&cur) {
                    continue;
                }
            }
            selected.push(i);
        }
        let (items, info, total) = page(selected, first, after);
        Ok(super::issues::PullRequestConnection {
            nodes: items.iter().map(|(_, i)| super::issues::PullRequest { repo: self.0.clone(), i: i.clone() }).collect(),
            edges: items
                .into_iter()
                .map(|(k, i)| super::issues::PullRequestEdge {
                    cursor: cursor(k),
                    node: super::issues::PullRequest { repo: self.0.clone(), i },
                })
                .collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn label(&self, ctx: &Context<'_>, name: String) -> Result<Option<super::issues::Label>> {
        let c = Ctx::from(ctx);
        Ok(c.db.label_by_name(self.0.repo.id(), &name).await.ok().map(|l| super::issues::Label { repo: self.0.clone(), l }))
    }
    async fn milestone(&self, ctx: &Context<'_>, number: i32) -> Result<Option<super::issues::Milestone>> {
        let c = Ctx::from(ctx);
        Ok(c.db
            .milestone_by_number(self.0.repo.id(), number as i64)
            .await
            .ok()
            .map(|m| super::issues::Milestone { repo: self.0.clone(), m }))
    }
    async fn suggested_actors(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        query: Option<String>,
        _capabilities: Option<Vec<super::issues::ActorCapability>>,
    ) -> Result<super::issues::AssigneeConnection> {
        let mut all = self.assignable_users(ctx, Some(100), None).await?.nodes.into_iter().map(|u| u.0).collect::<Vec<_>>();
        if let Some(q) = query {
            let q = q.to_lowercase();
            all.retain(|a| {
                a.login.to_lowercase().contains(&q) || a.name.as_deref().map(|n| n.to_lowercase().contains(&q)).unwrap_or(false)
            });
        }
        let (items, info, total) = page(all, first, after);
        Ok(super::issues::AssigneeConnection {
            nodes: items.into_iter().map(|(_, a)| super::issues::Assignee::User(User(a))).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn code_of_conduct(&self) -> Option<CodeOfConduct> {
        None
    }
    async fn contact_links(&self) -> Vec<RepositoryContactLink> {
        vec![]
    }
    async fn funding_links(&self) -> Vec<FundingLink> {
        vec![]
    }
    async fn default_branch_ref(&self, ctx: &Context<'_>) -> Result<Option<Ref>> {
        let c = Ctx::from(ctx);
        let path = c.store.path(self.0.repo.id());
        let want = self.0.repo.default_branch.clone();
        let branches = tokio::task::spawn_blocking(move || reef_git::RepoReader::open(&path)?.branches())
            .await
            .map_err(|e| from_core(reef_core::Error::internal(e)))?
            .map_err(from_core)?;
        Ok(branches.into_iter().find(|(n, _)| *n == want).map(|(name, sha)| Ref {
            repo: self.0.clone(),
            name,
            prefix: "refs/heads/".into(),
            sha,
        }))
    }
    #[graphql(name = "ref")]
    async fn git_ref(&self, ctx: &Context<'_>, qualified_name: String) -> Result<Option<Ref>> {
        let c = Ctx::from(ctx);
        let path = c.store.path(self.0.repo.id());
        let (prefix, name) = if let Some(n) = qualified_name.strip_prefix("refs/heads/") {
            ("refs/heads/", n.to_string())
        } else if let Some(n) = qualified_name.strip_prefix("refs/tags/") {
            ("refs/tags/", n.to_string())
        } else {
            ("refs/heads/", qualified_name.clone())
        };
        let full = format!("{prefix}{name}");
        let sha = tokio::task::spawn_blocking(move || reef_git::RepoReader::open(&path)?.resolve(&full))
            .await
            .map_err(|e| from_core(reef_core::Error::internal(e)))?;
        match sha {
            Ok(sha) => Ok(Some(Ref { repo: self.0.clone(), name, prefix: prefix.into(), sha })),
            Err(reef_core::Error::NotFound) => Ok(None),
            Err(e) => Err(from_core(e)),
        }
    }
    async fn refs(&self, ctx: &Context<'_>, ref_prefix: String, first: Option<i32>, after: Option<String>) -> Result<RefConnection> {
        let c = Ctx::from(ctx);
        let path = c.store.path(self.0.repo.id());
        let prefix = ref_prefix.clone();
        let list = tokio::task::spawn_blocking(move || {
            let r = reef_git::RepoReader::open(&path)?;
            if prefix == "refs/tags/" { r.tags() } else { r.branches() }
        })
        .await
        .map_err(|e| from_core(reef_core::Error::internal(e)))?
        .map_err(from_core)?;
        let (items, info, total) = page(list, first, after);
        Ok(RefConnection {
            nodes: items.into_iter().map(|(_, (name, sha))| Ref { repo: self.0.clone(), name, prefix: ref_prefix.clone(), sha }).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn object(&self, ctx: &Context<'_>, expression: Option<String>, oid: Option<String>) -> Result<Option<Commit>> {
        let c = Ctx::from(ctx);
        let path = c.store.path(self.0.repo.id());
        let rev = oid.or(expression).unwrap_or_else(|| self.0.repo.default_branch.clone());
        let res = tokio::task::spawn_blocking(move || {
            let r = reef_git::RepoReader::open(&path)?;
            let id = r.resolve(&rev)?;
            r.commit(&id)
        })
        .await
        .map_err(|e| from_core(reef_core::Error::internal(e)))?;
        match res {
            Ok(commit) => Ok(Some(Commit { repo: self.0.clone(), c: commit })),
            Err(reef_core::Error::NotFound) => Ok(None),
            Err(e) => Err(from_core(e)),
        }
    }
    async fn disk_usage(&self) -> i32 {
        self.0.repo.size_kb as i32
    }
    async fn is_archived(&self) -> bool {
        Self::b(self.0.repo.archived)
    }
    async fn is_disabled(&self) -> bool {
        false
    }
    async fn is_empty(&self) -> bool {
        Self::b(self.0.repo.is_empty)
    }
    async fn is_fork(&self) -> bool {
        Self::b(self.0.repo.is_fork)
    }
    async fn is_in_organization(&self) -> bool {
        self.0.owner.is_org()
    }
    async fn is_locked(&self) -> bool {
        false
    }
    async fn is_mirror(&self) -> bool {
        false
    }
    async fn is_private(&self) -> bool {
        Self::b(self.0.repo.private)
    }
    async fn visibility(&self) -> RepositoryVisibility {
        if Self::b(self.0.repo.private) { RepositoryVisibility::Private } else { RepositoryVisibility::Public }
    }
    async fn is_template(&self) -> bool {
        Self::b(self.0.repo.is_template)
    }
    async fn is_user_configuration_repository(&self) -> bool {
        self.0.owner.is_user() && self.0.repo.name.eq_ignore_ascii_case(&self.0.owner.login)
    }
    async fn license_info(&self) -> Option<License> {
        None
    }
    async fn viewer_can_administer(&self, ctx: &Context<'_>) -> Result<bool> {
        Ok(self.perm(ctx).await?.can_admin())
    }
    async fn viewer_can_update_topics(&self, ctx: &Context<'_>) -> Result<bool> {
        Ok(self.perm(ctx).await?.can_write())
    }
    async fn viewer_can_subscribe(&self) -> bool {
        true
    }
    async fn viewer_default_commit_email(&self, ctx: &Context<'_>) -> Option<String> {
        Ctx::from(ctx).viewer().and_then(|v| v.email.clone())
    }
    async fn viewer_default_merge_method(&self) -> PullRequestMergeMethod {
        if Self::b(self.0.repo.allow_merge_commit) {
            PullRequestMergeMethod::Merge
        } else if Self::b(self.0.repo.allow_squash_merge) {
            PullRequestMergeMethod::Squash
        } else {
            PullRequestMergeMethod::Rebase
        }
    }
    async fn viewer_has_starred(&self) -> bool {
        false
    }
    async fn viewer_permission(&self, ctx: &Context<'_>) -> Result<Option<RepositoryPermission>> {
        Ok(RepositoryPermission::from(self.perm(ctx).await?))
    }
    async fn viewer_possible_commit_emails(&self, ctx: &Context<'_>) -> Option<Vec<String>> {
        Ctx::from(ctx).viewer().map(|v| v.email.clone().into_iter().collect())
    }
    async fn viewer_subscription(&self) -> Option<SubscriptionState> {
        Some(SubscriptionState::Unsubscribed)
    }
    async fn repository_topics(&self, _first: Option<i32>) -> RepositoryTopicConnection {
        RepositoryTopicConnection { nodes: vec![], total_count: 0 }
    }
    async fn primary_language(&self) -> Option<Language> {
        None
    }
    async fn languages(&self, _first: Option<i32>) -> LanguageConnection {
        LanguageConnection { edges: vec![], nodes: vec![], total_count: 0, total_size: 0 }
    }
    async fn issue_templates(&self) -> Vec<IssueTemplate> {
        vec![]
    }
    async fn pull_request_templates(&self) -> Vec<PullRequestTemplate> {
        vec![]
    }
    async fn labels(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        query: Option<String>,
        _order_by: Option<super::issues::LabelOrder>,
    ) -> Result<super::issues::LabelConnection2> {
        let c = Ctx::from(ctx);
        let mut all = c.db.labels(self.0.repo.id()).await.map_err(from_core)?;
        if let Some(q) = query {
            let q = q.to_lowercase();
            all.retain(|l| l.name.to_lowercase().contains(&q));
        }
        let (items, info, total) = page(all, first, after);
        Ok(super::issues::LabelConnection2 {
            nodes: items.iter().map(|(_, l)| super::issues::Label { repo: self.0.clone(), l: l.clone() }).collect(),
            edges: items
                .into_iter()
                .map(|(k, l)| super::issues::LabelEdge { cursor: cursor(k), node: super::issues::Label { repo: self.0.clone(), l } })
                .collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn releases(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        order_by: Option<ReleaseOrder>,
    ) -> Result<ReleaseConnection> {
        let c = Ctx::from(ctx);
        let perm = c.db.permission(c.viewer(), &self.0).await.map_err(from_core)?;
        let mut rels = c.db.releases(self.0.repo.id(), perm >= reef_db::repos::Permission::Write).await.map_err(from_core)?;
        if let Some(o) = order_by {
            match (o.field, o.direction) {
                (ReleaseOrderField::Name, OrderDirection::Asc) => rels.sort_by(|a, b| a.name.cmp(&b.name)),
                (ReleaseOrderField::Name, OrderDirection::Desc) => rels.sort_by(|a, b| b.name.cmp(&a.name)),
                (ReleaseOrderField::CreatedAt, OrderDirection::Asc) => rels.sort_by_key(|r| r.created_at),
                (ReleaseOrderField::CreatedAt, OrderDirection::Desc) => rels.sort_by_key(|r| std::cmp::Reverse(r.created_at)),
            }
        }
        let latest = c.db.latest_release(self.0.repo.id()).await.ok().map(|r| r.id);
        let total = rels.len();
        let (items, info, _) = page(rels, first, after);
        let s = super::app(ctx);
        let mut nodes = Vec::new();
        for (_, r) in items {
            let author = match r.author_id {
                Some(a) => c.db.account_by_id(reef_core::Id(a)).await.ok().map(User),
                None => None,
            };
            nodes.push(Release {
                id: ID(shapes::node_id("RE", r.id)),
                database_id: r.id,
                name: r.name.clone(),
                tag_name: r.tag_name.clone(),
                description: r.body.clone(),
                is_draft: r.draft != 0,
                is_prerelease: r.prerelease != 0,
                is_latest: latest == Some(r.id),
                created_at: reef_core::time::iso(r.created_at),
                published_at: reef_core::time::iso_opt(r.published_at),
                url: format!("{}/{}/releases/tag/{}", s.config.site_url(), self.0.full_name(), shapes::urlpath(&r.tag_name)),
                author,
                tag_commit: None,
            });
        }
        Ok(ReleaseConnection { nodes, page_info: info, total_count: total as i32 })
    }
    async fn latest_release(&self, ctx: &Context<'_>) -> Result<Option<Release>> {
        let conn = self.releases(ctx, Some(100), None, None).await?;
        Ok(conn.nodes.into_iter().find(|r| r.is_latest))
    }
    async fn milestones(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        states: Option<Vec<MilestoneState>>,
        query: Option<String>,
        _order_by: Option<super::issues::MilestoneOrder>,
    ) -> Result<super::issues::MilestoneConnection2> {
        let c = Ctx::from(ctx);
        let st = match states.as_deref() {
            None => "all",
            Some(s) if s.contains(&MilestoneState::Open) && s.contains(&MilestoneState::Closed) => "all",
            Some(s) if s.contains(&MilestoneState::Closed) => "closed",
            _ => "open",
        };
        let mut all = c.db.milestones(self.0.repo.id(), st).await.map_err(from_core)?;
        if let Some(q) = query {
            let q = q.to_lowercase();
            all.retain(|m| m.title.to_lowercase().contains(&q));
        }
        let (items, info, total) = page(all, first, after);
        Ok(super::issues::MilestoneConnection2 {
            nodes: items.into_iter().map(|(_, m)| super::issues::Milestone { repo: self.0.clone(), m }).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn assignable_users(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<UserConnection> {
        let c = Ctx::from(ctx);
        let mut users: Vec<Account> = Vec::new();
        if self.0.owner.is_user() {
            users.push(self.0.owner.clone());
        } else {
            users.extend(c.db.org_members(self.0.owner.id(), None).await.map_err(from_core)?.into_iter().map(|(a, _)| a));
        }
        for (a, _) in c.db.collaborators(self.0.repo.id()).await.map_err(from_core)? {
            if !users.iter().any(|u| u.id == a.id) {
                users.push(a);
            }
        }
        let (items, info, total) = page(users, first, after);
        Ok(UserConnection { nodes: items.into_iter().map(|(_, a)| User(a)).collect(), page_info: info, total_count: total as i32 })
    }
    async fn mentionable_users(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<UserConnection> {
        self.assignable_users(ctx, first, after).await
    }
    async fn projects(&self, _first: Option<i32>, _states: Option<Vec<ProjectState>>) -> ProjectConnection {
        ProjectConnection { nodes: vec![], total_count: 0 }
    }
    #[graphql(name = "projectsV2")]
    async fn projects_v2(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        query: Option<String>,
    ) -> Result<super::projects::ProjectV2Connection> {
        let c = Ctx::from(ctx);
        let s = super::app(ctx);
        let mut all = Vec::new();
        for p in c.db.projects_of_repo(self.0.repo.id()).await.map_err(from_core)? {
            let owner = c.db.account_by_id(reef_core::Id(p.owner_id)).await.map_err(from_core)?;
            let a = crate::projects::access(&s, c.viewer(), &owner, Some(&p)).await.map_err(|e| from_core(e.0))?;
            if a >= crate::projects::Access::Read && query.as_deref().map(|q| !q.contains("is:closed") || p.closed != 0).unwrap_or(true) {
                all.push(super::projects::ProjectV2Obj { owner, p });
            }
        }
        let total = all.len();
        let (items, info, _) = page(all, first, after);
        Ok(super::projects::ProjectV2Connection {
            nodes: items.into_iter().map(|(_, p)| p).collect(),
            page_info: info,
            total_count: total as i32,
        })
    }
    async fn forks(&self, _first: Option<i32>, _affiliations: Option<Vec<RepositoryAffiliation>>) -> RepositoryConnection {
        RepositoryConnection {
            nodes: vec![],
            edges: vec![],
            page_info: PageInfo { has_next_page: false, has_previous_page: false, start_cursor: None, end_cursor: None },
            total_count: 0,
        }
    }
    async fn collaborators(&self, ctx: &Context<'_>, first: Option<i32>, after: Option<String>) -> Result<UserConnection> {
        if !self.perm(ctx).await?.can_write() {
            return Err(forbidden_err());
        }
        self.assignable_users(ctx, first, after).await
    }
}

fn forbidden_err() -> async_graphql::Error {
    super::forbidden("Must have push access to view repository collaborators.")
}

#[derive(SimpleObject)]
pub struct RefConnection {
    pub nodes: Vec<Ref>,
    pub page_info: PageInfo,
    pub total_count: i32,
}

// Keep the ComplexObject import used for future split types.
#[derive(SimpleObject)]
#[graphql(complex)]
pub struct Placeholder {
    pub id: ID,
}

#[ComplexObject]
impl Placeholder {
    async fn nothing(&self) -> bool {
        false
    }
}

pub fn _unused(_: &str) -> Option<String> {
    not_found("").message.into()
}
