import { ActionList, ActionMenu, Avatar, Button, Dialog, FormControl, Label, TextInput, UnderlineNav } from '@primer/react';
import { CodeIcon, IssueOpenedIcon, GitPullRequestIcon, PlayIcon, TableIcon, BookIcon, GearIcon, GraphIcon, ShieldIcon, EyeIcon, RepoForkedIcon, StarIcon, StarFillIcon, BellSlashIcon, TriangleDownIcon } from '@primer/octicons-react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useRouterState } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo } from '../lib/api';
import { useSession } from '../lib/session';
import { UnderlineNavLink } from './links';

/// The repository tab row, under the global header's owner/name breadcrumb. The name itself,
/// with the owner's avatar and the watch/fork/star actions, is `RepoTitle` on the code home.
export function RepoHeader({ repo }: { repo: Repo }) {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const base = `/${repo.owner.login}/${repo.name}`;
  const rest = path.slice(base.length);
  const section = rest.split('/')[1] ?? '';
  const current = (name: string) => (name === '' ? ['', 'tree', 'blob', 'commits', 'commit', 'branches', 'tags', 'releases', 'new', 'edit', 'compare'].includes(section) : name === 'pulse' ? ['pulse', 'graphs', 'forks'].includes(section) : section === name);
  const p = { owner: repo.owner.login, repo: repo.name };
  const items: { name: string; label: string; icon: typeof CodeIcon; to: string; count?: number; show: boolean }[] = [
    { name: '', label: 'Code', icon: CodeIcon, to: '/$owner/$repo', show: true },
    { name: 'issues', label: 'Issues', icon: IssueOpenedIcon, to: '/$owner/$repo/issues', count: repo.open_issues_only_count ?? repo.open_issues_count, show: repo.has_issues },
    { name: 'pulls', label: 'Pull requests', icon: GitPullRequestIcon, to: '/$owner/$repo/pulls', count: repo.open_pull_requests_count, show: true },
    { name: 'actions', label: 'Actions', icon: PlayIcon, to: '/$owner/$repo/actions', show: true },
    { name: 'projects', label: 'Projects', icon: TableIcon, to: '/$owner/$repo/projects', show: repo.has_projects },
    { name: 'wiki', label: 'Wiki', icon: BookIcon, to: '/$owner/$repo/wiki', show: repo.has_wiki },
    { name: 'security', label: 'Security', icon: ShieldIcon, to: '/$owner/$repo/security', show: true },
    { name: 'pulse', label: 'Insights', icon: GraphIcon, to: '/$owner/$repo/pulse', show: true },
    { name: 'settings', label: 'Settings', icon: GearIcon, to: '/$owner/$repo/settings', show: !!repo.permissions?.admin },
  ];
  return (
    <div className="repo-header">
      <div className="repo-nav-row">
        <UnderlineNav aria-label="Repository">
          {items
            .filter((i) => i.show)
            .map((i) => (
              <UnderlineNavLink key={i.name} to={i.to} params={p} icon={i.icon} activeOptions={{ exact: true }} aria-current={current(i.name) ? 'page' : undefined} counter={i.count || undefined}>
                {i.label}
              </UnderlineNavLink>
            ))}
        </UnderlineNav>
      </div>
    </div>
  );
}

/// The repository's name row on the code home, above the branch selector: the owner's avatar,
/// the name, its visibility, and watch / fork / star on the right, ending with the About column.
export function RepoTitle({ repo }: { repo: Repo }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  return (
    <div className="repo-title-row">
      <Link to="/$owner" params={{ owner: repo.owner.login }} className="owner-avatar" aria-label={repo.owner.login}>
        <Avatar src={repo.owner.avatar_url} size={24} square={repo.owner.type === 'Organization'} alt="" />
      </Link>
      <Link to="/$owner/$repo" params={p} className="name">
        {repo.name}
      </Link>
      <Label className="visibility" variant="secondary">
        {repo.private ? 'Private' : 'Public'}
      </Label>
      {repo.archived ? <Label variant="attention">Archived</Label> : null}
      {repo.is_template ? <Label variant="secondary">Template</Label> : null}
      <div className="actions">
        <RepoActions repo={repo} />
      </div>
    </div>
  );
}

