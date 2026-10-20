import type { Repo } from './api';
import { queryOptions } from '@tanstack/react-query';
import { api, type User } from './api';

export interface Label {
  id: number;
  node_id: string;
  name: string;
  color: string;
  description: string | null;
  default: boolean;
}

export interface Milestone {
  id: number;
  number: number;
  title: string;
  description: string | null;
  state: 'open' | 'closed';
  open_issues: number;
  closed_issues: number;
  due_on: string | null;
  created_at: string;
}

export interface Reactions {
  total_count: number;
  '+1': number;
  '-1': number;
  laugh: number;
  hooray: number;
  confused: number;
  heart: number;
  rocket: number;
  eyes: number;
  /** Reef extra: the reactions the viewer has left on this subject. */
  viewer_reactions?: string[];
}

export interface Issue {
  /** Set on cross-repository lists (GET /issues): the repository the issue is in. */
  repository?: Repo;
  id: number;
  node_id: string;
  number: number;
  title: string;
  body: string | null;
  body_html?: string;
  state: 'open' | 'closed';
  state_reason: string | null;
  user: User;
  labels: Label[];
  assignees: User[];
  milestone: Milestone | null;
  locked: boolean;
  active_lock_reason: string | null;
  comments: number;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
  closed_by: User | null;
  author_association: string;
  reactions: Reactions;
  pull_request?: { url: string; html_url?: string; merged_at: string | null };
  html_url: string;
  draft?: boolean;
  /** Pull requests only (Reef extension): latest review decision and combined checks state. */
  review_decision?: 'APPROVED' | 'CHANGES_REQUESTED' | 'REVIEW_REQUIRED';
  checks_state?: 'success' | 'failure' | 'pending' | null;
}

export interface IssueComment {
  id: number;
  body: string;
  body_html?: string;
  user: User;
  created_at: string;
  updated_at: string;
  author_association: string;
  reactions: Reactions;
}

export interface TimelineItem {
  id: number;
  event: string;
  actor: User | null;
  created_at: string;
  body?: string;
  body_html?: string;
  user?: User;
  label?: { name: string; color: string };
  assignee?: User | null;
  milestone?: { title: string | null };
  rename?: { from: string; to: string };
  state_reason?: string | null;
  lock_reason?: string | null;
  commit_id?: string | null;
  merge_method?: string | null;
  base_ref?: string | null;
  head_ref?: string | null;
  closing_pull?: { number: number; title: string; state: string; is_pull: boolean; html_url: string } | null;
  source?: { type: string; issue: { number: number; title: string; state: string; merged: boolean; pull_request: { merged: boolean } | null; html_url: string } } | null;
  will_close?: boolean;
  reactions?: Reactions;
  author_association?: string;
}

export const HTML = { accept: 'application/vnd.github.html+json' } as const;

export interface IssueListParams {
  state?: string;
  labels?: string;
  assignee?: string;
  creator?: string;
  milestone?: string;
  sort?: string;
  direction?: string;
  q?: string;
  type?: string;
  page?: number;
}

export function issueListQuery(full: string, p: IssueListParams) {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(p)) if (v !== undefined && v !== '' && v !== null) qs.set(k, String(v));
  qs.set('per_page', '25');
  return queryOptions({
    queryKey: ['issues', full.toLowerCase(), 'list', qs.toString()],
    queryFn: async () => {
      const res = await fetch(`/api/v3/repos/${full}/issues?${qs}`, { headers: { Accept: 'application/vnd.github+json' } });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const items = (await res.json()) as Issue[];
      return { items, total: Number(res.headers.get('x-total-count') ?? items.length) };
    },
  });
}

export const issueQuery = (full: string, number: number) =>
  queryOptions({ queryKey: ['issues', full.toLowerCase(), number], queryFn: () => api<Issue>(`/repos/${full}/issues/${number}`, HTML) });

export const timelineQuery = (full: string, number: number) =>
  queryOptions({ queryKey: ['issues', full.toLowerCase(), number, 'timeline'], queryFn: () => api<TimelineItem[]>(`/repos/${full}/issues/${number}/timeline?per_page=100`, HTML) });

export const labelsQuery = (full: string) => queryOptions({ queryKey: ['labels', full.toLowerCase()], queryFn: () => api<Label[]>(`/repos/${full}/labels?per_page=100`) });

