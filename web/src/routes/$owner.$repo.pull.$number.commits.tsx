import { createFileRoute } from '@tanstack/react-router';
import { PullCommits } from '../components/pulls/Commits';
import { usePull } from './$owner.$repo.pull.$number';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/pull/$number/commits')({ component: Page });

function Page() {
  const repo = useRepo();
  const pr = usePull();
  return <PullCommits repo={repo} pr={pr} />;
}
