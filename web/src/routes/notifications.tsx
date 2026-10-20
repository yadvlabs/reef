import { Avatar, Button, RelativeTime } from '@primer/react';
import { IssueOpenedIcon, GitPullRequestIcon, PlayIcon, CheckIcon, XIcon, MentionIcon, PersonIcon, EyeIcon, CommentIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { Loading } from '../components/Loading';
import { reef, type User } from '../lib/api';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/notifications')({
  validateSearch: (s: Record<string, unknown>): { all?: boolean } => (s.all === true ? { all: true } : {}),
  component: Notifications,
});

interface Notification {
  id: number;
  subject_type: string;
  subject_id: number | null;
  reason: string;
  title: string;
  url: string;
  unread: boolean;
  repository: string | null;
  actor: User | null;
  updated_at: string;
}

const REASONS: Record<string, { label: string; icon: typeof MentionIcon }> = {
  mention: { label: 'mentioned you', icon: MentionIcon },
  assign: { label: 'assigned you', icon: PersonIcon },
  review_requested: { label: 'requested your review', icon: EyeIcon },
  comment: { label: 'commented', icon: CommentIcon },
  state_change: { label: 'changed the state', icon: CheckIcon },
  review: { label: 'reviewed', icon: EyeIcon },
  ci_activity: { label: 'workflow failed', icon: PlayIcon },
};

function SubjectIcon({ t }: { t: string }) {
  if (t === 'pull') return <GitPullRequestIcon fill="var(--fgColor-success)" />;
  if (t === 'run') return <PlayIcon fill="var(--fgColor-danger)" />;
  return <IssueOpenedIcon fill="var(--fgColor-success)" />;
}

function Notifications() {
  const { all } = Route.useSearch();
  const { user, loading } = useSession();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['notifications', !!all], queryFn: () => reef<{ unread_count: number; notifications: Notification[] }>(`/notifications?all=${all ? 'true' : 'false'}&per_page=100`), refetchInterval: 30_000 });
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ['notifications'] });
    qc.invalidateQueries({ queryKey: ['session'] });
  };
  const markAll = useMutation({ mutationFn: () => reef('/notifications', { method: 'POST' }), onSuccess: invalidate });
  const markOne = useMutation({ mutationFn: (id: number) => reef(`/notifications/${id}`, { method: 'PATCH' }), onSuccess: invalidate });
  const remove = useMutation({ mutationFn: (id: number) => reef(`/notifications/${id}`, { method: 'DELETE' }), onSuccess: invalidate });
  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: '/notifications' } });
    return null;
  }
  const list = q.data?.notifications ?? [];
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <div className="notif-head">
          <h1>Notifications</h1>
          <div className="release-tabs" style={{ margin: 0 }}>
            <button className={`tab${!all ? ' active' : ''}`} onClick={() => navigate({ to: '/notifications', search: {} })}>
              Unread {q.data ? <span className="counter">{q.data.unread_count}</span> : null}
            </button>
            <button className={`tab${all ? ' active' : ''}`} onClick={() => navigate({ to: '/notifications', search: { all: true } })}>
              All
            </button>
          </div>
          <span style={{ flex: 1 }} />
          <Button size="small" leadingVisual={CheckIcon} onClick={() => markAll.mutate()} disabled={!q.data?.unread_count}>
            Mark all as read
          </Button>
        </div>
        {q.isPending ? <Loading /> : null}
        {list.length === 0 && q.data ? (
          <Blankslate spacious border>
            <Blankslate.Visual>
              <CheckIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>All caught up</Blankslate.Heading>
            <Blankslate.Description>You'll be notified here when someone mentions you, assigns you, requests your review, or updates something you take part in.</Blankslate.Description>
          </Blankslate>
        ) : null}
        <div className="notif-list">
          {list.map((n) => {
            const r = REASONS[n.reason] ?? { label: n.reason, icon: CommentIcon };
            const Icon = r.icon;
            return (
              <div key={n.id} className={`notif${n.unread ? ' unread' : ''}`}>
                <SubjectIcon t={n.subject_type} />
                <div className="notif-main">
                  <a
                    href={n.url}
                    onClick={(e) => {
                      e.preventDefault();
                      markOne.mutate(n.id);
                      navigate({ href: n.url });
                    }}
                    className="notif-title"
                  >
                    {n.title}
                  </a>
                  <div className="muted small" style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                    {n.actor ? (
                      <>
                        <Avatar src={n.actor.avatar_url} size={16} alt="" /> {n.actor.login}
                      </>
                    ) : null}
                    <Icon size={12} /> {r.label}
                    {n.repository ? <span>· {n.repository}</span> : null}
                    <span>
                      · <RelativeTime datetime={n.updated_at} />
                    </span>
                  </div>
                </div>
                <div className="notif-actions">
                  {n.unread ? (
                    <button className="linkish muted" aria-label="Mark as read" title="Mark as read" onClick={() => markOne.mutate(n.id)}>
                      <CheckIcon />
                    </button>
                  ) : null}
                  <button className="linkish muted" aria-label="Remove" title="Remove" onClick={() => remove.mutate(n.id)}>
                    <XIcon />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </>
  );
}
