import { RepoIcon, LockIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { Loading } from '../components/Loading';
import { api, type Repo } from '../lib/api';

export const Route = createFileRoute('/search')({
  validateSearch: (s: Record<string, unknown>): { q?: string } => (typeof s.q === 'string' ? { q: s.q } : {}),
  component: Search,
});

function Search() {
  const { q = '' } = Route.useSearch();
  const res = useQuery({
    queryKey: ['search', 'repos', q],
    queryFn: () => api<{ total_count: number; items: Repo[] }>(`/search/repositories?q=${encodeURIComponent(q)}&per_page=50`),
    enabled: q.length > 0,
  });
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <h1 className="section-title">
          {res.data ? `${res.data.total_count} repository results` : 'Search'}
        </h1>
        {res.isPending && q ? <Loading /> : null}
        <div className="repo-list">
          {res.data?.items.map((r) => (
            <div key={r.id} className="item">
              <div className="headline">
                {r.private ? <LockIcon className="muted" /> : <RepoIcon className="muted" />}
                <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
                  {r.full_name}
                </Link>
              </div>
              {r.description ? <p className="desc">{r.description}</p> : null}
            </div>
          ))}
        </div>
      </div>
    </>
  );
}
