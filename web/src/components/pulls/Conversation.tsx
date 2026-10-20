// The Conversation tab: description, comments, reviews and review threads interleaved on a
// timeline, the merge box, the comment form, and the sidebar.
import { ActionList, ActionMenu, Avatar, Button, RelativeTime } from '@primer/react';
import { AlertIcon, CheckIcon, CommentDiscussionIcon, DotFillIcon, EyeIcon, GitMergeIcon, GitPullRequestClosedIcon, GitPullRequestIcon, XIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo } from '../../lib/api';
import { timelineQuery, type TimelineItem } from '../../lib/issues';
import { checkRunsQuery, combinedStatusQuery, pullReviewsQuery, reviewCommentsQuery, threadsOf, type CheckRun, type CommitStatus, type Pull, type Review, type Thread } from '../../lib/pulls';
import { useSession } from '../../lib/session';
import { IssueSidebar, MarkdownEditor } from '../IssueBits';
import { ErrorFlash, Loading } from '../Loading';
import { Markdown } from '../Markdown';
import { CommentCard, TimelineEntry } from '../../routes/$owner.$repo.issues.$number';
import { invalidatePull } from './PullHeader';
import { ReviewThread } from './ReviewThread';

const SKIP_EVENTS = new Set(['reviewed', 'review_commented', 'mentioned', 'subscribed', 'unsubscribed']);

export function PullConversation({ repo, pr }: { repo: Repo; pr: Pull }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const timeline = useQuery(timelineQuery(repo.full_name, pr.number));
  const reviews = useQuery(pullReviewsQuery(repo.full_name, pr.number));
  const comments = useQuery(reviewCommentsQuery(repo.full_name, pr.number));
  const refresh = () => {
    invalidatePull(qc, repo, pr.number);
    qc.invalidateQueries({ queryKey: timelineQuery(repo.full_name, pr.number).queryKey });
  };
  if (timeline.isPending || reviews.isPending || comments.isPending) return <Loading />;
  if (timeline.isError) return <ErrorFlash error={timeline.error} />;
  const submitted = (reviews.data ?? []).filter((r) => r.state !== 'PENDING');
  const threads = threadsOf((comments.data ?? []).filter((c) => !c.pending));
  const byReview = new Map<number, Thread[]>();
  const standalone: Thread[] = [];
  for (const t of threads) {
    const rid = t.root.pull_request_review_id;
    if (rid && submitted.some((r) => r.id === rid)) byReview.set(rid, [...(byReview.get(rid) ?? []), t]);
    else standalone.push(t);
  }
  const entries: { at: number; key: string; node: React.ReactNode }[] = [];
  for (const t of timeline.data ?? []) {
    if (SKIP_EVENTS.has(t.event)) continue;
    // A merged pull request shows the merge event only, as on GitHub; the implicit close is noise.
    if (t.event === 'closed' && pr.merged) continue;
    entries.push({ at: Date.parse(t.created_at), key: `t-${t.event}-${t.id}`, node: <TimelineEntry item={t as TimelineItem} repo={repo} me={user} onChanged={refresh} /> });
  }
  for (const r of submitted) {
    entries.push({ at: Date.parse(r.submitted_at ?? '') || 0, key: `r-${r.id}`, node: <ReviewCard review={r} threads={byReview.get(r.id) ?? []} repo={repo} pr={pr} /> });
  }
  for (const t of standalone) {
    entries.push({ at: Date.parse(t.root.created_at), key: `th-${t.root.id}`, node: <ReviewThread thread={t} repo={repo} pr={pr} showFile /> });
  }
  entries.sort((a, b) => a.at - b.at);
  const canEdit = !!repo.permissions?.triage || user?.id === pr.user.id;
  return (
    <div className="issue-layout pr-layout">
      <div className="pr-main">
      <div className="timeline pr-timeline">
        <CommentCard repo={repo} author={pr.user} createdAt={pr.created_at} bodyHtml={pr.body_html ?? ''} body={pr.body ?? ''} association={pr.author_association} editable={canEdit} subject={`/repos/${repo.full_name}/issues/${pr.number}`} reactionsPath={`/repos/${repo.full_name}/issues/${pr.number}/reactions`} onChanged={refresh} me={user} isBody />
        {entries.map((e) => (
          <div key={e.key} className="tl-entry">
            {e.node}
          </div>
        ))}
      </div>
      <MergeBox repo={repo} pr={pr} reviews={submitted} />
      {user ? <PullCommentBox repo={repo} pr={pr} onPosted={refresh} /> : <p className="muted" style={{ marginTop: 16 }}><Link to="/login" search={{ return_to: location.pathname }}>Sign in</Link> to comment.</p>}
      </div>
      <IssueSidebar repo={repo} issue={pr} disabled={!repo.permissions?.triage} pull={pr} />
    </div>
  );
}

