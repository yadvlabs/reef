import { ActionList, ActionMenu, Avatar, AvatarStack, Button, RelativeTime, TextInput, Textarea } from '@primer/react';
import { CheckIcon, CommentIcon, DotFillIcon, IssueClosedIcon, IssueOpenedIcon, GitPullRequestIcon, GitMergeIcon, GitPullRequestClosedIcon, GitPullRequestDraftIcon, MilestoneIcon, SkipIcon, XIcon, CheckCircleFillIcon, XCircleFillIcon, GearIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo, type User } from '../lib/api';
import { assigneesQuery, labelTextColor, labelsQuery, milestonesQuery, type Issue, type Label, type Milestone } from '../lib/issues';
import { pullReviewsQuery, type Pull } from '../lib/pulls';
import { Markdown } from './Markdown';
import { IssueProjects } from './IssueProjects';

export function LabelBadge({ label, small }: { label: Label; small?: boolean }) {
  return (
    <span
      className="issue-label"
      style={{ background: `#${label.color}`, color: labelTextColor(label.color), fontSize: small ? 12 : 12, padding: '0 7px', lineHeight: '20px', borderRadius: 999, fontWeight: 500, display: 'inline-block', whiteSpace: 'nowrap' }}
      title={label.description ?? undefined}
    >
      {label.name}
    </span>
  );
}

export function StateIcon({ issue, size = 16 }: { issue: Issue; size?: number }) {
  if (issue.pull_request) {
    if (issue.pull_request.merged_at) return <GitMergeIcon size={size} fill="var(--fgColor-done)" />;
    if (issue.state === 'closed') return <GitPullRequestClosedIcon size={size} fill="var(--fgColor-danger)" />;
    return issue.draft ? <GitPullRequestDraftIcon size={size} fill="var(--fgColor-muted)" /> : <GitPullRequestIcon size={size} fill="var(--fgColor-success)" />;
  }
  if (issue.state === 'closed') {
    return issue.state_reason === 'not_planned' ? <SkipIcon size={size} fill="var(--fgColor-muted)" /> : <IssueClosedIcon size={size} fill="var(--fgColor-done)" />;
  }
  return <IssueOpenedIcon size={size} fill="var(--fgColor-success)" />;
}

export function StateBadge({ issue }: { issue: Issue }) {
  const merged = !!issue.pull_request?.merged_at;
  const bg = merged ? 'var(--bgColor-done-emphasis)' : issue.state === 'open' ? 'var(--bgColor-success-emphasis)' : issue.state_reason === 'not_planned' ? 'var(--bgColor-neutral-emphasis)' : issue.pull_request ? 'var(--bgColor-danger-emphasis)' : 'var(--bgColor-done-emphasis)';
  const text = merged ? 'Merged' : issue.state === 'open' ? (issue.pull_request ? (issue.draft ? 'Draft' : 'Open') : 'Open') : 'Closed';
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, background: bg, color: '#fff', borderRadius: 999, padding: '5px 12px', fontWeight: 500, fontSize: 14 }}>
      <StateIcon issue={{ ...issue, state: issue.state }} />
      {text}
    </span>
  );
}

/** Write / Preview markdown editor. Preview is rendered by the server. */
export function MarkdownEditor({ value, onChange, repo, placeholder, autoFocus, minHeight = 120 }: { value: string; onChange: (v: string) => void; repo?: Repo; placeholder?: string; autoFocus?: boolean; minHeight?: number }) {
  const [tab, setTab] = useState<'write' | 'preview'>('write');
  const [html, setHtml] = useState('');
  async function preview() {
    setTab('preview');
    const res = await fetch('/api/v3/markdown', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ text: value, mode: 'gfm', context: repo?.full_name }) });
    setHtml(value.trim() ? await res.text() : '<p class="muted">Nothing to preview</p>');
  }
  return (
    <div className="md-editor">
      <div className="md-tabs">
        <button type="button" className={tab === 'write' ? 'active' : ''} onClick={() => setTab('write')}>
          Write
        </button>
        <button type="button" className={tab === 'preview' ? 'active' : ''} onClick={preview}>
          Preview
        </button>
      </div>
      {tab === 'write' ? (
        <Textarea value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder ?? 'Add your description here...'} block rows={Math.max(5, Math.min(30, value.split('\n').length + 2))} autoFocus={autoFocus} style={{ minHeight, fontFamily: 'var(--fontStack-monospace)', fontSize: 13 }} />
      ) : (
        <div className="md-preview">
          <Markdown html={html} />
        </div>
      )}
      <div className="small muted" style={{ marginTop: 4 }}>
        Markdown is supported. Reference issues with #number and people with @login.
      </div>
    </div>
  );
}

