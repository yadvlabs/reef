import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { ProjectShell } from '../components/projects/ProjectShell';
import { Workflows } from '../components/projects/Workflows';
import { ApiError } from '../lib/api';
import { projectQuery } from '../lib/projects';

export const Route = createFileRoute('/$owner/projects/$number_/workflows')({ component: Page });

function Page() {
  const { owner, number } = Route.useParams();
  const q = useQuery(projectQuery(owner, Number(number)));
  if (q.isPending) return <><AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} /><Loading /></>;
  if (q.isError) return <><AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} />{q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="project" /> : <ErrorFlash error={q.error} />}</>;
  return (
    <ProjectShell project={q.data}>
      <Workflows project={q.data} />
    </ProjectShell>
  );
}