export const milestonesQuery = (full: string, state = 'all') =>
  queryOptions({ queryKey: ['milestones', full.toLowerCase(), state], queryFn: () => api<Milestone[]>(`/repos/${full}/milestones?state=${state}&per_page=100`) });

export const assigneesQuery = (full: string) => queryOptions({ queryKey: ['assignees', full.toLowerCase()], queryFn: () => api<User[]>(`/repos/${full}/assignees`) });

/** Text colour for a label background, per GitHub's contrast rule. */
export function labelTextColor(hex: string): string {
  const n = parseInt(hex, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum > 0.6 ? '#1f2328' : '#ffffff';
}

// ----- issue and pull request templates (.github/ISSUE_TEMPLATE, PULL_REQUEST_TEMPLATE) -----

export interface IssueTemplate {
  file: string;
  name: string;
  about: string;
  title: string;
  labels: string[];
  assignees: string[];
  body: string;
}

/** Parse a `---` front matter block; returns the fields and the remaining body. */
export function parseFrontMatter(text: string): { meta: Record<string, string>; body: string } {
  const m = text.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (!m) return { meta: {}, body: text };
  const meta: Record<string, string> = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = line.match(/^([A-Za-z_]+):\s*(.*)$/);
    if (kv) meta[kv[1].toLowerCase()] = kv[2].trim().replace(/^["']|["']$/g, '');
  }
  return { meta, body: m[2].replace(/^\r?\n/, '') };
}

function listField(v: string | undefined): string[] {
  if (!v) return [];
  return v
    .replace(/^\[|\]$/g, '')
    .split(',')
    .map((x) => x.trim().replace(/^["']|["']$/g, ''))
    .filter(Boolean);
}

async function fetchText(full: string, path: string, ref: string): Promise<string | null> {
  const res = await fetch(`/api/v3/repos/${full}/contents/${path.split('/').map(encodeURIComponent).join('/')}?ref=${encodeURIComponent(ref)}`, { headers: { Accept: 'application/vnd.github.raw' } });
  if (!res.ok) return null;
  return res.text();
}

export const issueTemplatesQuery = (full: string, ref: string) =>
  queryOptions({
    queryKey: ['issue-templates', full.toLowerCase(), ref],
    staleTime: 300_000,
    queryFn: async (): Promise<IssueTemplate[]> => {
      const out: IssueTemplate[] = [];
      const dir = await fetch(`/api/v3/repos/${full}/contents/.github/ISSUE_TEMPLATE?ref=${encodeURIComponent(ref)}`, { headers: { Accept: 'application/vnd.github+json' } });
      if (dir.ok) {
        const entries = (await dir.json()) as { name: string; path: string; type: string }[];
        if (Array.isArray(entries)) {
          for (const e of entries.filter((x) => x.type === 'file' && /\.md$/i.test(x.name))) {
            const text = await fetchText(full, e.path, ref);
            if (text === null) continue;
            const { meta, body } = parseFrontMatter(text);
            out.push({ file: e.name, name: meta.name || e.name.replace(/\.md$/i, ''), about: meta.about || '', title: meta.title || '', labels: listField(meta.labels), assignees: listField(meta.assignees), body });
          }
        }
      }
      if (out.length === 0) {
        const single = (await fetchText(full, '.github/ISSUE_TEMPLATE.md', ref)) ?? (await fetchText(full, 'ISSUE_TEMPLATE.md', ref));
        if (single !== null) {
          const { meta, body } = parseFrontMatter(single);
          out.push({ file: 'ISSUE_TEMPLATE.md', name: meta.name || 'Issue', about: meta.about || '', title: meta.title || '', labels: listField(meta.labels), assignees: listField(meta.assignees), body });
        }
      }
      return out;
    },
  });

export const pullTemplateQuery = (full: string, ref: string) =>
  queryOptions({
    queryKey: ['pull-template', full.toLowerCase(), ref],
    staleTime: 300_000,
    queryFn: async (): Promise<string | null> => {
      for (const path of ['.github/PULL_REQUEST_TEMPLATE.md', '.github/pull_request_template.md', 'PULL_REQUEST_TEMPLATE.md', 'docs/PULL_REQUEST_TEMPLATE.md']) {
        const t = await fetchText(full, path, ref);
        if (t !== null) return parseFrontMatter(t).body;
      }
      return null;
    },
  });
