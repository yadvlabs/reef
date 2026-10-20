import { createFileRoute } from '@tanstack/react-router';
import { FileEditor } from '../components/FileEditor';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/edit/$rev/$')({
  validateSearch: (s: Record<string, unknown>): { delete?: boolean } => (s.delete === true ? { delete: true } : {}),
  component: EditFile,
});

function EditFile() {
  const repo = useRepo();
  const { rev, _splat } = Route.useParams();
  const { delete: del } = Route.useSearch();
  return <FileEditor repo={repo} rev={rev} path={(_splat ?? '').replace(/^\/+|\/+$/g, '')} mode={del ? 'delete' : 'edit'} />;
}
