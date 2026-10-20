import { createFileRoute } from '@tanstack/react-router';
import { WikiPages } from '../components/Wiki';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/wiki/$')({ component: RepoWiki });

function RepoWiki() {
  const repo = useRepo();
  const { _splat } = Route.useParams();
  return <WikiPages apiBase={`/repos/${repo.full_name}/wiki`} webBase={`/${repo.full_name}/wiki`} splat={_splat ?? ''} title={repo.name} canEdit={!!repo.permissions?.push} repo={repo} />;
}
