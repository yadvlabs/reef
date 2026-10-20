import { createFileRoute } from '@tanstack/react-router';
import { FileEditor } from '../components/FileEditor';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/new/$rev/$')({ component: NewFile });

function NewFile() {
  const repo = useRepo();
  const { rev, _splat } = Route.useParams();
  return <FileEditor repo={repo} rev={rev} path={(_splat ?? '').replace(/^\/+|\/+$/g, '')} mode="new" />;
}
