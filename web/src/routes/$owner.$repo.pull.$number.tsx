// Pull request layout: header (title, state, refs, tabs) with the tab content as a child route.
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Outlet, useNavigate } from '@tanstack/react-router';
import { useEffect } from 'react';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { PullHeader } from '../components/pulls/PullHeader';
import { ApiError } from '../lib/api';
import { pullQuery, type Pull } from '../lib/pulls';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/pull/$number')({
  // Legacy `?tab=` links are redirected to the tab routes below.
  validateSearch: (s: Record<string, unknown>): { tab?: string } => (typeof s.tab === 'string' ? { tab: s.tab } : {}),
  component: PullLayout,
});

export function usePull(): Pull {
  const repo = useRepo();
  const { number } = Route.useParams();
  const q = useQuery(pullQuery(repo.full_name, Number(number)));
  return q.data as Pull;
}

function PullLayout() {
  const repo = useRepo();
  const { number } = Route.useParams();
  const { tab } = Route.useSearch();
  const navigate = useNavigate();
  const q = useQuery(pullQuery(repo.full_name, Number(number)));
  const p = { owner: repo.owner.login, repo: repo.name, number };
  useEffect(() => {
    if (tab === 'files') navigate({ to: '/$owner/$repo/pull/$number/files', params: p, replace: true });
    else if (tab === 'commits') navigate({ to: '/$owner/$repo/pull/$number/commits', params: p, replace: true });
    else if (tab === 'checks') navigate({ to: '/$owner/$repo/pull/$number/checks', params: p, replace: true });
    else if (tab) navigate({ to: '/$owner/$repo/pull/$number', params: p, search: {}, replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);
  if (q.isPending) return <Loading />;
  if (q.isError) return q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="pull request" /> : <ErrorFlash error={q.error} />;
  return (
    <div className="pull-page">
      <PullHeader repo={repo} pr={q.data} />
      <Outlet />
    </div>
  );
}
