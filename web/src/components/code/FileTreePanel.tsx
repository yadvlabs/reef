// A lazy file tree: each directory is fetched with the contents API when first expanded, so
// large repositories never load everything at once. Directories along the current path start
// expanded and the current file is highlighted.
import { TreeView } from '@primer/react';
import { FileDirectoryFillIcon, FileDirectoryOpenFillIcon, FileIcon, FileSubmoduleIcon, FileSymlinkFileIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import type { ContentEntry, Repo } from '../../lib/api';
import { contentsQuery } from '../../lib/repo';

export function FileTreePanel({ repo, rev, current }: { repo: Repo; rev: string; current: string }) {
  return (
    <nav className="file-tree">
      <TreeView aria-label="Files">
        <DirChildren repo={repo} rev={rev} dir="" current={current} />
      </TreeView>
    </nav>
  );
}

function DirChildren({ repo, rev, dir, current }: { repo: Repo; rev: string; dir: string; current: string }) {
  const q = useQuery(contentsQuery(repo.owner.login, repo.name, rev, dir));
  if (q.isPending) return <TreeView.Item id={`${dir}/…loading`}>Loading…</TreeView.Item>;
  if (q.isError || !Array.isArray(q.data)) return <TreeView.Item id={`${dir}/…error`}>Could not load</TreeView.Item>;
  const entries = [...q.data].sort((a, b) => (a.type === 'dir' ? 0 : 1) - (b.type === 'dir' ? 0 : 1) || a.name.localeCompare(b.name));
  return (
    <>
      {entries.map((e) => (e.type === 'dir' ? <DirNode key={e.path} repo={repo} rev={rev} entry={e} current={current} /> : <FileNode key={e.path} repo={repo} rev={rev} entry={e} current={current} />))}
    </>
  );
}

function DirNode({ repo, rev, entry, current }: { repo: Repo; rev: string; entry: ContentEntry; current: string }) {
  const onPath = current === entry.path || current.startsWith(entry.path + '/');
  const [open, setOpen] = useState(onPath);
  const navigate = useNavigate();
  return (
    <TreeView.Item
      id={entry.path}
      expanded={open}
      onExpandedChange={setOpen}
      current={current === entry.path}
      onSelect={() => navigate({ to: '/$owner/$repo/tree/$rev/$', params: { owner: repo.owner.login, repo: repo.name, rev, _splat: entry.path } })}
    >
      <TreeView.LeadingVisual>{open ? <FileDirectoryOpenFillIcon fill="#54aeff" /> : <FileDirectoryFillIcon fill="#54aeff" />}</TreeView.LeadingVisual>
      {entry.name}
      <TreeView.SubTree>{open ? <DirChildren repo={repo} rev={rev} dir={entry.path} current={current} /> : null}</TreeView.SubTree>
    </TreeView.Item>
  );
}

function FileNode({ repo, rev, entry, current }: { repo: Repo; rev: string; entry: ContentEntry; current: string }) {
  const navigate = useNavigate();
  const Icon = entry.type === 'symlink' ? FileSymlinkFileIcon : entry.type === 'submodule' ? FileSubmoduleIcon : FileIcon;
  return (
    <TreeView.Item id={entry.path} current={current === entry.path} onSelect={() => navigate({ to: '/$owner/$repo/blob/$rev/$', params: { owner: repo.owner.login, repo: repo.name, rev, _splat: entry.path } })}>
      <TreeView.LeadingVisual>
        <Icon />
      </TreeView.LeadingVisual>
      {entry.name}
    </TreeView.Item>
  );
}
