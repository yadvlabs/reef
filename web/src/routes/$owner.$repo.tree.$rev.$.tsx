import { createFileRoute } from '@tanstack/react-router';
import { CodeLayout } from '../components/code/CodeLayout';
import { TreeView } from '../components/RepoCode';
import { RepoSidebar } from '../components/RepoSidebar';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/tree/$rev/$')({ component: Tree });

function Tree() {
  const repo = useRepo();
  const { rev, _splat } = Route.useParams();
  const path = (_splat ?? '').replace(/^\/+|\/+$/g, '');
  return (
    <CodeLayout repo={repo} rev={rev} path={path} aside={path ? undefined : <RepoSidebar repo={repo} empty={false} />}>
      <TreeView repo={repo} rev={rev} path={path} />
    </CodeLayout>
  );
}
