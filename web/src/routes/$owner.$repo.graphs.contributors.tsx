import { createFileRoute } from '@tanstack/react-router';
import { InsightsLayout, ContributorsPage } from '../components/Insights';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/graphs/contributors')({ component: Page });

function Page() {
  const repo = useRepo();
  return (
    <InsightsLayout repo={repo}>
      <ContributorsPage repo={repo} />
    </InsightsLayout>
  );
}
