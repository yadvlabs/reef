// A review thread (root line comment + replies) as GitHub renders it in the conversation
// (with file header and diff excerpt) and inline in the diff (comments only).
import { Avatar, Button, RelativeTime } from '@primer/react';
import { CheckIcon, FileIcon, KebabHorizontalIcon } from '@primer/octicons-react';
import { ActionList, ActionMenu } from '@primer/react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo } from '../../lib/api';
import { type Pull, type ReviewComment, type Thread } from '../../lib/pulls';
import { useSession } from '../../lib/session';
import { MarkdownEditor } from '../IssueBits';
import { Markdown } from '../Markdown';

export function useThreadMutations(repo: Repo, pr: Pull) {
  const qc = useQueryClient();
  const full = repo.full_name;
  const refresh = () => {
    qc.invalidateQueries({ queryKey: ['pulls', full.toLowerCase(), pr.number] });
    qc.invalidateQueries({ queryKey: ['issues', full.toLowerCase(), pr.number] });
  };
  const reply = useMutation({ mutationFn: (v: { root: number; body: string }) => api(`/repos/${full}/pulls/${pr.number}/comments`, { method: 'POST', json: { body: v.body, in_reply_to: v.root, path: '' } }), onSuccess: refresh });
  const resolve = useMutation({ mutationFn: (v: { id: number; resolved: boolean }) => api(`/repos/${full}/pulls/comments/${v.id}/resolve`, { method: v.resolved ? 'PUT' : 'DELETE' }), onSuccess: refresh });
  const edit = useMutation({ mutationFn: (v: { id: number; body: string }) => api(`/repos/${full}/pulls/comments/${v.id}`, { method: 'PATCH', json: { body: v.body } }), onSuccess: refresh });
  const remove = useMutation({ mutationFn: (id: number) => api(`/repos/${full}/pulls/comments/${id}`, { method: 'DELETE' }), onSuccess: refresh });
  return { reply, resolve, edit, remove, refresh };
}

