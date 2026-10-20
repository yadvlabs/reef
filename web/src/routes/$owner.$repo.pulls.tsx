import { Button, TextInput } from '@primer/react';
import { GitPullRequestIcon, GitPullRequestClosedIcon, SearchIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { ButtonLink } from '../components/links';
import { IssueFilters, IssueRow } from '../components/IssueBits';
import { ErrorFlash, Loading } from '../components/Loading';
import { issueListQuery } from '../lib/issues';
import { parseFilter } from './$owner.$repo.issues.index';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/pulls')({
  validateSearch: (s: Record<string, unknown>): { q?: string; page?: number } => ({ ...(typeof s.q === 'string' ? { q: s.q } : {}), ...(typeof s.page === 'number' ? { page: s.page } : {}) }),
  component: Pulls,
});

function Pulls() {
  const repo = useRepo();
  const { q = 'is:pr is:open', page = 1 } = Route.useSearch();
  const navigate = useNavigate();
  const [draft, setDraft] = useState(q);
  const filter = parseFilter(q);
  const list = useQuery(issueListQuery(repo.full_name, { ...filter, state: filter.state ?? 'open', type: 'pr', page }));
  const openCount = useQuery(issueListQuery(repo.full_name, { ...filter, state: 'open', type: 'pr', page: 1 }));
  const closedCount = useQuery(issueListQuery(repo.full_name, { ...filter, state: 'closed', type: 'pr', page: 1 }));
  const p = { owner: repo.owner.login, repo: repo.name };
  const setQ = (nq: string) => navigate({ to: '/$owner/$repo/pulls', params: p, search: { q: nq, page: 1 } });
  const state = filter.state ?? 'open';
  return (
    <div>
      <div style={{ display: 'flex', gap: 8, marginBottom: 16, alignItems: 'center' }}>
        <form
          style={{ flex: 1 }}
          onSubmit={(e) => {
            e.preventDefault();
            setQ(draft);
          }}
        >
          <TextInput block leadingVisual={SearchIcon} value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Search all pull requests" aria-label="Search pull requests" />
        </form>
        <ButtonLink to="/$owner/$repo/compare/$" params={{ ...p, _splat: '' }} variant="primary">
          New pull request
        </ButtonLink>
      </div>
      {list.isError ? <ErrorFlash error={list.error} /> : null}
      <div className="issue-list">
        <div className="list-header">
          <button className="linkish" style={{ color: state === 'open' ? 'var(--fgColor-default)' : 'var(--fgColor-muted)', fontWeight: 600, display: 'flex', gap: 4, alignItems: 'center' }} onClick={() => setQ('is:pr is:open')}>
            <GitPullRequestIcon /> {openCount.data?.total ?? '…'} Open
          </button>
          <button className="linkish" style={{ color: state === 'closed' ? 'var(--fgColor-default)' : 'var(--fgColor-muted)', fontWeight: 600, display: 'flex', gap: 4, alignItems: 'center' }} onClick={() => setQ('is:pr is:closed')}>
            <GitPullRequestClosedIcon /> {closedCount.data?.total ?? '…'} Closed
          </button>
          <IssueFilters repo={repo} q={q} setQ={setQ} sort={filter.sort} direction={filter.direction} isPr />
        </div>
        {list.isPending ? <Loading /> : null}
        {list.data?.items.map((i) => (
          <IssueRow key={i.id} issue={i} repo={repo} />
        ))}
        {list.data && list.data.items.length === 0 ? (
          <Blankslate>
            <Blankslate.Visual>
              <GitPullRequestIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>Welcome to pull requests!</Blankslate.Heading>
            <Blankslate.Description>Pull requests help you collaborate on code with other people. As pull requests are created, they'll appear here in a searchable and filterable list.</Blankslate.Description>
          </Blankslate>
        ) : null}
      </div>
      {list.data && list.data.total > 25 ? (
        <div style={{ display: 'flex', justifyContent: 'center', gap: 8, marginTop: 16 }}>
          <Button as={Link} disabled={page <= 1} {...({ to: '/$owner/$repo/pulls', params: p, search: { q, page: page - 1 } } as object)}>
            Previous
          </Button>
          <Button as={Link} disabled={page * 25 >= list.data.total} {...({ to: '/$owner/$repo/pulls', params: p, search: { q, page: page + 1 } } as object)}>
            Next
          </Button>
        </div>
      ) : null}
    </div>
  );
}
