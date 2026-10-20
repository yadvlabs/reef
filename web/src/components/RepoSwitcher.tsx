// A dropdown next to the repository name in the header that jumps to another repository of
// the same owner or one of the viewer's own, with a filter box.
import { ActionList, ActionMenu, TextInput } from '@primer/react';
import { LockIcon, RepoIcon, SearchIcon, TriangleDownIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo } from '../lib/api';
import { useSession } from '../lib/session';

export function RepoSwitcher({ owner, repo }: { owner: string; repo: string }) {
  const { user } = useSession();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState('');
  const ownerRepos = useQuery({
    queryKey: ['account', owner.toLowerCase(), 'repos', 'switcher'],
    queryFn: () => api<Repo[]>(`/users/${owner}/repos?sort=updated&per_page=50`),
    enabled: open,
    staleTime: 60_000,
  });
  const mine = useQuery({
    queryKey: ['user', 'repos', 'switcher'],
    queryFn: () => api<Repo[]>(`/user/repos?sort=updated&per_page=50`),
    enabled: open && !!user,
    staleTime: 60_000,
  });
  const needle = q.trim().toLowerCase();
  const filter = (list: Repo[] | undefined) => (list ?? []).filter((r) => r.full_name.toLowerCase().includes(needle));
  const own = filter(ownerRepos.data);
  const others = filter(mine.data).filter((r) => r.owner.login.toLowerCase() !== owner.toLowerCase());
  const go = (r: Repo) => {
    setOpen(false);
    navigate({ to: '/$owner/$repo', params: { owner: r.owner.login, repo: r.name } });
  };
  return (
    <ActionMenu open={open} onOpenChange={setOpen}>
      <ActionMenu.Anchor>
        <button className="repo-switcher" aria-label="Switch repository" title="Switch repository">
          <TriangleDownIcon size={16} />
        </button>
      </ActionMenu.Anchor>
      <ActionMenu.Overlay width="medium" align="start">
        <div style={{ padding: 8 }}>
          <TextInput leadingVisual={SearchIcon} placeholder="Find a repository…" value={q} onChange={(e) => setQ(e.target.value)} block size="small" autoFocus aria-label="Find a repository" />
        </div>
        <ActionList selectionVariant="single" showDividers>
          <ActionList.Group>
            <ActionList.GroupHeading>{owner}</ActionList.GroupHeading>
            {own.map((r) => (
              <ActionList.Item key={r.id} selected={r.name.toLowerCase() === repo.toLowerCase()} onSelect={() => go(r)}>
                <ActionList.LeadingVisual>{r.private ? <LockIcon /> : <RepoIcon />}</ActionList.LeadingVisual>
                {r.name}
              </ActionList.Item>
            ))}
            {ownerRepos.isPending && open ? <ActionList.Item disabled>Loading…</ActionList.Item> : null}
            {own.length === 0 && ownerRepos.data ? <ActionList.Item disabled>No matching repositories</ActionList.Item> : null}
          </ActionList.Group>
          {others.length ? (
            <ActionList.Group>
              <ActionList.GroupHeading>Your repositories</ActionList.GroupHeading>
              {others.map((r) => (
                <ActionList.Item key={r.id} onSelect={() => go(r)}>
                  <ActionList.LeadingVisual>{r.private ? <LockIcon /> : <RepoIcon />}</ActionList.LeadingVisual>
                  {r.full_name}
                </ActionList.Item>
              ))}
            </ActionList.Group>
          ) : null}
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}
