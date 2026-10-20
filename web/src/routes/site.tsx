import { Avatar, Button, FormControl, TextInput } from '@primer/react';
import { PeopleIcon, OrganizationIcon, RepoIcon, PlayIcon, GearIcon, ServerIcon, HomeIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { StatusIcon } from '../components/ActionsBits';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading } from '../components/Loading';
import { reef, type Repo, type User } from '../lib/api';
import type { WorkflowRun } from '../lib/actions';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/site')({
  validateSearch: (s: Record<string, unknown>): { tab?: string; q?: string } => ({ ...(typeof s.tab === 'string' ? { tab: s.tab } : {}), ...(typeof s.q === 'string' ? { q: s.q } : {}) }),
  component: Admin,
});

type Tab = 'overview' | 'users' | 'orgs' | 'repos' | 'actions' | 'settings';
const TABS: { key: Tab; label: string; icon: typeof HomeIcon }[] = [
  { key: 'overview', label: 'Overview', icon: HomeIcon },
  { key: 'users', label: 'Users', icon: PeopleIcon },
  { key: 'orgs', label: 'Organizations', icon: OrganizationIcon },
  { key: 'repos', label: 'Repositories', icon: RepoIcon },
  { key: 'actions', label: 'Actions', icon: PlayIcon },
  { key: 'settings', label: 'Settings', icon: GearIcon },
];

interface AdminUser extends User {
  suspended: boolean;
  repos: number;
  orgs: string[];
  created_at: string;
}
interface AdminOrg extends User {
  members: number;
  owners: string[];
  repos: number;
  created_at: string;
}

function Admin() {
  const { tab = 'overview' } = Route.useSearch();
  const { user, loading } = useSession();
  const navigate = useNavigate();
  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: '/site' } });
    return null;
  }
  if (!user.site_admin) return <><AppHeader /><div className="page"><ErrorFlash error="Site administrators only." /></div></>;
  return (
    <>
      <AppHeader />
      <div className="page">
        <div className="settings-layout">
          <nav className="settings-nav">
            <div className="muted small" style={{ padding: '4px 8px 8px', fontWeight: 600 }}>Site administration</div>
            {TABS.map((t) => (
              <Link key={t.key} to="/site" search={{ tab: t.key }} className={tab === t.key ? 'active' : ''}>
                <t.icon /> {t.label}
              </Link>
            ))}
          </nav>
          <div>{tab === 'overview' ? <Overview /> : tab === 'users' ? <Users /> : tab === 'orgs' ? <Orgs /> : tab === 'repos' ? <Repos /> : tab === 'actions' ? <Actions /> : <Settings />}</div>
        </div>
      </div>
    </>
  );
}

