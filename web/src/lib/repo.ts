import { queryOptions } from '@tanstack/react-query';
import { api, reef, type Branch, type Commit, type ContentEntry, type ContentFile, type Contributor, type Release, type Repo, type Tag } from './api';

export const repoQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase()], queryFn: () => api<Repo>(`/repos/${owner}/${repo}`) });

export const branchesQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'branches'], queryFn: () => api<Branch[]>(`/repos/${owner}/${repo}/branches?per_page=100`) });

export const tagsQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'tags'], queryFn: () => api<Tag[]>(`/repos/${owner}/${repo}/tags?per_page=100`) });

export const contentsQuery = (owner: string, repo: string, rev: string, path: string) =>
  queryOptions({
    queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'contents', rev, path],
    queryFn: () => api<ContentEntry[] | ContentFile>(`/repos/${owner}/${repo}/contents${path ? '/' + path.split('/').map(encodeURIComponent).join('/') : ''}?ref=${encodeURIComponent(rev)}`),
  });

export const commitsQuery = (owner: string, repo: string, rev: string, path: string | undefined, page: number, perPage = 35) =>
  queryOptions({
    queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'commits', rev, path ?? '', page, perPage],
    queryFn: () => api<Commit[]>(`/repos/${owner}/${repo}/commits?sha=${encodeURIComponent(rev)}${path ? `&path=${encodeURIComponent(path)}` : ''}&page=${page}&per_page=${perPage}`),
  });

export const commitQuery = (owner: string, repo: string, sha: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'commit', sha], queryFn: () => api<Commit>(`/repos/${owner}/${repo}/commits/${sha}`) });

export const readmeHtmlQuery = (owner: string, repo: string, rev: string) =>
  queryOptions({
    queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'readme', rev],
    queryFn: () => api<string>(`/repos/${owner}/${repo}/readme?ref=${encodeURIComponent(rev)}`, { accept: 'application/vnd.github.html', raw: true }).catch(() => ''),
  });

export const fileHtmlQuery = (owner: string, repo: string, rev: string, path: string) =>
  queryOptions({
    queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'html', rev, path],
    queryFn: () => api<string>(`/repos/${owner}/${repo}/contents/${path.split('/').map(encodeURIComponent).join('/')}?ref=${encodeURIComponent(rev)}`, { accept: 'application/vnd.github.html', raw: true }),
  });

export function isMarkdown(name: string) {
  return /\.(md|markdown|mdown|mkdn)$/i.test(name);
}

export const languagesQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'languages'], queryFn: () => api<Record<string, number>>(`/repos/${owner}/${repo}/languages`) });

export const contributorsQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'contributors'], queryFn: () => api<Contributor[]>(`/repos/${owner}/${repo}/contributors?per_page=100`) });

export const releasesQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'releases'], queryFn: () => api<Release[]>(`/repos/${owner}/${repo}/releases?per_page=100&html=1`) });

export const releaseByTagQuery = (owner: string, repo: string, tag: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'release', tag], queryFn: () => api<Release>(`/repos/${owner}/${repo}/releases/tags/${encodeURIComponent(tag)}?html=1`) });

export interface TreeEntry {
  path: string;
  type: 'blob' | 'tree' | 'commit';
  sha: string;
  size?: number;
}
export const fullTreeQuery = (owner: string, repo: string, rev: string) =>
  queryOptions({
    queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'fulltree', rev],
    queryFn: () => api<{ tree: TreeEntry[]; truncated: boolean }>(`/repos/${owner}/${repo}/git/trees/${encodeURIComponent(rev)}?recursive=1`),
    staleTime: 60_000,
  });

/// GitHub language colours for the languages bar.
export const LANGUAGE_COLORS: Record<string, string> = {
  Rust: '#dea584', Go: '#00ADD8', TypeScript: '#3178c6', JavaScript: '#f1e05a', Python: '#3572A5', Ruby: '#701516', Java: '#b07219', Kotlin: '#A97BFF', Swift: '#F05138', C: '#555555', 'C++': '#f34b7d', 'C#': '#178600', PHP: '#4F5D95', Scala: '#c22d40', Shell: '#89e051', PowerShell: '#012456', Lua: '#000080', Dart: '#00B4AB', Elixir: '#6e4a7e', Erlang: '#B83998', Haskell: '#5e5086', Clojure: '#db5855', OCaml: '#ef7a08', Zig: '#ec915c', Nim: '#ffc200', R: '#198CE7', Julia: '#a270ba', 'Objective-C': '#438eff', Vue: '#41b883', Svelte: '#ff3e00', HTML: '#e34c26', CSS: '#663399', SCSS: '#c6538c', Less: '#1d365d', PLpgSQL: '#336790', 'Protocol Buffer': '#e10098', GraphQL: '#e10098', HCL: '#844FBA', Nix: '#7e7eff', Dockerfile: '#384d54', Makefile: '#427819', CMake: '#DA3434', 'Vim Script': '#199f4b', 'Emacs Lisp': '#c065db', Assembly: '#6E4C13', WebAssembly: '#04133b', Solidity: '#AA6746', Perl: '#0298c3', Groovy: '#4298b8', Gradle: '#02303a',
};

// ----- code view: blame, raw, security -----

export interface BlameHunk {
  sha: string;
  author: string;
  author_email: string;
  author_login: string | null;
  avatar_url: string;
  date: string;
  summary: string;
  lines: [number, number];
}
export const blameQuery = (owner: string, repo: string, rev: string, path: string) =>
  queryOptions({
    queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'blame', rev, path],
    queryFn: () => api<BlameHunk[]>(`/repos/${owner}/${repo}/blame/${path.split('/').map(encodeURIComponent).join('/')}?ref=${encodeURIComponent(rev)}`),
    staleTime: 60_000,
  });

/** URL of the raw bytes of a file, served with a safe content type. */
export function rawUrl(fullName: string, rev: string, path: string): string {
  return `/api/v3/repos/${fullName}/raw/${encodeURIComponent(rev)}/${path.split('/').map(encodeURIComponent).join('/')}`;
}

export interface SecurityPackage {
  name: string;
  requirement: string;
  kind: 'runtime' | 'dev';
}
export interface SecurityManifest {
  manifest: string;
  ecosystem: string;
  packages: SecurityPackage[];
}
export interface SecurityOverview {
  policy: { path: string; html: string } | null;
  advisories: unknown[];
  dependencies: SecurityManifest[];
  total_dependencies: number;
}
export const securityQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: ['repo', owner.toLowerCase(), repo.toLowerCase(), 'security'], queryFn: () => reef<SecurityOverview>(`/repos/${owner}/${repo}/security`), staleTime: 60_000 });
