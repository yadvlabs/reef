import { Avatar, Button, Label, RelativeTime } from '@primer/react';
import { TagIcon, DownloadIcon, GitCommitIcon, PencilIcon, TrashIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { ButtonLink } from '../components/links';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { Markdown } from '../components/Markdown';
import { api, ApiError } from '../lib/api';
import { releaseByTagQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/releases/tag/$tag')({ component: ReleasePage });

function ReleasePage() {
  const repo = useRepo();
  const { tag } = Route.useParams();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const q = useQuery(releaseByTagQuery(repo.owner.login, repo.name, tag));
  const p = { owner: repo.owner.login, repo: repo.name };
  const del = useMutation({
    mutationFn: () => api(`/repos/${repo.full_name}/releases/${q.data!.id}`, { method: 'DELETE' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase()] });
      navigate({ to: '/$owner/$repo/releases', params: p });
    },
  });
  if (q.isPending) return <Loading />;
  if (q.isError) return q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="release" /> : <ErrorFlash error={q.error} />;
  const r = q.data;
  return (
    <div className="release single">
      <aside className="release-side">
        <Link to="/$owner/$repo/releases" params={p} className="small">
          ← All releases
        </Link>
        <Link to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev: r.tag_name, _splat: '' }} className="muted small" style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <TagIcon /> {r.tag_name}
        </Link>
        <span className="muted small" style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <GitCommitIcon /> {r.target_commitish}
        </span>
      </aside>
      <div className="release-body">
        <div className="release-title">
          <h2>{r.name || r.tag_name}</h2>
          {r.prerelease ? <Label variant="attention">Pre-release</Label> : null}
          {r.draft ? <Label variant="secondary">Draft</Label> : null}
          <span style={{ flex: 1 }} />
          {repo.permissions?.push ? (
            <>
              <ButtonLink to="/$owner/$repo/releases/new" params={p} search={{ edit: r.id }} size="small" leadingVisual={PencilIcon}>
                Edit
              </ButtonLink>
              <Button size="small" variant="danger" leadingVisual={TrashIcon} onClick={() => confirm('Delete this release? The tag stays.') && del.mutate()}>
                Delete
              </Button>
            </>
          ) : null}
        </div>
        {del.isError ? <ErrorFlash error={del.error} /> : null}
        {r.author ? (
          <div className="muted small" style={{ display: 'flex', gap: 6, alignItems: 'center', margin: '4px 0 12px' }}>
            <Avatar src={r.author.avatar_url} size={20} alt="" /> {r.author.login} released this {r.published_at ? <RelativeTime datetime={r.published_at} /> : null}
          </div>
        ) : null}
        {r.body_html ? <Markdown html={r.body_html} /> : <p className="muted">No release notes.</p>}
        <details className="release-assets" open>
          <summary>Assets <span className="counter">2</span></summary>
          <div className="asset-list">
            <a href={`/api/v3/repos/${repo.full_name}/zipball/${encodeURIComponent(r.tag_name)}`}>
              <DownloadIcon /> Source code (zip)
            </a>
            <a href={`/api/v3/repos/${repo.full_name}/tarball/${encodeURIComponent(r.tag_name)}`}>
              <DownloadIcon /> Source code (tar.gz)
            </a>
          </div>
        </details>
      </div>
    </div>
  );
}
