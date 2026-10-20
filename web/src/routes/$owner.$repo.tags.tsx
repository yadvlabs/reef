import { Button } from '@primer/react';
import { TagIcon, DownloadIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ButtonLink } from '../components/links';
import { ErrorFlash, Loading } from '../components/Loading';
import { releasesQuery, tagsQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/tags')({ component: Tags });

function Tags() {
  const repo = useRepo();
  const q = useQuery(tagsQuery(repo.owner.login, repo.name));
  const releases = useQuery(releasesQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  return (
    <div>
      <div className="release-tabs">
        <Link to="/$owner/$repo/releases" params={p} className="tab">
          Releases
        </Link>
        <Link to="/$owner/$repo/tags" params={p} className="tab active">
          Tags
        </Link>
      </div>
      <div className="key-list">
        {q.data.map((t) => {
          const rel = releases.data?.find((r) => r.tag_name === t.name);
          return (
            <div key={t.name} className="item" style={{ alignItems: 'center' }}>
              <div className="body" style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <TagIcon className="muted" />
                <Link to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev: t.name, _splat: '' }} style={{ fontWeight: 600 }}>
                  {t.name}
                </Link>
                <code className="muted small">{t.commit.sha.slice(0, 7)}</code>
                {rel ? (
                  <Link to="/$owner/$repo/releases/tag/$tag" params={{ ...p, tag: t.name }} className="small">
                    Release notes
                  </Link>
                ) : repo.permissions?.push ? (
                  <Link to="/$owner/$repo/releases/new" params={p} search={{ tag: t.name }} className="small">
                    Create release
                  </Link>
                ) : null}
              </div>
              <Button as="a" size="small" variant="invisible" leadingVisual={DownloadIcon} {...({ href: `/api/v3/repos/${repo.full_name}/zipball/${encodeURIComponent(t.name)}` } as object)}>
                zip
              </Button>
              <Button as="a" size="small" variant="invisible" leadingVisual={DownloadIcon} {...({ href: `/api/v3/repos/${repo.full_name}/tarball/${encodeURIComponent(t.name)}` } as object)}>
                tar.gz
              </Button>
            </div>
          );
        })}
        {q.data.length === 0 ? (
          <Blankslate>
            <Blankslate.Visual>
              <TagIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>There aren't any tags yet</Blankslate.Heading>
            <Blankslate.Description>Tags mark points in history; releases add notes to them.</Blankslate.Description>
          </Blankslate>
        ) : null}
      </div>
      {repo.permissions?.push ? (
        <div style={{ marginTop: 16 }}>
          <ButtonLink to="/$owner/$repo/releases/new" params={p} search={{}} variant="primary">
            Create a new release
          </ButtonLink>
        </div>
      ) : null}
    </div>
  );
}
