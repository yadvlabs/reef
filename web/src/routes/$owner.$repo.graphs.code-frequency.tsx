import { createFileRoute } from '@tanstack/react-router';
import { InsightsLayout, CodeFrequencyPage } from '../components/Insights';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/graphs/code-frequency')({ component: Page });

function Page() {
  const repo = useRepo();
  return (
    <InsightsLayout repo={repo}>
      <CodeFrequencyPage repo={repo} />
    </InsightsLayout>
  );
}
