import { useQuery } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { OwnerTabs, accountQuery, useOwnerRole } from '../components/OwnerTabs';
import { WikiPages } from '../components/Wiki';
import { ApiError } from '../lib/api';

export const Route = createFileRoute('/$owner/wiki/$')({ component: OrgWiki });

function OrgWiki() {
  const { owner, _splat } = Route.useParams();
  const account = useQuery(accountQuery(owner));
  const role = useOwnerRole(owner);
  if (account.isPending) return <><AppHeader context={{ owner }} /><Loading /></>;
  if (account.isError) return <><AppHeader context={{ owner }} />{account.error instanceof ApiError && account.error.status === 404 ? <NotFound what="organization" /> : <ErrorFlash error={account.error} />}</>;
  const a = account.data;
  if (a.type !== 'Organization') return <><AppHeader context={{ owner }} /><NotFound what="wiki" /></>;
  return (
    <>
      <AppHeader context={{ owner: a.login, section: { label: 'Wiki', to: 'wiki' } }} />
      <OwnerTabs account={a} current="wiki" />
      <div className="page">
        <WikiPages apiBase={`/orgs/${a.login}/wiki`} webBase={`/${a.login}/wiki`} splat={_splat ?? ''} title={a.name ?? a.login} canEdit={role.member} />
      </div>
    </>
  );
}
