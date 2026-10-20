import { Avatar, Label, RelativeTime } from '@primer/react';
import { TagIcon, DownloadIcon, GitCommitIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ButtonLink } from '../components/links';
import { ErrorFlash, Loading } from '../components/Loading';
import { Markdown } from '../components/Markdown';
import { releasesQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/releases/')({ component: Releases });

function Releases() {
  const repo = useRepo();
  const q = useQuery(releasesQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const latest = q.data.find((r) => !r.draft && !r.prerelease);
  return (
    <div>
      <div className="release-tabs">
        <Link to="/$owner/$repo/releases" params={p} className="tab active">
          Releases
        </Link>
        <Link to="/$owner/$repo/tags" params={p} className="tab">
          Tags
        </Link>
        <span style={{ flex: 1 }} />
        {repo.permissions?.push ? (
          <ButtonLink to="/$owner/$repo/releases/new" params={p} search={{}} variant="primary" size="small">
            Draft a new release
          </ButtonLink>
        ) : null}
      </div>
      {q.data.length === 0 ? (
        <Blankslate spacious>
          <Blankslate.Visual>
            <TagIcon size="medium" />
          </Blankslate.Visual>
          <Blankslate.Heading>There aren't any releases here</Blankslate.Heading>
          <Blankslate.Description>Releases package software, along with release notes and links to binary files, for other people to use.</Blankslate.Description>
          {repo.permissions?.push ? <Blankslate.PrimaryAction href={`/${repo.full_name}/releases/new`}>Create a new release</Blankslate.PrimaryAction> : null}
        </Blankslate>
      ) : (
        <div className="release-list">
          {q.data.map((r) => (
            <div key={r.id} className="release">
              <aside className="release-side">
                <div className="muted small">{r.published_at ? <RelativeTime datetime={r.published_at} /> : 'Draft'}</div>
                <Link to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev: r.tag_name, _splat: '' }} className="muted small" style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
                  <TagIcon /> {r.tag_name}
                </Link>
                <span className="muted small" style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
                  <GitCommitIcon /> {r.target_commitish}
                </span>
              </aside>
              <div className="release-body">
                <div className="release-title">
                  <Link to="/$owner/$repo/releases/tag/$tag" params={{ ...p, tag: r.tag_name }}>
                    <h2>{r.name || r.tag_name}</h2>
                  </Link>
                  {latest?.id === r.id ? <Label variant="success">Latest</Label> : null}
                  {r.prerelease ? <Label variant="attention">Pre-release</Label> : null}
                  {r.draft ? <Label variant="secondary">Draft</Label> : null}
                </div>
                {r.author ? (
                  <div className="muted small" style={{ display: 'flex', gap: 6, alignItems: 'center', margin: '4px 0 12px' }}>
                    <Avatar src={r.author.avatar_url} size={20} alt="" /> {r.author.login} released this {r.published_at ? <RelativeTime datetime={r.published_at} /> : null}
                  </div>
                ) : null}
                {r.body_html ? <Markdown html={r.body_html} /> : null}
                <details className="release-assets">
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
          ))}
        </div>
      )}
    </div>
  );
}
