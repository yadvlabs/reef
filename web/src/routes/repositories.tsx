// Every repository the signed-in person can reach (their own and their
// organizations'), like GitHub's "Your repositories".
import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading } from '../components/Loading';
import { DEFAULT_FILTER, filterRepos, RepoFilterBar, type RepoFilter } from '../components/RepoFilterBar';
import { RepoList } from '../components/RepoList';
import { api, type Repo } from '../lib/api';

export const Route = createFileRoute('/repositories')({ component: Repositories });

function Repositories() {
  const q = useQuery({ queryKey: ['user', 'repos', 'all'], queryFn: () => api<Repo[]>('/user/repos?sort=updated&per_page=100') });
  const [filter, setFilter] = useState<RepoFilter>(DEFAULT_FILTER);
  const repos = filterRepos(q.data ?? [], filter);
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
          <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>Your repositories</h1>
          <span className="muted small">{q.data ? `${repos.length} of ${q.data.length}` : ''}</span>
        </div>
        <RepoFilterBar repos={q.data ?? []} value={filter} onChange={setFilter} canCreate />
        {q.isPending ? <Loading /> : null}
        {q.isError ? <ErrorFlash error={q.error} /> : null}
        {q.data ? <RepoList repos={repos} showOwner empty={filter.q || filter.type !== 'all' || filter.language ? 'No repository matches.' : 'No repositories yet.'} /> : null}
      </div>
    </>
  );
}
