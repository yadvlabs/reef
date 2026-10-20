import { createFileRoute } from '@tanstack/react-router';
import { InsightsLayout, CommitsPage } from '../components/Insights';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/graphs/commit-activity')({ component: Page });

function Page() {
  const repo = useRepo();
  return (
    <InsightsLayout repo={repo}>
      <CommitsPage repo={repo} />
    </InsightsLayout>
  );
}
