// Every repository the signed-in person can reach (their own and their
// organizations'), like GitHub's "Your repositories".
import { RepoIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ButtonLink } from '../components/links';
import { ErrorFlash, Loading } from '../components/Loading';
import { RepoList } from '../components/RepoList';
import { api, type Repo } from '../lib/api';

export const Route = createFileRoute('/repositories')({ component: Repositories });

function Repositories() {
  const q = useQuery({ queryKey: ['user', 'repos', 'all'], queryFn: () => api<Repo[]>('/user/repos?sort=updated&per_page=100') });
  const [filter, setFilter] = useState('');
  const repos = (q.data ?? []).filter((r) => r.full_name.toLowerCase().includes(filter.toLowerCase()));
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
          <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>Your repositories</h1>
          <span className="muted small">{q.data ? `${repos.length} of ${q.data.length}` : ''}</span>
          <span style={{ flex: 1 }} />
          <input className="side-filter" style={{ width: 260 }} placeholder="Find a repository…" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Find a repository" />
          <ButtonLink to="/new" size="small" variant="primary" leadingVisual={RepoIcon}>
            New
          </ButtonLink>
        </div>
        {q.isPending ? <Loading /> : null}
        {q.isError ? <ErrorFlash error={q.error} /> : null}
        {q.data ? <RepoList repos={repos} showOwner empty={filter ? 'No repository matches.' : 'No repositories yet.'} /> : null}
      </div>
    </>
  );
}