/** Sidebar picker: assignees, labels, milestone. `disabled` renders read-only. */
export function IssueSidebar({ repo, issue, disabled, pull }: { repo: Repo; issue: Issue; disabled: boolean; pull?: Pull }) {
  const qc = useQueryClient();
  const full = repo.full_name;
  const labels = useQuery(labelsQuery(full));
  const assignees = useQuery(assigneesQuery(full));
  const milestones = useQuery(milestonesQuery(full, 'open'));
  const patch = useMutation({
    mutationFn: (body: Record<string, unknown>) => api<Issue>(`/repos/${full}/issues/${issue.number}`, { method: 'PATCH', json: body, accept: 'application/vnd.github.html+json' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['issues', full.toLowerCase(), issue.number] });
      qc.invalidateQueries({ queryKey: ['issues', full.toLowerCase(), 'list'] });
    },
  });
  const [labelFilter, setLabelFilter] = useState('');
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);
  return (
    <aside className="issue-sidebar">
      {pull ? <ReviewersSection repo={repo} pr={pull} disabled={disabled} assignees={assignees.data ?? []} /> : null}
      <section>
        <ActionMenu>
          <ActionMenu.Anchor>
            <button className="sidebar-head" disabled={disabled} title={disabled ? undefined : "Edit"}>
              Assignees {!disabled ? <GearIcon size={16} /> : null}
            </button>
          </ActionMenu.Anchor>
          <ActionMenu.Overlay width="medium">
            <ActionList selectionVariant="multiple">
              {assignees.data?.map((u) => (
                <ActionList.Item key={u.id} selected={issue.assignees.some((a) => a.id === u.id)} onSelect={() => patch.mutate({ assignees: toggle(issue.assignees.map((a) => a.login), u.login) })}>
                  <ActionList.LeadingVisual>
                    <Avatar src={u.avatar_url} size={20} alt="" />
                  </ActionList.LeadingVisual>
                  {u.login}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        <div className="sidebar-body">
          {issue.assignees.length === 0 ? (
            <span className="muted">
              No one{!disabled ? <> – <button className="linkish" onClick={() => patch.mutate({ assignees: [issue.user.login] })}>assign yourself</button></> : null}
            </span>
          ) : (
            issue.assignees.map((a) => (
              <Link key={a.id} to="/$owner" params={{ owner: a.login }} style={{ display: 'flex', alignItems: 'center', gap: 6, color: 'var(--fgColor-default)', fontWeight: 600 }}>
                <Avatar src={a.avatar_url} size={20} alt="" /> {a.login}
              </Link>
            ))
          )}
        </div>
      </section>
      <section>
        <ActionMenu onOpenChange={() => setLabelFilter('')}>
          <ActionMenu.Anchor>
            <button className="sidebar-head" disabled={disabled} title={disabled ? undefined : "Edit"}>
              Labels {!disabled ? <GearIcon size={16} /> : null}
            </button>
          </ActionMenu.Anchor>
          <ActionMenu.Overlay width="medium">
            <div style={{ padding: 8 }}>
              <TextInput block size="small" placeholder="Filter labels" value={labelFilter} onChange={(e) => setLabelFilter(e.target.value)} />
            </div>
            <ActionList selectionVariant="multiple">
              {labels.data
                ?.filter((l) => l.name.toLowerCase().includes(labelFilter.toLowerCase()))
                .map((l) => (
                  <ActionList.Item key={l.id} selected={issue.labels.some((x) => x.id === l.id)} onSelect={() => patch.mutate({ labels: toggle(issue.labels.map((x) => x.name), l.name) })}>
                    <ActionList.LeadingVisual>
                      <span style={{ width: 14, height: 14, borderRadius: 999, background: `#${l.color}`, display: 'inline-block' }} />
                    </ActionList.LeadingVisual>
                    {l.name}
                    {l.description ? <ActionList.Description variant="block">{l.description}</ActionList.Description> : null}
                  </ActionList.Item>
                ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        <div className="sidebar-body" style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
          {issue.labels.length === 0 ? <span className="muted">None yet</span> : issue.labels.map((l) => <LabelBadge key={l.id} label={l} />)}
        </div>
      </section>
      <section>
        <ActionMenu>
          <ActionMenu.Anchor>
            <button className="sidebar-head" disabled={disabled} title={disabled ? undefined : "Edit"}>
              Milestone {!disabled ? <GearIcon size={16} /> : null}
            </button>
          </ActionMenu.Anchor>
          <ActionMenu.Overlay width="medium">
            <ActionList selectionVariant="single">
              <ActionList.Item selected={!issue.milestone} onSelect={() => patch.mutate({ milestone: null })}>
                No milestone
              </ActionList.Item>
              {milestones.data?.map((m: Milestone) => (
                <ActionList.Item key={m.id} selected={issue.milestone?.id === m.id} onSelect={() => patch.mutate({ milestone: m.number })}>
                  {m.title}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        <div className="sidebar-body">
          {issue.milestone ? (
            <Link to="/$owner/$repo/milestones" params={{ owner: repo.owner.login, repo: repo.name }} style={{ fontWeight: 600, color: 'var(--fgColor-default)' }}>
              {issue.milestone.title}
            </Link>
          ) : (
            <span className="muted">No milestone</span>
          )}
        </div>
      </section>
      {!disabled ? (
        <section>
          <div className="sidebar-head" style={{ cursor: 'default' }}>
            {issue.locked ? 'Unlock conversation' : 'Lock conversation'}
          </div>
          <div className="sidebar-body">
            <Button
              size="small"
              variant="invisible"
              onClick={async () => {
                await api(`/repos/${full}/issues/${issue.number}/lock`, { method: issue.locked ? 'DELETE' : 'PUT', json: issue.locked ? undefined : { lock_reason: 'resolved' } });
                qc.invalidateQueries({ queryKey: ['issues', full.toLowerCase(), issue.number] });
              }}
            >
              {issue.locked ? 'Unlock' : 'Lock'}
            </Button>
          </div>
        </section>
      ) : null}
      <IssueProjects repo={repo} number={issue.number} disabled={disabled} />
    </aside>
  );
}

/** Requested reviewers and everyone who has reviewed, with their latest verdict. */
function ReviewersSection({ repo, pr, disabled, assignees }: { repo: Repo; pr: Pull; disabled: boolean; assignees: User[] }) {
  const qc = useQueryClient();
  const full = repo.full_name;
  const reviews = useQuery(pullReviewsQuery(full, pr.number));
  const request = useMutation({
    mutationFn: (v: { login: string; remove: boolean }) => api(`/repos/${full}/pulls/${pr.number}/requested_reviewers`, { method: v.remove ? 'DELETE' : 'POST', json: { reviewers: [v.login] } }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['pulls', full.toLowerCase(), pr.number] }),
  });
  const latest = new Map<number, { user: User; state: string }>();
  for (const r of reviews.data ?? []) if (r.user && r.state !== 'PENDING' && r.state !== 'DISMISSED') latest.set(r.user.id, { user: r.user, state: r.state });
  const requested = pr.requested_reviewers ?? [];
  const rows: { user: User; state: string }[] = [...requested.map((u) => ({ user: u, state: 'REQUESTED' })), ...[...latest.values()].filter((l) => !requested.some((u) => u.id === l.user.id))];
  const candidates = assignees.filter((u) => u.id !== pr.user.id);
  return (
    <section>
      <ActionMenu>
        <ActionMenu.Anchor>
          <button className="sidebar-head" disabled={disabled} title={disabled ? undefined : "Edit"}>
            Reviewers {!disabled ? <GearIcon size={16} /> : null}
          </button>
        </ActionMenu.Anchor>
        <ActionMenu.Overlay width="medium">
          <ActionList selectionVariant="multiple">
            {candidates.map((u) => {
              const on = requested.some((r) => r.id === u.id);
              return (
                <ActionList.Item key={u.id} selected={on} onSelect={() => request.mutate({ login: u.login, remove: on })}>
                  <ActionList.LeadingVisual>
                    <Avatar src={u.avatar_url} size={20} alt="" />
                  </ActionList.LeadingVisual>
                  {u.login}
                </ActionList.Item>
              );
            })}
            {candidates.length === 0 ? <ActionList.Item disabled>No one to request</ActionList.Item> : null}
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      <div className="sidebar-body">
        {rows.length === 0 ? (
          <span className="muted">No reviews</span>
        ) : (
          rows.map((r) => (
            <div key={r.user.id} className="reviewer-row">
              <Link to="/$owner" params={{ owner: r.user.login }} style={{ display: 'flex', alignItems: 'center', gap: 6, color: 'var(--fgColor-default)', fontWeight: 600 }}>
                <Avatar src={r.user.avatar_url} size={20} alt="" /> {r.user.login}
              </Link>
              <span className="reviewer-state" title={r.state === 'REQUESTED' ? 'Awaiting requested review' : r.state === 'APPROVED' ? 'Approved' : r.state === 'CHANGES_REQUESTED' ? 'Requested changes' : 'Left review comments'}>
                {r.state === 'REQUESTED' ? <DotFillIcon fill="var(--fgColor-attention)" /> : r.state === 'APPROVED' ? <CheckIcon fill="var(--fgColor-success)" /> : r.state === 'CHANGES_REQUESTED' ? <XIcon fill="var(--fgColor-danger)" /> : <CommentIcon fill="var(--fgColor-muted)" />}
              </span>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

export function CommentBox({ repo, issue, me, onPosted }: { repo: Repo; issue: Issue; me: User; onPosted: () => void }) {
  const [body, setBody] = useState('');
  const qc = useQueryClient();
  const full = repo.full_name;
  const post = useMutation({
    mutationFn: () => api(`/repos/${full}/issues/${issue.number}/comments`, { method: 'POST', json: { body } }),
    onSuccess: () => {
      setBody('');
      qc.invalidateQueries({ queryKey: ['issues', full.toLowerCase(), issue.number] });
      onPosted();
    },
  });
  const close = useMutation({
    mutationFn: async (reason: 'completed' | 'not_planned' | 'reopen') => {
      if (body.trim()) await api(`/repos/${full}/issues/${issue.number}/comments`, { method: 'POST', json: { body } });
      await api(`/repos/${full}/issues/${issue.number}`, { method: 'PATCH', json: reason === 'reopen' ? { state: 'open' } : { state: 'closed', state_reason: reason } });
    },
    onSuccess: () => {
      setBody('');
      qc.invalidateQueries({ queryKey: ['issues', full.toLowerCase()] });
      onPosted();
    },
  });
  const canClose = repo.permissions?.triage || me.id === issue.user.id;
  return (
    <div className="comment-box" style={{ display: 'flex', gap: 12, marginTop: 16 }}>
      <Avatar src={me.avatar_url} size={40} alt="" />
      <div style={{ flex: 1 }}>
        <div className="timeline-comment">
          <div className="comment-header">Add a comment</div>
          <div style={{ padding: 8 }}>
            <MarkdownEditor value={body} onChange={setBody} repo={repo} placeholder="Leave a comment" />
          </div>
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: '0 8px 8px' }}>
            {canClose && !issue.pull_request ? (
              issue.state === 'open' ? (
                <ActionMenu>
                  <ActionMenu.Button leadingVisual={IssueClosedIcon}>Close issue</ActionMenu.Button>
                  <ActionMenu.Overlay>
                    <ActionList>
                      <ActionList.Item onSelect={() => close.mutate('completed')}>
                        <ActionList.LeadingVisual>
                          <CheckIcon />
                        </ActionList.LeadingVisual>
                        Close as completed
                      </ActionList.Item>
                      <ActionList.Item onSelect={() => close.mutate('not_planned')}>
                        <ActionList.LeadingVisual>
                          <SkipIcon />
                        </ActionList.LeadingVisual>
                        Close as not planned
                      </ActionList.Item>
                    </ActionList>
                  </ActionMenu.Overlay>
                </ActionMenu>
              ) : (
                <Button leadingVisual={IssueOpenedIcon} onClick={() => close.mutate('reopen')}>
                  Reopen issue
                </Button>
              )
            ) : null}
            <Button variant="primary" disabled={!body.trim() || post.isPending} onClick={() => post.mutate()}>
              Comment
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function IssueRow({ issue, repo }: { issue: Issue; repo: Repo }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  const isPr = !!issue.pull_request;
  const merged = !!issue.pull_request?.merged_at;
  const when = issue.state === 'open' ? issue.created_at : (merged ? issue.pull_request?.merged_at : issue.closed_at) ?? issue.updated_at;
  const verb = issue.state === 'open' ? 'opened' : merged ? 'merged' : 'closed';
  return (
    <div className="issue-row">
      <span className="state">
        <StateIcon issue={issue} />
      </span>
      <div className="main">
        <div className="title-line">
          {isPr ? (
            <Link to="/$owner/$repo/pull/$number" params={{ ...p, number: String(issue.number) }} search={{}} className="title">
              {issue.title}
            </Link>
          ) : (
            <Link to="/$owner/$repo/issues/$number" params={{ ...p, number: String(issue.number) }} className="title">
              {issue.title}
            </Link>
          )}
          {issue.labels.map((l) => (
            <LabelBadge key={l.id} label={l} small />
          ))}
        </div>
        <div className="meta">
          #{issue.number} {verb} <RelativeTime datetime={when} /> by{' '}
          <Link to="/$owner" params={{ owner: issue.user.login }} className="muted">
            {issue.user.login}
          </Link>
          {issue.milestone ? (
            <>
              {' '}
              ·{' '}
              <Link to="/$owner/$repo/milestones" params={p} className="muted" style={{ display: 'inline-flex', alignItems: 'center', gap: 3 }}>
                <MilestoneIcon size={12} /> {issue.milestone.title}
              </Link>
            </>
          ) : null}
          {isPr && issue.review_decision === 'APPROVED' ? <span className="review-pill approved">· Approved</span> : null}
          {isPr && issue.review_decision === 'CHANGES_REQUESTED' ? <span className="review-pill changes">· Changes requested</span> : null}
          {isPr && issue.state === 'open' && issue.review_decision === 'REVIEW_REQUIRED' ? <span className="muted">· Review required</span> : null}
        </div>
      </div>
      <div className="side">
        {isPr && issue.checks_state ? (
          <span className={`checks-dot ${issue.checks_state}`} title={issue.checks_state === 'success' ? 'All checks have passed' : issue.checks_state === 'failure' ? 'Some checks were not successful' : 'Checks in progress'}>
            {issue.checks_state === 'success' ? <CheckCircleFillIcon size={14} /> : issue.checks_state === 'failure' ? <XCircleFillIcon size={14} /> : <DotFillIcon size={14} />}
          </span>
        ) : null}
        {issue.assignees.length ? (
          <AvatarStack>
            {issue.assignees.slice(0, 3).map((a) => (
              <Avatar key={a.id} src={a.avatar_url} size={20} alt={a.login} />
            ))}
          </AvatarStack>
        ) : null}
        {issue.comments > 0 ? (
          <span className="muted small comments-count">
            <CommentIcon size={14} /> {issue.comments}
          </span>
        ) : null}
      </div>
    </div>
  );
}

/** Author / Label / Milestones / Assignee / Sort dropdowns that edit a GitHub-style filter string. */
export function IssueFilters({ repo, q, setQ, sort, direction, isPr }: { repo: Repo; q: string; setQ: (q: string) => void; sort?: string; direction?: string; isPr?: boolean }) {
  const labels = useQuery(labelsQuery(repo.full_name));
  const milestones = useQuery(milestonesQuery(repo.full_name, 'open'));
  const assignees = useQuery(assigneesQuery(repo.full_name));
  const replaceTok = (prefix: string, val: string | null) => {
    const rest = q.split(/\s+/).filter((t) => t && !t.startsWith(prefix + ':'));
    return [...rest, ...(val ? [`${prefix}:${val.includes(' ') ? `"${val}"` : val}`] : [])].join(' ');
  };
  const Menu = ({ label, children, width }: { label: string; children: React.ReactNode; width?: 'medium' | 'auto' }) => (
    <ActionMenu>
      <ActionMenu.Button variant="invisible" size="small">
        {label}
      </ActionMenu.Button>
      <ActionMenu.Overlay width={width ?? 'auto'}>{children}</ActionMenu.Overlay>
    </ActionMenu>
  );
  return (
    <div className="filters">
      <Menu label="Author" width="medium">
        <ActionList>
          <ActionList.Item onSelect={() => setQ(replaceTok('author', null))}>Anyone</ActionList.Item>
          <ActionList.Item onSelect={() => setQ(replaceTok('author', '@me'))}>Created by me</ActionList.Item>
          {assignees.data?.map((u) => (
            <ActionList.Item key={u.id} onSelect={() => setQ(replaceTok('author', u.login))}>
              <ActionList.LeadingVisual>
                <Avatar src={u.avatar_url} size={20} alt="" />
              </ActionList.LeadingVisual>
              {u.login}
            </ActionList.Item>
          ))}
        </ActionList>
      </Menu>
      <Menu label="Label" width="medium">
        <ActionList>
          <ActionList.Item onSelect={() => setQ(q.split(/\s+/).filter((t) => !t.startsWith('label:')).join(' '))}>Any label</ActionList.Item>
          <ActionList.Item onSelect={() => setQ(replaceTok('no', 'label'))}>Unlabeled</ActionList.Item>
          {labels.data?.map((l) => (
            <ActionList.Item key={l.id} onSelect={() => setQ(`${q} label:${l.name.includes(' ') ? `"${l.name}"` : l.name}`)}>
              <ActionList.LeadingVisual>
                <span style={{ width: 14, height: 14, borderRadius: 999, background: `#${l.color}`, display: 'inline-block' }} />
              </ActionList.LeadingVisual>
              {l.name}
              {l.description ? <ActionList.Description variant="block">{l.description}</ActionList.Description> : null}
            </ActionList.Item>
          ))}
        </ActionList>
      </Menu>
      <Menu label="Milestones" width="medium">
        <ActionList>
          <ActionList.Item onSelect={() => setQ(replaceTok('milestone', null))}>Any milestone</ActionList.Item>
          <ActionList.Item onSelect={() => setQ(replaceTok('no', 'milestone'))}>No milestone</ActionList.Item>
          {milestones.data?.map((m) => (
            <ActionList.Item key={m.id} onSelect={() => setQ(replaceTok('milestone', m.title))}>
              {m.title}
            </ActionList.Item>
          ))}
        </ActionList>
      </Menu>
      <Menu label="Assignee" width="medium">
        <ActionList>
          <ActionList.Item onSelect={() => setQ(replaceTok('assignee', null))}>Anyone</ActionList.Item>
          <ActionList.Item onSelect={() => setQ(replaceTok('no', 'assignee'))}>Assigned to nobody</ActionList.Item>
          <ActionList.Item onSelect={() => setQ(replaceTok('assignee', '@me'))}>Assigned to me</ActionList.Item>
          {assignees.data?.map((u) => (
            <ActionList.Item key={u.id} onSelect={() => setQ(replaceTok('assignee', u.login))}>
              <ActionList.LeadingVisual>
                <Avatar src={u.avatar_url} size={20} alt="" />
              </ActionList.LeadingVisual>
              {u.login}
            </ActionList.Item>
          ))}
        </ActionList>
      </Menu>
      <Menu label="Sort">
        <ActionList selectionVariant="single">
          {[
            ['Newest', 'created-desc'],
            ['Oldest', 'created-asc'],
            ['Most commented', 'comments-desc'],
            ['Least commented', 'comments-asc'],
            ['Recently updated', 'updated-desc'],
            ['Least recently updated', 'updated-asc'],
          ].map(([label, v]) => (
            <ActionList.Item key={v} selected={(sort ? `${sort}-${direction}` : 'created-desc') === v} onSelect={() => setQ(replaceTok('sort', v))}>
              {label}
            </ActionList.Item>
          ))}
        </ActionList>
      </Menu>
      {isPr ? null : null}
    </div>
  );
}
