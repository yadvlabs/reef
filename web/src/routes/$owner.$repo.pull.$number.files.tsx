import { createFileRoute } from '@tanstack/react-router';
import { PullFiles } from '../components/pulls/FilesChanged';
import { usePull } from './$owner.$repo.pull.$number';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/pull/$number/files')({ component: Page });

function Page() {
  const repo = useRepo();
  const pr = usePull();
  return <PullFiles repo={repo} pr={pr} />;
}
