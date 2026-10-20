// The repository "About" sidebar: description, topics, links, releases, packages,
// contributors and languages, plus the edit dialog for admins.
import { Avatar, Dialog, FormControl, IconButton, Label, RelativeTime, TextInput, Textarea } from '@primer/react';
import { LinkIcon, StarIcon, RepoForkedIcon, EyeIcon, TagIcon, GearIcon, BookIcon, PackageIcon, PulseIcon, LawIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type Repo } from '../lib/api';
import { contributorsQuery, LANGUAGE_COLORS, languagesQuery, releasesQuery } from '../lib/repo';

export function RepoSidebar({ repo, empty }: { repo: Repo; empty: boolean }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  const releases = useQuery({ ...releasesQuery(repo.owner.login, repo.name), enabled: !empty });
  const contributors = useQuery({ ...contributorsQuery(repo.owner.login, repo.name), enabled: !empty });
  const languages = useQuery({ ...languagesQuery(repo.owner.login, repo.name), enabled: !empty });
  const [editing, setEditing] = useState(false);
  const canAdmin = !!repo.permissions?.admin;
  const latest = releases.data?.find((r) => !r.draft && !r.prerelease) ?? releases.data?.find((r) => !r.draft);
  const langTotal = Object.values(languages.data ?? {}).reduce((a, b) => a + b, 0);
  const langs = Object.entries(languages.data ?? {}).sort((a, b) => b[1] - a[1]);
  return (
    <aside className="about repo-sidebar">
      <section>
        <h2>
          About
          {canAdmin ? <IconButton icon={GearIcon} aria-label="Edit repository details" size="small" variant="invisible" onClick={() => setEditing(true)} /> : null}
        </h2>
        {repo.parent ? (
          <p className="muted small" style={{ marginTop: -4 }}>
            forked from{' '}
            <Link to="/$owner/$repo" params={{ owner: repo.parent.owner.login, repo: repo.parent.name }}>
              {repo.parent.full_name}
            </Link>
          </p>
        ) : null}
        {repo.description ? <p>{repo.description}</p> : <p className="muted">No description, website, or topics provided.</p>}
        {repo.homepage ? (
          <p>
            <a href={repo.homepage} style={{ display: 'inline-flex', gap: 6, alignItems: 'center', fontWeight: 600 }}>
              <LinkIcon /> {repo.homepage.replace(/^https?:\/\//, '')}
            </a>
          </p>
        ) : null}
        {repo.topics?.length ? (
          <div className="topics">
            {repo.topics.map((t) => (
              <Link key={t} to="/search" search={{ q: `topic:${t}` }} className="topic">
                {t}
              </Link>
            ))}
          </div>
        ) : null}
        <div className="meta">
          <Link to="/$owner/$repo" params={p}>
            <BookIcon /> Readme
          </Link>
          {repo.license ? (
            <Link to="/$owner/$repo/blob/$rev/$" params={{ ...p, rev: repo.default_branch, _splat: repo.license.path ?? 'LICENSE' }}>
              <LawIcon /> {repo.license.spdx_id === 'NOASSERTION' ? 'License' : `${repo.license.name.replace(' License', '')} license`}
            </Link>
          ) : null}
          <Link to="/$owner/$repo/commits/$rev" params={{ ...p, rev: repo.default_branch }}>
            <PulseIcon /> Activity
          </Link>
          <span>
            <StarIcon /> <strong>{repo.stargazers_count}</strong> stars
          </span>
          <span>
            <EyeIcon /> <strong>{repo.watchers_count}</strong> watching
          </span>
          <span>
            <RepoForkedIcon /> <strong>{repo.forks_count}</strong> forks
          </span>
        </div>
      </section>

      <section>
        <h2>
          <Link to="/$owner/$repo/releases" params={p}>
            Releases {releases.data?.length ? <span className="counter">{releases.data.length}</span> : null}
          </Link>
        </h2>
        {latest ? (
          <Link to="/$owner/$repo/releases/tag/$tag" params={{ ...p, tag: latest.tag_name }} className="latest-release">
            <TagIcon className="release-icon" />
            <span>
              <strong>{latest.name || latest.tag_name}</strong> {!latest.prerelease ? <Label variant="success" size="small">Latest</Label> : <Label variant="attention" size="small">Pre-release</Label>}
              <span className="muted small" style={{ display: 'block' }}>{latest.published_at ? <RelativeTime datetime={latest.published_at} /> : null}</span>
            </span>
          </Link>
        ) : (
          <p className="muted small">
            No releases published.
            {repo.permissions?.push ? (
              <>
                {' '}
                <Link to="/$owner/$repo/releases/new" params={p} search={{}}>
                  Create a new release
                </Link>
              </>
            ) : null}
          </p>
        )}
        {releases.data && releases.data.length > 1 ? (
          <Link to="/$owner/$repo/releases" params={p} className="small">
            + {releases.data.length - 1} {releases.data.length === 2 ? 'release' : 'releases'}
          </Link>
        ) : null}
      </section>

      <section>
        <h2>
          <span>
            Packages
          </span>
        </h2>
        <p className="muted small" style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
          <PackageIcon /> No packages published
        </p>
      </section>

      {contributors.data?.length ? (
        <section>
          <h2>
            Contributors <span className="counter">{contributors.data.length}</span>
          </h2>
          <div className="contributors">
            {contributors.data.slice(0, 14).map((c, i) =>
              c.login ? (
                <Link key={c.login} to="/$owner" params={{ owner: c.login }} title={`${c.login} · ${c.contributions} commits`}>
                  <Avatar src={c.avatar_url ?? ''} size={32} alt={c.login} />
                </Link>
              ) : (
                <span key={i} title={`${c.name ?? c.email} · ${c.contributions} commits`}>
                  <Avatar src={c.avatar_url ?? ''} size={32} alt={c.name ?? ''} />
                </span>
              ),
            )}
          </div>
          {contributors.data.length > 14 ? <p className="small muted" style={{ marginTop: 8 }}>+ {contributors.data.length - 14} contributors</p> : null}
        </section>
      ) : null}

      {langs.length ? (
        <section>
          <h2>Languages</h2>
          <div className="lang-bar">
            {langs.map(([name, bytes]) => (
              <span key={name} style={{ width: `${(bytes / langTotal) * 100}%`, background: LANGUAGE_COLORS[name] ?? '#ededed' }} title={`${name} ${((bytes / langTotal) * 100).toFixed(1)}%`} />
            ))}
          </div>
          <ul className="lang-list">
            {langs.slice(0, 8).map(([name, bytes]) => (
              <li key={name}>
                <span className="dot" style={{ background: LANGUAGE_COLORS[name] ?? '#ededed' }} />
                <strong>{name}</strong> <span className="muted">{((bytes / langTotal) * 100).toFixed(1)}%</span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {editing ? <EditAbout repo={repo} onClose={() => setEditing(false)} /> : null}
    </aside>
  );
}

function EditAbout({ repo, onClose }: { repo: Repo; onClose: () => void }) {
  const qc = useQueryClient();
  const [description, setDescription] = useState(repo.description ?? '');
  const [homepage, setHomepage] = useState(repo.homepage ?? '');
  const [topics, setTopics] = useState(repo.topics?.join(' ') ?? '');
  const save = useMutation({
    mutationFn: async () => {
      await api(`/repos/${repo.full_name}`, { method: 'PATCH', json: { description, homepage } });
      const names = topics.split(/[\s,]+/).map((t) => t.trim().toLowerCase()).filter(Boolean);
      await api(`/repos/${repo.full_name}/topics`, { method: 'PUT', json: { names } });
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase()] });
      onClose();
    },
  });
  return (
    <Dialog
      title="Edit repository details"
      onClose={onClose}
      width="medium"
      footerButtons={[
        { content: 'Cancel', onClick: onClose },
        { content: 'Save changes', buttonType: 'primary', onClick: () => save.mutate(), loading: save.isPending },
      ]}
    >
      <div className="form-stack">
        {save.isError ? <ErrorText error={save.error} /> : null}
        <FormControl>
          <FormControl.Label>Description</FormControl.Label>
          <Textarea block rows={2} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Short description of this repository" />
        </FormControl>
        <FormControl>
          <FormControl.Label>Website</FormControl.Label>
          <TextInput block value={homepage} onChange={(e) => setHomepage(e.target.value)} placeholder="https://example.com" />
        </FormControl>
        <FormControl>
          <FormControl.Label>Topics</FormControl.Label>
          <TextInput block value={topics} onChange={(e) => setTopics(e.target.value)} placeholder="rust web git" />
          <FormControl.Caption>Separate topics with spaces. Lowercase letters, numbers and hyphens.</FormControl.Caption>
        </FormControl>
      </div>
    </Dialog>
  );
}

function ErrorText({ error }: { error: unknown }) {
  return <p style={{ color: 'var(--fgColor-danger)', margin: 0 }}>{error instanceof Error ? error.message : String(error)}</p>;
}
