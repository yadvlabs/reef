import { Avatar, RelativeTime } from '@primer/react';
import { ButtonLink } from '../components/links';
import { RepoIcon, PeopleIcon, LocationIcon, LinkIcon, OrganizationIcon, MailIcon, GearIcon, ClockIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link, Outlet, useRouterState } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { ContributionGraph } from '../components/ContributionGraph';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { accountQuery, orgMembersQuery, OrgsBlock, OwnerTabs, useOwnerRole } from '../components/OwnerTabs';
import { PinnedRepos } from '../components/Pins';
import { LanguageDot, RepoList } from '../components/RepoList';
import { api, ApiError, type Repo, type User } from '../lib/api';
import { projectsQuery } from '../lib/projects';

export const Route = createFileRoute('/$owner')({
  validateSearch: (s: Record<string, unknown>): { tab?: string } => (typeof s.tab === 'string' ? { tab: s.tab } : {}),
  component: OwnerLayout,
});

function OwnerLayout() {
  const { owner } = Route.useParams();
  const depth = useRouterState({ select: (s) => s.location.pathname.split('/').filter(Boolean).length });
  if (depth > 1) return <Outlet />;
  return <Profile owner={owner} />;
}

type Tab = 'overview' | 'repositories' | 'people' | 'stars';

function Profile({ owner }: { owner: string }) {
  const { tab } = Route.useSearch();
  const account = useQuery(accountQuery(owner));
  const isOrg = account.data?.type === 'Organization';
  const repos = useQuery({
    queryKey: ['account', owner.toLowerCase(), 'repos'],
    queryFn: () => api<Repo[]>(`/${isOrg ? 'orgs' : 'users'}/${owner}/repos?sort=updated&per_page=100`),
    enabled: !!account.data,
  });
  const projects = useQuery({ ...projectsQuery(owner), enabled: !!account.data });
  if (account.isPending) return <><AppHeader /><Loading /></>;
  if (account.isError) return <><AppHeader />{account.error instanceof ApiError && account.error.status === 404 ? <NotFound what="user" /> : <ErrorFlash error={account.error} />}</>;
  const a = account.data;
  const current = (tab ?? 'overview') as Tab;
  return isOrg ? <OrgProfile a={a} current={current} repos={repos.data} projects={projects.data?.length} /> : <UserProfile a={a} current={current} repos={repos.data} projects={projects.data?.length} />;
}

// ----- organization -----

