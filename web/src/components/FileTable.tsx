import { Avatar, RelativeTime } from '@primer/react';
import { FileDirectoryFillIcon, FileIcon, FileSymlinkFileIcon, FileSubmoduleIcon, HistoryIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import type { Commit, ContentEntry, Repo } from '../lib/api';
import { treeCommitsQuery } from '../lib/insights';
import { fullTreeQuery, type TreeEntry } from '../lib/repo';

/// A directory whose only entry is another directory is shown as one row, `a/b/c`, linking
/// to the deepest one, as GitHub does for `.github/workflows`. Needs the full tree.
function collapsed(tree: TreeEntry[] | undefined, entry: ContentEntry): { label: string; path: string } {
  if (!tree) return { label: entry.name, path: entry.path };
  let path = entry.path;
  let label = entry.name;
  for (;;) {
    const prefix = path + '/';
    const children = tree.filter((t) => t.path.startsWith(prefix) && !t.path.slice(prefix.length).includes('/'));
    if (children.length !== 1 || children[0].type !== 'tree') break;
    path = children[0].path;
    label += '/' + path.slice(prefix.length);
  }
  return { label, path };
}

export function FileTable({ repo, rev, path, entries, latest }: { repo: Repo; rev: string; path: string; entries: ContentEntry[]; latest?: Commit | null }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  const parent = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
  const tc = useQuery({ ...treeCommitsQuery(repo.owner.login, repo.name, rev, path), enabled: entries.length > 0 });
  const full = useQuery({ ...fullTreeQuery(repo.owner.login, repo.name, rev), enabled: entries.some((e) => e.type === 'dir') });
  const tree = full.data && !full.data.truncated ? full.data.tree : undefined;
  const commitCount = tc.data?.commits;
  return (
    <div className="file-table">
      <div className="latest-commit">
        {latest ? (
          <>
            {latest.author ? (
              <Link to="/$owner" params={{ owner: latest.author.login }} className="author" style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Avatar src={latest.author.avatar_url} size={20} alt="" /> {latest.author.login}
              </Link>
            ) : (
              <span className="author">{latest.commit.author.name}</span>
            )}
            <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: latest.sha }} className="message" title={latest.commit.message}>
              {latest.commit.message.split('\n')[0]}
            </Link>
            <span className="meta">
              <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: latest.sha }} className="sha">
                {latest.sha.slice(0, 7)}
              </Link>
              <span className="dot">·</span>
              <span>
                <RelativeTime datetime={latest.commit.committer.date} />
              </span>
              <Link to="/$owner/$repo/commits/$rev" params={{ ...p, rev }} className="muted commits-link" style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                <HistoryIcon /> {commitCount !== undefined ? <><strong>{commitCount.toLocaleString('en-US')}</strong> {commitCount === 1 ? 'Commit' : 'Commits'}</> : 'History'}
              </Link>
            </span>
          </>
        ) : (
          <span className="muted">No commits yet</span>
        )}
      </div>
      {path ? (
        <Link to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev, _splat: parent }} className="row">
          <span className="icon dir">
            <FileDirectoryFillIcon />
          </span>
          <span className="name">..</span>
          <span className="commit" />
          <span className="age" />
        </Link>
      ) : null}
      {entries.map((e) => {
        const isDir = e.type === 'dir';
        const Icon = isDir ? FileDirectoryFillIcon : e.type === 'symlink' ? FileSymlinkFileIcon : e.type === 'submodule' ? FileSubmoduleIcon : FileIcon;
        const c = tc.data?.entries[e.name];
        const shown = isDir ? collapsed(tree, e) : { label: e.name, path: e.path };
        return (
          <Link
            key={e.path}
            to={isDir ? '/$owner/$repo/tree/$rev/$' : '/$owner/$repo/blob/$rev/$'}
            params={{ ...p, rev, _splat: shown.path }}
            className="row"
          >
            <span className={`icon${isDir ? ' dir' : ''}`}>
              <Icon />
            </span>
            <span className="name" title={shown.label}>
              {shown.label}
            </span>
            <span className="commit" title={c?.message}>
              {c ? (
                <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: c.sha }} onClick={(ev) => ev.stopPropagation()}>
                  {c.message}
                </Link>
              ) : null}
            </span>
            <span className="age">{c ? <RelativeTime datetime={c.date} /> : null}</span>
          </Link>
        );
      })}
    </div>
  );
}