function RepoActions({ repo }: { repo: Repo }) {
  const { user, orgs } = useSession();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const key = ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase()];
  const invalidate = () => qc.invalidateQueries({ queryKey: key });
  const star = useMutation({ mutationFn: (on: boolean) => api(`/user/starred/${repo.full_name}`, { method: on ? 'PUT' : 'DELETE' }), onSuccess: invalidate });
  const watch = useMutation({
    mutationFn: (mode: 'subscribed' | 'ignored' | null) => (mode === null ? api(`/repos/${repo.full_name}/subscription`, { method: 'DELETE' }) : api(`/repos/${repo.full_name}/subscription`, { method: 'PUT', json: { subscribed: mode === 'subscribed', ignored: mode === 'ignored' } })),
    onSuccess: invalidate,
  });
  const [forking, setForking] = useState(false);
  const requireLogin = () => navigate({ to: '/login', search: { return_to: location.pathname } });
  const watching = repo.viewer_subscription ?? null;
  return (
    <div className="repo-actions">
      <ActionMenu>
        <ActionMenu.Button size="small" leadingVisual={watching === 'ignored' ? BellSlashIcon : EyeIcon} count={repo.watchers_count}>
          {watching === 'subscribed' ? 'Unwatch' : watching === 'ignored' ? 'Ignoring' : 'Watch'}
        </ActionMenu.Button>
        <ActionMenu.Overlay align="end" width="medium">
          <ActionList selectionVariant="single">
            <ActionList.Item selected={watching === null} onSelect={() => (user ? watch.mutate(null) : requireLogin())}>
              Participating and @mentions
              <ActionList.Description variant="block">Only receive notifications from this repository when participating or @mentioned.</ActionList.Description>
            </ActionList.Item>
            <ActionList.Item selected={watching === 'subscribed'} onSelect={() => (user ? watch.mutate('subscribed') : requireLogin())}>
              All activity
              <ActionList.Description variant="block">Notified of all notifications on this repository.</ActionList.Description>
            </ActionList.Item>
            <ActionList.Item selected={watching === 'ignored'} onSelect={() => (user ? watch.mutate('ignored') : requireLogin())}>
              Ignore
              <ActionList.Description variant="block">Never be notified.</ActionList.Description>
            </ActionList.Item>
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      <Button size="small" leadingVisual={RepoForkedIcon} trailingVisual={TriangleDownIcon} count={repo.forks_count} onClick={() => (user ? setForking(true) : requireLogin())} disabled={repo.owner.login === user?.login && !orgs.length}>
        Fork
      </Button>
      <Button size="small" leadingVisual={repo.viewer_starred ? StarFillIcon : StarIcon} count={repo.stargazers_count} onClick={() => (user ? star.mutate(!repo.viewer_starred) : requireLogin())} className={repo.viewer_starred ? "starred" : undefined}>
        {repo.viewer_starred ? 'Starred' : 'Star'}
      </Button>
      {forking ? <ForkDialog repo={repo} onClose={() => setForking(false)} /> : null}
    </div>
  );
}

function ForkDialog({ repo, onClose }: { repo: Repo; onClose: () => void }) {
  const { user, orgs } = useSession();
  const navigate = useNavigate();
  const owners = [user!, ...orgs].filter((o) => o.login !== repo.owner.login);
  const [owner, setOwner] = useState(owners[0]?.login ?? '');
  const [name, setName] = useState(repo.name);
  const fork = useMutation({
    mutationFn: () => api<Repo>(`/repos/${repo.full_name}/forks`, { method: 'POST', json: { organization: owner === user!.login ? undefined : owner, name } }),
    onSuccess: (r) => {
      onClose();
      navigate({ to: '/$owner/$repo', params: { owner: r.owner.login, repo: r.name } });
    },
  });
  return (
    <Dialog title="Create a new fork" onClose={onClose} footerButtons={[{ content: 'Cancel', onClick: onClose }, { content: 'Create fork', buttonType: 'primary', onClick: () => fork.mutate(), loading: fork.isPending, disabled: !owner || !name.trim() }]}>
      <div className="form-stack">
        <p className="muted small" style={{ margin: 0 }}>
          A fork is a copy of a repository. Forking lets you freely experiment with changes without affecting the original project.
        </p>
        {fork.isError ? <p style={{ color: 'var(--fgColor-danger)', margin: 0 }}>{(fork.error as Error).message}</p> : null}
        <FormControl>
          <FormControl.Label>Owner</FormControl.Label>
          <select className="select" value={owner} onChange={(e) => setOwner(e.target.value)}>
            {owners.map((o) => (
              <option key={o.id} value={o.login}>
                {o.login}
              </option>
            ))}
          </select>
        </FormControl>
        <FormControl>
          <FormControl.Label>Repository name</FormControl.Label>
          <TextInput block value={name} onChange={(e) => setName(e.target.value)} />
        </FormControl>
        <p className="muted small" style={{ margin: 0 }}>
          By default, forks are named the same as their upstream repository.
        </p>
      </div>
    </Dialog>
  );
}
