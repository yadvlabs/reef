import { createFileRoute, Navigate } from '@tanstack/react-router';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/commits/')({ component: () => {
  const repo = useRepo();
  return <Navigate to="/$owner/$repo/commits/$rev" params={{ owner: repo.owner.login, repo: repo.name, rev: repo.default_branch }} replace />;
} });
