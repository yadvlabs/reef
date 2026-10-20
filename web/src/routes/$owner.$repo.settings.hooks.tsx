import { createFileRoute } from '@tanstack/react-router';
import { NotFound } from '../components/Loading';
import { SettingsNav } from '../components/SettingsNav';
import { WebhooksPanel } from '../components/webhooks/WebhooksPanel';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/settings/hooks')({ component: Page });

function Page() {
  const repo = useRepo();
  if (!repo.permissions?.admin) return <NotFound />;
  return (
    <div className="settings-layout">
      <SettingsNav repo={repo} current="hooks" />
      <WebhooksPanel base={`/repos/${repo.full_name}`} canAdmin={!!repo.permissions?.admin} />
    </div>
  );
}
