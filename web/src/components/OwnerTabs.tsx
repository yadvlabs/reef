// The tab row shared by an owner's profile, projects, and (for organizations) wiki pages.
import { Avatar, UnderlineNav } from '@primer/react';
import { BookIcon, PeopleIcon, RepoIcon, TableIcon, StarIcon } from '@primer/octicons-react';
import { queryOptions, useQuery } from '@tanstack/react-query';
import { api, type User } from '../lib/api';
import { useSession } from '../lib/session';
import { UnderlineNavLink } from './links';

export const accountQuery = (owner: string) => queryOptions({ queryKey: ['account', owner.toLowerCase()], queryFn: () => api<User>(`/users/${owner}`) });
export const userOrgsQuery = (owner: string) => queryOptions({ queryKey: ['account', owner.toLowerCase(), 'orgs'], queryFn: () => api<User[]>(`/users/${owner}/orgs`) });
export const orgMembersQuery = (owner: string) => queryOptions({ queryKey: ['org', owner.toLowerCase(), 'members'], queryFn: () => api<User[]>(`/orgs/${owner}/members`) });

/// Whether the signed-in user is the owner, or a member of the organization.
export function useOwnerRole(owner: string): { self: boolean; member: boolean } {
  const { user, orgs } = useSession();
  const self = !!user && user.login.toLowerCase() === owner.toLowerCase();
  const member = self || (!!user?.site_admin) || (orgs ?? []).some((o) => o.login.toLowerCase() === owner.toLowerCase());
  return { self, member };
}

export function OwnerTabs({ account, current, counts, hideIdent }: { account: User; hideIdent?: boolean; current: 'overview' | 'repositories' | 'projects' | 'wiki' | 'people' | 'stars'; counts?: { repos?: number; members?: number; projects?: number; stars?: number } }) {
  const isOrg = account.type === 'Organization';
  const p = { owner: account.login };
  return (
    <div className="owner-tabs">
      <div className="owner-tabs-inner">
        {isOrg && !hideIdent ? (
          <div className="owner-ident">
            <Avatar src={account.avatar_url} size={32} square alt="" />
            <strong>{account.name ?? account.login}</strong>
          </div>
        ) : null}
        <UnderlineNav aria-label="Owner">
          <UnderlineNavLink to="/$owner" params={p} search={{ tab: undefined }} activeOptions={{ exact: true, includeSearch: true }} icon={BookIcon} aria-current={current === 'overview' ? 'page' : undefined}>
            Overview
          </UnderlineNavLink>
          <UnderlineNavLink to="/$owner" params={p} search={{ tab: 'repositories' }} activeOptions={{ exact: true, includeSearch: true }} icon={RepoIcon} aria-current={current === 'repositories' ? 'page' : undefined} counter={counts?.repos}>
            Repositories
          </UnderlineNavLink>
          <UnderlineNavLink to="/$owner/projects" params={p} search={{}} icon={TableIcon} aria-current={current === 'projects' ? 'page' : undefined} counter={counts?.projects}>
            Projects
          </UnderlineNavLink>
          {isOrg ? (
            <UnderlineNavLink to="/$owner/wiki/$" params={{ ...p, _splat: '' }} icon={BookIcon} aria-current={current === 'wiki' ? 'page' : undefined}>
              Wiki
            </UnderlineNavLink>
          ) : null}
          {isOrg ? (
            <UnderlineNavLink to="/$owner" params={p} search={{ tab: 'people' }} activeOptions={{ exact: true, includeSearch: true }} icon={PeopleIcon} aria-current={current === 'people' ? 'page' : undefined} counter={counts?.members}>
              People
            </UnderlineNavLink>
          ) : (
            <UnderlineNavLink to="/$owner" params={p} search={{ tab: 'stars' }} activeOptions={{ exact: true, includeSearch: true }} icon={StarIcon} aria-current={current === 'stars' ? 'page' : undefined} counter={counts?.stars}>
              Stars
            </UnderlineNavLink>
          )}
        </UnderlineNav>
      </div>
    </div>
  );
}

export function OrgsBlock({ owner }: { owner: string }) {
  const orgs = useQuery(userOrgsQuery(owner));
  if (!orgs.data?.length) return null;
  return (
    <div className="orgs-block">
      <h2>Organizations</h2>
      <div className="org-avatars">
        {orgs.data.map((o) => (
          <a key={o.id} href={`/${o.login}`} title={o.login}>
            <Avatar src={o.avatar_url} size={32} square alt={o.login} />
          </a>
        ))}
      </div>
    </div>
  );
}
