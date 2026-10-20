// Repository rows as on GitHub's "Repositories" tab: name, visibility, description, topics,
// language, stars, forks, license and last update.
import { CrustyEmpty } from './Crusty';
import { RelativeTime } from '@primer/react';
import { LawIcon, RepoForkedIcon, StarIcon } from '@primer/octicons-react';
import { Link } from '@tanstack/react-router';
import type { Repo } from '../lib/api';
import { LANGUAGE_COLORS } from '../lib/repo';

export function LanguageDot({ name }: { name: string }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
      <span className="lang-dot" style={{ background: LANGUAGE_COLORS[name] ?? '#ededed' }} /> {name}
    </span>
  );
}

export function RepoListItem({ r, showOwner }: { r: Repo; showOwner?: boolean }) {
  return (
    <div className="item">
      <div className="headline">
        <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
          {showOwner ? `${r.owner.login}/` : ''}
          {r.name}
        </Link>
        <span className="visibility-pill">{r.archived ? 'Public archive' : r.private ? 'Private' : r.is_template ? 'Public template' : 'Public'}</span>
      </div>
      {r.fork && r.parent ? (
        <p className="desc small">
          Forked from{' '}
          <Link to="/$owner/$repo" params={{ owner: r.parent.owner.login, repo: r.parent.name }}>
            {r.parent.full_name}
          </Link>
        </p>
      ) : null}
      {r.description ? <p className="desc">{r.description}</p> : null}
      {r.topics?.length ? (
        <div className="topics" style={{ margin: '4px 0' }}>
          {r.topics.slice(0, 8).map((t) => (
            <Link key={t} to="/search" search={{ q: `topic:${t}` }} className="topic">
              {t}
            </Link>
          ))}
        </div>
      ) : null}
      <div className="meta">
        {r.language ? <LanguageDot name={r.language} /> : null}
        {r.stargazers_count ? (
          <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }} className="muted" style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <StarIcon /> {r.stargazers_count}
          </Link>
        ) : null}
        {r.forks_count ? (
          <Link to="/$owner/$repo/forks" params={{ owner: r.owner.login, repo: r.name }} className="muted" style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <RepoForkedIcon /> {r.forks_count}
          </Link>
        ) : null}
        {r.license && r.license.spdx_id !== 'NOASSERTION' ? (
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4 }}>
            <LawIcon /> {r.license.spdx_id} license
          </span>
        ) : null}
        <span>
          Updated <RelativeTime datetime={r.pushed_at ?? r.updated_at} />
        </span>
      </div>
    </div>
  );
}

export function RepoList({ repos, showOwner, empty }: { repos: Repo[]; showOwner?: boolean; empty?: React.ReactNode }) {
  return (
    <div className="repo-list">
      {repos.map((r) => (
        <RepoListItem key={r.id} r={r} showOwner={showOwner} />
      ))}
      {repos.length === 0 ? <CrustyEmpty mood="builder" title={empty ?? 'No repositories.'}>Create one, or push an existing repository, and it appears here.</CrustyEmpty> : null}
    </div>
  );
}
