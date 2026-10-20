// The repository list controls GitHub shows on profiles and organizations: a search box,
// Type / Language / Sort menus, and the "New" button for people who may create one. The list
// itself is filtered client side, since owner lists are fetched whole.
import { ActionList, ActionMenu, TextInput } from '@primer/react';
import { RepoIcon, SearchIcon } from '@primer/octicons-react';
import type { Repo } from '../lib/api';
import { ButtonLink } from './links';

export type RepoType = 'all' | 'public' | 'private' | 'sources' | 'forks' | 'archived' | 'templates';
export type RepoSort = 'updated' | 'name' | 'stars';

export interface RepoFilter {
  q: string;
  type: RepoType;
  language: string;
  sort: RepoSort;
}

export const DEFAULT_FILTER: RepoFilter = { q: '', type: 'all', language: '', sort: 'updated' };

const TYPES: { id: RepoType; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'public', label: 'Public' },
  { id: 'private', label: 'Private' },
  { id: 'sources', label: 'Sources' },
  { id: 'forks', label: 'Forks' },
  { id: 'archived', label: 'Archived' },
  { id: 'templates', label: 'Templates' },
];
const SORTS: { id: RepoSort; label: string }[] = [
  { id: 'updated', label: 'Last updated' },
  { id: 'name', label: 'Name' },
  { id: 'stars', label: 'Stars' },
];

export function filterRepos(repos: Repo[], f: RepoFilter): Repo[] {
  const q = f.q.trim().toLowerCase();
  const out = repos.filter((r) => {
    if (q && !r.name.toLowerCase().includes(q) && !(r.description ?? '').toLowerCase().includes(q)) return false;
    if (f.language && r.language !== f.language) return false;
    switch (f.type) {
      case 'public':
        return !r.private;
      case 'private':
        return r.private;
      case 'sources':
        return !r.fork;
      case 'forks':
        return !!r.fork;
      case 'archived':
        return !!r.archived;
      case 'templates':
        return !!r.is_template;
      default:
        return true;
    }
  });
  const time = (r: Repo) => new Date(r.pushed_at ?? r.updated_at).getTime();
  return out.sort((a, b) => (f.sort === 'name' ? a.name.localeCompare(b.name) : f.sort === 'stars' ? b.stargazers_count - a.stargazers_count || time(b) - time(a) : time(b) - time(a)));
}

export function RepoFilterBar({ repos, value, onChange, newRepoOwner, canCreate, newLabel = 'New' }: { repos: Repo[]; value: RepoFilter; onChange: (f: RepoFilter) => void; newRepoOwner?: string; canCreate?: boolean; newLabel?: string }) {
  const languages = [...new Set(repos.map((r) => r.language).filter((l): l is string => !!l))].sort();
  const typeLabel = TYPES.find((t) => t.id === value.type)?.label ?? 'All';
  const sortLabel = SORTS.find((s) => s.id === value.sort)?.label ?? 'Last updated';
  return (
    <div className="repo-filter-bar">
      <TextInput leadingVisual={SearchIcon} placeholder="Find a repository…" aria-label="Find a repository" value={value.q} onChange={(e) => onChange({ ...value, q: e.target.value })} block className="repo-filter-search" />
      <ActionMenu>
        <ActionMenu.Button>{value.type === 'all' ? 'Type' : `Type: ${typeLabel}`}</ActionMenu.Button>
        <ActionMenu.Overlay>
          <ActionList selectionVariant="single">
            <ActionList.Group>
              <ActionList.GroupHeading>Select type</ActionList.GroupHeading>
              {TYPES.map((t) => (
                <ActionList.Item key={t.id} selected={value.type === t.id} onSelect={() => onChange({ ...value, type: t.id })}>
                  {t.label}
                </ActionList.Item>
              ))}
            </ActionList.Group>
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      <ActionMenu>
        <ActionMenu.Button>{value.language ? `Language: ${value.language}` : 'Language'}</ActionMenu.Button>
        <ActionMenu.Overlay>
          <ActionList selectionVariant="single">
            <ActionList.Group>
              <ActionList.GroupHeading>Select language</ActionList.GroupHeading>
              <ActionList.Item selected={!value.language} onSelect={() => onChange({ ...value, language: '' })}>
                All
              </ActionList.Item>
              {languages.map((l) => (
                <ActionList.Item key={l} selected={value.language === l} onSelect={() => onChange({ ...value, language: l })}>
                  {l}
                </ActionList.Item>
              ))}
            </ActionList.Group>
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      <ActionMenu>
        <ActionMenu.Button>{value.sort === 'updated' ? 'Sort' : `Sort: ${sortLabel}`}</ActionMenu.Button>
        <ActionMenu.Overlay>
          <ActionList selectionVariant="single">
            <ActionList.Group>
              <ActionList.GroupHeading>Select order</ActionList.GroupHeading>
              {SORTS.map((s) => (
                <ActionList.Item key={s.id} selected={value.sort === s.id} onSelect={() => onChange({ ...value, sort: s.id })}>
                  {s.label}
                </ActionList.Item>
              ))}
            </ActionList.Group>
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      {canCreate ? (
        <ButtonLink to="/new" search={newRepoOwner ? { owner: newRepoOwner } : {}} variant="primary" leadingVisual={RepoIcon}>
          {newLabel}
        </ButtonLink>
      ) : null}
    </div>
  );
}
