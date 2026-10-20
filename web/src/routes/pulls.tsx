import { createFileRoute } from '@tanstack/react-router';
import { GlobalIssues, parseGlobalIssuesSearch } from '../components/GlobalIssues';

export const Route = createFileRoute('/pulls')({
  validateSearch: parseGlobalIssuesSearch,
  component: Page,
});

function Page() {
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return <GlobalIssues kind="pulls" search={search} onSearch={(s) => navigate({ search: s })} />;
}
