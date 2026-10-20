// Issues or pull requests across every repository, for the signed-in person:
// created by, assigned to, or mentioning them (GitHub's /issues and /pulls).
import { Button, ButtonGroup } from '@primer/react';
import { IssueOpenedIcon, IssueClosedIcon, GitPullRequestIcon, GitMergeIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { AppHeader } from './AppHeader';
import { CrustyEmpty } from './Crusty';
import { IssueRow } from './IssueBits';
import { ErrorFlash, Loading } from './Loading';
import { api } from '../lib/api';
import type { Issue } from '../lib/issues';

export type IssueFilter = 'created' | 'assigned' | 'mentioned';
export type IssueState = 'open' | 'closed';
export interface GlobalIssuesSearch {
  filter?: IssueFilter;
  state?: IssueState;
}

export function parseGlobalIssuesSearch(s: Record<string, unknown>): GlobalIssuesSearch {
  const out: GlobalIssuesSearch = {};
  if (s.filter === 'created' || s.filter === 'assigned' || s.filter === 'mentioned') out.filter = s.filter;
  if (s.state === 'open' || s.state === 'closed') out.state = s.state;
  return out;
}

const FILTERS: [IssueFilter, string][] = [
  ['created', 'Created'],
  ['assigned', 'Assigned'],
  ['mentioned', 'Mentioned'],
];

export function GlobalIssues({ kind, search, onSearch }: { kind: 'issues' | 'pulls'; search: GlobalIssuesSearch; onSearch: (s: GlobalIssuesSearch) => void }) {
  const filter = search.filter ?? 'created';
  const state = search.state ?? 'open';
  const q = useQuery({
    queryKey: ['global', kind, filter, state],
    queryFn: () => api<Issue[]>(`/issues?filter=${filter}&state=${state}&sort=updated&per_page=100`),
  });
  const pulls = kind === 'pulls';
  const items = (q.data ?? []).filter((i) => !!i.pull_request === pulls && i.repository);
  // Grouped by repository, as the rows link into each repository's own list.
  const groups = new Map<string, Issue[]>();
  for (const i of items) {
    const key = i.repository!.full_name;
    groups.set(key, [...(groups.get(key) ?? []), i]);
  }
  const title = pulls ? 'Pull requests' : 'Issues';
  const OpenIcon = pulls ? GitPullRequestIcon : IssueOpenedIcon;
  const ClosedIcon = pulls ? GitMergeIcon : IssueClosedIcon;
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap', marginBottom: 16 }}>
          <h1 style={{ fontSize: 20, fontWeight: 600, margin: 0 }}>{title}</h1>
          <ButtonGroup>
            {FILTERS.map(([f, label]) => (
              <Button key={f} size="small" variant={f === filter ? 'primary' : 'default'} onClick={() => onSearch({ ...search, filter: f })}>
                {label}
              </Button>
            ))}
          </ButtonGroup>
          <span style={{ flex: 1 }} />
          <ButtonGroup>
            <Button size="small" variant={state === 'open' ? 'primary' : 'default'} leadingVisual={OpenIcon} onClick={() => onSearch({ ...search, state: 'open' })}>
              Open
            </Button>
            <Button size="small" variant={state === 'closed' ? 'primary' : 'default'} leadingVisual={ClosedIcon} onClick={() => onSearch({ ...search, state: 'closed' })}>
              Closed
            </Button>
          </ButtonGroup>
        </div>
        {q.isPending ? <Loading /> : null}
        {q.isError ? <ErrorFlash error={q.error} /> : null}
        {q.data && items.length === 0 ? (
          <CrustyEmpty mood={state === 'open' ? 'resting' : 'peekaboo'} title={`No ${state} ${pulls ? 'pull requests' : 'issues'} ${filter === 'created' ? 'you created' : filter === 'assigned' ? 'assigned to you' : 'mentioning you'}.`}>
            {pulls ? 'Pull requests across every repository you can see show up here.' : 'Issues across every repository you can see show up here.'}
          </CrustyEmpty>
        ) : null}
        {[...groups.entries()].map(([full, list]) => {
          const repo = list[0].repository!;
          return (
            <section key={full} className="card" style={{ marginBottom: 16 }}>
              <div className="card-header" style={{ display: 'flex', alignItems: 'center', gap: 8, fontWeight: 600 }}>
                <Link to="/$owner/$repo" params={{ owner: repo.owner.login, repo: repo.name }}>
                  {full}
                </Link>
                <span className="muted small">{list.length}</span>
              </div>
              {list.map((i) => (
                <IssueRow key={i.id} issue={i} repo={repo} />
              ))}
            </section>
          );
        })}
      </div>
    </>
  );
}
