// HTTP client for the Reef server. `api()` talks GitHub REST (/api/v3), `reef()` talks
// Reef's own session API (/api/reef), `gql()` talks GraphQL.

export class ApiError extends Error {
  status: number;
  body: unknown;
  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.status = status;
    this.body = body;
  }
  get fieldErrors(): string[] {
    const b = this.body as { errors?: { message?: string; field?: string; code?: string }[] } | null;
    return (b?.errors ?? []).map((e) => e.message ?? `${e.field ?? ''} ${e.code ?? ''}`.trim());
  }
}

type Init = Omit<RequestInit, 'body'> & { json?: unknown; accept?: string; raw?: boolean };

async function request<T>(url: string, init: Init = {}): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set('Accept', init.accept ?? 'application/vnd.github+json');
  let body: BodyInit | undefined;
  if (init.json !== undefined) {
    headers.set('Content-Type', 'application/json');
    body = JSON.stringify(init.json);
  }
  const res = await fetch(url, { ...init, headers, body, credentials: 'same-origin' });
  if (res.status === 204) return undefined as T;
  if (init.raw) {
    if (!res.ok) throw new ApiError(res.status, res.statusText, null);
    return (await res.text()) as T;
  }
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!res.ok) {
    const msg = (data as { message?: string })?.message ?? res.statusText;
    throw new ApiError(res.status, msg, data);
  }
  return data as T;
}

export const api = <T>(path: string, init?: Init) => request<T>(`/api/v3${path}`, init);
export const reef = <T>(path: string, init?: Init) => request<T>(`/api/reef${path}`, init);

export async function gql<T>(query: string, variables: Record<string, unknown> = {}): Promise<T> {
  const res = await request<{ data: T; errors?: { message: string; type?: string }[] }>('/api/graphql', {
    method: 'POST',
    json: { query, variables },
  });
  if (res.errors?.length) throw new ApiError(400, res.errors[0].message, res);
  return res.data;
}

// ----- types (GitHub shapes) -----

export interface User {
  login: string;
  id: number;
  node_id: string;
  avatar_url: string;
  html_url: string;
  type: 'User' | 'Organization';
  site_admin: boolean;
  name?: string | null;
  email?: string | null;
  bio?: string | null;
  company?: string | null;
  location?: string | null;
  blog?: string | null;
  public_repos?: number;
  created_at?: string;
  description?: string | null;
}

export interface Repo {
  id: number;
  node_id: string;
  name: string;
  full_name: string;
  private: boolean;
  owner: User;
  html_url: string;
  description: string | null;
  fork: boolean;
  homepage: string | null;
  default_branch: string;
  clone_url: string;
  ssh_url: string;
  size: number;
  stargazers_count: number;
  watchers_count: number;
  forks_count: number;
  open_issues_count: number;
  open_issues_only_count?: number;
  open_pull_requests_count?: number;
  has_issues: boolean;
  has_projects: boolean;
  has_wiki: boolean;
  archived: boolean;
  is_template: boolean;
  visibility: 'public' | 'private';
  created_at: string;
  updated_at: string;
  pushed_at: string | null;
  topics: string[];
  language?: string | null;
  license?: { key: string; name: string; spdx_id: string; path?: string } | null;
  parent?: Repo;
  viewer_starred?: boolean;
  viewer_subscription?: 'subscribed' | 'ignored' | null;
  permissions?: { admin: boolean; maintain: boolean; push: boolean; triage: boolean; pull: boolean };
  allow_squash_merge?: boolean;
  allow_merge_commit?: boolean;
  allow_rebase_merge?: boolean;
  delete_branch_on_merge?: boolean;
}

export interface Branch {
  name: string;
  commit: { sha: string; url: string };
  protected: boolean;
}

export interface Tag {
  name: string;
  commit: { sha: string };
  tarball_url?: string;
  zipball_url?: string;
}

export interface Release {
  id: number;
  tag_name: string;
  target_commitish: string;
  name: string | null;
  body: string | null;
  body_html?: string;
  draft: boolean;
  prerelease: boolean;
  created_at: string;
  published_at: string | null;
  author: User | null;
  html_url: string;
  tarball_url: string;
  zipball_url: string;
}

export interface Contributor extends Partial<User> {
  contributions: number;
  name?: string | null;
}

export interface GitUser {
  name: string;
  email: string;
  date: string;
}

export interface Verification {
  verified: boolean;
  reason: 'valid' | 'unsigned' | 'unknown_key' | 'bad_email' | 'unverified_email' | 'expired_key' | 'invalid' | 'unknown_signature_type' | string;
  signature: string | null;
  payload: string | null;
  verified_at: string | null;
}

export interface Commit {
  sha: string;
  html_url: string;
  commit: { author: GitUser; committer: GitUser; message: string; tree: { sha: string }; verification?: Verification };
  author: User | null;
  committer: User | null;
  parents: { sha: string }[];
}

export interface ContentEntry {
  type: 'file' | 'dir' | 'symlink' | 'submodule';
  size: number;
  name: string;
  path: string;
  sha: string;
  download_url: string | null;
}

export interface ContentFile extends ContentEntry {
  encoding: 'base64';
  content: string;
}

export interface UserEmail {
  email: string;
  primary: boolean;
  verified: boolean;
  visibility: 'public' | 'private';
}

export interface GpgKey {
  id: number;
  key_id: string;
  fingerprint: string;
  public_key: string;
  emails: { email: string; verified: boolean }[];
  subkeys: { key_id: string; can_sign: boolean; expires_at: string | null }[];
  can_sign: boolean;
  can_certify: boolean;
  can_encrypt_comms: boolean;
  can_encrypt_storage: boolean;
  created_at: string;
  expires_at: string | null;
}

export interface SshSigningKey {
  id: number;
  key: string;
  title: string;
  created_at: string;
}

export interface SshKey {
  id: number;
  key: string;
  title: string;
  created_at: string;
}

export interface Token {
  id: number;
  name: string;
  kind: string;
  token?: string | null;
  token_last4: string;
  scopes: string[];
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
}

export interface Session {
  user: User | null;
  orgs: User[];
  instance: {
    name: string;
    site_url: string;
    api_url: string;
    ssh_host: string;
    ssh_port: number;
    allow_signup: boolean;
    announcement?: string | null;
    password_login: boolean;
    oidc: { name: string; login_url: string } | null;
    version: string;
  };
  unread_notifications?: number;
}

export function decodeContent(f: ContentFile): string {
  const bin = atob(f.content.replace(/\n/g, ''));
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

export function isBinary(text: string): boolean {
  return /\u0000|�/.test(text.slice(0, 8000));
}

export function humanSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} Bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
