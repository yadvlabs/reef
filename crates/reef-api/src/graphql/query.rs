use super::types::*;
use super::{Ctx, from_core, not_found};
use crate::shapes;
use async_graphql::{Context, ID, Object, Result, Union};
use reef_core::time::iso;

pub struct Query;

#[Object]
impl Query {
    async fn viewer(&self, ctx: &Context<'_>) -> Result<User> {
        let c = Ctx::from(ctx);
        match c.viewer() {
            Some(v) => Ok(User(v.clone())),
            None => Err(super::gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated.")),
        }
    }

    async fn user(&self, ctx: &Context<'_>, login: String) -> Result<Option<User>> {
        let c = Ctx::from(ctx);
        match c.db.account_by_login(&login).await {
            Ok(a) if a.is_user() => Ok(Some(User(a))),
            Ok(_) | Err(reef_core::Error::NotFound) => Err(not_found(format!("Could not resolve to a User with the login of '{login}'."))),
            Err(e) => Err(from_core(e)),
        }
    }

    async fn organization(&self, ctx: &Context<'_>, login: String) -> Result<Option<Organization>> {
        let c = Ctx::from(ctx);
        match c.db.account_by_login(&login).await {
            Ok(a) if a.is_org() => Ok(Some(Organization(a))),
            Ok(_) | Err(reef_core::Error::NotFound) => {
                Err(not_found(format!("Could not resolve to an Organization with the login of '{login}'.")))
            }
            Err(e) => Err(from_core(e)),
        }
    }

    async fn repository_owner(&self, ctx: &Context<'_>, login: String) -> Result<Option<RepositoryOwner>> {
        let c = Ctx::from(ctx);
        match c.db.account_by_login(&login).await {
            Ok(a) => Ok(Some(RepositoryOwner::from_account(a))),
            Err(reef_core::Error::NotFound) => {
                Err(not_found(format!("Could not resolve to a RepositoryOwner with the login of '{login}'.")))
            }
            Err(e) => Err(from_core(e)),
        }
    }

    async fn repository(
        &self,
        ctx: &Context<'_>,
        owner: String,
        name: String,
        _follow_renames: Option<bool>,
    ) -> Result<Option<Repository>> {
        let c = Ctx::from(ctx);
        let nf = || not_found(format!("Could not resolve to a Repository with the name '{owner}/{name}'."));
        match c.db.repo_by_path(&owner, &name).await {
            Ok(rw) => {
                let perm = c.db.permission(c.viewer(), &rw).await.map_err(from_core)?;
                if perm.can_read() { Ok(Some(Repository(rw))) } else { Err(nf()) }
            }
            Err(reef_core::Error::NotFound) => Err(nf()),
            Err(e) => Err(from_core(e)),
        }
    }

    async fn node(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Node>> {
        let c = Ctx::from(ctx);
        let Some((prefix, num)) = shapes::decode_node_id(&id.0) else { return Ok(None) };
        let n = reef_core::Id(num);
        match prefix.as_str() {
            shapes::NODE_USER | shapes::NODE_ORG => match c.db.account_by_id(n).await {
                Ok(a) if a.is_org() => Ok(Some(Node::Organization(Organization(a)))),
                Ok(a) => Ok(Some(Node::User(User(a)))),
                Err(_) => Ok(None),
            },
            shapes::NODE_REPO => match c.db.repo_with_owner_by_id(n).await {
                Ok(rw) => {
                    let perm = c.db.permission(c.viewer(), &rw).await.map_err(from_core)?;
                    Ok(if perm.can_read() { Some(Node::Repository(Repository(rw))) } else { None })
                }
                Err(_) => Ok(None),
            },
            shapes::NODE_ISSUE | shapes::NODE_PULL => match super::issues::load_issue(ctx, &id).await {
                Ok((rw, i)) => Ok(Some(match super::issues::wrap(&rw, i) {
                    super::issues::IssueOrPullRequest::Issue(i) => Node::Issue(i),
                    super::issues::IssueOrPullRequest::PullRequest(p) => Node::PullRequest(p),
                })),
                Err(_) => Ok(None),
            },
            _ => Ok(None),
        }
    }

    async fn nodes(&self, ctx: &Context<'_>, ids: Vec<ID>) -> Result<Vec<Option<Node>>> {
        let mut out = Vec::new();
        for id in ids {
            out.push(self.node(ctx, id).await?);
        }
        Ok(out)
    }

    /// Resolve an issue or pull request from its web URL (`gh project item-add --url`).
    async fn resource(&self, ctx: &Context<'_>, url: Uri) -> Result<Option<super::issues::IssueOrPullRequest>> {
        let c = Ctx::from(ctx);
        let url = url.0;
        let path = url.split("://").nth(1).and_then(|r| r.split_once('/')).map(|(_, p)| p.to_string()).unwrap_or(url.clone());
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        if parts.len() < 4 || (parts[2] != "issues" && parts[2] != "pull") {
            return Ok(None);
        }
        let number: i64 = parts[3].parse().map_err(|_| not_found("Could not resolve to a node."))?;
        let rw = c.db.repo_by_path(parts[0], parts[1]).await.map_err(|_| not_found("Could not resolve to a node."))?;
        if !c.db.permission(c.viewer(), &rw).await.map_err(from_core)?.can_read() {
            return Ok(None);
        }
        let i = c.db.issue_full_by_number(rw.repo.id(), number).await.map_err(|_| not_found("Could not resolve to a node."))?;
        Ok(Some(super::issues::wrap(&rw, i)))
    }

    /// The caller's budget as the limiter sees it; a caller rate limiting is off for reports
    /// a full budget.
    async fn rate_limit(&self, ctx: &Context<'_>, _dry_run: Option<bool>) -> RateLimit {
        let key = ctx.data_opt::<Option<crate::ratelimit::RateKey>>().copied().flatten();
        let status = key.and_then(|k| super::app(ctx).limiter.status(k.0).status());
        match status {
            Some(st) => RateLimit {
                limit: st.limit as i32,
                cost: 1,
                remaining: st.remaining as i32,
                used: st.used as i32,
                reset_at: iso(st.reset * 1000),
                node_count: 0,
            },
            None => RateLimit {
                limit: 5000,
                cost: 1,
                remaining: 5000,
                used: 0,
                reset_at: iso(reef_core::time::now_ms() + 3_600_000),
                node_count: 0,
            },
        }
    }

    /// Search. Only repositories are searchable for now; other types return no results.
    async fn search(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(name = "type")] kind: SearchType,
        first: Option<i32>,
        after: Option<String>,
    ) -> Result<SearchResultItemConnection> {
        let c = Ctx::from(ctx);
        let mut items: Vec<SearchResultItem> = Vec::new();
        if kind == SearchType::Repository {
            let (terms, filters) = parse_search(&query);
            let candidates =
                if let Some(owner) = filters.iter().find_map(|(k, v)| if k == "user" || k == "org" { Some(v.clone()) } else { None }) {
                    match c.db.account_by_login(&owner).await {
                        Ok(a) => c.db.repos_for_owner(&a, c.viewer(), 1, 10_000).await.map_err(from_core)?,
                        Err(_) => vec![],
                    }
                } else {
                    let mut all = c.db.public_repos(0, 1000).await.map_err(from_core)?;
                    if let Some(v) = c.viewer() {
                        for rw in c.db.repos_for_user(v, "owner,collaborator,organization_member").await.map_err(from_core)? {
                            if !all.iter().any(|r| r.repo.id == rw.repo.id) {
                                all.push(rw);
                            }
                        }
                    }
                    all
                };
            for rw in candidates {
                let hay = format!("{} {}", rw.full_name(), rw.repo.description.clone().unwrap_or_default()).to_lowercase();
                if terms.iter().all(|t| hay.contains(&t.to_lowercase())) {
                    let ok = filters.iter().all(|(k, v)| match k.as_str() {
                        "is" => match v.as_str() {
                            "public" => !rw.repo.is_private(),
                            "private" => rw.repo.is_private(),
                            "archived" => rw.repo.archived != 0,
                            "fork" => rw.repo.is_fork != 0,
                            _ => true,
                        },
                        "fork" => (v == "true" || v == "only") == (rw.repo.is_fork != 0) || v == "true",
                        "archived" => (v == "true") == (rw.repo.archived != 0),
                        _ => true,
                    });
                    if ok {
                        items.push(SearchResultItem::Repository(Repository(rw)));
                    }
                }
            }
        }
        if kind == SearchType::Issue {
            let (terms, quals) = parse_search(&query);
            let state = ctx.data_unchecked::<crate::AppStateHandle>().0.clone();
            let (repo, f) =
                crate::rest::issues::filter_from_search(&state, &terms, &quals, None, None, c.viewer().map(|v| v.login.as_str()))
                    .await
                    .map_err(|e| from_core(e.0))?;
            let (page_items, info, total) = super::issues::issue_page(ctx, repo.as_ref().map(|r| r.repo.id()), &f, first, after).await?;
            let mut nodes = Vec::new();
            for (_, i) in page_items {
                let rw = match &repo {
                    Some(r) => r.clone(),
                    None => match c.db.repo_with_owner_by_id(reef_core::Id(i.issue.repo_id)).await {
                        Ok(rw) if c.db.permission(c.viewer(), &rw).await.map_err(from_core)?.can_read() => rw,
                        _ => continue,
                    },
                };
                nodes.push(match super::issues::wrap(&rw, i) {
                    super::issues::IssueOrPullRequest::Issue(i) => SearchResultItem::Issue(i),
                    super::issues::IssueOrPullRequest::PullRequest(p) => SearchResultItem::PullRequest(p),
                });
            }
            let edges = nodes.iter().enumerate().map(|(k, n)| SearchResultItemEdge { cursor: cursor(k), node: n.clone() }).collect();
            return Ok(SearchResultItemConnection {
                nodes,
                edges,
                page_info: info,
                repository_count: 0,
                issue_count: total as i32,
                user_count: 0,
                discussion_count: 0,
            });
        }
        let count = items.len() as i32;
        let (page_items, info, _) = page(items, first, after);
        let nodes: Vec<SearchResultItem> = page_items.into_iter().map(|(_, i)| i).collect();
        Ok(SearchResultItemConnection {
            edges: nodes.iter().enumerate().map(|(k, n)| SearchResultItemEdge { cursor: cursor(k), node: n.clone() }).collect(),
            nodes,
            page_info: info,
            repository_count: if kind == SearchType::Repository { count } else { 0 },
            issue_count: 0,
            user_count: 0,
            discussion_count: 0,
        })
    }
}

/// Split a GitHub search string into free terms and `key:value` qualifiers.
pub fn parse_search(q: &str) -> (Vec<String>, Vec<(String, String)>) {
    let mut terms = Vec::new();
    let mut filters = Vec::new();
    for tok in q.split_whitespace() {
        match tok.split_once(':') {
            Some((k, v)) if !k.is_empty() && !v.is_empty() => filters.push((k.to_string(), v.trim_matches('"').to_string())),
            _ => terms.push(tok.trim_matches('"').to_string()),
        }
    }
    (terms, filters)
}

#[derive(Union, Clone)]
pub enum SearchResultItem {
    Repository(Repository),
    User(User),
    Organization(Organization),
    Issue(super::issues::Issue),
    PullRequest(super::issues::PullRequest),
}

#[derive(async_graphql::SimpleObject)]
pub struct SearchResultItemEdge {
    pub cursor: String,
    pub node: SearchResultItem,
}

#[derive(async_graphql::SimpleObject)]
pub struct SearchResultItemConnection {
    pub nodes: Vec<SearchResultItem>,
    pub edges: Vec<SearchResultItemEdge>,
    pub page_info: PageInfo,
    pub repository_count: i32,
    pub issue_count: i32,
    pub user_count: i32,
    pub discussion_count: i32,
}
