import { NavList } from '@primer/react';
import { createFileRoute, Outlet, useRouterState } from '@tanstack/react-router';
import { NavListLink } from '../components/links';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/security')({ component: SecurityLayout });

function SecurityLayout() {
  const repo = useRepo();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const base = `/${repo.owner.login}/${repo.name}/security`;
  const p = { owner: repo.owner.login, repo: repo.name };
  const is = (suffix: string) => (path.replace(/\/$/, '') === base + suffix ? 'page' : undefined);
  return (
    <div className="security-layout">
      <NavList aria-label="Security">
        <NavListLink to="/$owner/$repo/security" params={p} aria-current={is('')}>
          Overview
        </NavListLink>
        <NavList.Group title="Reporting">
          <NavListLink to="/$owner/$repo/security/policy" params={p} aria-current={is('/policy')}>
            Policy
          </NavListLink>
          <NavListLink to="/$owner/$repo/security/advisories" params={p} aria-current={is('/advisories')}>
            Advisories
          </NavListLink>
        </NavList.Group>
        <NavList.Group title="Supply chain">
          <NavListLink to="/$owner/$repo/security/dependencies" params={p} aria-current={is('/dependencies')}>
            Dependencies
          </NavListLink>
        </NavList.Group>
      </NavList>
      <div className="security-main">
        <Outlet />
      </div>
    </div>
  );
}