function ReviewCard({ review, threads, repo, pr }: { review: Review; threads: Thread[]; repo: Repo; pr: Pull }) {
  const verb = review.state === 'APPROVED' ? 'approved these changes' : review.state === 'CHANGES_REQUESTED' ? 'requested changes' : review.state === 'DISMISSED' ? 'left a review that was dismissed' : threads.length ? 'reviewed' : 'commented';
  const icon = review.state === 'APPROVED' ? <CheckIcon /> : review.state === 'CHANGES_REQUESTED' ? <XIcon /> : <EyeIcon />;
  const cls = review.state === 'APPROVED' ? 'approved' : review.state === 'CHANGES_REQUESTED' ? 'changes' : '';
  return (
    <div className={`review-card ${cls}`} id={`pullrequestreview-${review.id}`}>
      <div className="timeline-event review-event">
        <span className={`icon ${cls}`}>{icon}</span>
        {review.user ? <Avatar src={review.user.avatar_url} size={20} alt="" /> : null}
        {review.user ? (
          <Link to="/$owner" params={{ owner: review.user.login }} style={{ color: 'inherit' }}>
            <strong>{review.user.login}</strong>
          </Link>
        ) : (
          <strong>someone</strong>
        )}
        {verb} {review.submitted_at ? <RelativeTime datetime={review.submitted_at} /> : null}
      </div>
      {review.body ? (
        <div className="review-body">
          <Markdown html={review.body_html ?? review.body} />
        </div>
      ) : null}
      {threads.map((t) => (
        <ReviewThread key={t.root.id} thread={t} repo={repo} pr={pr} showFile />
      ))}
    </div>
  );
}

function checkState(runs: CheckRun[], statuses: CommitStatus[]): 'success' | 'failure' | 'pending' | null {
  const states: string[] = statuses.map((s) => (s.state === 'error' ? 'failure' : s.state));
  for (const r of runs) states.push(r.status !== 'completed' ? 'pending' : r.conclusion === 'success' || r.conclusion === 'skipped' || r.conclusion === 'neutral' ? 'success' : 'failure');
  if (states.length === 0) return null;
  if (states.includes('failure')) return 'failure';
  if (states.includes('pending')) return 'pending';
  return 'success';
}

