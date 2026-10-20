import { NavList } from '@primer/react';
import { NavListLink } from '../components/links';
import { PersonIcon, KeyIcon, ShieldLockIcon, MailIcon } from '@primer/octicons-react';
import { createFileRoute, Outlet, useNavigate, useRouterState } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { Loading } from '../components/Loading';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/settings')({ component: SettingsLayout });

function SettingsLayout() {
  const { user, loading } = useSession();
  const navigate = useNavigate();
  const path = useRouterState({ select: (s) => s.location.pathname });
  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: path } });
    return null;
  }
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <div className="settings-layout">
          <NavList aria-label="Settings">
            <NavListLink to="/settings" activeOptions={{ exact: true }} aria-current={path === '/settings' ? 'page' : undefined}>
              <NavList.LeadingVisual>
                <PersonIcon />
              </NavList.LeadingVisual>
              Public profile
            </NavListLink>
            <NavListLink to="/settings/security" aria-current={path === '/settings/security' ? 'page' : undefined}>
              <NavList.LeadingVisual>
                <ShieldLockIcon />
              </NavList.LeadingVisual>
              Password and identities
            </NavListLink>
            <NavListLink to="/settings/emails" aria-current={path === '/settings/emails' ? 'page' : undefined}>
              <NavList.LeadingVisual>
                <MailIcon />
              </NavList.LeadingVisual>
              Emails
            </NavListLink>
            <NavListLink to="/settings/keys" aria-current={path === '/settings/keys' ? 'page' : undefined}>
              <NavList.LeadingVisual>
                <KeyIcon />
              </NavList.LeadingVisual>
              SSH and GPG keys
            </NavListLink>
            <NavListLink to="/settings/tokens" aria-current={path === '/settings/tokens' ? 'page' : undefined}>
              <NavList.LeadingVisual>
                <KeyIcon />
              </NavList.LeadingVisual>
              Personal access tokens
            </NavListLink>
          </NavList>
          <div>
            <Outlet />
          </div>
        </div>
      </div>
    </>
  );
}
