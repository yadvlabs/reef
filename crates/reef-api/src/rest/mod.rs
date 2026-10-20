//! GitHub REST API v3.

pub mod contents;
pub mod issues;
pub mod labels;
pub mod meta;
pub mod orgs;
pub mod protection;
pub mod pulls;
pub mod releases;
pub mod repos;
pub mod search;
pub mod social;
pub mod users;

use crate::AppState;
use crate::timeout::exempt;
use axum::Router;
use axum::http::HeaderMap;
use axum::routing::{get, post};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(meta::root))
        .route("/meta", get(meta::meta))
        .route("/versions", get(meta::versions))
        .route("/rate_limit", get(meta::rate_limit))
        .route("/zen", get(meta::zen))
        .route("/octocat", get(meta::octocat))
        .route("/emojis", get(meta::emojis))
        .route("/gitignore/templates", get(meta::gitignore_templates))
        .route("/licenses", get(meta::licenses))
        // users
        .route("/user", get(users::current).patch(users::update_current))
        .route("/users", get(users::list))
        .route("/users/{login}", get(users::show))
        .route("/user/keys", get(users::list_keys).post(users::add_key))
        .route("/user/keys/{id}", get(users::show_key).delete(users::delete_key))
        .route("/users/{login}/keys", get(users::public_keys))
        .route("/user/emails", get(users::emails).post(users::add_emails).delete(users::delete_emails))
        .route("/user/emails/primary", axum::routing::patch(users::set_primary_email))
        .route("/user/public_emails", get(users::public_emails))
        .route("/user/email/visibility", axum::routing::patch(users::set_email_visibility))
        .route("/user/ssh_signing_keys", get(users::list_signing_keys).post(users::add_signing_key))
        .route("/user/ssh_signing_keys/{id}", get(users::show_signing_key).delete(users::delete_signing_key))
        .route("/user/gpg_keys", get(users::list_gpg_keys).post(users::add_gpg_key))
        .route("/user/gpg_keys/{id}", get(users::show_gpg_key).delete(users::delete_gpg_key))
        .route("/user/social_accounts", get(users::empty_list))
        .route("/users/{login}/ssh_signing_keys", get(users::public_signing_keys))
        .route("/users/{login}/gpg_keys", get(users::public_gpg_keys))
        .route("/search/repositories", get(search::repositories))
        .route("/search/users", get(search::users))
        .route("/search/issues", get(issues::search))
        .route("/search/code", get(search::empty))
        .route("/search/commits", get(search::empty))
        .route("/search/topics", get(search::empty))
        .route("/user/orgs", get(orgs::current_user_orgs))
        .route("/users/{login}/orgs", get(orgs::user_orgs))
        .route("/user/memberships/orgs", get(orgs::current_memberships))
        .route("/user/memberships/orgs/{org}", get(orgs::current_membership))
        // orgs
        .route("/organizations", get(orgs::list))
        .route("/admin/organizations", post(orgs::admin_create))
        .route("/orgs/{org}", get(orgs::show).patch(orgs::update).delete(orgs::delete))
        .route("/orgs/{org}/members", get(orgs::members))
        .route("/orgs/{org}/members/{login}", get(orgs::is_member).delete(orgs::remove_member))
        .route("/orgs/{org}/memberships/{login}", get(orgs::membership).put(orgs::set_membership).delete(orgs::remove_member))
        .route("/orgs/{org}/repos", get(repos::list_for_org).post(repos::create_in_org))
        // repos
        .route("/repositories", get(repos::list_public))
        .route("/user/repos", get(repos::list_for_current).post(repos::create_for_current))
        .route("/users/{login}/repos", get(repos::list_for_user))
        .route("/repos/{owner}/{repo}", get(repos::show).patch(repos::update).delete(repos::delete))
        .route("/repos/{owner}/{repo}/collaborators", get(repos::collaborators))
        .route(
            "/repos/{owner}/{repo}/collaborators/{login}",
            get(repos::is_collaborator).put(repos::add_collaborator).delete(repos::remove_collaborator),
        )
        .route("/repos/{owner}/{repo}/collaborators/{login}/permission", get(repos::collaborator_permission))
        .route("/repos/{owner}/{repo}/branches", get(contents::branches))
        .route("/repos/{owner}/{repo}/protection-rules", get(protection::list))
        .route(
            "/repos/{owner}/{repo}/protection-rules/{*pattern}",
            axum::routing::put(protection::put_rule).delete(protection::delete_rule),
        )
        // Branch names may contain `/`, so one catch-all serves both `/branches/{name}` and
        // `/branches/{name}/protection`.
        .route(
            "/repos/{owner}/{repo}/branches/{*branch}",
            get(protection::branch_get).put(protection::branch_put).delete(protection::branch_delete),
        )
        .route("/repos/{owner}/{repo}/tags", get(contents::tags))
        .route("/repos/{owner}/{repo}/commits", get(contents::commits))
        .route("/repos/{owner}/{repo}/commits/{sha}", get(contents::commit))
        .route("/repos/{owner}/{repo}/readme", get(contents::readme))
        .route("/repos/{owner}/{repo}/contents", get(contents::contents_root))
        .route(
            "/repos/{owner}/{repo}/contents/{*path}",
            get(contents::contents).put(contents::put_contents).delete(contents::delete_contents),
        )
        .route("/repos/{owner}/{repo}/git/trees/{sha}", get(contents::git_tree))
        .route("/repos/{owner}/{repo}/raw/{rev}/{*path}", get(contents::raw))
        .route("/repos/{owner}/{repo}/blame/{*path}", get(contents::blame))
        // Source archives stream: the only REST routes outside the request timeout.
        .route("/repos/{owner}/{repo}/tarball", get(exempt(releases::tarball_default)))
        .route("/repos/{owner}/{repo}/tarball/{*ref}", get(exempt(releases::tarball)))
        .route("/repos/{owner}/{repo}/zipball", get(exempt(releases::zipball_default)))
        .route("/repos/{owner}/{repo}/zipball/{*ref}", get(exempt(releases::zipball)))
        .route("/repos/{owner}/{repo}/contributors", get(repos::contributors))
        .route("/repos/{owner}/{repo}/git/refs", get(contents::git_refs).post(contents::create_ref))
        .route("/repos/{owner}/{repo}/git/refs/{*name}", axum::routing::patch(contents::update_ref).delete(contents::delete_ref))
        .route("/repos/{owner}/{repo}/git/ref/{*name}", get(contents::git_ref))
        // issues
        .route("/issues", get(issues::user_issues))
        .route("/user/issues", get(issues::user_issues))
        .route("/markdown", post(issues::markdown))
        .route("/repos/{owner}/{repo}/issues", get(issues::list).post(issues::create))
        .route("/repos/{owner}/{repo}/issues/comments", get(issues::list_repo_comments))
        .route(
            "/repos/{owner}/{repo}/issues/comments/{id}",
            get(issues::show_comment).patch(issues::update_comment).delete(issues::delete_comment),
        )
        .route("/repos/{owner}/{repo}/issues/comments/{id}/reactions", get(issues::comment_reactions).post(issues::add_comment_reaction))
        .route("/repos/{owner}/{repo}/issues/comments/{id}/reactions/{rid}", axum::routing::delete(issues::delete_comment_reaction))
        .route("/repos/{owner}/{repo}/issues/{number}", get(issues::show).patch(issues::update).delete(issues::delete))
        .route("/repos/{owner}/{repo}/issues/{number}/lock", axum::routing::put(issues::lock).delete(issues::unlock))
        .route("/repos/{owner}/{repo}/issues/{number}/comments", get(issues::list_comments).post(issues::create_comment))
        .route("/repos/{owner}/{repo}/issues/{number}/events", get(issues::events))
        .route("/repos/{owner}/{repo}/issues/{number}/timeline", get(issues::timeline))
        .route(
            "/repos/{owner}/{repo}/issues/{number}/labels",
            get(issues::issue_labels).post(issues::add_labels).put(issues::set_labels).delete(issues::clear_labels),
        )
        .route("/repos/{owner}/{repo}/issues/{number}/labels/{name}", axum::routing::delete(issues::remove_label))
        .route("/repos/{owner}/{repo}/issues/{number}/assignees", post(issues::add_assignees).delete(issues::remove_assignees))
        .route("/repos/{owner}/{repo}/issues/{number}/reactions", get(issues::issue_reactions).post(issues::add_issue_reaction))
        .route("/repos/{owner}/{repo}/issues/{number}/reactions/{rid}", axum::routing::delete(issues::delete_issue_reaction))
        .route("/repos/{owner}/{repo}/assignees", get(issues::assignees))
        .route("/repos/{owner}/{repo}/assignees/{login}", get(issues::check_assignee))
        .route("/repos/{owner}/{repo}/labels", get(labels::list).post(labels::create))
        .route("/repos/{owner}/{repo}/labels/{name}", get(labels::show).patch(labels::update).delete(labels::delete))
        .route("/repos/{owner}/{repo}/milestones", get(labels::list_milestones).post(labels::create_milestone))
        .route(
            "/repos/{owner}/{repo}/milestones/{number}",
            get(labels::show_milestone).patch(labels::update_milestone).delete(labels::delete_milestone),
        )
        .route("/repos/{owner}/{repo}/milestones/{number}/labels", get(labels::milestone_labels))
        // pull requests
        .route("/repos/{owner}/{repo}/pulls", get(pulls::list).post(pulls::create))
        .route("/repos/{owner}/{repo}/pulls/{number}", get(pulls::show).patch(pulls::update))
        .route("/repos/{owner}/{repo}/pulls/{number}/commits", get(pulls::commits))
        .route("/repos/{owner}/{repo}/pulls/{number}/files", get(pulls::files))
        .route("/repos/{owner}/{repo}/pulls/{number}/merge", get(pulls::is_merged).put(pulls::merge))
        .route("/repos/{owner}/{repo}/pulls/{number}/reviews", get(pulls::reviews).post(pulls::create_review))
        .route("/repos/{owner}/{repo}/pulls/{number}/reviews/{id}/events", post(pulls::submit_review))
        .route("/repos/{owner}/{repo}/pulls/{number}/reviews/{id}/dismissals", axum::routing::put(pulls::dismiss_review))
        .route("/repos/{owner}/{repo}/pulls/{number}/comments", get(pulls::review_comments).post(pulls::create_review_comment))
        .route(
            "/repos/{owner}/{repo}/pulls/{number}/reviews/{id}",
            get(pulls::review_show).put(pulls::review_update).delete(pulls::review_delete),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/comments/{id}",
            get(pulls::review_comment_show).patch(pulls::review_comment_update).delete(pulls::review_comment_delete),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/comments/{id}/resolve",
            axum::routing::put(pulls::resolve_thread).delete(pulls::unresolve_thread),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/{number}/requested_reviewers",
            get(pulls::requested_reviewers).post(pulls::request_reviewers).delete(pulls::remove_reviewers),
        )
        .route("/repos/{owner}/{repo}/compare/{*basehead}", get(pulls::compare))
        .route("/repos/{owner}/{repo}/statuses/{sha}", post(pulls::create_status))
        .route("/repos/{owner}/{repo}/commits/{ref}/statuses", get(pulls::list_statuses))
        .route("/repos/{owner}/{repo}/commits/{ref}/status", get(pulls::combined_status))
        .route("/repos/{owner}/{repo}/topics", get(repos::topics).put(repos::set_topics))
        .route("/repos/{owner}/{repo}/license", get(repos::license))
        .route("/user/starred", get(social::current_starred))
        .route("/user/starred/{owner}/{repo}", get(social::is_starred).put(social::star).delete(social::unstar))
        .route("/users/{login}/starred", get(social::user_starred))
        .route("/repos/{owner}/{repo}/stargazers", get(social::stargazers))
        .route(
            "/repos/{owner}/{repo}/subscription",
            get(social::subscription).put(social::set_subscription).delete(social::delete_subscription),
        )
        .route("/repos/{owner}/{repo}/subscribers", get(social::subscribers))
        .route("/user/subscriptions", get(social::current_subscriptions))
        .route("/repos/{owner}/{repo}/forks", get(social::list_forks).post(social::create_fork))
        .route("/repos/{owner}/{repo}/languages", get(repos::languages))
        .route("/repos/{owner}/{repo}/releases", get(releases::list).post(releases::create))
        .route("/repos/{owner}/{repo}/releases/latest", get(releases::latest))
        .route("/repos/{owner}/{repo}/releases/tags/{tag}", get(releases::by_tag))
        .route("/repos/{owner}/{repo}/releases/{id}", get(releases::show).patch(releases::update).delete(releases::delete))
        .fallback(meta::fallback)
        .merge(crate::actions::rest::router())
        .merge(crate::notifications::github_router())
        .merge(crate::stats::github_router())
        .merge(crate::webhooks::rest::router())
        .merge(crate::mirror::rest_router())
}

