import { Button, TextInput } from '@primer/react';
import { IssueOpenedIcon, IssueClosedIcon, SearchIcon, TagIcon, MilestoneIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { ButtonLink } from '../components/links';
import { IssueFilters, IssueRow } from '../components/IssueBits';
import { ErrorFlash, Loading } from '../components/Loading';
import { issueListQuery } from '../lib/issues';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/issues/')({
  validateSearch: (s: Record<string, unknown>): { q?: string; page?: number } => ({ ...(typeof s.q === 'string' ? { q: s.q } : {}), ...(typeof s.page === 'number' ? { page: s.page } : {}) }),
  component: IssuesIndex,
});

/** GitHub-style filter string: `is:issue is:open label:bug author:me sort:updated-desc words`. */
export function parseFilter(q: string): Record<string, string | undefined> & { labels: string; q: string } {
  const p: Record<string, string> = {};
  const words: string[] = [];
  const labels: string[] = [];
  for (const tok of q.split(/\s+/).filter(Boolean)) {
    const m = tok.match(/^(is|state|label|author|assignee|milestone|sort|no|type):(.+)$/);
    if (!m) {
      words.push(tok);
      continue;
    }
    const [, k, v] = m;
    const val = v.replace(/^"|"$/g, '');
    if (k === 'is' || k === 'state') {
      if (val === 'open' || val === 'closed') p.state = val;
      if (val === 'issue') p.type = 'issue';
      if (val === 'pr') p.type = 'pr';
    } else if (k === 'label') labels.push(val);
    else if (k === 'author') p.creator = val;
    else if (k === 'assignee') p.assignee = val;
    else if (k === 'milestone') p.milestone = val;
    else if (k === 'no') {
      if (val === 'assignee') p.assignee = 'none';
      if (val === 'milestone') p.milestone = 'none';
      if (val === 'label') p.labels = '';
    } else if (k === 'sort') {
      const [f, d] = val.split('-');
      p.sort = f;
      p.direction = d ?? 'desc';
    } else if (k === 'type') p.type = val;
  }
  return { ...p, labels: labels.join(','), q: words.join(' ') };
}

function IssuesIndex() {
  const repo = useRepo();
  const { q = 'is:issue is:open', page = 1 } = Route.useSearch();
  const navigate = useNavigate();
  const [draft, setDraft] = useState(q);
  const filter = parseFilter(q);
  const list = useQuery(issueListQuery(repo.full_name, { ...filter, state: filter.state ?? 'open', type: filter.type ?? 'issue', page }));
  const openCount = useQuery(issueListQuery(repo.full_name, { ...filter, state: 'open', type: 'issue', page: 1 }));
  const closedCount = useQuery(issueListQuery(repo.full_name, { ...filter, state: 'closed', type: 'issue', page: 1 }));
  const p = { owner: repo.owner.login, repo: repo.name };
  const setQ = (nq: string) => navigate({ to: '/$owner/$repo/issues', params: p, search: { q: nq, page: 1 } });
  const state = filter.state ?? 'open';
  const replaceTok = (prefix: string, val: string | null) => {
    const rest = q.split(/\s+/).filter((t) => t && !t.startsWith(prefix + ':'));
    return [...rest, ...(val ? [`${prefix}:${val.includes(' ') ? `"${val}"` : val}`] : [])].join(' ');
  };
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
          <TextInput block leadingVisual={SearchIcon} value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Search all issues" aria-label="Search issues" />
        </form>
        <ButtonLink to="/$owner/$repo/labels" params={p} leadingVisual={TagIcon}>
          Labels
        </ButtonLink>
        <ButtonLink to="/$owner/$repo/milestones" params={p} leadingVisual={MilestoneIcon}>
          Milestones
        </ButtonLink>
        <ButtonLink to="/$owner/$repo/issues/new" params={p} variant="primary">
          New issue
        </ButtonLink>
      </div>
      {list.isError ? <ErrorFlash error={list.error} /> : null}
      <div className="issue-list">
        <div className="list-header">
          <button className="linkish" style={{ color: state === 'open' ? 'var(--fgColor-default)' : 'var(--fgColor-muted)', fontWeight: 600, display: 'flex', gap: 4, alignItems: 'center' }} onClick={() => setQ(replaceTok('is', 'open').replace(/\bis:closed\b/, ''))}>
            <IssueOpenedIcon /> {openCount.data?.total ?? '…'} Open
          </button>
          <button className="linkish" style={{ color: state === 'closed' ? 'var(--fgColor-default)' : 'var(--fgColor-muted)', fontWeight: 600, display: 'flex', gap: 4, alignItems: 'center' }} onClick={() => setQ(replaceTok('is', 'closed').replace(/\bis:open\b/, ''))}>
            <IssueClosedIcon /> {closedCount.data?.total ?? '…'} Closed
          </button>
          <IssueFilters repo={repo} q={q} setQ={setQ} sort={filter.sort} direction={filter.direction} />
        </div>
        {list.isPending ? <Loading /> : null}
        {list.data?.items.map((i) => (
          <IssueRow key={i.id} issue={i} repo={repo} />
        ))}
        {list.data && list.data.items.length === 0 ? (
          <Blankslate>
            <Blankslate.Visual>
              <IssueOpenedIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>{q.includes('is:closed') ? 'No closed issues' : 'Welcome to issues!'}</Blankslate.Heading>
            <Blankslate.Description>Issues are used to track todos, bugs, feature requests, and more. As issues are created, they'll appear here in a searchable and filterable list.</Blankslate.Description>
            <Blankslate.PrimaryAction href={`/${repo.full_name}/issues/new`}>New issue</Blankslate.PrimaryAction>
          </Blankslate>
        ) : null}
      </div>
      {list.data && list.data.total > 25 ? (
        <div style={{ display: 'flex', justifyContent: 'center', gap: 8, marginTop: 16 }}>
          <Button as={Link} disabled={page <= 1} {...({ to: '/$owner/$repo/issues', params: p, search: { q, page: page - 1 } } as object)}>
            Previous
          </Button>
          <Button as={Link} disabled={page * 25 >= list.data.total} {...({ to: '/$owner/$repo/issues', params: p, search: { q, page: page + 1 } } as object)}>
            Next
          </Button>
        </div>
      ) : null}
    </div>
  );
}