function Overview() {
  const q = useQuery({ queryKey: ['admin', 'overview'], queryFn: () => reef<{ counts: Record<string, number>; recent_users: AdminUser[]; recent_repos: Repo[]; version: string; database: string; data_dir: string; site_url: string; oidc: { issuer: string; client_id: string; name: string; only: boolean } | null }>('/admin/overview') });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const d = q.data;
  const cards: [string, string, Tab][] = [['Users', 'users', 'users'], ['Organizations', 'orgs', 'orgs'], ['Repositories', 'repos', 'repos'], ['Issues and PRs', 'issues', 'repos'], ['Workflow runs', 'runs', 'actions'], ['Runners', 'runners', 'actions'], ['Projects', 'projects', 'orgs'], ['Releases', 'releases', 'repos']];
  return (
    <div className="form-stack">
      <h2 className="section-title">Overview</h2>
      <div className="stat-grid">
        {cards.map(([label, key, tab]) => (
          <Link key={key} to="/site" search={{ tab }} className="stat">
            <span className="stat-n">{d.counts[key] ?? 0}</span>
            <span className="muted small">{label}</span>
          </Link>
        ))}
      </div>
      <div className="home-grid">
        <section>
          <h3 className="overview-title">Newest users</h3>
          <div className="key-list">
            {d.recent_users.map((u) => (
              <div key={u.id} className="item" style={{ alignItems: 'center' }}>
                <Avatar src={u.avatar_url} size={28} alt="" />
                <div className="body">
                  <Link to="/$owner" params={{ owner: u.login }}>
                    <strong>{u.login}</strong>
                  </Link>{' '}
                  <span className="muted small">{u.email}</span>
                  {u.site_admin ? <span className="visibility-pill" style={{ marginLeft: 6 }}>admin</span> : null}
                </div>
              </div>
            ))}
          </div>
          <h3 className="overview-title" style={{ marginTop: 16 }}>Newest repositories</h3>
          <div className="key-list">
            {d.recent_repos.map((r) => (
              <div key={r.id} className="item" style={{ alignItems: 'center' }}>
                <div className="body">
                  <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
                    {r.full_name}
                  </Link>{' '}
                  <span className="visibility-pill">{r.private ? 'Private' : 'Public'}</span>
                </div>
              </div>
            ))}
          </div>
        </section>
        <aside>
          <h3 className="overview-title">Instance</h3>
          <div className="card form-stack" style={{ gap: 6, fontSize: 14 }}>
            <div>
              <span className="muted">Version</span> {d.version}
            </div>
            <div>
              <span className="muted">Database</span> {d.database}
            </div>
            <div>
              <span className="muted">Data directory</span> <code>{d.data_dir}</code>
            </div>
            <div>
              <span className="muted">Site URL</span> {d.site_url}
            </div>
            <div>
              <span className="muted">Identity</span> {d.oidc ? `${d.oidc.name} (${d.oidc.issuer}, client ${d.oidc.client_id}${d.oidc.only ? ', exclusive' : ''})` : 'native accounts only'}
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}

function Users() {
  const { user: me } = useSession();
  const qc = useQueryClient();
  const [q, setQ] = useState('');
  const [error, setError] = useState<unknown>(null);
  const list = useQuery({ queryKey: ['admin', 'users', q], queryFn: () => reef<{ total: number; users: AdminUser[] }>(`/admin/users?q=${encodeURIComponent(q)}&per_page=200`) });
  const patch = useMutation({ mutationFn: ({ login, body }: { login: string; body: Record<string, unknown> }) => reef(`/admin/users/${login}`, { method: 'PATCH', json: body }), onSuccess: () => qc.invalidateQueries({ queryKey: ['admin'] }), onError: setError });
  return (
    <div className="form-stack">
      <h2 className="section-title">Users {list.data ? <span className="counter">{list.data.total}</span> : null}</h2>
      {error ? <ErrorFlash error={error} /> : null}
      <TextInput value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search by login, name or email" block />
      <div className="key-list">
        {list.data?.users.map((u) => (
          <div key={u.id} className="item" style={{ alignItems: 'center' }}>
            <Avatar src={u.avatar_url} size={32} alt="" />
            <div className="body">
              <Link to="/$owner" params={{ owner: u.login }}>
                <strong>{u.login}</strong>
              </Link>
              {u.site_admin ? <span className="visibility-pill" style={{ marginLeft: 6 }}>site admin</span> : null}
              {u.suspended ? <span className="visibility-pill" style={{ marginLeft: 6, color: 'var(--fgColor-danger)' }}>suspended</span> : null}
              <div className="muted small">
                {u.name ? `${u.name} · ` : ''}{u.email} · {u.repos} repos{u.orgs.length ? ` · ${u.orgs.join(', ')}` : ''} · joined {new Date(u.created_at).toLocaleDateString()}
              </div>
            </div>
            {u.login !== me?.login ? (
              <>
                <Button size="small" onClick={() => patch.mutate({ login: u.login, body: { site_admin: !u.site_admin } })}>
                  {u.site_admin ? 'Remove admin' : 'Make admin'}
                </Button>
                <Button size="small" variant={u.suspended ? 'default' : 'danger'} onClick={() => confirm(`${u.suspended ? 'Unsuspend' : 'Suspend'} ${u.login}?`) && patch.mutate({ login: u.login, body: { suspended: !u.suspended } })}>
                  {u.suspended ? 'Unsuspend' : 'Suspend'}
                </Button>
                <Button
                  size="small"
                  onClick={() => {
                    const pw = prompt(`New password for ${u.login}`);
                    if (pw) patch.mutate({ login: u.login, body: { password: pw } });
                  }}
                >
                  Set password
                </Button>
                <Button
                  size="small"
                  onClick={() => {
                    const next = prompt(`New username for ${u.login}`, u.login);
                    if (next && next.trim() !== u.login) patch.mutate({ login: u.login, body: { login: next.trim() } });
                  }}
                >
                  Rename
                </Button>
              </>
            ) : (
              <span className="muted small">you</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function Orgs() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['admin', 'orgs'], queryFn: () => reef<{ total: number; orgs: AdminOrg[] }>('/admin/orgs') });
  const [error, setError] = useState<unknown>(null);
  const renameOrg = useMutation({
    mutationFn: ({ login, next }: { login: string; next: string }) => reef(`/admin/orgs/${login}`, { method: 'PATCH', json: { login: next } }),
    onSuccess: () => {
      setError(null);
      qc.invalidateQueries({ queryKey: ['admin'] });
    },
    onError: setError,
  });
  return (
    <div className="form-stack">
      <h2 className="section-title">Organizations {q.data ? <span className="counter">{q.data.total}</span> : null}</h2>
      {error ? <ErrorFlash error={error} /> : null}
      <div className="key-list">
        {q.data?.orgs.map((o) => (
          <div key={o.id} className="item" style={{ alignItems: 'center' }}>
            <Avatar src={o.avatar_url} size={32} square alt="" />
            <div className="body">
              <Link to="/$owner" params={{ owner: o.login }}>
                <strong>{o.login}</strong>
              </Link>{' '}
              <span className="muted small">{o.name}</span>
              <div className="muted small">
                {o.members} members · owners: {o.owners.join(', ') || 'none'} · {o.repos} repos · created {new Date(o.created_at).toLocaleDateString()}
              </div>
            </div>
            <Button
              size="small"
              onClick={() => {
                const next = prompt(`New name for organization ${o.login}`, o.login);
                if (next && next.trim() !== o.login) renameOrg.mutate({ login: o.login, next: next.trim() });
              }}
            >
              Rename
            </Button>
            <Link to="/organizations/$org/settings" params={{ org: o.login }} search={{}} className="small">
              Settings
            </Link>
          </div>
        ))}
        {q.data?.orgs.length === 0 ? <p className="muted" style={{ padding: 16 }}>No organizations.</p> : null}
      </div>
    </div>
  );
}

function Repos() {
  const [q, setQ] = useState('');
  const list = useQuery({ queryKey: ['admin', 'repos', q], queryFn: () => reef<{ total: number; repos: (Repo & { size_kb: number })[] }>(`/admin/repos?q=${encodeURIComponent(q)}&per_page=200`) });
  return (
    <div className="form-stack">
      <h2 className="section-title">Repositories {list.data ? <span className="counter">{list.data.total}</span> : null}</h2>
      <TextInput value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search by owner/name" block />
      <div className="key-list">
        {list.data?.repos.map((r) => (
          <div key={r.id} className="item" style={{ alignItems: 'center' }}>
            <div className="body">
              <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
                <strong>{r.full_name}</strong>
              </Link>{' '}
              <span className="visibility-pill">{r.private ? 'Private' : 'Public'}</span>
              <div className="muted small">
                {r.description ?? ''} · {Math.round(r.size_kb / 1024)} MB · {r.open_issues_count} open issues · updated {new Date(r.updated_at).toLocaleDateString()}
              </div>
            </div>
            <Link to="/$owner/$repo/settings" params={{ owner: r.owner.login, repo: r.name }} className="small">
              Settings
            </Link>
          </div>
        ))}
      </div>
    </div>
  );
}

function Actions() {
  const runners = useQuery({ queryKey: ['admin', 'runners'], queryFn: () => reef<{ runners: { id: number; name: string; scope: string; scope_kind: string; os: string | null; version: string | null; labels: string[]; online: boolean; busy: boolean; ephemeral: boolean; last_seen_at: string | null }[]; queued_jobs: number }>('/admin/runners'), refetchInterval: 10_000 });
  const runs = useQuery({ queryKey: ['admin', 'runs'], queryFn: () => reef<{ runs: (WorkflowRun & { repository: Repo })[] }>('/admin/runs'), refetchInterval: 10_000 });
  return (
    <div className="form-stack">
      <h2 className="section-title">
        <ServerIcon /> Runners {runners.data ? <span className="muted small">· {runners.data.queued_jobs} jobs queued</span> : null}
      </h2>
      <div className="key-list">
        {runners.data?.runners.map((r) => (
          <div key={r.id} className="item" style={{ alignItems: 'center' }}>
            <span className={`runner-dot ${r.online ? 'online' : ''}`} />
            <div className="body">
              <strong>{r.name}</strong>{' '}
              <span className="muted small">
                {r.scope_kind === 'org' ? 'organization' : 'repository'} {r.scope} · {r.os ?? ''} {r.version ? `v${r.version}` : ''} · {r.busy ? 'active' : r.online ? 'idle' : 'offline'}
                {r.ephemeral ? ' · ephemeral' : ''}
              </span>
              <div style={{ display: 'flex', gap: 4, marginTop: 4, flexWrap: 'wrap' }}>
                {r.labels.map((l) => (
                  <span key={l} className="visibility-pill">
                    {l}
                  </span>
                ))}
              </div>
            </div>
          </div>
        ))}
        {runners.data?.runners.length === 0 ? <p className="muted" style={{ padding: 16 }}>No runners registered. Jobs stay queued until one picks them up.</p> : null}
      </div>
      <h2 className="section-title">Recent workflow runs</h2>
      <div className="run-list">
        {runs.data?.runs.map((r) => (
          <div key={r.id} className="run-row">
            <StatusIcon status={r.status} conclusion={r.conclusion} />
            <div className="run-main">
              <a href={`/${r.repository.full_name}/actions/runs/${r.id}`} className="run-title">
                {r.display_title}
              </a>
              <div className="muted small">
                {r.repository.full_name} · {r.name} #{r.run_number} · {r.event}
              </div>
            </div>
            <div />
            <div className="run-time muted small">{new Date(r.created_at).toLocaleString()}</div>
          </div>
        ))}
      </div>
    </div>
  );
}

function Settings() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['admin', 'settings'], queryFn: () => reef<{ settings: { key: string; description: string; value: string; overridden: boolean }[] }>('/admin/settings') });
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [error, setError] = useState<unknown>(null);
  const save = useMutation({
    mutationFn: (values: Record<string, string | null>) => reef('/admin/settings', { method: 'PUT', json: { values } }),
    onSuccess: () => {
      setDraft({});
      qc.invalidateQueries({ queryKey: ['admin', 'settings'] });
      qc.invalidateQueries({ queryKey: ['session'] });
    },
    onError: setError,
  });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  return (
    <div className="form-stack">
      <h2 className="section-title">Settings</h2>
      <p className="muted small">Values set here override the configuration file until reset. Listeners, storage, and identity provider settings stay in the file or environment.</p>
      {error ? <ErrorFlash error={error} /> : null}
      <form
        className="form-stack"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate(draft);
        }}
      >
        {q.data.settings.map((s) => (
          <FormControl key={s.key}>
            <FormControl.Label>
              {s.key} {s.overridden ? <span className="visibility-pill" style={{ marginLeft: 6 }}>overridden</span> : null}
            </FormControl.Label>
            {s.key === 'allow_signup' ? (
              <select className="select" value={draft[s.key] ?? s.value} onChange={(e) => setDraft({ ...draft, [s.key]: e.target.value })}>
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            ) : (
              <TextInput block value={draft[s.key] ?? s.value} onChange={(e) => setDraft({ ...draft, [s.key]: e.target.value })} />
            )}
            <FormControl.Caption>
              {s.description}
              {s.overridden ? (
                <>
                  {' '}
                  <button type="button" className="linkish small" onClick={() => save.mutate({ [s.key]: null })}>
                    reset to file value
                  </button>
                </>
              ) : null}
            </FormControl.Caption>
          </FormControl>
        ))}
        <div>
          <Button type="submit" variant="primary" disabled={Object.keys(draft).length === 0} loading={save.isPending}>
            Save changes
          </Button>
        </div>
      </form>
    </div>
  );
}
