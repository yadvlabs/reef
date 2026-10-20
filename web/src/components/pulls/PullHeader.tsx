// Title row, state row ("user wants to merge N commits into base from head") and the
// Conversation / Commits / Checks / Files changed tabs.
import { ActionList, ActionMenu, Button, TextInput, UnderlineNav } from '@primer/react';
import { CheckCircleIcon, CodeIcon, CommentDiscussionIcon, CopyIcon, FileDiffIcon, GitCommitIcon, PencilIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useRouterState } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo } from '../../lib/api';
import { checkRunsQuery, combinedStatusQuery, type Pull } from '../../lib/pulls';
import { useSession } from '../../lib/session';
import { StateBadge } from '../IssueBits';
import { UnderlineNavLink } from '../links';

export function invalidatePull(qc: ReturnType<typeof useQueryClient>, repo: Repo, number: number) {
  qc.invalidateQueries({ queryKey: ['pulls', repo.full_name.toLowerCase(), number] });
  qc.invalidateQueries({ queryKey: ['issues', repo.full_name.toLowerCase(), number] });
  qc.invalidateQueries({ queryKey: ['issues', repo.full_name.toLowerCase(), 'list'] });
}

function RefChip({ repo, name }: { repo: Repo; name: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <span className="ref-chip">
      <Link to="/$owner/$repo/tree/$rev/$" params={{ owner: repo.owner.login, repo: repo.name, rev: name, _splat: '' }}>
        {name}
      </Link>
      <button
        className="linkish"
        aria-label={`Copy ${name}`}
        title={copied ? 'Copied' : 'Copy'}
        onClick={() => {
          navigator.clipboard?.writeText(name);
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        }}
      >
        <CopyIcon size={12} />
      </button>
    </span>
  );
}

export function PullHeader({ repo, pr }: { repo: Repo; pr: Pull }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const base = `/${repo.owner.login}/${repo.name}/pull/${pr.number}`;
  const section = path.slice(base.length).replace(/^\//, '').split('/')[0] ?? '';
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(pr.title);
  const canEdit = !!repo.permissions?.triage || user?.id === pr.user.id;
  const rename = useMutation({
    mutationFn: () => api(`/repos/${repo.full_name}/pulls/${pr.number}`, { method: 'PATCH', json: { title } }),
    onSuccess: () => {
      setEditing(false);
      invalidatePull(qc, repo, pr.number);
    },
  });
  const checks = useQuery(checkRunsQuery(repo.full_name, pr.head.sha));
  const statuses = useQuery(combinedStatusQuery(repo.full_name, pr.head.sha));
  const checksCount = (checks.data?.total_count ?? 0) + (statuses.data?.total_count ?? 0);
  const p = { owner: repo.owner.login, repo: repo.name, number: String(pr.number) };
  const cloneCmd = `git fetch origin pull/${pr.number}/head:${pr.head.ref}\ngit checkout ${pr.head.ref}`;
  return (
    <div className="pr-head">
      {editing ? (
        <form
          className="pr-title-edit"
          onSubmit={(e) => {
            e.preventDefault();
            rename.mutate();
          }}
        >
          <TextInput value={title} onChange={(e) => setTitle(e.target.value)} block autoFocus aria-label="Title" />
          <Button type="submit" variant="primary" loading={rename.isPending}>
            Save
          </Button>
          <Button onClick={() => setEditing(false)}>Cancel</Button>
        </form>
      ) : (
        <div className="pr-title-row">
          <h1>
            {pr.title} <span className="number">#{pr.number}</span>
          </h1>
          <div className="pr-title-actions">
            {canEdit ? (
              <Button size="small" leadingVisual={PencilIcon} onClick={() => setEditing(true)}>
                Edit
              </Button>
            ) : null}
            <ActionMenu>
              <ActionMenu.Button size="small" leadingVisual={CodeIcon}>
                Code
              </ActionMenu.Button>
              <ActionMenu.Overlay align="end" width="large">
                <div className="pr-code-menu">
                  <strong>Checkout with GitHub CLI</strong>
                  <pre>gh pr checkout {pr.number}</pre>
                  <strong>Checkout with Git</strong>
                  <pre>{cloneCmd}</pre>
                  <ActionList>
                    <ActionList.Item onSelect={() => navigator.clipboard?.writeText(`gh pr checkout ${pr.number}`)}>Copy CLI command</ActionList.Item>
                    <ActionList.Item onSelect={() => navigator.clipboard?.writeText(cloneCmd)}>Copy Git commands</ActionList.Item>
                  </ActionList>
                </div>
              </ActionMenu.Overlay>
            </ActionMenu>
          </div>
        </div>
      )}
      <div className="pr-state-row">
        <StateBadge issue={pr} />
        <span className="muted">
          <Link to="/$owner" params={{ owner: pr.user.login }} className="author">
            {pr.user.login}
          </Link>{' '}
          {pr.merged ? (
            <>
              merged {pr.commits} {pr.commits === 1 ? 'commit' : 'commits'} into <RefChip repo={repo} name={pr.base.ref} /> from <RefChip repo={repo} name={pr.head.ref} />
            </>
          ) : (
            <>
              wants to merge {pr.commits} {pr.commits === 1 ? 'commit' : 'commits'} into <RefChip repo={repo} name={pr.base.ref} /> from <RefChip repo={repo} name={pr.head.ref} />
            </>
          )}
        </span>
      </div>
      <UnderlineNav aria-label="Pull request" className="pr-tabs">
        <UnderlineNavLink to="/$owner/$repo/pull/$number" params={p} search={{}} icon={CommentDiscussionIcon} counter={pr.comments + pr.review_comments} aria-current={section === '' ? 'page' : undefined}>
          Conversation
        </UnderlineNavLink>
        <UnderlineNavLink to="/$owner/$repo/pull/$number/commits" params={p} icon={GitCommitIcon} counter={pr.commits} aria-current={section === 'commits' ? 'page' : undefined}>
          Commits
        </UnderlineNavLink>
        <UnderlineNavLink to="/$owner/$repo/pull/$number/checks" params={p} icon={CheckCircleIcon} counter={checksCount} aria-current={section === 'checks' ? 'page' : undefined}>
          Checks
        </UnderlineNavLink>
        <UnderlineNavLink to="/$owner/$repo/pull/$number/files" params={p} icon={FileDiffIcon} counter={pr.changed_files} aria-current={section === 'files' ? 'page' : undefined}>
          Files changed
        </UnderlineNavLink>
      </UnderlineNav>
    </div>
  );
}
