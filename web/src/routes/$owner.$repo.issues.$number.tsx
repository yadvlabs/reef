import { Avatar, Button, RelativeTime, TextInput, ActionMenu } from '@primer/react';
import { IssueClosedIcon, IssueOpenedIcon, TagIcon, PersonIcon, MilestoneIcon, PencilIcon, LockIcon, GitMergeIcon, SkipIcon, SmileyIcon, CrossReferenceIcon, GitPullRequestIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useState } from 'react';
import { CommentBox, IssueSidebar, LabelBadge, StateBadge, MarkdownEditor } from '../components/IssueBits';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { Markdown } from '../components/Markdown';
import { api, ApiError, type Repo, type User } from '../lib/api';
import { issueQuery, timelineQuery, type Issue, type TimelineItem } from '../lib/issues';
import { useSession } from '../lib/session';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/issues/$number')({ component: IssuePage });

export function IssuePage() {
  const repo = useRepo();
  const { number } = Route.useParams();
  const n = Number(number);
  const q = useQuery(issueQuery(repo.full_name, n));
  if (q.isPending) return <Loading />;
  if (q.isError) return q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="issue" /> : <ErrorFlash error={q.error} />;
  return <IssueView repo={repo} issue={q.data} />;
}

export function IssueView({ repo, issue, extraHead, after }: { repo: Repo; issue: Issue; extraHead?: React.ReactNode; after?: React.ReactNode }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const timeline = useQuery(timelineQuery(repo.full_name, issue.number));
  const [editingTitle, setEditingTitle] = useState(false);
  const [title, setTitle] = useState(issue.title);
  const canEdit = !!repo.permissions?.triage || user?.id === issue.user.id;
  const rename = useMutation({
    mutationFn: () => api(`/repos/${repo.full_name}/issues/${issue.number}`, { method: 'PATCH', json: { title } }),
    onSuccess: () => {
      setEditingTitle(false);
      qc.invalidateQueries({ queryKey: ['issues', repo.full_name.toLowerCase()] });
    },
  });
  const refresh = () => qc.invalidateQueries({ queryKey: ['issues', repo.full_name.toLowerCase(), issue.number] });
  return (
    <div>
      <div className="issue-head">
        {editingTitle ? (
          <form
            style={{ display: 'flex', gap: 8 }}
            onSubmit={(e) => {
              e.preventDefault();
              rename.mutate();
            }}
          >
            <TextInput value={title} onChange={(e) => setTitle(e.target.value)} block autoFocus />
            <Button type="submit" variant="primary">
              Save
            </Button>
            <Button onClick={() => setEditingTitle(false)}>Cancel</Button>
          </form>
        ) : (
          <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
            <h1 style={{ flex: 1 }}>
              {issue.title} <span className="number">#{issue.number}</span>
            </h1>
            {canEdit ? (
              <Button size="small" onClick={() => setEditingTitle(true)} leadingVisual={PencilIcon}>
                Edit
              </Button>
            ) : null}
          </div>
        )}
        <div className="meta">
          <StateBadge issue={issue} />
          <span>
            <Link to="/$owner" params={{ owner: issue.user.login }} style={{ fontWeight: 600, color: 'var(--fgColor-muted)' }}>
              {issue.user.login}
            </Link>{' '}
            opened this {issue.pull_request ? 'pull request' : 'issue'} <RelativeTime datetime={issue.created_at} /> · {issue.comments} {issue.comments === 1 ? 'comment' : 'comments'}
          </span>
        </div>
        {extraHead}
      </div>
      <div className="issue-layout">
        <div className="timeline">
          <CommentCard repo={repo} author={issue.user} createdAt={issue.created_at} bodyHtml={issue.body_html ?? ''} body={issue.body ?? ''} association={issue.author_association} reactions={issue.reactions} editable={canEdit} subject={`/repos/${repo.full_name}/issues/${issue.number}`} reactionsPath={`/repos/${repo.full_name}/issues/${issue.number}/reactions`} onChanged={refresh} me={user} isBody />
          {timeline.data?.map((t) => (
            <TimelineEntry key={`${t.event}-${t.id}`} item={t} repo={repo} me={user} onChanged={() => { refresh(); qc.invalidateQueries({ queryKey: timelineQuery(repo.full_name, issue.number).queryKey }); }} />
          ))}
          {after}
          {user ? (
            issue.locked && !repo.permissions?.triage ? (
              <p className="muted" style={{ marginTop: 16, display: 'flex', gap: 8, alignItems: 'center' }}>
                <LockIcon /> This conversation has been locked and limited to collaborators.
              </p>
            ) : (
              <CommentBox repo={repo} issue={issue} me={user} onPosted={() => qc.invalidateQueries({ queryKey: timelineQuery(repo.full_name, issue.number).queryKey })} />
            )
          ) : (
            <p className="muted" style={{ marginTop: 16 }}>
              <Link to="/login" search={{ return_to: location.pathname }}>Sign in</Link> to comment.
            </p>
          )}
        </div>
        <IssueSidebar repo={repo} issue={issue} disabled={!repo.permissions?.triage} />
      </div>
    </div>
  );
}

