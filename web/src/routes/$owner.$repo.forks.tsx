import { createFileRoute } from '@tanstack/react-router';
import { InsightsLayout, ForksPage } from '../components/Insights';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/forks')({ component: Page });

function Page() {
  const repo = useRepo();
  return (
    <InsightsLayout repo={repo}>
      <ForksPage repo={repo} />
    </InsightsLayout>
  );
}
