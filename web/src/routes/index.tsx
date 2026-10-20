import { ActionList, ActionMenu, Avatar, RelativeTime } from '@primer/react';
import { RepoIcon, LockIcon, ChevronDownIcon, OrganizationIcon, PersonIcon, GitCommitIcon, IssueOpenedIcon, GitPullRequestIcon, TagIcon, StarIcon, RepoForkedIcon, PlayIcon, BookIcon, GitBranchIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link, createFileRoute } from '@tanstack/react-router';
import { useEffect, useState, type ReactElement } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ButtonLink } from '../components/links';
import { Loading } from '../components/Loading';
import { Markdown } from '../components/Markdown';
import { Logo, Wordmark } from '../components/Logo';
import { CrustyEmpty } from '../components/Crusty';
import { api, reef, type Repo, type User } from '../lib/api';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/')({ component: Home });

function Home() {
  const { user, instance, loading } = useSession();
  if (loading) return <Loading />;
  return (
    <>
      <AppHeader dashboard={!!user} />
      {user ? <Dashboard /> : <Landing name={instance?.name ?? 'Reef'} allowSignup={instance?.allow_signup ?? false} oidc={instance?.oidc?.name} />}
    </>
  );
}

function Landing({ name, allowSignup, oidc }: { name: string; allowSignup: boolean; oidc?: string }) {
  return (
    <div className="page landing">
      <Wordmark height={64} />
      <h1>{name}</h1>
      <p className="muted lead">Code hosting, reviews, Actions, projects, and wikis. Works with the GitHub tools you already use.</p>
      <div style={{ display: 'flex', gap: 8, justifyContent: 'center' }}>
        <ButtonLink to="/login" variant="primary" size="large">
          {oidc ? `Sign in with ${oidc}` : 'Sign in'}
        </ButtonLink>
        {allowSignup ? (
          <ButtonLink to="/join" size="large">
            Create an account
          </ButtonLink>
        ) : null}
      </div>
      <div className="landing-grid">
        {[
          ['Repositories', 'Git over HTTPS and SSH, branch protection, releases, wikis.'],
          ['Issues and pull requests', 'Reviews, labels, milestones, and a GitHub-compatible API and GraphQL.'],
          ['Actions', 'Workflows run on stock actions/runner binaries you register with a token.'],
          ['Projects', 'Boards with swimlanes, tables, saved views, and workflows tied to issues.'],
        ].map(([t, d]) => (
          <div key={t} className="landing-card">
            <strong>{t}</strong>
            <span className="muted small">{d}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

interface FeedEvent {
  id: number;
  kind: string;
  payload: Record<string, unknown>;
  repository: { full_name: string; owner: string; name: string; private: boolean; description: string | null };
  actor: User | null;
  created_at: string;
}

function eventLine(e: FeedEvent): { icon: ReactElement; text: string; href: string } | null {
  const p = e.payload;
  const repo = e.repository.full_name;
  const num = typeof p.number === 'number' ? p.number : null;
  switch (e.kind) {
    case 'push': {
      const ref = String(p.ref ?? '').replace('refs/heads/', '').replace('refs/tags/', '');
      return { icon: <GitCommitIcon />, text: `pushed to ${ref} in ${repo}`, href: `/${repo}/commits/${encodeURIComponent(ref)}` };
    }
    case 'create': {
      const ref = String(p.ref ?? '');
      const isTag = ref.startsWith('refs/tags/');
      const name = ref.replace(/^refs\/(heads|tags)\//, '');
      return { icon: isTag ? <TagIcon /> : <GitBranchIcon />, text: `created ${isTag ? 'tag' : 'branch'} ${name} in ${repo}`, href: `/${repo}/tree/${encodeURIComponent(name)}` };
    }
    case 'delete':
      return { icon: <GitBranchIcon />, text: `deleted ${String(p.ref ?? '').replace(/^refs\/(heads|tags)\//, '')} in ${repo}`, href: `/${repo}` };
    case 'issues.opened':
      return { icon: <IssueOpenedIcon fill="var(--fgColor-success)" />, text: `opened issue #${num} in ${repo}`, href: `/${repo}/issues/${num}` };
    case 'pull_request.opened':
      return { icon: <GitPullRequestIcon fill="var(--fgColor-success)" />, text: `opened pull request #${num} in ${repo}`, href: `/${repo}/pull/${num}` };
    case 'release.published':
      return { icon: <TagIcon />, text: `published release ${String(p.tag ?? '')} in ${repo}`, href: `/${repo}/releases/tag/${encodeURIComponent(String(p.tag ?? ''))}` };
    case 'release.drafted':
      return { icon: <TagIcon />, text: `drafted a release in ${repo}`, href: `/${repo}/releases` };
    case 'watch':
      return { icon: <StarIcon fill="var(--fgColor-attention)" />, text: `starred ${repo}`, href: `/${repo}` };
    case 'fork':
      return { icon: <RepoForkedIcon />, text: `forked ${repo} to ${String(p.forkee ?? '')}`, href: `/${String(p.forkee ?? repo)}` };
    case 'workflow_run.completed':
      return { icon: <PlayIcon fill={p.conclusion === 'success' ? 'var(--fgColor-success)' : 'var(--fgColor-danger)'} />, text: `${String(p.name ?? 'workflow')} run ${String(p.conclusion ?? '')} in ${repo}`, href: `/${repo}/actions/runs/${String(p.run_id ?? '')}` };
    case 'wiki':
      return { icon: <BookIcon />, text: `edited the wiki in ${repo}`, href: `/${repo}/wiki` };
    default:
      return null;
  }
}

interface ExploreOwner {
  owner: User;
  public_repos: number;
  repos: Repo[];
}

const OWNER_KEY = 'reef.dashboard.owner';

function Dashboard() {
  const { user, instance, orgs } = useSession();
  // Whose repositories the left column shows: you, or one of your organizations (as on GitHub).
  const choices: User[] = user ? [user, ...orgs] : orgs;
  const [ownerLogin, setOwnerLogin] = useState<string>(() => localStorage.getItem(OWNER_KEY) ?? user?.login ?? '');
  const owner = choices.find((c) => c.login === ownerLogin) ?? user;
  useEffect(() => {
    if (owner?.login) localStorage.setItem(OWNER_KEY, owner.login);
  }, [owner?.login]);
  const isMe = !owner || owner.login === user?.login;
  const repos = useQuery({
    queryKey: ['dashboard', 'repos', owner?.login ?? ''],
    queryFn: () => api<Repo[]>(isMe ? '/user/repos?sort=updated&per_page=100' : `/orgs/${encodeURIComponent(owner!.login)}/repos?sort=updated&per_page=100`),
  });
  const feed = useQuery({ queryKey: ['feed'], queryFn: () => reef<{ events: FeedEvent[] }>('/feed'), refetchInterval: 60_000 });
  const explore = useQuery({ queryKey: ['explore', 'owners'], queryFn: () => reef<ExploreOwner[]>('/explore/owners') });
  const [filter, setFilter] = useState('');
  const mine = (repos.data ?? []).filter((r) => r.full_name.toLowerCase().includes(filter.toLowerCase()));
  return (
    <div className="dashboard">
      <aside className="side">
        {choices.length > 1 ? (
          <ActionMenu>
            <ActionMenu.Anchor>
              <button type="button" className="owner-switch" aria-label="Choose whose repositories to show">
                <Avatar src={owner?.avatar_url ?? ''} size={20} square={owner?.type === 'Organization'} alt="" />
                <span>{owner?.login}</span>
                <ChevronDownIcon size={16} className="chev" />
              </button>
            </ActionMenu.Anchor>
            <ActionMenu.Overlay width="medium">
              <ActionList selectionVariant="single">
                {choices.map((c) => (
                  <ActionList.Item key={c.login} selected={c.login === owner?.login} onSelect={() => setOwnerLogin(c.login)}>
                    <ActionList.LeadingVisual>
                      <Avatar src={c.avatar_url} size={20} square={c.type === 'Organization'} alt="" />
                    </ActionList.LeadingVisual>
                    {c.login}
                    <ActionList.Description variant="inline">{c.login === user?.login ? 'you' : 'organization'}</ActionList.Description>
                  </ActionList.Item>
                ))}
              </ActionList>
            </ActionMenu.Overlay>
          </ActionMenu>
        ) : null}
        <h2>
          Top repositories
          <span className="spacer" />
          <ButtonLink to="/new" size="small" variant="primary" leadingVisual={RepoIcon}>
            New
          </ButtonLink>
        </h2>
        <input className="side-filter" placeholder="Find a repository…" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Find a repository" />
        {repos.isPending ? <Loading /> : null}
        {mine.length === 0 && repos.data ? <p className="muted small">{isMe ? 'No repositories yet.' : `No repositories in ${owner?.login} yet.`}</p> : null}
        {mine.slice(0, 20).map((r) => (
          <Link key={r.id} to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }} className="mini-repo">
            <Avatar src={r.owner.avatar_url} size={16} square={r.owner.type === 'Organization'} alt="" />
            <span>
              {r.owner.login}/<strong>{r.name}</strong>
            </span>
            {r.private ? <LockIcon className="muted" size={12} /> : null}
          </Link>
        ))}
        {mine.length > 20 ? (
          <Link to="/repositories" className="muted small" style={{ display: 'block', marginTop: 8 }}>
            Show all
          </Link>
        ) : null}
        {user?.site_admin ? (
          <div style={{ marginTop: 24 }}>
            <h2>Administration</h2>
            <ButtonLink to="/site" search={{}} size="small" block>
              Site admin console
            </ButtonLink>
          </div>
        ) : null}
      </aside>
      <main className="main">
        {instance?.announcement ? (
          <div className="announcement">
            <Markdown html={instance.announcement} />
          </div>
        ) : null}
        <h1 style={{ fontSize: 20, fontWeight: 600, margin: '0 0 16px' }}>Home</h1>
        <div className="home-grid">
          <section>
            <h2 className="overview-title">Latest activity</h2>
            {feed.isPending ? <Loading /> : null}
            {feed.data?.events.length === 0 ? <p className="muted">Activity in repositories you own, watch, or star shows up here.</p> : null}
            <div className="feed">
              {feed.data?.events.map((e) => {
                const line = eventLine(e);
                if (!line) return null;
                return (
                  <div key={e.id} className="feed-item">
                    {e.actor ? <Avatar src={e.actor.avatar_url} size={24} alt="" /> : <span className="feed-icon">{line.icon}</span>}
                    <div className="feed-body">
                      <span>
                        {e.actor ? (
                          <Link to="/$owner" params={{ owner: e.actor.login }} style={{ fontWeight: 600, color: 'var(--fgColor-default)' }}>
                            {e.actor.login}
                          </Link>
                        ) : (
                          <strong>Reef</strong>
                        )}{' '}
                        <a href={line.href}>{line.text}</a>
                      </span>
                      <span className="muted small">
                        {line.icon} <RelativeTime datetime={e.created_at} />
                      </span>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
          <aside className="home-right">
            <h2 className="overview-title">Explore repositories</h2>
            <p className="muted small" style={{ marginTop: -6, marginBottom: 8 }}>Every organization and person on this Reef, with their public repositories.</p>
            {explore.isPending ? <Loading /> : null}
            <div className="explore">
              {explore.data?.map((o) => (
                <div key={o.owner.login} className="explore-owner">
                  <div className="who">
                    <Avatar src={o.owner.avatar_url} size={20} square={o.owner.type === 'Organization'} alt="" />
                    <Link to="/$owner" params={{ owner: o.owner.login }}>
                      {o.owner.login}
                    </Link>
                    <span className="kind">{o.owner.type === 'Organization' ? <OrganizationIcon size={12} /> : <PersonIcon size={12} />}</span>
                    <span className="count">{o.public_repos} public</span>
                  </div>
                  <div className="repos">
                    {o.repos.map((r) => (
                      <div key={r.id}>
                        <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
                          <strong>{r.name}</strong>
                        </Link>
                        {r.stargazers_count ? <span className="muted small"> ★ {r.stargazers_count}</span> : null}
                        {r.description ? <div className="desc">{r.description}</div> : null}
                      </div>
                    ))}
                    {o.public_repos > o.repos.length ? (
                      <Link to="/$owner" params={{ owner: o.owner.login }} className="muted small">
                        and {o.public_repos - o.repos.length} more
                      </Link>
                    ) : null}
                  </div>
                </div>
              ))}
              {explore.data?.length === 0 ? <CrustyEmpty mood="builder" size={80} title="No public repositories yet." /> : null}
            </div>
            <div className="muted small" style={{ marginTop: 24, display: 'flex', gap: 8, alignItems: 'center' }}>
              <Logo size={16} /> Reef {instance?.version ? `v${instance.version}` : ''}
            </div>
          </aside>
        </div>
      </main>
    </div>
  );
}
