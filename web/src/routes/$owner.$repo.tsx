import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Outlet } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { RepoHeader } from '../components/RepoHeader';
import { ApiError, type Repo } from '../lib/api';
import { repoQuery } from '../lib/repo';

export const Route = createFileRoute('/$owner/$repo')({ component: RepoLayout });

export function useRepo(): Repo {
  const { owner, repo } = Route.useParams();
  const q = useQuery(repoQuery(owner, repo));
  return q.data as Repo;
}

function RepoLayout() {
  const { owner, repo } = Route.useParams();
  const q = useQuery(repoQuery(owner, repo));
  if (q.isPending) return <><AppHeader context={{ owner, repo }} /><Loading /></>;
  if (q.isError) return <><AppHeader context={{ owner, repo }} />{q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="repository" /> : <ErrorFlash error={q.error} />}</>;
  return (
    <>
      <AppHeader context={{ owner: q.data.owner.login, repo: q.data.name, repoData: q.data }} />
      <RepoHeader repo={q.data} />
      <div className="page">
        <Outlet />
      </div>
    </>
  );
}
