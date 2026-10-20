import { Avatar, RelativeTime } from '@primer/react';
import { ButtonLink } from '../components/links';
import { GitCommitIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { BranchMenu } from '../components/RepoCode';
import { ErrorFlash, Loading } from '../components/Loading';
import { commitsQuery } from '../lib/repo';
import type { Commit } from '../lib/api';
import { VerifiedBadge } from '../components/identity/VerifiedBadge';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/commits/$rev')({
  validateSearch: (s: Record<string, unknown>): { path?: string; page?: number } => ({ ...(typeof s.path === 'string' ? { path: s.path } : {}), ...(typeof s.page === 'number' ? { page: s.page } : {}) }),
  component: Commits,
});

function Commits() {
  const repo = useRepo();
  const { rev } = Route.useParams();
  const { path, page = 1 } = Route.useSearch();
  const per = 35;
  const q = useQuery(commitsQuery(repo.owner.login, repo.name, rev, path, page, per));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const groups = new Map<string, Commit[]>();
  for (const c of q.data) {
    const day = new Date(c.commit.committer.date).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
    groups.set(day, [...(groups.get(day) ?? []), c]);
  }
  return (
    <div className="commit-list">
      <div className="code-toolbar">
        <BranchMenu repo={repo} rev={rev} path={path ?? ''} mode="tree" />
        {path ? <span className="muted">History for <code>{path}</code></span> : null}
      </div>
      {[...groups.entries()].map(([day, cs]) => (
        <div key={day}>
          <div className="day">
            <GitCommitIcon /> Commits on {day}
          </div>
          <div className="group">
            {cs.map((c) => (
              <div key={c.sha} className="commit">
                <div className="body">
                  <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: c.sha }} className="title" title={c.commit.message}>
                    {c.commit.message.split('\n')[0]}
                  </Link>
                  <div className="who">
                    {c.author ? (
                      <>
                        <Avatar src={c.author.avatar_url} size={16} alt="" />
                        <Link to="/$owner" params={{ owner: c.author.login }}>
                          <strong>{c.author.login}</strong>
                        </Link>
                      </>
                    ) : (
                      <>
                        <Avatar src={`/avatars/${encodeURIComponent(c.commit.author.name)}`} size={16} alt="" />
                        <strong>{c.commit.author.name}</strong>
                      </>
                    )}
                    committed <RelativeTime datetime={c.commit.committer.date} />
                  </div>
                </div>
                <VerifiedBadge commit={c} />
                <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: c.sha }} className="sha">
                  {c.sha.slice(0, 7)}
                </Link>
                <ButtonLink to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev: c.sha, _splat: '' }} size="small" variant="invisible">
                  Browse
                </ButtonLink>
              </div>
            ))}
          </div>
        </div>
      ))}
      <div style={{ display: 'flex', justifyContent: 'center', gap: 8, marginTop: 16 }}>
        <ButtonLink to="/$owner/$repo/commits/$rev" params={{ ...p, rev }} search={{ path, page: Math.max(1, page - 1) }} disabled={page <= 1}>
          Newer
        </ButtonLink>
        <ButtonLink to="/$owner/$repo/commits/$rev" params={{ ...p, rev }} search={{ path, page: page + 1 }} disabled={q.data.length < per}>
          Older
        </ButtonLink>
      </div>
    </div>
  );
}
