import { Avatar, RelativeTime } from '@primer/react';
import { ButtonLink } from '../components/links';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ErrorFlash, Loading } from '../components/Loading';
import { commitDetailQuery } from '../lib/pulls';
import { FilesChanged } from '../components/Diff';
import { VerifiedBadge } from '../components/identity/VerifiedBadge';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/commit/$sha')({ component: CommitPage });

function CommitPage() {
  const repo = useRepo();
  const { sha } = Route.useParams();
  const q = useQuery(commitDetailQuery(repo.full_name, sha));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const c = q.data;
  const [title, ...rest] = c.commit.message.split('\n');
  const body = rest.join('\n').trim();
  return (
    <div>
      <div style={{ border: '1px solid var(--borderColor-default)', borderRadius: 6, padding: 16, marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
          <div style={{ flex: 1 }}>
            <h1 style={{ fontSize: 18, margin: 0 }}>{title}</h1>
            {body ? <pre style={{ whiteSpace: 'pre-wrap', fontFamily: 'inherit', color: 'var(--fgColor-muted)', margin: '8px 0 0' }}>{body}</pre> : null}
          </div>
          <ButtonLink to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev: c.sha, _splat: '' }} size="small">
            Browse files
          </ButtonLink>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 12, fontSize: 12, color: 'var(--fgColor-muted)' }}>
          {c.author ? (
            <>
              <Avatar src={c.author.avatar_url} size={20} alt="" />
              <Link to="/$owner" params={{ owner: c.author.login }}>
                <strong>{c.author.login}</strong>
              </Link>
            </>
          ) : (
            <>
              <Avatar src={`/avatars/${encodeURIComponent(c.commit.author.name)}`} size={20} alt="" />
              <strong>{c.commit.author.name}</strong>
            </>
          )}
          <span>
            committed <RelativeTime datetime={c.commit.committer.date} />
          </span>
          <VerifiedBadge commit={c} />
          <span style={{ flex: 1 }} />
          <span>
            commit <code>{c.sha}</code>
          </span>
          <span>
            <a href={`/${repo.full_name}/commit/${c.sha}.patch`}>patch</a> · <a href={`/${repo.full_name}/commit/${c.sha}.diff`}>diff</a>
          </span>
          {c.parents.length ? (
            <span>
              {c.parents.length === 1 ? 'parent' : 'parents'}{' '}
              {c.parents.map((pa) => (
                <Link key={pa.sha} to="/$owner/$repo/commit/$sha" params={{ ...p, sha: pa.sha }} style={{ marginLeft: 4 }}>
                  <code>{pa.sha.slice(0, 7)}</code>
                </Link>
              ))}
            </span>
          ) : null}
        </div>
      </div>
      <FilesChanged files={c.files ?? []} />
    </div>
  );
}
