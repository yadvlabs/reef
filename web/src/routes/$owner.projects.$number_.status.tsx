import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { ProjectShell } from '../components/projects/ProjectShell';
import { StatusUpdatesList } from '../components/projects/StatusUpdates';
import { ApiError } from '../lib/api';
import { projectQuery } from '../lib/projects';

export const Route = createFileRoute('/$owner/projects/$number_/status')({ component: Page });

function Page() {
  const { owner, number } = Route.useParams();
  const q = useQuery(projectQuery(owner, Number(number)));
  if (q.isPending) return <><AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} /><Loading /></>;
  if (q.isError) return <><AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} />{q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="project" /> : <ErrorFlash error={q.error} />}</>;
  return (
    <ProjectShell project={q.data}>
      <StatusUpdatesList project={q.data} />
    </ProjectShell>
  );
}
