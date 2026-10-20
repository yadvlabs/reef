import { createFileRoute } from '@tanstack/react-router';
import { BlobView } from '../components/code/BlobView';
import { CodeLayout } from '../components/code/CodeLayout';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/blob/$rev/$')({ component: Blob });

function Blob() {
  const repo = useRepo();
  const { rev, _splat } = Route.useParams();
  const path = (_splat ?? '').replace(/^\/+|\/+$/g, '');
  return (
    <CodeLayout repo={repo} rev={rev} path={path}>
      <BlobView repo={repo} rev={rev} path={path} />
    </CodeLayout>
  );
}