function HunkExcerpt({ hunk }: { hunk: string }) {
  const lines = hunk.split('\n').filter((l) => l.length > 0);
  if (lines.length === 0) return null;
  return (
    <table className="diff-table excerpt">
      <tbody>
        {lines.map((l, i) => (
          <tr key={i} className={l.startsWith('@@') ? 'hunk' : l.startsWith('+') ? 'add' : l.startsWith('-') ? 'del' : 'ctx'}>
            <td className="code">{l}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ThreadComment({ c, repo, m }: { c: ReviewComment; repo: Repo; m: ReturnType<typeof useThreadMutations> }) {
  const { user } = useSession();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.body);
  const mine = !!user && user.id === c.user?.id;
  const canEdit = mine || !!repo.permissions?.push;
  return (
    <div className={`review-comment${c.pending ? ' pending' : ''}`} id={`discussion_r${c.id}`}>
      <Avatar src={c.user?.avatar_url ?? ''} size={24} alt="" />
      <div className="rc-body">
        <div className="rc-head">
          {c.user ? (
            <Link to="/$owner" params={{ owner: c.user.login }} className="author">
              {c.user.login}
            </Link>
          ) : (
            <strong>someone</strong>
          )}
          <span className="muted">
            <RelativeTime datetime={c.created_at} />
          </span>
          {c.author_association && c.author_association !== 'NONE' ? <span className="assoc">{c.author_association.charAt(0) + c.author_association.slice(1).toLowerCase()}</span> : null}
          {c.pending ? <span className="pending-pill">Pending</span> : null}
          {canEdit ? (
            <span className="rc-menu">
              <ActionMenu>
                <ActionMenu.Anchor>
                  <button className="linkish muted" aria-label="Comment options">
                    <KebabHorizontalIcon />
                  </button>
                </ActionMenu.Anchor>
                <ActionMenu.Overlay align="end">
                  <ActionList>
                    <ActionList.Item onSelect={() => setEditing(true)}>Edit</ActionList.Item>
                    <ActionList.Item variant="danger" onSelect={() => confirm('Delete this comment?') && m.remove.mutate(c.id)}>
                      Delete
                    </ActionList.Item>
                  </ActionList>
                </ActionMenu.Overlay>
              </ActionMenu>
            </span>
          ) : null}
        </div>
        {editing ? (
          <div>
            <MarkdownEditor value={draft} onChange={setDraft} repo={repo} minHeight={80} />
            <div className="rc-actions">
              <Button size="small" onClick={() => setEditing(false)}>
                Cancel
              </Button>
              <Button
                size="small"
                variant="primary"
                onClick={() =>
                  m.edit.mutate(
                    { id: c.id, body: draft },
                    {
                      onSuccess: () => setEditing(false),
                    },
                  )
                }
              >
                Update comment
              </Button>
            </div>
          </div>
        ) : (
          <Markdown html={c.body_html} />
        )}
      </div>
    </div>
  );
}

export function ReviewThread({ thread, repo, pr, showFile, compact }: { thread: Thread; repo: Repo; pr: Pull; showFile?: boolean; compact?: boolean }) {
  const { user } = useSession();
  const m = useThreadMutations(repo, pr);
  const [replying, setReplying] = useState(false);
  const [body, setBody] = useState('');
  const [open, setOpen] = useState(!thread.root.resolved);
  const root = thread.root;
  const line = root.start_line && root.start_line !== root.line ? `Lines ${root.start_line} to ${root.line}` : `Line ${root.line ?? ''}`;
  return (
    <div className={`review-thread${compact ? ' compact' : ''}${root.resolved ? ' resolved' : ''}`}>
      {showFile || root.resolved ? (
        <div className="thread-file">
          {showFile ? (
            <>
              <FileIcon className="muted" />
              <Link to="/$owner/$repo/pull/$number/files" params={{ owner: repo.owner.login, repo: repo.name, number: String(pr.number) }} hash={`diff-${root.path}`} className="monospace">
                {root.path}
              </Link>
              <span className="muted small">{line}</span>
            </>
          ) : (
            <span className="muted small">
              {root.user?.login ?? 'someone'} started a conversation
            </span>
          )}
          {root.resolved ? (
            <span className="resolved-pill">
              <CheckIcon size={12} /> Resolved
            </span>
          ) : null}
          {root.resolved ? (
            <button className="linkish small" style={{ marginLeft: 'auto' }} onClick={() => setOpen(!open)}>
              {open ? 'Hide resolved' : 'Show resolved'}
            </button>
          ) : null}
        </div>
      ) : null}
      {open || !root.resolved ? (
        <>
          {showFile && root.diff_hunk ? <HunkExcerpt hunk={root.diff_hunk} /> : null}
          <div className="thread-comments">
            <ThreadComment c={root} repo={repo} m={m} />
            {thread.replies.map((c) => (
              <ThreadComment key={c.id} c={c} repo={repo} m={m} />
            ))}
          </div>
          {user ? (
            <div className="thread-reply">
              {replying ? (
                <div>
                  <MarkdownEditor value={body} onChange={setBody} repo={repo} placeholder="Reply…" minHeight={70} autoFocus />
                  <div className="rc-actions">
                    <Button
                      size="small"
                      onClick={() => {
                        setReplying(false);
                        setBody('');
                      }}
                    >
                      Cancel
                    </Button>
                    <Button
                      size="small"
                      variant="primary"
                      disabled={!body.trim()}
                      loading={m.reply.isPending}
                      onClick={() =>
                        m.reply.mutate(
                          { root: root.id, body },
                          {
                            onSuccess: () => {
                              setBody('');
                              setReplying(false);
                            },
                          },
                        )
                      }
                    >
                      Comment
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="thread-reply-row">
                  <Avatar src={user.avatar_url} size={24} alt="" />
                  <button className="reply-input" onClick={() => setReplying(true)}>
                    Reply…
                  </button>
                  {!root.pending ? (
                    <Button size="small" onClick={() => m.resolve.mutate({ id: root.id, resolved: !root.resolved })}>
                      {root.resolved ? 'Unresolve conversation' : 'Resolve conversation'}
                    </Button>
                  ) : null}
                </div>
              )}
            </div>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
