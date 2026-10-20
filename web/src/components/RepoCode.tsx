// The code browser shared by the repository home, tree pages, and blob pages.
import { ActionList, ActionMenu, Breadcrumbs, Button, IconButton, TextInput } from '@primer/react';
import { GitBranchIcon, TagIcon, CodeIcon, CopyIcon, CheckIcon, TerminalIcon, BookIcon, PlusIcon, FileIcon, SearchIcon, LawIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useRef, useState } from 'react';
import { api, type ContentEntry, type ContentFile, type Repo } from '../lib/api';
import { branchesQuery, commitsQuery, contentsQuery, fullTreeQuery, readmeHtmlQuery, tagsQuery } from '../lib/repo';
import { FileTable } from './FileTable';
import { ActionListLink, BreadcrumbLink } from './links';
import { ErrorFlash, Loading } from './Loading';
import { Markdown } from './Markdown';
import { useSession } from '../lib/session';
import { usePanelToggle } from './code/CodeLayout';

export function BranchMenu({ repo, rev, path, mode }: { repo: Repo; rev: string; path: string; mode: 'tree' | 'blob' }) {
  const branches = useQuery(branchesQuery(repo.owner.login, repo.name));
  const tags = useQuery(tagsQuery(repo.owner.login, repo.name));
  const navigate = useNavigate();
  const [tab, setTab] = useState<'branches' | 'tags'>('branches');
  const [filter, setFilter] = useState('');
  const isTag = tags.data?.some((t) => t.name === rev);
  const p = { owner: repo.owner.login, repo: repo.name };
  const go = (r: string) => {
    if (!path && mode === 'tree') navigate({ to: '/$owner/$repo/tree/$rev/$', params: { ...p, rev: r, _splat: '' } });
    else navigate({ to: mode === 'tree' ? '/$owner/$repo/tree/$rev/$' : '/$owner/$repo/blob/$rev/$', params: { ...p, rev: r, _splat: path } });
  };
  const list = (tab === 'branches' ? branches.data?.map((b) => b.name) : tags.data?.map((t) => t.name)) ?? [];
  return (
    <ActionMenu>
      <ActionMenu.Button leadingVisual={isTag ? TagIcon : GitBranchIcon}>
        {rev.length === 40 ? <span className="monospace" style={{ fontSize: 12 }}>{rev.slice(0, 7)}</span> : rev}
      </ActionMenu.Button>
      <ActionMenu.Overlay width="medium">
        <div style={{ padding: 8 }}>
          <TextInput block size="small" placeholder={`Find a ${tab === 'branches' ? 'branch' : 'tag'}...`} value={filter} onChange={(e) => setFilter(e.target.value)} />
          <div style={{ display: 'flex', gap: 4, marginTop: 8 }}>
            <Button size="small" variant={tab === 'branches' ? 'default' : 'invisible'} onClick={() => setTab('branches')}>
              Branches
            </Button>
            <Button size="small" variant={tab === 'tags' ? 'default' : 'invisible'} onClick={() => setTab('tags')}>
              Tags
            </Button>
          </div>
        </div>
        <ActionList selectionVariant="single">
          {list
            .filter((n) => n.includes(filter))
            .slice(0, 50)
            .map((n) => (
              <ActionList.Item key={n} selected={n === rev} onSelect={() => go(n)}>
                {n}
                {n === repo.default_branch && tab === 'branches' ? <ActionList.TrailingVisual>default</ActionList.TrailingVisual> : null}
              </ActionList.Item>
            ))}
          {list.length === 0 ? <ActionList.Item disabled>Nothing to show</ActionList.Item> : null}
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}

function CopyField({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div style={{ display: 'flex', gap: 4 }}>
      <TextInput readOnly value={value} block monospace size="small" aria-label="Clone URL" />
      <IconButton
        icon={copied ? CheckIcon : CopyIcon}
        aria-label="Copy"
        size="small"
        onClick={() => {
          navigator.clipboard.writeText(value);
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        }}
      />
    </div>
  );
}

export function CloneMenu({ repo }: { repo: Repo }) {
  const { instance } = useSession();
  const [proto, setProto] = useState<'https' | 'ssh' | 'cli'>('https');
  const https = `${instance?.site_url ?? location.origin}/${repo.full_name}.git`;
  const host = instance?.site_url?.replace(/^https?:\/\//, '') ?? location.host;
  return (
    <ActionMenu>
      <ActionMenu.Button variant="primary" leadingVisual={CodeIcon}>
        Code
      </ActionMenu.Button>
      <ActionMenu.Overlay align="end" width="large">
        <div className="clone-box">
          <div style={{ display: 'flex', gap: 4 }}>
            <Button size="small" variant={proto === 'https' ? 'default' : 'invisible'} onClick={() => setProto('https')}>
              HTTPS
            </Button>
            <Button size="small" variant={proto === 'ssh' ? 'default' : 'invisible'} onClick={() => setProto('ssh')}>
              SSH
            </Button>
            <Button size="small" variant={proto === 'cli' ? 'default' : 'invisible'} onClick={() => setProto('cli')} leadingVisual={TerminalIcon}>
              GitHub CLI
            </Button>
          </div>
          {proto === 'https' ? (
            <>
              <CopyField value={https} />
              <span className="small muted">Clone using the web URL. Sign in with your username and a personal access token.</span>
            </>
          ) : proto === 'ssh' ? (
            <>
              <CopyField value={repo.ssh_url} />
              <span className="small muted">Use a password-protected SSH key.</span>
            </>
          ) : (
            <>
              <CopyField value={`gh repo clone ${repo.full_name}`} />
              <span className="small muted">
                Works with the GitHub CLI: <code>gh auth login --hostname {host}</code>
              </span>
            </>
          )}
        </div>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}

export function PathBreadcrumbs({ repo, rev, path, mode }: { repo: Repo; rev: string; path: string; mode: 'tree' | 'blob' }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  const parts = path ? path.split('/') : [];
  return (
    <Breadcrumbs>
      <BreadcrumbLink to="/$owner/$repo/tree/$rev/$" params={{ ...p, rev, _splat: '' }} selected={parts.length === 0}>
        {repo.name}
      </BreadcrumbLink>
      {parts.map((seg, i) => {
        const sub = parts.slice(0, i + 1).join('/');
        const last = i === parts.length - 1;
        return (
          <BreadcrumbLink key={sub} to={last && mode === 'blob' ? '/$owner/$repo/blob/$rev/$' : '/$owner/$repo/tree/$rev/$'} params={{ ...p, rev, _splat: sub }} selected={last}>
            {seg}
          </BreadcrumbLink>
        );
      })}
    </Breadcrumbs>
  );
}

export function TreeView({ repo, rev, path }: { repo: Repo; rev: string; path: string }) {
  const owner = repo.owner.login;
  const contents = useQuery(contentsQuery(owner, repo.name, rev, path));
  const latest = useQuery({ ...commitsQuery(owner, repo.name, rev, path || undefined, 1, 1), enabled: !contents.isError });
  const readme = useQuery({ ...readmeHtmlQuery(owner, repo.name, rev), enabled: path === '' });
  const license = useQuery({ queryKey: ['repo', owner.toLowerCase(), repo.name.toLowerCase(), 'license'], queryFn: () => api<{ text: string; path: string; license: { name: string; spdx_id: string } }>(`/repos/${repo.full_name}/license`).catch(() => null), enabled: path === '' && rev === repo.default_branch });
  const [tab, setTab] = useState<'readme' | 'license'>('readme');
  const toggle = usePanelToggle();
  if (contents.isPending) return <Loading />;
  if (contents.isError) return <ErrorFlash error={contents.error} />;
  const entries = Array.isArray(contents.data) ? contents.data : [];
  return (
    <>
      <div className="code-toolbar">
        {toggle}
        <BranchMenu repo={repo} rev={rev} path={path} mode="tree" />
        {path ? <PathBreadcrumbs repo={repo} rev={rev} path={path} mode="tree" /> : <RefCounts repo={repo} />}
        <div className="spacer" />
        <GoToFile repo={repo} rev={rev} />
        {repo.permissions?.push ? <AddFileMenu repo={repo} rev={rev} path={path} /> : null}
        {!path ? <CloneMenu repo={repo} /> : null}
      </div>
      <FileTable repo={repo} rev={rev} path={path} entries={entries} latest={latest.data?.[0] ?? null} />
      {path === '' && (readme.data || license.data) ? (
        <div className="readme">
          <div className="readme-header tabs">
            {readme.data ? (
              <button className={tab === 'readme' ? 'active' : ''} onClick={() => setTab('readme')}>
                <BookIcon /> README
              </button>
            ) : null}
            {license.data ? (
              <button className={tab === 'license' || !readme.data ? 'active' : ''} onClick={() => setTab('license')}>
                <LawIcon /> {license.data.license.spdx_id === 'NOASSERTION' ? 'License' : `${license.data.license.spdx_id} license`}
              </button>
            ) : null}
          </div>
          {tab === 'readme' && readme.data ? <Markdown html={readme.data} /> : license.data ? <pre className="license-text">{license.data.text}</pre> : null}
        </div>
      ) : null}
    </>
  );
}

export function entryList(x: ContentEntry[] | ContentFile | undefined): ContentEntry[] {
  return Array.isArray(x) ? x : [];
}


/// Branch and tag counts next to the branch picker, like GitHub's repository home.
export function RefCounts({ repo }: { repo: Repo }) {
  const branches = useQuery(branchesQuery(repo.owner.login, repo.name));
  const tags = useQuery(tagsQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  const nb = branches.data?.length ?? 0;
  const nt = tags.data?.length ?? 0;
  return (
    <div className="ref-counts">
      <Link to="/$owner/$repo/branches" params={p}>
        <GitBranchIcon /> <strong>{nb}</strong> {nb === 1 ? 'Branch' : 'Branches'}
      </Link>
      <Link to="/$owner/$repo/tags" params={p}>
        <TagIcon /> <strong>{nt}</strong> {nt === 1 ? 'Tag' : 'Tags'}
      </Link>
    </div>
  );
}

export function AddFileMenu({ repo, rev, path }: { repo: Repo; rev: string; path: string }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  return (
    <ActionMenu>
      <ActionMenu.Button leadingVisual={PlusIcon} size="small">
        Add file
      </ActionMenu.Button>
      <ActionMenu.Overlay align="end">
        <ActionList>
          <ActionListLink to="/$owner/$repo/new/$rev/$" params={{ ...p, rev, _splat: path }}>
            <ActionList.LeadingVisual>
              <FileIcon />
            </ActionList.LeadingVisual>
            Create new file
          </ActionListLink>
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}

/// "Go to file": a fuzzy file finder over the full tree at the current revision.
export function GoToFile({ repo, rev }: { repo: Repo; rev: string }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState('');
  const [sel, setSel] = useState(0);
  const navigate = useNavigate();
  const tree = useQuery({ ...fullTreeQuery(repo.owner.login, repo.name, rev), enabled: open });
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 't' && !e.metaKey && !e.ctrlKey && !e.altKey && !(e.target instanceof HTMLInputElement) && !(e.target instanceof HTMLTextAreaElement)) {
        e.preventDefault();
        setOpen(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  useEffect(() => {
    if (open) setTimeout(() => inputRef.current?.focus(), 0);
  }, [open]);
  const files = (tree.data?.tree ?? []).filter((e) => e.type === 'blob');
  const needle = q.trim().toLowerCase();
  const matches = (needle ? files.filter((f) => fuzzy(f.path.toLowerCase(), needle)) : files).slice(0, 50);
  const go = (path: string) => {
    setOpen(false);
    setQ('');
    navigate({ to: '/$owner/$repo/blob/$rev/$', params: { owner: repo.owner.login, repo: repo.name, rev, _splat: path } });
  };
  return (
    <>
      <Button size="small" onClick={() => setOpen(true)} leadingVisual={SearchIcon} trailingVisual={() => <kbd className="kbd">t</kbd>}>
        Go to file
      </Button>
      {open ? (
        <div className="finder-backdrop" onClick={() => setOpen(false)}>
          <div className="finder" onClick={(e) => e.stopPropagation()} role="dialog" aria-label="Go to file">
            <TextInput
              ref={inputRef}
              block
              leadingVisual={SearchIcon}
              placeholder={`Go to file in ${repo.name}`}
              value={q}
              onChange={(e) => {
                setQ(e.target.value);
                setSel(0);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setOpen(false);
                if (e.key === 'ArrowDown') {
                  e.preventDefault();
                  setSel((v) => Math.min(v + 1, matches.length - 1));
                }
                if (e.key === 'ArrowUp') {
                  e.preventDefault();
                  setSel((v) => Math.max(v - 1, 0));
                }
                if (e.key === 'Enter' && matches[sel]) go(matches[sel].path);
              }}
            />
            <div className="finder-list">
              {tree.isPending ? <div className="muted small" style={{ padding: 8 }}>Loading files…</div> : null}
              {matches.map((f, i) => {
                const dir = f.path.includes('/') ? f.path.slice(0, f.path.lastIndexOf('/') + 1) : '';
                const name = f.path.slice(dir.length);
                return (
                  <button key={f.path} className={`finder-row${i === sel ? ' selected' : ''}`} onMouseEnter={() => setSel(i)} onClick={() => go(f.path)}>
                    <FileIcon className="muted" />
                    <span>
                      <span className="muted">{dir}</span>
                      <strong>{name}</strong>
                    </span>
                  </button>
                );
              })}
              {!tree.isPending && matches.length === 0 ? <div className="muted small" style={{ padding: 8 }}>No files matched</div> : null}
              {tree.data?.truncated ? <div className="muted small" style={{ padding: 8 }}>The tree is too large to list every file.</div> : null}
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}

function fuzzy(hay: string, needle: string): boolean {
  let i = 0;
  for (const c of hay) {
    if (c === needle[i]) i++;
    if (i === needle.length) return true;
  }
  return i === needle.length;
}

