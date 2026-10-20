import { NavList } from '@primer/react';
import { GearIcon, GitBranchIcon, PeopleIcon, PlayIcon, WebhookIcon, MirrorIcon } from '@primer/octicons-react';
import type { Repo } from '../lib/api';
import { NavListLink } from './links';

export function SettingsNav({ repo, current }: { repo: Repo; current: 'general' | 'branches' | 'collaborators' | 'actions' | 'hooks' | 'mirror' }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  return (
    <NavList aria-label="Repository settings">
      <NavListLink to="/$owner/$repo/settings" params={p} activeOptions={{ exact: true }} aria-current={current === 'general' ? 'page' : undefined}>
        <NavList.LeadingVisual>
          <GearIcon />
        </NavList.LeadingVisual>
        General
      </NavListLink>
      <NavListLink to="/$owner/$repo/settings/collaborators" params={p} aria-current={current === 'collaborators' ? 'page' : undefined}>
        <NavList.LeadingVisual>
          <PeopleIcon />
        </NavList.LeadingVisual>
        Collaborators
      </NavListLink>
      <NavListLink to="/$owner/$repo/settings/branches" params={p} aria-current={current === 'branches' ? 'page' : undefined}>
        <NavList.LeadingVisual>
          <GitBranchIcon />
        </NavList.LeadingVisual>
        Branches
      </NavListLink>
      <NavListLink to="/$owner/$repo/settings/actions" params={p} aria-current={current === 'actions' ? 'page' : undefined}>
        <NavList.LeadingVisual>
          <PlayIcon />
        </NavList.LeadingVisual>
        Actions
      </NavListLink>
      <NavListLink to="/$owner/$repo/settings/hooks" params={p} aria-current={current === 'hooks' ? 'page' : undefined}>
        <NavList.LeadingVisual>
          <WebhookIcon />
        </NavList.LeadingVisual>
        Webhooks
      </NavListLink>
      <NavListLink to="/$owner/$repo/settings/mirror" params={p} aria-current={current === 'mirror' ? 'page' : undefined}>
        <NavList.LeadingVisual>
          <MirrorIcon />
        </NavList.LeadingVisual>
        Mirrors and LFS
      </NavListLink>
    </NavList>
  );
}
