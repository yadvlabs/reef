use super::types::*;
use super::{Ctx, forbidden, from_core, gql_error, not_found};
use crate::shapes;
use async_graphql::{Context, ID, InputObject, Object, Result, SimpleObject};
use reef_db::repos::{NewRepo, RepoPatch};

#[derive(Default)]
pub struct RepoMutations;

#[derive(InputObject)]
pub struct CreateRepositoryInput {
    pub name: String,
    pub owner_id: Option<ID>,
    pub description: Option<String>,
    pub visibility: RepositoryVisibility,
    pub template: Option<bool>,
    pub homepage_url: Option<String>,
    pub has_wiki_enabled: Option<bool>,
    pub has_issues_enabled: Option<bool>,
    pub team_id: Option<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct CreateRepositoryPayload {
    pub repository: Option<Repository>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct UpdateRepositoryInput {
    pub repository_id: ID,
    pub name: Option<String>,
    pub description: Option<String>,
    pub template: Option<bool>,
    pub homepage_url: Option<String>,
    pub has_wiki_enabled: Option<bool>,
    pub has_issues_enabled: Option<bool>,
    pub has_projects_enabled: Option<bool>,
    pub has_discussions_enabled: Option<bool>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UpdateRepositoryPayload {
    pub repository: Option<Repository>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct CloneTemplateRepositoryInput {
    pub repository_id: ID,
    pub name: String,
    pub owner_id: ID,
    pub visibility: RepositoryVisibility,
    pub description: Option<String>,
    pub include_all_branches: Option<bool>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct CloneTemplateRepositoryPayload {
    pub repository: Option<Repository>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteRepositoryInput {
    pub repository_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct DeleteRepositoryPayload {
    pub client_mutation_id: Option<String>,
}

#[Object]
impl RepoMutations {
    async fn create_repository(&self, ctx: &Context<'_>, input: CreateRepositoryInput) -> Result<CreateRepositoryPayload> {
        let c = Ctx::from(ctx);
        let Some(viewer) = c.viewer() else { return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated.")) };
        let owner = match &input.owner_id {
            Some(id) => {
                let (_, num) = shapes::decode_node_id(&id.0).ok_or_else(|| not_found("Could not resolve to a node with the given id."))?;
                c.db.account_by_id(reef_core::Id(num)).await.map_err(|_| not_found("Could not resolve to a node with the given id."))?
            }
            None => viewer.clone(),
        };
        if !c.db.can_create_in(viewer, &owner).await.map_err(from_core)? {
            return Err(forbidden("You do not have permission to create repositories on this account."));
        }
        let name = reef_core::names::sanitize_repo_name(input.name.trim());
        let new = NewRepo {
            name,
            description: input.description.filter(|d| !d.is_empty()),
            homepage: input.homepage_url.filter(|d| !d.is_empty()),
            private: !matches!(input.visibility, RepositoryVisibility::Public),
            default_branch: None,
            has_issues: input.has_issues_enabled,
            has_projects: None,
            has_wiki: input.has_wiki_enabled,
            is_template: input.template.unwrap_or(false),
        };
        let s = super::app(ctx);
        let rw = crate::rest::repos::create_repository(&s, viewer, &owner, new, false).await.map_err(from_core)?;
        Ok(CreateRepositoryPayload { repository: Some(Repository(rw)), client_mutation_id: input.client_mutation_id })
    }

    async fn update_repository(&self, ctx: &Context<'_>, input: UpdateRepositoryInput) -> Result<UpdateRepositoryPayload> {
        let c = Ctx::from(ctx);
        let Some(viewer) = c.viewer() else { return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated.")) };
        let (_, num) =
            shapes::decode_node_id(&input.repository_id.0).ok_or_else(|| not_found("Could not resolve to a node with the given id."))?;
        let rw =
            c.db.repo_with_owner_by_id(reef_core::Id(num))
                .await
                .map_err(|_| not_found("Could not resolve to a Repository with the given id."))?;
        let perm = c.db.permission(Some(viewer), &rw).await.map_err(from_core)?;
        if !perm.can_admin() {
            return Err(forbidden("You must have admin access to update this repository."));
        }
        let updated =
            c.db.update_repo(
                rw.repo.id(),
                RepoPatch {
                    name: input.name,
                    description: input.description.map(|d| if d.is_empty() { None } else { Some(d) }),
                    homepage: input.homepage_url.map(|d| if d.is_empty() { None } else { Some(d) }),
                    has_issues: input.has_issues_enabled,
                    has_projects: input.has_projects_enabled,
                    has_wiki: input.has_wiki_enabled,
                    is_template: input.template,
                    ..Default::default()
                },
            )
            .await
            .map_err(from_core)?;
        let rw = c.db.repo_with_owner_by_id(updated.id()).await.map_err(from_core)?;
        Ok(UpdateRepositoryPayload { repository: Some(Repository(rw)), client_mutation_id: input.client_mutation_id })
    }

    async fn clone_template_repository(
        &self,
        _ctx: &Context<'_>,
        _input: CloneTemplateRepositoryInput,
    ) -> Result<CloneTemplateRepositoryPayload> {
        Err(gql_error("UNPROCESSABLE", "Template repositories are not supported yet."))
    }

    /// Reef extension: GitHub has no GraphQL delete; the web UI uses this.
    async fn delete_repository(&self, ctx: &Context<'_>, input: DeleteRepositoryInput) -> Result<DeleteRepositoryPayload> {
        let c = Ctx::from(ctx);
        let Some(viewer) = c.viewer() else { return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated.")) };
        let (_, num) =
            shapes::decode_node_id(&input.repository_id.0).ok_or_else(|| not_found("Could not resolve to a node with the given id."))?;
        let rw =
            c.db.repo_with_owner_by_id(reef_core::Id(num))
                .await
                .map_err(|_| not_found("Could not resolve to a Repository with the given id."))?;
        let perm = c.db.permission(Some(viewer), &rw).await.map_err(from_core)?;
        if !perm.can_admin() {
            return Err(forbidden("You must have admin access to delete this repository."));
        }
        crate::rest::repos::delete_repository(&super::app(ctx), rw.repo.id()).await.map_err(from_core)?;
        Ok(DeleteRepositoryPayload { client_mutation_id: input.client_mutation_id })
    }
}

// ----- issues, comments, labels, assignees, reactions -----

use super::issues::{self as gi, IssueOrPullRequest, IssueStateReason, LockReason, ReactionContent};
use reef_db::issues::IssuePatch;
use reef_db::repos::Permission;

#[derive(InputObject)]
pub struct CreateIssueInput {
    pub repository_id: ID,
    pub title: String,
    pub body: Option<String>,
    pub assignee_ids: Option<Vec<ID>>,
    pub label_ids: Option<Vec<ID>>,
    pub milestone_id: Option<ID>,
    pub project_ids: Option<Vec<ID>>,
    pub issue_template: Option<String>,
    pub issue_type_id: Option<ID>,
    pub parent_issue_id: Option<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct CreateIssuePayload {
    pub issue: Option<gi::Issue>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct UpdateIssueInput {
    pub id: ID,
    pub title: Option<String>,
    pub body: Option<String>,
    pub assignee_ids: Option<Vec<ID>>,
    pub label_ids: Option<Vec<ID>>,
    pub milestone_id: Option<ID>,
    pub state: Option<IssueState>,
    pub project_ids: Option<Vec<ID>>,
    pub issue_type_id: Option<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UpdateIssuePayload {
    pub issue: Option<gi::Issue>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct CloseIssueInput {
    pub issue_id: ID,
    pub state_reason: Option<IssueStateReason>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct CloseIssuePayload {
    pub issue: Option<gi::Issue>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct ReopenIssueInput {
    pub issue_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteIssueInput {
    pub issue_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct DeleteIssuePayload {
    pub repository: Option<Repository>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct AddCommentInput {
    pub subject_id: ID,
    pub body: String,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct AddCommentPayload {
    pub comment_edge: Option<gi::IssueCommentEdge>,
    pub subject: Option<IssueOrPullRequest>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct UpdateIssueCommentInput {
    pub id: ID,
    pub body: String,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UpdateIssueCommentPayload {
    pub issue_comment: Option<gi::IssueComment>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteIssueCommentInput {
    pub id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct ClientMutationPayload {
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "AddLabelsToLabelableInput")]
pub struct AddLabelsToLabelableInput {
    pub labelable_id: ID,
    pub label_ids: Vec<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "RemoveLabelsFromLabelableInput")]
pub struct RemoveLabelsFromLabelableInput {
    pub labelable_id: ID,
    pub label_ids: Vec<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct LabelablePayload {
    pub labelable: Option<IssueOrPullRequest>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "AddAssigneesToAssignableInput")]
pub struct AddAssigneesToAssignableInput {
    pub assignable_id: ID,
    pub assignee_ids: Vec<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "RemoveAssigneesFromAssignableInput")]
pub struct RemoveAssigneesFromAssignableInput {
    pub assignable_id: ID,
    pub assignee_ids: Vec<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ReplaceActorsForAssignableInput")]
pub struct ReplaceActorsInput {
    pub assignable_id: ID,
    pub actor_ids: Vec<ID>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct AssignablePayload {
    pub assignable: Option<IssueOrPullRequest>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct LockLockableInput {
    pub lockable_id: ID,
    pub lock_reason: Option<LockReason>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct UnlockLockableInput {
    pub lockable_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct LockablePayload {
    pub locked_record: Option<IssueOrPullRequest>,
    pub unlocked_record: Option<IssueOrPullRequest>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct CreateLabelInput {
    pub repository_id: ID,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct LabelPayload {
    pub label: Option<gi::Label>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct UpdateLabelInput {
    pub id: ID,
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: Option<String>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteLabelInput {
    pub id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "AddReactionInput")]
pub struct AddReactionInput {
    pub subject_id: ID,
    pub content: ReactionContent,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "RemoveReactionInput")]
pub struct RemoveReactionInput {
    pub subject_id: ID,
    pub content: ReactionContent,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct ReactionPayload {
    pub reaction: Option<Reaction>,
    pub subject: Option<IssueOrPullRequest>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct Reaction {
    pub id: ID,
    pub content: ReactionContent,
    pub user: Option<User>,
    pub created_at: String,
}

#[derive(InputObject)]
#[graphql(name = "PinIssueInput")]
pub struct PinIssueInput {
    pub issue_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UnpinIssueInput")]
pub struct UnpinIssueInput {
    pub issue_id: ID,
    pub client_mutation_id: Option<String>,
}

fn ids_to_labels(ids: &[ID]) -> Vec<reef_core::Id> {
    ids.iter()
        .filter_map(|i| shapes::decode_node_id(&i.0))
        .filter(|(p, _)| p == shapes::NODE_LABEL)
        .map(|(_, n)| reef_core::Id(n))
        .collect()
}

fn ids_to_users(ids: &[ID]) -> Vec<reef_core::Id> {
    ids.iter().filter_map(|i| shapes::decode_node_id(&i.0)).filter(|(p, _)| p == shapes::NODE_USER).map(|(_, n)| reef_core::Id(n)).collect()
}

async fn label_names(ctx: &Context<'_>, ids: &[ID]) -> Result<Vec<String>> {
    let c = Ctx::from(ctx);
    let mut out = Vec::new();
    for id in ids_to_labels(ids) {
        out.push(c.db.label_by_id(id).await.map_err(|_| not_found("Could not resolve to a Label."))?.name);
    }
    Ok(out)
}

async fn user_logins(ctx: &Context<'_>, ids: &[ID]) -> Result<Vec<String>> {
    let c = Ctx::from(ctx);
    let mut out = Vec::new();
    for id in ids_to_users(ids) {
        out.push(c.db.account_by_id(id).await.map_err(|_| not_found("Could not resolve to a User."))?.login);
    }
    Ok(out)
}

#[Object]
impl IssueMutations {
    async fn create_issue(&self, ctx: &Context<'_>, input: CreateIssueInput) -> Result<CreateIssuePayload> {
        let c = Ctx::from(ctx);
        let (_, num) = shapes::decode_node_id(&input.repository_id.0).ok_or_else(|| not_found("Could not resolve to a Repository."))?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(num)).await.map_err(|_| not_found("Could not resolve to a Repository."))?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "create issues").await?;
        if rw.repo.has_issues == 0 {
            return Err(forbidden("Issues are disabled for this repository."));
        }
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        let triage = perm >= Permission::Triage;
        let milestone = match &input.milestone_id {
            Some(id) if triage => Some(gi::decode_id(id, shapes::NODE_MILESTONE)?.0),
            _ => None,
        };
        let i =
            c.db.create_issue(
                rw.repo.id(),
                &viewer,
                reef_db::issues::NewIssue {
                    title: input.title,
                    body: input.body,
                    is_pull: false,
                    labels: if triage { label_names(ctx, input.label_ids.as_deref().unwrap_or(&[])).await? } else { vec![] },
                    assignees: if triage { user_logins(ctx, input.assignee_ids.as_deref().unwrap_or(&[])).await? } else { vec![] },
                    milestone,
                },
            )
            .await
            .map_err(from_core)?;
        Ok(CreateIssuePayload { issue: Some(gi::Issue { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }

    async fn update_issue(&self, ctx: &Context<'_>, input: UpdateIssueInput) -> Result<UpdateIssuePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "update issues").await?;
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        let metadata = input.assignee_ids.is_some() || input.label_ids.is_some() || input.milestone_id.is_some();
        if perm < Permission::Triage && (metadata || viewer.id != i.author.id) {
            return Err(forbidden("You do not have permission to update this issue."));
        }
        let patch = IssuePatch {
            title: input.title,
            body: input.body.map(Some),
            state: input.state.map(|s| if s == IssueState::Open { "open".to_string() } else { "closed".to_string() }),
            state_reason: None,
            milestone: match input.milestone_id {
                Some(id) => Some(Some(gi::decode_id(&id, shapes::NODE_MILESTONE)?.0)),
                None => None,
            },
            labels: match input.label_ids {
                Some(ids) => Some(label_names(ctx, &ids).await?),
                None => None,
            },
            assignees: match input.assignee_ids {
                Some(ids) => Some(user_logins(ctx, &ids).await?),
                None => None,
            },
            close_commit: None,
            close_pull: None,
        };
        let updated = c.db.update_issue(i.issue.id(), &viewer, patch).await.map_err(from_core)?;
        Ok(UpdateIssuePayload { issue: Some(gi::Issue { repo: rw, i: updated }), client_mutation_id: input.client_mutation_id })
    }

    async fn close_issue(&self, ctx: &Context<'_>, input: CloseIssueInput) -> Result<CloseIssuePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.issue_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "close issues").await?;
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        if perm < Permission::Triage && viewer.id != i.author.id {
            return Err(forbidden("You do not have permission to close this issue."));
        }
        let reason = match input.state_reason {
            Some(IssueStateReason::NotPlanned) => "not_planned",
            Some(IssueStateReason::Duplicate) => "duplicate",
            _ => "completed",
        };
        let updated =
            c.db.update_issue(
                i.issue.id(),
                &viewer,
                IssuePatch { state: Some("closed".into()), state_reason: Some(Some(reason.into())), ..Default::default() },
            )
            .await
            .map_err(from_core)?;
        Ok(CloseIssuePayload { issue: Some(gi::Issue { repo: rw, i: updated }), client_mutation_id: input.client_mutation_id })
    }

    async fn reopen_issue(&self, ctx: &Context<'_>, input: ReopenIssueInput) -> Result<CloseIssuePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.issue_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "reopen issues").await?;
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        if perm < Permission::Triage && viewer.id != i.author.id {
            return Err(forbidden("You do not have permission to reopen this issue."));
        }
        let updated =
            c.db.update_issue(i.issue.id(), &viewer, IssuePatch { state: Some("open".into()), ..Default::default() })
                .await
                .map_err(from_core)?;
        Ok(CloseIssuePayload { issue: Some(gi::Issue { repo: rw, i: updated }), client_mutation_id: input.client_mutation_id })
    }

    async fn delete_issue(&self, ctx: &Context<'_>, input: DeleteIssueInput) -> Result<DeleteIssuePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.issue_id).await?;
        gi::require_perm(ctx, &rw, Permission::Admin, "delete issues").await?;
        c.db.delete_issue(i.issue.id()).await.map_err(from_core)?;
        Ok(DeleteIssuePayload { repository: Some(Repository(rw)), client_mutation_id: input.client_mutation_id })
    }

    async fn add_comment(&self, ctx: &Context<'_>, input: AddCommentInput) -> Result<AddCommentPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.subject_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "comment").await?;
        if i.issue.locked != 0 && c.db.permission(Some(&viewer), &rw).await.map_err(from_core)? < Permission::Triage {
            return Err(forbidden("This conversation is locked."));
        }
        let cm = c.db.create_comment(i.issue.id(), &viewer, &input.body).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(AddCommentPayload {
            comment_edge: Some(gi::IssueCommentEdge {
                cursor: cursor(0),
                node: gi::IssueComment { repo: rw.clone(), issue: i.clone(), c: cm },
            }),
            subject: Some(gi::wrap(&rw, i)),
            client_mutation_id: input.client_mutation_id,
        })
    }

    async fn update_issue_comment(&self, ctx: &Context<'_>, input: UpdateIssueCommentInput) -> Result<UpdateIssueCommentPayload> {
        let c = Ctx::from(ctx);
        let id = gi::decode_id(&input.id, shapes::NODE_COMMENT)?;
        let cm = c.db.comment_by_id(id).await.map_err(|_| not_found("Could not resolve to an IssueComment."))?;
        let i = c.db.issue_full_by_id(reef_core::Id(cm.issue_id)).await.map_err(from_core)?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(cm.repo_id)).await.map_err(from_core)?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "edit comments").await?;
        if cm.author_id != viewer.id && c.db.permission(Some(&viewer), &rw).await.map_err(from_core)? < Permission::Write {
            return Err(forbidden("You cannot edit this comment."));
        }
        let cm = c.db.update_comment(id, &input.body).await.map_err(from_core)?;
        Ok(UpdateIssueCommentPayload {
            issue_comment: Some(gi::IssueComment { repo: rw, issue: i, c: cm }),
            client_mutation_id: input.client_mutation_id,
        })
    }

    async fn delete_issue_comment(&self, ctx: &Context<'_>, input: DeleteIssueCommentInput) -> Result<ClientMutationPayload> {
        let c = Ctx::from(ctx);
        let id = gi::decode_id(&input.id, shapes::NODE_COMMENT)?;
        let cm = c.db.comment_by_id(id).await.map_err(|_| not_found("Could not resolve to an IssueComment."))?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(cm.repo_id)).await.map_err(from_core)?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "delete comments").await?;
        if cm.author_id != viewer.id && c.db.permission(Some(&viewer), &rw).await.map_err(from_core)? < Permission::Write {
            return Err(forbidden("You cannot delete this comment."));
        }
        c.db.delete_comment(id).await.map_err(from_core)?;
        Ok(ClientMutationPayload { client_mutation_id: input.client_mutation_id })
    }

    async fn add_labels_to_labelable(&self, ctx: &Context<'_>, input: AddLabelsToLabelableInput) -> Result<LabelablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.labelable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "add labels").await?;
        let names = label_names(ctx, &input.label_ids).await?;
        c.db.add_issue_labels(i.issue.id(), rw.repo.id(), &viewer, &names).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(LabelablePayload { labelable: Some(gi::wrap(&rw, i)), client_mutation_id: input.client_mutation_id })
    }

    async fn remove_labels_from_labelable(&self, ctx: &Context<'_>, input: RemoveLabelsFromLabelableInput) -> Result<LabelablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.labelable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "remove labels").await?;
        for n in label_names(ctx, &input.label_ids).await? {
            c.db.remove_issue_label(i.issue.id(), rw.repo.id(), &viewer, &n).await.map_err(from_core)?;
        }
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(LabelablePayload { labelable: Some(gi::wrap(&rw, i)), client_mutation_id: input.client_mutation_id })
    }

    async fn add_assignees_to_assignable(&self, ctx: &Context<'_>, input: AddAssigneesToAssignableInput) -> Result<AssignablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.assignable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "assign").await?;
        let logins = user_logins(ctx, &input.assignee_ids).await?;
        c.db.add_issue_assignees(i.issue.id(), rw.repo.id(), &viewer, &logins).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(AssignablePayload { assignable: Some(gi::wrap(&rw, i)), client_mutation_id: input.client_mutation_id })
    }

    async fn remove_assignees_from_assignable(
        &self,
        ctx: &Context<'_>,
        input: RemoveAssigneesFromAssignableInput,
    ) -> Result<AssignablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.assignable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "unassign").await?;
        let logins = user_logins(ctx, &input.assignee_ids).await?;
        c.db.remove_issue_assignees(i.issue.id(), rw.repo.id(), &viewer, &logins).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(AssignablePayload { assignable: Some(gi::wrap(&rw, i)), client_mutation_id: input.client_mutation_id })
    }

    async fn replace_actors_for_assignable(&self, ctx: &Context<'_>, input: ReplaceActorsInput) -> Result<AssignablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.assignable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "assign").await?;
        let logins = user_logins(ctx, &input.actor_ids).await?;
        c.db.set_issue_assignees(i.issue.id(), rw.repo.id(), &viewer, &logins).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(AssignablePayload { assignable: Some(gi::wrap(&rw, i)), client_mutation_id: input.client_mutation_id })
    }

    async fn lock_lockable(&self, ctx: &Context<'_>, input: LockLockableInput) -> Result<LockablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.lockable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "lock").await?;
        let reason = input.lock_reason.map(|r| match r {
            LockReason::OffTopic => "off-topic",
            LockReason::TooHeated => "too heated",
            LockReason::Resolved => "resolved",
            LockReason::Spam => "spam",
        });
        c.db.set_issue_lock(i.issue.id(), &viewer, true, reason).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(LockablePayload { locked_record: Some(gi::wrap(&rw, i)), unlocked_record: None, client_mutation_id: input.client_mutation_id })
    }

    async fn unlock_lockable(&self, ctx: &Context<'_>, input: UnlockLockableInput) -> Result<LockablePayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = gi::load_issue(ctx, &input.lockable_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Triage, "unlock").await?;
        c.db.set_issue_lock(i.issue.id(), &viewer, false, None).await.map_err(from_core)?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(LockablePayload { locked_record: None, unlocked_record: Some(gi::wrap(&rw, i)), client_mutation_id: input.client_mutation_id })
    }

    async fn create_label(&self, ctx: &Context<'_>, input: CreateLabelInput) -> Result<LabelPayload> {
        let c = Ctx::from(ctx);
        let (_, num) = shapes::decode_node_id(&input.repository_id.0).ok_or_else(|| not_found("Could not resolve to a Repository."))?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(num)).await.map_err(|_| not_found("Could not resolve to a Repository."))?;
        gi::require_perm(ctx, &rw, Permission::Triage, "create labels").await?;
        let l = c.db.create_label(rw.repo.id(), &input.name, &input.color, input.description.as_deref(), false).await.map_err(from_core)?;
        Ok(LabelPayload { label: Some(gi::Label { repo: rw, l }), client_mutation_id: input.client_mutation_id })
    }

    async fn update_label(&self, ctx: &Context<'_>, input: UpdateLabelInput) -> Result<LabelPayload> {
        let c = Ctx::from(ctx);
        let id = gi::decode_id(&input.id, shapes::NODE_LABEL)?;
        let l = c.db.label_by_id(id).await.map_err(|_| not_found("Could not resolve to a Label."))?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(l.repo_id)).await.map_err(from_core)?;
        gi::require_perm(ctx, &rw, Permission::Triage, "update labels").await?;
        let l =
            c.db.update_label(id, input.name.as_deref(), input.color.as_deref(), input.description.as_ref().map(|d| Some(d.as_str())))
                .await
                .map_err(from_core)?;
        Ok(LabelPayload { label: Some(gi::Label { repo: rw, l }), client_mutation_id: input.client_mutation_id })
    }

    async fn delete_label(&self, ctx: &Context<'_>, input: DeleteLabelInput) -> Result<ClientMutationPayload> {
        let c = Ctx::from(ctx);
        let id = gi::decode_id(&input.id, shapes::NODE_LABEL)?;
        let l = c.db.label_by_id(id).await.map_err(|_| not_found("Could not resolve to a Label."))?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(l.repo_id)).await.map_err(from_core)?;
        gi::require_perm(ctx, &rw, Permission::Triage, "delete labels").await?;
        c.db.delete_label(id).await.map_err(from_core)?;
        Ok(ClientMutationPayload { client_mutation_id: input.client_mutation_id })
    }

    async fn add_reaction(&self, ctx: &Context<'_>, input: AddReactionInput) -> Result<ReactionPayload> {
        let c = Ctx::from(ctx);
        let Some(viewer) = c.viewer().cloned() else {
            return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated."));
        };
        let (prefix, num) = shapes::decode_node_id(&input.subject_id.0).ok_or_else(|| not_found("Could not resolve to a node."))?;
        let (subject_type, subject) = match prefix.as_str() {
            shapes::NODE_ISSUE | shapes::NODE_PULL => {
                let (rw, i) = gi::load_issue(ctx, &input.subject_id).await?;
                ("issue", Some(gi::wrap(&rw, i)))
            }
            shapes::NODE_COMMENT => ("issue_comment", None),
            _ => return Err(not_found("Could not resolve to a Reactable.")),
        };
        let r = c.db.add_reaction(subject_type, reef_core::Id(num), viewer.id(), input.content.key()).await.map_err(from_core)?;
        Ok(ReactionPayload {
            reaction: Some(Reaction {
                id: ID(shapes::node_id("RE", r.id)),
                content: input.content,
                user: Some(User(viewer)),
                created_at: reef_core::time::iso(r.created_at),
            }),
            subject,
            client_mutation_id: input.client_mutation_id,
        })
    }

    async fn remove_reaction(&self, ctx: &Context<'_>, input: RemoveReactionInput) -> Result<ReactionPayload> {
        let c = Ctx::from(ctx);
        let Some(viewer) = c.viewer().cloned() else {
            return Err(gql_error("UNAUTHORIZED", "This endpoint requires you to be authenticated."));
        };
        let (prefix, num) = shapes::decode_node_id(&input.subject_id.0).ok_or_else(|| not_found("Could not resolve to a node."))?;
        let subject_type = match prefix.as_str() {
            shapes::NODE_ISSUE | shapes::NODE_PULL => "issue",
            shapes::NODE_COMMENT => "issue_comment",
            _ => return Err(not_found("Could not resolve to a Reactable.")),
        };
        for r in c.db.reactions(subject_type, reef_core::Id(num)).await.map_err(from_core)? {
            if r.user_id == viewer.id && r.content == input.content.key() {
                c.db.delete_reaction(reef_core::Id(r.id), viewer.id()).await.map_err(from_core)?;
            }
        }
        Ok(ReactionPayload { reaction: None, subject: None, client_mutation_id: input.client_mutation_id })
    }

    async fn pin_issue(&self, ctx: &Context<'_>, input: PinIssueInput) -> Result<CloseIssuePayload> {
        let (rw, i) = gi::load_issue(ctx, &input.issue_id).await?;
        Ok(CloseIssuePayload { issue: Some(gi::Issue { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }

    async fn unpin_issue(&self, ctx: &Context<'_>, input: UnpinIssueInput) -> Result<CloseIssuePayload> {
        let (rw, i) = gi::load_issue(ctx, &input.issue_id).await?;
        Ok(CloseIssuePayload { issue: Some(gi::Issue { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }
}

#[derive(Default)]
pub struct IssueMutations;

/// The root mutation: every mutation group merged into one type.
#[derive(async_graphql::MergedObject, Default)]
pub struct Mutation(RepoMutations, IssueMutations, PullMutations, super::projects::ProjectMutations);

// ----- pull requests -----

use super::issues::PullRequest as GqlPull;
use crate::pulls as prsvc;

#[derive(InputObject)]
#[graphql(name = "CreatePullRequestInput")]
pub struct CreatePullRequestInput {
    pub repository_id: ID,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub head_repository_id: Option<ID>,
    pub title: String,
    pub body: Option<String>,
    pub maintainer_can_modify: Option<bool>,
    pub draft: Option<bool>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct PullRequestPayload {
    pub pull_request: Option<GqlPull>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "UpdatePullRequestInput")]
pub struct UpdatePullRequestInput {
    pub pull_request_id: ID,
    pub base_ref_name: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<PullRequestUpdateState>,
    pub maintainer_can_modify: Option<bool>,
    pub assignee_ids: Option<Vec<ID>>,
    pub milestone_id: Option<ID>,
    pub label_ids: Option<Vec<ID>>,
    pub project_ids: Option<Vec<ID>>,
    pub client_mutation_id: Option<String>,
}

#[derive(async_graphql::Enum, Copy, Clone, Eq, PartialEq)]
pub enum PullRequestUpdateState {
    Open,
    Closed,
}

#[derive(InputObject)]
#[graphql(name = "ClosePullRequestInput")]
pub struct ClosePullRequestInput {
    pub pull_request_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ReopenPullRequestInput")]
pub struct ReopenPullRequestInput {
    pub pull_request_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "MarkPullRequestReadyForReviewInput")]
pub struct MarkPullRequestReadyForReviewInput {
    pub pull_request_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "ConvertPullRequestToDraftInput")]
pub struct ConvertPullRequestToDraftInput {
    pub pull_request_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "MergePullRequestInput")]
pub struct MergePullRequestInput {
    pub pull_request_id: ID,
    pub commit_headline: Option<String>,
    pub commit_body: Option<String>,
    pub expected_head_oid: Option<String>,
    pub merge_method: Option<PullRequestMergeMethod>,
    pub author_email: Option<String>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "AddPullRequestReviewInput")]
pub struct AddPullRequestReviewInput {
    pub pull_request_id: ID,
    pub commit_oid: Option<String>,
    pub body: Option<String>,
    pub event: Option<PullRequestReviewEvent>,
    pub comments: Option<Vec<DraftPullRequestReviewComment>>,
    pub threads: Option<Vec<DraftPullRequestReviewThread>>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DraftPullRequestReviewComment")]
pub struct DraftPullRequestReviewComment {
    pub path: String,
    pub position: i32,
    pub body: String,
}

#[derive(InputObject)]
#[graphql(name = "DraftPullRequestReviewThread")]
pub struct DraftPullRequestReviewThread {
    pub path: String,
    pub line: i32,
    pub side: Option<String>,
    pub start_line: Option<i32>,
    pub start_side: Option<String>,
    pub body: String,
}

#[derive(async_graphql::Enum, Copy, Clone, Eq, PartialEq)]
pub enum PullRequestReviewEvent {
    Comment,
    Approve,
    RequestChanges,
    Dismiss,
}

#[derive(SimpleObject)]
pub struct AddPullRequestReviewPayload {
    pub pull_request_review: Option<super::issues::PullRequestReview>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "SubmitPullRequestReviewInput")]
pub struct SubmitPullRequestReviewInput {
    pub pull_request_id: Option<ID>,
    pub pull_request_review_id: Option<ID>,
    pub event: PullRequestReviewEvent,
    pub body: Option<String>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "RequestReviewsInput")]
pub struct RequestReviewsInput {
    pub pull_request_id: ID,
    pub user_ids: Option<Vec<ID>>,
    pub team_ids: Option<Vec<ID>>,
    pub union: Option<bool>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct RequestReviewsPayload {
    pub pull_request: Option<GqlPull>,
    pub requested_reviewers_edge: Option<UserEdge>,
    pub client_mutation_id: Option<String>,
}

#[derive(SimpleObject)]
pub struct UserEdge {
    pub cursor: String,
    pub node: User,
}

#[derive(InputObject)]
#[graphql(name = "RequestReviewsByLoginInput")]
pub struct RequestReviewsByLoginInput {
    pub pull_request_id: ID,
    pub user_logins: Option<Vec<String>>,
    pub team_slugs: Option<Vec<String>>,
    pub union: Option<bool>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "EnablePullRequestAutoMergeInput")]
pub struct EnablePullRequestAutoMergeInput {
    pub pull_request_id: ID,
    pub merge_method: Option<PullRequestMergeMethod>,
    pub commit_headline: Option<String>,
    pub commit_body: Option<String>,
    pub author_email: Option<String>,
    pub client_mutation_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "DisablePullRequestAutoMergeInput")]
pub struct DisablePullRequestAutoMergeInput {
    pub pull_request_id: ID,
    pub client_mutation_id: Option<String>,
}

#[derive(Default)]
pub struct PullMutations;

async fn load_pull(ctx: &Context<'_>, id: &ID) -> Result<(reef_db::repos::RepoWithOwner, reef_db::issues::IssueFull)> {
    let (rw, i) = gi::load_issue(ctx, id).await?;
    if !i.issue.is_pull() {
        return Err(not_found("Could not resolve to a PullRequest with the given id."));
    }
    Ok((rw, i))
}

#[Object]
impl PullMutations {
    async fn create_pull_request(&self, ctx: &Context<'_>, input: CreatePullRequestInput) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (_, num) = shapes::decode_node_id(&input.repository_id.0).ok_or_else(|| not_found("Could not resolve to a Repository."))?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(num)).await.map_err(|_| not_found("Could not resolve to a Repository."))?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "create pull requests").await?;
        let s = super::app(ctx);
        let head = input.head_ref_name.split_once(':').map(|(_, r)| r.to_string()).unwrap_or(input.head_ref_name);
        let (i, _) = prsvc::create(
            &s,
            &rw,
            &viewer,
            prsvc::CreatePull {
                title: input.title,
                body: input.body,
                head,
                base: input.base_ref_name,
                draft: input.draft.unwrap_or(false),
                maintainer_can_modify: input.maintainer_can_modify.unwrap_or(false),
            },
        )
        .await
        .map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }

    async fn update_pull_request(&self, ctx: &Context<'_>, input: UpdatePullRequestInput) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "update pull requests").await?;
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        let metadata = input.assignee_ids.is_some() || input.label_ids.is_some() || input.milestone_id.is_some();
        if perm < Permission::Triage && (metadata || viewer.id != i.author.id) {
            return Err(forbidden("You do not have permission to update this pull request."));
        }
        if let Some(base) = &input.base_ref_name {
            let path = c.store.path(rw.repo.id());
            let sha = reef_git::ops::rev_parse(c.store.git_binary(), &path, &format!("refs/heads/{base}"))
                .await
                .map_err(|_| gql_error("UNPROCESSABLE", "base branch not found"))?;
            c.db.set_pull_base(i.issue.id(), base, &sha).await.map_err(from_core)?;
        }
        let patch = IssuePatch {
            title: input.title,
            body: input.body.map(Some),
            state: input.state.map(|s| if s == PullRequestUpdateState::Open { "open".to_string() } else { "closed".to_string() }),
            state_reason: None,
            milestone: match input.milestone_id {
                Some(id) => Some(Some(gi::decode_id(&id, shapes::NODE_MILESTONE)?.0)),
                None => None,
            },
            labels: match input.label_ids {
                Some(ids) => Some(label_names(ctx, &ids).await?),
                None => None,
            },
            assignees: match input.assignee_ids {
                Some(ids) => Some(user_logins(ctx, &ids).await?),
                None => None,
            },
            close_commit: None,
            close_pull: None,
        };
        let updated = c.db.update_issue(i.issue.id(), &viewer, patch).await.map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i: updated }), client_mutation_id: input.client_mutation_id })
    }

    async fn close_pull_request(&self, ctx: &Context<'_>, input: ClosePullRequestInput) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "close pull requests").await?;
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        if perm < Permission::Triage && viewer.id != i.author.id {
            return Err(forbidden("You do not have permission to close this pull request."));
        }
        let updated =
            c.db.update_issue(i.issue.id(), &viewer, IssuePatch { state: Some("closed".into()), ..Default::default() })
                .await
                .map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i: updated }), client_mutation_id: input.client_mutation_id })
    }

    async fn reopen_pull_request(&self, ctx: &Context<'_>, input: ReopenPullRequestInput) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "reopen pull requests").await?;
        let m = c.db.pull_meta(i.issue.id()).await.map_err(from_core)?;
        if m.merged != 0 {
            return Err(gql_error("UNPROCESSABLE", "Merged pull requests cannot be reopened."));
        }
        let perm = c.db.permission(Some(&viewer), &rw).await.map_err(from_core)?;
        if perm < Permission::Triage && viewer.id != i.author.id {
            return Err(forbidden("You do not have permission to reopen this pull request."));
        }
        let updated =
            c.db.update_issue(i.issue.id(), &viewer, IssuePatch { state: Some("open".into()), ..Default::default() })
                .await
                .map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i: updated }), client_mutation_id: input.client_mutation_id })
    }

    async fn mark_pull_request_ready_for_review(
        &self,
        ctx: &Context<'_>,
        input: MarkPullRequestReadyForReviewInput,
    ) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        gi::require_perm(ctx, &rw, Permission::Read, "update pull requests").await?;
        c.db.set_pull_draft(i.issue.id(), false).await.map_err(from_core)?;
        c.db.add_issue_event(i.issue.id(), rw.repo.id(), c.viewer().map(|v| v.id()), "ready_for_review", &serde_json::json!({}))
            .await
            .map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }

    async fn convert_pull_request_to_draft(&self, ctx: &Context<'_>, input: ConvertPullRequestToDraftInput) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        gi::require_perm(ctx, &rw, Permission::Read, "update pull requests").await?;
        c.db.set_pull_draft(i.issue.id(), true).await.map_err(from_core)?;
        c.db.add_issue_event(i.issue.id(), rw.repo.id(), c.viewer().map(|v| v.id()), "convert_to_draft", &serde_json::json!({}))
            .await
            .map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }

    async fn merge_pull_request(&self, ctx: &Context<'_>, input: MergePullRequestInput) -> Result<PullRequestPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Write, "merge").await?;
        let s = super::app(ctx);
        let method = match input.merge_method.unwrap_or(PullRequestMergeMethod::Merge) {
            PullRequestMergeMethod::Merge => prsvc::MergeMethod::Merge,
            PullRequestMergeMethod::Squash => prsvc::MergeMethod::Squash,
            PullRequestMergeMethod::Rebase => prsvc::MergeMethod::Rebase,
        };
        prsvc::merge(
            &s,
            &rw,
            &viewer,
            &i,
            prsvc::MergeOptions {
                method,
                commit_title: input.commit_headline,
                commit_message: input.commit_body,
                expected_head: input.expected_head_oid,
            },
        )
        .await
        .map_err(|e| match e {
            reef_core::Error::Conflict(m) => gql_error("UNPROCESSABLE", m),
            o => from_core(o),
        })?;
        let i = c.db.issue_full_by_id(i.issue.id()).await.map_err(from_core)?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }

    async fn add_pull_request_review(&self, ctx: &Context<'_>, input: AddPullRequestReviewInput) -> Result<AddPullRequestReviewPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "review").await?;
        let state = match input.event {
            Some(PullRequestReviewEvent::Approve) => "approved",
            Some(PullRequestReviewEvent::RequestChanges) => "changes_requested",
            Some(PullRequestReviewEvent::Comment) => "commented",
            _ => "pending",
        };
        if state == "approved" && viewer.id == i.author.id {
            return Err(gql_error("UNPROCESSABLE", "Can not approve your own pull request"));
        }
        if state == "changes_requested" && input.body.as_deref().map(str::trim).unwrap_or("").is_empty() {
            return Err(gql_error("UNPROCESSABLE", "A body is required when requesting changes"));
        }
        let m = c.db.pull_meta(i.issue.id()).await.map_err(from_core)?;
        let sha = input.commit_oid.clone().unwrap_or(m.head_sha);
        let r =
            c.db.create_review(i.issue.id(), rw.repo.id(), &viewer, state, input.body.as_deref(), Some(&sha)).await.map_err(from_core)?;
        let _ = c.db.notify_participants(i.issue.id(), Some(viewer.id()), "review").await;
        for t in input.threads.unwrap_or_default() {
            c.db.create_review_comment(
                i.issue.id(),
                rw.repo.id(),
                Some(reef_core::Id(r.id)),
                &viewer,
                &t.body,
                &t.path,
                Some(t.line as i64),
                t.start_line.map(|l| l as i64),
                t.side.as_deref().unwrap_or("RIGHT"),
                &sha,
                None,
            )
            .await
            .map_err(from_core)?;
        }
        if state != "pending" {
            c.db.add_issue_event(
                i.issue.id(),
                rw.repo.id(),
                Some(viewer.id()),
                "reviewed",
                &serde_json::json!({"review_id": r.id, "state": state}),
            )
            .await
            .map_err(from_core)?;
        }
        let review =
            super::pulls::reviews(ctx, &rw, &i, Some(200), None, None, false).await?.nodes.into_iter().find(|n| n.database_id == r.id);
        Ok(AddPullRequestReviewPayload { pull_request_review: review, client_mutation_id: input.client_mutation_id })
    }

    async fn submit_pull_request_review(
        &self,
        ctx: &Context<'_>,
        input: SubmitPullRequestReviewInput,
    ) -> Result<AddPullRequestReviewPayload> {
        let c = Ctx::from(ctx);
        let id = input
            .pull_request_review_id
            .as_ref()
            .and_then(|r| shapes::decode_node_id(&r.0))
            .map(|(_, n)| reef_core::Id(n))
            .ok_or_else(|| not_found("Could not resolve to a PullRequestReview."))?;
        let r = c.db.review_by_id(id).await.map_err(|_| not_found("Could not resolve to a PullRequestReview."))?;
        let i = c.db.issue_full_by_id(reef_core::Id(r.pull_id)).await.map_err(from_core)?;
        let rw = c.db.repo_with_owner_by_id(reef_core::Id(r.repo_id)).await.map_err(from_core)?;
        let state = match input.event {
            PullRequestReviewEvent::Approve => "approved",
            PullRequestReviewEvent::RequestChanges => "changes_requested",
            _ => "commented",
        };
        let r = c.db.submit_review(id, state, input.body.as_deref()).await.map_err(from_core)?;
        let review =
            super::pulls::reviews(ctx, &rw, &i, Some(200), None, None, false).await?.nodes.into_iter().find(|n| n.database_id == r.id);
        Ok(AddPullRequestReviewPayload { pull_request_review: review, client_mutation_id: input.client_mutation_id })
    }

    async fn request_reviews(&self, ctx: &Context<'_>, input: RequestReviewsInput) -> Result<RequestReviewsPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "request reviews").await?;
        let logins = user_logins(ctx, input.user_ids.as_deref().unwrap_or(&[])).await?;
        if input.union != Some(true) {
            let existing: Vec<String> = c.db.review_requests(i.issue.id()).await.map_err(from_core)?.into_iter().map(|a| a.login).collect();
            c.db.remove_review_requests(i.issue.id(), &existing).await.map_err(from_core)?;
        }
        c.db.request_reviewers(i.issue.id(), &logins).await.map_err(from_core)?;
        for login in &logins {
            if let Ok(a) = c.db.account_by_login(login).await {
                let _ = c.db.notify_user_about_issue(a.id(), i.issue.id(), Some(viewer.id()), "review_requested").await;
            }
        }
        for l in &logins {
            c.db.add_issue_event(
                i.issue.id(),
                rw.repo.id(),
                Some(viewer.id()),
                "review_requested",
                &serde_json::json!({"requested_reviewer": l}),
            )
            .await
            .map_err(from_core)?;
        }
        Ok(RequestReviewsPayload {
            pull_request: Some(GqlPull { repo: rw, i }),
            requested_reviewers_edge: None,
            client_mutation_id: input.client_mutation_id,
        })
    }

    async fn request_reviews_by_login(&self, ctx: &Context<'_>, input: RequestReviewsByLoginInput) -> Result<RequestReviewsPayload> {
        let c = Ctx::from(ctx);
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        let viewer = gi::require_perm(ctx, &rw, Permission::Read, "request reviews").await?;
        let logins = input.user_logins.unwrap_or_default();
        if input.union != Some(true) {
            let existing: Vec<String> = c.db.review_requests(i.issue.id()).await.map_err(from_core)?.into_iter().map(|a| a.login).collect();
            c.db.remove_review_requests(i.issue.id(), &existing).await.map_err(from_core)?;
        }
        c.db.request_reviewers(i.issue.id(), &logins).await.map_err(from_core)?;
        for l in &logins {
            c.db.add_issue_event(
                i.issue.id(),
                rw.repo.id(),
                Some(viewer.id()),
                "review_requested",
                &serde_json::json!({"requested_reviewer": l}),
            )
            .await
            .map_err(from_core)?;
        }
        Ok(RequestReviewsPayload {
            pull_request: Some(GqlPull { repo: rw, i }),
            requested_reviewers_edge: None,
            client_mutation_id: input.client_mutation_id,
        })
    }

    async fn enable_pull_request_auto_merge(
        &self,
        _ctx: &Context<'_>,
        _input: EnablePullRequestAutoMergeInput,
    ) -> Result<PullRequestPayload> {
        Err(gql_error("UNPROCESSABLE", "Auto-merge is not available on this instance yet."))
    }

    async fn disable_pull_request_auto_merge(
        &self,
        ctx: &Context<'_>,
        input: DisablePullRequestAutoMergeInput,
    ) -> Result<PullRequestPayload> {
        let (rw, i) = load_pull(ctx, &input.pull_request_id).await?;
        Ok(PullRequestPayload { pull_request: Some(GqlPull { repo: rw, i }), client_mutation_id: input.client_mutation_id })
    }
}