export function MergeBox({ repo, pr, reviews }: { repo: Repo; pr: Pull; reviews: Review[] }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const runs = useQuery(checkRunsQuery(repo.full_name, pr.head.sha));
  const statuses = useQuery(combinedStatusQuery(repo.full_name, pr.head.sha));
  const methods = (['merge', 'squash', 'rebase'] as const).filter((m) => (m === 'merge' ? repo.allow_merge_commit !== false : m === 'squash' ? repo.allow_squash_merge !== false : repo.allow_rebase_merge !== false));
  const [method, setMethod] = useState<'merge' | 'squash' | 'rebase'>(methods[0] ?? 'merge');
  const [error, setError] = useState<unknown>(null);
  const refresh = () => {
    invalidatePull(qc, repo, pr.number);
    qc.invalidateQueries({ queryKey: ['repo'] });
    qc.invalidateQueries({ queryKey: timelineQuery(repo.full_name, pr.number).queryKey });
  };
  const merge = useMutation({ mutationFn: () => api(`/repos/${repo.full_name}/pulls/${pr.number}/merge`, { method: 'PUT', json: { merge_method: method, sha: pr.head.sha } }), onSuccess: refresh, onError: setError });
  const toggleDraft = useMutation({ mutationFn: () => api(`/repos/${repo.full_name}/pulls/${pr.number}`, { method: 'PATCH', json: { draft: !pr.draft } }), onSuccess: refresh, onError: setError });
  const latest = new Map<number, Review>();
  for (const r of reviews) if (r.user && r.state !== 'COMMENTED' && r.state !== 'DISMISSED' && r.state !== 'PENDING') latest.set(r.user.id, r);
  const approvals = [...latest.values()].filter((r) => r.state === 'APPROVED');
  const changes = [...latest.values()].filter((r) => r.state === 'CHANGES_REQUESTED');
  const canMerge = !!repo.permissions?.push && pr.state === 'open' && !pr.merged;
  const methodLabel = { merge: 'Merge pull request', squash: 'Squash and merge', rebase: 'Rebase and merge' } as const;
  const allRuns = runs.data?.check_runs ?? [];
  const allStatuses = statuses.data?.statuses ?? [];
  const cs = checkState(allRuns, allStatuses);
  const checksTotal = allRuns.length + allStatuses.length;
  const p = { owner: repo.owner.login, repo: repo.name, number: String(pr.number) };
  if (pr.merged) {
    return (
      <div className="merge-box merged">
        <div className="row">
          <span className="status-icon" style={{ background: 'var(--bgColor-done-emphasis)' }}>
            <GitMergeIcon />
          </span>
          <div className="text">
            <strong>Pull request successfully merged and closed</strong>
            {pr.merged_by ? (
              <span>
                <Link to="/$owner" params={{ owner: pr.merged_by.login }}>
                  {pr.merged_by.login}
                </Link>{' '}
                merged{' '}
                {pr.merge_commit_sha ? (
                  <Link to="/$owner/$repo/commit/$sha" params={{ owner: repo.owner.login, repo: repo.name, sha: pr.merge_commit_sha }}>
                    <code>{pr.merge_commit_sha.slice(0, 7)}</code>
                  </Link>
                ) : null}{' '}
                into <code>{pr.base.ref}</code> from <code>{pr.head.ref}</code>
                {pr.merged_at ? (
                  <>
                    {' '}
                    <RelativeTime datetime={pr.merged_at} />
                  </>
                ) : null}
                . The <code>{pr.head.ref}</code> branch can now be safely deleted.
              </span>
            ) : (
              <span>
                You're all set — the <code>{pr.head.ref}</code> branch can now be safely deleted.
              </span>
            )}
          </div>
        </div>
      </div>
    );
  }
  if (pr.state === 'closed') {
    return (
      <div className="merge-box closed">
        <div className="row">
          <span className="status-icon bad">
            <GitPullRequestClosedIcon />
          </span>
          <div className="text">
            <strong>Closed with unmerged commits</strong>This pull request is closed, but the <code>{pr.head.ref}</code> branch has unmerged commits.
          </div>
        </div>
      </div>
    );
  }
  return (
    <div className="merge-box">
      {error ? <div style={{ padding: 8 }}><ErrorFlash error={error} /></div> : null}
      <div className="row">
        <span className={`status-icon ${changes.length ? 'bad' : approvals.length ? 'ok' : 'neutral'}`}>{changes.length ? <XIcon /> : approvals.length ? <CheckIcon /> : <CommentDiscussionIcon />}</span>
        <div className="text">
          <strong>{changes.length ? 'Changes requested' : approvals.length ? `${approvals.length} approving ${approvals.length === 1 ? 'review' : 'reviews'}` : 'Review required'}</strong>
          {changes.length ? `${changes.map((r) => r.user?.login).join(', ')} requested changes.` : approvals.length ? `Approved by ${approvals.map((r) => r.user?.login).join(', ')}.` : 'At least one approving review is required before merging.'}
        </div>
      </div>
      {checksTotal > 0 ? (
        <div className="row">
          <span className={`status-icon ${cs === 'failure' ? 'bad' : cs === 'success' ? 'ok' : 'warn'}`}>{cs === 'failure' ? <XIcon /> : cs === 'success' ? <CheckIcon /> : <DotFillIcon />}</span>
          <div className="text">
            <strong>{cs === 'failure' ? 'Some checks were not successful' : cs === 'success' ? 'All checks have passed' : "Some checks haven't completed yet"}</strong>
            {checksTotal} {checksTotal === 1 ? 'check' : 'checks'} ·{' '}
            <Link to="/$owner/$repo/pull/$number/checks" params={p}>
              Show all checks
            </Link>
          </div>
        </div>
      ) : null}
      <div className="row">
        <span className={`status-icon ${pr.mergeable === false ? 'bad' : pr.mergeable ? 'ok' : 'neutral'}`}>{pr.mergeable ? <CheckIcon /> : <AlertIcon />}</span>
        <div className="text">
          <strong>{pr.mergeable === false ? 'This branch has conflicts that must be resolved' : pr.mergeable ? 'This branch has no conflicts with the base branch' : 'Checking for ability to merge automatically…'}</strong>
          {pr.mergeable ? 'Merging can be performed automatically.' : pr.mergeable === false ? 'Use the command line to resolve conflicts before merging.' : ''}
        </div>
      </div>
      {pr.draft ? (
        <div className="row">
          <span className="status-icon neutral">
            <GitPullRequestIcon />
          </span>
          <div className="text">
            <strong>This pull request is still a work in progress</strong>Draft pull requests cannot be merged.
          </div>
          {user && (user.id === pr.user.id || repo.permissions?.push) ? <Button onClick={() => toggleDraft.mutate()}>Ready for review</Button> : null}
        </div>
      ) : canMerge ? (
        <div className="row merge-actions">
          <div className="merge-button-group">
            <Button variant="primary" disabled={pr.mergeable === false} loading={merge.isPending} onClick={() => merge.mutate()}>
              {methodLabel[method]}
            </Button>
            <ActionMenu>
              <ActionMenu.Anchor>
                <Button variant="primary" aria-label="Select merge method" className="merge-caret">
                  ▾
                </Button>
              </ActionMenu.Anchor>
              <ActionMenu.Overlay>
                <ActionList selectionVariant="single">
                  {methods.map((m) => (
                    <ActionList.Item key={m} selected={m === method} onSelect={() => setMethod(m)}>
                      {methodLabel[m]}
                      <ActionList.Description variant="block">{m === 'merge' ? `All commits from this branch will be added to ${pr.base.ref} via a merge commit.` : m === 'squash' ? `The ${pr.commits} commits from this branch will be combined into one commit.` : `The ${pr.commits} commits from this branch will be rebased and added to ${pr.base.ref}.`}</ActionList.Description>
                    </ActionList.Item>
                  ))}
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          </div>
          <span className="muted small">You can also merge this with the command line.</span>
          {user && user.id === pr.user.id ? (
            <Button variant="invisible" size="small" onClick={() => toggleDraft.mutate()}>
              Convert to draft
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function PullCommentBox({ repo, pr, onPosted }: { repo: Repo; pr: Pull; onPosted: () => void }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const [body, setBody] = useState('');
  const full = repo.full_name;
  const post = useMutation({
    mutationFn: () => api(`/repos/${full}/issues/${pr.number}/comments`, { method: 'POST', json: { body } }),
    onSuccess: () => {
      setBody('');
      onPosted();
    },
  });
  const setState = useMutation({
    mutationFn: async (state: 'open' | 'closed') => {
      if (body.trim()) await api(`/repos/${full}/issues/${pr.number}/comments`, { method: 'POST', json: { body } });
      await api(`/repos/${full}/pulls/${pr.number}`, { method: 'PATCH', json: { state } });
    },
    onSuccess: () => {
      setBody('');
      qc.invalidateQueries({ queryKey: ['repo'] });
      onPosted();
    },
  });
  if (!user) return null;
  const canClose = !!repo.permissions?.triage || user.id === pr.user.id;
  return (
    <div className="comment-box" style={{ display: 'flex', gap: 12, marginTop: 16 }}>
      <Avatar src={user.avatar_url} size={40} alt="" />
      <div style={{ flex: 1 }}>
        <div className="timeline-comment">
          <div className="comment-header">Add a comment</div>
          <div style={{ padding: 8 }}>
            <MarkdownEditor value={body} onChange={setBody} repo={repo} placeholder="Leave a comment" />
          </div>
          <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: '0 8px 8px' }}>
            {canClose && !pr.merged ? (
              pr.state === 'open' ? (
                <Button leadingVisual={GitPullRequestClosedIcon} onClick={() => setState.mutate('closed')} loading={setState.isPending}>
                  {body.trim() ? 'Close with comment' : 'Close pull request'}
                </Button>
              ) : (
                <Button leadingVisual={GitPullRequestIcon} onClick={() => setState.mutate('open')} loading={setState.isPending}>
                  Reopen pull request
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
