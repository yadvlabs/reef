import { Label } from '@primer/react';
import { ButtonLink } from '../components/links';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ErrorFlash, Loading } from '../components/Loading';
import { branchesQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/branches')({ component: Branches });

function Branches() {
  const repo = useRepo();
  const q = useQuery(branchesQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const sorted = [...q.data].sort((a, b) => (a.name === repo.default_branch ? -1 : b.name === repo.default_branch ? 1 : a.name.localeCompare(b.name)));
  return (
    <div>
      <h2 className="section-title">Branches</h2>
      <div className="key-list">
        {sorted.map((b) => (
          <div key={b.name} className="item" style={{ alignItems: 'center' }}>
            <div className="body" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Link to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev: b.name, _splat: '' }} className="monospace">
                {b.name}
              </Link>
              {b.name === repo.default_branch ? <Label>Default</Label> : null}
            </div>
            <code className="muted small">{b.commit.sha.slice(0, 7)}</code>
            <ButtonLink to="/$owner/$repo/commits/$rev" params={{ ...p, rev: b.name }} size="small" variant="invisible">
              Commits
            </ButtonLink>
          </div>
        ))}
        {sorted.length === 0 ? <p className="muted">No branches yet.</p> : null}
      </div>
    </div>
  );
}