/// Standard pagination query (`page`, `per_page`). Parsed from the raw query string so it
/// can be combined with any other query struct.
#[derive(Debug, Clone)]
pub struct Page {
    pub page: i64,
    pub per_page: i64,
}

impl Default for Page {
    fn default() -> Self {
        Page { page: 1, per_page: 30 }
    }
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Page {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut axum::http::request::Parts, _s: &S) -> Result<Self, Self::Rejection> {
        let mut p = Page::default();
        if let Some(q) = parts.uri.query() {
            for kv in q.split('&') {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                match k {
                    "page" => p.page = v.parse().unwrap_or(1),
                    "per_page" => p.per_page = v.parse().unwrap_or(30),
                    _ => {}
                }
            }
        }
        Ok(p)
    }
}

impl Page {
    pub fn clamp(&self) -> (i64, i64) {
        (self.page.max(1), self.per_page.clamp(1, 100))
    }
    /// Slice a fully-materialised list and build the `Link` header.
    pub fn slice<T: Clone>(&self, all: &[T], base_url: &str) -> (Vec<T>, HeaderMap) {
        let (page, per) = self.clamp();
        let start = ((page - 1) * per) as usize;
        let items: Vec<T> = all.iter().skip(start).take(per as usize).cloned().collect();
        let total_pages = ((all.len() as i64 + per - 1) / per).max(1);
        let mut links = Vec::new();
        let sep = if base_url.contains('?') { "&" } else { "?" };
        let link = |p: i64, rel: &str| format!("<{base_url}{sep}page={p}&per_page={per}>; rel=\"{rel}\"");
        if page < total_pages {
            links.push(link(page + 1, "next"));
            links.push(link(total_pages, "last"));
        }
        if page > 1 {
            links.push(link(page - 1, "prev"));
            links.push(link(1, "first"));
        }
        let mut h = HeaderMap::new();
        if !links.is_empty() {
            h.insert("link", links.join(", ").parse().unwrap());
        }
        (items, h)
    }
}
