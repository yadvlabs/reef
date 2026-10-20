// Pull/push mirrors, source imports and Git LFS settings (Reef API + GitHub import shape).
import { queryOptions } from '@tanstack/react-query';
import { api, reef } from './api';

export interface MirrorConfig {
  pull_url: string;
  username: string | null;
  has_password: boolean;
  interval_minutes: number;
  last_synced_at: string | null;
  last_error: string | null;
  next_sync_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface PushMirror {
  id: number;
  url: string;
  username: string | null;
  has_password: boolean;
  pending: boolean;
  last_attempt_at: string | null;
  last_pushed_at: string | null;
  last_error: string | null;
  created_at: string;
}

export interface LfsOverview {
  enabled: boolean;
  max_object_size: number;
  object_count: number;
  total_bytes: number;
  endpoint: string;
  objects: { oid: string; size: number; created_at: string }[];
  locks: { id: number; path: string; owner: string; locked_at: string }[];
}

export interface ImportStatus {
  vcs?: string;
  vcs_url?: string;
  status: 'none' | 'importing' | 'complete' | 'error' | 'cancelled';
  status_text: string;
  commit_count?: number | null;
  message?: string | null;
  started_at?: string;
  finished_at?: string | null;
  repository_url: string;
}

const key = (full: string, ...rest: unknown[]) => ['repo', ...full.toLowerCase().split('/'), ...rest];

export const mirrorQuery = (full: string) =>
  queryOptions({ queryKey: key(full, 'mirror'), queryFn: () => reef<{ mirror: MirrorConfig | null; allow_file_urls: boolean }>(`/repos/${full}/mirror`) });
export const pushMirrorsQuery = (full: string) =>
  queryOptions({ queryKey: key(full, 'push-mirrors'), queryFn: () => reef<{ push_mirrors: PushMirror[] }>(`/repos/${full}/push-mirrors`) });
export const lfsQuery = (full: string) => queryOptions({ queryKey: key(full, 'lfs'), queryFn: () => reef<LfsOverview>(`/repos/${full}/lfs`) });
export const importQuery = (full: string) =>
  queryOptions({
    queryKey: key(full, 'import'),
    queryFn: () => api<ImportStatus>(`/repos/${full}/import`).catch((e) => (e && typeof e === 'object' && 'status' in e && (e as { status: number }).status === 404 ? ({ status: 'none', status_text: 'No import', repository_url: '' } as ImportStatus) : Promise.reject(e))),
  });

export function startImport(full: string, body: { vcs_url: string; vcs_username?: string; vcs_password?: string }) {
  return api<ImportStatus>(`/repos/${full}/import`, { method: 'PUT', json: { vcs: 'git', ...body } });
}

/** A Git LFS pointer file, if `text` is one. */
export interface LfsPointerData {
  oid: string;
  size: number;
  version: string;
}

export function parseLfsPointer(text: string): LfsPointerData | null {
  if (!text.startsWith('version https://git-lfs.github.com/spec/') || text.length > 1024) return null;
  let version = '';
  let oid = '';
  let size = -1;
  for (const line of text.split('\n')) {
    const [k, v] = line.split(' ', 2);
    if (k === 'version') version = v ?? '';
    else if (k === 'oid') oid = (v ?? '').replace(/^sha256:/, '');
    else if (k === 'size') size = Number(v);
  }
  if (!/^[0-9a-f]{64}$/.test(oid) || !(size >= 0)) return null;
  return { oid, size, version };
}
