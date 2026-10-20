import { createFileRoute } from '@tanstack/react-router';
import { BlameView } from '../components/code/BlameView';
import { CodeLayout } from '../components/code/CodeLayout';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/blame/$rev/$')({ component: Blame });

function Blame() {
  const repo = useRepo();
  const { rev, _splat } = Route.useParams();
  const path = (_splat ?? '').replace(/^\/+|\/+$/g, '');
  return (
    <CodeLayout repo={repo} rev={rev} path={path}>
      <BlameView repo={repo} rev={rev} path={path} />
    </CodeLayout>
  );
}