function OrgProfile({ a, current, repos, projects }: { a: User; current: Tab; repos?: Repo[]; projects?: number }) {
  const role = useOwnerRole(a.login);
  const members = useQuery(orgMembersQuery(a.login));
  const langs = topLanguages(repos ?? []);
  const topics = topTopics(repos ?? []);
  return (
    <>
      <AppHeader context={{ owner: a.login }} />
      <OwnerTabs account={a} current={current} counts={{ repos: repos?.length, members: members.data?.length, projects }} hideIdent />
      <div className="org-header">
        <div className="org-header-inner">
          <Avatar src={a.avatar_url} size={64} square alt={a.login} />
          <div className="org-ident">
            <h1>
              {a.name ?? a.login}
            </h1>
            {a.description ? <p>{a.description}</p> : null}
            <div className="org-meta">
              {a.location ? (
                <span>
                  <LocationIcon /> {a.location}
                </span>
              ) : null}
              {a.blog ? (
                <a href={a.blog}>
                  <LinkIcon /> {a.blog.replace(/^https?:\/\//, '')}
                </a>
              ) : null}
              {a.email ? (
                <a href={`mailto:${a.email}`}>
                  <MailIcon /> {a.email}
                </a>
              ) : null}
              {a.created_at ? (
                <span>
                  <ClockIcon /> Created <RelativeTime datetime={a.created_at} />
                </span>
              ) : null}
              {members.data ? (
                <Link to="/$owner" params={{ owner: a.login }} search={{ tab: 'people' }}>
                  <PeopleIcon /> {members.data.length} {members.data.length === 1 ? 'member' : 'members'}
                </Link>
              ) : null}
            </div>
          </div>
          <span style={{ flex: 1 }} />
          {role.member ? (
            <ButtonLink to="/organizations/$org/settings" params={{ org: a.login }} search={{}} leadingVisual={GearIcon} size="small">
              Settings
            </ButtonLink>
          ) : null}
        </div>
      </div>
      <div className="page">
        {current === 'people' ? (
          <PeopleList members={members.data ?? []} />
        ) : current === 'repositories' ? (
          <>
            {role.member ? (
              <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 8 }}>
                <ButtonLink to="/new" search={{ owner: a.login }} variant="primary" size="small" leadingVisual={RepoIcon}>
                  New repository
                </ButtonLink>
              </div>
            ) : null}
            <RepoList repos={repos ?? []} empty={`${a.login} doesn't have any public repositories yet.`} />
          </>
        ) : (
          <div className="org-overview">
            <main>
              <PinnedRepos owner={a.login} fallback={repos ?? []} canEdit={role.member} isOrg />
              {repos?.length ? (
                <section style={{ marginTop: 24 }}>
                  <h2 className="overview-title">Repositories</h2>
                  <RepoList repos={repos.slice(0, 10)} />
                  {repos.length > 10 ? (
                    <p style={{ marginTop: 8 }}>
                      <Link to="/$owner" params={{ owner: a.login }} search={{ tab: 'repositories' }}>
                        View all repositories
                      </Link>
                    </p>
                  ) : null}
                </section>
              ) : null}
            </main>
            <aside className="org-sidebar">
              <section>
                <h2>
                  People{' '}
                  {members.data ? (
                    <Link to="/$owner" params={{ owner: a.login }} search={{ tab: 'people' }} className="counter">
                      {members.data.length}
                    </Link>
                  ) : null}
                </h2>
                <div className="org-avatars">
                  {(members.data ?? []).slice(0, 20).map((m) => (
                    <Link key={m.id} to="/$owner" params={{ owner: m.login }} title={m.login}>
                      <Avatar src={m.avatar_url} size={32} alt={m.login} />
                    </Link>
                  ))}
                </div>
                {members.data && members.data.length > 20 ? (
                  <p className="small" style={{ marginTop: 8 }}>
                    <Link to="/$owner" params={{ owner: a.login }} search={{ tab: 'people' }}>
                      View all
                    </Link>
                  </p>
                ) : null}
              </section>
              {langs.length ? (
                <section>
                  <h2>Top languages</h2>
                  <div className="lang-chips">
                    {langs.map((l) => (
                      <span key={l} className="lang-chip">
                        <LanguageDot name={l} />
                      </span>
                    ))}
                  </div>
                </section>
              ) : null}
              {topics.length ? (
                <section>
                  <h2>Most used topics</h2>
                  <div className="topics">
                    {topics.map((t) => (
                      <Link key={t} to="/search" search={{ q: `topic:${t}` }} className="topic">
                        {t}
                      </Link>
                    ))}
                  </div>
                </section>
              ) : null}
              {projects ? (
                <section>
                  <h2>
                    Projects <span className="counter">{projects}</span>
                  </h2>
                  <p className="small">
                    <Link to="/$owner/projects" params={{ owner: a.login }} search={{}}>
                      View all projects
                    </Link>
                  </p>
                </section>
              ) : null}
            </aside>
          </div>
        )}
      </div>
    </>
  );
}

