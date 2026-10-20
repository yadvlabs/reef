// The "Files changed" tab: diffs with inline review threads, a pending review that collects
// comments until "Finish your review", unified/split toggle, viewed files and a file jump list.
import { ActionList, ActionMenu, Button, Radio, RadioGroup, SegmentedControl } from '@primer/react';
import { FileIcon, TriangleDownIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';
import { api, type Repo } from '../../lib/api';
import { pullFilesQuery, pullReviewsQuery, reviewCommentsQuery, threadKey, threadsOf, type Pull, type PullFile } from '../../lib/pulls';
import { useSession } from '../../lib/session';
import { FileDiff, type Side } from '../Diff';
import { MarkdownEditor } from '../IssueBits';
import { ErrorFlash, Loading } from '../Loading';
import { invalidatePull } from './PullHeader';
import { ReviewThread } from './ReviewThread';

type Event = 'COMMENT' | 'APPROVE' | 'REQUEST_CHANGES';

function useViewed(key: string): [Set<string>, (path: string, v: boolean) => void] {
  const [set, setSet] = useState<Set<string>>(() => {
    try {
      return new Set(JSON.parse(localStorage.getItem(key) ?? '[]') as string[]);
    } catch {
      return new Set();
    }
  });
  const update = (path: string, v: boolean) => {
    const next = new Set(set);
    if (v) next.add(path);
    else next.delete(path);
    setSet(next);
    localStorage.setItem(key, JSON.stringify([...next]));
  };
  return [set, update];
}

export function PullFiles({ repo, pr }: { repo: Repo; pr: Pull }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const files = useQuery(pullFilesQuery(repo.full_name, pr.number));
  const comments = useQuery(reviewCommentsQuery(repo.full_name, pr.number));
  const reviews = useQuery(pullReviewsQuery(repo.full_name, pr.number));
  const [split, setSplit] = useState(() => localStorage.getItem('reef.diff.split') === '1');
  const [viewed, setViewed] = useViewed(`reef.viewed.${repo.full_name}.${pr.number}.${pr.head.sha}`);
  const [forms, setForms] = useState<Record<string, boolean>>({});
  const [reviewOpen, setReviewOpen] = useState(false);
  useEffect(() => localStorage.setItem('reef.diff.split', split ? '1' : '0'), [split]);
  useEffect(() => {
    if (location.hash) document.getElementById(decodeURIComponent(location.hash.slice(1)))?.scrollIntoView();
  }, [files.data]);
  const refresh = () => invalidatePull(qc, repo, pr.number);
  const pending = (reviews.data ?? []).find((r) => r.state === 'PENDING') ?? null;
  const pendingCount = (comments.data ?? []).filter((c) => c.pending).length;
  const threads = useMemo(() => {
    const map = new Map<string, ReturnType<typeof threadsOf>>();
    for (const t of threadsOf(comments.data ?? [])) {
      const k = threadKey(t.root.path, t.root.side, t.root.line);
      map.set(k, [...(map.get(k) ?? []), t]);
    }
    return map;
  }, [comments.data]);
  const addSingle = useMutation({ mutationFn: (v: { path: string; line: number; side: Side; body: string }) => api(`/repos/${repo.full_name}/pulls/${pr.number}/comments`, { method: 'POST', json: v }), onSuccess: refresh });
  const addPending = useMutation({ mutationFn: (v: { path: string; line: number; side: Side; body: string }) => api(`/repos/${repo.full_name}/pulls/${pr.number}/reviews`, { method: 'POST', json: { comments: [v] } }), onSuccess: refresh });
  const submit = useMutation({
    mutationFn: (v: { body: string; event: Event }) => (pending ? api(`/repos/${repo.full_name}/pulls/${pr.number}/reviews/${pending.id}/events`, { method: 'POST', json: v }) : api(`/repos/${repo.full_name}/pulls/${pr.number}/reviews`, { method: 'POST', json: v })),
    onSuccess: () => {
      setReviewOpen(false);
      refresh();
    },
  });
  const discard = useMutation({ mutationFn: () => api(`/repos/${repo.full_name}/pulls/${pr.number}/reviews/${pending!.id}`, { method: 'DELETE' }), onSuccess: refresh });
  if (files.isPending || comments.isPending) return <Loading />;
  if (files.isError) return <ErrorFlash error={files.error} />;
  const list: PullFile[] = files.data;
  const adds = list.reduce((a, f) => a + f.additions, 0);
  const dels = list.reduce((a, f) => a + f.deletions, 0);
  const canComment = !!user && pr.state === 'open';
  const formKey = (path: string, side: Side, line: number) => threadKey(path, side, line);
  const under = (file: PullFile) => (a: { side: Side; line: number }) => {
    const k = formKey(file.filename, a.side, a.line);
    const ts = threads.get(k) ?? [];
    const open = forms[k];
    if (ts.length === 0 && !open) return null;
    return (
      <div className="inline-threads">
        {ts.map((t) => (
          <ReviewThread key={t.root.id} thread={t} repo={repo} pr={pr} compact />
        ))}
        {open ? (
          <InlineCommentForm
            hasPending={!!pending}
            busy={addSingle.isPending || addPending.isPending}
            onCancel={() => setForms((f) => ({ ...f, [k]: false }))}
            onSingle={(body) => addSingle.mutate({ path: file.filename, line: a.line, side: a.side, body }, { onSuccess: () => setForms((f) => ({ ...f, [k]: false })) })}
            onReview={(body) => addPending.mutate({ path: file.filename, line: a.line, side: a.side, body }, { onSuccess: () => setForms((f) => ({ ...f, [k]: false })) })}
          />
        ) : null}
      </div>
    );
  };
  return (
    <div className="pr-files">
      <div className="pr-files-toolbar">
        <ActionMenu>
          <ActionMenu.Button size="small" trailingAction={TriangleDownIcon} leadingVisual={FileIcon}>
            {list.length} {list.length === 1 ? 'file' : 'files'}
          </ActionMenu.Button>
          <ActionMenu.Overlay width="large">
            <ActionList>
              {list.map((f) => (
                <ActionList.LinkItem key={f.filename} href={`#diff-${f.filename}`}>
                  <ActionList.LeadingVisual>
                    <FileIcon />
                  </ActionList.LeadingVisual>
                  <span className="monospace small">{f.filename}</span>
                  <ActionList.TrailingVisual>
                    <span className="add">+{f.additions}</span> <span className="del">−{f.deletions}</span>
                  </ActionList.TrailingVisual>
                </ActionList.LinkItem>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        <span className="muted small">
          <span className="add">+{adds}</span> <span className="del">−{dels}</span> · {viewed.size} / {list.length} files viewed
        </span>
        <span className="grow" />
        <SegmentedControl aria-label="Diff view" size="small" onChange={(i) => setSplit(i === 1)}>
          <SegmentedControl.Button selected={!split}>Unified</SegmentedControl.Button>
          <SegmentedControl.Button selected={split}>Split</SegmentedControl.Button>
        </SegmentedControl>
        {canComment ? (
          <div className="review-changes">
            <Button variant={pendingCount ? 'primary' : 'default'} size="small" trailingAction={TriangleDownIcon} onClick={() => setReviewOpen(!reviewOpen)}>
              {pendingCount ? `Finish your review` : 'Review changes'}
              {pendingCount ? <span className="counter">{pendingCount}</span> : null}
            </Button>
            {reviewOpen ? (
              <ReviewPopover
                pr={pr}
                repo={repo}
                pendingCount={pendingCount}
                busy={submit.isPending}
                error={submit.error}
                onClose={() => setReviewOpen(false)}
                onSubmit={(body, event) => submit.mutate({ body, event })}
                onDiscard={pending ? () => confirm('Discard your pending review and its comments?') && discard.mutate() : undefined}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      {list.map((f) => (
        <FileDiff
          key={f.filename}
          file={f}
          defaultOpen={list.length <= 30}
          split={split}
          viewed={viewed.has(f.filename)}
          onViewed={(v) => setViewed(f.filename, v)}
          canComment={canComment}
          onAddComment={(a) => setForms((s) => ({ ...s, [formKey(f.filename, a.side, a.line)]: true }))}
          under={under(f)}
        />
      ))}
    </div>
  );
}

function InlineCommentForm({ hasPending, busy, onCancel, onSingle, onReview }: { hasPending: boolean; busy: boolean; onCancel: () => void; onSingle: (body: string) => void; onReview: (body: string) => void }) {
  const [body, setBody] = useState('');
  return (
    <div className="inline-comment-form">
      <MarkdownEditor value={body} onChange={setBody} placeholder="Leave a comment" minHeight={70} autoFocus />
      <div className="rc-actions">
        <Button size="small" onClick={onCancel}>
          Cancel
        </Button>
        {!hasPending ? (
          <Button size="small" disabled={!body.trim()} loading={busy} onClick={() => onSingle(body)}>
            Add single comment
          </Button>
        ) : null}
        <Button size="small" variant="primary" disabled={!body.trim()} loading={busy} onClick={() => onReview(body)}>
          {hasPending ? 'Add review comment' : 'Start a review'}
        </Button>
      </div>
    </div>
  );
}

function ReviewPopover({ pr, repo, pendingCount, busy, error, onClose, onSubmit, onDiscard }: { pr: Pull; repo: Repo; pendingCount: number; busy: boolean; error: unknown; onClose: () => void; onSubmit: (body: string, event: Event) => void; onDiscard?: () => void }) {
  const { user } = useSession();
  const own = !!user && user.id === pr.user.id;
  const [body, setBody] = useState('');
  const [event, setEvent] = useState<Event>('COMMENT');
  return (
    <div className="review-popover">
      <div className="review-popover-head">
        <strong>Finish your review</strong>
        <button className="linkish muted" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </div>
      <MarkdownEditor value={body} onChange={setBody} repo={repo} placeholder="Leave a comment" minHeight={90} autoFocus />
      {error ? <ErrorFlash error={error} /> : null}
      <RadioGroup name="review-event" onChange={(v) => setEvent((v as Event) ?? 'COMMENT')}>
        <RadioGroup.Label visuallyHidden>Review type</RadioGroup.Label>
        <label className="review-option">
          <Radio value="COMMENT" checked={event === 'COMMENT'} />
          <span>
            <strong>Comment</strong>
            <span className="muted small">Submit general feedback without explicit approval.</span>
          </span>
        </label>
        <label className={`review-option${own ? ' disabled' : ''}`}>
          <Radio value="APPROVE" checked={event === 'APPROVE'} disabled={own} />
          <span>
            <strong>Approve</strong>
            <span className="muted small">{own ? "Pull request authors can't approve their own pull request." : 'Submit feedback and approve merging these changes.'}</span>
          </span>
        </label>
        <label className={`review-option${own ? ' disabled' : ''}`}>
          <Radio value="REQUEST_CHANGES" checked={event === 'REQUEST_CHANGES'} disabled={own} />
          <span>
            <strong>Request changes</strong>
            <span className="muted small">{own ? "Pull request authors can't request changes on their own pull request." : 'Submit feedback that must be addressed before merging.'}</span>
          </span>
        </label>
      </RadioGroup>
      <div className="rc-actions">
        {onDiscard ? (
          <Button size="small" variant="danger" onClick={onDiscard}>
            Abandon review
          </Button>
        ) : null}
        <span className="grow" />
        <Button size="small" variant="primary" loading={busy} disabled={event !== 'APPROVE' && !body.trim() && pendingCount === 0} onClick={() => onSubmit(body, event)}>
          Submit review{pendingCount ? ` (${pendingCount})` : ''}
        </Button>
      </div>
    </div>
  );
}