const REACTION_EMOJI: Record<string, string> = { '+1': '👍', '-1': '👎', laugh: '😄', hooray: '🎉', confused: '😕', heart: '❤️', rocket: '🚀', eyes: '👀' };

export function CommentCard({ repo, author, createdAt, bodyHtml, body, association, reactions, editable, subject, reactionsPath, onChanged, me, isBody }: { repo: Repo; author: User; createdAt: string; bodyHtml: string; body: string; association: string; reactions?: Issue['reactions']; editable: boolean; subject: string; reactionsPath: string; onChanged: () => void; me: User | null; isBody?: boolean }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(body);
  const save = useMutation({
    mutationFn: () => api(subject, { method: 'PATCH', json: { body: draft } }),
    onSuccess: () => {
      setEditing(false);
      onChanged();
    },
  });
  const mine = reactions?.viewer_reactions ?? [];
  const react = useMutation({
    mutationFn: async (content: string) => {
      if (mine.includes(content)) {
        // Un-react: find our own reaction and delete it, as the GitHub API requires.
        const list = await api<{ id: number; content: string; user: { id: number } }[]>(`${reactionsPath}?content=${encodeURIComponent(content)}&per_page=100`);
        const own = list.find((r) => r.user?.id === me?.id && r.content === content);
        if (own) await api(`${reactionsPath}/${own.id}`, { method: 'DELETE' });
        return;
      }
      await api(reactionsPath, { method: 'POST', json: { content } });
    },
    onSuccess: onChanged,
  });
  const del = useMutation({
    mutationFn: () => api(subject, { method: 'DELETE' }),
    onSuccess: onChanged,
  });
  return (
    <div className="timeline-item">
      <Avatar src={author.avatar_url} size={40} alt="" />
      <div className="body">
        <div className="timeline-comment">
          <div className="comment-header">
            <Link to="/$owner" params={{ owner: author.login }} style={{ color: 'inherit' }}>
              <strong>{author.login}</strong>
            </Link>
            commented <RelativeTime datetime={createdAt} />
            {association !== 'NONE' ? <span className="badge">{association.charAt(0) + association.slice(1).toLowerCase()}</span> : null}
            {editable ? (
              <span style={{ marginLeft: association !== 'NONE' ? 8 : 'auto', display: 'flex', gap: 4 }}>
                <Button size="small" variant="invisible" onClick={() => setEditing(true)}>
                  Edit
                </Button>
                {!isBody ? (
                  <Button size="small" variant="invisible" onClick={() => confirm('Delete this comment?') && del.mutate()}>
                    Delete
                  </Button>
                ) : null}
              </span>
            ) : null}
          </div>
          {editing ? (
            <div style={{ padding: 8 }}>
              <MarkdownEditor value={draft} onChange={setDraft} repo={repo} />
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 8 }}>
                <Button onClick={() => setEditing(false)}>Cancel</Button>
                <Button variant="primary" onClick={() => save.mutate()}>
                  Update comment
                </Button>
              </div>
            </div>
          ) : (
            <Markdown html={bodyHtml || '<p class="muted"><em>No description provided.</em></p>'} />
          )}
          {reactions && !editing ? (
            <div className="reactions">
              {Object.entries(REACTION_EMOJI).map(([k, e]) =>
                (reactions[k as keyof typeof reactions] as number) > 0 ? (
                  <button key={k} className={mine.includes(k) ? 'mine' : undefined} onClick={() => me && react.mutate(k)} title={mine.includes(k) ? `Remove your ${e} reaction` : `React with ${e}`} disabled={!me || react.isPending}>
                    {e} {reactions[k as keyof typeof reactions] as number}
                  </button>
                ) : null,
              )}
              {me ? (
                <ActionMenu>
                  <ActionMenu.Anchor>
                    <button className="reaction-add" title="Add reaction" aria-label="Add reaction">
                      <SmileyIcon size={14} />
                    </button>
                  </ActionMenu.Anchor>
                  <ActionMenu.Overlay>
                    <div className="reaction-picker">
                      {Object.entries(REACTION_EMOJI).map(([k, e]) => (
                        <button key={k} className={mine.includes(k) ? 'mine' : undefined} onClick={() => react.mutate(k)} title={k}>
                          {e}
                        </button>
                      ))}
                    </div>
                  </ActionMenu.Overlay>
                </ActionMenu>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function TimelineEntry({ item, repo, me, onChanged }: { item: TimelineItem; repo: Repo; me: User | null; onChanged: () => void }) {
  const who = item.actor ? (
    <Link to="/$owner" params={{ owner: item.actor.login }} style={{ color: 'inherit' }}>
      <strong>{item.actor.login}</strong>
    </Link>
  ) : (
    <strong>someone</strong>
  );
  const when = <RelativeTime datetime={item.created_at} />;
  if (item.event === 'commented' && item.user) {
    const editable = !!me && (me.id === item.user.id || !!repo.permissions?.push);
    return (
      <CommentCard repo={repo} author={item.user} createdAt={item.created_at} bodyHtml={item.body_html ?? ''} body={item.body ?? ''} association={item.author_association ?? 'NONE'} reactions={item.reactions} editable={editable} subject={`/repos/${repo.full_name}/issues/comments/${item.id}`} reactionsPath={`/repos/${repo.full_name}/issues/comments/${item.id}/reactions`} onChanged={onChanged} me={me} />
    );
  }
  const av = item.actor ? <Avatar src={item.actor.avatar_url} size={20} alt="" /> : null;
  switch (item.event) {
    case 'closed':
      return (
        <div className="timeline-event">
          <span className={`icon ${item.state_reason === 'not_planned' ? '' : 'closed'}`}>{item.state_reason === 'not_planned' ? <SkipIcon /> : <IssueClosedIcon />}</span>
          {av} {who} closed this as {item.state_reason === 'not_planned' ? 'not planned' : 'completed'}
          {item.closing_pull ? (
            <>
              {' '}
              in{' '}
              <Link to="/$owner/$repo/pull/$number" params={{ owner: repo.owner.login, repo: repo.name, number: String(item.closing_pull.number) }} className="ref-link">
                #{item.closing_pull.number}
              </Link>
            </>
          ) : null}
          {item.commit_id ? (
            <>
              {' '}
              via{' '}
              <Link to="/$owner/$repo/commit/$sha" params={{ owner: repo.owner.login, repo: repo.name, sha: item.commit_id }}>
                <code>{item.commit_id.slice(0, 7)}</code>
              </Link>
            </>
          ) : null}{' '}
          {when}
        </div>
      );
    case 'cross-referenced':
      return item.source ? (
        <div className="timeline-event">
          <span className="icon">
            <CrossReferenceIcon />
          </span>
          {av} {who} {item.will_close ? 'linked a pull request that will close this issue' : 'mentioned this'} {when}
          <div className="ref-card">
            {item.source.issue.pull_request ? (
              <GitPullRequestIcon fill={item.source.issue.merged ? 'var(--fgColor-done)' : item.source.issue.state === 'open' ? 'var(--fgColor-success)' : 'var(--fgColor-danger)'} />
            ) : (
              <IssueOpenedIcon fill={item.source.issue.state === 'open' ? 'var(--fgColor-success)' : 'var(--fgColor-done)'} />
            )}
            <a href={item.source.issue.html_url}>
              <strong>{item.source.issue.title}</strong> <span className="muted">#{item.source.issue.number}</span>
            </a>
          </div>
        </div>
      ) : null;
    case 'reopened':
      return (
        <div className="timeline-event">
          <span className="icon open">
            <IssueOpenedIcon />
          </span>
          {av} {who} reopened this {when}
        </div>
      );
    case 'merged':
      return (
        <div className="timeline-event">
          <span className="icon merged">
            <GitMergeIcon />
          </span>
          {av} {who} merged commit{' '}
          {item.commit_id ? (
            <Link to="/$owner/$repo/commit/$sha" params={{ owner: repo.owner.login, repo: repo.name, sha: item.commit_id }}>
              <code>{item.commit_id.slice(0, 7)}</code>
            </Link>
          ) : null}{' '}
          into <code>{item.base_ref ?? ''}</code>
          {item.head_ref ? (
            <>
              {' '}
              from <code>{item.head_ref}</code>
            </>
          ) : null}
          {item.merge_method && item.merge_method !== 'merge' ? <span className="muted"> ({item.merge_method === 'squash' ? 'squashed' : 'rebased'})</span> : null} {when}
        </div>
      );
    case 'labeled':
    case 'unlabeled':
      return (
        <div className="timeline-event">
          <span className="icon">
            <TagIcon />
          </span>
          {av} {who} {item.event === 'labeled' ? 'added' : 'removed'} {item.label ? <LabelBadge label={{ id: 0, node_id: '', name: item.label.name, color: item.label.color, description: null, default: false }} small /> : null} {when}
        </div>
      );
    case 'assigned':
    case 'unassigned':
      return (
        <div className="timeline-event">
          <span className="icon">
            <PersonIcon />
          </span>
          {av} {who} {item.event} {item.assignee ? <strong>{item.assignee.login}</strong> : ''} {when}
        </div>
      );
    case 'milestoned':
    case 'demilestoned':
      return (
        <div className="timeline-event">
          <span className="icon">
            <MilestoneIcon />
          </span>
          {av} {who} {item.event === 'milestoned' ? 'added this to' : 'removed this from'} the <strong>{item.milestone?.title ?? 'deleted'}</strong> milestone {when}
        </div>
      );
    case 'renamed':
      return (
        <div className="timeline-event">
          <span className="icon">
            <PencilIcon />
          </span>
          {av} {who} changed the title <s>{item.rename?.from}</s> <strong>{item.rename?.to}</strong> {when}
        </div>
      );
    case 'locked':
    case 'unlocked':
      return (
        <div className="timeline-event">
          <span className="icon">
            <LockIcon />
          </span>
          {av} {who} {item.event} this conversation {when}
        </div>
      );
    default:
      return null;
  }
}