/** Company text with `@login` mentions linked to the account, like GitHub. */
function CompanyText({ text }: { text: string }) {
  const parts = text.split(/(@[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)/g);
  return (
    <>
      {parts.map((p, i) =>
        p.startsWith('@') ? (
          <Link key={i} to="/$owner" params={{ owner: p.slice(1) }} style={{ fontWeight: 600 }}>
            {p}
          </Link>
        ) : (
          <span key={i}>{p}</span>
        ),
      )}
    </>
  );
}

function topLanguages(repos: Repo[]): string[] {
  const counts = new Map<string, number>();
  for (const r of repos) if (r.language) counts.set(r.language, (counts.get(r.language) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([l]) => l).slice(0, 8);
}

function topTopics(repos: Repo[]): string[] {
  const counts = new Map<string, number>();
  for (const r of repos) for (const t of r.topics ?? []) counts.set(t, (counts.get(t) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([t]) => t).slice(0, 10);
}

function PeopleList({ members }: { members: User[] }) {
  return (
    <div className="key-list">
      {members.map((m) => (
        <div key={m.id} className="item" style={{ alignItems: 'center' }}>
          <Avatar src={m.avatar_url} size={40} alt="" />
          <div className="body">
            <Link to="/$owner" params={{ owner: m.login }}>
              <strong>{m.login}</strong>
            </Link>
            <div className="muted small">{m.name}</div>
          </div>
        </div>
      ))}
      {members.length === 0 ? <p className="muted" style={{ padding: 24 }}>No members.</p> : null}
    </div>
  );
}

// ----- user -----

function UserProfile({ a, current, repos, projects }: { a: User; current: Tab; repos?: Repo[]; projects?: number }) {
  const role = useOwnerRole(a.login);
  const starred = useQuery({ queryKey: ['account', a.login.toLowerCase(), 'starred'], queryFn: () => api<Repo[]>(`/users/${a.login}/starred?per_page=100`) });
  return (
    <>
      <AppHeader context={{ owner: a.login }} />
      <div className="page">
        <div className="profile-layout">
          <aside className="vcard">
            <Avatar src={a.avatar_url} size={260} alt={a.login} />
            <h1>{a.name ?? a.login}</h1>
            <div className="login">{a.login}</div>
            {a.bio ? <p>{a.bio}</p> : null}
            {role.self ? (
              <ButtonLink to="/settings" block style={{ marginTop: 16 }}>
                Edit profile
              </ButtonLink>
            ) : null}
            <div className="about" style={{ marginTop: 16 }}>
              <div className="meta">
                {a.company ? (
                  <span>
                    <OrganizationIcon /> <CompanyText text={a.company} />
                  </span>
                ) : null}
                {a.location ? (
                  <span>
                    <LocationIcon /> {a.location}
                  </span>
                ) : null}
                {a.email ? (
                  <a href={`mailto:${a.email}`}>
                    <MailIcon /> {a.email}
                  </a>
                ) : null}
                {a.blog ? (
                  <a href={a.blog}>
                    <LinkIcon /> {a.blog.replace(/^https?:\/\//, '')}
                  </a>
                ) : null}
              </div>
            </div>
            <OrgsBlock owner={a.login} />
          </aside>
          <main>
            <OwnerTabs account={a} current={current} counts={{ repos: repos?.length, projects, stars: starred.data?.length }} />
            <div style={{ marginTop: 16 }}>
              {current === 'repositories' ? (
                <>
                  {role.self ? (
                    <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 8 }}>
                      <ButtonLink to="/new" search={{ owner: a.login }} variant="primary" size="small" leadingVisual={RepoIcon}>
                        New
                      </ButtonLink>
                    </div>
                  ) : null}
                  <RepoList repos={repos ?? []} empty={`${a.login} doesn't have any public repositories yet.`} />
                </>
              ) : current === 'stars' ? (
                <RepoList repos={starred.data ?? []} showOwner empty={`${a.login} doesn't have any starred repositories yet.`} />
              ) : (
                <div className="overview-grid">
                  <PinnedRepos owner={a.login} fallback={repos ?? []} canEdit={role.self} isOrg={false} />
                  <ContributionGraph login={a.login} since={a.created_at} />
                </div>
              )}
            </div>
          </main>
        </div>
      </div>
    </>
  );
}
