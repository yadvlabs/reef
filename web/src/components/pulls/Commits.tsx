// The Commits tab: the pull request's commits grouped by day, like the repository history.
import { Avatar, RelativeTime } from '@primer/react';
import { GitCommitIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import type { Commit, Repo } from '../../lib/api';
import { pullCommitsQuery, type Pull } from '../../lib/pulls';
import { ErrorFlash, Loading } from '../Loading';
import { ButtonLink } from '../links';

export function PullCommits({ repo, pr }: { repo: Repo; pr: Pull }) {
  const q = useQuery(pullCommitsQuery(repo.full_name, pr.number));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const groups = new Map<string, Commit[]>();
  for (const c of q.data) {
    const day = new Date(c.commit.committer.date).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
    groups.set(day, [...(groups.get(day) ?? []), c]);
  }
  return (
    <div className="commit-list pr-commits">
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
                      <strong>{c.commit.author.name}</strong>
                    )}
                    committed <RelativeTime datetime={c.commit.committer.date} />
                  </div>
                </div>
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
    </div>
  );
}
