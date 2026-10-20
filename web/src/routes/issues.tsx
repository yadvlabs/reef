import { createFileRoute } from '@tanstack/react-router';
import { GlobalIssues, parseGlobalIssuesSearch } from '../components/GlobalIssues';

export const Route = createFileRoute('/issues')({
  validateSearch: parseGlobalIssuesSearch,
  component: Page,
});

function Page() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return <GlobalIssues kind="issues" search={search} onSearch={(s) => navigate({ search: s })} />;
}
