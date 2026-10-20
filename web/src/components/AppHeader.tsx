import { ActionList, ActionMenu, Avatar, IconButton, Label, TextInput } from '@primer/react';
import { ButtonLink, ActionListLink } from './links';
import { PlusIcon, SearchIcon, RepoIcon, OrganizationIcon, PersonIcon, GearIcon, SignOutIcon, KeyIcon, TableIcon, BookIcon, ShieldLockIcon, StarIcon, IssueOpenedIcon, GitPullRequestIcon } from '@primer/octicons-react';
import { Link, useNavigate } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { reef, type Repo } from '../lib/api';
import { accountQuery } from './OwnerTabs';
import { useRefreshSession, useSession } from '../lib/session';
import { Logo } from './Logo';
import { NotificationsBell } from './NotificationsBell';
import { RepoSwitcher } from './RepoSwitcher';

/// What the header breadcrumb points at: an owner, a repository, or an owner-level section
/// (projects, wiki), like GitHub's global navigation context.
export interface HeaderContext {
  owner: string;
  repo?: string;
  repoData?: Repo;
  section?: { label: string; to: 'projects' | 'wiki' };
}

export function AppHeader({ context, dashboard }: { context?: HeaderContext; dashboard?: boolean }) {
  const { user, instance } = useSession();
  const refresh = useRefreshSession();
  const navigate = useNavigate();

  async function signOut() {
    // A session opened through an identity provider answers with the provider's end_session URL:
    // going there ends the provider sign-in too, and the provider sends the browser back to Reef.
    const out = await reef<{ logout_url?: string | null } | undefined>('/session', { method: 'DELETE' });
    if (out?.logout_url) {
      window.location.assign(out.logout_url);
      return;
    }
    await refresh();
    navigate({ to: '/' });
  }

  const r = context?.repoData;
  const ownerAccount = useQuery({ ...accountQuery(context?.owner ?? ''), enabled: !!context && !context.repo });
  const ownerAvatar = ownerAccount.data ? <Avatar src={ownerAccount.data.avatar_url} size={20} square={ownerAccount.data.type === 'Organization'} alt="" /> : null;
  return (
    <header className="app-header">
      <Link to="/" className="brand" aria-label={instance?.name ?? 'Reef'}>
        <Logo size={28} />
        {context ? null : <span className="product">{instance?.name ?? 'Reef'}{dashboard ? ' Dashboard' : ''}</span>}
      </Link>
      {context ? (
        <nav className="context" aria-label="Page context">
          <span className="ctx-icon">
            {context.repo ? (r ? <Avatar src={r.owner.avatar_url} size={20} square={r.owner.type === 'Organization'} alt="" /> : <RepoIcon size={16} />) : ownerAvatar ?? (context.section?.to === 'projects' ? <TableIcon size={16} /> : context.section?.to === 'wiki' ? <BookIcon size={16} /> : <PersonIcon size={16} />)}
          </span>
          <Link to="/$owner" params={{ owner: context.owner }} className="owner">
            {context.owner}
          </Link>
          {context.repo ? (
            <>
              <span className="sep">/</span>
              <Link to="/$owner/$repo" params={{ owner: context.owner, repo: context.repo }} className="repo">
                {context.repo}
              </Link>
              <RepoSwitcher owner={context.owner} repo={context.repo} />
              {r ? (
                <Label className="visibility" variant="secondary">
                  {r.private ? 'Private' : 'Public'}
                </Label>
              ) : null}
              {r?.archived ? <Label variant="attention">Archived</Label> : null}
              {r?.is_template ? <Label variant="secondary">Template</Label> : null}
            </>
          ) : context.section ? (
            <>
              <span className="sep">/</span>
              {context.section.to === 'projects' ? (
                <Link to="/$owner/projects" params={{ owner: context.owner }} search={{}} className="repo">
                  {context.section.label}
                </Link>
              ) : (
                <a href={`/${context.owner}/wiki`} className="repo">
                  {context.section.label}
                </a>
              )}
            </>
          ) : null}
        </nav>
      ) : null}
      <div className="spacer" />
      <form
        className="search"
        onSubmit={(e) => {
          e.preventDefault();
          const q = new FormData(e.currentTarget).get('q');
          navigate({ to: '/search', search: { q: String(q ?? '') } });
        }}
      >
        <TextInput name="q" leadingVisual={SearchIcon} placeholder="Type / to search" block size="medium" aria-label="Search" />
      </form>
      <div className="actions">
        {user ? (
          <>
            <ActionMenu>
              <ActionMenu.Anchor>
                <IconButton icon={PlusIcon} aria-label="Create new" variant="default" />
              </ActionMenu.Anchor>
              <ActionMenu.Overlay align="end">
                <ActionList>
                  <ActionListLink to="/new">
                    <ActionList.LeadingVisual>
                      <RepoIcon />
                    </ActionList.LeadingVisual>
                    New repository
                  </ActionListLink>
                  <ActionListLink to="/$owner/projects" params={{ owner: context?.owner ?? user.login }} search={{ new: true }}>
                    <ActionList.LeadingVisual>
                      <TableIcon />
                    </ActionList.LeadingVisual>
                    New project
                  </ActionListLink>
                  <ActionListLink to="/organizations/new">
                    <ActionList.LeadingVisual>
                      <OrganizationIcon />
                    </ActionList.LeadingVisual>
                    New organization
                  </ActionListLink>
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
            <IconButton icon={IssueOpenedIcon} aria-label="Issues" title="Issues" variant="default" onClick={() => navigate({ to: '/issues' })} />
            <IconButton icon={GitPullRequestIcon} aria-label="Pull requests" title="Pull requests" variant="default" onClick={() => navigate({ to: '/pulls' })} />
            <IconButton icon={RepoIcon} aria-label="Repositories" title="Repositories" variant="default" onClick={() => navigate({ to: '/repositories' })} />
            <NotificationsBell />
            <ActionMenu>
              <ActionMenu.Anchor>
                <button className="avatar-button" aria-label="Open user menu">
                  <Avatar src={user.avatar_url} size={32} alt={user.login} />
                </button>
              </ActionMenu.Anchor>
              <ActionMenu.Overlay align="end" width="medium">
                <ActionList>
                  <ActionList.Item disabled>
                    <ActionList.LeadingVisual>
                      <Avatar src={user.avatar_url} size={20} alt="" />
                    </ActionList.LeadingVisual>
                    {user.login}
                    <ActionList.Description variant="block">{user.name ?? ''}</ActionList.Description>
                  </ActionList.Item>
                  <ActionList.Divider />
                  <ActionListLink to="/$owner" params={{ owner: user.login }}>
                    <ActionList.LeadingVisual>
                      <PersonIcon />
                    </ActionList.LeadingVisual>
                    Your profile
                  </ActionListLink>
                  <ActionListLink to="/$owner" params={{ owner: user.login }} search={{ tab: 'repositories' }}>
                    <ActionList.LeadingVisual>
                      <RepoIcon />
                    </ActionList.LeadingVisual>
                    Your repositories
                  </ActionListLink>
                  <ActionListLink to="/$owner/projects" params={{ owner: user.login }} search={{}}>
                    <ActionList.LeadingVisual>
                      <TableIcon />
                    </ActionList.LeadingVisual>
                    Your projects
                  </ActionListLink>
                  <ActionListLink to="/settings/keys">
                    <ActionList.LeadingVisual>
                      <KeyIcon />
                    </ActionList.LeadingVisual>
                    SSH and tokens
                  </ActionListLink>
                  <ActionListLink to="/settings">
                    <ActionList.LeadingVisual>
                      <GearIcon />
                    </ActionList.LeadingVisual>
                    Settings
                  </ActionListLink>
                  <ActionListLink to="/user/starred">
                    <ActionList.LeadingVisual>
                      <StarIcon />
                    </ActionList.LeadingVisual>
                    Your stars
                  </ActionListLink>
                  {user.site_admin ? (
                    <>
                      <ActionList.Divider />
                      <ActionListLink to="/site" search={{}}>
                        <ActionList.LeadingVisual>
                          <ShieldLockIcon />
                        </ActionList.LeadingVisual>
                        Site admin
                      </ActionListLink>
                    </>
                  ) : null}
                  <ActionList.Divider />
                  <ActionList.Item onSelect={signOut}>
                    <ActionList.LeadingVisual>
                      <SignOutIcon />
                    </ActionList.LeadingVisual>
                    Sign out
                  </ActionList.Item>
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          </>
        ) : (
          <>
            <ButtonLink to="/login" variant="invisible">
              Sign in
            </ButtonLink>
            {instance?.allow_signup ? (
              <ButtonLink to="/join" variant="default">
                Sign up
              </ButtonLink>
            ) : null}
          </>
        )}
      </div>
    </header>
  );
}
