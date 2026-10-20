import { createFileRoute } from '@tanstack/react-router';
import { PullChecks } from '../components/pulls/Checks';
import { usePull } from './$owner.$repo.pull.$number';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/pull/$number/checks')({ component: Page });

function Page() {
  const repo = useRepo();
  const pr = usePull();
  return <PullChecks repo={repo} pr={pr} />;
}
